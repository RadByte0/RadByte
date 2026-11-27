//! Per-connection L7 protocol handler for `RadProxy`.

pub mod auth;
mod login;
mod play;
pub mod plugin_msg;
mod routing;

#[cfg(test)]
mod tests;

use login::*;
use play::*;
use routing::*;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arc_swap::{ArcSwap, ArcSwapOption};

use bytes::{Buf, Bytes, BytesMut};
use rad_core::codec::decode_raw_frame_slice;
use rad_core::crypto::mojang_server_hash;
use rad_core::nbt::NbtTag;
use rad_core::packet::configuration::{
    AcknowledgeFinishConfiguration, ConfigClientboundPluginMessage, ConfigurationDisconnect,
    FinishConfiguration,
};
use rad_core::packet::handshake::Handshake;
use rad_core::packet::login::{
    EncryptionRequest, EncryptionResponse, GameProfileProperty, LoginAcknowledged, LoginDisconnect,
    LoginPluginRequest, LoginPluginResponse, LoginStart, LoginSuccess, SetCompression,
};
use rad_core::packet::play::{
    ChatCommand, ConfigurationAcknowledged, DeclareCommands, KeepAliveClientbound, MovePlayerPos,
    MovePlayerPosRot, PlayClientboundPluginMessage, PlayDisconnect, PlayServerboundPluginMessage,
    StartConfiguration, SynchronizePlayerPosition, SystemChatMessage,
};
use rad_core::packet::status::{
    PingRequest, PlayerSample, PongResponse, StatusPayload, StatusPlayers, StatusRequest,
    StatusResponse, StatusVersion,
};
use rad_core::prelude::*;
use rand::RngCore;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, info, warn};
use uuid::Uuid;

use self::auth::{ProxyKeypair, verify_mojang_session};
use self::plugin_msg::{
    BungeeRequest, build_proxy_declare_commands, encode_brand_payload, merge_declare_commands,
    parse_bungee_request, write_bungee_utf,
};
use crate::config::{ForwardingMode, LoadBalanceStrategy, PingPassthroughMode, ProxyConfig};
use crate::metrics::ProxyMetrics;
use crate::net::filter::IpFilter;
use crate::net::haproxy::read_haproxy_peer;
use crate::net::stream::EncryptedStream;
use crate::radon::{RadonAction, RadonContext, RadonManager};

struct CachedStatus {
    online_count: i32,
    protocol_version: i32,
    encoded_frame: Bytes,
}

struct CachedPassthrough {
    protocol_version: i32,
    fetched_at: Instant,
    payload: StatusPayload,
}

/// Control message sent to an active player session (`/send`, `/alert`, `/kick`, shutdown).
#[derive(Debug, Clone)]
pub enum PlayerControl {
    /// Switch the player to another backend server.
    Switch {
        /// Target server name in `[servers]`.
        name: String,
        /// Target server address.
        addr: SocketAddr,
    },
    /// Deliver a system chat message to the player.
    SystemChat(String),
    /// Disconnect the player with a reason.
    Kick(String),
}

#[derive(Debug, Clone)]
struct ActivePlayer {
    username: String,
    uuid: Uuid,
    server: Option<String>,
    tx: mpsc::UnboundedSender<PlayerControl>,
}

/// Shared proxy state across all active client tasks.
pub struct ProxyState {
    /// Runtime configuration (hot-reloadable, lock-free reads).
    pub config: ArcSwap<ProxyConfig>,
    /// Per-IP rate limiter and concurrency guard.
    pub ip_filter: Arc<IpFilter>,
    /// Loaded proxy radons (hot-reloadable, lock-free reads).
    pub radons: ArcSwap<RadonManager>,
    /// Optional RSA keypair for `online-mode`.
    pub keypair: Option<ProxyKeypair>,
    /// Shared HTTP client for Mojang session verification.
    pub http: reqwest::Client,
    /// Atomic proxy metrics.
    pub metrics: ProxyMetrics,
    /// Current number of active proxied players.
    pub online_players: AtomicI32,
    config_path: String,
    server_players: Mutex<HashMap<String, usize>>,
    active_players: Mutex<HashMap<String, ActivePlayer>>,
    status_cache: ArcSwapOption<CachedStatus>,
    passthrough_cache: Mutex<Option<CachedPassthrough>>,
    motd_counter: AtomicU64,
}

impl ProxyState {
    /// Creates a new [`ProxyState`] from `config`, `radons`, and optional `keypair`.
    #[must_use]
    pub fn new(config: ProxyConfig, radons: RadonManager, keypair: Option<ProxyKeypair>) -> Self {
        let ip_filter = Arc::new(IpFilter::new(
            config.max_connections_per_ip,
            config.rate_limit_per_sec,
        ));
        Self {
            config: ArcSwap::from_pointee(config),
            ip_filter,
            radons: ArcSwap::from_pointee(radons),
            keypair,
            http: reqwest::Client::new(),
            metrics: ProxyMetrics::default(),
            online_players: AtomicI32::new(0),
            config_path: "radproxy.toml".to_owned(),
            server_players: Mutex::new(HashMap::new()),
            active_players: Mutex::new(HashMap::new()),
            status_cache: ArcSwapOption::empty(),
            passthrough_cache: Mutex::new(None),
            motd_counter: AtomicU64::new(0),
        }
    }

    /// Sets the active configuration file path used by [`Self::reload_current`].
    #[must_use]
    pub fn with_config_path(mut self, path: impl Into<String>) -> Self {
        self.config_path = path.into();
        self
    }

    /// Hot-reloads the configured proxy file (`--config` or `radproxy.toml`).
    pub fn reload_current(&self) -> Result<usize> {
        self.reload(&self.config_path)
    }

    fn increment_server_player(&self, name: &str) {
        let mut guard = self.server_players.lock().expect("server_players poisoned");
        *guard.entry(name.to_string()).or_insert(0) += 1;
    }

    fn decrement_server_player(&self, name: &str) {
        let mut guard = self.server_players.lock().expect("server_players poisoned");
        if let Some(count) = guard.get_mut(name) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                guard.remove(name);
            }
        }
    }

    fn server_player_counts(&self) -> HashMap<String, usize> {
        self.server_players
            .lock()
            .expect("server_players poisoned")
            .clone()
    }

    fn register_player(
        &self,
        username: String,
        uuid: Uuid,
        server: Option<String>,
        tx: mpsc::UnboundedSender<PlayerControl>,
    ) {
        let mut guard = self.active_players.lock().expect("active_players poisoned");
        guard.insert(
            username.to_ascii_lowercase(),
            ActivePlayer {
                username,
                uuid,
                server,
                tx,
            },
        );
    }

    fn unregister_player(&self, username: &str, tx: &mpsc::UnboundedSender<PlayerControl>) {
        let mut guard = self.active_players.lock().expect("active_players poisoned");
        let key = username.to_ascii_lowercase();
        if let Some(existing) = guard.get(&key)
            && existing.tx.same_channel(tx)
        {
            guard.remove(&key);
        }
    }

    fn update_player_server(&self, username: &str, server: Option<String>) {
        let mut guard = self.active_players.lock().expect("active_players poisoned");
        if let Some(player) = guard.get_mut(&username.to_ascii_lowercase()) {
            player.server = server;
        }
    }

    /// Resolves either a direct `[servers]` key or a `[server_groups]` load-balanced group.
    #[must_use]
    pub fn resolve_named_target(
        &self,
        target: &str,
        cfg: &ProxyConfig,
    ) -> Option<(String, SocketAddr)> {
        self.resolve_named_target_filtered(target, cfg, None)
    }

    pub fn resolve_named_target_for_user(
        &self,
        username: &str,
        target: &str,
        cfg: &ProxyConfig,
    ) -> Option<(String, SocketAddr)> {
        self.resolve_named_target_filtered(target, cfg, Some(username))
    }

    fn resolve_named_target_filtered(
        &self,
        target: &str,
        cfg: &ProxyConfig,
        username: Option<&str>,
    ) -> Option<(String, SocketAddr)> {
        if let Some(&addr) = cfg.servers.get(target) {
            if let Some(user) = username
                && !can_access_server(user, target, cfg)
            {
                return None;
            }
            return Some((target.to_owned(), addr));
        }
        if let Some(user) = username
            && !can_access_server(user, target, cfg)
        {
            return None;
        }
        let group = cfg.server_groups.get(target)?;
        let valid: Vec<(&str, SocketAddr)> = group
            .iter()
            .filter(|m| username.is_none_or(|user| can_access_server(user, m, cfg)))
            .filter_map(|m| cfg.servers.get(m).map(|&addr| (m.as_str(), addr)))
            .collect();
        if valid.is_empty() {
            return None;
        }
        let chosen = match cfg.load_balance_strategy {
            LoadBalanceStrategy::LeastPlayers => {
                let counts = self.server_player_counts();
                valid
                    .into_iter()
                    .min_by_key(|(m, _)| counts.get(*m).copied().unwrap_or(0))?
            }
            LoadBalanceStrategy::RoundRobin => {
                let idx = self.motd_counter.fetch_add(1, Ordering::Relaxed) as usize % valid.len();
                valid[idx]
            }
        };
        Some((chosen.0.to_owned(), chosen.1))
    }

    /// Finds an online player's display name and current server name.
    #[must_use]
    pub fn find_player(&self, username: &str) -> Option<(String, Option<String>)> {
        let guard = self.active_players.lock().expect("active_players poisoned");
        guard
            .get(&username.to_ascii_lowercase())
            .map(|p| (p.username.clone(), p.server.clone()))
    }

    /// Sends a single player to `server_name` (`addr`). Returns `true` if the player is online.
    pub fn send_player(&self, username: &str, server_name: &str, addr: SocketAddr) -> bool {
        let guard = self.active_players.lock().expect("active_players poisoned");
        if let Some(player) = guard.get(&username.to_ascii_lowercase()) {
            let _ = player.tx.send(PlayerControl::Switch {
                name: server_name.to_owned(),
                addr,
            });
            return true;
        }
        false
    }

    /// Sends all online players to `server_name` (`addr`). Returns the number of players signalled.
    pub fn send_all(&self, server_name: &str, addr: SocketAddr) -> usize {
        let guard = self.active_players.lock().expect("active_players poisoned");
        let mut count = 0;
        for player in guard.values() {
            if player
                .tx
                .send(PlayerControl::Switch {
                    name: server_name.to_owned(),
                    addr,
                })
                .is_ok()
            {
                count += 1;
            }
        }
        count
    }

    /// Broadcasts a system chat alert to all online players. Returns the number of recipients.
    pub fn alert_all(&self, message: &str) -> usize {
        let guard = self.active_players.lock().expect("active_players poisoned");
        let mut count = 0;
        for player in guard.values() {
            if player
                .tx
                .send(PlayerControl::SystemChat(message.to_owned()))
                .is_ok()
            {
                count += 1;
            }
        }
        count
    }

    /// Kicks a specific online player with `reason`. Returns `true` if found.
    pub fn kick_player(&self, username: &str, reason: &str) -> bool {
        let guard = self.active_players.lock().expect("active_players poisoned");
        if let Some(player) = guard.get(&username.to_ascii_lowercase()) {
            let _ = player.tx.send(PlayerControl::Kick(reason.to_owned()));
            return true;
        }
        false
    }

    /// Kicks all online players (e.g. during graceful proxy shutdown).
    pub fn kick_all(&self, reason: &str) {
        let guard = self.active_players.lock().expect("active_players poisoned");
        for player in guard.values() {
            let _ = player.tx.send(PlayerControl::Kick(reason.to_owned()));
        }
    }

    fn sample_players(&self, limit: usize) -> Vec<PlayerSample> {
        let guard = self.active_players.lock().expect("active_players poisoned");
        guard
            .values()
            .take(limit)
            .map(|p| PlayerSample {
                name: p.username.clone(),
                id: p.uuid,
            })
            .collect()
    }

    fn online_player_names(&self, server_filter: Option<&str>) -> Vec<String> {
        let guard = self.active_players.lock().expect("active_players poisoned");
        guard
            .values()
            .filter(|p| server_filter.is_none_or(|srv| p.server.as_deref() == Some(srv)))
            .map(|p| p.username.clone())
            .collect()
    }

    /// Returns a snapshot of the current [`ProxyConfig`].
    #[must_use]
    pub fn current_config(&self) -> Arc<ProxyConfig> {
        self.config.load_full()
    }

    /// Returns a snapshot of the current [`RadonManager`].
    #[must_use]
    pub fn current_radons(&self) -> Arc<RadonManager> {
        self.radons.load_full()
    }

    /// Hot-reloads `rad.toml` and `radons/` from disk without dropping active players.
    pub fn reload(&self, config_path: &str) -> Result<usize> {
        let new_cfg = ProxyConfig::load_or_create(config_path)?;
        let mut new_radons = RadonManager::new();
        new_radons.load_from_dirs(&new_cfg.radons_dir, &new_cfg.radonfigs_dir)?;
        let count = new_radons.len();

        self.config.store(Arc::new(new_cfg));
        let prev_radons = self.radons.swap(Arc::new(new_radons));
        prev_radons.disable_all();
        self.status_cache.store(None);
        *self
            .passthrough_cache
            .lock()
            .expect("passthrough_cache poisoned") = None;
        Ok(count)
    }

    fn pick_motd(&self, cfg: &ProxyConfig) -> TextComponent {
        if cfg.motds.is_empty() {
            return cfg.motd.clone();
        }
        let idx = self.motd_counter.fetch_add(1, Ordering::Relaxed) as usize % cfg.motds.len();
        cfg.motds[idx].clone()
    }

    /// Returns a pre-encoded `StatusResponse` wire frame, rebuilding when needed.
    async fn status_frame(&self, cfg: &ProxyConfig, client_protocol: i32) -> Option<Bytes> {
        let current_online = self.online_players.load(Ordering::Relaxed);

        if cfg.ping_passthrough == PingPassthroughMode::None
            && cfg.motds.is_empty()
            && !cfg.sample_players_in_ping
        {
            let guard = self.status_cache.load();
            if let Some(cached) = guard.as_ref()
                && cached.online_count == current_online
                && cached.protocol_version == client_protocol
            {
                return Some(cached.encoded_frame.clone());
            }
            drop(guard);
        }

        let passthrough = if cfg.ping_passthrough != PingPassthroughMode::None {
            self.fetch_passthrough_status(cfg, client_protocol).await
        } else {
            None
        };

        let sample = if cfg.sample_players_in_ping {
            self.sample_players(12)
        } else {
            Vec::new()
        };

        let mut payload = StatusPayload {
            version: StatusVersion {
                name: format!("RadByte {MINECRAFT_VERSION}"),
                protocol: client_protocol,
            },
            players: Some(StatusPlayers {
                max: cfg.max_players,
                online: current_online,
                sample,
            }),
            description: self.pick_motd(cfg),
            favicon: cfg.favicon.clone(),
            enforces_secure_chat: Some(cfg.online_mode),
        };

        if let Some(backend_payload) = passthrough {
            match cfg.ping_passthrough {
                PingPassthroughMode::None => {}
                PingPassthroughMode::Players => {
                    if backend_payload.players.is_some() {
                        payload.players = backend_payload.players;
                    }
                }
                PingPassthroughMode::Description => {
                    payload.description = backend_payload.description;
                }
                PingPassthroughMode::All => {
                    payload = backend_payload;
                }
            }
        }

        let resp = StatusResponse::from_payload(&payload).ok()?;
        let mut buf = BytesMut::with_capacity(256);
        encode_packet(&resp, &mut buf).ok()?;
        let frozen = buf.freeze();

        if cfg.ping_passthrough == PingPassthroughMode::None
            && cfg.motds.is_empty()
            && !cfg.sample_players_in_ping
        {
            self.status_cache.store(Some(Arc::new(CachedStatus {
                online_count: current_online,
                protocol_version: client_protocol,
                encoded_frame: frozen.clone(),
            })));
        }

        Some(frozen)
    }

    async fn fetch_passthrough_status(
        &self,
        cfg: &ProxyConfig,
        client_protocol: i32,
    ) -> Option<StatusPayload> {
        {
            let guard = self
                .passthrough_cache
                .lock()
                .expect("passthrough_cache poisoned");
            if let Some(cached) = guard.as_ref()
                && cached.protocol_version == client_protocol
                && cached.fetched_at.elapsed() < Duration::from_secs(3)
            {
                return Some(cached.payload.clone());
            }
        }

        let target = cfg.backend_addr.or_else(|| {
            cfg.try_servers
                .iter()
                .find_map(|n| cfg.servers.get(n).copied())
        })?;
        let mut stream = timeout(cfg.connect_timeout, TcpStream::connect(target))
            .await
            .ok()?
            .ok()?;

        let mut out = BytesMut::with_capacity(64);
        encode_packet(
            &Handshake {
                protocol_version: VarInt(client_protocol),
                server_address: target.ip().to_string(),
                server_port: target.port(),
                next_state: ConnectionState::Status,
            },
            &mut out,
        )
        .ok()?;
        encode_packet(&StatusRequest, &mut out).ok()?;
        stream.write_all(&out).await.ok()?;

        let mut in_buf = BytesMut::with_capacity(1024);
        let frame = timeout(
            cfg.connect_timeout,
            read_next_frame(&mut stream, &mut in_buf, cfg.max_packet_size),
        )
        .await
        .ok()?
        .ok()??;
        let resp = frame.decode_as::<StatusResponse>().ok()?;
        let parsed: StatusPayload = serde_json::from_str(&resp.json).ok()?;

        let mut guard = self
            .passthrough_cache
            .lock()
            .expect("passthrough_cache poisoned");
        *guard = Some(CachedPassthrough {
            protocol_version: client_protocol,
            fetched_at: Instant::now(),
            payload: parsed.clone(),
        });
        Some(parsed)
    }
}

/// Authenticated (or offline) player identity established during `Login`.
#[derive(Debug, Clone)]
struct PlayerIdentity {
    uuid: Uuid,
    username: String,
    properties: Vec<GameProfileProperty>,
}

/// Handles a single incoming TCP connection from a Minecraft client.
pub async fn handle_connection(
    mut raw_client: TcpStream,
    mut peer_addr: SocketAddr,
    state: Arc<ProxyState>,
) {
    state.metrics.record_connection();

    let cfg = state.config.load();
    let radons = state.radons.load();

    let mut stack_buf = [0u8; 512];
    let mut filled = 0usize;
    let mut pos = 0usize;

    if cfg.haproxy_protocol {
        let trusted =
            cfg.haproxy_trusted_proxies.contains(&peer_addr.ip()) || cfg.haproxy_allow_untrusted;
        if !trusted {
            warn!(%peer_addr, "rejected HAProxy connection from untrusted peer");
            return;
        }
        match timeout(
            cfg.handshake_timeout,
            read_haproxy_peer(&mut raw_client, &mut stack_buf, &mut filled, peer_addr),
        )
        .await
        {
            Ok(Ok(real_addr)) => peer_addr = real_addr,
            Ok(Err(err)) => {
                debug!(%peer_addr, %err, "invalid HAProxy PROXY header");
                return;
            }
            Err(_) => {
                debug!(%peer_addr, "timed out waiting for HAProxy PROXY header");
                return;
            }
        }
    }

    if cfg.banned_ips.contains(&peer_addr.ip()) {
        state
            .metrics
            .blocked_by_filter
            .fetch_add(1, Ordering::Relaxed);
        debug!(%peer_addr, "connection rejected: IP is banned");
        return;
    }

    let Some(_ip_guard) = state.ip_filter.try_acquire(peer_addr.ip()) else {
        state
            .metrics
            .blocked_by_filter
            .fetch_add(1, Ordering::Relaxed);
        debug!(%peer_addr, "connection rejected by IP filter / rate limiter");
        return;
    };

    let _ = raw_client.set_nodelay(true);
    let mut client = EncryptedStream::new(raw_client);

    let first_frame = match timeout(
        cfg.handshake_timeout,
        read_stack_frame(
            &mut client,
            &mut stack_buf,
            &mut filled,
            cfg.max_packet_size,
        ),
    )
    .await
    {
        Ok(Ok(StackFrame::Ready {
            id,
            payload_start,
            frame_end,
        })) => {
            pos = frame_end;
            Some((id, &stack_buf[payload_start..frame_end]))
        }
        Ok(Ok(StackFrame::Eof)) => {
            debug!(%peer_addr, "client disconnected before handshake");
            return;
        }
        Ok(Ok(StackFrame::TooLarge)) => None,
        Ok(Err(err)) => {
            debug!(%peer_addr, %err, "invalid handshake frame");
            return;
        }
        Err(_) => {
            debug!(%peer_addr, "handshake timed out");
            return;
        }
    };

    if let Some((frame_id, payload)) = first_frame {
        if frame_id != Handshake::ID {
            debug!(%peer_addr, id = frame_id, "expected Handshake packet");
            return;
        }

        if radons.len() == 0 {
            match Handshake::peek_next_state(payload) {
                Ok(ConnectionState::Status) => {
                    let mut peek = payload;
                    let client_protocol =
                        VarInt::decode(&mut peek).map_or(PROTOCOL_VERSION, |v| v.0);
                    let _ = timeout(
                        cfg.handshake_timeout,
                        handle_status_stack(
                            &mut client,
                            &mut stack_buf,
                            &mut pos,
                            &mut filled,
                            peer_addr,
                            client_protocol,
                            &state,
                            &cfg,
                        ),
                    )
                    .await;
                    return;
                }
                Ok(ConnectionState::Login) => {}
                Ok(other) => {
                    debug!(%peer_addr, state = %other, "unsupported post-handshake state");
                    return;
                }
                Err(err) => {
                    debug!(%peer_addr, %err, "failed to decode Handshake");
                    return;
                }
            }
        }
    }

    let mut read_buf = BytesMut::with_capacity(4096);
    let handshake: Handshake = if let Some((frame_id, mut payload)) = first_frame {
        if frame_id != Handshake::ID {
            debug!(%peer_addr, id = frame_id, "expected Handshake packet");
            return;
        }
        if pos < filled {
            read_buf.extend_from_slice(&stack_buf[pos..filled]);
        }
        match Handshake::decode_body(&mut payload) {
            Ok(hs) => hs,
            Err(err) => {
                debug!(%peer_addr, %err, "failed to decode Handshake");
                return;
            }
        }
    } else {
        read_buf.extend_from_slice(&stack_buf[..filled]);
        let handshake_frame = match timeout(
            cfg.handshake_timeout,
            read_next_frame(&mut client, &mut read_buf, cfg.max_packet_size),
        )
        .await
        {
            Ok(Ok(Some(frame))) => frame,
            Ok(Ok(None)) => {
                debug!(%peer_addr, "client disconnected before handshake");
                return;
            }
            Ok(Err(err)) => {
                debug!(%peer_addr, %err, "invalid handshake frame");
                return;
            }
            Err(_) => {
                debug!(%peer_addr, "handshake timed out");
                return;
            }
        };

        if handshake_frame.id != Handshake::ID {
            debug!(%peer_addr, id = handshake_frame.id, "expected Handshake packet");
            return;
        }

        match handshake_frame.decode_as() {
            Ok(hs) => hs,
            Err(err) => {
                debug!(%peer_addr, %err, "failed to decode Handshake");
                return;
            }
        }
    };

    let mut target_backend = resolve_forced_host(&handshake.server_address, &cfg)
        .map(|(_, addr)| addr)
        .or(cfg.backend_addr);
    if radons.len() > 0 {
        let ctx = RadonContext {
            peer_addr,
            online_players: state.online_players.load(Ordering::Relaxed),
        };
        match radons.dispatch_handshake(ctx, &handshake) {
            RadonAction::Continue => {}
            RadonAction::RouteTo(addr) => target_backend = Some(addr),
            RadonAction::Deny(reason) => {
                state
                    .metrics
                    .blocked_by_radon
                    .fetch_add(1, Ordering::Relaxed);
                if handshake.next_state == ConnectionState::Login {
                    send_login_disconnect(&mut client, &reason).await;
                }
                return;
            }
        }
    }

    match handshake.next_state {
        ConnectionState::Status => {
            let _ = timeout(
                cfg.handshake_timeout,
                handle_status(
                    &mut client,
                    &mut read_buf,
                    peer_addr,
                    handshake.protocol_version.0,
                    &state,
                    &cfg,
                ),
            )
            .await;
        }
        ConnectionState::Login => {
            handle_login(
                client,
                read_buf,
                peer_addr,
                &handshake,
                target_backend,
                &state,
                &cfg,
                &radons,
            )
            .await;
        }
        other => {
            debug!(%peer_addr, state = %other, "unsupported post-handshake state");
        }
    }
}

async fn handle_status(
    client: &mut EncryptedStream,
    read_buf: &mut BytesMut,
    peer_addr: SocketAddr,
    client_protocol: i32,
    state: &ProxyState,
    cfg: &ProxyConfig,
) {
    loop {
        let Ok(Some(frame)) = read_next_frame(client, read_buf, cfg.max_packet_size).await else {
            return;
        };

        match frame.id {
            StatusRequest::ID => {
                state.metrics.record_status_ping();
                let Some(encoded) = state.status_frame(cfg, client_protocol).await else {
                    return;
                };
                if client.write_all(&encoded).await.is_err() {
                    return;
                }
                debug!(%peer_addr, "sent StatusResponse");
            }
            PingRequest::ID => {
                let Ok(ping) = frame.decode_as::<PingRequest>() else {
                    return;
                };
                let ts = ping.timestamp.to_be_bytes();
                let pong_frame: [u8; 10] = [
                    0x09,
                    PongResponse::ID as u8,
                    ts[0],
                    ts[1],
                    ts[2],
                    ts[3],
                    ts[4],
                    ts[5],
                    ts[6],
                    ts[7],
                ];
                let _ = client.write_all(&pong_frame).await;
                debug!(%peer_addr, "sent PongResponse");
                return;
            }
            other => {
                debug!(%peer_addr, id = other, "unexpected packet in Status state");
                return;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn read_stack_frame<S: AsyncRead + Unpin>(
    stream: &mut S,
    buf: &mut [u8],
    filled: &mut usize,
    max_packet_size: usize,
) -> Result<StackFrame> {
    loop {
        if *filled > 0 {
            let mut peek = &buf[..*filled];
            match VarInt::decode(&mut peek) {
                Ok(frame_len) => {
                    let frame_len = frame_len.0;
                    if frame_len <= 0 || (frame_len as usize) > max_packet_size {
                        return Err(Error::InvalidPacketLength {
                            length: frame_len,
                            max: max_packet_size,
                        });
                    }
                    let frame_len = frame_len as usize;
                    let header_len = *filled - peek.len();
                    let frame_end = header_len + frame_len;
                    if frame_end > buf.len() {
                        return Ok(StackFrame::TooLarge);
                    }
                    if *filled >= frame_end {
                        let mut frame_slice = &buf[header_len..frame_end];
                        let id = VarInt::decode(&mut frame_slice)?.0;
                        let payload_start = frame_end - frame_slice.len();
                        return Ok(StackFrame::Ready {
                            id,
                            payload_start,
                            frame_end,
                        });
                    }
                }
                Err(Error::UnexpectedEof { .. }) => {
                    if *filled == buf.len() {
                        return Ok(StackFrame::TooLarge);
                    }
                }
                Err(e) => return Err(e),
            }
        }

        let n = stream.read(&mut buf[*filled..]).await?;
        if n == 0 {
            return Ok(StackFrame::Eof);
        }
        *filled += n;
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_status_stack(
    client: &mut EncryptedStream,
    buf: &mut [u8],
    pos: &mut usize,
    filled: &mut usize,
    peer_addr: SocketAddr,
    client_protocol: i32,
    state: &ProxyState,
    cfg: &ProxyConfig,
) {
    loop {
        if *pos > 0 {
            let rem = *filled - *pos;
            if rem > 0 {
                buf.copy_within(*pos..*filled, 0);
            }
            *filled = rem;
            *pos = 0;
        }

        let Ok(StackFrame::Ready {
            id,
            payload_start,
            frame_end,
        }) = read_stack_frame(client, buf, filled, cfg.max_packet_size).await
        else {
            return;
        };
        *pos = frame_end;

        match id {
            StatusRequest::ID => {
                state.metrics.record_status_ping();
                let Some(encoded) = state.status_frame(cfg, client_protocol).await else {
                    return;
                };
                if client.write_all(&encoded).await.is_err() {
                    return;
                }
                debug!(%peer_addr, "sent StatusResponse");
            }
            PingRequest::ID => {
                let mut payload = &buf[payload_start..frame_end];
                let Ok(ping) = PingRequest::decode_body(&mut payload) else {
                    return;
                };
                let ts = ping.timestamp.to_be_bytes();
                let pong_frame: [u8; 10] = [
                    0x09,
                    PongResponse::ID as u8,
                    ts[0],
                    ts[1],
                    ts[2],
                    ts[3],
                    ts[4],
                    ts[5],
                    ts[6],
                    ts[7],
                ];
                let _ = client.write_all(&pong_frame).await;
                debug!(%peer_addr, "sent PongResponse");
                return;
            }
            other => {
                debug!(%peer_addr, id = other, "unexpected packet in Status state");
                return;
            }
        }
    }
}

/// Reads bytes from `stream` into `buf` until one complete [`RawPacket`] frame is decoded.
async fn read_next_frame<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    buf: &mut BytesMut,
    max_packet_size: usize,
) -> Result<Option<RawPacket>> {
    read_next_frame_with_compression(stream, buf, max_packet_size, None).await
}

async fn read_next_frame_with_compression<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    buf: &mut BytesMut,
    max_packet_size: usize,
    compression: Option<CompressionConfig>,
) -> Result<Option<RawPacket>> {
    loop {
        if !buf.is_empty()
            && let Some(frame) = decode_frame_with_compression(buf, max_packet_size, compression)?
        {
            return Ok(Some(frame));
        }
        let n = stream.read_buf(buf).await?;
        if n == 0 {
            return Ok(None);
        }
    }
}
