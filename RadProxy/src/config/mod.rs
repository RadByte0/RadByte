//! Configuration (`rad.properties` with legacy `radproxy.toml` compatibility) for `RadProxy`.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use rad_core::prelude::*;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Player identity forwarding mode to backend servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ForwardingMode {
    /// Direct pass-through without forwarding real player IP/UUID.
    #[default]
    None,
    /// BungeeCord legacy IP/UUID forwarding via `Handshake.server_address`.
    Legacy,
    /// BungeeGuard token forwarding via `Handshake.server_address` properties.
    BungeeGuard,
    /// Velocity Modern Forwarding (`HMAC-SHA256` signed `velocity:player_info`).
    Modern,
}

/// Server List Ping passthrough mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PingPassthroughMode {
    /// Serve the proxy's own MOTD, favicon, and online player count.
    #[default]
    None,
    /// Pass through player counts/sample from the default backend.
    Players,
    /// Pass through MOTD description from the default backend.
    Description,
    /// Pass through description, players, and favicon from the default backend.
    All,
}

/// Load-balancing strategy for `[server_groups]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LoadBalanceStrategy {
    /// Route to the backend in the group with the fewest active players.
    #[default]
    LeastPlayers,
    /// Rotate sequentially across backends in the group.
    RoundRobin,
}

/// Current `rad.toml` schema version.
pub const CONFIG_VERSION: u32 = 1;

/// Serializable `rad.toml` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxyConfigFile {
    /// Configuration schema version (currently `1`).
    #[serde(default = "default_config_version")]
    pub config_version: u32,
    /// Address and port to listen on (e.g., `"0.0.0.0:25565"`).
    pub bind_addr: String,
    /// Optional default backend server address (e.g., `"127.0.0.1:25566"`).
    pub backend_addr: Option<String>,
    /// Optional default server name from `[servers]` (used when `backend_addr` is unset).
    pub default_server: Option<String>,
    /// Named backend servers for routing and `/server <name>` switching.
    #[serde(default)]
    pub servers: HashMap<String, String>,
    /// Ordered list of server names from `[servers]` to try on connect or fallback.
    #[serde(default)]
    pub try_servers: Vec<String>,
    /// Hostname-to-server routing (`"play.example.com" = "lobby"`).
    #[serde(default)]
    pub forced_hosts: HashMap<String, String>,
    /// Whether to parse incoming HAProxy `PROXY` protocol (v1/v2) headers for real client IP.
    #[serde(default)]
    pub haproxy_protocol: bool,
    /// List of trusted reverse-proxy IPs allowed to send HAProxy `PROXY` headers.
    #[serde(default)]
    pub haproxy_trusted_proxies: Vec<IpAddr>,
    /// Explicit opt-in to accept HAProxy `PROXY` headers from any peer when `haproxy_trusted_proxies` is empty.
    #[serde(default)]
    pub haproxy_allow_untrusted: bool,
    /// Forwarding mode (`"none"`, `"legacy"`, `"bungeeguard"`, or `"modern"`).
    pub forwarding_mode: ForwardingMode,
    /// Shared secret used for Velocity Modern Forwarding or BungeeGuard token.
    pub forwarding_secret: String,
    /// Whether to authenticate players against Mojang's session servers (`AES-128-CFB8` + RSA).
    pub online_mode: bool,
    /// Server list MOTD text.
    pub motd: String,
    /// Optional list of rotating/random MOTDs (overrides `motd` when non-empty).
    #[serde(default)]
    pub motds: Vec<String>,
    /// Optional path to a 64x64 PNG server icon (`server-icon.png`).
    pub favicon_path: String,
    /// Maximum player count advertised in `StatusResponse`.
    pub max_players: i32,
    /// Reject new logins once `online_players >= max_players` (admins bypass).
    #[serde(default)]
    pub enforce_max_players: bool,
    /// Status ping passthrough mode (`"none"`, `"players"`, `"description"`, `"all"`).
    #[serde(default)]
    pub ping_passthrough: PingPassthroughMode,
    /// Include online player names in the Status hover sample list.
    #[serde(default)]
    pub sample_players_in_ping: bool,
    /// Timeout in milliseconds for receiving the initial `Handshake` packet.
    pub handshake_timeout_ms: u64,
    /// Timeout in milliseconds for connecting to a backend server.
    #[serde(default = "default_connect_timeout_ms")]
    pub connect_timeout_ms: u64,
    /// Maximum allowed uncompressed packet size in bytes.
    pub max_packet_size: usize,
    /// Maximum concurrent connections allowed from a single IP address (`0` = unlimited).
    pub max_connections_per_ip: usize,
    /// Maximum new connections per second allowed from a single IP address (`0` = unlimited).
    pub rate_limit_per_sec: u32,
    /// If `true`, kick the existing session when the same player joins again.
    #[serde(default = "default_true")]
    pub kick_existing_players: bool,
    /// Enable player whitelist (`whitelist` + `admins` are allowed).
    #[serde(default)]
    pub whitelist_enabled: bool,
    /// Whitelisted player usernames (case-insensitive).
    #[serde(default)]
    pub whitelist: Vec<String>,
    /// Banned player usernames (case-insensitive).
    #[serde(default)]
    pub banned_players: Vec<String>,
    /// Banned IP addresses.
    #[serde(default)]
    pub banned_ips: Vec<String>,
    /// Admin usernames allowed to run `/send`, `/alert`, `/find`, `/kick`, `/radproxy`.
    #[serde(default)]
    pub admins: Vec<String>,
    /// Per-server player allowlists (`server_name = ["Player1", ...]`).
    #[serde(default)]
    pub restricted_servers: HashMap<String, Vec<String>>,
    /// Optional HTTP bind address for Prometheus `/metrics` (e.g., `"127.0.0.1:9100"`).
    #[serde(default)]
    pub metrics_bind_addr: Option<String>,
    /// Log player join/leave events.
    #[serde(default = "default_true")]
    pub log_player_connections: bool,
    /// Log proxy command executions.
    #[serde(default = "default_true")]
    pub log_command_executions: bool,
    /// Directory scanned for RadByte radons (`radons/`).
    pub radons_dir: String,
    /// Directory storing per-radon configuration files (`radonfigs/`).
    #[serde(default = "default_radonfigs_dir")]
    pub radonfigs_dir: String,
    /// Load-balancing strategy for `[server_groups]` (`"least_players"` or `"round_robin"`).
    #[serde(default)]
    pub load_balance_strategy: LoadBalanceStrategy,
    /// Named server groups for load balancing (`lobby = ["lobby-1", "lobby-2"]`).
    #[serde(default)]
    pub server_groups: HashMap<String, Vec<String>>,
    /// Handle BungeeCord plugin messaging (`bungeecord:main` / `BungeeCord`).
    #[serde(default = "default_true")]
    pub bungeecord_messaging: bool,
    /// Optional `minecraft:brand` override shown in F3 (e.g., `"RadProxy"`).
    #[serde(default)]
    pub proxy_brand: Option<String>,
}

fn default_connect_timeout_ms() -> u64 {
    3000
}

fn default_true() -> bool {
    true
}

fn default_radonfigs_dir() -> String {
    "radonfigs".into()
}

fn default_config_version() -> u32 {
    CONFIG_VERSION
}

impl Default for ProxyConfigFile {
    fn default() -> Self {
        let mut servers = HashMap::new();
        servers.insert("limbo".into(), "127.0.0.1:25566".into());

        Self {
            config_version: CONFIG_VERSION,
            bind_addr: "0.0.0.0:25565".into(),
            backend_addr: None,
            default_server: Some("limbo".into()),
            servers,
            try_servers: vec!["limbo".into()],
            forced_hosts: HashMap::new(),
            haproxy_protocol: false,
            haproxy_trusted_proxies: Vec::new(),
            haproxy_allow_untrusted: false,
            forwarding_mode: ForwardingMode::Modern,
            forwarding_secret: "change-me-radbyte-secret".into(),
            online_mode: false,
            motd: "RadProxy".into(),
            motds: Vec::new(),
            favicon_path: "server-icon.png".into(),
            max_players: 1000,
            enforce_max_players: false,
            ping_passthrough: PingPassthroughMode::None,
            sample_players_in_ping: false,
            handshake_timeout_ms: 3000,
            connect_timeout_ms: default_connect_timeout_ms(),
            max_packet_size: 64 * 1024,
            max_connections_per_ip: 16,
            rate_limit_per_sec: 25,
            kick_existing_players: true,
            whitelist_enabled: false,
            whitelist: Vec::new(),
            banned_players: Vec::new(),
            banned_ips: Vec::new(),
            admins: Vec::new(),
            restricted_servers: HashMap::new(),
            metrics_bind_addr: None,
            log_player_connections: true,
            log_command_executions: true,
            radons_dir: "radons".into(),
            radonfigs_dir: default_radonfigs_dir(),
            load_balance_strategy: LoadBalanceStrategy::LeastPlayers,
            server_groups: HashMap::new(),
            bungeecord_messaging: true,
            proxy_brand: Some("RadProxy".into()),
        }
    }
}

/// Validated runtime configuration for `RadProxy`.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Configuration schema version.
    pub config_version: u32,
    /// Parsed listen address.
    pub bind_addr: SocketAddr,
    /// Optional parsed default backend address.
    pub backend_addr: Option<SocketAddr>,
    /// Named backend servers (`name -> SocketAddr`).
    pub servers: HashMap<String, SocketAddr>,
    /// Ordered fallback server names (`[servers]` keys) tried on connect/disconnect.
    pub try_servers: Vec<String>,
    /// Lowercased forced-host mapping (`host -> server_name`).
    pub forced_hosts: HashMap<String, String>,
    /// Whether HAProxy `PROXY` protocol v1/v2 headers are expected on incoming TCP streams.
    pub haproxy_protocol: bool,
    /// Trusted proxy IPs allowed to send HAProxy headers.
    pub haproxy_trusted_proxies: HashSet<IpAddr>,
    /// Explicit opt-in to accept HAProxy headers from any peer (not recommended for public binds).
    pub haproxy_allow_untrusted: bool,
    /// Player forwarding mode.
    pub forwarding_mode: ForwardingMode,
    /// Shared secret bytes for Velocity Modern Forwarding or BungeeGuard token.
    pub forwarding_secret: Vec<u8>,
    /// Whether online-mode Mojang authentication + encryption is enabled.
    pub online_mode: bool,
    /// Displayed server MOTD component.
    pub motd: TextComponent,
    /// Optional rotating MOTD components.
    pub motds: Vec<TextComponent>,
    /// Pre-encoded `data:image/png;base64,...` favicon if `server-icon.png` exists.
    pub favicon: Option<String>,
    /// Maximum player count advertised in `StatusResponse`.
    pub max_players: i32,
    /// Reject new logins once `online_players >= max_players` (admins bypass).
    pub enforce_max_players: bool,
    /// Status ping passthrough mode.
    pub ping_passthrough: PingPassthroughMode,
    /// Include online player names in Status hover sample.
    pub sample_players_in_ping: bool,
    /// Timeout for receiving the initial `Handshake` packet.
    pub handshake_timeout: Duration,
    /// Timeout for connecting to a backend server.
    pub connect_timeout: Duration,
    /// Maximum allowed packet size in bytes.
    pub max_packet_size: usize,
    /// Maximum concurrent connections per IP.
    pub max_connections_per_ip: usize,
    /// Maximum new connections per second per IP.
    pub rate_limit_per_sec: u32,
    /// Kick existing session when the same player joins again.
    pub kick_existing_players: bool,
    /// Enable player whitelist.
    pub whitelist_enabled: bool,
    /// Lowercased whitelisted usernames.
    pub whitelist: HashSet<String>,
    /// Lowercased banned usernames.
    pub banned_players: HashSet<String>,
    /// Banned client IP addresses.
    pub banned_ips: HashSet<IpAddr>,
    /// Lowercased admin usernames.
    pub admins: HashSet<String>,
    /// Per-server allowed usernames (`server_name -> lowercased usernames`).
    pub restricted_servers: HashMap<String, HashSet<String>>,
    /// Optional HTTP `/metrics` listen address.
    pub metrics_bind_addr: Option<SocketAddr>,
    /// Log player join/leave events.
    pub log_player_connections: bool,
    /// Log proxy command executions.
    pub log_command_executions: bool,
    /// Path to the unified `radons/` directory.
    pub radons_dir: String,
    /// Path to the `radonfigs/` directory for radon configurations.
    pub radonfigs_dir: String,
    /// Load-balancing strategy for `[server_groups]`.
    pub load_balance_strategy: LoadBalanceStrategy,
    /// Named server groups for load balancing.
    pub server_groups: HashMap<String, Vec<String>>,
    /// Handle BungeeCord plugin messaging (`bungeecord:main`).
    pub bungeecord_messaging: bool,
    /// Optional `minecraft:brand` override shown in F3.
    pub proxy_brand: Option<String>,
}

impl ProxyConfig {
    /// Loads configuration from `path`, creating a default `rad.properties` if it does not exist.
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let is_toml = path.extension().is_some_and(|ext| ext == "toml");
        let raw = if path.exists() {
            let content = fs::read_to_string(path)?;
            if is_toml {
                toml::from_str::<ProxyConfigFile>(&content)
                    .map_err(|e| Error::InvalidString(format!("invalid {}: {e}", path.display())))?
            } else {
                read_proxy_properties(&content)
                    .map_err(|e| Error::InvalidString(format!("invalid {}: {e}", path.display())))?
            }
        } else if is_toml {
            let default_cfg = ProxyConfigFile::default();
            fs::write(path, default_rad_toml())?;
            info!(path = %path.display(), "created default rad.toml");
            default_cfg
        } else {
            let legacy = path.with_file_name("radproxy.toml");
            let default_cfg = if legacy.exists() {
                let content = fs::read_to_string(&legacy)?;
                toml::from_str::<ProxyConfigFile>(&content).map_err(|e| {
                    Error::InvalidString(format!("invalid {}: {e}", legacy.display()))
                })?
            } else {
                ProxyConfigFile::default()
            };
            let props = write_proxy_properties(&default_cfg)
                .map_err(|e| Error::InvalidString(format!("invalid proxy defaults: {e}")))?;
            fs::write(path, props)?;
            info!(path = %path.display(), "created default rad.properties");
            default_cfg
        };

        if raw.config_version > CONFIG_VERSION {
            warn!(
                found = raw.config_version,
                supported = CONFIG_VERSION,
                "rad.toml uses a newer config_version than this build of RadProxy"
            );
        }

        let bind_addr: SocketAddr = raw
            .bind_addr
            .parse()
            .map_err(|e| Error::InvalidString(format!("invalid bind_addr: {e}")))?;

        let mut servers = HashMap::with_capacity(raw.servers.len());
        for (name, addr_str) in raw.servers {
            let addr: SocketAddr = addr_str.parse().map_err(|e| {
                Error::InvalidString(format!("invalid address for server '{name}': {e}"))
            })?;
            servers.insert(name, addr);
        }

        let mut backend_addr = match raw.backend_addr {
            Some(addr) if !addr.trim().is_empty() => Some(
                addr.parse::<SocketAddr>()
                    .map_err(|e| Error::InvalidString(format!("invalid backend_addr: {e}")))?,
            ),
            _ => None,
        };

        if backend_addr.is_none()
            && let Some(ref default_name) = raw.default_server
        {
            backend_addr = Some(*servers.get(default_name).ok_or_else(|| {
                Error::InvalidString(format!(
                    "default_server '{default_name}' not found in [servers]"
                ))
            })?);
        }

        let mut forced_hosts = HashMap::with_capacity(raw.forced_hosts.len());
        for (host, server_name) in raw.forced_hosts {
            if !servers.contains_key(&server_name) {
                return Err(Error::InvalidString(format!(
                    "forced_hosts target '{server_name}' for '{host}' not found in [servers]"
                )));
            }
            forced_hosts.insert(
                host.trim().trim_end_matches('.').to_ascii_lowercase(),
                server_name,
            );
        }

        let mut banned_ips = HashSet::with_capacity(raw.banned_ips.len());
        for ip_str in raw.banned_ips {
            let ip: IpAddr = ip_str
                .parse()
                .map_err(|e| Error::InvalidString(format!("invalid banned_ips '{ip_str}': {e}")))?;
            banned_ips.insert(ip);
        }

        let metrics_bind_addr = match raw.metrics_bind_addr {
            Some(addr) if !addr.trim().is_empty() => Some(
                addr.parse::<SocketAddr>()
                    .map_err(|e| Error::InvalidString(format!("invalid metrics_bind_addr: {e}")))?,
            ),
            _ => None,
        };

        let favicon = load_favicon(&raw.favicon_path);
        let motds = raw
            .motds
            .into_iter()
            .filter(|m| !m.trim().is_empty())
            .map(|m| TextComponent::text(m).color("aqua"))
            .collect();

        let restricted_servers = raw
            .restricted_servers
            .into_iter()
            .map(|(srv, users)| {
                (
                    srv,
                    users.into_iter().map(|u| u.to_ascii_lowercase()).collect(),
                )
            })
            .collect();

        Ok(Self {
            config_version: raw.config_version,
            bind_addr,
            backend_addr,
            servers,
            try_servers: raw.try_servers,
            forced_hosts,
            haproxy_protocol: raw.haproxy_protocol,
            haproxy_trusted_proxies: raw.haproxy_trusted_proxies.into_iter().collect(),
            haproxy_allow_untrusted: raw.haproxy_allow_untrusted,
            forwarding_mode: raw.forwarding_mode,
            forwarding_secret: raw.forwarding_secret.into_bytes(),
            online_mode: raw.online_mode,
            motd: TextComponent::text(raw.motd).color("aqua"),
            motds,
            favicon,
            max_players: raw.max_players,
            enforce_max_players: raw.enforce_max_players,
            ping_passthrough: raw.ping_passthrough,
            sample_players_in_ping: raw.sample_players_in_ping,
            handshake_timeout: Duration::from_millis(raw.handshake_timeout_ms),
            connect_timeout: Duration::from_millis(raw.connect_timeout_ms),
            max_packet_size: raw.max_packet_size,
            max_connections_per_ip: raw.max_connections_per_ip,
            rate_limit_per_sec: raw.rate_limit_per_sec,
            kick_existing_players: raw.kick_existing_players,
            whitelist_enabled: raw.whitelist_enabled,
            whitelist: raw
                .whitelist
                .into_iter()
                .map(|u| u.to_ascii_lowercase())
                .collect(),
            banned_players: raw
                .banned_players
                .into_iter()
                .map(|u| u.to_ascii_lowercase())
                .collect(),
            banned_ips,
            admins: raw
                .admins
                .into_iter()
                .map(|u| u.to_ascii_lowercase())
                .collect(),
            restricted_servers,
            metrics_bind_addr,
            log_player_connections: raw.log_player_connections,
            log_command_executions: raw.log_command_executions,
            radons_dir: raw.radons_dir,
            radonfigs_dir: raw.radonfigs_dir,
            load_balance_strategy: raw.load_balance_strategy,
            server_groups: raw.server_groups,
            bungeecord_messaging: raw.bungeecord_messaging,
            proxy_brand: raw.proxy_brand.filter(|s| !s.trim().is_empty()),
        })
    }
}

fn load_favicon(path_str: &str) -> Option<String> {
    let path = Path::new(path_str);
    if !path.exists() {
        return None;
    }
    match fs::read(path) {
        Ok(bytes) => {
            info!(path = %path.display(), bytes = bytes.len(), "loaded server favicon");
            Some(format!("data:image/png;base64,{}", BASE64.encode(&bytes)))
        }
        Err(err) => {
            warn!(path = %path.display(), %err, "failed to read server favicon");
            None
        }
    }
}

fn default_rad_toml() -> String {
    format!(
        r#"# ==============================================================================
#  ____      _    ____  ____  ____   ___ __  ____   __
# |  _ \    / \  |  _ \|  _ \|  _ \ / _ \\ \/ /\ \ / /
# | |_) |  / _ \ | | | | |_) | |_) | | | |\  /  \ V /
# |  _ <  / ___ \| |_| |  __/|  _ <| |_| |/  \   | |
# |_| \_\/_/   \_\____/|_|   |_| \_\\___//_/\_\  |_|
#
# radproxy.toml — RadProxy Configuration
# ==============================================================================

# Configuration schema version (incremented on breaking format changes).
config_version = {CONFIG_VERSION}

# --- Network & Authentication -------------------------------------------------
bind_addr = "0.0.0.0:25565"
online_mode = false              # Verify players with Mojang + AES-128-CFB8
haproxy_protocol = false         # Parse HAProxy PROXY v1/v2 headers (TCPShield/nginx)
connect_timeout_ms = 3000        # Backend TCP connect timeout

# --- Forwarding & Plugin Messaging --------------------------------------------
# "none" | "legacy" | "bungeeguard" | "modern"
forwarding_mode = "modern"
forwarding_secret = "change-me-radbyte-secret"
bungeecord_messaging = true      # Handle `bungeecord:main` plugin channel
proxy_brand = "RadProxy"         # Custom F3 server brand (optional)

# --- Server List (MOTD & Ping) ------------------------------------------------
motd = "RadProxy"
motds = []                       # Optional rotating MOTD list
favicon_path = "server-icon.png"
max_players = 1000
enforce_max_players = false      # Reject joins when online >= max_players (admins bypass)
ping_passthrough = "none"        # "none" | "players" | "description" | "all"
sample_players_in_ping = false   # Show online player names on hover

# --- Security, Anti-Bot & Access Control --------------------------------------
handshake_timeout_ms = 3000
max_packet_size = 65536
max_connections_per_ip = 16
rate_limit_per_sec = 25
kick_existing_players = true     # Kick old session if same player joins again
whitelist_enabled = false
whitelist = []
banned_players = []
banned_ips = []

# --- Permissions & Logging ----------------------------------------------------
# Admins can run /send, /alert, /find, /kick, /radproxy and bypass whitelist/limits.
admins = []
log_player_connections = true
log_command_executions = true

# --- Observability & Plugins --------------------------------------------------
# metrics_bind_addr = "127.0.0.1:9100"   # Optional Prometheus HTTP /metrics endpoint
radons_dir = "radons"
radonfigs_dir = "radonfigs"

# --- Routing & Backend Servers ------------------------------------------------
# backend_addr = "127.0.0.1:25566"       # Direct backend (if default_server is unset)
default_server = "limbo"                 # Default server key from [servers]
try_servers = ["limbo"]                  # Ordered fallback list from [servers]
load_balance_strategy = "least_players"  # "least_players" | "round_robin"

[servers]
limbo = "127.0.0.1:25566"
# lobby = "127.0.0.1:25567"
# survival = "127.0.0.1:25568"

[server_groups]
# lobby = ["lobby-1", "lobby-2"]

[forced_hosts]
# "play.example.com" = "lobby"
# "survival.example.com" = "survival"

[restricted_servers]
# staff = ["AdminName"]
"#
    )
}

fn write_proxy_properties(file: &ProxyConfigFile) -> std::result::Result<String, String> {
    let addr: SocketAddr = file
        .bind_addr
        .parse()
        .map_err(|e| format!("invalid bind_addr '{}': {e}", file.bind_addr))?;
    let forwarding = match file.forwarding_mode {
        ForwardingMode::None => "none",
        ForwardingMode::Legacy => "legacy",
        ForwardingMode::BungeeGuard => "bungeeguard",
        ForwardingMode::Modern => "modern",
    };
    let passthrough = match file.ping_passthrough {
        PingPassthroughMode::None => "none",
        PingPassthroughMode::Players => "players",
        PingPassthroughMode::Description => "description",
        PingPassthroughMode::All => "all",
    };
    let lb_strategy = match file.load_balance_strategy {
        LoadBalanceStrategy::LeastPlayers => "least_players",
        LoadBalanceStrategy::RoundRobin => "round_robin",
    };

    let mut out = String::new();
    out.push_str("# RadProxy Configuration (Minecraft 26.3 / protocol 777)\n");
    out.push_str(&format!("config-version={}\n\n", file.config_version));
    out.push_str("# Network and status\n");
    out.push_str(&format!("server-ip={}\n", addr.ip()));
    out.push_str(&format!("server-port={}\n", addr.port()));
    out.push_str(&format!("online-mode={}\n", file.online_mode));
    out.push_str(&format!("haproxy-protocol={}\n", file.haproxy_protocol));
    out.push_str(&format!("motd={}\n", file.motd));
    if !file.motds.is_empty() {
        out.push_str(&format!("motds={}\n", file.motds.join("|")));
    }
    out.push_str(&format!("max-players={}\n", file.max_players));
    out.push_str(&format!(
        "enforce-max-players={}\n",
        file.enforce_max_players
    ));
    out.push_str(&format!("ping-passthrough={passthrough}\n"));
    out.push_str(&format!(
        "sample-players-in-ping={}\n",
        file.sample_players_in_ping
    ));
    out.push_str(&format!("favicon-path={}\n\n", file.favicon_path));

    out.push_str("# Proxy forwarding and plugin channels\n");
    out.push_str(&format!("forwarding-mode={forwarding}\n"));
    out.push_str(&format!("forwarding-secret={}\n", file.forwarding_secret));
    if let Some(ref brand) = file.proxy_brand {
        out.push_str(&format!("proxy-brand={brand}\n"));
    }
    out.push_str(&format!(
        "bungeecord-messaging={}\n\n",
        file.bungeecord_messaging
    ));

    out.push_str("# Security and rate limits\n");
    out.push_str(&format!(
        "handshake-timeout-ms={}\n",
        file.handshake_timeout_ms
    ));
    out.push_str(&format!("connect-timeout-ms={}\n", file.connect_timeout_ms));
    out.push_str(&format!("max-packet-size={}\n", file.max_packet_size));
    out.push_str(&format!(
        "max-connections-per-ip={}\n",
        file.max_connections_per_ip
    ));
    out.push_str(&format!("rate-limit-per-sec={}\n", file.rate_limit_per_sec));
    out.push_str(&format!(
        "kick-existing-players={}\n",
        file.kick_existing_players
    ));
    out.push_str(&format!("white-list={}\n", file.whitelist_enabled));
    if !file.whitelist.is_empty() {
        out.push_str(&format!("whitelist={}\n", file.whitelist.join(",")));
    }
    if !file.banned_players.is_empty() {
        out.push_str(&format!(
            "banned-players={}\n",
            file.banned_players.join(",")
        ));
    }
    if !file.banned_ips.is_empty() {
        out.push_str(&format!("banned-ips={}\n", file.banned_ips.join(",")));
    }
    if !file.admins.is_empty() {
        out.push_str(&format!("admins={}\n", file.admins.join(",")));
    }
    out.push('\n');

    out.push_str("# Routing and backend servers\n");
    if let Some(ref backend) = file.backend_addr {
        out.push_str(&format!("backend-addr={backend}\n"));
    }
    if let Some(ref def_srv) = file.default_server {
        out.push_str(&format!("default-server={def_srv}\n"));
    }
    if !file.try_servers.is_empty() {
        out.push_str(&format!("try-servers={}\n", file.try_servers.join(",")));
    }
    out.push_str(&format!("load-balance-strategy={lb_strategy}\n"));
    let mut server_keys: Vec<_> = file.servers.keys().collect();
    server_keys.sort();
    for k in server_keys {
        if let Some(v) = file.servers.get(k) {
            out.push_str(&format!("server.{k}={v}\n"));
        }
    }
    out.push('\n');

    out.push_str("# Observability and Plugins (Radons)\n");
    if let Some(ref metrics) = file.metrics_bind_addr {
        out.push_str(&format!("metrics-bind-addr={metrics}\n"));
    }
    out.push_str(&format!(
        "log-player-connections={}\n",
        file.log_player_connections
    ));
    out.push_str(&format!(
        "log-command-executions={}\n",
        file.log_command_executions
    ));
    out.push_str(&format!("radons-dir={}\n", file.radons_dir));
    out.push_str(&format!("radonfigs-dir={}\n", file.radonfigs_dir));
    Ok(out)
}

fn parse_csv_list(val: &str) -> Vec<String> {
    val.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn read_proxy_properties(raw: &str) -> std::result::Result<ProxyConfigFile, String> {
    let mut cfg = ProxyConfigFile::default();
    let mut servers_overridden = false;
    let mut server_ip: Option<String> = None;
    let mut server_port: Option<u16> = None;
    let mut explicit_bind: Option<String> = None;

    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        let (raw_key, raw_val) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected key=value", idx + 1))?;
        let raw_key = raw_key.trim();
        let val = raw_val.trim().trim_matches('"');

        if let Some(srv_name) = raw_key
            .strip_prefix("server.")
            .or_else(|| raw_key.strip_prefix("servers."))
        {
            if !servers_overridden {
                cfg.servers.clear();
                servers_overridden = true;
            }
            cfg.servers
                .insert(srv_name.trim().to_owned(), val.to_owned());
            continue;
        }
        if let Some(grp_name) = raw_key
            .strip_prefix("server-group.")
            .or_else(|| raw_key.strip_prefix("server_groups."))
        {
            cfg.server_groups
                .insert(grp_name.trim().to_owned(), parse_csv_list(val));
            continue;
        }
        if let Some(host_name) = raw_key
            .strip_prefix("forced-host.")
            .or_else(|| raw_key.strip_prefix("forced_hosts."))
        {
            cfg.forced_hosts
                .insert(host_name.trim().to_owned(), val.to_owned());
            continue;
        }
        if let Some(res_name) = raw_key
            .strip_prefix("restricted-server.")
            .or_else(|| raw_key.strip_prefix("restricted_servers."))
        {
            cfg.restricted_servers
                .insert(res_name.trim().to_owned(), parse_csv_list(val));
            continue;
        }

        let key = raw_key.replace('_', "-").to_ascii_lowercase();
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
            "backend-addr" => {
                cfg.backend_addr = if val.is_empty() {
                    None
                } else {
                    Some(val.to_owned())
                };
            }
            "default-server" => {
                cfg.default_server = if val.is_empty() {
                    None
                } else {
                    Some(val.to_owned())
                };
            }
            "try-servers" => {
                cfg.try_servers = parse_csv_list(val);
            }
            "haproxy-protocol" => {
                cfg.haproxy_protocol = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid haproxy-protocol", idx + 1))?;
            }
            "haproxy-trusted-proxies" => {
                let mut ips = Vec::new();
                for item in parse_csv_list(val) {
                    let ip: IpAddr = item.parse().map_err(|e| {
                        format!(
                            "line {}: invalid haproxy-trusted-proxies IP '{item}': {e}",
                            idx + 1
                        )
                    })?;
                    ips.push(ip);
                }
                cfg.haproxy_trusted_proxies = ips;
            }
            "haproxy-allow-untrusted" => {
                cfg.haproxy_allow_untrusted = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid haproxy-allow-untrusted", idx + 1))?;
            }
            "forwarding-mode" => {
                cfg.forwarding_mode = match val.to_ascii_lowercase().as_str() {
                    "none" => ForwardingMode::None,
                    "legacy" => ForwardingMode::Legacy,
                    "bungeeguard" => ForwardingMode::BungeeGuard,
                    "modern" => ForwardingMode::Modern,
                    _ => {
                        return Err(format!("line {}: invalid forwarding-mode '{val}'", idx + 1));
                    }
                };
            }
            "forwarding-secret" => cfg.forwarding_secret = val.to_owned(),
            "online-mode" => {
                cfg.online_mode = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid online-mode", idx + 1))?;
            }
            "motd" => cfg.motd = val.to_owned(),
            "motds" => {
                cfg.motds = val
                    .split('|')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "favicon-path" => cfg.favicon_path = val.to_owned(),
            "max-players" => {
                cfg.max_players = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid max-players", idx + 1))?;
            }
            "enforce-max-players" => {
                cfg.enforce_max_players = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid enforce-max-players", idx + 1))?;
            }
            "ping-passthrough" => {
                cfg.ping_passthrough = match val.to_ascii_lowercase().as_str() {
                    "none" => PingPassthroughMode::None,
                    "players" => PingPassthroughMode::Players,
                    "description" => PingPassthroughMode::Description,
                    "all" => PingPassthroughMode::All,
                    _ => {
                        return Err(format!(
                            "line {}: invalid ping-passthrough '{val}'",
                            idx + 1
                        ));
                    }
                };
            }
            "sample-players-in-ping" => {
                cfg.sample_players_in_ping = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid sample-players-in-ping", idx + 1))?;
            }
            "handshake-timeout-ms" => {
                cfg.handshake_timeout_ms = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid handshake-timeout-ms", idx + 1))?;
            }
            "connect-timeout-ms" => {
                cfg.connect_timeout_ms = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid connect-timeout-ms", idx + 1))?;
            }
            "max-packet-size" => {
                cfg.max_packet_size = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid max-packet-size", idx + 1))?;
            }
            "max-connections-per-ip" => {
                cfg.max_connections_per_ip = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid max-connections-per-ip", idx + 1))?;
            }
            "rate-limit-per-sec" => {
                cfg.rate_limit_per_sec = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid rate-limit-per-sec", idx + 1))?;
            }
            "kick-existing-players" => {
                cfg.kick_existing_players = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid kick-existing-players", idx + 1))?;
            }
            "white-list" | "whitelist-enabled" => {
                cfg.whitelist_enabled = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid white-list", idx + 1))?;
            }
            "whitelist" => cfg.whitelist = parse_csv_list(val),
            "banned-players" => cfg.banned_players = parse_csv_list(val),
            "banned-ips" => cfg.banned_ips = parse_csv_list(val),
            "admins" => cfg.admins = parse_csv_list(val),
            "metrics-bind-addr" => {
                cfg.metrics_bind_addr = if val.is_empty() {
                    None
                } else {
                    Some(val.to_owned())
                };
            }
            "log-player-connections" => {
                cfg.log_player_connections = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid log-player-connections", idx + 1))?;
            }
            "log-command-executions" => {
                cfg.log_command_executions = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid log-command-executions", idx + 1))?;
            }
            "radons-dir" => cfg.radons_dir = val.to_owned(),
            "radonfigs-dir" => cfg.radonfigs_dir = val.to_owned(),
            "load-balance-strategy" => {
                cfg.load_balance_strategy = match val.to_ascii_lowercase().as_str() {
                    "least_players" | "least-players" => LoadBalanceStrategy::LeastPlayers,
                    "round_robin" | "round-robin" => LoadBalanceStrategy::RoundRobin,
                    _ => {
                        return Err(format!(
                            "line {}: invalid load-balance-strategy '{val}'",
                            idx + 1
                        ));
                    }
                };
            }
            "bungeecord-messaging" => {
                cfg.bungeecord_messaging = val
                    .parse()
                    .map_err(|_| format!("line {}: invalid bungeecord-messaging", idx + 1))?;
            }
            "proxy-brand" => {
                cfg.proxy_brand = if val.is_empty() {
                    None
                } else {
                    Some(val.to_owned())
                };
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
            .unwrap_or_else(|_| "0.0.0.0:25565".parse().unwrap());
        let ip = server_ip
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| default_addr.ip().to_string());
        let port = server_port.unwrap_or_else(|| default_addr.port());
        cfg.bind_addr = format!("{ip}:{port}");
    }

    Ok(cfg)
}
