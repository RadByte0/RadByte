//! Configuration (`rad.properties`) for `RadLimbo`.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Current `rad.properties` schema version for `RadLimbo`.
pub const LIMBO_CONFIG_VERSION: u32 = 1;

/// Forwarding mode expected from the upstream proxy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LimboForwardingMode {
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

/// Serializable `limbo.toml` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LimboConfigFile {
    /// Configuration schema version (currently `1`).
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    /// Address and port to listen on (e.g., `"127.0.0.1:25566"`).
    pub bind_addr: String,
    /// Expected proxy forwarding mode (`"modern"`, `"legacy"`, `"bungeeguard"`, or `"none"`).
    pub forwarding_mode: LimboForwardingMode,
    /// Shared secret for `modern` (Velocity) or `bungeeguard` forwarding.
    pub forwarding_secret: String,
    /// Server list MOTD (used when pinged directly).
    pub motd: String,
    /// Maximum players reported in Status / PlayLogin.
    pub max_players: i32,
    /// Client view distance in chunks.
    pub view_distance: i32,
    /// Game mode (`0`=Survival, `1`=Creative, `2`=Adventure, `3`=Spectator).
    pub game_mode: u8,
    /// Spawn X coordinate.
    pub spawn_x: f64,
    /// Spawn Y coordinate.
    pub spawn_y: f64,
    /// Spawn Z coordinate.
    pub spawn_z: f64,
    /// Brand string sent on `minecraft:brand` during Configuration.
    pub brand: String,
    /// Optional welcome message sent to the player on Play spawn.
    pub welcome_message: Option<String>,
    /// Optional target server name for `/lobby` / `/hub` via `bungeecord:main` `Connect`.
    pub reconnect_server: Option<String>,
    /// KeepAlive interval in seconds.
    pub keep_alive_interval_secs: u64,
    /// Maximum allowed uncompressed packet size in bytes.
    pub max_packet_size: usize,
}

fn default_config_version() -> u32 {
    LIMBO_CONFIG_VERSION
}

impl Default for LimboConfigFile {
    fn default() -> Self {
        Self {
            config_version: LIMBO_CONFIG_VERSION,
            bind_addr: "127.0.0.1:25566".into(),
            forwarding_mode: LimboForwardingMode::None,
            forwarding_secret: "change-me-radbyte-secret".into(),
            motd: "RadLimbo 26.3 - Waiting Room".into(),
            max_players: 1000,
            view_distance: 2,
            game_mode: 3,
            spawn_x: 0.5,
            spawn_y: 64.0,
            spawn_z: 0.5,
            brand: "RadLimbo".into(),
            welcome_message: Some("Welcome to RadLimbo! Waiting for a backend server...".into()),
            reconnect_server: Some("lobby".into()),
            keep_alive_interval_secs: 15,
            max_packet_size: 2 * 1024 * 1024,
        }
    }
}

/// Validated runtime configuration for `RadLimbo`.
#[derive(Debug, Clone)]
pub struct LimboConfig {
    /// Parsed socket address to bind.
    pub bind_addr: SocketAddr,
    /// Expected forwarding mode.
    pub forwarding_mode: LimboForwardingMode,
    /// Shared secret bytes.
    pub forwarding_secret: Vec<u8>,
    /// Server list MOTD.
    pub motd: String,
    /// Maximum players reported.
    pub max_players: i32,
    /// View distance in chunks.
    pub view_distance: i32,
    /// Player game mode (`0..=3`).
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
    /// Optional `bungeecord:main` `Connect` target for `/lobby` / `/hub`.
    pub reconnect_server: Option<String>,
    /// KeepAlive interval.
    pub keep_alive_interval: Duration,
    /// Maximum uncompressed packet size.
    pub max_packet_size: usize,
}

impl LimboConfig {
    /// Builds validated runtime config from a [`LimboConfigFile`].
    pub fn from_file_struct(file: LimboConfigFile) -> Result<Self, String> {
        if !file.spawn_x.is_finite() || !file.spawn_y.is_finite() || !file.spawn_z.is_finite() {
            return Err("spawn coordinates must be finite".to_owned());
        }
        let bind_addr: SocketAddr = file
            .bind_addr
            .parse()
            .map_err(|e| format!("invalid bind_addr '{}': {e}", file.bind_addr))?;
        Ok(Self {
            bind_addr,
            forwarding_mode: file.forwarding_mode,
            forwarding_secret: file.forwarding_secret.into_bytes(),
            motd: file.motd,
            max_players: file.max_players.max(1),
            view_distance: file.view_distance.clamp(2, 32),
            game_mode: file.game_mode.min(3),
            spawn_x: file.spawn_x,
            spawn_y: file.spawn_y,
            spawn_z: file.spawn_z,
            brand: file.brand,
            welcome_message: file.welcome_message,
            reconnect_server: file.reconnect_server,
            keep_alive_interval: Duration::from_secs(file.keep_alive_interval_secs.max(1)),
            max_packet_size: file.max_packet_size.max(4096),
        })
    }

    /// Loads `rad.properties` from disk (migrating legacy `radlimbo.toml` if present) or writes defaults on first start.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if path.extension().is_some_and(|ext| ext == "toml") {
            let raw = std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
            let parsed: LimboConfigFile = toml::from_str(&raw)
                .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
            return Self::from_file_struct(parsed);
        }
        if !path.exists() {
            let legacy = path.with_file_name("radlimbo.toml");
            let defaults = if legacy.exists() {
                let raw = std::fs::read_to_string(&legacy)
                    .map_err(|e| format!("failed to read {}: {e}", legacy.display()))?;
                toml::from_str::<LimboConfigFile>(&raw)
                    .map_err(|e| format!("failed to parse {}: {e}", legacy.display()))?
            } else {
                LimboConfigFile::default()
            };
            let props_text = write_limbo_properties(&defaults)?;
            std::fs::write(path, props_text)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
            return Self::from_file_struct(defaults);
        }

        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let parsed = read_limbo_properties(&raw)
            .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
        Self::from_file_struct(parsed)
    }
}

fn write_limbo_properties(file: &LimboConfigFile) -> Result<String, String> {
    let addr: SocketAddr = file
        .bind_addr
        .parse()
        .map_err(|e| format!("invalid bind_addr '{}': {e}", file.bind_addr))?;
    let forwarding = match file.forwarding_mode {
        LimboForwardingMode::None => "none",
        LimboForwardingMode::Legacy => "legacy",
        LimboForwardingMode::BungeeGuard => "bungeeguard",
        LimboForwardingMode::Modern => "modern",
    };
    let gamemode = match file.game_mode {
        0 => "survival",
        1 => "creative",
        2 => "adventure",
        _ => "spectator",
    };
    let mut out = String::new();
    out.push_str("# RadLimbo Configuration (Minecraft 26.3 / protocol 777)\n");
    out.push_str(&format!("config-version={}\n\n", file.config_version));
    out.push_str("# Network and forwarding\n");
    out.push_str(&format!("server-ip={}\n", addr.ip()));
    out.push_str(&format!("server-port={}\n", addr.port()));
    out.push_str(&format!("forwarding-mode={forwarding}\n"));
    out.push_str(&format!("forwarding-secret={}\n\n", file.forwarding_secret));
    out.push_str("# Status and waiting room gameplay\n");
    out.push_str(&format!("motd={}\n", file.motd));
    out.push_str(&format!("max-players={}\n", file.max_players));
    out.push_str(&format!("view-distance={}\n", file.view_distance));
    out.push_str(&format!("gamemode={gamemode}\n\n"));
    out.push_str("# Spawn coordinates\n");
    out.push_str(&format!("spawn-x={}\n", file.spawn_x));
    out.push_str(&format!("spawn-y={}\n", file.spawn_y));
    out.push_str(&format!("spawn-z={}\n\n", file.spawn_z));
    out.push_str("# Messages and reconnect target\n");
    out.push_str(&format!("brand={}\n", file.brand));
    if let Some(ref welcome) = file.welcome_message {
        out.push_str(&format!("welcome-message={welcome}\n"));
    }
    if let Some(ref reconnect) = file.reconnect_server {
        out.push_str(&format!("reconnect-server={reconnect}\n"));
    }
    out.push_str(&format!(
        "keep-alive-interval-secs={}\n",
        file.keep_alive_interval_secs
    ));
    out.push_str(&format!("max-packet-size={}\n", file.max_packet_size));
    Ok(out)
}

fn read_limbo_properties(raw: &str) -> Result<LimboConfigFile, String> {
    let mut cfg = LimboConfigFile::default();
    let mut server_ip: Option<String> = None;
    let mut server_port: Option<u16> = None;
    let mut explicit_bind: Option<String> = None;

    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let (key, val) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected key=value", idx + 1))?;
        let key = key.trim().replace('_', "-").to_ascii_lowercase();
        let val = val.trim().trim_matches('"');
        match key.as_str() {
            "config-version" => {
                cfg.config_version = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid config-version", idx + 1))?;
            }
            "bind-addr" | "bind-address" => {
                explicit_bind = Some(val.to_owned());
            }
            "server-ip" => {
                server_ip = Some(val.to_owned());
            }
            "server-port" => {
                server_port = Some(
                    val.parse()
                        .map_err(|_| format!("line {}: invalid server-port", idx + 1))?,
                );
            }
            "forwarding-mode" => {
                cfg.forwarding_mode = match val.to_ascii_lowercase().as_str() {
                    "none" => LimboForwardingMode::None,
                    "legacy" => LimboForwardingMode::Legacy,
                    "bungeeguard" => LimboForwardingMode::BungeeGuard,
                    "modern" => LimboForwardingMode::Modern,
                    _ => {
                        return Err(format!("line {}: invalid forwarding-mode '{val}'", idx + 1));
                    }
                };
            }
            "forwarding-secret" => cfg.forwarding_secret = val.to_owned(),
            "motd" => cfg.motd = val.to_owned(),
            "max-players" => {
                cfg.max_players = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid max-players", idx + 1))?;
            }
            "view-distance" => {
                cfg.view_distance = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid view-distance", idx + 1))?;
            }
            "gamemode" | "game-mode" => {
                cfg.game_mode = match val.to_ascii_lowercase().as_str() {
                    "survival" | "0" => 0,
                    "creative" | "1" => 1,
                    "adventure" | "2" => 2,
                    "spectator" | "3" => 3,
                    _ => return Err(format!("line {}: invalid gamemode '{val}'", idx + 1)),
                };
            }
            "spawn-x" => {
                cfg.spawn_x = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid spawn-x", idx + 1))?;
            }
            "spawn-y" => {
                cfg.spawn_y = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid spawn-y", idx + 1))?;
            }
            "spawn-z" => {
                cfg.spawn_z = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid spawn-z", idx + 1))?;
            }
            "brand" => cfg.brand = val.to_owned(),
            "welcome-message" => {
                cfg.welcome_message = if val.is_empty() {
                    None
                } else {
                    Some(val.to_owned())
                };
            }
            "reconnect-server" => {
                cfg.reconnect_server = if val.is_empty() {
                    None
                } else {
                    Some(val.to_owned())
                };
            }
            "keep-alive-interval-secs" => {
                cfg.keep_alive_interval_secs = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid keep-alive-interval-secs", idx + 1))?;
            }
            "max-packet-size" => {
                cfg.max_packet_size = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid max-packet-size", idx + 1))?;
            }
            k if k.starts_with("auto-update") || k.starts_with("auto_update") => {}
            other => {
                return Err(format!("line {}: unknown property '{other}'", idx + 1));
            }
        }
    }

    if let Some(bind) = explicit_bind {
        cfg.bind_addr = bind;
    } else if server_ip.is_some() || server_port.is_some() {
        let default_addr: SocketAddr = cfg
            .bind_addr
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:25566".parse().unwrap());
        let ip = server_ip
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_addr.ip().to_string());
        let port = server_port.unwrap_or_else(|| default_addr.port());
        cfg.bind_addr = format!("{ip}:{port}");
    }

    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_finite_spawn_coordinates() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for axis in 0..3 {
                let mut file = LimboConfigFile::default();
                match axis {
                    0 => file.spawn_x = value,
                    1 => file.spawn_y = value,
                    _ => file.spawn_z = value,
                }
                assert!(LimboConfig::from_file_struct(file).is_err());
            }
        }
    }

    #[test]
    fn default_limbo_config_validates() {
        let cfg = LimboConfig::from_file_struct(LimboConfigFile::default()).unwrap();
        assert_eq!(cfg.bind_addr, "127.0.0.1:25566".parse().unwrap());
        assert_eq!(cfg.forwarding_mode, LimboForwardingMode::None);
        assert_eq!(cfg.game_mode, 3);
    }

    #[test]
    fn properties_round_trip_validates() {
        let defaults = LimboConfigFile::default();
        let props = write_limbo_properties(&defaults).unwrap();
        let parsed = read_limbo_properties(&props).unwrap();
        let cfg = LimboConfig::from_file_struct(parsed).unwrap();
        assert_eq!(cfg.bind_addr, "127.0.0.1:25566".parse().unwrap());
        assert_eq!(cfg.game_mode, 3);
        assert_eq!(cfg.view_distance, 2);
    }
}
