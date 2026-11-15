//! Unified **Radon** extension system for `RadProxy`.
//!
//! Radons live in the shared `radons/` directory and can be:
//! - Compiled WebAssembly modules (`*.wasm`) executed in a fuel-metered `wasmi` sandbox,
//!   with host functions in the `"radbyte"` namespace for reading strings and setting
//!   custom disconnect reasons.
//! - Declarative `*.toml` manifests (e.g. hostname-based backend routing).
//! - Native Rust implementations of [`ProxyRadon`].

use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use rad_api::v1::API_VERSION;
pub use rad_api::v1::{ProxyRadon, RadonAction, RadonContext};
use rad_core::packet::handshake::Handshake;
use rad_core::packet::login::LoginStart;
use rad_core::prelude::*;
use tracing::{info, warn};
use wasmi::{
    Caller, Config as WasmiConfig, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

/// Maximum fuel units a single `.wasm` radon hook invocation may consume.
const WASM_HOOK_FUEL: u64 = 100_000;
/// Maximum linear memory a single `.wasm` radon instance may allocate (`16 MiB`).
const WASM_MAX_MEMORY_BYTES: usize = 16 * 1024 * 1024;

fn default_wasm_limits() -> StoreLimits {
    StoreLimitsBuilder::new()
        .memory_size(WASM_MAX_MEMORY_BYTES)
        .table_elements(10_000)
        .instances(4)
        .memories(2)
        .tables(4)
        .build()
}

/// Built-in declarative host-routing radon loaded from `radons/*.toml` manifests.
struct HostRouteRadon {
    name: String,
    version: String,
    match_host: String,
    target: SocketAddr,
}

impl ProxyRadon for HostRouteRadon {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        &self.version
    }

    fn on_handshake(&self, _ctx: RadonContext, handshake: &Handshake) -> RadonAction {
        let host = handshake.server_address.split('\0').next().unwrap_or("");
        if host.eq_ignore_ascii_case(&self.match_host) {
            RadonAction::RouteTo(self.target)
        } else {
            RadonAction::Continue
        }
    }
}

/// Per-invocation host state exposed to a `.wasm` radon.
struct WasmHostState {
    ip: String,
    host: String,
    username: String,
    config: String,
    deny_reason: Option<String>,
    limits: StoreLimits,
}

/// Sandboxed WebAssembly radon loaded from `radons/*.wasm`.
///
/// ## Host Functions (`"radbyte"` module)
/// - `rad_get_host(ptr: i32, max_len: i32) -> i32` — writes handshake hostname into WASM memory, returns bytes written.
/// - `rad_get_username(ptr: i32, max_len: i32) -> i32` — writes player username into WASM memory, returns bytes written.
/// - `rad_get_ip(ptr: i32, max_len: i32) -> i32` — writes client IP string into WASM memory, returns bytes written.
/// - `rad_get_config(ptr: i32, max_len: i32) -> i32` — writes `radonfigs/<name>.toml` into WASM memory, returns bytes written.
/// - `rad_set_deny_reason(ptr: i32, len: i32)` — sets custom disconnect reason from WASM memory.
///
/// ## Exported WASM Hooks (all optional)
/// - `radon_on_handshake(protocol: i32, port: i32, next_state: i32) -> i32` (`0` = Continue, `1` = Deny)
/// - `radon_on_login(online_players: i32) -> i32` (`0` = Continue, `1` = Deny)
pub struct WasmRadon {
    name: String,
    config: String,
    engine: Engine,
    module: Module,
}

impl WasmRadon {
    /// Compiles a `.wasm` module into a sandboxed [`WasmRadon`].
    #[allow(dead_code)]
    pub fn from_bytes(name: impl Into<String>, wasm_bytes: &[u8]) -> Result<Self> {
        Self::from_bytes_with_config(name, wasm_bytes, String::new())
    }

    /// Compiles a `.wasm` module with an associated `radonfigs/<name>.toml` string.
    pub fn from_bytes_with_config(
        name: impl Into<String>,
        wasm_bytes: &[u8],
        config: String,
    ) -> Result<Self> {
        let mut cfg = WasmiConfig::default();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg);
        let module = Module::new(&engine, wasm_bytes)
            .map_err(|e| Error::InvalidString(format!("invalid wasm radon: {e}")))?;

        Ok(Self {
            name: name.into(),
            config,
            engine,
            module,
        })
    }

    fn build_linker(engine: &Engine) -> Linker<WasmHostState> {
        let mut linker = Linker::<WasmHostState>::new(engine);

        let _ = linker.func_wrap(
            "radbyte",
            "rad_get_host",
            |mut caller: Caller<'_, WasmHostState>, ptr: i32, max_len: i32| -> i32 {
                let s = caller.data().host.clone();
                write_str_to_wasm(&mut caller, ptr, max_len, &s)
            },
        );
        let _ = linker.func_wrap(
            "radbyte",
            "rad_get_username",
            |mut caller: Caller<'_, WasmHostState>, ptr: i32, max_len: i32| -> i32 {
                let s = caller.data().username.clone();
                write_str_to_wasm(&mut caller, ptr, max_len, &s)
            },
        );
        let _ = linker.func_wrap(
            "radbyte",
            "rad_get_ip",
            |mut caller: Caller<'_, WasmHostState>, ptr: i32, max_len: i32| -> i32 {
                let s = caller.data().ip.clone();
                write_str_to_wasm(&mut caller, ptr, max_len, &s)
            },
        );
        let _ = linker.func_wrap(
            "radbyte",
            "rad_get_config",
            |mut caller: Caller<'_, WasmHostState>, ptr: i32, max_len: i32| -> i32 {
                let s = caller.data().config.clone();
                write_str_to_wasm(&mut caller, ptr, max_len, &s)
            },
        );
        let _ = linker.func_wrap(
            "radbyte",
            "rad_set_deny_reason",
            |mut caller: Caller<'_, WasmHostState>, ptr: i32, len: i32| {
                if let Some(reason) = read_str_from_wasm(&mut caller, ptr, len) {
                    caller.data_mut().deny_reason = Some(reason);
                }
            },
        );

        linker
    }

    fn call_hook(
        &self,
        host_state: WasmHostState,
        fn_name: &str,
        args: &[i32],
    ) -> (Option<i32>, Option<String>) {
        let mut store = Store::new(&self.engine, host_state);
        store.limiter(|state| &mut state.limits);
        if store.set_fuel(WASM_HOOK_FUEL).is_err() {
            return (None, None);
        }
        let linker = Self::build_linker(&self.engine);
        let Ok(pre) = linker.instantiate(&mut store, &self.module) else {
            return (None, None);
        };
        let Ok(instance) = pre.start(&mut store) else {
            return (None, None);
        };

        let code = match args.len() {
            1 => instance
                .get_typed_func::<i32, i32>(&store, fn_name)
                .ok()
                .and_then(|f| f.call(&mut store, args[0]).ok()),
            3 => instance
                .get_typed_func::<(i32, i32, i32), i32>(&store, fn_name)
                .ok()
                .and_then(|f| f.call(&mut store, (args[0], args[1], args[2])).ok()),
            _ => None,
        };

        (code, store.into_data().deny_reason)
    }
}

fn write_str_to_wasm(
    caller: &mut Caller<'_, WasmHostState>,
    ptr: i32,
    max_len: i32,
    s: &str,
) -> i32 {
    let Ok(offset) = usize::try_from(ptr) else {
        return -1;
    };
    let Ok(cap) = usize::try_from(max_len) else {
        return -1;
    };
    let Some(mem) = caller.get_export("memory").and_then(|e| e.into_memory()) else {
        return -1;
    };
    let bytes = s.as_bytes();
    let write_len = bytes.len().min(cap);
    if mem.write(caller, offset, &bytes[..write_len]).is_err() {
        return -1;
    }
    i32::try_from(write_len).unwrap_or(0)
}

fn read_str_from_wasm(
    caller: &mut Caller<'_, WasmHostState>,
    ptr: i32,
    len: i32,
) -> Option<String> {
    let offset = usize::try_from(ptr).ok()?;
    let byte_len = usize::try_from(len).ok()?.min(4096);
    let mem = caller.get_export("memory").and_then(|e| e.into_memory())?;
    let mut buf = vec![0u8; byte_len];
    mem.read(caller, offset, &mut buf).ok()?;
    String::from_utf8(buf).ok()
}

impl ProxyRadon for WasmRadon {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        "wasm"
    }

    fn on_handshake(&self, ctx: RadonContext, handshake: &Handshake) -> RadonAction {
        let state = WasmHostState {
            ip: ctx.peer_addr.ip().to_string(),
            host: handshake.server_address.clone(),
            username: String::new(),
            config: self.config.clone(),
            deny_reason: None,
            limits: default_wasm_limits(),
        };
        let (code, reason) = self.call_hook(
            state,
            "radon_on_handshake",
            &[
                handshake.protocol_version.0,
                i32::from(handshake.server_port),
                handshake.next_state_id(),
            ],
        );
        match code {
            Some(1) => RadonAction::Deny(
                TextComponent::text(
                    reason.unwrap_or_else(|| "Connection rejected by WASM radon.".into()),
                )
                .color("red"),
            ),
            _ => RadonAction::Continue,
        }
    }

    fn on_login(
        &self,
        ctx: RadonContext,
        handshake: &Handshake,
        login: &LoginStart,
    ) -> RadonAction {
        let state = WasmHostState {
            ip: ctx.peer_addr.ip().to_string(),
            host: handshake.server_address.clone(),
            username: login.username.clone(),
            config: self.config.clone(),
            deny_reason: None,
            limits: default_wasm_limits(),
        };
        let (code, reason) = self.call_hook(state, "radon_on_login", &[ctx.online_players]);
        match code {
            Some(1) => RadonAction::Deny(
                TextComponent::text(
                    reason.unwrap_or_else(|| "Login rejected by WASM radon.".into()),
                )
                .color("red"),
            ),
            _ => RadonAction::Continue,
        }
    }
}

/// Manages registered [`ProxyRadon`] instances and external radon discovery.
#[derive(Default)]
pub struct RadonManager {
    radons: Vec<Arc<dyn ProxyRadon>>,
    configs: std::collections::HashMap<String, String>,
}

impl RadonManager {
    /// Creates an empty [`RadonManager`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a Rust [`ProxyRadon`] and invokes its `on_enable` hook.
    pub fn register<R: ProxyRadon + 'static>(&mut self, radon: R) {
        let radon = Arc::new(radon);
        info!(
            name = radon.name(),
            version = radon.version(),
            "enabling proxy radon"
        );
        radon.on_enable();
        self.radons.push(radon);
    }

    /// Ensures both `radons_dir` and `radonfigs_dir` exist, loading radons and their configs.
    pub fn load_from_dirs(
        &mut self,
        radons_dir: impl AsRef<Path>,
        radonfigs_dir: impl AsRef<Path>,
    ) -> Result<()> {
        self.load_radonfigs(radonfigs_dir)?;
        self.load_from_dir(radons_dir)
    }

    /// Ensures `radonfigs_dir` exists and loads per-radon `*.toml` config files.
    pub fn load_radonfigs(&mut self, radonfigs_dir: impl AsRef<Path>) -> Result<()> {
        let dir = radonfigs_dir.as_ref();
        if !dir.exists() {
            fs::create_dir_all(dir)?;
            info!(dir = %dir.display(), "created radonfigs directory");
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            match fs::read_to_string(&path) {
                Ok(raw) => {
                    if let Ok(manifest) = toml::from_str::<RadonManifest>(&raw)
                        && manifest.matches_target(RadonTarget::Proxy)
                        && let (Some(host), Some(addr_str)) =
                            (manifest.route_host, manifest.route_to)
                        && let Ok(target) = addr_str.parse::<SocketAddr>()
                    {
                        self.register(HostRouteRadon {
                            name: manifest.id,
                            version: manifest.version,
                            match_host: host,
                            target,
                        });
                    }
                    self.configs.insert(stem.to_owned(), raw);
                }
                Err(err) => {
                    warn!(path = %path.display(), %err, "failed to read radonfig");
                }
            }
        }
        Ok(())
    }

    /// Returns the raw TOML config string for a radon from `radonfigs/<name>.toml`.
    #[allow(dead_code)]
    #[must_use]
    pub fn config_for(&self, name: &str) -> Option<&str> {
        self.configs.get(name).map(String::as_str)
    }

    /// Ensures `radons_dir` exists and loads `*.toml` manifests and `*.wasm` radons.
    pub fn load_from_dir(&mut self, radons_dir: impl AsRef<Path>) -> Result<()> {
        let dir = radons_dir.as_ref();
        if !dir.exists() {
            fs::create_dir_all(dir)?;
            info!(dir = %dir.display(), "created radons directory");
            return Ok(());
        }

        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            match path.extension().and_then(|e| e.to_str()) {
                Some("toml") => self.load_toml_manifest(&path),
                Some("wasm") => self.load_wasm_file(&path),
                _ => {}
            }
        }
        Ok(())
    }

    fn load_toml_manifest(&mut self, path: &Path) {
        match fs::read_to_string(path).and_then(|s| {
            toml::from_str::<RadonManifest>(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        }) {
            Ok(manifest) => {
                if !manifest.matches_target(RadonTarget::Proxy) {
                    return;
                }
                if manifest.api_version > API_VERSION {
                    warn!(
                        id = %manifest.id,
                        found_api = manifest.api_version,
                        supported_api = API_VERSION,
                        "radon manifest targets a newer RadAPI version"
                    );
                }
                info!(
                    id = %manifest.id,
                    version = %manifest.version,
                    api = manifest.api_version,
                    description = %manifest.description,
                    "discovered proxy radon manifest"
                );
                if let (Some(host), Some(addr_str)) = (manifest.route_host, manifest.route_to) {
                    match addr_str.parse::<SocketAddr>() {
                        Ok(target) => {
                            self.register(HostRouteRadon {
                                name: manifest.id,
                                version: manifest.version,
                                match_host: host,
                                target,
                            });
                        }
                        Err(err) => {
                            warn!(path = %path.display(), %err, "invalid route_to address in radon manifest");
                        }
                    }
                }
            }
            Err(err) => {
                warn!(path = %path.display(), %err, "failed to parse radon manifest");
            }
        }
    }

    fn load_wasm_file(&mut self, path: &Path) {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed-wasm");
        let cfg = self.config_for(name).unwrap_or("").to_owned();
        match fs::read(path)
            .map_err(Error::Io)
            .and_then(|bytes| WasmRadon::from_bytes_with_config(name, &bytes, cfg))
        {
            Ok(radon) => {
                info!(path = %path.display(), name, "loaded WASM radon");
                self.register(radon);
            }
            Err(err) => {
                warn!(path = %path.display(), %err, "failed to load WASM radon");
            }
        }
    }

    /// Runs `on_handshake` across all registered radons.
    #[must_use]
    pub fn dispatch_handshake(&self, ctx: RadonContext, handshake: &Handshake) -> RadonAction {
        let mut route_override = None;
        for radon in &self.radons {
            match radon.on_handshake(ctx, handshake) {
                RadonAction::Continue => {}
                RadonAction::Deny(reason) => return RadonAction::Deny(reason),
                RadonAction::RouteTo(addr) => route_override = Some(addr),
            }
        }
        route_override.map_or(RadonAction::Continue, RadonAction::RouteTo)
    }

    /// Runs `on_login` across all registered radons.
    #[must_use]
    pub fn dispatch_login(
        &self,
        ctx: RadonContext,
        handshake: &Handshake,
        login: &LoginStart,
    ) -> RadonAction {
        let mut route_override = None;
        for radon in &self.radons {
            match radon.on_login(ctx, handshake, login) {
                RadonAction::Continue => {}
                RadonAction::Deny(reason) => return RadonAction::Deny(reason),
                RadonAction::RouteTo(addr) => route_override = Some(addr),
            }
        }
        route_override.map_or(RadonAction::Continue, RadonAction::RouteTo)
    }

    /// Returns `(name, version)` pairs for all loaded radons.
    #[must_use]
    pub fn list(&self) -> Vec<(&str, &str)> {
        self.radons
            .iter()
            .map(|r| (r.name(), r.version()))
            .collect()
    }

    /// Returns the number of loaded radons.
    #[must_use]
    pub fn len(&self) -> usize {
        self.radons.len()
    }

    /// Disables all loaded radons on shutdown.
    pub fn disable_all(&self) {
        for radon in &self.radons {
            radon.on_disable();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BlockMaintenanceRadon;
    impl ProxyRadon for BlockMaintenanceRadon {
        fn name(&self) -> &str {
            "maintenance"
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn on_handshake(&self, _ctx: RadonContext, hs: &Handshake) -> RadonAction {
            if hs.server_address == "blocked.radbyte.xyz" {
                RadonAction::Deny(TextComponent::text("Blocked by radon"))
            } else {
                RadonAction::Continue
            }
        }
    }

    #[test]
    fn radon_can_deny_handshake() {
        let mut manager = RadonManager::new();
        manager.register(BlockMaintenanceRadon);

        let ctx = RadonContext {
            peer_addr: "127.0.0.1:50000".parse().unwrap(),
            online_players: 0,
        };
        let hs = Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "blocked.radbyte.xyz".into(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        assert!(matches!(
            manager.dispatch_handshake(ctx, &hs),
            RadonAction::Deny(_)
        ));
    }

    #[test]
    fn wasm_radon_executes_fuel_metered_hook() {
        // Minimal valid WASM module exporting `radon_on_handshake(i32, i32, i32) -> i32` returning 1 (Deny).
        let wasm: &[u8] = &[
            0x00, 0x61, 0x73, 0x6D, // magic
            0x01, 0x00, 0x00, 0x00, // version
            0x01, 0x08, 0x01, 0x60, 0x03, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, // type
            0x03, 0x02, 0x01, 0x00, // func
            0x07, 0x16, 0x01, 0x12, b'r', b'a', b'd', b'o', b'n', b'_', b'o', b'n', b'_', b'h',
            b'a', b'n', b'd', b's', b'h', b'a', b'k', b'e', 0x00, 0x00, // export
            0x0A, 0x06, 0x01, 0x04, 0x00, 0x41, 0x01, 0x0B, // code
        ];

        let mut manager = RadonManager::new();
        manager.register(WasmRadon::from_bytes("test-wasm", wasm).unwrap());

        let ctx = RadonContext {
            peer_addr: "127.0.0.1:50000".parse().unwrap(),
            online_players: 0,
        };
        let hs = Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "play.radbyte.xyz".into(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        assert!(matches!(
            manager.dispatch_handshake(ctx, &hs),
            RadonAction::Deny(_)
        ));
    }

    #[test]
    fn wasm_radon_enforces_memory_limits() {
        // Module declaring 300 pages (19.2 MiB > 16 MiB limit) and returning 1 (Deny)
        let wasm: &[u8] = &[
            0x00, 0x61, 0x73, 0x6D, // magic
            0x01, 0x00, 0x00, 0x00, // version
            0x01, 0x08, 0x01, 0x60, 0x03, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, // type
            0x03, 0x02, 0x01, 0x00, // func
            0x05, 0x04, 0x01, 0x00, 0xAC, 0x02, // memory: min = 300 (LEB128 0xAC 0x02)
            0x07, 0x16, 0x01, 0x12, b'r', b'a', b'd', b'o', b'n', b'_', b'o', b'n', b'_', b'h',
            b'a', b'n', b'd', b's', b'h', b'a', b'k', b'e', 0x00, 0x00, // export
            0x0A, 0x06, 0x01, 0x04, 0x00, 0x41, 0x01, 0x0B, // code
        ];

        let mut manager = RadonManager::new();
        manager.register(WasmRadon::from_bytes("oom-wasm", wasm).unwrap());

        let ctx = RadonContext {
            peer_addr: "127.0.0.1:50000".parse().unwrap(),
            online_players: 0,
        };
        let hs = Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "play.radbyte.xyz".into(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };

        // StoreLimits rejects instantiation cleanly -> falls back to Continue
        assert!(matches!(
            manager.dispatch_handshake(ctx, &hs),
            RadonAction::Continue
        ));
    }

    #[test]
    fn compiled_wasm32_guest_radon_works() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let guest_manifest = root
            .join("examples")
            .join("radon-guest-proxy")
            .join("Cargo.toml");
        let wasm_path = root
            .join("examples")
            .join("radon-guest-proxy")
            .join("target")
            .join("wasm32-unknown-unknown")
            .join("release")
            .join("radon_guest_proxy.wasm");

        if !wasm_path.exists() {
            let status = std::process::Command::new("cargo")
                .args([
                    "build",
                    "--manifest-path",
                    guest_manifest.to_str().unwrap(),
                    "--target",
                    "wasm32-unknown-unknown",
                    "--release",
                ])
                .status()
                .expect("cargo build for wasm32 guest");
            assert!(status.success(), "wasm32 guest build should succeed");
        }

        let wasm_bytes = std::fs::read(&wasm_path).expect("read compiled .wasm");
        let mut manager = RadonManager::new();
        manager.register(
            WasmRadon::from_bytes_with_config(
                "radon-guest-example",
                &wasm_bytes,
                "deny_proto=999".into(),
            )
            .unwrap(),
        );

        let ctx = RadonContext {
            peer_addr: "127.0.0.1:50000".parse().unwrap(),
            online_players: 0,
        };
        let denied_hs = Handshake {
            protocol_version: VarInt(999),
            server_address: "play.radbyte.xyz".into(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };
        assert!(matches!(
            manager.dispatch_handshake(ctx, &denied_hs),
            RadonAction::Deny(ref tc) if tc.text == "blocked by compiled wasm radon"
        ));

        let ok_hs = Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "play.radbyte.xyz".into(),
            server_port: 25565,
            next_state: ConnectionState::Login,
        };
        assert_eq!(
            manager.dispatch_handshake(ctx, &ok_hs),
            RadonAction::Continue
        );
    }
}
