//! Configuration (`rad.properties`) for `RadServer`.

use std::net::SocketAddr;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Current RadServer configuration schema version.
pub const SERVER_CONFIG_VERSION: u32 = 1;

/// Forwarding mode expected from the upstream proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ServerForwardingMode {
    /// Direct connections (no proxy forwarding verification).
    #[default]
    None,
    /// BungeeCord legacy `Handshake.server_address` (`\0ip\0uuid\0properties`).
    Legacy,
    /// BungeeGuard token inside `Handshake.server_address` properties.
    BungeeGuard,
    /// Velocity Modern Forwarding (`velocity:player_info` HMAC-SHA256).
    Modern,
}

/// Serializable RadServer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfigFile {
    /// Configuration schema version (currently `1`).
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    /// Address and port to listen on (e.g., `"127.0.0.1:25567"`).
    pub bind_addr: String,
    /// Expected proxy forwarding mode (`"modern"`, `"legacy"`, `"bungeeguard"`, or `"none"`).
    pub forwarding_mode: ServerForwardingMode,
    /// Shared secret for `modern` (Velocity) or `bungeeguard` forwarding.
    pub forwarding_secret: String,
    /// Server list MOTD.
    pub motd: String,
    /// Maximum players reported.
    pub max_players: i32,
    /// View distance in chunks (`2..=16`).
    pub view_distance: i32,
    /// Simulation distance in chunks (`2..=32`). When omitted in an existing config, inherits `view_distance`.
    #[serde(default)]
    pub simulation_distance: Option<i32>,
    /// Target ticks per second (default `20`).
    pub tps: u32,
    /// Default game mode (properties use survival, creative, adventure or spectator).
    pub game_mode: u8,
    /// Spawn X coordinate.
    pub spawn_x: f64,
    /// Spawn Y coordinate.
    pub spawn_y: f64,
    /// Spawn Z coordinate.
    pub spawn_z: f64,
    /// `minecraft:brand` string.
    pub brand: String,
    /// Optional welcome chat message on join.
    pub welcome_message: Option<String>,
    /// Optional `bungeecord:main` `Connect` target for `/lobby` / `/hub`.
    pub lobby_server: Option<String>,
    /// Maximum allowed uncompressed packet size in bytes.
    pub max_packet_size: usize,
    /// Timeout for each login packet, in seconds.
    pub login_timeout_secs: u64,
    /// Timeout for the proxy forwarding response, in seconds.
    pub forwarding_timeout_secs: u64,
    /// Deadline for client configuration, in seconds.
    pub configuration_timeout_secs: u64,
    /// Timeout while waiting for a play packet, in seconds.
    pub play_read_timeout_secs: u64,
    /// Usernames (case-insensitive) allowed to run admin commands (`/summon`, `/heal`, `/save`).
    #[serde(default)]
    pub admins: Vec<String>,
    /// Maximum number of active mobs in the world.
    #[serde(default = "default_max_mobs")]
    pub max_mobs: usize,
    /// Periodic autosave interval in seconds (`0` disables autosave).
    #[serde(default = "default_autosave_interval_secs")]
    pub autosave_interval_secs: u64,
    /// World generator type (`"flat"` or `"seeded"`).
    #[serde(default = "default_world_type")]
    pub world_type: String,
    /// World generation seed used when `world_type = "seeded"`.
    #[serde(default)]
    pub world_seed: i64,
    /// World directory / level name (vanilla `level-name`, default `"world"`).
    #[serde(default = "default_level_name")]
    pub level_name: String,
    /// Game difficulty (`"peaceful"`/`0`, `"easy"`/`1`, `"normal"`/`2`, `"hard"`/`3`; default `1` = easy).
    #[serde(
        default = "default_difficulty",
        deserialize_with = "deserialize_difficulty"
    )]
    pub difficulty: u8,
    /// Whether player-versus-player combat is enabled (default `true`).
    #[serde(default = "default_true")]
    pub pvp: bool,
    /// Force players into the server default `game_mode` on every join (`force-gamemode`, default `false`).
    #[serde(default)]
    pub force_gamemode: bool,
    /// Optional startup whitelist toggle (`white-list`). When omitted, preserves `world/access_control.json`.
    #[serde(default)]
    pub white_list: Option<bool>,
    /// Kick online non-whitelisted non-op players when whitelist is enabled or reloaded (`enforce-whitelist`).
    #[serde(default)]
    pub enforce_whitelist: bool,
    /// Default operator permission level (`1..=4`, vanilla `op-permission-level`, default `4`).
    #[serde(default = "default_op_permission_level")]
    pub op_permission_level: u8,
    /// Radius in blocks around spawn protected from non-op block edits (`0` disables, default `16`).
    #[serde(default = "default_spawn_protection")]
    pub spawn_protection: i32,
    /// Respond to Server List Ping status requests (`enable-status`, default `true`).
    #[serde(default = "default_true")]
    pub enable_status: bool,
    /// Hide online player count/sample in Server List Ping (`hide-online-players`, default `false`).
    #[serde(default)]
    pub hide_online_players: bool,
    /// Enable Hardcore mode (`hardcore`, default `false`; forces difficulty to Hard and switches dead players to Spectator).
    #[serde(default)]
    pub hardcore: bool,
    /// Kick players after this many idle minutes (`player-idle-timeout`, `0` disables, max `1440`).
    #[serde(default)]
    pub player_idle_timeout: i32,
    /// Maximum X/Z world radius in blocks (`max-world-size`, `1..=29999984`, default `29999984`).
    #[serde(default = "default_max_world_size")]
    pub max_world_size: i32,
}

fn default_config_version() -> u32 {
    SERVER_CONFIG_VERSION
}

fn default_max_mobs() -> usize {
    256
}

fn default_autosave_interval_secs() -> u64 {
    300
}

fn default_world_type() -> String {
    "flat".into()
}

fn default_level_name() -> String {
    "world".into()
}

fn default_difficulty() -> u8 {
    1
}

fn default_true() -> bool {
    true
}

fn default_op_permission_level() -> u8 {
    4
}

fn default_spawn_protection() -> i32 {
    16
}

fn default_max_world_size() -> i32 {
    29_999_984
}

fn deserialize_difficulty<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct DiffVisitor;
    impl serde::de::Visitor<'_> for DiffVisitor {
        type Value = u8;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("difficulty 0..=3 or \"peaceful\"|\"easy\"|\"normal\"|\"hard\"")
        }

        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<u8, E> {
            u8::try_from(v)
                .ok()
                .filter(|&d| d <= 3)
                .ok_or_else(|| E::custom(format!("invalid difficulty {v}; expected 0..=3")))
        }

        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<u8, E> {
            u8::try_from(v)
                .ok()
                .filter(|&d| d <= 3)
                .ok_or_else(|| E::custom(format!("invalid difficulty {v}; expected 0..=3")))
        }

        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<u8, E> {
            match v.trim().to_ascii_lowercase().as_str() {
                "0" | "peaceful" => Ok(0),
                "1" | "easy" => Ok(1),
                "2" | "normal" => Ok(2),
                "3" | "hard" => Ok(3),
                other => Err(E::custom(format!(
                    "invalid difficulty '{other}'; expected peaceful, easy, normal, or hard"
                ))),
            }
        }
    }
    deserializer.deserialize_any(DiffVisitor)
}

impl Default for ServerConfigFile {
    fn default() -> Self {
        Self {
            config_version: SERVER_CONFIG_VERSION,
            bind_addr: "127.0.0.1:25567".into(),
            forwarding_mode: ServerForwardingMode::None,
            forwarding_secret: "change-me-radbyte-secret".into(),
            motd: "RadServer 26.3 - Hub".into(),
            max_players: 200,
            view_distance: 12,
            simulation_distance: Some(8),
            tps: 20,
            game_mode: 2,
            spawn_x: 0.5,
            spawn_y: 64.0,
            spawn_z: 0.5,
            brand: "RadServer".into(),
            welcome_message: Some("Welcome to RadServer 26.3!".into()),
            lobby_server: Some("limbo".into()),
            max_packet_size: 2 * 1024 * 1024,
            login_timeout_secs: 10,
            forwarding_timeout_secs: 5,
            configuration_timeout_secs: 10,
            play_read_timeout_secs: 35,
            admins: Vec::new(),
            max_mobs: 256,
            autosave_interval_secs: 300,
            world_type: default_world_type(),
            world_seed: 0,
            level_name: default_level_name(),
            difficulty: default_difficulty(),
            pvp: true,
            force_gamemode: false,
            white_list: Some(false),
            enforce_whitelist: false,
            op_permission_level: default_op_permission_level(),
            spawn_protection: default_spawn_protection(),
            enable_status: true,
            hide_online_players: false,
            hardcore: false,
            player_idle_timeout: 0,
            max_world_size: default_max_world_size(),
        }
    }
}

/// Validated runtime configuration for `RadServer`.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Parsed socket address to bind.
    pub bind_addr: SocketAddr,
    /// Expected forwarding mode.
    pub forwarding_mode: ServerForwardingMode,
    /// Shared secret bytes.
    pub forwarding_secret: Vec<u8>,
    /// Server list MOTD.
    pub motd: String,
    /// Maximum players reported.
    pub max_players: i32,
    /// View distance in chunks (`2..=16`).
    pub view_distance: i32,
    /// Simulation distance in chunks (`2..=32`).
    pub simulation_distance: i32,
    /// Target ticks per second.
    pub tps: u32,
    /// Default player game mode (`0..=3`).
    pub game_mode: u8,
    /// Spawn X coordinate.
    pub spawn_x: f64,
    /// Spawn Y coordinate.
    pub spawn_y: f64,
    /// Spawn Z coordinate.
    pub spawn_z: f64,
    /// `minecraft:brand` string.
    pub brand: String,
    /// Optional welcome chat message.
    pub welcome_message: Option<String>,
    /// Optional `bungeecord:main` target for `/lobby` / `/hub`.
    pub lobby_server: Option<String>,
    /// Maximum uncompressed packet size.
    pub max_packet_size: usize,
    /// Login packet timeout in seconds.
    pub login_timeout_secs: u64,
    /// Proxy forwarding timeout in seconds.
    pub forwarding_timeout_secs: u64,
    /// Configuration deadline in seconds.
    pub configuration_timeout_secs: u64,
    /// Play packet read timeout in seconds.
    pub play_read_timeout_secs: u64,
    /// Admin usernames (lowercase).
    pub admins: std::collections::HashSet<String>,
    /// Maximum active mobs allowed in the world.
    pub max_mobs: usize,
    /// Periodic autosave interval in seconds (`0` disables autosave).
    pub autosave_interval_secs: u64,
    /// World generator type (`"flat"` or `"seeded"`).
    pub world_type: String,
    /// World generation seed.
    pub world_seed: i64,
    /// World directory / level name (`level-name`).
    pub level_name: String,
    /// Game difficulty (`0`=peaceful, `1`=easy, `2`=normal, `3`=hard).
    pub difficulty: u8,
    /// Whether PvP combat is enabled.
    pub pvp: bool,
    /// Force default `game_mode` on join.
    pub force_gamemode: bool,
    /// Optional startup whitelist override (`white-list`).
    pub white_list: Option<bool>,
    /// Kick online non-whitelisted non-ops when whitelist is enabled or reloaded.
    pub enforce_whitelist: bool,
    /// Default operator permission level (`1..=4`).
    pub op_permission_level: u8,
    /// Spawn protection radius in blocks (`0` disables).
    pub spawn_protection: i32,
    /// Whether Server List Ping status responses are enabled.
    pub enable_status: bool,
    /// Whether online player count is hidden in Server List Ping.
    pub hide_online_players: bool,
    /// Whether Hardcore mode is enabled (forces difficulty=3 and Spectator on death).
    pub hardcore: bool,
    /// Idle kick timeout in minutes (`0` disables, clamped to `0..=1440`).
    pub player_idle_timeout: i32,
    /// Maximum X/Z world radius in blocks (`1..=29999984`).
    pub max_world_size: i32,
}

impl ServerConfig {
    /// Builds validated runtime config from a [`ServerConfigFile`].
    pub fn from_file_struct(file: ServerConfigFile) -> Result<Self, String> {
        if matches!(
            file.forwarding_mode,
            ServerForwardingMode::Modern | ServerForwardingMode::BungeeGuard
        ) && file.forwarding_secret.is_empty()
        {
            return Err("forwarding secret must not be empty".to_owned());
        }
        if !file.spawn_x.is_finite() || !file.spawn_y.is_finite() || !file.spawn_z.is_finite() {
            return Err("spawn coordinates must be finite".to_owned());
        }
        let bind_addr: SocketAddr = file
            .bind_addr
            .parse()
            .map_err(|e| format!("invalid bind_addr '{}': {e}", file.bind_addr))?;
        let level_name = file.level_name.trim().to_owned();
        if level_name.is_empty()
            || level_name == "."
            || level_name == ".."
            || level_name.contains("..")
            || level_name.contains('/')
            || level_name.contains('\\')
            || level_name.contains(':')
            || level_name.contains('\0')
        {
            return Err(format!(
                "invalid level_name '{}': must be a simple relative directory name",
                file.level_name
            ));
        }
        if !(1..=4).contains(&file.op_permission_level) {
            return Err(format!(
                "invalid op_permission_level {}: expected 1..=4",
                file.op_permission_level
            ));
        }
        if file.spawn_protection < 0 {
            return Err(format!(
                "invalid spawn_protection {}: expected >= 0",
                file.spawn_protection
            ));
        }
        if file.difficulty > 3 {
            return Err(format!(
                "invalid difficulty {}: expected 0..=3",
                file.difficulty
            ));
        }
        for (name, value) in [
            ("login-timeout-secs", file.login_timeout_secs),
            ("forwarding-timeout-secs", file.forwarding_timeout_secs),
            (
                "configuration-timeout-secs",
                file.configuration_timeout_secs,
            ),
            ("play-read-timeout-secs", file.play_read_timeout_secs),
        ] {
            if !(1..=300).contains(&value) {
                return Err(format!("invalid {name}: expected 1..=300 seconds"));
            }
        }
        let view_distance = file.view_distance.clamp(2, 32);
        let simulation_distance = match file.simulation_distance {
            Some(val) => {
                if !(2..=32).contains(&val) {
                    return Err(format!(
                        "invalid simulation_distance {val}: expected 2..=32"
                    ));
                }
                val
            }
            None => view_distance,
        };
        if file.player_idle_timeout < 0 {
            return Err(format!(
                "invalid player_idle_timeout {}: expected >= 0",
                file.player_idle_timeout
            ));
        }
        let difficulty = if file.hardcore { 3 } else { file.difficulty };
        Ok(Self {
            bind_addr,
            forwarding_mode: file.forwarding_mode,
            forwarding_secret: file.forwarding_secret.into_bytes(),
            motd: file.motd,
            max_players: file.max_players.max(1),
            view_distance,
            simulation_distance,
            tps: file.tps.clamp(1, 100),
            game_mode: file.game_mode.min(3),
            spawn_x: file.spawn_x,
            spawn_y: file.spawn_y,
            spawn_z: file.spawn_z,
            brand: file.brand,
            welcome_message: file.welcome_message,
            lobby_server: file.lobby_server,
            max_packet_size: file.max_packet_size.max(4096),
            login_timeout_secs: file.login_timeout_secs,
            forwarding_timeout_secs: file.forwarding_timeout_secs,
            configuration_timeout_secs: file.configuration_timeout_secs,
            play_read_timeout_secs: file.play_read_timeout_secs,
            admins: file
                .admins
                .into_iter()
                .map(|s| s.to_ascii_lowercase())
                .collect(),
            max_mobs: file.max_mobs.clamp(1, 4096),
            autosave_interval_secs: file.autosave_interval_secs,
            world_type: file.world_type,
            world_seed: file.world_seed,
            level_name,
            difficulty,
            pvp: file.pvp,
            force_gamemode: file.force_gamemode,
            white_list: file.white_list,
            enforce_whitelist: file.enforce_whitelist,
            op_permission_level: file.op_permission_level,
            spawn_protection: file.spawn_protection,
            enable_status: file.enable_status,
            hide_online_players: file.hide_online_players,
            hardcore: file.hardcore,
            player_idle_timeout: file.player_idle_timeout.clamp(0, 1440),
            max_world_size: file.max_world_size.clamp(1, 29_999_984),
        })
    }

    /// Loads properties, migrating a sibling legacy TOML file when necessary.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        // Explicit legacy --config paths retain their previous meaning.
        if path.extension().is_some_and(|ext| ext == "toml") {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let file: ServerConfigFile = toml::from_str(&raw)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
            return Self::from_file_struct(file);
        }
        if !path.exists() {
            let legacy = path.with_file_name("radserver.toml");
            let file = if legacy.exists() {
                let raw = std::fs::read_to_string(&legacy)
                    .map_err(|e| format!("failed to read {}: {e}", legacy.display()))?;
                toml::from_str::<ServerConfigFile>(&raw)
                    .map_err(|e| format!("failed to parse {}: {e}", legacy.display()))?
            } else {
                ServerConfigFile::default()
            };
            let contents = write_properties(&file)?;
            let extra = write_extra_properties(&file)?;
            let extra_path = extra_config_path(path)?;
            // An existing extra file takes effect even when the main file is new.
            let effective_extra = if extra_path.exists() {
                std::fs::read_to_string(&extra_path)
                    .map_err(|e| format!("failed to read {}: {e}", extra_path.display()))?
            } else {
                extra.clone()
            };
            let validated = Self::from_file_struct(read_properties(&format!(
                "{contents}\n{effective_extra}"
            ))?)?;
            if !extra_path.exists() {
                if let Some(parent) = extra_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
                }
                std::fs::write(&extra_path, extra)
                    .map_err(|e| format!("failed to create {}: {e}", extra_path.display()))?;
            }
            use std::io::Write;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map_err(|e| format!("failed to create {}: {e}", path.display()))?;
            output
                .write_all(contents.as_bytes())
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
            return Ok(validated);
        }
        let mut raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let extra_path = extra_config_path(path)?;
        if extra_path.exists() {
            raw.push('\n');
            raw.push_str(
                &std::fs::read_to_string(&extra_path)
                    .map_err(|e| format!("failed to read {}: {e}", extra_path.display()))?,
            );
        }
        Self::from_file_struct(
            read_properties(&raw)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?,
        )
    }
}

fn extra_config_path(path: &Path) -> Result<std::path::PathBuf, String> {
    let legacy = path.with_file_name("radserver-extra.properties");
    let current = path
        .with_file_name("config")
        .join("radserver-extra.properties");
    match (legacy.exists(), current.exists()) {
        (true, true) => Err(format!(
            "both {} and {} exist; remove the duplicate after checking its values",
            legacy.display(),
            current.display()
        )),
        (true, false) => Ok(legacy),
        _ => Ok(current),
    }
}

// Properties values are UTF-8. Backslashes escape backslash, newline, carriage
// return and tab; other characters, including ':' and '=', are literal values.
fn escape_property(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    let leading = escaped.len() - escaped.trim_start_matches(' ').len();
    let trailing = escaped.trim_start_matches(' ').len() - escaped.trim_matches(' ').len();
    if leading == escaped.len() {
        return "\\u0020".repeat(leading);
    }
    format!(
        "{}{}{}",
        "\\u0020".repeat(leading),
        &escaped[leading..escaped.len() - trailing],
        "\\u0020".repeat(trailing)
    )
}
fn unescape_property(value: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some('u') => {
                let digits: String = chars.by_ref().take(4).collect();
                if digits.len() != 4 {
                    return Err("incomplete Unicode escape".into());
                }
                let value =
                    u32::from_str_radix(&digits, 16).map_err(|_| "invalid Unicode escape")?;
                out.push(char::from_u32(value).ok_or("invalid Unicode scalar")?);
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => return Err("trailing escape in property value".into()),
        }
    }
    Ok(out)
}
fn write_properties(file: &ServerConfigFile) -> Result<String, String> {
    write_properties_group(file, false)
}
fn write_extra_properties(file: &ServerConfigFile) -> Result<String, String> {
    write_properties_group(file, true)
}
fn is_extra_property(key: &str) -> bool {
    matches!(
        key,
        "config_version"
            | "forwarding_mode"
            | "forwarding_secret"
            | "tps"
            | "spawn_x"
            | "spawn_y"
            | "spawn_z"
            | "brand"
            | "welcome_message"
            | "lobby_server"
            | "max_packet_size"
            | "login_timeout_secs"
            | "forwarding_timeout_secs"
            | "configuration_timeout_secs"
            | "play_read_timeout_secs"
            | "admins"
            | "max_mobs"
            | "autosave_interval_secs"
            | "world_type"
    )
}
fn write_properties_group(file: &ServerConfigFile, extra: bool) -> Result<String, String> {
    let value = serde_json::to_value(file).map_err(|e| e.to_string())?;
    let map = value.as_object().ok_or("invalid configuration")?;
    let mut output = String::from("# RadServer configuration. UTF-8, key=value.\n");
    let sections: &[(&str, &[&str])] = if extra {
        &[
            ("Version", &["config_version"]),
            (
                "Proxy forwarding",
                &["forwarding_mode", "forwarding_secret"],
            ),
            (
                "Network limits and timeouts",
                &[
                    "max_packet_size",
                    "login_timeout_secs",
                    "forwarding_timeout_secs",
                    "configuration_timeout_secs",
                    "play_read_timeout_secs",
                ],
            ),
            ("Server tick and saving", &["tps", "autosave_interval_secs"]),
            (
                "World and spawn",
                &["world_type", "spawn_x", "spawn_y", "spawn_z", "max_mobs"],
            ),
            (
                "Messages and lobby",
                &["brand", "welcome_message", "lobby_server"],
            ),
            ("Legacy administrators", &["admins"]),
        ]
    } else {
        &[
            ("Network", &["bind_addr"]),
            (
                "Server list",
                &[
                    "motd",
                    "enable_status",
                    "hide_online_players",
                    "max_players",
                ],
            ),
            (
                "Access",
                &["white_list", "enforce_whitelist", "op_permission_level"],
            ),
            ("World", &["level_name", "world_seed", "max_world_size"]),
            ("Distances", &["view_distance", "simulation_distance"]),
            (
                "Gameplay",
                &[
                    "game_mode",
                    "force_gamemode",
                    "difficulty",
                    "hardcore",
                    "pvp",
                    "spawn_protection",
                    "player_idle_timeout",
                ],
            ),
        ]
    };
    let mut included = std::collections::HashSet::new();
    for (heading, keys) in sections {
        output.push_str(&format!("\n# {heading}\n"));
        for &key in *keys {
            let value = map
                .get(key)
                .ok_or_else(|| format!("unknown configured property {key}"))?;
            if is_extra_property(key) != extra || !included.insert(key) {
                return Err(format!("invalid configuration section for {key}"));
            }
            if value.is_null() {
                continue;
            }
            if key == "bind_addr" {
                let addr: SocketAddr = file
                    .bind_addr
                    .parse()
                    .map_err(|e| format!("invalid bind address: {e}"))?;
                output.push_str(&format!(
                    "server-ip={}\nserver-port={}\n",
                    addr.ip(),
                    addr.port()
                ));
                continue;
            }
            let text = if key == "game_mode" {
                match file.game_mode {
                0 => "survival",
                1 => "creative",
                2 => "adventure",
                3 => "spectator",
                _ => {
                    return Err(format!(
                        "invalid game-mode {}; expected survival, creative, adventure or spectator",
                        file.game_mode
                    ));
                }
            }
            .to_owned()
            } else if key == "difficulty" {
                match file.difficulty {
                    0 => "peaceful",
                    1 => "easy",
                    2 => "normal",
                    3 => "hard",
                    _ => return Err(format!("invalid difficulty {}", file.difficulty)),
                }
                .to_owned()
            } else {
                match value {
                    serde_json::Value::String(s) => escape_property(s),
                    serde_json::Value::Array(items) => items
                        .iter()
                        .map(|v| v.as_str().ok_or("invalid list item"))
                        .collect::<Result<Vec<_>, _>>()?
                        .join(","),
                    _ => value.to_string(),
                }
            };
            let property = if key == "world_seed" {
                "level-seed".to_owned()
            } else if key == "game_mode" {
                "gamemode".to_owned()
            } else {
                key.replace('_', "-")
            };
            output.push_str(&format!("{property}={text}\n"));
        }
    }
    if let Some(key) = map
        .keys()
        .find(|key| is_extra_property(key) == extra && !included.contains(key.as_str()))
    {
        return Err(format!("property {key} has no configuration section"));
    }
    Ok(output)
}
fn read_properties(raw: &str) -> Result<ServerConfigFile, String> {
    let defaults = serde_json::to_value(ServerConfigFile::default()).map_err(|e| e.to_string())?;
    let defaults = defaults.as_object().ok_or("invalid configuration")?;
    let mut values = serde_json::Map::new();
    let mut server_ip: Option<String> = None;
    let mut server_port: Option<u16> = None;
    for (index, line) in raw.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let (key, raw_value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected key=value", index + 1))?;
        let key = key.trim().replace('-', "_");
        if key == "server_ip" || key == "server_port" {
            let value = unescape_property(raw_value.trim())?;
            if key == "server_ip" {
                if server_ip.replace(value).is_some() {
                    return Err("duplicate server-ip".into());
                }
            } else {
                let port = value.parse::<u16>().map_err(|_| "invalid server-port")?;
                if server_port.replace(port).is_some() {
                    return Err("duplicate server-port".into());
                }
            }
            continue;
        }
        let key = if key == "gamemode" {
            "game_mode".to_owned()
        } else if key == "level_seed" {
            "world_seed".to_owned()
        } else {
            key
        };
        if key.starts_with("auto_update")
            || key.starts_with("auto-update")
            || key == "version"
            || key.starts_with("version-")
            || key == "minecraft-version"
        {
            continue;
        }
        let default = defaults
            .get(&key)
            .ok_or_else(|| format!("line {}: unknown property {key}", index + 1))?;
        if values.contains_key(&key) {
            return Err(format!("line {}: duplicate property {key}", index + 1));
        }
        let value = unescape_property(raw_value.trim())?;
        let parsed = if key == "game_mode" {
            let mode = match value.to_ascii_lowercase().as_str() {
                "survival" | "0" => 0,
                "creative" | "1" => 1,
                "adventure" | "2" => 2,
                "spectator" | "3" => 3,
                _ => {
                    return Err(format!(
                        "invalid game-mode '{value}'; expected survival, creative, adventure or spectator"
                    ));
                }
            };
            serde_json::Value::from(mode)
        } else if key == "difficulty" {
            serde_json::Value::String(value)
        } else if default.is_boolean() || key == "white_list" {
            serde_json::Value::Bool(
                value
                    .parse::<bool>()
                    .map_err(|_| format!("invalid boolean for {key}"))?,
            )
        } else if default.is_number() {
            serde_json::from_str::<serde_json::Value>(&value)
                .map_err(|e| format!("invalid number for {key}: {e}"))?
        } else if default.is_array() {
            serde_json::Value::Array(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
                    .map(|v| serde_json::Value::String(v.to_owned()))
                    .collect(),
            )
        } else {
            serde_json::Value::String(value)
        };
        values.insert(key, parsed);
    }
    if server_ip.is_some() || server_port.is_some() {
        if values.contains_key("bind_addr") {
            return Err("bind-addr conflicts with server-ip/server-port".into());
        }
        let default: SocketAddr = ServerConfigFile::default()
            .bind_addr
            .parse()
            .map_err(|e| format!("invalid default address: {e}"))?;
        let ip = match server_ip.as_deref() {
            Some("") => "0.0.0.0".parse(),
            Some(ip) => ip.parse(),
            None => Ok(default.ip()),
        }
        .map_err(|_| "invalid server-ip")?;
        values.insert(
            "bind_addr".into(),
            serde_json::Value::String(
                SocketAddr::new(ip, server_port.unwrap_or(default.port())).to_string(),
            ),
        );
    }
    serde_json::from_value(serde_json::Value::Object(values)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authenticated_forwarding_requires_nonempty_secret() {
        for mode in [
            ServerForwardingMode::Modern,
            ServerForwardingMode::BungeeGuard,
        ] {
            let file = ServerConfigFile {
                forwarding_mode: mode,
                forwarding_secret: String::new(),
                ..Default::default()
            };
            assert!(ServerConfig::from_file_struct(file).is_err());
        }
    }

    #[test]
    fn rejects_non_finite_spawn_and_null_in_world_directory() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for axis in 0..3 {
                let mut file = ServerConfigFile::default();
                match axis {
                    0 => file.spawn_x = value,
                    1 => file.spawn_y = value,
                    _ => file.spawn_z = value,
                }
                assert!(ServerConfig::from_file_struct(file).is_err());
            }
        }
        let file = ServerConfigFile {
            level_name: "world\0hidden".into(),
            ..Default::default()
        };
        assert!(ServerConfig::from_file_struct(file).is_err());
    }

    #[test]
    fn properties_migrate_legacy_without_overwriting_and_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "rad-properties-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = dir.join("radserver.toml");
        let original = "bind_addr = \"127.0.0.1:25642\"\nmotd = \"Test # = hi\"\ngame_mode = 1\nadmins = [\"Alice\", \"Bob\"]\n";
        std::fs::write(&legacy, original).unwrap();
        let path = dir.join("rad.properties");
        let migrated = ServerConfig::load_or_create(&path).unwrap();
        assert_eq!(migrated.bind_addr.port(), 25642);
        assert_eq!(migrated.motd, "Test # = hi");
        assert_eq!(migrated.game_mode, 1);
        assert!(migrated.admins.contains("alice"));
        let split_reload = ServerConfig::load_or_create(&path).unwrap();
        assert_eq!(split_reload.bind_addr.port(), 25642);
        assert!(split_reload.admins.contains("alice"));
        let current_extra = dir.join("config/radserver-extra.properties");
        let legacy_extra = dir.join("radserver-extra.properties");
        assert!(current_extra.exists());
        std::fs::rename(&current_extra, &legacy_extra).unwrap();
        assert!(
            ServerConfig::load_or_create(&path)
                .unwrap()
                .admins
                .contains("alice")
        );
        std::fs::copy(&legacy_extra, &current_extra).unwrap();
        assert!(
            ServerConfig::load_or_create(&path)
                .unwrap_err()
                .contains("both")
        );
        std::fs::remove_file(&legacy_extra).unwrap();
        assert_eq!(std::fs::read_to_string(&legacy).unwrap(), original);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("server-ip=127.0.0.1"));
        assert!(written.contains("server-port=25642"));
        assert!(written.contains("gamemode=creative"));
        assert!(written.contains("difficulty=easy"));
        std::fs::write(&path, "bind-addr=127.0.0.1:25643\nmotd=Different\n").unwrap();
        let reloaded = ServerConfig::load_or_create(&path).unwrap();
        assert_eq!(reloaded.bind_addr.port(), 25643);
        assert_eq!(reloaded.motd, "Different");
        assert!(read_properties("pvp=maybe").is_err());
        assert!(read_properties("bad-key=1").is_err());
        assert!(read_properties("pvp=true\npvp=false").is_err());
        for (name, expected) in [
            ("survival", 0),
            ("creative", 1),
            ("adventure", 2),
            ("spectator", 3),
            ("1", 1),
            ("CREATIVE", 1),
        ] {
            assert_eq!(
                read_properties(&format!("game-mode={name}"))
                    .unwrap()
                    .game_mode,
                expected
            );
        }
        assert!(read_properties("game-mode=unknown").is_err());
        assert_eq!(read_properties("gamemode=creative").unwrap().game_mode, 1);
        assert!(read_properties("gamemode=creative\ngame-mode=survival").is_err());
        let paper = read_properties("server-ip=\nserver-port=25565\nlevel-seed=42").unwrap();
        assert_eq!(paper.bind_addr, "0.0.0.0:25565");
        assert_eq!(paper.world_seed, 42);
        assert!(read_properties("bind-addr=127.0.0.1:25565\nserver-port=25565").is_err());
        let timeouts = read_properties("login-timeout-secs=20\nforwarding-timeout-secs=6\nconfiguration-timeout-secs=25\nplay-read-timeout-secs=60").unwrap();
        let timeouts = ServerConfig::from_file_struct(timeouts).unwrap();
        assert_eq!(
            (
                timeouts.login_timeout_secs,
                timeouts.forwarding_timeout_secs,
                timeouts.configuration_timeout_secs,
                timeouts.play_read_timeout_secs
            ),
            (20, 6, 25, 60)
        );
        let invalid = read_properties("login-timeout-secs=0").unwrap();
        assert!(ServerConfig::from_file_struct(invalid).is_err());
        let spaced = ServerConfigFile {
            motd: "  Test # = hi  ".into(),
            ..Default::default()
        };
        assert_eq!(
            read_properties(&write_properties(&spaced).unwrap())
                .unwrap()
                .motd,
            spaced.motd
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn default_server_config_validates() {
        let cfg = ServerConfig::from_file_struct(ServerConfigFile::default()).unwrap();
        assert_eq!(cfg.bind_addr, "127.0.0.1:25567".parse().unwrap());
        assert_eq!(cfg.forwarding_mode, ServerForwardingMode::None);
        assert_eq!(cfg.tps, 20);
        assert_eq!(cfg.game_mode, 2);
        assert_eq!(cfg.level_name, "world");
        assert_eq!(cfg.difficulty, 1);
        assert!(cfg.pvp);
        assert!(!cfg.force_gamemode);
        assert_eq!(cfg.white_list, Some(false));
        assert!(!cfg.enforce_whitelist);
        assert_eq!(cfg.op_permission_level, 4);
        assert_eq!(cfg.spawn_protection, 16);
        assert!(cfg.enable_status);
        assert!(!cfg.hide_online_players);
    }

    #[test]
    fn generated_properties_are_grouped_and_preserve_values() {
        let file = ServerConfigFile {
            motd: "Example".into(),
            forwarding_secret: "private value".into(),
            ..Default::default()
        };
        let main = write_properties(&file).unwrap();
        let extra = write_extra_properties(&file).unwrap();
        assert!(main.find("# Network").unwrap() < main.find("# Gameplay").unwrap());
        assert!(
            extra.find("# Proxy forwarding").unwrap() < extra.find("# World and spawn").unwrap()
        );
        let decoded = read_properties(&format!("{main}\n{extra}")).unwrap();
        assert_eq!(decoded.motd, file.motd);
        assert_eq!(decoded.forwarding_secret, file.forwarding_secret);
    }

    #[test]
    fn partial_legacy_toml_and_invalid_configs_are_handled() {
        // Legacy partial config without new 26.3 fields loads cleanly with safe defaults.
        let legacy_toml = r#"
config_version = 1
bind_addr = "127.0.0.1:25577"
motd = "Legacy Hub"
difficulty = "hard"
"#;
        let parsed: ServerConfigFile = toml::from_str(legacy_toml).unwrap();
        let cfg = ServerConfig::from_file_struct(parsed).unwrap();
        assert_eq!(cfg.motd, "Legacy Hub");
        assert_eq!(cfg.difficulty, 3);
        assert_eq!(cfg.white_list, None);
        assert_eq!(cfg.op_permission_level, 4);
        assert_eq!(cfg.level_name, "world");

        // Invalid op_permission_level
        let bad_op = ServerConfigFile {
            op_permission_level: 5,
            ..Default::default()
        };
        assert!(
            ServerConfig::from_file_struct(bad_op)
                .unwrap_err()
                .contains("op_permission_level")
        );

        // Invalid level_name traversal
        let bad_lvl = ServerConfigFile {
            level_name: "../escape".into(),
            ..Default::default()
        };
        assert!(
            ServerConfig::from_file_struct(bad_lvl)
                .unwrap_err()
                .contains("level_name")
        );

        // Invalid spawn_protection
        let bad_sp = ServerConfigFile {
            spawn_protection: -1,
            ..Default::default()
        };
        assert!(
            ServerConfig::from_file_struct(bad_sp)
                .unwrap_err()
                .contains("spawn_protection")
        );

        // Invalid difficulty string
        let bad_diff: Result<ServerConfigFile, _> = toml::from_str("difficulty = \"impossible\"");
        assert!(bad_diff.is_err());
    }

    #[test]
    fn simulation_distance_is_independent_validated_and_backward_compatible() {
        // 1. Independent view-distance and simulation-distance in rad.properties
        let props = "\
server-ip=127.0.0.1
server-port=25567
view-distance=8
simulation-distance=3
";
        let file = read_properties(props).unwrap();
        let cfg = ServerConfig::from_file_struct(file.clone()).unwrap();
        assert_eq!(cfg.view_distance, 8);
        assert_eq!(cfg.simulation_distance, 3);
        let written = write_properties(&file).unwrap();
        assert!(written.contains("# Distances\nview-distance=8\nsimulation-distance=3\n"));

        // 2. Omitted simulation-distance in legacy config inherits view-distance
        let legacy_props = "\
server-ip=127.0.0.1
server-port=25567
view-distance=6
";
        let legacy_cfg =
            ServerConfig::from_file_struct(read_properties(legacy_props).unwrap()).unwrap();
        assert_eq!(legacy_cfg.view_distance, 6);
        assert_eq!(legacy_cfg.simulation_distance, 6);

        // 3. Invalid values rejected
        for bad in ["0", "1", "33", "-2", "abc"] {
            let bad_props = format!("view-distance=4\nsimulation-distance={bad}\n");
            let parsed = read_properties(&bad_props).and_then(ServerConfig::from_file_struct);
            assert!(
                parsed.is_err(),
                "simulation-distance={bad} must fail validation"
            );
        }
    }

    #[test]
    fn hardcore_idle_timeout_and_max_world_size_parse_and_validate() {
        let props = "\
hardcore=true
difficulty=peaceful
player-idle-timeout=15
max-world-size=500
";
        let file = read_properties(props).unwrap();
        let cfg = ServerConfig::from_file_struct(file.clone()).unwrap();
        assert!(cfg.hardcore);
        assert_eq!(
            cfg.difficulty, 3,
            "hardcore must force difficulty to Hard (3)"
        );
        assert_eq!(cfg.player_idle_timeout, 15);
        assert_eq!(cfg.max_world_size, 500);

        let written = write_properties(&file).unwrap();
        assert!(written.contains("hardcore=true"));
        assert!(written.contains("player-idle-timeout=15"));
        assert!(written.contains("max-world-size=500"));

        let bad_idle =
            read_properties("player-idle-timeout=-5\n").and_then(ServerConfig::from_file_struct);
        assert!(bad_idle.is_err());

        let clamped_world = read_properties("max-world-size=999999999\n")
            .and_then(ServerConfig::from_file_struct)
            .unwrap();
        assert_eq!(clamped_world.max_world_size, 29_999_984);
    }
}
