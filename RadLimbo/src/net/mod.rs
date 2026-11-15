//! Connection lifecycle handler for `RadLimbo`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use bytes::{BufMut, BytesMut};
use rad_core::codec::{DEFAULT_MAX_PACKET_SIZE, decode_frame, encode_packet};
use rad_core::forwarding::{VELOCITY_FORWARDING_CHANNEL, decode_velocity_forwarding};
use rad_core::nbt::NbtTag;
use rad_core::packet::Packet;
use rad_core::packet::configuration::{
    AcknowledgeFinishConfiguration, ClientboundKnownPacks, ConfigClientboundPluginMessage,
    FinishConfiguration, KnownPack, RegistryData,
};
use rad_core::packet::handshake::Handshake;
use rad_core::packet::login::{
    LoginAcknowledged, LoginDisconnect, LoginPluginRequest, LoginPluginResponse, LoginStart,
    LoginSuccess,
};
use rad_core::packet::play::{
    ChatCommand, ConfirmTeleportation, GameEvent, KeepAliveClientbound, KeepAliveServerbound,
    PlayClientboundPluginMessage, PlayLogin, SetChunkCacheCenter, SetDefaultSpawnPosition,
    SynchronizePlayerPosition, SystemChatMessage,
};
use rad_core::packet::status::{PingRequest, PongResponse, StatusRequest, StatusResponse};
use rad_core::registry_provider::RegistryProvider;
use rad_core::state::ConnectionState;
use rad_core::text::TextComponent;
use rad_core::types::{BlockPos, VarInt, write_string};
use rad_core::{MINECRAFT_VERSION, PROTOCOL_VERSION};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{interval, timeout};
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::config::{LimboConfig, LimboForwardingMode};

/// Shared runtime state for `RadLimbo`.
pub struct LimboState {
    /// Active configuration.
    pub config: LimboConfig,
    /// Per-boot server session UUID sent in `login_finished`.
    pub server_session_id: Uuid,
    /// Number of players currently inside the Limbo Play state.
    pub online_players: AtomicUsize,
    /// Optional registry/tag provider for Configuration state.
    pub registry_provider: Option<Arc<dyn RegistryProvider>>,
}

impl LimboState {
    /// Creates a new [`LimboState`] without a registry provider.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn new(config: LimboConfig) -> Self {
        Self::with_registry_provider(config, None)
    }

    /// Creates a new [`LimboState`] with an optional [`RegistryProvider`].
    #[must_use]
    pub fn with_registry_provider(
        config: LimboConfig,
        registry_provider: Option<Arc<dyn RegistryProvider>>,
    ) -> Self {
        let boot_seed = format!(
            "radlimbo:{}:{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        );
        Self {
            config,
            server_session_id: Uuid::new_v3(&Uuid::NAMESPACE_OID, boot_seed.as_bytes()),
            online_players: AtomicUsize::new(0),
            registry_provider,
        }
    }
}

struct OnlineGuard<'a> {
    counter: &'a AtomicUsize,
}

impl Drop for OnlineGuard<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Handles a single accepted TCP connection to `RadLimbo`.
pub async fn handle_connection(
    mut stream: TcpStream,
    peer_addr: SocketAddr,
    state: Arc<LimboState>,
) {
    let _ = stream.set_nodelay(true);
    let mut read_buf = BytesMut::with_capacity(4096);
    let max_packet = state.config.max_packet_size.max(DEFAULT_MAX_PACKET_SIZE);

    let hs = match timeout(
        Duration::from_secs(5),
        read_typed_packet::<Handshake>(&mut stream, &mut read_buf, max_packet),
    )
    .await
    {
        Ok(Some(hs)) => hs,
        _ => return,
    };

    match hs.next_state {
        ConnectionState::Status => handle_status(&mut stream, &mut read_buf, &state).await,
        ConnectionState::Login => {
            handle_login_and_play(&mut stream, &mut read_buf, peer_addr, &hs, &state).await;
        }
        ConnectionState::Handshaking | ConnectionState::Configuration | ConnectionState::Play => {}
    }
}

async fn handle_status(stream: &mut TcpStream, read_buf: &mut BytesMut, state: &LimboState) {
    let max_packet = state.config.max_packet_size;
    if timeout(
        Duration::from_secs(5),
        read_typed_packet::<StatusRequest>(stream, read_buf, max_packet),
    )
    .await
    .ok()
    .flatten()
    .is_none()
    {
        return;
    }

    let online = state.online_players.load(Ordering::Relaxed);
    let payload = json!({
        "version": { "name": MINECRAFT_VERSION, "protocol": PROTOCOL_VERSION },
        "players": { "max": state.config.max_players, "online": online },
        "description": { "text": state.config.motd },
    })
    .to_string();

    if send_packet(stream, &StatusResponse { json: payload })
        .await
        .is_err()
    {
        return;
    }

    if let Ok(Some(ping)) = timeout(
        Duration::from_secs(5),
        read_typed_packet::<PingRequest>(stream, read_buf, max_packet),
    )
    .await
    {
        let _ = send_packet(
            stream,
            &PongResponse {
                timestamp: ping.timestamp,
            },
        )
        .await;
    }
}

async fn handle_login_and_play(
    stream: &mut TcpStream,
    read_buf: &mut BytesMut,
    peer_addr: SocketAddr,
    hs: &Handshake,
    state: &LimboState,
) {
    let cfg = &state.config;
    let login_start = match timeout(
        Duration::from_secs(10),
        read_typed_packet::<LoginStart>(stream, read_buf, cfg.max_packet_size),
    )
    .await
    {
        Ok(Some(pkt)) => pkt,
        _ => return,
    };

    let (player_uuid, username) =
        match verify_forwarding(stream, read_buf, hs, &login_start, cfg).await {
            Ok(identity) => identity,
            Err(reason) => {
                warn!(%peer_addr, player = %login_start.username, %reason, "rejected limbo login");
                let json_reason = TextComponent::text(reason)
                    .to_json()
                    .unwrap_or_else(|_| format!(r#"{{"text":"{reason}"}}"#));
                let _ = send_packet(
                    stream,
                    &LoginDisconnect {
                        reason_json: json_reason,
                    },
                )
                .await;
                return;
            }
        };

    if send_packet(
        stream,
        &LoginSuccess {
            uuid: player_uuid,
            username: username.clone(),
            properties: Vec::new(),
            session_id: state.server_session_id,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    // Wait for client's LoginAcknowledged (0x03) to enter Configuration.
    if timeout(
        Duration::from_secs(10),
        read_typed_packet::<LoginAcknowledged>(stream, read_buf, cfg.max_packet_size),
    )
    .await
    .ok()
    .flatten()
    .is_none()
    {
        return;
    }

    if !run_configuration(stream, read_buf, cfg, state.registry_provider.as_deref()).await {
        return;
    }

    run_play(stream, read_buf, peer_addr, &username, state).await;
}

async fn verify_forwarding(
    stream: &mut TcpStream,
    read_buf: &mut BytesMut,
    hs: &Handshake,
    login_start: &LoginStart,
    cfg: &LimboConfig,
) -> Result<(Uuid, String), &'static str> {
    match cfg.forwarding_mode {
        LimboForwardingMode::None => Ok((login_start.player_uuid, login_start.username.clone())),
        LimboForwardingMode::Legacy => parse_legacy_handshake(&hs.server_address, None)
            .map(|uuid| (uuid, login_start.username.clone())),
        LimboForwardingMode::BungeeGuard => {
            let token = std::str::from_utf8(&cfg.forwarding_secret).unwrap_or("");
            parse_legacy_handshake(&hs.server_address, Some(token))
                .map(|uuid| (uuid, login_start.username.clone()))
        }
        LimboForwardingMode::Modern => {
            let req = LoginPluginRequest {
                message_id: VarInt(1),
                channel: VELOCITY_FORWARDING_CHANNEL.into(),
                data: Vec::new(),
            };
            if send_packet(stream, &req).await.is_err() {
                return Err("Failed to send Velocity forwarding request");
            }
            let resp = timeout(
                Duration::from_secs(5),
                read_typed_packet::<LoginPluginResponse>(stream, read_buf, cfg.max_packet_size),
            )
            .await
            .ok()
            .flatten()
            .ok_or("Timed out waiting for Velocity forwarding response")?;
            if !resp.successful || resp.message_id.0 != 1 {
                return Err("Proxy did not provide Velocity forwarding data");
            }
            let fwd = decode_velocity_forwarding(&resp.data, &cfg.forwarding_secret)
                .map_err(|_| "Invalid Velocity forwarding signature")?;
            Ok((fwd.uuid, fwd.username))
        }
    }
}

fn parse_legacy_handshake(
    server_address: &str,
    required_bungeeguard_token: Option<&str>,
) -> Result<Uuid, &'static str> {
    let parts: Vec<&str> = server_address.split('\0').collect();
    if parts.len() < 3 {
        return Err("Missing legacy forwarding data in Handshake");
    }
    let uuid = Uuid::parse_str(parts[2]).map_err(|_| "Invalid forwarded UUID")?;
    if let Some(expected_token) = required_bungeeguard_token {
        let props_json = parts.get(3).ok_or("Missing BungeeGuard properties")?;
        let props: Vec<serde_json::Value> =
            serde_json::from_str(props_json).map_err(|_| "Invalid BungeeGuard JSON")?;
        let has_valid_token = props.iter().any(|p| {
            p.get("name").and_then(|v| v.as_str()) == Some("bungeeguard-token")
                && p.get("value").and_then(|v| v.as_str()) == Some(expected_token)
        });
        if !has_valid_token {
            return Err("Invalid BungeeGuard token");
        }
    }
    Ok(uuid)
}

async fn run_configuration(
    stream: &mut TcpStream,
    read_buf: &mut BytesMut,
    cfg: &LimboConfig,
    registry_provider: Option<&dyn RegistryProvider>,
) -> bool {
    let mut batch = BytesMut::with_capacity(1024);

    let mut brand_payload = BytesMut::new();
    if write_string(&mut brand_payload, &cfg.brand, 32_767).is_err() {
        return false;
    }
    if encode_packet(
        &ConfigClientboundPluginMessage {
            channel: "minecraft:brand".into(),
            data: brand_payload.to_vec(),
        },
        &mut batch,
    )
    .is_err()
    {
        return false;
    }

    if encode_packet(
        &ClientboundKnownPacks {
            packs: vec![KnownPack {
                namespace: "minecraft".into(),
                id: "core".into(),
                version: MINECRAFT_VERSION.into(),
            }],
        },
        &mut batch,
    )
    .is_err()
    {
        return false;
    }

    if stream.write_all(&batch).await.is_err() {
        return false;
    }
    batch.clear();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let client_has_core = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let frame = match timeout(
            remaining,
            read_raw_frame(stream, read_buf, cfg.max_packet_size),
        )
        .await
        {
            Ok(Some(frame)) => frame,
            _ => return false,
        };
        if frame.id == rad_core::packet::configuration::ServerboundKnownPacks::ID {
            let Ok(known) =
                frame.decode_as::<rad_core::packet::configuration::ServerboundKnownPacks>()
            else {
                return false;
            };
            break known.packs.iter().any(|p| {
                p.namespace == "minecraft" && p.id == "core" && p.version == MINECRAFT_VERSION
            });
        }
    };

    let registries = if client_has_core {
        RegistryData::minimal_vanilla_26_3()
    } else {
        RegistryData::minimal_fallback_nbt_26_3()
    };
    for reg in &registries {
        if encode_packet(reg, &mut batch).is_err() {
            return false;
        }
    }

    let Some(provider) = registry_provider else {
        tracing::warn!("missing configuration tag provider; refusing configuration handshake");
        return false;
    };
    let Ok(tags) = rad_core::packet::configuration::ConfigUpdateTags::for_registries_with_provider(
        &registries,
        provider,
    ) else {
        tracing::warn!("missing configuration tag tables; refusing configuration handshake");
        return false;
    };
    if encode_packet(&tags, &mut batch).is_err() {
        return false;
    }

    if encode_packet(&FinishConfiguration, &mut batch).is_err() {
        return false;
    }

    if stream.write_all(&batch).await.is_err() {
        return false;
    }

    // Drain ClientInformation / PluginMessages until AcknowledgeFinishConfiguration.
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let frame = match timeout(
            remaining,
            read_raw_frame(stream, read_buf, cfg.max_packet_size),
        )
        .await
        {
            Ok(Some(frame)) => frame,
            _ => return false,
        };
        if frame.id == AcknowledgeFinishConfiguration::ID {
            return true;
        }
    }
}

async fn run_play(
    stream: &mut TcpStream,
    read_buf: &mut BytesMut,
    peer_addr: SocketAddr,
    username: &str,
    state: &LimboState,
) {
    let cfg = &state.config;
    state.online_players.fetch_add(1, Ordering::Relaxed);
    let _guard = OnlineGuard {
        counter: &state.online_players,
    };
    info!(%peer_addr, %username, "player entered RadLimbo");

    let mut batch = BytesMut::with_capacity(1024);
    let play_login = PlayLogin {
        entity_id: 1,
        is_hardcore: false,
        dimension_names: vec!["minecraft:overworld".into()],
        max_players: VarInt(cfg.max_players),
        view_distance: VarInt(cfg.view_distance),
        simulation_distance: VarInt(cfg.view_distance),
        reduced_debug_info: false,
        enable_respawn_screen: false,
        do_limited_crafting: false,
        dimension_type: VarInt(0),
        dimension_name: "minecraft:overworld".into(),
        hashed_seed: 0,
        game_mode: cfg.game_mode,
        previous_game_mode: -1,
        is_debug: false,
        is_flat: true,
        portal_cooldown: VarInt(0),
        sea_level: VarInt(64),
        online_mode: false,
        enforces_secure_chat: false,
    };
    if encode_packet(&play_login, &mut batch).is_err() {
        return;
    }
    if encode_packet(
        &SetChunkCacheCenter {
            chunk_x: VarInt(0),
            chunk_z: VarInt(0),
        },
        &mut batch,
    )
    .is_err()
    {
        return;
    }
    if encode_packet(
        &SetDefaultSpawnPosition {
            dimension_name: "minecraft:overworld".into(),
            location: BlockPos::new(cfg.spawn_x as i32, cfg.spawn_y as i32, cfg.spawn_z as i32),
            yaw: 0.0,
            pitch: 0.0,
        },
        &mut batch,
    )
    .is_err()
    {
        return;
    }
    if encode_packet(
        &GameEvent {
            event: GameEvent::START_WAITING_FOR_CHUNKS,
            value: 0.0,
        },
        &mut batch,
    )
    .is_err()
    {
        return;
    }
    if encode_packet(
        &SynchronizePlayerPosition {
            teleport_id: VarInt(1),
            x: cfg.spawn_x,
            y: cfg.spawn_y,
            z: cfg.spawn_z,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
        },
        &mut batch,
    )
    .is_err()
    {
        return;
    }
    if let Some(ref msg) = cfg.welcome_message
        && encode_packet(
            &SystemChatMessage {
                content: NbtTag::String(msg.clone()),
                overlay: false,
            },
            &mut batch,
        )
        .is_err()
    {
        return;
    }

    if stream.write_all(&batch).await.is_err() {
        return;
    }

    let mut ka_ticker = interval(cfg.keep_alive_interval);
    ka_ticker.tick().await;
    let ka_timeout = cfg.keep_alive_interval.saturating_mul(2);
    let mut keep_alive_seq: i64 = 1;
    let mut pending_ka: Option<(i64, tokio::time::Instant)> = None;

    loop {
        let ka_deadline = async {
            match pending_ka {
                Some((_, sent_at)) => tokio::time::sleep_until(sent_at + ka_timeout).await,
                None => std::future::pending::<()>().await,
            }
        };

        tokio::select! {
            _ = ka_deadline => {
                warn!(%username, "limbo keep-alive timed out");
                let _ = send_packet(stream, &rad_core::packet::play::PlayDisconnect {
                    reason: NbtTag::String("Timed out".into()),
                }).await;
                break;
            }
            _ = ka_ticker.tick() => {
                if pending_ka.is_none() {
                    keep_alive_seq = keep_alive_seq.wrapping_add(1);
                    if send_packet(stream, &KeepAliveClientbound { id: keep_alive_seq }).await.is_err() {
                        break;
                    }
                    pending_ka = Some((keep_alive_seq, tokio::time::Instant::now()));
                }
            }
            frame_opt = read_raw_frame(stream, read_buf, cfg.max_packet_size) => {
                let Some(frame) = frame_opt else { break; };
                if frame.id == KeepAliveServerbound::ID {
                    if let Ok(ka) = frame.decode_as::<KeepAliveServerbound>() {
                        match pending_ka {
                            Some((expected, _)) if ka.id == expected => {
                                pending_ka = None;
                                debug!(%username, id = ka.id, "limbo keep-alive ack");
                            }
                            Some((expected, _)) => {
                                warn!(%username, expected, got = ka.id, "limbo mismatched keep-alive id");
                                let _ = send_packet(stream, &rad_core::packet::play::PlayDisconnect {
                                    reason: NbtTag::String("Invalid keep-alive".into()),
                                }).await;
                                break;
                            }
                            None => {
                                warn!(%username, got = ka.id, "limbo unexpected keep-alive");
                                break;
                            }
                        }
                    }
                } else if frame.id == ConfirmTeleportation::ID {
                    let _ = frame.decode_as::<ConfirmTeleportation>();
                } else if frame.id == ChatCommand::ID
                    && let Ok(cmd) = frame.decode_as::<ChatCommand>()
                {
                    let trimmed = cmd.command.trim();
                    if (trimmed.eq_ignore_ascii_case("lobby") || trimmed.eq_ignore_ascii_case("hub"))
                        && let Some(ref target) = cfg.reconnect_server
                    {
                        let mut data = BytesMut::new();
                        write_bungee_utf(&mut data, "Connect");
                        write_bungee_utf(&mut data, target);
                        let pkt = PlayClientboundPluginMessage {
                            channel: "bungeecord:main".into(),
                            data: data.to_vec(),
                        };
                        if send_packet(stream, &pkt).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }

    info!(%peer_addr, %username, "player left RadLimbo");
}

fn write_bungee_utf(buf: &mut BytesMut, s: &str) {
    let bytes = s.as_bytes();
    let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
    buf.put_u16(len);
    buf.put_slice(&bytes[..usize::from(len)]);
}

async fn send_packet<P: Packet>(stream: &mut TcpStream, pkt: &P) -> std::io::Result<()> {
    let mut out = BytesMut::new();
    encode_packet(pkt, &mut out)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    stream.write_all(&out).await
}

async fn read_raw_frame(
    stream: &mut TcpStream,
    buf: &mut BytesMut,
    max_packet: usize,
) -> Option<rad_core::codec::RawPacket> {
    let max_buf = max_packet.saturating_add(5);
    loop {
        match decode_frame(buf, max_packet) {
            Ok(Some(frame)) => return Some(frame),
            Ok(None) => {}
            Err(_) => return None,
        }
        if buf.len() >= max_buf {
            return None;
        }
        match stream.read_buf(buf).await {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
    }
}

async fn read_typed_packet<P: Packet>(
    stream: &mut TcpStream,
    buf: &mut BytesMut,
    max_packet: usize,
) -> Option<P> {
    let frame = read_raw_frame(stream, buf, max_packet).await?;
    frame.decode_as::<P>().ok()
}

#[cfg(test)]
mod tests;
