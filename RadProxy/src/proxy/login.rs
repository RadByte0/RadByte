//! Login, Mojang authentication, and Velocity forwarding for `RadProxy`.

use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_login(
    mut client: EncryptedStream,
    mut read_buf: BytesMut,
    peer_addr: SocketAddr,
    handshake: &Handshake,
    mut target_backend: Option<SocketAddr>,
    state: &ProxyState,
    cfg: &ProxyConfig,
    radons: &RadonManager,
) {
    read_buf.reserve(4096);
    let login_frame = match timeout(
        cfg.handshake_timeout,
        read_next_frame(&mut client, &mut read_buf, cfg.max_packet_size),
    )
    .await
    {
        Ok(Ok(Some(frame))) if frame.id == LoginStart::ID => frame,
        _ => return,
    };

    let legacy_protocol = handshake.protocol_version.0 < 764;
    let login = if legacy_protocol {
        let mut p = &login_frame.payload[..];
        let Ok(username) = rad_core::types::read_string(&mut p, LoginStart::MAX_USERNAME_LEN)
        else {
            return;
        };
        let player_uuid = Uuid::new_v3(
            &Uuid::NAMESPACE_OID,
            format!("OfflinePlayer:{username}").as_bytes(),
        );
        LoginStart {
            username,
            player_uuid,
        }
    } else {
        let Ok(login) = login_frame.decode_as::<LoginStart>() else {
            return;
        };
        login
    };

    let ctx = RadonContext {
        peer_addr,
        online_players: state.online_players.load(Ordering::Relaxed),
    };

    match radons.dispatch_login(ctx, handshake, &login) {
        RadonAction::Continue => {}
        RadonAction::RouteTo(addr) => target_backend = Some(addr),
        RadonAction::Deny(reason) => {
            state
                .metrics
                .blocked_by_radon
                .fetch_add(1, Ordering::Relaxed);
            send_login_disconnect(&mut client, &reason).await;
            return;
        }
    }

    // Perform online-mode RSA + AES-128-CFB8 + Mojang session authentication if enabled.
    let identity = if cfg.online_mode {
        match authenticate_online_client(&mut client, &mut read_buf, &login, peer_addr, state, cfg)
            .await
        {
            Ok(id) => id,
            Err(reason) => {
                send_login_disconnect(&mut client, &reason).await;
                return;
            }
        }
    } else {
        PlayerIdentity {
            uuid: login.player_uuid,
            username: login.username.clone(),
            properties: Vec::new(),
        }
    };

    let lower_name = identity.username.to_ascii_lowercase();
    let is_admin = cfg.admins.contains(&lower_name);

    if cfg.banned_players.contains(&lower_name) {
        let reason = TextComponent::text("You are banned from this proxy.").color("red");
        send_login_disconnect(&mut client, &reason).await;
        return;
    }

    if cfg.whitelist_enabled && !is_admin && !cfg.whitelist.contains(&lower_name) {
        let reason = TextComponent::text("You are not whitelisted on this server.").color("red");
        send_login_disconnect(&mut client, &reason).await;
        return;
    }

    if cfg.enforce_max_players
        && !is_admin
        && state.online_players.load(Ordering::Relaxed) >= cfg.max_players
    {
        let reason = TextComponent::text("The server is currently full.").color("red");
        send_login_disconnect(&mut client, &reason).await;
        return;
    }

    if state.find_player(&identity.username).is_some() {
        if cfg.kick_existing_players {
            state.kick_player(&identity.username, "You logged in from another location.");
        } else {
            let reason =
                TextComponent::text("You are already connected to this proxy.").color("red");
            send_login_disconnect(&mut client, &reason).await;
            return;
        }
    }

    let forced_host_match = resolve_forced_host(&handshake.server_address, cfg);
    if let Some((ref fh_name, _)) = forced_host_match
        && !can_access_server(&identity.username, fh_name, cfg)
    {
        let reason =
            TextComponent::text(format!("You do not have permission to join '{fh_name}'."))
                .color("red");
        send_login_disconnect(&mut client, &reason).await;
        return;
    }
    let Some((mut current_server, mut backend_addr)) = forced_host_match
        .map(|(name, addr)| (Some(name), addr))
        .or_else(|| resolve_initial_server(&identity.username, target_backend, cfg))
    else {
        info!(%peer_addr, username = %identity.username, uuid = %identity.uuid, "login attempt (no backend configured)");
        let reason = TextComponent::text("RadProxy is online! ")
            .color("aqua")
            .extra(TextComponent::text("No backend server configured yet.").color("gray"));
        send_login_disconnect(&mut client, &reason).await;
        return;
    };

    let (control_tx, mut control_rx) = mpsc::unbounded_channel();
    state.register_player(
        identity.username.clone(),
        identity.uuid,
        current_server.clone(),
        control_tx.clone(),
    );
    state.online_players.fetch_add(1, Ordering::Relaxed);
    if cfg.log_player_connections {
        info!(%peer_addr, username = %identity.username, uuid = %identity.uuid, "player connected");
    }

    let mut pending_chat: Option<String> = None;
    let mut client_state = WireState::Login;
    let mut client_compression: Option<CompressionConfig> = None;
    let mut last_pos = PlayerHoldPos::default();
    let mut player_lang = PlayerLocaleState::default();
    let mut preconnected_backend: Option<TcpStream> = None;

    loop {
        let Some((connected_name, connected_addr, mut backend)) =
            (if let Some(stream) = preconnected_backend.take() {
                Some((current_server.clone(), backend_addr, stream))
            } else {
                connect_with_fallback(
                    &identity.username,
                    current_server.as_deref(),
                    backend_addr,
                    cfg,
                )
                .await
            })
        else {
            warn!(%peer_addr, %backend_addr, "failed to connect to backend and all fallback servers");
            send_disconnect_for_state(
                &mut client,
                client_state,
                client_compression,
                "Backend server is currently unavailable.",
            )
            .await;
            break;
        };
        current_server = connected_name;
        backend_addr = connected_addr;
        state.update_player_server(&identity.username, current_server.clone());
        let _ = backend.set_nodelay(true);

        // Replay Handshake + LoginStart to the backend (with optional Legacy/BungeeGuard forwarding).
        let forwarded_handshake = format_forwarded_handshake(handshake, peer_addr, &identity, cfg);
        let mut initial_out = BytesMut::with_capacity(256);
        if encode_packet(&forwarded_handshake, &mut initial_out).is_err() {
            break;
        }
        let login_encoded = if legacy_protocol {
            encode_frame_with_compression(
                LoginStart::ID,
                &login_frame.payload,
                &mut initial_out,
                None,
            )
        } else {
            encode_packet(&login, &mut initial_out)
        };
        if login_encoded.is_err() || backend.write_all(&initial_out).await.is_err() {
            break;
        }

        let (backend_compression, initial_backend_buf, backend_state) = if client_state
            == WireState::Configuration
        {
            let Some((backend_comp, buf)) =
                complete_backend_login_for_switch(&mut backend, peer_addr, &identity, cfg).await
            else {
                break;
            };
            (backend_comp, buf, WireState::Configuration)
        } else {
            let Some((backend_comp, buf)) = negotiate_initial_backend_login(
                &mut client,
                &mut backend,
                peer_addr,
                &identity,
                cfg,
            )
            .await
            else {
                break;
            };
            client_compression = backend_comp;
            if legacy_protocol {
                client_state = WireState::Play;
                (backend_comp, buf, WireState::Play)
            } else {
                (backend_comp, buf, WireState::Login)
            }
        };

        if let Some(ref server_name) = current_server {
            state.increment_server_player(server_name);
        }

        info!(%peer_addr, username = %identity.username, %backend_addr, server = ?current_server, "proxying connection");

        let exit = proxy_with_server_switch(
            &mut client,
            &mut backend,
            &mut read_buf,
            initial_backend_buf,
            &mut client_state,
            backend_state,
            client_compression,
            backend_compression,
            pending_chat.take(),
            state,
            peer_addr,
            &identity.username,
            current_server.as_deref(),
            &mut control_rx,
            &mut last_pos,
            &mut player_lang,
            legacy_protocol,
            cfg,
        )
        .await;

        if let Some(ref server_name) = current_server {
            state.decrement_server_player(server_name);
        }

        match exit {
            ProxyExit::Switch { name, addr } => {
                state
                    .metrics
                    .server_switches
                    .fetch_add(1, Ordering::Relaxed);
                info!(%peer_addr, username = %identity.username, %addr, server = %name, "switching backend server");
                current_server = Some(name);
                backend_addr = addr;
                client_state = WireState::Configuration;
            }
            ProxyExit::BackendClosed { kick_reason } => {
                if client_state == WireState::Play
                    && cfg.seamless_restart
                    && is_restart_kick_reason(kick_reason.as_deref())
                {
                    info!(
                        %peer_addr,
                        username = %identity.username,
                        %backend_addr,
                        timeout_secs = cfg.seamless_restart_timeout_secs,
                        "backend closed during play; holding player for seamless restart"
                    );
                    match hold_player_during_restart(
                        &mut client,
                        &mut read_buf,
                        client_compression,
                        backend_addr,
                        last_pos,
                        cfg,
                    )
                    .await
                    {
                        HoldResult::Reconnected(stream) => {
                            preconnected_backend = Some(stream);
                            client_state = WireState::Configuration;
                            pending_chat = Some(rad_core::i18n::translate(
                                player_lang.effective(),
                                rad_core::i18n::ClientMsg::Reconnected,
                            ));
                            continue;
                        }
                        HoldResult::ClientClosed => break,
                        HoldResult::Timeout => {
                            warn!(%peer_addr, username = %identity.username, %backend_addr, "seamless restart timed out");
                        }
                    }
                }
                if let Some((fb_name, fb_addr)) = find_fallback_server(
                    &identity.username,
                    current_server.as_deref(),
                    backend_addr,
                    cfg,
                ) {
                    let can_switch = match client_state {
                        WireState::Play => {
                            if begin_server_switch(
                                &mut client,
                                &mut read_buf,
                                client_compression,
                                cfg,
                            )
                            .await
                            .is_ok()
                            {
                                client_state = WireState::Configuration;
                                true
                            } else {
                                false
                            }
                        }
                        WireState::Configuration | WireState::Login => true,
                    };
                    if can_switch {
                        state
                            .metrics
                            .server_switches
                            .fetch_add(1, Ordering::Relaxed);
                        warn!(%peer_addr, username = %identity.username, fallback = %fb_name, %fb_addr, ?kick_reason, "backend disconnected, moving player to fallback server");
                        if let Some(reason_text) = kick_reason {
                            pending_chat = Some(format!(
                                "Disconnected from previous server ({reason_text}); moved to {fb_name}."
                            ));
                        }
                        current_server = Some(fb_name);
                        backend_addr = fb_addr;
                        continue;
                    }
                }
                if let Some(reason_text) = kick_reason {
                    send_disconnect_for_state(
                        &mut client,
                        client_state,
                        client_compression,
                        &reason_text,
                    )
                    .await;
                }
                break;
            }
            ProxyExit::Closed => break,
        }
    }

    state.unregister_player(&identity.username, &control_tx);
    state.online_players.fetch_sub(1, Ordering::Relaxed);
    if cfg.log_player_connections {
        info!(%peer_addr, username = %identity.username, "player disconnected");
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PlayerLocaleState {
    pub detected: rad_core::i18n::ClientLang,
    pub override_lang: Option<rad_core::i18n::ClientLang>,
}

impl PlayerLocaleState {
    #[must_use]
    pub fn effective(&self) -> rad_core::i18n::ClientLang {
        self.override_lang.unwrap_or(self.detected)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct PlayerHoldPos {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub valid: bool,
}

enum HoldResult {
    Reconnected(TcpStream),
    Timeout,
    ClientClosed,
}

fn is_restart_kick_reason(reason: Option<&str>) -> bool {
    let Some(r) = reason else {
        return true;
    };
    let lower = r.to_ascii_lowercase();
    if lower.contains("ban") || lower.contains("whitelist") || lower.contains("full") {
        return false;
    }
    lower.contains("restart")
        || lower.contains("shutting down")
        || lower.contains("server closed")
        || lower.contains("closed")
        || lower.contains("updating")
        || lower.contains("maintenance")
}

async fn send_hold_chat(
    client: &mut EncryptedStream,
    compression: Option<CompressionConfig>,
    text: &str,
    overlay: bool,
) -> Result<()> {
    let pkt = SystemChatMessage {
        content: NbtTag::String(text.to_owned()),
        overlay,
    };
    let mut out = BytesMut::new();
    encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))?;
    client.write_all(&out).await?;
    Ok(())
}

async fn send_freeze_position(
    client: &mut EncryptedStream,
    compression: Option<CompressionConfig>,
    pos: PlayerHoldPos,
    teleport_id: i32,
) -> Result<()> {
    if !pos.valid {
        return Ok(());
    }
    let pkt = SynchronizePlayerPosition {
        teleport_id: VarInt(teleport_id),
        x: pos.x,
        y: pos.y,
        z: pos.z,
        vx: 0.0,
        vy: 0.0,
        vz: 0.0,
        yaw: pos.yaw,
        pitch: pos.pitch,
        flags: 0,
    };
    let mut out = BytesMut::new();
    encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))?;
    client.write_all(&out).await?;
    Ok(())
}

async fn hold_player_during_restart(
    client: &mut EncryptedStream,
    read_buf: &mut BytesMut,
    client_compression: Option<CompressionConfig>,
    backend_addr: SocketAddr,
    last_pos: PlayerHoldPos,
    cfg: &ProxyConfig,
) -> HoldResult {
    if send_hold_chat(
        client,
        client_compression,
        &cfg.seamless_restart_message,
        false,
    )
    .await
    .is_err()
    {
        return HoldResult::ClientClosed;
    }
    let mut tp_id = 900_000i32;
    if cfg.seamless_restart_freeze
        && send_freeze_position(client, client_compression, last_pos, tp_id)
            .await
            .is_err()
    {
        return HoldResult::ClientClosed;
    }

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(cfg.seamless_restart_timeout_secs);
    let mut poll_tick = tokio::time::interval(Duration::from_millis(cfg.seamless_restart_poll_ms));
    poll_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut keepalive_tick = tokio::time::interval(Duration::from_secs(2));
    keepalive_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut ka_id = 1i64;

    loop {
        if tokio::time::Instant::now() >= deadline {
            return HoldResult::Timeout;
        }
        tokio::select! {
            _ = poll_tick.tick() => {
                if let Ok(Ok(probe)) = tokio::time::timeout(
                    Duration::from_millis(400),
                    TcpStream::connect(backend_addr),
                ).await {
                    if begin_server_switch(client, read_buf, client_compression, cfg).await.is_ok() {
                        return HoldResult::Reconnected(probe);
                    }
                    return HoldResult::ClientClosed;
                }
            }
            _ = keepalive_tick.tick() => {
                ka_id = ka_id.wrapping_add(1);
                let mut out = BytesMut::new();
                if encode_packet_with_compression(
                    &KeepAliveClientbound { id: ka_id },
                    &mut out,
                    client_compression.map(|c| c.threshold),
                ).is_err() || client.write_all(&out).await.is_err() {
                    return HoldResult::ClientClosed;
                }
                let _ = send_hold_chat(client, client_compression, &cfg.seamless_restart_message, true).await;
            }
            res = client.read_buf(read_buf) => {
                match res {
                    Ok(0) | Err(_) => return HoldResult::ClientClosed,
                    Ok(_) => {
                        let mut consumed = 0usize;
                        while let FramePeek::Ready(rel_end) = peek_frame_status(&read_buf[consumed..], cfg.max_packet_size) {
                            let frame_end = consumed + rel_end;
                            if let Ok(pkt) = decode_raw_frame_slice(
                                &read_buf[consumed..frame_end],
                                cfg.max_packet_size,
                                client_compression,
                            ) && cfg.seamless_restart_freeze
                                && last_pos.valid
                                && (pkt.id == MovePlayerPos::ID || pkt.id == MovePlayerPosRot::ID)
                            {
                                tp_id = tp_id.wrapping_add(1);
                                if send_freeze_position(client, client_compression, last_pos, tp_id).await.is_err() {
                                    return HoldResult::ClientClosed;
                                }
                            }
                            consumed = frame_end;
                        }
                        read_buf.advance(consumed);
                    }
                }
            }
        }
    }
}

/// Current Minecraft protocol wire state for a connection half (`Login` -> `Configuration` -> `Play`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WireState {
    Login,
    Configuration,
    Play,
}

pub(super) enum ProxyExit {
    Closed,
    BackendClosed { kick_reason: Option<String> },
    Switch { name: String, addr: SocketAddr },
}

pub(super) enum ProxyCommand<'a> {
    ServerList,
    ServerSwitch(&'a str),
    Glist,
    Find(&'a str),
    Send { target: &'a str, server: &'a str },
    Alert(&'a str),
    Kick { target: &'a str, reason: &'a str },
    RadProxy(&'a str),
    Lang(&'a str),
}

/// Performs `EncryptionRequest` / `EncryptionResponse` + Mojang `hasJoined` verification
/// and activates `AES-128-CFB8` on `client`.
pub(super) async fn authenticate_online_client(
    client: &mut EncryptedStream,
    read_buf: &mut BytesMut,
    login: &LoginStart,
    peer_addr: SocketAddr,
    state: &ProxyState,
    cfg: &ProxyConfig,
) -> std::result::Result<PlayerIdentity, TextComponent> {
    let keypair = state
        .keypair
        .as_ref()
        .ok_or_else(|| TextComponent::text("Proxy RSA keypair not initialized.").color("red"))?;

    let mut verify_token = [0u8; 4];
    rand::thread_rng().fill_bytes(&mut verify_token);

    let enc_req = EncryptionRequest {
        server_id: String::new(),
        public_key: keypair.public_key_der.clone(),
        verify_token: verify_token.to_vec(),
        should_authenticate: true,
    };

    let mut out = BytesMut::new();
    if encode_packet(&enc_req, &mut out).is_err() || client.write_all(&out).await.is_err() {
        return Err(TextComponent::text("Failed to send encryption request.").color("red"));
    }

    let resp_frame = match timeout(
        cfg.handshake_timeout,
        read_next_frame(client, read_buf, cfg.max_packet_size),
    )
    .await
    {
        Ok(Ok(Some(f))) if f.id == EncryptionResponse::ID => f,
        _ => return Err(TextComponent::text("Encryption response timed out.").color("red")),
    };

    let enc_resp = resp_frame
        .decode_as::<EncryptionResponse>()
        .map_err(|_| TextComponent::text("Malformed encryption response.").color("red"))?;

    let decrypted_token = keypair
        .decrypt(&enc_resp.verify_token)
        .map_err(|_| TextComponent::text("Invalid verify token.").color("red"))?;
    if decrypted_token.as_slice() != verify_token {
        return Err(TextComponent::text("Verify token mismatch.").color("red"));
    }

    let decrypted_secret = keypair
        .decrypt(&enc_resp.shared_secret)
        .map_err(|_| TextComponent::text("Invalid shared secret.").color("red"))?;
    let secret: [u8; 16] = decrypted_secret
        .try_into()
        .map_err(|_| TextComponent::text("Shared secret must be 16 bytes.").color("red"))?;

    client.enable_encryption(&secret);

    let server_hash = mojang_server_hash("", &secret, &keypair.public_key_der);
    let client_ip = cfg.prevent_client_proxy_connections.then(|| peer_addr.ip());
    let profile = verify_mojang_session(&state.http, &login.username, &server_hash, client_ip)
        .await
        .map_err(|_| TextComponent::text("Failed to verify username with Mojang.").color("red"))?;

    Ok(PlayerIdentity {
        uuid: profile.uuid,
        username: profile.name,
        properties: profile.properties,
    })
}

/// Negotiates initial Login with the backend for all forwarding modes (`None`, `Legacy`,
/// `BungeeGuard`, and `Modern`), tracking `SetCompression` until `LoginSuccess` or `LoginDisconnect`.
///
/// Returns `Some((compression, leftover_backend_buf))` on `LoginSuccess`, or `None` if login failed.
pub(super) async fn negotiate_initial_backend_login(
    client: &mut EncryptedStream,
    backend: &mut TcpStream,
    peer_addr: SocketAddr,
    identity: &PlayerIdentity,
    config: &ProxyConfig,
) -> Option<(Option<CompressionConfig>, BytesMut)> {
    let mut backend_buf = BytesMut::with_capacity(1024);
    let mut compression: Option<CompressionConfig> = None;

    loop {
        let frame = match timeout(
            config.handshake_timeout,
            read_next_frame_with_compression(
                backend,
                &mut backend_buf,
                config.max_packet_size,
                compression,
            ),
        )
        .await
        {
            Ok(Ok(Some(frame))) => frame,
            _ => return None,
        };

        match frame.id {
            LoginPluginRequest::ID => {
                let Ok(req) = frame.decode_as::<LoginPluginRequest>() else {
                    return None;
                };
                if config.forwarding_mode == ForwardingMode::Modern
                    && req.channel == VELOCITY_FORWARDING_CHANNEL
                {
                    let data = ForwardingData {
                        address: peer_addr.ip().to_string(),
                        uuid: identity.uuid,
                        username: identity.username.clone(),
                        properties: identity.properties.clone(),
                    };
                    let Ok(signed) = encode_velocity_forwarding(&data, &config.forwarding_secret)
                    else {
                        return None;
                    };
                    let resp = LoginPluginResponse {
                        message_id: req.message_id,
                        successful: true,
                        data: signed,
                    };
                    let mut out = BytesMut::new();
                    if encode_packet_with_compression(
                        &resp,
                        &mut out,
                        compression.map(|c| c.threshold),
                    )
                    .is_err()
                        || backend.write_all(&out).await.is_err()
                    {
                        return None;
                    }
                    debug!(%peer_addr, username = %identity.username, "sent Velocity Modern Forwarding payload");
                } else {
                    let mut out = BytesMut::new();
                    if encode_frame_with_compression(
                        frame.id,
                        &frame.payload,
                        &mut out,
                        compression.map(|c| c.threshold),
                    )
                    .is_err()
                        || client.write_all(&out).await.is_err()
                    {
                        return None;
                    }
                }
            }
            SetCompression::ID => {
                let Ok(pkt) = frame.decode_as::<SetCompression>() else {
                    return None;
                };
                let mut out = BytesMut::new();
                if encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))
                    .is_err()
                    || client.write_all(&out).await.is_err()
                {
                    return None;
                }
                if pkt.threshold.0 >= 0 {
                    compression = Some(CompressionConfig::new(pkt.threshold.0 as usize));
                }
            }
            LoginSuccess::ID | LoginDisconnect::ID => {
                let is_disconnect = frame.id == LoginDisconnect::ID;
                let mut out = BytesMut::new();
                if encode_frame_with_compression(
                    frame.id,
                    &frame.payload,
                    &mut out,
                    compression.map(|c| c.threshold),
                )
                .is_err()
                    || client.write_all(&out).await.is_err()
                {
                    return None;
                }
                return (!is_disconnect).then_some((compression, backend_buf));
            }
            _ => {
                let mut out = BytesMut::new();
                if encode_frame_with_compression(
                    frame.id,
                    &frame.payload,
                    &mut out,
                    compression.map(|c| c.threshold),
                )
                .is_err()
                    || client.write_all(&out).await.is_err()
                {
                    return None;
                }
            }
        }
    }
}

/// Completes the Login phase with a new backend on behalf of a client that is
/// already in `Configuration` state (after [`begin_server_switch`]).
///
/// Consumes the backend's `LoginSuccess`, sends `LoginAcknowledged` to the backend,
/// and returns the backend's negotiated compression state plus any buffered Configuration bytes
/// without leaking `Login`-state packets to the `Configuration`-state client.
async fn complete_backend_login_for_switch(
    backend: &mut TcpStream,
    peer_addr: SocketAddr,
    identity: &PlayerIdentity,
    cfg: &ProxyConfig,
) -> Option<(Option<CompressionConfig>, BytesMut)> {
    let mut backend_buf = BytesMut::with_capacity(2048);
    let mut backend_compression: Option<CompressionConfig> = None;
    let deadline = tokio::time::Instant::now() + cfg.handshake_timeout;

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let frame = timeout(
            remaining,
            read_next_frame_with_compression(
                backend,
                &mut backend_buf,
                cfg.max_packet_size,
                backend_compression,
            ),
        )
        .await
        .ok()?
        .ok()??;

        match frame.id {
            LoginPluginRequest::ID => {
                let Ok(req) = frame.decode_as::<LoginPluginRequest>() else {
                    return None;
                };
                let (successful, data) = if cfg.forwarding_mode == ForwardingMode::Modern
                    && req.channel == VELOCITY_FORWARDING_CHANNEL
                {
                    let data = ForwardingData {
                        address: peer_addr.ip().to_string(),
                        uuid: identity.uuid,
                        username: identity.username.clone(),
                        properties: identity.properties.clone(),
                    };
                    let Ok(signed) = encode_velocity_forwarding(&data, &cfg.forwarding_secret)
                    else {
                        return None;
                    };
                    (true, signed)
                } else {
                    (false, Vec::new())
                };
                let resp = LoginPluginResponse {
                    message_id: req.message_id,
                    successful,
                    data,
                };
                let mut out = BytesMut::new();
                if encode_packet_with_compression(
                    &resp,
                    &mut out,
                    backend_compression.map(|c| c.threshold),
                )
                .is_err()
                    || backend.write_all(&out).await.is_err()
                {
                    return None;
                }
            }
            SetCompression::ID => {
                let Ok(pkt) = frame.decode_as::<SetCompression>() else {
                    return None;
                };
                if pkt.threshold.0 >= 0 {
                    backend_compression = Some(CompressionConfig::new(pkt.threshold.0 as usize));
                }
            }
            LoginSuccess::ID => {
                let mut out = BytesMut::new();
                if encode_packet_with_compression(
                    &LoginAcknowledged,
                    &mut out,
                    backend_compression.map(|c| c.threshold),
                )
                .is_err()
                    || backend.write_all(&out).await.is_err()
                {
                    return None;
                }
                return Some((backend_compression, backend_buf));
            }
            LoginDisconnect::ID => return None,
            _ => {}
        }
    }
}

/// Transitions an active `Play` client back to `Configuration` state using
/// [`StartConfiguration`] so `RadProxy` can connect them to a new backend server.
#[allow(dead_code)]
pub async fn begin_server_switch(
    client: &mut EncryptedStream,
    client_buf: &mut BytesMut,
    compression: Option<CompressionConfig>,
    config: &ProxyConfig,
) -> Result<()> {
    let mut out = BytesMut::new();
    encode_packet_with_compression(
        &StartConfiguration,
        &mut out,
        compression.map(|c| c.threshold),
    )?;
    client.write_all(&out).await?;

    // Wait for client's Play `ConfigurationAcknowledged` (`0x10` in 777), ignoring any
    // in-flight Play packets sent by the client before it processed `StartConfiguration`.
    let deadline = tokio::time::Instant::now() + config.handshake_timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(Error::InvalidString("server switch timed out".into()));
        }
        let frame = timeout(
            remaining,
            read_next_frame_with_compression(
                client,
                client_buf,
                config.max_packet_size,
                compression,
            ),
        )
        .await
        .map_err(|_| Error::InvalidString("server switch timed out".into()))??
        .ok_or(Error::UnexpectedEof { needed: 1 })?;

        if frame.id == ConfigurationAcknowledged::ID {
            return Ok(());
        }
    }
}

pub(super) async fn send_login_disconnect(client: &mut EncryptedStream, reason: &TextComponent) {
    if let Ok(pkt) = LoginDisconnect::from_text(reason) {
        let mut out = BytesMut::new();
        if encode_packet(&pkt, &mut out).is_ok() {
            let _ = client.write_all(&out).await;
        }
    }
}

pub(super) async fn send_disconnect_for_state(
    client: &mut EncryptedStream,
    state: WireState,
    compression: Option<CompressionConfig>,
    reason: &str,
) {
    let mut out = BytesMut::new();
    let encoded = match state {
        WireState::Login => LoginDisconnect::from_text(&TextComponent::text(reason).color("red"))
            .and_then(|pkt| {
                encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))
            }),
        WireState::Configuration => {
            let pkt = ConfigurationDisconnect {
                reason: NbtTag::String(reason.to_owned()),
            };
            encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))
        }
        WireState::Play => {
            let pkt = PlayDisconnect {
                reason: NbtTag::String(reason.to_owned()),
            };
            encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))
        }
    };
    if encoded.is_ok() {
        let _ = client.write_all(&out).await;
    }
}

pub(super) enum StackFrame {
    Ready {
        id: i32,
        payload_start: usize,
        frame_end: usize,
    },
    TooLarge,
    Eof,
}
