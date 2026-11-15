//! Fuel-metered WASM & native [`ServerRadon`] host for `RadServer`.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use rad_api::v1::wasm::{ACTION_ALLOW, ACTION_CANCEL};
use rad_api::v1::{ServerCommandAction, ServerEventResult, ServerPlayerContext, ServerRadon};
use tracing::{info, warn};
use wasmi::{
    Caller, Config as WasmiConfig, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder,
};

/// Maximum fuel units a single `.wasm` hook invocation may consume.
const WASM_HOOK_FUEL: u64 = 100_000;
/// Maximum linear memory a single `.wasm` server radon instance may allocate (`16 MiB`).
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

/// Per-invocation host state exposed to a `.wasm` server radon.
struct WasmServerState {
    entity_id: i32,
    username: String,
    config: String,
    actions: Vec<ServerCommandAction>,
    limits: StoreLimits,
}

/// Sandboxed WebAssembly radon for `RadServer` (`wasmi`, fuel-metered).
///
/// ## Host Functions (`"radbyte"` namespace)
/// - `rad_get_username(ptr: i32, max_len: i32) -> i32`
/// - `rad_get_config(ptr: i32, max_len: i32) -> i32`
/// - `rad_send_chat(ptr: i32, len: i32)`
/// - `rad_broadcast_chat(ptr: i32, len: i32)`
/// - `rad_teleport_player(x: f64, y: f64, z: f64)`
/// - `rad_set_block(x: i32, y: i32, z: i32, state_id: i32)`
///
/// ## Exported WASM Hooks (all optional)
/// - `radon_on_player_join(entity_id: i32)`
/// - `radon_on_player_quit(entity_id: i32)`
/// - `radon_on_player_chat(entity_id: i32) -> i32` (`0` = Allow, `1` = Cancel)
/// - `radon_on_block_break(x: i32, y: i32, z: i32, state_id: i32) -> i32`
/// - `radon_on_block_place(x: i32, y: i32, z: i32, state_id: i32) -> i32`
/// - `radon_on_player_damage(entity_id: i32, amount: f32) -> i32`
/// - `radon_on_server_tick(tick: i64)`
pub struct WasmServerRadon {
    name: String,
    config: String,
    engine: Engine,
    module: Module,
}

impl WasmServerRadon {
    /// Compiles a `.wasm` binary into a sandboxed [`WasmServerRadon`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn from_bytes(name: impl Into<String>, wasm_bytes: &[u8]) -> Result<Self, String> {
        Self::from_bytes_with_config(name, wasm_bytes, String::new())
    }

    /// Compiles a `.wasm` binary with an associated `radonfigs/<name>.toml` string.
    pub fn from_bytes_with_config(
        name: impl Into<String>,
        wasm_bytes: &[u8],
        config: String,
    ) -> Result<Self, String> {
        let mut cfg = WasmiConfig::default();
        cfg.consume_fuel(true);
        let engine = Engine::new(&cfg);
        let module = Module::new(&engine, wasm_bytes).map_err(|e| e.to_string())?;
        Ok(Self {
            name: name.into(),
            config,
            engine,
            module,
        })
    }

    fn build_store_and_instance(
        &self,
        entity_id: i32,
        username: String,
    ) -> Result<(Store<WasmServerState>, wasmi::Instance), String> {
        let mut store = Store::new(
            &self.engine,
            WasmServerState {
                entity_id,
                username,
                config: self.config.clone(),
                actions: Vec::new(),
                limits: default_wasm_limits(),
            },
        );
        store.limiter(|state| &mut state.limits);
        store.set_fuel(WASM_HOOK_FUEL).map_err(|e| e.to_string())?;

        let mut linker = <Linker<WasmServerState>>::new(&self.engine);

        linker
            .func_wrap(
                "radbyte",
                "rad_get_username",
                |mut caller: Caller<'_, WasmServerState>, ptr: i32, max_len: i32| -> i32 {
                    let s = caller.data().username.clone();
                    write_guest_str(&mut caller, ptr, max_len, &s)
                },
            )
            .map_err(|e| e.to_string())?;

        linker
            .func_wrap(
                "radbyte",
                "rad_get_config",
                |mut caller: Caller<'_, WasmServerState>, ptr: i32, max_len: i32| -> i32 {
                    let s = caller.data().config.clone();
                    write_guest_str(&mut caller, ptr, max_len, &s)
                },
            )
            .map_err(|e| e.to_string())?;

        linker
            .func_wrap(
                "radbyte",
                "rad_send_chat",
                |mut caller: Caller<'_, WasmServerState>, ptr: i32, len: i32| {
                    if let Some(msg) = read_guest_str(&mut caller, ptr, len) {
                        let eid = caller.data().entity_id;
                        caller
                            .data_mut()
                            .actions
                            .push(ServerCommandAction::SendMessage {
                                entity_id: eid,
                                message: msg,
                            });
                    }
                },
            )
            .map_err(|e| e.to_string())?;

        linker
            .func_wrap(
                "radbyte",
                "rad_broadcast_chat",
                |mut caller: Caller<'_, WasmServerState>, ptr: i32, len: i32| {
                    if let Some(msg) = read_guest_str(&mut caller, ptr, len) {
                        caller
                            .data_mut()
                            .actions
                            .push(ServerCommandAction::BroadcastChat(msg));
                    }
                },
            )
            .map_err(|e| e.to_string())?;

        linker
            .func_wrap(
                "radbyte",
                "rad_teleport_player",
                |mut caller: Caller<'_, WasmServerState>, x: f64, y: f64, z: f64| {
                    let eid = caller.data().entity_id;
                    caller
                        .data_mut()
                        .actions
                        .push(ServerCommandAction::TeleportPlayer {
                            entity_id: eid,
                            x,
                            y,
                            z,
                        });
                },
            )
            .map_err(|e| e.to_string())?;

        linker
            .func_wrap(
                "radbyte",
                "rad_set_block",
                |mut caller: Caller<'_, WasmServerState>, x: i32, y: i32, z: i32, state_id: i32| {
                    let state = u16::try_from(state_id).unwrap_or(0);
                    caller
                        .data_mut()
                        .actions
                        .push(ServerCommandAction::SetBlock {
                            x,
                            y,
                            z,
                            state_id: state,
                        });
                },
            )
            .map_err(|e| e.to_string())?;

        let pre = linker
            .instantiate(&mut store, &self.module)
            .map_err(|e| e.to_string())?;
        let instance = pre.start(&mut store).map_err(|e| e.to_string())?;
        Ok((store, instance))
    }
}

impl ServerRadon for WasmServerRadon {
    fn name(&self) -> &str {
        &self.name
    }

    fn version(&self) -> &str {
        "wasm-v3"
    }

    fn on_player_join(&self, player: &ServerPlayerContext, actions: &mut Vec<ServerCommandAction>) {
        let Ok((mut store, instance)) =
            self.build_store_and_instance(player.entity_id, player.username.clone())
        else {
            return;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_player_join")
            && let Ok(typed) = func.typed::<i32, ()>(&store)
            && typed.call(&mut store, player.entity_id).is_ok()
        {
            actions.extend(store.into_data().actions);
        }
    }

    fn on_player_quit(&self, player: &ServerPlayerContext, actions: &mut Vec<ServerCommandAction>) {
        let Ok((mut store, instance)) =
            self.build_store_and_instance(player.entity_id, player.username.clone())
        else {
            return;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_player_quit")
            && let Ok(typed) = func.typed::<i32, ()>(&store)
            && typed.call(&mut store, player.entity_id).is_ok()
        {
            actions.extend(store.into_data().actions);
        }
    }

    fn on_player_chat(
        &self,
        player: &ServerPlayerContext,
        _message: &str,
        actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        let Ok((mut store, instance)) =
            self.build_store_and_instance(player.entity_id, player.username.clone())
        else {
            return ServerEventResult::Allow;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_player_chat")
            && let Ok(typed) = func.typed::<i32, i32>(&store)
            && let Ok(res) = typed.call(&mut store, player.entity_id)
        {
            actions.extend(store.into_data().actions);
            return if res == ACTION_CANCEL {
                ServerEventResult::Cancel
            } else {
                ServerEventResult::Allow
            };
        }
        ServerEventResult::Allow
    }

    fn on_block_break(
        &self,
        player: &ServerPlayerContext,
        x: i32,
        y: i32,
        z: i32,
        state_id: u16,
        actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        let Ok((mut store, instance)) =
            self.build_store_and_instance(player.entity_id, player.username.clone())
        else {
            return ServerEventResult::Allow;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_block_break")
            && let Ok(typed) = func.typed::<(i32, i32, i32, i32), i32>(&store)
            && let Ok(res) = typed.call(&mut store, (x, y, z, i32::from(state_id)))
        {
            actions.extend(store.into_data().actions);
            return if res == ACTION_CANCEL {
                ServerEventResult::Cancel
            } else {
                ServerEventResult::Allow
            };
        }
        ServerEventResult::Allow
    }

    fn on_block_place(
        &self,
        player: &ServerPlayerContext,
        x: i32,
        y: i32,
        z: i32,
        state_id: u16,
        actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        let Ok((mut store, instance)) =
            self.build_store_and_instance(player.entity_id, player.username.clone())
        else {
            return ServerEventResult::Allow;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_block_place")
            && let Ok(typed) = func.typed::<(i32, i32, i32, i32), i32>(&store)
            && let Ok(res) = typed.call(&mut store, (x, y, z, i32::from(state_id)))
        {
            actions.extend(store.into_data().actions);
            return if res == ACTION_CANCEL {
                ServerEventResult::Cancel
            } else {
                ServerEventResult::Allow
            };
        }
        ServerEventResult::Allow
    }

    fn on_player_damage(
        &self,
        player: &ServerPlayerContext,
        amount: f32,
        actions: &mut Vec<ServerCommandAction>,
    ) -> ServerEventResult {
        let Ok((mut store, instance)) =
            self.build_store_and_instance(player.entity_id, player.username.clone())
        else {
            return ServerEventResult::Allow;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_player_damage")
            && let Ok(typed) = func.typed::<(i32, f32), i32>(&store)
            && let Ok(res) = typed.call(&mut store, (player.entity_id, amount))
        {
            actions.extend(store.into_data().actions);
            return if res == ACTION_CANCEL {
                ServerEventResult::Cancel
            } else {
                ServerEventResult::Allow
            };
        }
        ServerEventResult::Allow
    }

    fn on_server_tick(&self, tick: u64, actions: &mut Vec<ServerCommandAction>) {
        let Ok((mut store, instance)) = self.build_store_and_instance(0, String::new()) else {
            return;
        };
        if let Some(func) = instance.get_func(&store, "radon_on_server_tick")
            && let Ok(typed) = func.typed::<i64, ()>(&store)
            && typed.call(&mut store, tick as i64).is_ok()
        {
            actions.extend(store.into_data().actions);
        }
    }
}

/// Manages all registered native and `.wasm` [`ServerRadon`] plugins.
#[derive(Default)]
pub struct ServerRadonManager {
    radons: Vec<Arc<dyn ServerRadon>>,
}

impl ServerRadonManager {
    /// Creates an empty manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `*.wasm` server radons from `dir` if it exists.
    #[must_use]
    pub fn load_dir(dir: &Path) -> Self {
        let mut mgr = Self::new();
        let Ok(entries) = fs::read_dir(dir) else {
            return mgr;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("wasm")
                && let Ok(bytes) = fs::read(&path)
            {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("wasm-radon");
                let config = dir
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join("radonfigs")
                    .join(format!("{name}.toml"));
                let config_str = fs::read_to_string(config).unwrap_or_default();
                match WasmServerRadon::from_bytes_with_config(name, &bytes, config_str) {
                    Ok(radon) => {
                        info!(radon = name, "loaded WASM server radon");
                        mgr.register(Arc::new(radon));
                    }
                    Err(e) => {
                        warn!(radon = name, error = %e, "failed to load WASM server radon");
                    }
                }
            }
        }
        mgr
    }

    /// Registers a native or WASM [`ServerRadon`].
    pub fn register(&mut self, radon: Arc<dyn ServerRadon>) {
        self.radons.push(radon);
    }

    /// Dispatches `on_player_join`.
    pub fn dispatch_join(&self, player: &ServerPlayerContext) -> Vec<ServerCommandAction> {
        let mut actions = Vec::new();
        for r in &self.radons {
            r.on_player_join(player, &mut actions);
        }
        actions
    }

    /// Dispatches `on_player_quit`.
    pub fn dispatch_quit(&self, player: &ServerPlayerContext) -> Vec<ServerCommandAction> {
        let mut actions = Vec::new();
        for r in &self.radons {
            r.on_player_quit(player, &mut actions);
        }
        actions
    }

    /// Dispatches `on_player_chat`.
    pub fn dispatch_chat(
        &self,
        player: &ServerPlayerContext,
        msg: &str,
    ) -> (ServerEventResult, Vec<ServerCommandAction>) {
        let mut actions = Vec::new();
        for r in &self.radons {
            if r.on_player_chat(player, msg, &mut actions) == ServerEventResult::Cancel {
                return (ServerEventResult::Cancel, actions);
            }
        }
        (ServerEventResult::Allow, actions)
    }

    /// Dispatches `on_block_break`.
    pub fn dispatch_block_break(
        &self,
        player: &ServerPlayerContext,
        x: i32,
        y: i32,
        z: i32,
        state_id: u16,
    ) -> (ServerEventResult, Vec<ServerCommandAction>) {
        let mut actions = Vec::new();
        for r in &self.radons {
            if r.on_block_break(player, x, y, z, state_id, &mut actions)
                == ServerEventResult::Cancel
            {
                return (ServerEventResult::Cancel, actions);
            }
        }
        (ServerEventResult::Allow, actions)
    }

    /// Dispatches `on_block_place`.
    pub fn dispatch_block_place(
        &self,
        player: &ServerPlayerContext,
        x: i32,
        y: i32,
        z: i32,
        state_id: u16,
    ) -> (ServerEventResult, Vec<ServerCommandAction>) {
        let mut actions = Vec::new();
        for r in &self.radons {
            if r.on_block_place(player, x, y, z, state_id, &mut actions)
                == ServerEventResult::Cancel
            {
                return (ServerEventResult::Cancel, actions);
            }
        }
        (ServerEventResult::Allow, actions)
    }

    /// Dispatches `on_player_damage`.
    pub fn dispatch_damage(
        &self,
        player: &ServerPlayerContext,
        amount: f32,
    ) -> (ServerEventResult, Vec<ServerCommandAction>) {
        let mut actions = Vec::new();
        for r in &self.radons {
            if r.on_player_damage(player, amount, &mut actions) == ServerEventResult::Cancel {
                return (ServerEventResult::Cancel, actions);
            }
        }
        (ServerEventResult::Allow, actions)
    }

    /// Dispatches `on_server_tick`.
    pub fn dispatch_tick(&self, tick: u64) -> Vec<ServerCommandAction> {
        let mut actions = Vec::new();
        for r in &self.radons {
            r.on_server_tick(tick, &mut actions);
        }
        actions
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn _allow_const_check() -> i32 {
        ACTION_ALLOW
    }
}

fn write_guest_str(
    caller: &mut Caller<'_, WasmServerState>,
    ptr: i32,
    max_len: i32,
    s: &str,
) -> i32 {
    if ptr < 0 || max_len <= 0 {
        return 0;
    }
    let Some(wasmi::Extern::Memory(mem)) = caller.get_export("memory") else {
        return 0;
    };
    let bytes = s.as_bytes();
    let n = bytes.len().min(max_len as usize);
    if mem.write(&mut *caller, ptr as usize, &bytes[..n]).is_err() {
        return 0;
    }
    n as i32
}

fn read_guest_str(caller: &mut Caller<'_, WasmServerState>, ptr: i32, len: i32) -> Option<String> {
    if ptr < 0 || !(0..=4096).contains(&len) {
        return None;
    }
    let wasmi::Extern::Memory(mem) = caller.get_export("memory")? else {
        return None;
    };
    let mut buf = vec![0u8; len as usize];
    mem.read(&*caller, ptr as usize, &mut buf).ok()?;
    String::from_utf8(buf).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a minimal valid WASM module:
    /// - imports `radbyte.rad_send_chat(i32, i32)`
    /// - exports memory (1 page) with `"hi"` at offset 0
    /// - exports `radon_on_player_join(i32)` -> calls `rad_send_chat(0, 2)`
    /// - exports `radon_on_block_break(i32, i32, i32, i32) -> i32` -> returns `1` (`ACTION_CANCEL`) when `x == 0`
    fn minimal_test_wasm() -> Vec<u8> {
        #[rustfmt::skip]
        let wasm: Vec<u8> = vec![
            0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00, // magic + version
            // Type section (id=1): 3 types
            0x01, 0x10, 0x03,
                0x60, 0x02, 0x7F, 0x7F, 0x00,                   // type 0: (i32, i32) -> ()
                0x60, 0x01, 0x7F, 0x00,                         // type 1: (i32) -> ()
                0x60, 0x04, 0x7F, 0x7F, 0x7F, 0x7F, 0x01, 0x7F, // type 2: (i32,i32,i32,i32) -> i32
            // Import section (id=2): radbyte.rad_send_chat : type 0
            0x02, 0x19, 0x01,
                0x07, b'r', b'a', b'd', b'b', b'y', b't', b'e',
                0x0D, b'r', b'a', b'd', b'_', b's', b'e', b'n', b'd', b'_', b'c', b'h', b'a', b't',
                0x00, 0x00,
            // Function section (id=3): func 1 -> type 1, func 2 -> type 2
            0x03, 0x03, 0x02, 0x01, 0x02,
            // Memory section (id=5): 1 page min
            0x05, 0x03, 0x01, 0x00, 0x01,
            // Export section (id=7): memory, radon_on_player_join, radon_on_block_break
            0x07, 0x39, 0x03,
                0x06, b'm', b'e', b'm', b'o', b'r', b'y', 0x02, 0x00,
                0x14, b'r', b'a', b'd', b'o', b'n', b'_', b'o', b'n', b'_', b'p', b'l', b'a', b'y', b'e', b'r', b'_', b'j', b'o', b'i', b'n', 0x00, 0x01,
                0x14, b'r', b'a', b'd', b'o', b'n', b'_', b'o', b'n', b'_', b'b', b'l', b'o', b'c', b'k', b'_', b'b', b'r', b'e', b'a', b'k', 0x00, 0x02,
            // Code section (id=10):
            0x0A, 0x15, 0x02,
                // body 1 (radon_on_player_join): i32.const 0; i32.const 2; call 0; end
                0x08, 0x00, 0x41, 0x00, 0x41, 0x02, 0x10, 0x00, 0x0B,
                // body 2 (radon_on_block_break): local.get 0; i32.eqz; end (returns 1 if x==0 else 0)
                0x0A, 0x00, 0x20, 0x00, 0x45, 0x04, 0x7F, 0x41, 0x01, 0x05, 0x41, 0x00, 0x0B, 0x0B,
            // Data section (id=11): offset 0 -> "hi"
            0x0B, 0x08, 0x01, 0x00, 0x41, 0x00, 0x0B, 0x02, b'h', b'i',
        ];
        // Fix section lengths? Let's verify exact lengths:
        // Type section: 1+16? Wait: 0x60,0x02,0x7F,0x7F,0x00 (5) + 0x60,0x01,0x7F,0x00 (4) + 0x60,0x04,0x7F,0x7F,0x7F,0x7F,0x01,0x7F (8) + count(1) = 18 = 0x12!
        // Export section: 1 + (1+6+2=9) + (1+20+2=23) + (1+20+2=23) = 56 = 0x38!
        // Code section: 1 + (1+8=9) + (1+12=13) = 23 = 0x17!
        let _ = wasm;
        // Build with exact helper so byte lengths are always 100% exact!
        let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let mut push_sec = |id: u8, body: &[u8]| {
            out.push(id);
            out.push(body.len() as u8);
            out.extend_from_slice(body);
        };
        // 1: Type
        push_sec(
            1,
            &[
                3, 0x60, 2, 0x7F, 0x7F, 0x00, 0x60, 1, 0x7F, 0x00, 0x60, 4, 0x7F, 0x7F, 0x7F, 0x7F,
                1, 0x7F,
            ],
        );
        // 2: Import
        let mut imp = vec![1, 7];
        imp.extend_from_slice(b"radbyte");
        imp.push(13);
        imp.extend_from_slice(b"rad_send_chat");
        imp.extend_from_slice(&[0x00, 0x00]);
        push_sec(2, &imp);
        // 3: Function
        push_sec(3, &[2, 1, 2]);
        // 5: Memory
        push_sec(5, &[1, 0, 1]);
        // 7: Export
        let mut exp = vec![3];
        exp.push(6);
        exp.extend_from_slice(b"memory");
        exp.extend_from_slice(&[0x02, 0x00]);
        exp.push(20);
        exp.extend_from_slice(b"radon_on_player_join");
        exp.extend_from_slice(&[0x00, 0x01]);
        exp.push(20);
        exp.extend_from_slice(b"radon_on_block_break");
        exp.extend_from_slice(&[0x00, 0x02]);
        push_sec(7, &exp);
        // 10: Code
        let b1 = [0x00, 0x41, 0x00, 0x41, 0x02, 0x10, 0x00, 0x0B];
        let b2 = [0x00, 0x20, 0x00, 0x45, 0x0B]; // i32.eqz -> 1 if x == 0 else 0
        let mut code = vec![2, b1.len() as u8];
        code.extend_from_slice(&b1);
        code.push(b2.len() as u8);
        code.extend_from_slice(&b2);
        push_sec(10, &code);
        // 11: Data
        push_sec(11, &[1, 0, 0x41, 0x00, 0x0B, 2, b'h', b'i']);
        out
    }

    #[test]
    fn wasm_server_radon_handles_join_and_block_break() {
        let radon = WasmServerRadon::from_bytes("test-guard", &minimal_test_wasm()).unwrap();
        let mut mgr = ServerRadonManager::new();
        mgr.register(Arc::new(radon));

        let ctx = ServerPlayerContext {
            entity_id: 7,
            username: "Steve".into(),
            x: 0.5,
            y: 64.0,
            z: 0.5,
            hp: 20.0,
        };

        let join_actions = mgr.dispatch_join(&ctx);
        assert_eq!(
            join_actions,
            vec![ServerCommandAction::SendMessage {
                entity_id: 7,
                message: "hi".into()
            }]
        );

        let (res_protected, _) = mgr.dispatch_block_break(&ctx, 0, 63, 0, 9);
        assert_eq!(res_protected, ServerEventResult::Cancel);

        let (res_allowed, _) = mgr.dispatch_block_break(&ctx, 5, 63, 0, 9);
        assert_eq!(res_allowed, ServerEventResult::Allow);
    }

    #[test]
    fn wasm_server_radon_enforces_memory_limits_and_reads_config() {
        // 1. Over-limit memory (300 pages = 19.2 MiB > 16 MiB cap) fails build_store_and_instance
        let mut oom = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        oom.extend_from_slice(&[5, 4, 1, 0, 0xAC, 0x02]); // memory min=300 pages
        let oom_radon = WasmServerRadon::from_bytes("oom-srv", &oom).unwrap();
        assert!(
            oom_radon
                .build_store_and_instance(1, "Steve".into())
                .is_err()
        );

        // 2. Module importing rad_get_config(ptr, max_len) -> i32 and rad_send_chat(ptr, len)
        let mut out = vec![0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00];
        let mut push_sec = |id: u8, body: &[u8]| {
            out.push(id);
            out.push(body.len() as u8);
            out.extend_from_slice(body);
        };
        // Types: 0: (i32,i32)->i32, 1: (i32,i32)->(), 2: (i32)->()
        push_sec(
            1,
            &[
                3, 0x60, 2, 0x7F, 0x7F, 1, 0x7F, 0x60, 2, 0x7F, 0x7F, 0x00, 0x60, 1, 0x7F, 0x00,
            ],
        );
        let mut imp = vec![2, 7];
        imp.extend_from_slice(b"radbyte");
        imp.push(14);
        imp.extend_from_slice(b"rad_get_config");
        imp.extend_from_slice(&[0x00, 0x00, 7]);
        imp.extend_from_slice(b"radbyte");
        imp.push(13);
        imp.extend_from_slice(b"rad_send_chat");
        imp.extend_from_slice(&[0x00, 0x01]);
        push_sec(2, &imp);
        push_sec(3, &[1, 2]); // func 2 -> type 2
        push_sec(5, &[1, 0, 1]); // memory min=1
        let mut exp = vec![2, 6];
        exp.extend_from_slice(b"memory");
        exp.extend_from_slice(&[0x02, 0x00, 20]);
        exp.extend_from_slice(b"radon_on_player_join");
        exp.extend_from_slice(&[0x00, 0x02]);
        push_sec(7, &exp);
        // code: call rad_get_config(0, 32) -> n; call rad_send_chat(0, n)
        push_sec(
            10,
            &[1, 13, 0, 0x41, 0, 0x41, 32, 0x10, 0, 0x41, 0, 0x10, 1, 0x0B],
        );

        // Wait: stack order for rad_send_chat(0, n): push 0 FIRST, then call rad_get_config(0, 32)!
        out.clear();
        out.extend_from_slice(&[0x00, 0x61, 0x73, 0x6D, 0x01, 0x00, 0x00, 0x00]);
        let mut push_sec2 = |id: u8, body: &[u8]| {
            out.push(id);
            out.push(body.len() as u8);
            out.extend_from_slice(body);
        };
        push_sec2(
            1,
            &[
                3, 0x60, 2, 0x7F, 0x7F, 1, 0x7F, 0x60, 2, 0x7F, 0x7F, 0x00, 0x60, 1, 0x7F, 0x00,
            ],
        );
        push_sec2(2, &imp);
        push_sec2(3, &[1, 2]);
        push_sec2(5, &[1, 0, 1]);
        push_sec2(7, &exp);
        // i32.const 0 (ptr); i32.const 0; i32.const 32; call 0 (len); call 1 (rad_send_chat); end
        push_sec2(
            10,
            &[1, 12, 0, 0x41, 0, 0x41, 0, 0x41, 32, 0x10, 0, 0x10, 1, 0x0B],
        );

        let cfg_radon =
            WasmServerRadon::from_bytes_with_config("cfg-srv", &out, "hello=1".into()).unwrap();
        let mut mgr = ServerRadonManager::new();
        mgr.register(Arc::new(cfg_radon));
        let ctx = ServerPlayerContext {
            entity_id: 9,
            username: "Steve".into(),
            x: 0.5,
            y: 64.0,
            z: 0.5,
            hp: 20.0,
        };
        assert_eq!(
            mgr.dispatch_join(&ctx),
            vec![ServerCommandAction::SendMessage {
                entity_id: 9,
                message: "hello=1".into(),
            }]
        );
    }

    #[test]
    fn compiled_wasm32_guest_radon_works() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let guest_manifest = root
            .join("examples")
            .join("radon-guest-example")
            .join("Cargo.toml");
        let wasm_path = root
            .join("examples")
            .join("radon-guest-example")
            .join("target")
            .join("wasm32-unknown-unknown")
            .join("release")
            .join("radon_guest_example.wasm");

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
        let radon = WasmServerRadon::from_bytes_with_config(
            "radon-guest-example",
            &wasm_bytes,
            "protect_x0=true".into(),
        )
        .unwrap();
        let mut mgr = ServerRadonManager::new();
        mgr.register(Arc::new(radon));

        let ctx = ServerPlayerContext {
            entity_id: 7,
            username: "Alice".into(),
            x: 0.5,
            y: 64.0,
            z: 0.5,
            hp: 20.0,
        };
        assert_eq!(
            mgr.dispatch_join(&ctx),
            vec![ServerCommandAction::SendMessage {
                entity_id: 7,
                message: "Welcome Alice [protect_x0=true]".into(),
            }]
        );

        let (res, actions) = mgr.dispatch_block_break(&ctx, 0, 64, 0, 1);
        assert_eq!(res, ServerEventResult::Cancel);
        assert_eq!(
            actions,
            vec![ServerCommandAction::SendMessage {
                entity_id: 7,
                message: "protected by compiled wasm radon".into(),
            }]
        );

        let (res_allowed, actions_allowed) = mgr.dispatch_block_break(&ctx, 5, 64, 0, 1);
        assert_eq!(res_allowed, ServerEventResult::Allow);
        assert!(actions_allowed.is_empty());
    }
}
