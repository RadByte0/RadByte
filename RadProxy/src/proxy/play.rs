//! Active Play-state forwarding, server switching, commands, and BungeeCord channel.

use super::*;

/// Proxies traffic with zero-decompress raw frame pass-through when compression matches
/// (and bounded transcoding when compression differs), while tracking protocol state transitions,
/// proxy commands, `DeclareCommands` merging, `minecraft:brand`, and BungeeCord messages.
#[allow(clippy::too_many_arguments)]
pub(super) async fn proxy_with_server_switch(
    client: &mut EncryptedStream,
    backend: &mut TcpStream,
    client_buf: &mut BytesMut,
    mut backend_buf: BytesMut,
    client_state: &mut WireState,
    mut backend_state: WireState,
    client_compression: Option<CompressionConfig>,
    backend_compression: Option<CompressionConfig>,
    mut pending_chat: Option<String>,
    state: &ProxyState,
    peer_addr: SocketAddr,
    username: &str,
    current_server: Option<&str>,
    control_rx: &mut mpsc::UnboundedReceiver<PlayerControl>,
    last_pos: &mut PlayerHoldPos,
    player_lang: &mut PlayerLocaleState,
    legacy_protocol: bool,
    cfg: &ProxyConfig,
) -> ProxyExit {
    let is_admin = cfg.admins.contains(&username.to_ascii_lowercase());
    let mut commands_sent = false;
    client_buf.reserve(64 * 1024);
    backend_buf.reserve(64 * 1024);
    let mut to_backend_out = BytesMut::with_capacity(64 * 1024);
    let mut to_client_out = BytesMut::with_capacity(64 * 1024);

    if !client_buf.is_empty()
        && let Err(exit) = process_client_buf(
            client,
            backend,
            client_buf,
            &mut to_backend_out,
            client_state,
            &mut backend_state,
            client_compression,
            backend_compression,
            &mut commands_sent,
            &mut pending_chat,
            is_admin,
            state,
            username,
            current_server,
            last_pos,
            player_lang,
            legacy_protocol,
            cfg,
        )
        .await
    {
        return exit;
    }

    if !backend_buf.is_empty()
        && let Err(exit) = process_backend_buf(
            client,
            backend,
            client_buf,
            &mut backend_buf,
            &mut to_client_out,
            &mut backend_state,
            client_compression,
            backend_compression,
            &mut commands_sent,
            is_admin,
            state,
            peer_addr,
            username,
            current_server,
            last_pos,
            legacy_protocol,
            cfg,
        )
        .await
    {
        return exit;
    }

    let mut last_activity = tokio::time::Instant::now();

    loop {
        let read_deadline = cfg.read_timeout.map(|d| last_activity + d);
        tokio::select! {
            ctrl = control_rx.recv() => {
                if let Some(msg) = ctrl {
                    match msg {
                        PlayerControl::Switch { name, addr } => {
                            if *client_state == WireState::Configuration {
                                return ProxyExit::Switch { name, addr };
                            }
                            if begin_server_switch(client, client_buf, client_compression, cfg).await.is_ok() {
                                *client_state = WireState::Configuration;
                                return ProxyExit::Switch { name, addr };
                            }
                            return ProxyExit::Closed;
                        }
                        PlayerControl::SystemChat(text) => {
                            if *client_state == WireState::Play {
                                if send_system_chat(client, client_compression, text).await.is_err() {
                                    return ProxyExit::Closed;
                                }
                            } else {
                                pending_chat = Some(text);
                            }
                        }
                        PlayerControl::Kick(reason) => {
                            send_disconnect_for_state(client, *client_state, client_compression, &reason).await;
                            return ProxyExit::Closed;
                        }
                    }
                }
            }
            _ = async {
                if let Some(deadline) = read_deadline {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                send_disconnect_for_state(client, *client_state, client_compression, "Timed out").await;
                return ProxyExit::Closed;
            }
            res = client.read_buf(client_buf) => {
                match res {
                    Ok(0) | Err(_) => return ProxyExit::Closed,
                    Ok(_) => last_activity = tokio::time::Instant::now(),
                }
                if let Err(exit) = process_client_buf(
                    client,
                    backend,
                    client_buf,
                    &mut to_backend_out,
                    client_state,
                    &mut backend_state,
                    client_compression,
                    backend_compression,
                    &mut commands_sent,
                    &mut pending_chat,
                    is_admin,
                    state,
                    username,
                    current_server,
                    last_pos,
                    player_lang,
                    legacy_protocol,
                    cfg,
                )
                .await
                {
                    return exit;
                }
            }
            res = backend.read_buf(&mut backend_buf) => {
                match res {
                    Ok(0) | Err(_) => return ProxyExit::BackendClosed { kick_reason: None },
                    Ok(_) => last_activity = tokio::time::Instant::now(),
                }
                if let Err(exit) = process_backend_buf(
                    client,
                    backend,
                    client_buf,
                    &mut backend_buf,
                    &mut to_client_out,
                    &mut backend_state,
                    client_compression,
                    backend_compression,
                    &mut commands_sent,
                    is_admin,
                    state,
                    peer_addr,
                    username,
                    current_server,
                    last_pos,
                    legacy_protocol,
                    cfg,
                )
                .await
                {
                    return exit;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn process_client_buf(
    client: &mut EncryptedStream,
    backend: &mut TcpStream,
    client_buf: &mut BytesMut,
    out: &mut BytesMut,
    client_state: &mut WireState,
    backend_state: &mut WireState,
    client_compression: Option<CompressionConfig>,
    backend_compression: Option<CompressionConfig>,
    commands_sent: &mut bool,
    pending_chat: &mut Option<String>,
    is_admin: bool,
    state: &ProxyState,
    username: &str,
    current_server: Option<&str>,
    last_pos: &mut PlayerHoldPos,
    player_lang: &mut PlayerLocaleState,
    legacy_protocol: bool,
    cfg: &ProxyConfig,
) -> std::result::Result<(), ProxyExit> {
    let mut consumed = 0usize;
    out.clear();
    let mut just_entered_play = false;
    let same_compression = client_compression == backend_compression;

    if legacy_protocol && same_compression {
        while let FramePeek::Ready(rel_end) =
            peek_frame_status(&client_buf[consumed..], cfg.max_packet_size)
        {
            consumed += rel_end;
        }
        if matches!(
            peek_frame_status(&client_buf[consumed..], cfg.max_packet_size),
            FramePeek::Invalid
        ) || client_buf.len().saturating_sub(consumed) > cfg.max_packet_size.saturating_add(5)
        {
            return Err(ProxyExit::Closed);
        }
        if consumed > 0 {
            let write_res = backend.write_all(&client_buf[..consumed]).await;
            client_buf.advance(consumed);
            if write_res.is_err() {
                return Err(ProxyExit::BackendClosed { kick_reason: None });
            }
        }
        return Ok(());
    }

    while let FramePeek::Ready(rel_end) =
        peek_frame_status(&client_buf[consumed..], cfg.max_packet_size)
    {
        let frame_end = consumed + rel_end;
        let frame = &client_buf[consumed..frame_end];
        let Ok(pkt) = decode_raw_frame_slice(frame, cfg.max_packet_size, client_compression) else {
            return Err(ProxyExit::Closed);
        };

        if !legacy_protocol {
            if ((*client_state == WireState::Configuration && pkt.id == 0x00)
                || (*client_state == WireState::Play && pkt.id == 0x0E))
                && let Some(locale) = rad_core::i18n::decode_client_locale(&pkt.payload)
            {
                player_lang.detected = rad_core::i18n::ClientLang::from_locale(&locale);
            }

            if *client_state == WireState::Play {
                if pkt.id == MovePlayerPos::ID
                    && let Ok(mv) = MovePlayerPos::decode_body(&mut &pkt.payload[..])
                {
                    last_pos.x = mv.x;
                    last_pos.y = mv.y;
                    last_pos.z = mv.z;
                    last_pos.valid = true;
                } else if pkt.id == MovePlayerPosRot::ID
                    && let Ok(mv) = MovePlayerPosRot::decode_body(&mut &pkt.payload[..])
                {
                    last_pos.x = mv.x;
                    last_pos.y = mv.y;
                    last_pos.z = mv.z;
                    last_pos.yaw = mv.yaw;
                    last_pos.pitch = mv.pitch;
                    last_pos.valid = true;
                }
            }

            if *client_state == WireState::Login && pkt.id == LoginAcknowledged::ID {
                *client_state = WireState::Configuration;
                *backend_state = WireState::Configuration;
            } else if *client_state == WireState::Configuration
                && pkt.id == AcknowledgeFinishConfiguration::ID
            {
                *client_state = WireState::Play;
                just_entered_play = true;
            } else if *client_state == WireState::Play
                && pkt.id == ChatCommand::ID
                && let Some(cmd) = parse_proxy_command_payload(&pkt.payload)
            {
                if cfg.log_command_executions {
                    debug!(%username, "executed proxy command");
                }
                if !out.is_empty() {
                    if backend.write_all(out).await.is_err() {
                        return Err(ProxyExit::BackendClosed { kick_reason: None });
                    }
                    out.clear();
                }

                if let Some(exit) = handle_proxy_command(
                    cmd,
                    client,
                    client_buf,
                    frame_end,
                    client_state,
                    client_compression,
                    is_admin,
                    state,
                    username,
                    current_server,
                    player_lang,
                    cfg,
                )
                .await
                {
                    return Err(exit);
                }
                consumed = frame_end;
                continue;
            }
        }

        if same_compression {
            out.extend_from_slice(frame);
        } else if encode_frame_with_compression(
            pkt.id,
            &pkt.payload,
            out,
            backend_compression.map(|c| c.threshold),
        )
        .is_err()
        {
            return Err(ProxyExit::Closed);
        }
        consumed = frame_end;
    }

    if matches!(
        peek_frame_status(&client_buf[consumed..], cfg.max_packet_size),
        FramePeek::Invalid
    ) || client_buf.len().saturating_sub(consumed) > cfg.max_packet_size.saturating_add(5)
    {
        return Err(ProxyExit::Closed);
    }

    client_buf.advance(consumed);

    if !out.is_empty() && backend.write_all(out).await.is_err() {
        return Err(ProxyExit::BackendClosed { kick_reason: None });
    }

    if just_entered_play {
        if !*commands_sent {
            *commands_sent = true;
            let cmds = build_proxy_declare_commands(is_admin);
            let mut cmd_out = BytesMut::new();
            if encode_packet_with_compression(
                &cmds,
                &mut cmd_out,
                client_compression.map(|c| c.threshold),
            )
            .is_ok()
                && client.write_all(&cmd_out).await.is_err()
            {
                return Err(ProxyExit::Closed);
            }
        }
        if let Some(msg) = pending_chat.take()
            && send_system_chat(client, client_compression, msg)
                .await
                .is_err()
        {
            return Err(ProxyExit::Closed);
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_proxy_command(
    cmd: ProxyCommand<'_>,
    client: &mut EncryptedStream,
    client_buf: &mut BytesMut,
    frame_end: usize,
    client_state: &mut WireState,
    client_compression: Option<CompressionConfig>,
    is_admin: bool,
    state: &ProxyState,
    username: &str,
    current_server: Option<&str>,
    player_lang: &mut PlayerLocaleState,
    cfg: &ProxyConfig,
) -> Option<ProxyExit> {
    use rad_core::i18n::{ClientLang, ClientMsg, translate};
    let lang = player_lang.effective();
    match cmd {
        ProxyCommand::ServerSwitch(target_name) => {
            let target_owned = target_name.to_owned();
            if let Some((resolved_name, next_addr)) =
                state.resolve_named_target_for_user(username, &target_owned, cfg)
            {
                client_buf.advance(frame_end);
                if begin_server_switch(client, client_buf, client_compression, cfg)
                    .await
                    .is_ok()
                {
                    *client_state = WireState::Configuration;
                    return Some(ProxyExit::Switch {
                        name: resolved_name,
                        addr: next_addr,
                    });
                }
                return Some(ProxyExit::Closed);
            }
            let msg = if state.resolve_named_target(&target_owned, cfg).is_some() {
                translate(lang, ClientMsg::NoServerPermission(&target_owned))
            } else {
                format_unknown_server_message(&target_owned, cfg)
            };
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::ServerList => {
            let msg = format_server_list_message(state, current_server, cfg);
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::Glist => {
            let msg = format_glist_message(state, cfg);
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::Find(target_player) => {
            let target_owned = target_player.to_owned();
            let msg = if !is_admin {
                translate(lang, ClientMsg::NoPermission("/find"))
            } else if let Some((name, srv)) = state.find_player(&target_owned) {
                translate(
                    lang,
                    ClientMsg::PlayerOnServer {
                        player: &name,
                        server: srv.as_deref().unwrap_or("unknown"),
                    },
                )
            } else {
                translate(lang, ClientMsg::PlayerNotOnline(&target_owned))
            };
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::Send { target, server } => {
            let target_owned = target.to_owned();
            let server_owned = server.to_owned();
            let msg = if !is_admin {
                translate(lang, ClientMsg::NoPermission("/send"))
            } else if let Some(&addr) = cfg.servers.get(&server_owned) {
                if target_owned.eq_ignore_ascii_case("all") {
                    let count = state.send_all(&server_owned, addr);
                    translate(
                        lang,
                        ClientMsg::SentMany {
                            count,
                            server: &server_owned,
                        },
                    )
                } else if state.send_player(&target_owned, &server_owned, addr) {
                    translate(
                        lang,
                        ClientMsg::SentOne {
                            player: &target_owned,
                            server: &server_owned,
                        },
                    )
                } else {
                    translate(lang, ClientMsg::PlayerNotOnline(&target_owned))
                }
            } else {
                format_unknown_server_message(&server_owned, cfg)
            };
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::Alert(text) => {
            if !is_admin {
                if send_system_chat(
                    client,
                    client_compression,
                    translate(lang, ClientMsg::NoPermission("/alert")),
                )
                .await
                .is_err()
                {
                    return Some(ProxyExit::Closed);
                }
            } else {
                state.alert_all(&format!("[Alert] {text}"));
            }
        }
        ProxyCommand::Kick { target, reason } => {
            let msg = if !is_admin {
                translate(lang, ClientMsg::NoPermission("/kick"))
            } else if state.kick_player(target, reason) {
                translate(
                    lang,
                    ClientMsg::KickedPlayer {
                        player: target,
                        reason,
                    },
                )
            } else {
                translate(lang, ClientMsg::PlayerNotOnline(target))
            };
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::RadProxy(subcmd) => {
            let msg = if !is_admin {
                translate(lang, ClientMsg::NoPermission("/radproxy"))
            } else if subcmd.eq_ignore_ascii_case("reload") {
                match state.reload_current() {
                    Ok(radons) => translate(lang, ClientMsg::Reloaded(radons)),
                    Err(err) => format!("Failed to reload proxy config: {err}"),
                }
            } else {
                let snap = state.metrics.snapshot();
                format!(
                    "RadProxy | Online: {} | Connections: {} | Switches: {}",
                    state.online_players.load(Ordering::Relaxed),
                    snap.total_connections,
                    snap.server_switches
                )
            };
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
        ProxyCommand::Lang(arg) => {
            let msg = if arg.is_empty() {
                translate(lang, ClientMsg::LangCurrent(lang))
            } else if arg.eq_ignore_ascii_case("auto") {
                player_lang.override_lang = None;
                let detected = player_lang.detected;
                translate(detected, ClientMsg::LangAuto(detected))
            } else if let Some(new_lang) = ClientLang::from_code(arg) {
                player_lang.override_lang = Some(new_lang);
                translate(new_lang, ClientMsg::LangSet(new_lang))
            } else {
                translate(lang, ClientMsg::LangUsage)
            };
            if send_system_chat(client, client_compression, msg)
                .await
                .is_err()
            {
                return Some(ProxyExit::Closed);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
async fn process_backend_buf(
    client: &mut EncryptedStream,
    backend: &mut TcpStream,
    client_buf: &mut BytesMut,
    backend_buf: &mut BytesMut,
    out: &mut BytesMut,
    backend_state: &mut WireState,
    client_compression: Option<CompressionConfig>,
    backend_compression: Option<CompressionConfig>,
    commands_sent: &mut bool,
    is_admin: bool,
    state: &ProxyState,
    peer_addr: SocketAddr,
    username: &str,
    current_server: Option<&str>,
    last_pos: &mut PlayerHoldPos,
    legacy_protocol: bool,
    cfg: &ProxyConfig,
) -> std::result::Result<(), ProxyExit> {
    let mut consumed = 0usize;
    out.clear();
    let same_compression = client_compression == backend_compression;

    if legacy_protocol && same_compression {
        while let FramePeek::Ready(rel_end) =
            peek_frame_status(&backend_buf[consumed..], cfg.max_packet_size)
        {
            consumed += rel_end;
        }
        if matches!(
            peek_frame_status(&backend_buf[consumed..], cfg.max_packet_size),
            FramePeek::Invalid
        ) || backend_buf.len().saturating_sub(consumed) > cfg.max_packet_size.saturating_add(5)
        {
            return Err(ProxyExit::BackendClosed { kick_reason: None });
        }
        if consumed > 0 {
            let write_res = client.write_all(&backend_buf[..consumed]).await;
            backend_buf.advance(consumed);
            if write_res.is_err() {
                return Err(ProxyExit::Closed);
            }
        }
        return Ok(());
    }

    while let FramePeek::Ready(rel_end) =
        peek_frame_status(&backend_buf[consumed..], cfg.max_packet_size)
    {
        let frame_end = consumed + rel_end;
        let frame = &backend_buf[consumed..frame_end];
        let Ok(pkt) = decode_raw_frame_slice(frame, cfg.max_packet_size, backend_compression)
        else {
            return Err(ProxyExit::BackendClosed { kick_reason: None });
        };

        if !legacy_protocol {
            if *backend_state == WireState::Configuration && pkt.id == FinishConfiguration::ID {
                *backend_state = WireState::Play;
            }

            if *backend_state == WireState::Configuration
                && pkt.id == ConfigClientboundPluginMessage::ID
                && let Some(ref brand) = cfg.proxy_brand
                && let Ok(plugin_pkt) =
                    ConfigClientboundPluginMessage::decode_body(&mut &pkt.payload[..])
                && plugin_pkt.channel == "minecraft:brand"
            {
                let replacement = ConfigClientboundPluginMessage {
                    channel: plugin_pkt.channel,
                    data: encode_brand_payload(brand),
                };
                if encode_packet_with_compression(
                    &replacement,
                    out,
                    client_compression.map(|c| c.threshold),
                )
                .is_ok()
                {
                    consumed = frame_end;
                    continue;
                }
            }

            if *backend_state == WireState::Play
                && pkt.id == SynchronizePlayerPosition::ID
                && let Ok(pos) = SynchronizePlayerPosition::decode_body(&mut &pkt.payload[..])
            {
                last_pos.x = pos.x;
                last_pos.y = pos.y;
                last_pos.z = pos.z;
                last_pos.yaw = pos.yaw;
                last_pos.pitch = pos.pitch;
                last_pos.valid = true;
            }

            if *backend_state == WireState::Play && pkt.id == DeclareCommands::ID {
                *commands_sent = true;
                if let Ok(backend_cmds) = DeclareCommands::decode_body(&mut &pkt.payload[..]) {
                    let merged = merge_declare_commands(backend_cmds, is_admin);
                    if encode_packet_with_compression(
                        &merged,
                        out,
                        client_compression.map(|c| c.threshold),
                    )
                    .is_ok()
                    {
                        consumed = frame_end;
                        continue;
                    }
                }
            }

            if *backend_state == WireState::Play
                && cfg.bungeecord_messaging
                && pkt.id == PlayClientboundPluginMessage::ID
                && let Ok(plugin_pkt) =
                    PlayClientboundPluginMessage::decode_body(&mut &pkt.payload[..])
                && (plugin_pkt.channel == "bungeecord:main" || plugin_pkt.channel == "BungeeCord")
                && let Some(req) = parse_bungee_request(&plugin_pkt.data)
            {
                if let Some(exit) = dispatch_bungee_request(
                    req,
                    &plugin_pkt.channel,
                    client,
                    backend,
                    client_buf,
                    out,
                    client_compression,
                    backend_compression,
                    state,
                    peer_addr,
                    username,
                    current_server,
                    cfg,
                )
                .await
                {
                    backend_buf.advance(frame_end);
                    return Err(exit);
                }
                consumed = frame_end;
                continue;
            }

            if !cfg.try_servers.is_empty() || cfg.seamless_restart {
                let kick_reason =
                    if *backend_state == WireState::Play && pkt.id == PlayDisconnect::ID {
                        PlayDisconnect::decode_body(&mut &pkt.payload[..])
                            .ok()
                            .map(|p| format_nbt_reason(p.reason))
                    } else if *backend_state == WireState::Configuration
                        && pkt.id == ConfigurationDisconnect::ID
                    {
                        ConfigurationDisconnect::decode_body(&mut &pkt.payload[..])
                            .ok()
                            .map(|p| format_nbt_reason(p.reason))
                    } else {
                        None
                    };
                if let Some(reason) = kick_reason {
                    if !out.is_empty() && client.write_all(out).await.is_err() {
                        return Err(ProxyExit::Closed);
                    }
                    backend_buf.advance(frame_end);
                    return Err(ProxyExit::BackendClosed {
                        kick_reason: Some(reason),
                    });
                }
            }
        }

        if same_compression {
            out.extend_from_slice(frame);
        } else if encode_frame_with_compression(
            pkt.id,
            &pkt.payload,
            out,
            client_compression.map(|c| c.threshold),
        )
        .is_err()
        {
            return Err(ProxyExit::Closed);
        }
        consumed = frame_end;
    }

    if matches!(
        peek_frame_status(&backend_buf[consumed..], cfg.max_packet_size),
        FramePeek::Invalid
    ) || backend_buf.len().saturating_sub(consumed) > cfg.max_packet_size.saturating_add(5)
    {
        return Err(ProxyExit::BackendClosed { kick_reason: None });
    }

    backend_buf.advance(consumed);

    if !out.is_empty() && client.write_all(out).await.is_err() {
        return Err(ProxyExit::Closed);
    }

    Ok(())
}

fn format_nbt_reason(reason: NbtTag) -> String {
    match reason {
        NbtTag::String(s) => s,
        other => format!("{other:?}"),
    }
}

#[allow(clippy::too_many_arguments)]
async fn dispatch_bungee_request(
    req: BungeeRequest,
    channel: &str,
    client: &mut EncryptedStream,
    backend: &mut TcpStream,
    client_buf: &mut BytesMut,
    out: &mut BytesMut,
    client_compression: Option<CompressionConfig>,
    backend_compression: Option<CompressionConfig>,
    state: &ProxyState,
    peer_addr: SocketAddr,
    username: &str,
    current_server: Option<&str>,
    cfg: &ProxyConfig,
) -> Option<ProxyExit> {
    match req {
        BungeeRequest::Connect { server } => {
            if let Some((name, addr)) = state.resolve_named_target_for_user(username, &server, cfg)
            {
                if !out.is_empty() && client.write_all(out).await.is_err() {
                    return Some(ProxyExit::Closed);
                }
                out.clear();
                if begin_server_switch(client, client_buf, client_compression, cfg)
                    .await
                    .is_ok()
                {
                    return Some(ProxyExit::Switch { name, addr });
                }
                return Some(ProxyExit::Closed);
            }
        }
        BungeeRequest::ConnectOther { player, server } => {
            if let Some((name, addr)) = state.resolve_named_target_for_user(&player, &server, cfg) {
                state.send_player(&player, &name, addr);
            }
        }
        BungeeRequest::Ip => {
            let mut resp = BytesMut::new();
            write_bungee_utf(&mut resp, "IP");
            write_bungee_utf(&mut resp, &peer_addr.ip().to_string());
            resp.extend_from_slice(&peer_addr.port().to_be_bytes());
            let _ = send_bungee_reply(backend, channel, resp.to_vec(), backend_compression).await;
        }
        BungeeRequest::PlayerCount { server } => {
            let count = if server.eq_ignore_ascii_case("ALL") {
                state.online_players.load(Ordering::Relaxed)
            } else {
                state
                    .server_player_counts()
                    .get(&server)
                    .copied()
                    .unwrap_or(0) as i32
            };
            let mut resp = BytesMut::new();
            write_bungee_utf(&mut resp, "PlayerCount");
            write_bungee_utf(&mut resp, &server);
            resp.extend_from_slice(&count.to_be_bytes());
            let _ = send_bungee_reply(backend, channel, resp.to_vec(), backend_compression).await;
        }
        BungeeRequest::PlayerList { server } => {
            let names = if server.eq_ignore_ascii_case("ALL") {
                state.online_player_names(None)
            } else {
                state.online_player_names(Some(&server))
            };
            let mut resp = BytesMut::new();
            write_bungee_utf(&mut resp, "PlayerList");
            write_bungee_utf(&mut resp, &server);
            write_bungee_utf(&mut resp, &names.join(", "));
            let _ = send_bungee_reply(backend, channel, resp.to_vec(), backend_compression).await;
        }
        BungeeRequest::GetServers => {
            let mut names: Vec<&str> = cfg.servers.keys().map(String::as_str).collect();
            names.sort_unstable();
            let mut resp = BytesMut::new();
            write_bungee_utf(&mut resp, "GetServers");
            write_bungee_utf(&mut resp, &names.join(", "));
            let _ = send_bungee_reply(backend, channel, resp.to_vec(), backend_compression).await;
        }
        BungeeRequest::Message { player, message } => {
            if player.eq_ignore_ascii_case("ALL") {
                state.alert_all(&message);
            } else if let Some(p) = state
                .active_players
                .lock()
                .expect("active_players poisoned")
                .get(&player.to_ascii_lowercase())
            {
                let _ = p.tx.send(PlayerControl::SystemChat(message));
            }
        }
        BungeeRequest::KickPlayer { player, reason } => {
            state.kick_player(&player, &reason);
        }
    }
    let _ = current_server;
    None
}

#[allow(dead_code)]
pub(super) fn inspect_play_disconnect(
    raw_frame: &[u8],
    compression: Option<CompressionConfig>,
) -> Option<String> {
    let pkt = decode_raw_frame_slice(raw_frame, DEFAULT_MAX_PACKET_SIZE, compression).ok()?;
    if pkt.id != PlayDisconnect::ID {
        return None;
    }
    let body = PlayDisconnect::decode_body(&mut &pkt.payload[..]).ok()?;
    Some(format_nbt_reason(body.reason))
}

pub(super) async fn send_bungee_reply(
    backend: &mut TcpStream,
    channel: &str,
    data: Vec<u8>,
    compression: Option<CompressionConfig>,
) -> Result<()> {
    let reply = PlayServerboundPluginMessage {
        channel: channel.to_owned(),
        data,
    };
    let mut out = BytesMut::new();
    encode_packet_with_compression(&reply, &mut out, compression.map(|c| c.threshold))?;
    backend.write_all(&out).await?;
    Ok(())
}

pub(super) async fn send_system_chat(
    client: &mut EncryptedStream,
    compression: Option<CompressionConfig>,
    message: String,
) -> Result<()> {
    let pkt = SystemChatMessage {
        content: NbtTag::String(message),
        overlay: false,
    };
    let mut out = BytesMut::new();
    encode_packet_with_compression(&pkt, &mut out, compression.map(|c| c.threshold))?;
    client.write_all(&out).await?;
    Ok(())
}

pub(super) fn format_unknown_server_message(target: &str, cfg: &ProxyConfig) -> String {
    let mut names: Vec<&str> = cfg.servers.keys().map(String::as_str).collect();
    names.sort_unstable();
    format!("Unknown server '{target}'. Available: {}", names.join(", "))
}

pub(super) fn format_server_list_message(
    state: &ProxyState,
    current_server: Option<&str>,
    cfg: &ProxyConfig,
) -> String {
    let counts = state.server_player_counts();
    let mut names: Vec<&str> = cfg.servers.keys().map(String::as_str).collect();
    names.sort_unstable();
    let entries: Vec<String> = names
        .into_iter()
        .map(|name| {
            let count = counts.get(name).copied().unwrap_or(0);
            if current_server == Some(name) {
                format!("{name}* ({count})")
            } else {
                format!("{name} ({count})")
            }
        })
        .collect();
    format!("Servers: {}", entries.join(", "))
}

pub(super) fn format_glist_message(state: &ProxyState, cfg: &ProxyConfig) -> String {
    let total = state.online_players.load(Ordering::Relaxed);
    let counts = state.server_player_counts();
    let mut names: Vec<&str> = cfg.servers.keys().map(String::as_str).collect();
    names.sort_unstable();
    let per_server: Vec<String> = names
        .into_iter()
        .map(|name| format!("{name}: {}", counts.get(name).copied().unwrap_or(0)))
        .collect();
    if per_server.is_empty() {
        format!("Total players online: {total}")
    } else {
        format!("Total players online: {total} ({})", per_server.join(", "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum FramePeek {
    Incomplete,
    Ready(usize),
    Invalid,
}

pub(super) fn peek_frame_status(buf: &[u8], max_packet_size: usize) -> FramePeek {
    if buf.is_empty() {
        return FramePeek::Incomplete;
    }
    let mut peek = buf;
    let frame_len = match VarInt::decode(&mut peek) {
        Ok(v) => v.0,
        Err(_) => {
            // A Minecraft VarInt is at most 5 bytes; if we already have 5+ bytes or a
            // byte without continuation bit failed, the length prefix is malformed.
            let incomplete_varint = buf.len() < 5 && buf.iter().all(|b| (b & 0x80) != 0);
            return if incomplete_varint {
                FramePeek::Incomplete
            } else {
                FramePeek::Invalid
            };
        }
    };
    if frame_len <= 0 || (frame_len as usize) > max_packet_size {
        return FramePeek::Invalid;
    }
    let header_len = buf.len() - peek.len();
    let frame_end = header_len + frame_len as usize;
    if buf.len() >= frame_end {
        FramePeek::Ready(frame_end)
    } else {
        FramePeek::Incomplete
    }
}

#[allow(dead_code)]
pub(super) fn peek_raw_frame_end(buf: &[u8], max_packet_size: usize) -> Option<usize> {
    match peek_frame_status(buf, max_packet_size) {
        FramePeek::Ready(end) => Some(end),
        FramePeek::Incomplete | FramePeek::Invalid => None,
    }
}

#[allow(dead_code)]
pub(super) fn inspect_proxy_command(
    raw_frame: &[u8],
    compression: Option<CompressionConfig>,
) -> Option<ProxyCommand<'_>> {
    let mut slice = raw_frame;
    let _frame_len = VarInt::decode(&mut slice).ok()?;
    if compression.is_some() {
        let data_len = VarInt::decode(&mut slice).ok()?.0;
        if data_len != 0 {
            return None;
        }
    }
    let packet_id = VarInt::decode(&mut slice).ok()?.0;
    if packet_id != ChatCommand::ID {
        return None;
    }
    parse_proxy_command_payload(slice)
}

pub(super) fn parse_proxy_command_payload(mut slice: &[u8]) -> Option<ProxyCommand<'_>> {
    // ChatCommand wire format: VarInt string length + UTF-8 command bytes.
    let cmd_len = VarInt::decode(&mut slice).ok()?.0;
    if cmd_len <= 0 || (cmd_len as usize) > slice.len() {
        return None;
    }
    let cmd = core::str::from_utf8(&slice[..cmd_len as usize])
        .ok()?
        .trim();
    if cmd == "server" {
        return Some(ProxyCommand::ServerList);
    }
    if let Some(target) = cmd.strip_prefix("server ") {
        let target = target.trim();
        return if target.is_empty() {
            Some(ProxyCommand::ServerList)
        } else {
            Some(ProxyCommand::ServerSwitch(target))
        };
    }
    if cmd == "glist" {
        return Some(ProxyCommand::Glist);
    }
    if let Some(target) = cmd.strip_prefix("find ") {
        let target = target.trim();
        if !target.is_empty() {
            return Some(ProxyCommand::Find(target));
        }
    }
    if let Some(rest) = cmd.strip_prefix("send ") {
        let mut parts = rest.split_whitespace();
        if let (Some(target), Some(server)) = (parts.next(), parts.next()) {
            return Some(ProxyCommand::Send { target, server });
        }
    }
    if let Some(msg) = cmd.strip_prefix("alert ") {
        let msg = msg.trim();
        if !msg.is_empty() {
            return Some(ProxyCommand::Alert(msg));
        }
    }
    if let Some(rest) = cmd.strip_prefix("kick ") {
        let rest = rest.trim();
        if let Some((target, reason)) = rest.split_once(char::is_whitespace) {
            let reason = reason.trim();
            return Some(ProxyCommand::Kick {
                target,
                reason: if reason.is_empty() {
                    "Kicked by an administrator."
                } else {
                    reason
                },
            });
        } else if !rest.is_empty() {
            return Some(ProxyCommand::Kick {
                target: rest,
                reason: "Kicked by an administrator.",
            });
        }
    }
    if cmd == "lang" {
        return Some(ProxyCommand::Lang(""));
    }
    if let Some(arg) = cmd.strip_prefix("lang ") {
        return Some(ProxyCommand::Lang(arg.trim()));
    }
    if cmd == "radproxy" {
        return Some(ProxyCommand::RadProxy("info"));
    }
    if let Some(sub) = cmd.strip_prefix("radproxy ") {
        let sub = sub.trim();
        return Some(ProxyCommand::RadProxy(if sub.is_empty() {
            "info"
        } else {
            sub
        }));
    }
    None
}
