use super::*;

#[tokio::test]
async fn e2e_server_status_and_ping() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(ServerState::new(test_server_config(
        ServerForwardingMode::None,
    )));
    let st = Arc::clone(&state);
    tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, st).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "localhost".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Status,
        },
    )
    .await
    .unwrap();
    send_packet(&mut client, &StatusRequest).await.unwrap();

    let mut buf = BytesMut::new();
    let resp: StatusResponse = read_typed_packet(&mut client, &mut buf, 65536)
        .await
        .unwrap();
    assert!(resp.json.contains("RadServer Test"));
    send_packet(&mut client, &PingRequest { timestamp: 99 })
        .await
        .unwrap();
    let pong: PongResponse = read_typed_packet(&mut client, &mut buf, 65536)
        .await
        .unwrap();
    assert_eq!(pong.timestamp, 99);
}

#[tokio::test]
async fn e2e_server_empty_known_packs_sends_nbt_registries() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = Arc::new(ServerState::new(test_server_config(
        ServerForwardingMode::None,
    )));
    let st = Arc::clone(&state);
    tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, st).await;
    });

    let mut client = TcpStream::connect(addr).await.unwrap();
    let mut buf = BytesMut::new();
    send_packet(
        &mut client,
        &Handshake {
            protocol_version: VarInt(PROTOCOL_VERSION),
            server_address: "127.0.0.1".into(),
            server_port: addr.port(),
            next_state: ConnectionState::Login,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut client,
        &LoginStart {
            username: "NoPacks".into(),
            player_uuid: Uuid::from_u128(1),
        },
    )
    .await
    .unwrap();
    let _success: LoginSuccess = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    send_packet(&mut client, &LoginAcknowledged).await.unwrap();

    // Reply with empty ServerboundKnownPacks -> server must send inline NBT RegistryData
    let mut saw_nbt_registry = false;
    loop {
        let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == ClientboundKnownPacks::ID {
            let _ = frame.decode_as::<ClientboundKnownPacks>().unwrap();
            send_packet(
                &mut client,
                &rad_core::packet::configuration::ServerboundKnownPacks { packs: Vec::new() },
            )
            .await
            .unwrap();
        } else if frame.id == RegistryData::ID {
            let reg = frame.decode_as::<RegistryData>().unwrap();
            if reg.entries.iter().any(|e| e.data.is_some()) {
                saw_nbt_registry = true;
            }
        } else if frame.id == FinishConfiguration::ID {
            break;
        }
    }
    assert!(saw_nbt_registry, "expected fallback inline NBT registries");
}

#[tokio::test]
async fn outbound_channel_enforces_byte_budget_and_triggers_unified_cancel() {
    let (tx, mut rx, mut cancel_rx) = outbound_channel(16, 100);
    assert!(!tx.is_cancelled());
    assert!(tx.try_send(Bytes::from(vec![1u8; 60])));
    assert!(rx.recv().await.is_some());

    // After dequeue, byte budget is freed and another 60 bytes fit.
    assert!(tx.try_send(Bytes::from(vec![2u8; 60])));
    // Exceeding byte budget (60 + 50 > 100) must reject and trigger unified cancellation!
    assert!(!tx.try_send(Bytes::from(vec![3u8; 50])));
    assert!(tx.is_cancelled());
    assert!(*cancel_rx.borrow_and_update());
}

#[tokio::test]
async fn step_keep_alive_sends_single_id_and_disconnects_timed_out_session() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.difficulty = 2; // Normal: starvation damages down to 1.0 HP
    let state = ServerState::new(cfg);
    let (tx, mut rx, _cancel_rx) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    state.players.write().await.insert(
        1,
        PlayerSession {
            entity_id: 1,
            uuid: Uuid::from_u128(1),
            username: "TimeoutUser".into(),
            properties: Vec::new(),
            x: 0.0,
            y: 64.0,
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
            pending_keep_alive: None,
            last_activity: Instant::now(),
            inventory: Vec::new(),
            selected_hotbar: 0,
            open_chest: None,
            chest_sync_tx: None,
            pending_teleport_id: None,
            tx: tx.clone(),
        },
    );

    // First keep-alive sends ID 10.
    step_keep_alive(&state, 10, Duration::from_millis(40)).await;
    assert!(rx.recv().await.is_some());

    // Second call before timeout must NOT send ID 11 (single outstanding ID).
    step_keep_alive(&state, 11, Duration::from_millis(40)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), rx.recv())
            .await
            .is_err()
    );

    // After timeout elapses, player is removed and unified cancel is triggered.
    tokio::time::sleep(Duration::from_millis(50)).await;
    step_keep_alive(&state, 12, Duration::from_millis(40)).await;
    assert!(state.players.read().await.is_empty());
    assert!(tx.is_cancelled());
}

#[tokio::test]
async fn step_idle_timeouts_disconnects_only_idle_players() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.player_idle_timeout = 1;
    let state = Arc::new(ServerState::new(cfg));

    let (tx_idle, mut rx_idle, _cancel_idle) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    let (tx_active, mut rx_active, _cancel_active) =
        outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    let mut players = state.players.write().await;
    players.insert(
        1,
        PlayerSession {
            entity_id: 1,
            uuid: Uuid::from_u128(101),
            username: "IdlePlayer".into(),
            properties: Vec::new(),
            x: 0.0,
            y: 64.0,
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
            last_activity: Instant::now() - Duration::from_secs(120),
            inventory: Vec::new(),
            selected_hotbar: 0,
            tx: tx_idle,
        },
    );
    players.insert(
        2,
        PlayerSession {
            entity_id: 2,
            uuid: Uuid::from_u128(102),
            username: "ActivePlayer".into(),
            properties: Vec::new(),
            x: 0.0,
            y: 64.0,
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
            tx: tx_active,
        },
    );
    drop(players);

    step_idle_timeouts(&state, Duration::from_secs(60)).await;

    let players = state.players.read().await;
    assert!(!players.contains_key(&1), "idle player must be removed");
    assert!(players.contains_key(&2), "active player must remain");
    assert!(
        rx_idle.rx.try_recv().is_ok(),
        "idle player should receive PlayDisconnect"
    );
    assert!(
        rx_active.rx.try_recv().is_err(),
        "active player should not receive disconnect"
    );
}

#[tokio::test]
async fn headless_client_status_play_and_unsupported_protocol_rejection() {
    use rad_core::client::{ClientConfig, ClientError, HeadlessClient};

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let cfg = test_server_config(ServerForwardingMode::None);
    let state = Arc::new(ServerState::new(cfg));
    spawn_tick_loop(Arc::clone(&state));

    let accept_state = Arc::clone(&state);
    let server_task = tokio::spawn(async move {
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

    let ok_cfg = ClientConfig::default();
    let status = tokio::task::spawn_blocking(move || {
        HeadlessClient::fetch_status(addr, "127.0.0.1", addr.port(), &ok_cfg)
    })
    .await
    .unwrap()
    .unwrap();
    assert!(status.ping_echoed);
    assert!(status.json.contains("777"));

    let ok_cfg2 = ClientConfig::default();
    let mut session = tokio::task::spawn_blocking(move || {
        HeadlessClient::connect_play(addr, "127.0.0.1", addr.port(), "HeadlessTester", &ok_cfg2)
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(session.username, "HeadlessTester");
    // Poll initial Play packets (e.g. SynchronizePlayerPosition / KeepAlive) without error.
    let _ = tokio::task::spawn_blocking(move || session.poll_event())
        .await
        .unwrap()
        .unwrap();

    // Unsupported protocol version must be rejected cleanly with Disconnected (never decoded as 777).
    let bad_cfg = ClientConfig {
        protocol_version: 999,
        ..ClientConfig::default()
    };
    let err = tokio::task::spawn_blocking(move || {
        HeadlessClient::connect_play(addr, "127.0.0.1", addr.port(), "BadProto", &bad_cfg)
    })
    .await
    .unwrap()
    .err()
    .expect("unsupported protocol must be rejected");
    assert!(
        matches!(err, ClientError::Disconnected(_)),
        "expected clean Disconnect, got {err:?}"
    );

    server_task.abort();
}
