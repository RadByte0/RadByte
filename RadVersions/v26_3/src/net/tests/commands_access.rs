use super::*;

#[tokio::test]
async fn admin_commands_enforce_permissions_and_max_mobs_cap() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.admins.insert("adminuser".into());
    cfg.max_mobs = 1;
    let state = ServerState::new(cfg);
    let (tx, mut rx, _cancel) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);

    // Non-admin cannot summon, heal, save, gamemode, tp, or give
    commands::handle_server_command(&state, &tx, "RegularPlayer", "summon zombie", None).await;
    commands::handle_server_command(&state, &tx, "RegularPlayer", "heal", None).await;
    commands::handle_server_command(&state, &tx, "RegularPlayer", "save", None).await;
    commands::handle_server_command(&state, &tx, "RegularPlayer", "gamemode creative", None).await;
    commands::handle_server_command(&state, &tx, "RegularPlayer", "tp 10 70 10", None).await;
    commands::handle_server_command(&state, &tx, "RegularPlayer", "give chest 2", None).await;
    assert_eq!(state.entities.read().await.all_mobs().count(), 0);
    for _ in 0..6 {
        assert!(rx.recv().await.is_some());
    }

    // Admin can summon up to max_mobs
    commands::handle_server_command(&state, &tx, "AdminUser", "summon zombie", None).await;
    assert_eq!(state.entities.read().await.all_mobs().count(), 1);
    commands::handle_server_command(&state, &tx, "AdminUser", "summon pig", None).await;
    assert_eq!(state.entities.read().await.all_mobs().count(), 1);
    while rx.rx.try_recv().is_ok() {}

    // Admin /gamemode, /tp, /give + DeclareCommands
    let inv = PlayerInventory::default();
    let uuid = Uuid::from_u128(0x9999);
    state.players.write().await.insert(
        42,
        PlayerSession {
            entity_id: 42,
            uuid,
            username: "AdminUser".into(),
            properties: Vec::new(),
            x: 0.5,
            y: 64.0,
            z: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            hp: 20.0,
            food: 20,
            saturation: 5.0,
            on_ground: true,
            game_mode: 0,
            dimension: crate::world::worldgen::DimensionId::Overworld,
            dead: false,
            fall_start_y: None,
            pending_teleport_id: None,
            inventory: Vec::new(),
            selected_hotbar: 0,
            open_chest: None,
            chest_sync_tx: None,
            pending_keep_alive: None,
            last_activity: Instant::now(),
            tx: tx.clone(),
        },
    );

    let inv = std::sync::Mutex::new(inv);
    commands::handle_server_command(&state, &tx, "AdminUser", "gamemode creative", Some(&inv))
        .await;
    assert_eq!(state.players.read().await.get(&42).unwrap().game_mode, 1);

    commands::handle_server_command(&state, &tx, "AdminUser", "tp 32.5 80.0 -16.5", Some(&inv))
        .await;
    {
        let p = state.players.read().await;
        let s = p.get(&42).unwrap();
        assert_eq!((s.x, s.y, s.z), (32.5, 80.0, -16.5));
        assert!(s.pending_teleport_id.is_some());
    }

    commands::handle_server_command(&state, &tx, "AdminUser", "give chest 4", Some(&inv)).await;
    {
        let inv_guard = inv.lock().unwrap();
        assert_eq!(
            inv_guard.slots[41].item_id,
            crate::gameplay::inventory::CHEST_ITEM_ID
        );
        assert_eq!(inv_guard.slots[41].count, 4);
    }

    let dc = commands::build_declare_commands();
    assert!(dc.nodes.len() > 8);
}

#[tokio::test]
async fn access_control_ban_whitelist_kick_and_persistence() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.admins.insert("adminuser".into());

    let dir = std::env::temp_dir().join(format!(
        "radbyte_access_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let state = ServerState::new_with_world_dir(cfg.clone(), Some(dir.clone()));
    let (admin_tx, mut admin_rx, _admin_cancel) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    let (target_tx, mut target_rx, target_cancel) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);

    state.players.write().await.insert(
        77,
        PlayerSession {
            entity_id: 77,
            uuid: Uuid::from_u128(777),
            username: "TargetUser".into(),
            properties: Vec::new(),
            x: 0.5,
            y: 64.0,
            z: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            hp: 20.0,
            food: 20,
            saturation: 5.0,
            on_ground: true,
            fall_start_y: None,
            dead: false,
            game_mode: 0,
            dimension: crate::world::worldgen::DimensionId::Overworld,
            pending_keep_alive: None,
            last_activity: Instant::now(),
            inventory: Vec::new(),
            selected_hotbar: 0,
            open_chest: None,
            chest_sync_tx: None,
            pending_teleport_id: None,
            tx: target_tx,
        },
    );

    // Non-admin cannot invoke moderation commands
    for cmd in [
        "ban TargetUser",
        "pardon TargetUser",
        "whitelist on",
        "kick TargetUser",
    ] {
        commands::handle_server_command(&state, &admin_tx, "RegularPlayer", cmd, None).await;
    }
    assert!(!*target_cancel.borrow());
    assert!(!state.access.read().await.is_banned("TargetUser"));
    assert!(!state.access.read().await.whitelist_enabled);
    while admin_rx.rx.try_recv().is_ok() {}

    // Admin kick disconnects online session with PlayDisconnect
    commands::handle_server_command(&state, &admin_tx, "AdminUser", "kick TargetUser bye", None)
        .await;
    assert!(*target_cancel.borrow());
    assert!(target_rx.recv().await.is_some());

    // Admin ban, whitelist on & add
    commands::handle_server_command(&state, &admin_tx, "AdminUser", "ban Griefer", None).await;
    commands::handle_server_command(&state, &admin_tx, "AdminUser", "whitelist on", None).await;
    commands::handle_server_command(&state, &admin_tx, "AdminUser", "whitelist add Friend", None)
        .await;

    {
        let access = state.access.read().await;
        assert!(access.is_banned("griefer"));
        assert!(access.whitelist_enabled);
        assert!(access.is_allowed_by_whitelist("Friend", false));
        assert!(!access.is_allowed_by_whitelist("Stranger", false));
        assert!(access.is_allowed_by_whitelist("AdminUser", true));
    }

    // Persist and reload into fresh ServerState
    state.save_all().await.unwrap();
    assert!(dir.join("ops.json").exists());
    assert!(dir.join("whitelist.json").exists());
    assert!(dir.join("banned-players.json").exists());
    assert!(!dir.join("access_control.json").exists());

    let mut reload_cfg = cfg;
    reload_cfg.white_list = Some(true);
    let reloaded = ServerState::new_with_world_dir(reload_cfg, Some(dir.clone()));
    {
        let access = reloaded.access.read().await;
        assert!(access.is_banned("Griefer"));
        assert!(access.whitelist_enabled);
        assert!(access.is_allowed_by_whitelist("friend", false));
    }

    // Pardon, whitelist remove, whitelist off
    commands::handle_server_command(&reloaded, &admin_tx, "AdminUser", "pardon Griefer", None)
        .await;
    commands::handle_server_command(
        &reloaded,
        &admin_tx,
        "AdminUser",
        "whitelist remove Friend",
        None,
    )
    .await;
    commands::handle_server_command(&reloaded, &admin_tx, "AdminUser", "whitelist off", None).await;

    {
        let access = reloaded.access.read().await;
        assert!(!access.is_banned("Griefer"));
        assert!(!access.whitelist_enabled);
        assert!(access.is_allowed_by_whitelist("Stranger", false));
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn m1_multiplayer_gamemode_creative_adventure_tp_and_reconnect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.game_mode = 0;
    cfg.admins.insert("alice".into());
    let state = Arc::new(ServerState::new(cfg));

    let accept_state = Arc::clone(&state);
    tokio::spawn(async move {
        loop {
            let Ok((sock, peer)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&accept_state);
            tokio::spawn(async move {
                handle_connection(sock, peer, st).await;
            });
        }
    });

    let (mut p1, mut buf1) = connect_test_player(addr, "Alice").await;
    let (mut p2, mut buf2) = connect_test_player(addr, "Bob").await;

    // Drain Bob's join packets on Alice
    for _ in 0..64 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == SpawnEntity::ID
        {
            break;
        }
    }

    // 1. Alice switches to creative via /gamemode creative
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "gamemode creative".into(),
        },
    )
    .await
    .unwrap();

    // Bob receives PlayerInfoUpdate with Alice's game_mode == 1
    let mut saw_creative_tab = false;
    for _ in 0..16 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == PlayerInfoUpdate::ID
        {
            let upd = frame.decode_as::<PlayerInfoUpdate>().unwrap();
            if upd
                .entries
                .iter()
                .any(|e| e.name == "Alice" && e.game_mode.0 == 1)
            {
                saw_creative_tab = true;
                break;
            }
        }
    }
    assert!(
        saw_creative_tab,
        "Bob should see Alice switch to creative in PlayerInfoUpdate"
    );

    // 2. Creative instant break (status == 0 only, no StopDestroyBlock needed)
    send_packet(
        &mut p1,
        &PlayerAction {
            status: VarInt(0),
            location: BlockPos::new(2, 63, 2),
            face: 1,
            sequence: VarInt(1),
        },
    )
    .await
    .unwrap();

    let mut broken_seen = false;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == BlockUpdate::ID
        {
            let bu = frame.decode_as::<BlockUpdate>().unwrap();
            if bu.pos == BlockPos::new(2, 63, 2) && bu.block_state.0 == 0 {
                broken_seen = true;
                break;
            }
        }
    }
    assert!(
        broken_seen,
        "Creative mode should break blocks immediately on status=0"
    );

    // 3. Switch to adventure: block breaking & placing must be rejected
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "gamemode adventure".into(),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    send_packet(
        &mut p1,
        &PlayerAction {
            status: VarInt(0),
            location: BlockPos::new(3, 63, 2),
            face: 1,
            sequence: VarInt(2),
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut p1,
        &PlayerAction {
            status: VarInt(2),
            location: BlockPos::new(3, 63, 2),
            face: 1,
            sequence: VarInt(3),
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut p1,
        &UseItemOn {
            hand: VarInt(0),
            location: BlockPos::new(3, 63, 2),
            face: VarInt(1),
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: VarInt(4),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    {
        let mut world = state.world.write().await;
        assert_ne!(
            world.get_block(3, 63, 2),
            0,
            "Adventure must not break blocks"
        );
        assert_eq!(
            world.get_block(3, 64, 2),
            0,
            "Adventure must not place blocks"
        );
    }

    // 4. Switch back to creative, /tp 10 64 10, confirm teleport, and fly up
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "gamemode creative".into(),
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "tp 10 64 10".into(),
        },
    )
    .await
    .unwrap();

    let mut tp_pkt = None;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == SynchronizePlayerPosition::ID
        {
            tp_pkt = Some(frame.decode_as::<SynchronizePlayerPosition>().unwrap());
            break;
        }
    }
    let tp_pkt = tp_pkt.expect("Alice should receive SynchronizePlayerPosition from /tp");
    send_packet(
        &mut p1,
        &ConfirmTeleportation {
            teleport_id: tp_pkt.teleport_id,
            x: tp_pkt.x,
            y: tp_pkt.y,
            z: tp_pkt.z,
            yaw: tp_pkt.yaw,
            pitch: tp_pkt.pitch,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut p1,
        &MovePlayerPos {
            x: 10.5,
            y: 66.0,
            z: 10.5,
            flags: 0,
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    {
        let players = state.players.read().await;
        let alice = players.values().find(|p| p.username == "Alice").unwrap();
        assert!(
            (alice.y - 66.0).abs() < 0.1,
            "Creative mode should allow upward flight without rubberband, got y={}",
            alice.y
        );
    }

    // 5. Bob disconnects and reconnects cleanly
    drop(p2);
    let mut saw_remove = false;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == RemoveEntities::ID
        {
            saw_remove = true;
            break;
        }
    }
    assert!(
        saw_remove,
        "Alice should see RemoveEntities when Bob disconnects"
    );

    let (_p2_re, _buf2_re) = connect_test_player(addr, "Bob").await;
    let mut re_spawn = false;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == SpawnEntity::ID
        {
            re_spawn = true;
            break;
        }
    }
    assert!(re_spawn, "Alice should see Bob respawn on reconnect");
}

#[tokio::test]
async fn m11_ops_persistence_and_command_selectors() {
    for mode in [ServerForwardingMode::Modern, ServerForwardingMode::None] {
        let mut cfg = test_server_config(mode);
        let trusted = mode == ServerForwardingMode::Modern;
        cfg.admins.insert("rootadmin".into());

        let dir = std::env::temp_dir().join(format!(
            "radbyte_m11_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let state = ServerState::new_with_world_dir(cfg.clone(), Some(dir.clone()));
        let (tx1, _rx1, _c1) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
        let (tx2, _rx2, _c2) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);

        {
            let mut players = state.players.write().await;
            players.insert(
                1,
                PlayerSession {
                    entity_id: 1,
                    uuid: Uuid::from_u128(11),
                    username: "RootAdmin".into(),
                    properties: Vec::new(),
                    x: 0.5,
                    y: 64.0,
                    z: 0.5,
                    yaw: 0.0,
                    pitch: 0.0,
                    hp: 20.0,
                    food: 20,
                    saturation: 5.0,
                    on_ground: true,
                    fall_start_y: None,
                    dead: false,
                    game_mode: 0,
                    dimension: crate::world::worldgen::DimensionId::Overworld,
                    pending_keep_alive: None,
                    last_activity: Instant::now(),
                    inventory: Vec::new(),
                    selected_hotbar: 0,
                    open_chest: None,
                    chest_sync_tx: None,
                    pending_teleport_id: None,
                    tx: tx1.clone(),
                },
            );
            players.insert(
                2,
                PlayerSession {
                    entity_id: 2,
                    uuid: Uuid::from_u128(22),
                    username: "Helper".into(),
                    properties: Vec::new(),
                    x: 5.5,
                    y: 64.0,
                    z: 5.5,
                    yaw: 0.0,
                    pitch: 0.0,
                    hp: 20.0,
                    food: 20,
                    saturation: 5.0,
                    on_ground: true,
                    fall_start_y: None,
                    dead: false,
                    game_mode: 0,
                    dimension: crate::world::worldgen::DimensionId::Overworld,
                    pending_keep_alive: None,
                    last_activity: Instant::now(),
                    inventory: Vec::new(),
                    selected_hotbar: 0,
                    open_chest: None,
                    chest_sync_tx: None,
                    pending_teleport_id: None,
                    tx: tx2.clone(),
                },
            );
        }

        // Grant /op to Helper and verify Helper can run /gamemode creative @a and /tp @a 20 70 20
        commands::handle_server_command(&state, &tx1, "RootAdmin", "op Helper", None).await;
        assert_eq!(
            state
                .permission_level("Helper", Some(Uuid::from_u128(22)))
                .await,
            4
        );
        assert_eq!(state.is_admin("Helper").await, !trusted);
        assert_eq!(
            state
                .access
                .read()
                .await
                .op_entries
                .iter()
                .find(|entry| entry.name == "Helper")
                .unwrap()
                .uuid,
            if trusted {
                Some(Uuid::from_u128(22))
            } else {
                None
            }
        );

        commands::handle_server_command(&state, &tx2, "Helper", "gamemode creative @a", None).await;
        commands::handle_server_command(&state, &tx2, "Helper", "tp @a 20 70 20", None).await;
        {
            let players = state.players.read().await;
            for p in players.values() {
                assert_eq!(p.game_mode, 1);
                assert_eq!(p.x, 20.0);
                assert_eq!(p.y, 70.0);
                assert_eq!(p.z, 20.0);
            }
        }

        // Persist ops and verify reload + /deop
        state.save_all().await.unwrap();
        let reloaded = ServerState::new_with_world_dir(cfg, Some(dir.clone()));
        assert_eq!(
            reloaded
                .permission_level("Helper", Some(Uuid::from_u128(22)))
                .await,
            4
        );
        assert_eq!(reloaded.is_admin("Helper").await, !trusted);
        commands::handle_server_command(&reloaded, &tx1, "RootAdmin", "deop Helper", None).await;
        assert!(!reloaded.is_admin("Helper").await);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[tokio::test]
async fn vanilla_26_3_settings_uuid_whitelist_ops_and_status_behavior() {
    use rad_core::packet::status::{StatusRequest, StatusResponse};

    // 1. Modern forwarding: UUID is primary identity across username rename;
    //    max_players rejects non-ops when full; enforce_whitelist kicks online non-ops.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.white_list = Some(true);
    cfg.enforce_whitelist = true;
    cfg.max_players = 1;
    cfg.hide_online_players = true;
    let state = Arc::new(ServerState::new(cfg));

    let alice_uuid = Uuid::from_u128(0xAA11_CCEE_0000_0001);
    {
        let mut access = state.access.write().await;
        access.add_whitelist(Some(alice_uuid), "AliceOld".into());
        access.add_op(Some(alice_uuid), "AliceOld".into(), 3);
    }

    let accept_state = Arc::clone(&state);
    let accept_task = tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&accept_state);
            tokio::spawn(async move {
                handle_connection(stream, peer, st).await;
            });
        }
    });

    // Helper: connect with Modern forwarding using explicit UUID + username.
    async fn try_login_modern(
        addr: SocketAddr,
        secret: &[u8],
        uuid: Uuid,
        name: &str,
    ) -> Result<(TcpStream, BytesMut), String> {
        let mut client = TcpStream::connect(addr).await.unwrap();
        send_packet(
            &mut client,
            &Handshake {
                protocol_version: VarInt(PROTOCOL_VERSION),
                server_address: "localhost".into(),
                server_port: addr.port(),
                next_state: ConnectionState::Login,
            },
        )
        .await
        .unwrap();
        send_packet(
            &mut client,
            &LoginStart {
                username: name.into(),
                player_uuid: uuid,
            },
        )
        .await
        .unwrap();
        let mut buf = BytesMut::with_capacity(4096);
        let req = read_typed_packet::<LoginPluginRequest>(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        let fwd_bytes = encode_velocity_forwarding(
            &ForwardingData {
                address: "127.0.0.1".into(),
                uuid,
                username: name.into(),
                properties: Vec::new(),
            },
            secret,
        )
        .unwrap();
        send_packet(
            &mut client,
            &LoginPluginResponse {
                message_id: req.message_id,
                successful: true,
                data: fwd_bytes,
            },
        )
        .await
        .unwrap();
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == LoginDisconnect::ID {
            let dc = frame.decode_as::<LoginDisconnect>().unwrap();
            return Err(dc.reason_json);
        }
        assert_eq!(frame.id, LoginSuccess::ID);
        send_packet(&mut client, &LoginAcknowledged).await.unwrap();
        loop {
            let f = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
                .await
                .unwrap();
            if f.id == rad_core::packet::configuration::ClientboundKnownPacks::ID {
                send_packet(
                    &mut client,
                    &rad_core::packet::configuration::ServerboundKnownPacks { packs: Vec::new() },
                )
                .await
                .unwrap();
            } else if f.id == rad_core::packet::configuration::FinishConfiguration::ID {
                send_packet(
                    &mut client,
                    &rad_core::packet::configuration::AcknowledgeFinishConfiguration,
                )
                .await
                .unwrap();
                break;
            }
        }
        Ok((client, buf))
    }

    // 1a. Unlisted player rejected with LoginDisconnect
    let bob_uuid = Uuid::from_u128(0xBB00_0002);
    let err = try_login_modern(addr, b"server-test-secret", bob_uuid, "Bob")
        .await
        .unwrap_err();
    assert!(
        err.contains("not white-listed"),
        "expected whitelist disconnect, got {err}"
    );

    // 1b. Alice logs in under a NEW username ("AliceNew") -> matched by UUID, display name updated!
    let (mut alice_stream, mut alice_buf) =
        try_login_modern(addr, b"server-test-secret", alice_uuid, "AliceNew")
            .await
            .expect("renamed UUID-whitelisted player must be allowed");
    let mut alice_commands: Vec<String> = Vec::new();
    for _ in 0..40 {
        let f = read_raw_frame(&mut alice_stream, &mut alice_buf, 1024 * 1024)
            .await
            .unwrap();
        if f.id == DeclareCommands::ID {
            let dc = f.decode_as::<DeclareCommands>().unwrap();
            alice_commands = dc
                .nodes
                .iter()
                .filter_map(|n| match &n.kind {
                    CommandNodeKind::Literal { name } => Some(name.clone()),
                    _ => None,
                })
                .collect();
        } else if f.id == SynchronizePlayerPosition::ID {
            break;
        }
    }
    // Alice has op level 3 -> sees "op", "whitelist", NOT "save" (level 4)
    assert!(alice_commands.contains(&"op".to_owned()));
    assert!(alice_commands.contains(&"whitelist".to_owned()));
    assert!(!alice_commands.contains(&"save".to_owned()));

    // Verify display name was updated on UUID rename
    assert!(
        state
            .access
            .read()
            .await
            .whitelist_entries
            .iter()
            .any(|e| e.uuid == Some(alice_uuid) && e.name == "AliceNew")
    );

    // 1c. hide_online_players = true -> StatusResponse reports online = 0 even with Alice online
    {
        let mut status_sock = TcpStream::connect(addr).await.unwrap();
        send_packet(
            &mut status_sock,
            &Handshake {
                protocol_version: VarInt(PROTOCOL_VERSION),
                server_address: "localhost".into(),
                server_port: addr.port(),
                next_state: ConnectionState::Status,
            },
        )
        .await
        .unwrap();
        send_packet(&mut status_sock, &StatusRequest).await.unwrap();
        let mut sbuf = BytesMut::new();
        let resp = read_typed_packet::<StatusResponse>(&mut status_sock, &mut sbuf, 65536)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&resp.json).unwrap();
        assert_eq!(v["players"]["online"].as_i64(), Some(0));
    }

    // 1d. Whitelisted non-op Bob rejected when max_players (1) is reached
    state
        .access
        .write()
        .await
        .add_whitelist(Some(bob_uuid), "Bob".into());
    let full_err = try_login_modern(addr, b"server-test-secret", bob_uuid, "Bob")
        .await
        .unwrap_err();
    assert!(
        full_err.contains("server is full"),
        "expected full server disconnect, got {full_err}"
    );

    // 1e. enforce_whitelist = true: disable whitelist, let Bob in (raise max_players via op),
    //     or test direct kick via handle_console_command("whitelist on") after Bob joins while off.
    handle_console_command(&state, "whitelist off").await;
    // Allow Bob by removing whitelist limit? max_players is 1, so disconnect Alice first, let Bob & Charlie?
    // Instead drop Alice, let Bob join while whitelist is off, then run console `whitelist remove Bob` + `whitelist on`!
    drop(alice_stream);
    tokio::time::sleep(Duration::from_millis(80)).await;
    let (mut bob_stream, mut bob_buf) =
        try_login_modern(addr, b"server-test-secret", bob_uuid, "Bob")
            .await
            .expect("Bob can join when whitelist is off");
    for _ in 0..40 {
        let f = read_raw_frame(&mut bob_stream, &mut bob_buf, 1024 * 1024)
            .await
            .unwrap();
        if f.id == SynchronizePlayerPosition::ID {
            break;
        }
    }
    handle_console_command(&state, "whitelist remove Bob").await;
    handle_console_command(&state, "whitelist on").await;
    // Bob must be kicked immediately by enforce_whitelist = true!
    let mut kick_buf = [0u8; 256];
    let n = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::io::AsyncReadExt::read(&mut bob_stream, &mut kick_buf),
    )
    .await
    .unwrap()
    .unwrap_or(0);
    // Either PlayDisconnect packet bytes or immediate EOF (0) is valid; verify Bob is removed from players
    let _ = n;
    assert!(
        state.players.read().await.is_empty(),
        "enforce_whitelist must remove non-whitelisted online player Bob"
    );

    accept_task.abort();

    // 2. Direct mode: unverified username must NOT impersonate a UUID-bound op/whitelist entry!
    let d_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let d_addr = d_listener.local_addr().unwrap();
    let mut d_cfg = test_server_config(ServerForwardingMode::None);
    d_cfg.white_list = Some(true);
    d_cfg.enable_status = false;
    let d_state = Arc::new(ServerState::new(d_cfg));
    let real_uuid = Uuid::from_u128(0x1111_2222);
    let spoof_uuid = Uuid::from_u128(0x9999_8888);
    {
        let mut access = d_state.access.write().await;
        access.add_whitelist(Some(real_uuid), "TrustedUser".into());
        access.add_op(Some(real_uuid), "TrustedUser".into(), 4);
    }
    let d_accept_state = Arc::clone(&d_state);
    let d_task = tokio::spawn(async move {
        loop {
            let Ok((stream, peer)) = d_listener.accept().await else {
                break;
            };
            let st = Arc::clone(&d_accept_state);
            tokio::spawn(async move {
                handle_connection(stream, peer, st).await;
            });
        }
    });

    // 2a. enable_status = false -> Status ping gets no response (EOF/timeout)
    {
        let mut s = TcpStream::connect(d_addr).await.unwrap();
        send_packet(
            &mut s,
            &Handshake {
                protocol_version: VarInt(PROTOCOL_VERSION),
                server_address: "localhost".into(),
                server_port: d_addr.port(),
                next_state: ConnectionState::Status,
            },
        )
        .await
        .unwrap();
        send_packet(&mut s, &StatusRequest).await.unwrap();
        let mut sbuf = BytesMut::new();
        let res = tokio::time::timeout(
            Duration::from_millis(250),
            read_typed_packet::<StatusResponse>(&mut s, &mut sbuf, 65536),
        )
        .await;
        assert!(
            res.is_err() || matches!(res, Ok(None)),
            "enable_status=false must not send StatusResponse"
        );
    }

    // 2b. Spoofer in direct mode using "TrustedUser" with spoof_uuid is rejected!
    {
        let mut s = TcpStream::connect(d_addr).await.unwrap();
        send_packet(
            &mut s,
            &Handshake {
                protocol_version: VarInt(PROTOCOL_VERSION),
                server_address: "localhost".into(),
                server_port: d_addr.port(),
                next_state: ConnectionState::Login,
            },
        )
        .await
        .unwrap();
        send_packet(
            &mut s,
            &LoginStart {
                username: "TrustedUser".into(),
                player_uuid: spoof_uuid,
            },
        )
        .await
        .unwrap();
        let mut lbuf = BytesMut::new();
        let dc = read_typed_packet::<LoginDisconnect>(&mut s, &mut lbuf, 65536)
            .await
            .unwrap();
        assert!(dc.reason_json.contains("not white-listed"));
    }

    // 3. Runtime config enforcement: pvp = false, spawn_protection = 5, difficulty = 0 (peaceful)
    let mut g_cfg = test_server_config(ServerForwardingMode::None);
    g_cfg.pvp = false;
    g_cfg.spawn_protection = 5;
    g_cfg.difficulty = 0; // peaceful
    let g_state = Arc::new(ServerState::new(g_cfg));
    let (ptx, _prx, _pc) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    {
        let mut players = g_state.players.write().await;
        players.insert(
            1,
            PlayerSession {
                entity_id: 1,
                uuid: Uuid::from_u128(1),
                username: "PeacefulPlayer".into(),
                properties: Vec::new(),
                x: 0.5,
                y: 64.0,
                z: 0.5,
                yaw: 0.0,
                pitch: 0.0,
                hp: 10.0,
                food: 5,
                saturation: 0.0,
                on_ground: true,
                fall_start_y: None,
                dead: false,
                game_mode: 0,
                dimension: crate::world::worldgen::DimensionId::Overworld,
                pending_keep_alive: None,
                last_activity: Instant::now(),
                inventory: Vec::new(),
                selected_hotbar: 0,
                open_chest: None,
                chest_sync_tx: None,
                pending_teleport_id: None,
                tx: ptx,
            },
        );
    }
    step_player_survival_tick(&g_state, 1).await;
    {
        let players = g_state.players.read().await;
        let p = players.get(&1).unwrap();
        assert_eq!(p.hp, 11.0, "peaceful difficulty must regenerate health");
        assert_eq!(p.food, 20, "peaceful difficulty must restore hunger");
    }
    assert!(is_spawn_protected(
        &g_state.config,
        crate::world::worldgen::DimensionId::Overworld,
        BlockPos::new(5, 63, -5)
    ));
    assert!(!is_spawn_protected(
        &g_state.config,
        crate::world::worldgen::DimensionId::Overworld,
        BlockPos::new(6, 63, 0)
    ));

    d_task.abort();
}

#[test]
fn permission_event_and_gamemode_request_use_26_3_wire_format() {
    use rad_core::packet::Packet;
    let mut body = bytes::BytesMut::new();
    PermissionEvent {
        entity_id: 42,
        level: 4,
    }
    .encode_body(&mut body)
    .unwrap();
    assert_eq!(&body[..], &[0, 0, 0, 42, 28]);
    let decoded = PermissionEvent::decode_body(&mut body.freeze()).unwrap();
    assert_eq!((decoded.entity_id, decoded.level), (42, 4));
    let request = ChangeGameMode::decode_body(&mut &b"\x01"[..]).unwrap();
    assert_eq!(request.0.0, 1);
}

#[test]
fn uuid_bound_lists_do_not_authorize_a_different_trusted_uuid_by_name() {
    let mut access = AccessControl {
        whitelist_enabled: true,
        ..AccessControl::default()
    };
    let original = Uuid::from_u128(1);
    let different = Uuid::from_u128(2);
    access.add_whitelist(Some(original), "Example".into());
    access.add_op(Some(original), "Example".into(), 3);
    assert!(!access.is_allowed_by_whitelist_identity("Example", Some(different), false, true));
    assert_eq!(
        access.op_level_for_identity("Example", Some(different), true),
        0
    );
    assert!(access.is_allowed_by_whitelist_identity("Renamed", Some(original), false, true));
    assert_eq!(
        access.op_level_for_identity("Renamed", Some(original), true),
        3
    );
}

#[test]
fn paper_access_lists_migrate_legacy_and_roundtrip() {
    let dir = std::env::temp_dir().join(format!(
        "radbyte-paper-lists-{}-{}",
        std::process::id(),
        unique_tmp_suffix()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let id = Uuid::from_u128(0x1234);
    let mut original = AccessControl {
        whitelist_enabled: true,
        ..Default::default()
    };
    original.add_op(Some(id), "Player".into(), 3);
    original.add_whitelist(Some(id), "Player".into());
    std::fs::write(
        dir.join("access_control.json"),
        serde_json::to_vec(&original).unwrap(),
    )
    .unwrap();
    let migrated = AccessControl::load_from_dir(&dir);
    assert!(migrated.snapshot_if_dirty().is_some());
    AccessControl::write_snapshot_atomic(&dir, &migrated).unwrap();
    let ops: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("ops.json")).unwrap()).unwrap();
    assert_eq!(ops[0]["name"], "Player");
    assert_eq!(ops[0]["uuid"], id.to_string());
    assert_eq!(ops[0]["level"], 3);
    assert_eq!(ops[0]["bypassesPlayerLimit"], false);
    let whitelist: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("whitelist.json")).unwrap()).unwrap();
    assert_eq!(whitelist[0]["uuid"], id.to_string());
    assert_eq!(whitelist[0]["name"], "Player");
    // Paper-style manual removal is authoritative over legacy snapshot.
    std::fs::write(dir.join("ops.json"), b"[]").unwrap();
    let mut loaded = AccessControl::load_from_dir(&dir);
    assert!(loaded.op_entries.is_empty());
    assert_eq!(loaded.whitelist_entries.len(), 1);
    loaded.add_op(Some(id), "Player".into(), 4);
    AccessControl::write_snapshot_atomic(&dir, &loaded).unwrap();
    assert_eq!(AccessControl::load_from_dir(&dir).op_entries[0].level, 4);
    // Invalid user-edited files must never be overwritten by an autosave.
    std::fs::write(dir.join("ops.json"), b"broken json").unwrap();
    let bad = AccessControl::load_from_dir(&dir);
    assert!(AccessControl::write_snapshot_atomic(&dir, &bad).is_err());
    assert_eq!(std::fs::read(dir.join("ops.json")).unwrap(), b"broken json");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn styled_notices_encode_vanilla_translations_and_colors() {
    use rad_core::packet::Packet;
    let welcome = styled_text("Welcome to RadServer!", "aqua");
    let join = translated_text(
        "multiplayer.player.joined",
        NbtTag::String("Alex".into()),
        "yellow",
    );
    for content in [welcome.clone(), join.clone(), gamemode_notice(1).unwrap()] {
        let packet = SystemChatMessage {
            content: content.clone(),
            overlay: false,
        };
        let mut encoded = BytesMut::new();
        packet.encode_body(&mut encoded).unwrap();
        assert_eq!(
            SystemChatMessage::decode_body(&mut encoded.freeze())
                .unwrap()
                .content,
            content
        );
    }
    let NbtTag::Compound(welcome_fields) = welcome else {
        panic!("welcome must be a component")
    };
    assert_eq!(
        welcome_fields.get("color"),
        Some(&NbtTag::String("aqua".into()))
    );
    let NbtTag::Compound(join_fields) = join else {
        panic!("join must be a component")
    };
    assert_eq!(
        join_fields.get("translate"),
        Some(&NbtTag::String("multiplayer.player.joined".into()))
    );
    assert_eq!(
        join_fields.get("color"),
        Some(&NbtTag::String("yellow".into()))
    );
    for mode in 0..=3 {
        assert!(gamemode_notice(mode).is_some());
    }
    assert!(gamemode_notice(4).is_none());
}

#[test]
fn declare_commands_includes_subcommands_suggestions_and_filters_by_permission() {
    let lvl0 = commands::build_declare_commands_for_level(0);
    let root_literals = |dc: &DeclareCommands| -> Vec<String> {
        dc.nodes[0]
            .children
            .iter()
            .filter_map(|idx| match &dc.nodes[idx.0 as usize].kind {
                CommandNodeKind::Literal { name } => Some(name.clone()),
                _ => None,
            })
            .collect()
    };

    let cmds0 = root_literals(&lvl0);
    assert!(cmds0.contains(&"help".to_owned()));
    assert!(cmds0.contains(&"dimension".to_owned()));
    assert!(!cmds0.contains(&"gamemode".to_owned()));
    assert!(!cmds0.contains(&"op".to_owned()));
    assert!(!cmds0.contains(&"stop".to_owned()));

    let dim_children: Vec<String> = lvl0
        .nodes
        .iter()
        .find(|n| matches!(&n.kind, CommandNodeKind::Literal { name } if name == "dimension"))
        .unwrap()
        .children
        .iter()
        .filter_map(|idx| match &lvl0.nodes[idx.0 as usize].kind {
            CommandNodeKind::Literal { name } => Some(name.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(dim_children, vec!["overworld", "nether", "end"]);

    let lvl2 = commands::build_declare_commands_for_level(2);
    let cmds2 = root_literals(&lvl2);
    assert!(cmds2.contains(&"gamemode".to_owned()));
    assert!(cmds2.contains(&"give".to_owned()));
    assert!(!cmds2.contains(&"op".to_owned()));
    assert!(!cmds2.contains(&"stop".to_owned()));
    assert!(lvl2.nodes.iter().any(|n| {
        matches!(&n.kind, CommandNodeKind::GreedyStringArg { name } if name == "item")
            && n.suggestions_type.as_deref() == Some("minecraft:ask_server")
    }));

    let lvl4 = commands::build_declare_commands_for_level(4);
    let cmds4 = root_literals(&lvl4);
    assert!(cmds4.contains(&"op".to_owned()));
    assert!(cmds4.contains(&"whitelist".to_owned()));
    assert!(cmds4.contains(&"stop".to_owned()));
}

#[tokio::test]
async fn command_suggestions_are_filtered_by_permissions_and_suggest_arguments() {
    let dir = temp_test_dir("cmd-suggestions");
    let cfg = test_server_config(&dir);
    let mut state = ServerState::new(cfg, Arc::new(World::flat(2, 0)));
    state.access = RwLock::new({
        let mut ac = AccessControl::default();
        ac.add_op(None, "ModUser".into(), 2);
        ac.add_op(None, "AdminUser".into(), 3);
        ac
    });

    // Regular player (level 0): "/" only shows level 0 commands, "/gamemode " yields no suggestions
    let resp0 = commands::build_command_suggestions(
        &state,
        "GuestUser",
        None,
        &CommandSuggestionsRequest {
            id: VarInt(1),
            text: "/".into(),
        },
    )
    .await;
    let names0: Vec<&str> = resp0.matches.iter().map(|m| m.text.as_str()).collect();
    assert!(names0.contains(&"help"));
    assert!(names0.contains(&"dimension"));
    assert!(!names0.contains(&"gamemode"));
    assert!(!names0.contains(&"op"));
    assert!(resp0.matches.iter().all(|m| m.tooltip.is_some()));

    let denied_arg = commands::build_command_suggestions(
        &state,
        "GuestUser",
        None,
        &CommandSuggestionsRequest {
            id: VarInt(2),
            text: "/gamemode ".into(),
        },
    )
    .await;
    assert!(denied_arg.matches.is_empty());

    // Level 2 user: "/gamemode cr" suggests "creative" at start=10, length=2; "/op " is still empty
    let gm_resp = commands::build_command_suggestions(
        &state,
        "ModUser",
        None,
        &CommandSuggestionsRequest {
            id: VarInt(3),
            text: "/gamemode cr".into(),
        },
    )
    .await;
    assert_eq!(gm_resp.start, VarInt(10));
    assert_eq!(gm_resp.length, VarInt(2));
    assert_eq!(gm_resp.matches.len(), 1);
    assert_eq!(gm_resp.matches[0].text, "creative");

    let denied_op = commands::build_command_suggestions(
        &state,
        "ModUser",
        None,
        &CommandSuggestionsRequest {
            id: VarInt(4),
            text: "/op ".into(),
        },
    )
    .await;
    assert!(denied_op.matches.is_empty());

    // Level 3 user: "/whitelist " suggests all subcommands
    let wl_resp = commands::build_command_suggestions(
        &state,
        "AdminUser",
        None,
        &CommandSuggestionsRequest {
            id: VarInt(5),
            text: "/whitelist ".into(),
        },
    )
    .await;
    let wl_names: Vec<&str> = wl_resp.matches.iter().map(|m| m.text.as_str()).collect();
    for expected in ["add", "list", "off", "on", "reload", "remove"] {
        assert!(wl_names.contains(&expected));
    }

    let _ = std::fs::remove_dir_all(&dir);
}
