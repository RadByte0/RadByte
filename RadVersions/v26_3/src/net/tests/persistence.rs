use super::*;

#[tokio::test]
async fn player_inventory_and_hotbar_persist_atomically_across_save_all() {
    let dir = std::env::temp_dir().join("rad_playerdata_atomic_test");
    let _ = std::fs::remove_dir_all(&dir);

    let state = ServerState::new_with_world_dir(
        test_server_config(ServerForwardingMode::None),
        Some(dir.clone()),
    );
    let (tx, _rx, _cancel) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    let uuid = Uuid::from_u128(0xABCD);
    state.players.write().await.insert(
        42,
        PlayerSession {
            entity_id: 42,
            uuid,
            username: "PersistUser".into(),
            properties: Vec::new(),
            x: 12.5,
            y: 65.0,
            z: -4.5,
            yaw: 90.0,
            pitch: 15.0,
            hp: 16.5,
            food: 18,
            saturation: 3.5,
            on_ground: true,
            fall_start_y: None,
            dead: false,
            game_mode: 0,
            dimension: crate::world::worldgen::DimensionId::Overworld,
            pending_keep_alive: None,
            last_activity: Instant::now(),
            inventory: vec![SavedSlot {
                item_id: 1,
                count: 42,
                ..Default::default()
            }],
            selected_hotbar: 4,
            open_chest: None,
            chest_sync_tx: None,
            pending_teleport_id: None,
            tx,
        },
    );

    state.save_all().await.unwrap();
    let loaded = state.load_player_data(uuid).await.unwrap();
    assert_eq!(loaded.x, 12.5);
    assert_eq!(loaded.y, 65.0);
    assert_eq!(loaded.z, -4.5);
    assert_eq!(loaded.hp, 16.5);
    assert_eq!(loaded.selected_hotbar, 4);
    assert_eq!(loaded.inventory.len(), 1);
    assert_eq!(loaded.inventory[0].item_id, 1);
    assert_eq!(loaded.inventory[0].count, 42);

    let restarted = ServerState::new_with_world_dir(
        test_server_config(ServerForwardingMode::None),
        Some(dir.clone()),
    );
    let reloaded = restarted.load_player_data(uuid).await.unwrap();
    assert_eq!(reloaded.inventory[0].count, 42);
    assert_eq!(reloaded.selected_hotbar, 4);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn save_all_world_clock_does_not_inflate_without_ticks() {
    let dir = std::env::temp_dir().join(format!(
        "radbyte_g02_clock_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = test_server_config(ServerForwardingMode::Modern);
    let state = ServerState::new_with_world_dir(cfg, Some(dir.clone()));

    let init_day_time = state.level_meta.read().await.day_time;
    state.tick_count.store(40, Ordering::Relaxed);
    for _ in 0..100 {
        state.save_all().await.unwrap();
    }
    {
        let meta = state.level_meta.read().await;
        assert_eq!(
            meta.time, 40,
            "repeated save_all without ticks must not inflate world clock"
        );
        assert_eq!(meta.day_time, (init_day_time + 40) % 24000);
    }

    state.tick_count.store(55, Ordering::Relaxed);
    state.save_all().await.unwrap();
    {
        let meta = state.level_meta.read().await;
        assert_eq!(meta.time, 55);
        assert_eq!(meta.day_time, (init_day_time + 55) % 24000);
    }
    let restarted = ServerState::new_with_world_dir(
        test_server_config(ServerForwardingMode::Modern),
        Some(dir.clone()),
    );
    assert_eq!(
        restarted.level_meta.read().await.day_time,
        init_day_time + 55
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn concurrent_save_all_and_quit_write_stay_consistent() {
    let dir = std::env::temp_dir().join(format!(
        "radbyte_concurrent_save_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let cfg = test_server_config(ServerForwardingMode::Modern);
    let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
    let uuid = Uuid::new_v3(&Uuid::NAMESPACE_OID, b"concurrent_test");

    let s1 = Arc::clone(&state);
    let s2 = Arc::clone(&state);
    let t1 = tokio::spawn(async move {
        for _ in 0..8 {
            s1.save_all().await.unwrap();
        }
    });
    let t2 = tokio::spawn(async move {
        for i in 0..8 {
            let mut data = SavedPlayerData {
                x: f64::from(i),
                y: 64.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                hp: 20.0,
                food: 20,
                saturation: 5.0,
                game_mode: 0,
                inventory: Vec::new(),
                selected_hotbar: 0,
                dimension: crate::world::worldgen::DimensionId::Overworld,
                stats: PlayerStats::default(),
            };
            data.stats.blocks_placed = u64::from(i as u32);
            s2.save_player_data(uuid, &data).await.unwrap();
        }
    });
    let (r1, r2) = tokio::join!(t1, t2);
    r1.unwrap();
    r2.unwrap();
    let loaded = state.load_player_data(uuid).await.unwrap();
    assert_eq!(loaded.stats.blocks_placed, 7);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn e2e_world_time_sync_on_join_and_persists_across_rejoin_and_restart() {
    let dir = std::env::temp_dir().join(format!(
        "radserver_time_persist_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let cfg = test_server_config(ServerForwardingMode::Modern);
    let state = Arc::new(ServerState::new_with_world_dir(
        cfg.clone(),
        Some(dir.clone()),
    ));
    let (_game_time, day_time) = state.set_day_time(18_000).await;
    assert_eq!(day_time, 18_000);

    // 1. Join player while server is running -> must receive SetTime(18000)
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let st = Arc::clone(&state);
    let accept1 = tokio::spawn(async move {
        let (sock, peer) = listener.accept().await.unwrap();
        handle_connection(sock, peer, st).await;
    });

    let (s1, _b1, pkt1) = join_and_capture_set_time(addr, "TimePlayer").await;
    assert_eq!(pkt1.clock_updates.len(), 1);
    assert_eq!(pkt1.clock_updates[0].1.total_ticks.0, 18_000);

    drop(s1);
    let _ = accept1.await;

    // 2. Save & reload server from disk (simulates restart), then rejoin -> still 18000
    state.save_all().await.unwrap();
    let reloaded = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
    let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let st2 = Arc::clone(&reloaded);
    let accept2 = tokio::spawn(async move {
        let (sock, peer) = listener2.accept().await.unwrap();
        handle_connection(sock, peer, st2).await;
    });

    let (s2, _b2, pkt2) = join_and_capture_set_time(addr2, "TimePlayer").await;
    assert_eq!(pkt2.clock_updates[0].1.total_ticks.0, 18_000);

    drop(s2);
    let _ = accept2.await;
    let _ = std::fs::remove_dir_all(&dir);
}
