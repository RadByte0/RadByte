//! Configuration (`radlimbo.toml`) for `RadLimbo`.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Current `limbo.toml` schema version.
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

    /// Loads `limbo.toml` from disk or writes defaults on first start.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            let defaults = LimboConfigFile::default();
            let toml_text = toml::to_string_pretty(&defaults)
                .map_err(|e| format!("failed to serialize default limbo.toml: {e}"))?;
            std::fs::write(path, toml_text)
                .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
            return Self::from_file_struct(defaults);
        }

        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let parsed: LimboConfigFile =
            toml::from_str(&raw).map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
        Self::from_file_struct(parsed)
    }
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
}
