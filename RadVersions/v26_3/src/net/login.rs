//! Handshake, forwarding verification, and Configuration phase for `RadServer`.

use super::*;

pub(super) async fn handle_login_and_play(
    mut stream: TcpStream,
    mut read_buf: BytesMut,
    peer_addr: SocketAddr,
    hs: &Handshake,
    state: Arc<ServerState>,
) {
    let cfg = &state.config;
    let login_start = match timeout(
        Duration::from_secs(cfg.login_timeout_secs),
        read_typed_packet::<LoginStart>(&mut stream, &mut read_buf, cfg.max_packet_size),
    )
    .await
    {
        Ok(Some(pkt)) => pkt,
        _ => return,
    };

    let expected_proto = state.version_package.protocol_version();
    if hs.protocol_version.0 != expected_proto {
        let reason = format!(
            "Unsupported Minecraft protocol {} (server requires {} / protocol {})",
            hs.protocol_version.0,
            state.version_package.game_version(),
            expected_proto
        );
        warn!(%peer_addr, client_protocol = hs.protocol_version.0, expected_proto, "rejected unsupported protocol");
        if let Ok(dc) = LoginDisconnect::from_text(&TextComponent::text(reason)) {
            let _ = send_packet(&mut stream, &dc).await;
        }
        return;
    }

    let (player_uuid, username, properties) =
        match verify_forwarding(&mut stream, &mut read_buf, hs, &login_start, cfg).await {
            Ok(identity) => identity,
            Err(reason) => {
                warn!(%peer_addr, player = %login_start.username, %reason, "rejected server login");
                let json_reason = TextComponent::text(reason)
                    .to_json()
                    .unwrap_or_else(|_| format!(r#"{{"text":"{reason}"}}"#));
                let _ = send_packet(
                    &mut stream,
                    &LoginDisconnect {
                        reason_json: json_reason,
                    },
                )
                .await;
                return;
            }
        };

    {
        let trusted = state.trusts_forwarded_identity();
        let online_count = state.players.read().await.len();
        let max_players = usize::try_from(cfg.max_players).unwrap_or(usize::MAX);
        let mut access = state.access.write().await;
        let key = username.to_ascii_lowercase();
        let perm_level = if state.config.admins.contains(&key) {
            4
        } else {
            access.op_level_for_identity(&username, Some(player_uuid), trusted)
        };
        let is_op = perm_level > 0;
        let deny_reason = if access.is_banned(&username) {
            Some("You are banned from this server.")
        } else if !access.is_allowed_by_whitelist_identity(
            &username,
            Some(player_uuid),
            is_op,
            trusted,
        ) {
            Some("You are not white-listed on this server!")
        } else if online_count >= max_players && !is_op {
            Some("The server is full!")
        } else {
            if trusted {
                access.record_verified_identity(player_uuid, &username);
            }
            None
        };
        if let Some(reason) = deny_reason {
            warn!(%peer_addr, player = %username, %reason, "rejected server login by access control");
            let json_reason = TextComponent::text(reason)
                .to_json()
                .unwrap_or_else(|_| format!(r#"{{"text":"{reason}"}}"#));
            let _ = send_packet(
                &mut stream,
                &LoginDisconnect {
                    reason_json: json_reason,
                },
            )
            .await;
            return;
        }
    }

    if send_packet(
        &mut stream,
        &LoginSuccess {
            uuid: player_uuid,
            username: username.clone(),
            properties: properties.clone(),
            session_id: state.server_session_id,
        },
    )
    .await
    .is_err()
    {
        return;
    }

    if timeout(
        Duration::from_secs(cfg.login_timeout_secs),
        read_typed_packet::<LoginAcknowledged>(&mut stream, &mut read_buf, cfg.max_packet_size),
    )
    .await
    .ok()
    .flatten()
    .is_none()
    {
        return;
    }

    let Some(client_view_distance) = run_configuration(
        &mut stream,
        &mut read_buf,
        cfg,
        state.registries.provider().as_ref(),
    )
    .await
    else {
        return;
    };

    run_play(
        stream,
        read_buf,
        peer_addr,
        player_uuid,
        username,
        properties,
        client_view_distance,
        state,
    )
    .await;
}

pub(super) async fn verify_forwarding(
    stream: &mut TcpStream,
    read_buf: &mut BytesMut,
    hs: &Handshake,
    login_start: &LoginStart,
    cfg: &ServerConfig,
) -> Result<(Uuid, String, Vec<GameProfileProperty>), &'static str> {
    validate_player_name(&login_start.username)?;
    match cfg.forwarding_mode {
        ServerForwardingMode::None => Ok((
            login_start.player_uuid,
            login_start.username.clone(),
            Vec::new(),
        )),
        ServerForwardingMode::Legacy => parse_legacy_handshake(&hs.server_address, None)
            .map(|uuid| (uuid, login_start.username.clone(), Vec::new())),
        ServerForwardingMode::BungeeGuard => {
            let token = std::str::from_utf8(&cfg.forwarding_secret).unwrap_or("");
            parse_legacy_handshake(&hs.server_address, Some(token))
                .map(|uuid| (uuid, login_start.username.clone(), Vec::new()))
        }
        ServerForwardingMode::Modern => {
            let req = LoginPluginRequest {
                message_id: VarInt(1),
                channel: VELOCITY_FORWARDING_CHANNEL.into(),
                data: Vec::new(),
            };
            if send_packet(stream, &req).await.is_err() {
                return Err("Failed to send Velocity forwarding request");
            }
            let resp = timeout(
                Duration::from_secs(cfg.forwarding_timeout_secs),
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
            validate_forwarded_name(&login_start.username, &fwd.username)?;
            Ok((fwd.uuid, fwd.username, fwd.properties))
        }
    }
}

fn validate_player_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.encode_utf16().count() > 16 || name.chars().any(char::is_control) {
        return Err("Invalid player name");
    }
    Ok(())
}

fn validate_forwarded_name(requested: &str, forwarded: &str) -> Result<(), &'static str> {
    validate_player_name(forwarded)?;
    if !requested.eq_ignore_ascii_case(forwarded) {
        return Err("Forwarded player name does not match login");
    }
    Ok(())
}

pub(super) fn parse_legacy_handshake(
    server_address: &str,
    required_bungeeguard_token: Option<&str>,
) -> Result<Uuid, &'static str> {
    let parts: Vec<&str> = server_address.split('\0').collect();
    if !(3..=4).contains(&parts.len()) {
        return Err("Missing legacy forwarding data in Handshake");
    }
    parts[1]
        .parse::<std::net::IpAddr>()
        .map_err(|_| "Invalid forwarded IP address")?;
    let uuid = Uuid::parse_str(parts[2]).map_err(|_| "Invalid forwarded UUID")?;
    if let Some(expected_token) = required_bungeeguard_token {
        if expected_token.is_empty() {
            return Err("BungeeGuard token must not be empty");
        }
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

pub(super) async fn run_configuration(
    stream: &mut TcpStream,
    read_buf: &mut BytesMut,
    cfg: &ServerConfig,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
) -> Option<Option<i32>> {
    let mut client_view_distance: Option<i32> = None;
    let mut batch = BytesMut::with_capacity(1024);

    let mut brand_payload = BytesMut::new();
    if write_string(&mut brand_payload, &cfg.brand, 32_767).is_err() {
        return None;
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
        return None;
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
        return None;
    }

    if stream.write_all(&batch).await.is_err() {
        return None;
    }
    batch.clear();

    let deadline =
        tokio::time::Instant::now() + Duration::from_secs(cfg.configuration_timeout_secs);
    let client_has_core = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let frame = match timeout(
            remaining,
            read_raw_frame(stream, read_buf, cfg.max_packet_size),
        )
        .await
        {
            Ok(Some(frame)) => frame,
            _ => return None,
        };
        if frame.id == 0x00
            && let Some(vd) = decode_client_view_distance(&frame.payload)
        {
            client_view_distance = Some(vd);
        } else if frame.id == rad_core::packet::configuration::ServerboundKnownPacks::ID {
            let Ok(known) =
                frame.decode_as::<rad_core::packet::configuration::ServerboundKnownPacks>()
            else {
                return None;
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
            return None;
        }
    }

    let tags_pkt =
        match rad_core::packet::configuration::ConfigUpdateTags::for_registries_with_provider(
            &registries,
            provider,
        ) {
            Ok(tags) => tags,
            Err(rad_core::registry_provider::RegistryProviderError::MissingData(reason)) => {
                tracing::warn!(
                    reason = %reason,
                    "configuration tags not packaged in version data; sending minimal fallback update_tags"
                );
                super::fallback_tags::minimal_fallback_update_tags()
            }
            Err(err) => {
                tracing::error!(error = %err, "configuration tags invalid; refusing configuration");
                return None;
            }
        };
    if encode_packet(&tags_pkt, &mut batch).is_err() {
        return None;
    }

    if encode_packet(&FinishConfiguration, &mut batch).is_err() {
        return None;
    }

    if stream.write_all(&batch).await.is_err() {
        return None;
    }

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let frame = match timeout(
            remaining,
            read_raw_frame(stream, read_buf, cfg.max_packet_size),
        )
        .await
        {
            Ok(Some(frame)) => frame,
            _ => return None,
        };
        if frame.id == 0x00
            && let Some(vd) = decode_client_view_distance(&frame.payload)
        {
            client_view_distance = Some(vd);
        } else if frame.id == AcknowledgeFinishConfiguration::ID {
            return Some(client_view_distance);
        }
    }
}

#[cfg(test)]
mod identity_validation_tests {
    use super::*;

    #[test]
    fn rejects_empty_control_and_oversized_player_names() {
        for name in ["", "User\nAdmin", "User\0Admin", "abcdefghijklmnopq"] {
            assert!(validate_player_name(name).is_err());
        }
        assert!(validate_player_name("Player_123").is_ok());
    }

    #[test]
    fn forwarded_name_must_match_login_but_may_change_case() {
        assert!(validate_forwarded_name("player", "Player").is_ok());
        assert!(validate_forwarded_name("Player", "OtherPlayer").is_err());
        assert!(validate_forwarded_name("Player", "Player\n").is_err());
    }

    #[test]
    fn legacy_forwarding_requires_valid_ip_uuid_and_nonempty_guard_token() {
        let id = Uuid::from_u128(42);
        let valid = format!("localhost\0{}\0{id}", "127.0.0.1");
        assert_eq!(parse_legacy_handshake(&valid, None).unwrap(), id);
        assert!(parse_legacy_handshake(&format!("localhost\0invalid\0{id}"), None).is_err());
        assert!(parse_legacy_handshake(&format!("{valid}\0[]\0extra"), None).is_err());
        assert!(parse_legacy_handshake(&format!("{valid}\0[]"), Some("")).is_err());
        let properties = serde_json::json!([{"name":"bungeeguard-token","value":"test-key"}]);
        let guarded = format!("{valid}\0{properties}");
        assert!(parse_legacy_handshake(&guarded, Some("test-key")).is_ok());
        assert!(parse_legacy_handshake(&guarded, Some("wrong-key")).is_err());
    }
}
