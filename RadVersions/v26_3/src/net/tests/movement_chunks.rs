use super::*;

#[tokio::test]
async fn e2e_server_two_players_spawn_move_and_chat() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.game_mode = 0; // Survival mode for block break/place
    let state = Arc::new(ServerState::new(cfg));
    spawn_tick_loop(Arc::clone(&state));

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

    // Alice should receive SpawnEntity for Bob
    let mut saw_bob_spawn = false;
    for _ in 0..10 {
        let frame = read_raw_frame(&mut p1, &mut buf1, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == SpawnEntity::ID {
            saw_bob_spawn = true;
            break;
        }
    }
    assert!(saw_bob_spawn, "Alice should see Bob spawn");

    // Alice moves -> Bob receives MoveEntityPosRot
    send_packet(
        &mut p1,
        &MovePlayerPosRot {
            x: 1.5,
            y: 64.0,
            z: 0.5,
            yaw: 45.0,
            pitch: 0.0,
            flags: 1,
        },
    )
    .await
    .unwrap();

    let mut saw_move = false;
    for _ in 0..10 {
        let frame = read_raw_frame(&mut p2, &mut buf2, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == MoveEntityPosRot::ID {
            saw_move = true;
            break;
        }
    }
    assert!(saw_move, "Bob should receive Alice movement delta");

    // Alice chats -> Bob receives "<Alice> hi bob"
    send_packet(
        &mut p1,
        &ServerboundChat {
            message: "hi bob".into(),
            timestamp: 1,
            salt: 2,
        },
    )
    .await
    .unwrap();

    let mut saw_chat = false;
    for _ in 0..10 {
        let frame = read_raw_frame(&mut p2, &mut buf2, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == SystemChatMessage::ID
            && let Ok(chat) = frame.decode_as::<SystemChatMessage>()
            && chat.content == NbtTag::String("<Alice> hi bob".into())
        {
            saw_chat = true;
            break;
        }
    }
    assert!(saw_chat, "Bob should receive Alice chat");

    // Alice runs /tps
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "tps".into(),
        },
    )
    .await
    .unwrap();

    let mut saw_tps = false;
    for _ in 0..10 {
        let frame = read_raw_frame(&mut p1, &mut buf1, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == SystemChatMessage::ID
            && let Ok(chat) = frame.decode_as::<SystemChatMessage>()
            && let NbtTag::String(s) = chat.content
            && s.contains("Target TPS: 20")
        {
            saw_tps = true;
            break;
        }
    }
    assert!(saw_tps, "Alice should receive /tps output");
}

#[tokio::test]
async fn chunk_stream_waits_for_capacity_without_disconnecting() {
    let (tx, mut rx, _) = outbound_channel(2, 100);
    assert!(tx.try_send(Bytes::from(vec![1; 60])));
    let producer = tokio::spawn({
        let tx = tx.clone();
        async move { tx.send_wait(Bytes::from(vec![2; 60])).await }
    });
    tokio::task::yield_now().await;
    assert!(!producer.is_finished());
    assert!(!tx.is_cancelled());
    assert_eq!(rx.recv().await.unwrap().len(), 60);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), producer)
            .await
            .unwrap()
            .unwrap()
    );
    assert_eq!(rx.recv().await.unwrap().len(), 60);
}

#[tokio::test]
async fn large_view_distance_completes_initial_join() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.view_distance = 12;
    cfg.simulation_distance = 8;
    let state = Arc::new(ServerState::new(cfg));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_state = Arc::clone(&state);
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });
    let (stream, _, login) = tokio::time::timeout(
        Duration::from_secs(60),
        connect_test_player_with_login(addr, "ChunkViewer"),
    )
    .await
    .expect("large-distance login should finish without disconnecting");
    assert_eq!(login.view_distance.0, 12);
    assert_eq!(login.simulation_distance.0, 8);
    assert!(!task.is_finished());
    drop(stream);
    task.abort();
}

#[tokio::test]
async fn independent_simulation_distance_wire_lifecycle_and_restart() {
    let dir = std::env::temp_dir().join(format!("rad_sim_dist_net_e2e_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.view_distance = 6;
    cfg.simulation_distance = 2;

    let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_state = Arc::clone(&state);
    let accept_task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });

    // 1. Connect player and verify PlayLogin carries view_distance=6 and simulation_distance=2
    let (stream, _buf, play_login) = connect_test_player_with_login(addr, "sim_p1").await;
    assert_eq!(play_login.view_distance.0, 6);
    assert_eq!(play_login.simulation_distance.0, 2);

    // 2. Setup active furnace at (0, 64, 0) [chunk (0,0)] and inactive furnace+sand at (48, 64, 0) [chunk (3,0)]
    {
        let mut furnaces = state.furnaces.write().await;
        for bx in [0, 48] {
            furnaces.insert_in(
                crate::world::worldgen::DimensionId::Overworld,
                bx,
                64,
                0,
                crate::world::sim::FurnaceEntity {
                    input_item: rad_core::mc777_data::items::COBBLESTONE,
                    input_count: 2,
                    fuel_item: rad_core::mc777_data::items::COAL,
                    fuel_count: 1,
                    ..Default::default()
                },
            );
        }
    }
    {
        let mut world = state.world.write().await;
        world.set_block_in(
            crate::world::worldgen::DimensionId::Overworld,
            48,
            66,
            0,
            crate::world::sim::SAND_BLOCK_STATE,
        );
        world.set_block_in(crate::world::worldgen::DimensionId::Overworld, 48, 65, 0, 0);
        world.set_block_in(crate::world::worldgen::DimensionId::Overworld, 48, 64, 0, 1);
        state.world_sim.write().await.on_block_changed_in(
            &mut world,
            crate::world::worldgen::DimensionId::Overworld,
            48,
            66,
            0,
            0,
        );
    }

    // 3. Run 10 ticks while player is in chunk (0, 0): active furnace advances, chunk (3, 0) stays paused
    for t in 1..=10 {
        let area = state.simulation_area().await;
        step_world_tick_in_area(&state, &area).await;
        let mut world = state.world.write().await;
        state
            .world_sim
            .write()
            .await
            .step_in_area(&mut world, t, &area);
        state.furnaces.write().await.tick_in_area(&area);
    }
    {
        let furnaces = state.furnaces.read().await;
        assert_eq!(
            furnaces
                .get_in(crate::world::worldgen::DimensionId::Overworld, 0, 64, 0)
                .unwrap()
                .cook_progress,
            10
        );
        assert_eq!(
            furnaces
                .get_in(crate::world::worldgen::DimensionId::Overworld, 48, 64, 0)
                .unwrap()
                .cook_progress,
            0
        );
        let mut world = state.world.write().await;
        assert_eq!(
            world.get_block_in(crate::world::worldgen::DimensionId::Overworld, 48, 66, 0),
            crate::world::sim::SAND_BLOCK_STATE
        );
    }

    // 4. Teleport player to (32.5, 64.0, 0.5) [chunk (2, 0)]: chunk (3, 0) is now within simulation-distance=2
    {
        let mut players = state.players.write().await;
        for p in players.values_mut() {
            p.x = 32.5;
            p.z = 0.5;
        }
    }
    for t in 11..=15 {
        let area = state.simulation_area().await;
        let mut world = state.world.write().await;
        state
            .world_sim
            .write()
            .await
            .step_in_area(&mut world, t, &area);
        state.furnaces.write().await.tick_in_area(&area);
    }
    {
        let furnaces = state.furnaces.read().await;
        assert_eq!(
            furnaces
                .get_in(crate::world::worldgen::DimensionId::Overworld, 48, 64, 0)
                .unwrap()
                .cook_progress,
            5
        );
        let mut world = state.world.write().await;
        let sim = state.world_sim.read().await;
        let landed = world.get_block_in(crate::world::worldgen::DimensionId::Overworld, 48, 65, 0)
            == crate::world::sim::SAND_BLOCK_STATE;
        let falling = sim
            .falling_blocks
            .iter()
            .any(|fb| fb.block_state == crate::world::sim::SAND_BLOCK_STATE);
        assert!(landed || falling);
    }

    // 5. Switch player to Nether -> Overworld chunk (3, 0) pauses again
    {
        let mut players = state.players.write().await;
        for p in players.values_mut() {
            p.dimension = crate::world::worldgen::DimensionId::Nether;
        }
    }
    {
        let area = state.simulation_area().await;
        state.furnaces.write().await.tick_in_area(&area);
        let furnaces = state.furnaces.read().await;
        assert_eq!(
            furnaces
                .get_in(crate::world::worldgen::DimensionId::Overworld, 48, 64, 0)
                .unwrap()
                .cook_progress,
            5
        );
    }

    // 6. Disconnect player, save world, and reload -> state persists cleanly
    drop(stream);
    let _ = accept_task.await;
    assert!(state.simulation_area().await.is_empty());
    state.save_all().await.unwrap();

    let reloaded = ServerState::new_with_world_dir(state.config.clone(), Some(dir.clone()));
    assert_eq!(
        reloaded
            .furnaces
            .read()
            .await
            .get_in(crate::world::worldgen::DimensionId::Overworld, 48, 64, 0)
            .unwrap()
            .cook_progress,
        5
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn max_world_size_rejects_out_of_bounds_movement() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.max_world_size = 10;
    let state = Arc::new(ServerState::new(cfg));

    let (tx, mut rx, _cancel_rx) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    state.players.write().await.insert(
        1,
        PlayerSession {
            entity_id: 1,
            uuid: Uuid::from_u128(201),
            username: "BorderPlayer".into(),
            properties: Vec::new(),
            x: 9.0,
            y: 65.0,
            z: 0.0,
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
            open_chest: None,
            chest_sync_tx: None,
            pending_teleport_id: None,
            pending_keep_alive: None,
            last_activity: Instant::now(),
            inventory: Vec::new(),
            selected_hotbar: 0,
            tx: tx.clone(),
        },
    );

    let mut loaded = HashSet::new();
    let (mut cx, mut cy, mut cz) = (9.0, 65.0, 0.0);
    let (mut cyaw, mut cpitch) = (0.0, 0.0);
    let (mut ccx, mut ccz) = (0, 0);

    handle_player_move(
        &state,
        1,
        &tx,
        &mut loaded,
        &mut cx,
        &mut cy,
        &mut cz,
        &mut cyaw,
        &mut cpitch,
        &mut ccx,
        &mut ccz,
        Some((11.5, 65.0, 0.0)),
        None,
        0.05,
        state.config.view_distance,
    )
    .await;

    assert_eq!(cx, 9.0, "out-of-bounds X must be snapped back");
    assert!(
        rx.rx.try_recv().is_ok(),
        "must send SynchronizePlayerPosition snap-back"
    );
}

#[tokio::test]
async fn view_distance_12_simulation_distance_8_streams_625_chunks_and_moves_cleanly() {
    let mut cfg = test_server_config(ServerForwardingMode::None);
    cfg.view_distance = 12;
    cfg.simulation_distance = 8;
    let state = Arc::new(ServerState::new(cfg));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let st = Arc::clone(&state);
    let accept = tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, st).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(777),
            server_address: "127.0.0.1".into(),
            server_port: addr.port(),
            next_state: rad_core::state::ConnectionState::Login,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut client,
        &LoginStart {
            username: "View12Player".into(),
            player_uuid: Uuid::from_u128(401),
        },
    )
    .await
    .unwrap();

    let mut buf = BytesMut::with_capacity(65536);
    let _success: LoginSuccess = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    send_packet(&mut client, &LoginAcknowledged).await.unwrap();

    let mut play_login = None;
    let mut initial_chunks = 0usize;
    loop {
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == ClientboundKnownPacks::ID {
            let known = frame.decode_as::<ClientboundKnownPacks>().unwrap();
            send_packet(
                &mut client,
                &rad_core::packet::configuration::ServerboundKnownPacks { packs: known.packs },
            )
            .await
            .unwrap();
        } else if frame.id == FinishConfiguration::ID {
            send_packet(&mut client, &AcknowledgeFinishConfiguration)
                .await
                .unwrap();
        } else if frame.id == PlayLogin::ID {
            play_login = Some(frame.decode_as::<PlayLogin>().unwrap());
        } else if frame.id == rad_core::packet::play::LevelChunkWithLight::ID {
            initial_chunks += 1;
        } else if frame.id == SynchronizePlayerPosition::ID {
            let tp = frame.decode_as::<SynchronizePlayerPosition>().unwrap();
            send_packet(
                &mut client,
                &ConfirmTeleportation {
                    teleport_id: tp.teleport_id,
                    x: tp.x,
                    y: tp.y,
                    z: tp.z,
                    yaw: tp.yaw,
                    pitch: tp.pitch,
                },
            )
            .await
            .unwrap();
            break;
        }
    }

    let play_login = play_login.expect("player must receive PlayLogin");
    assert_eq!(play_login.view_distance.0, 12);
    assert_eq!(play_login.simulation_distance.0, 8);
    assert_eq!(
        initial_chunks, 625,
        "view-distance=12 must stream (2*12+1)^2 = 625 chunks"
    );

    // Step across chunk boundary (0 -> 1) at realistic speed (~10 blocks/sec)
    tokio::time::sleep(Duration::from_millis(100)).await;
    for step in 1..=17 {
        send_packet(
            &mut client,
            &MovePlayerPos {
                x: f64::from(step),
                y: 65.0,
                z: 0.5,
                flags: 0x01,
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let mut new_chunks = 0usize;
    let mut unloaded = 0usize;
    let mut new_center = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline && new_chunks < 25 {
        if let Ok(Some(frame)) = tokio::time::timeout(
            Duration::from_millis(300),
            read_raw_frame(&mut client, &mut buf, 1024 * 1024),
        )
        .await
        {
            if frame.id == rad_core::packet::play::LevelChunkWithLight::ID {
                new_chunks += 1;
            } else if frame.id == UnloadChunk::ID {
                unloaded += 1;
            } else if frame.id == rad_core::packet::play::SetChunkCacheCenter::ID {
                new_center = Some(
                    frame
                        .decode_as::<rad_core::packet::play::SetChunkCacheCenter>()
                        .unwrap()
                        .chunk_x
                        .0,
                );
            }
        }
    }
    assert_eq!(new_center, Some(1));
    assert_eq!(
        new_chunks, 25,
        "crossing into cx=1 must load 25 new edge chunks"
    );
    assert_eq!(
        unloaded, 25,
        "crossing into cx=1 must unload 25 old edge chunks"
    );

    drop(client);
    let _ = accept.await;
}

#[tokio::test]
async fn view_distance_12_disconnect_mid_stream_does_not_hang_or_leak_world_lock() {
    let mut cfg = test_server_config(ServerForwardingMode::None);
    cfg.view_distance = 12;
    cfg.simulation_distance = 8;
    let state = Arc::new(ServerState::new(cfg));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let st = Arc::clone(&state);
    let accept = tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, st).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(777),
            server_address: "127.0.0.1".into(),
            server_port: addr.port(),
            next_state: rad_core::state::ConnectionState::Login,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut client,
        &LoginStart {
            username: "MidDrop12".into(),
            player_uuid: Uuid::from_u128(402),
        },
    )
    .await
    .unwrap();

    let mut buf = BytesMut::with_capacity(8192);
    let _success: LoginSuccess = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    send_packet(&mut client, &LoginAcknowledged).await.unwrap();

    loop {
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == ClientboundKnownPacks::ID {
            let known = frame.decode_as::<ClientboundKnownPacks>().unwrap();
            send_packet(
                &mut client,
                &rad_core::packet::configuration::ServerboundKnownPacks { packs: known.packs },
            )
            .await
            .unwrap();
        } else if frame.id == FinishConfiguration::ID {
            send_packet(&mut client, &AcknowledgeFinishConfiguration)
                .await
                .unwrap();
        } else if frame.id == rad_core::packet::play::LevelChunkWithLight::ID {
            // Drop socket abruptly right after receiving the first chunk
            drop(client);
            break;
        }
    }

    tokio::time::timeout(Duration::from_secs(5), accept)
        .await
        .expect("server connection handler must exit promptly on mid-stream disconnect")
        .unwrap();
    // World lock must remain free and no lingering player session
    assert!(state.world.try_write().is_ok());
    assert!(state.players.read().await.is_empty());
}
