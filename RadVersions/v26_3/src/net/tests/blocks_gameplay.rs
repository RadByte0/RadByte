use super::*;

#[tokio::test]
async fn e2e_server_block_break_and_place() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.game_mode = 0; // Survival mode for block break/place
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

    let (mut p1, mut buf1) = connect_test_player(addr, "Builder1").await;
    let (mut p2, mut buf2) = connect_test_player(addr, "Builder2").await;

    // Place block on top of (2, 63, 2) -> (2, 64, 2) using slot 0 (stone = state 1)
    send_packet(
        &mut p1,
        &UseItemOn {
            hand: VarInt(0),
            location: BlockPos::new(2, 63, 2),
            face: VarInt(1),
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: VarInt(10),
        },
    )
    .await
    .unwrap();

    let mut saw_place_ack = false;
    for _ in 0..15 {
        let frame = read_raw_frame(&mut p1, &mut buf1, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == AcknowledgeBlockChange::ID {
            let ack = frame.decode_as::<AcknowledgeBlockChange>().unwrap();
            assert_eq!(ack.sequence.0, 10);
            saw_place_ack = true;
            break;
        }
    }
    assert!(saw_place_ack);

    let mut saw_place_update = false;
    for _ in 0..15 {
        let frame = read_raw_frame(&mut p2, &mut buf2, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == BlockUpdate::ID {
            let upd = frame.decode_as::<BlockUpdate>().unwrap();
            assert_eq!(upd.pos, BlockPos::new(2, 64, 2));
            assert_eq!(upd.block_state.0, 1);
            saw_place_update = true;
            break;
        }
    }
    assert!(saw_place_update);
    assert_eq!(state.world.write().await.get_block(2, 64, 2), 1);

    // A finish without a matching start must restore the client's predicted block.
    send_packet(
        &mut p1,
        &PlayerAction {
            status: VarInt(2),
            location: BlockPos::new(2, 64, 2),
            face: 1,
            sequence: VarInt(13),
        },
    )
    .await
    .unwrap();
    let mut restored = false;
    for _ in 0..20 {
        let frame = tokio::time::timeout(Duration::from_secs(2), read_raw_frame(&mut p1, &mut buf1, 1024 * 1024))
            .await.unwrap().unwrap();
        if frame.id == BlockUpdate::ID {
            let update = frame.decode_as::<BlockUpdate>().unwrap();
            if update.pos == BlockPos::new(2, 64, 2) && update.block_state.0 == 1 {
                restored = true;
                break;
            }
        }
    }
    assert!(restored, "rejected break must restore the block on the client");
    assert_eq!(state.world.write().await.get_block(2, 64, 2), 1);

    // Start digging (2, 64, 2), wait >=40ms for Survival mining timer, then finish digging -> air (0)
    send_packet(
        &mut p1,
        &PlayerAction {
            status: VarInt(0),
            location: BlockPos::new(2, 64, 2),
            face: 1,
            sequence: VarInt(11),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    send_packet(
        &mut p1,
        &PlayerAction {
            status: VarInt(2),
            location: BlockPos::new(2, 64, 2),
            face: 1,
            sequence: VarInt(12),
        },
    )
    .await
    .unwrap();

    let mut saw_break_update = false;
    for _ in 0..15 {
        let frame = read_raw_frame(&mut p2, &mut buf2, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == BlockUpdate::ID {
            let upd = frame.decode_as::<BlockUpdate>().unwrap();
            assert_eq!(upd.pos, BlockPos::new(2, 64, 2));
            assert_eq!(upd.block_state.0, 0);
            saw_break_update = true;
            break;
        }
    }
    assert!(saw_break_update);
    assert_eq!(state.world.write().await.get_block(2, 64, 2), 0);
}

#[tokio::test]
async fn e2e_server_survival_fall_damage_mob_combat_and_respawn() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.game_mode = 0; // Survival
    let state = Arc::new(ServerState::new(cfg));
    let zombie_id = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
    state
        .entities
        .write()
        .await
        .spawn_mob(zombie_id, MobKind::Zombie, 1.5, 64.0, 0.5);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let st = Arc::clone(&state);
    tokio::spawn(async move {
        while let Ok((stream, peer)) = listener.accept().await {
            let s = Arc::clone(&st);
            tokio::spawn(handle_connection(stream, peer, s));
        }
    });

    let (mut p1, mut buf1) = connect_test_player(addr, "Survivor").await;

    // 1. Fall 10 blocks within time-budgeted speed cap: airborne at Y=74 -> land at Y=64 (7.0 damage -> 13.0 HP)
    for &(y, flags) in &[(69.0, 0x00), (74.0, 0x00), (69.0, 0x00), (64.0, 0x01)] {
        tokio::time::sleep(Duration::from_millis(450)).await;
        send_packet(
            &mut p1,
            &MovePlayerPos {
                x: 0.5,
                y,
                z: 0.5,
                flags,
            },
        )
        .await
        .unwrap();
    }

    let mut hp_after_fall = None;
    for _ in 0..20 {
        let frame = timeout(
            Duration::from_secs(3),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .expect("timed out waiting for fall damage SetHealth")
        .unwrap();
        if frame.id == SetHealth::ID {
            hp_after_fall = Some(frame.decode_as::<SetHealth>().unwrap().health);
            break;
        }
    }
    assert_eq!(hp_after_fall, Some(13.0));

    // 2. Attack zombie (20 HP, 5 dmg/hit -> 4 hits kills it)
    for _ in 0..4 {
        send_packet(
            &mut p1,
            &Attack {
                entity_id: VarInt(zombie_id),
            },
        )
        .await
        .unwrap();
    }
    let mut saw_mob_removed = false;
    for _ in 0..20 {
        let frame = read_raw_frame(&mut p1, &mut buf1, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == RemoveEntities::ID {
            let rem = frame.decode_as::<RemoveEntities>().unwrap();
            if rem.entity_ids.contains(&VarInt(zombie_id)) {
                saw_mob_removed = true;
                break;
            }
        }
    }
    assert!(saw_mob_removed);

    // 3. Lethal fall within time-budgeted speed cap -> death (0.0 HP) -> ClientCommand(0) respawn -> 20.0 HP
    for &(y, flags) in &[
        (69.0, 0x00),
        (74.0, 0x00),
        (79.0, 0x00),
        (84.0, 0x00),
        (79.0, 0x00),
        (74.0, 0x00),
        (69.0, 0x00),
        (64.0, 0x01),
    ] {
        tokio::time::sleep(Duration::from_millis(450)).await;
        send_packet(
            &mut p1,
            &MovePlayerPos {
                x: 0.5,
                y,
                z: 0.5,
                flags,
            },
        )
        .await
        .unwrap();
    }

    let mut died_hp = None;
    for _ in 0..20 {
        let frame = read_raw_frame(&mut p1, &mut buf1, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == SetHealth::ID {
            died_hp = Some(frame.decode_as::<SetHealth>().unwrap().health);
            break;
        }
    }
    assert_eq!(died_hp, Some(0.0));

    send_packet(
        &mut p1,
        &ClientCommand {
            action_id: VarInt(0),
        },
    )
    .await
    .unwrap();

    let mut saw_respawn = false;
    let mut respawn_hp = None;
    for _ in 0..20 {
        let frame = read_raw_frame(&mut p1, &mut buf1, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == Respawn::ID {
            saw_respawn = true;
        } else if frame.id == SetHealth::ID {
            respawn_hp = Some(frame.decode_as::<SetHealth>().unwrap().health);
            if saw_respawn {
                break;
            }
        }
    }
    assert!(saw_respawn);
    assert_eq!(respawn_hp, Some(20.0));
}

#[tokio::test]
async fn tick_performance_under_150_entities() {
    let state = ServerState::new(test_server_config(ServerForwardingMode::Modern));
    {
        let mut entities = state.entities.write().await;
        for i in 0..150 {
            let kind = if i % 2 == 0 {
                MobKind::Zombie
            } else {
                MobKind::Pig
            };
            entities.spawn_mob(
                1000 + i,
                kind,
                f64::from(i % 16) + 0.5,
                64.0,
                f64::from(i / 16) + 0.5,
            );
        }
    }
    let start = Instant::now();
    for _ in 0..20 {
        step_world_tick(&state).await;
    }
    let avg_tick = start.elapsed() / 20;
    assert!(
        avg_tick < Duration::from_millis(20),
        "expected < 20 ms per tick for 150 entities, got {avg_tick:?}"
    );
}

#[tokio::test]
async fn survival_regeneration_and_starvation_tick_work() {
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.difficulty = 2; // Normal: starvation damages down to 1.0 HP
    let state = ServerState::new(cfg);
    let (tx1, mut rx1, _c1) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);
    let (tx2, mut rx2, _c2) = outbound_channel(16, MAX_PLAYER_OUTBOUND_BYTES);

    {
        let mut players = state.players.write().await;
        players.insert(
            1,
            PlayerSession {
                entity_id: 1,
                uuid: Uuid::from_u128(101),
                username: "WellFed".into(),
                properties: Vec::new(),
                x: 0.5,
                y: 64.0,
                z: 0.5,
                yaw: 0.0,
                pitch: 0.0,
                hp: 15.0,
                food: 20,
                saturation: 3.0,
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
                tx: tx1,
            },
        );
        players.insert(
            2,
            PlayerSession {
                entity_id: 2,
                uuid: Uuid::from_u128(102),
                username: "Starving".into(),
                properties: Vec::new(),
                x: 1.5,
                y: 64.0,
                z: 1.5,
                yaw: 0.0,
                pitch: 0.0,
                hp: 10.0,
                food: 0,
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
                tx: tx2,
            },
        );
    }

    step_player_survival_tick(&state, 80).await;

    {
        let players = state.players.read().await;
        let p1 = players.get(&1).unwrap();
        assert_eq!(p1.hp, 16.0);
        assert_eq!(p1.saturation, 1.5);

        let p2 = players.get(&2).unwrap();
        assert_eq!(p2.hp, 9.0);
    }

    assert!(rx1.recv().await.is_some());
    assert!(rx2.recv().await.is_some());
}

#[tokio::test]
async fn m4_mob_loot_and_food_consumption_restores_hunger() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.game_mode = 0;
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

    let (mut p1, mut buf1) = connect_test_player(addr, "Hunter").await;
    // Set Hunter's food to 10
    {
        let mut players = state.players.write().await;
        let hunter = players
            .values_mut()
            .find(|p| p.username == "Hunter")
            .unwrap();
        hunter.food = 10;
        hunter.saturation = 0.0;
    }

    // Spawn a Pig within reach of (0.5, 64.0, 0.5)
    let pig_id = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
    state.entities.write().await.spawn_mob(
        pig_id,
        crate::gameplay::entity::MobKind::Pig,
        1.5,
        64.0,
        1.5,
    );

    // Attack twice (10 HP / 5.0 damage = killed)
    for _ in 0..2 {
        send_packet(
            &mut p1,
            &Attack {
                entity_id: VarInt(pig_id),
            },
        )
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(80)).await;

    // Find slot index containing COOKED_PORKCHOP_ITEM_ID
    let pork_slot = {
        let players = state.players.read().await;
        let hunter = players.values().find(|p| p.username == "Hunter").unwrap();
        hunter
            .inventory
            .iter()
            .position(|s| {
                s.item_id == crate::gameplay::inventory::COOKED_PORKCHOP_ITEM_ID && s.count > 0
            })
            .expect("Killing Pig should drop cooked porkchop into inventory")
    };
    assert!(
        (36..45).contains(&pork_slot),
        "Porkchop should land in hotbar, got slot {pork_slot}"
    );
    let hotbar_idx = (pork_slot - 36) as i16;

    send_packet(&mut p1, &SetCarriedItem { slot: hotbar_idx })
        .await
        .unwrap();
    send_packet(
        &mut p1,
        &UseItem {
            hand: VarInt(0),
            sequence: VarInt(7),
            yaw: 0.0,
            pitch: 0.0,
        },
    )
    .await
    .unwrap();

    let mut saw_fed = false;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_secs(2),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == SetHealth::ID
        {
            let sh = frame.decode_as::<SetHealth>().unwrap();
            if sh.food.0 == 18 {
                saw_fed = true;
                break;
            }
        }
    }
    assert!(
        saw_fed,
        "UseItem with cooked porkchop should restore food from 10 to 18"
    );
}

#[tokio::test]
async fn e2e_dimension_respawn_isolation_and_reconnect() {
    use rad_core::packet::play::{ChatCommand, PlayLogin, RemoveEntities, Respawn};

    let dir = std::env::temp_dir().join(format!(
        "radbyte_g03_dim_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.admins.insert("dim_alice".into());
    let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_state = Arc::clone(&state);
    let accept_task = tokio::spawn(async move {
        for _ in 0..3 {
            if let Ok((stream, peer)) = listener.accept().await {
                let s = Arc::clone(&accept_state);
                tokio::spawn(async move {
                    handle_connection(stream, peer, s).await;
                });
            }
        }
    });

    let (mut p1, mut buf1) = connect_test_player(addr, "dim_alice").await;
    let (mut p2, mut buf2) = connect_test_player(addr, "dim_bob").await;

    // Drain initial packets for both players
    let _ = timeout(
        Duration::from_millis(120),
        read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
    )
    .await;
    let _ = timeout(
        Duration::from_millis(120),
        read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
    )
    .await;

    // Alice switches to Nether
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "dimension nether".into(),
        },
    )
    .await
    .unwrap();

    let mut p1_saw_respawn = false;
    for _ in 0..48 {
        if let Some(frame) = timeout(
            Duration::from_millis(400),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == Respawn::ID
        {
            let r = frame.decode_as::<Respawn>().unwrap();
            assert_eq!(r.dimension_name, "minecraft:the_nether");
            assert_eq!(r.dimension_type.0, 1);
            p1_saw_respawn = true;
            break;
        }
    }
    assert!(
        p1_saw_respawn,
        "Alice should receive Respawn(minecraft:the_nether)"
    );

    // Bob in Overworld should see RemoveEntities for Alice, and NOT see Alice's Nether block updates
    let mut p2_saw_remove = false;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_millis(250),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == RemoveEntities::ID
        {
            p2_saw_remove = true;
            break;
        }
    }
    assert!(
        p2_saw_remove,
        "Bob in Overworld should receive RemoveEntities when Alice switches to Nether"
    );

    // Mutate Nether block vs Overworld block at same (2, 64, 2)
    {
        let mut world = state.world.write().await;
        world.set_block_in(crate::world::worldgen::DimensionId::Nether, 2, 64, 2, 14);
        assert_ne!(
            world.get_block_in(crate::world::worldgen::DimensionId::Overworld, 2, 64, 2),
            14,
            "Overworld and Nether at same (x,y,z) must be isolated"
        );
    }

    // Broadcast a block update in Nether; Bob in Overworld must NOT receive it
    state
        .broadcast_in_dimension_except(
            crate::world::worldgen::DimensionId::Nether,
            &BlockUpdate {
                pos: BlockPos::new(2, 64, 2),
                block_state: VarInt(14),
            },
            None,
        )
        .await;
    let mut p2_saw_nether_block = false;
    for _ in 0..4 {
        if let Some(frame) = timeout(
            Duration::from_millis(120),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == BlockUpdate::ID
        {
            p2_saw_nether_block = true;
        }
    }
    assert!(
        !p2_saw_nether_block,
        "Overworld player must not receive Nether BlockUpdate"
    );

    // Save & disconnect Alice, then reconnect and verify PlayLogin restores Nether
    state.save_all().await.unwrap();
    drop(p1);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(
        dir.join("DIM-1").join("region").exists(),
        "DIM-1/region should be persisted"
    );

    // Reconnect Alice using connect_test_player and verify PlayLogin restores Nether
    let mut p1_re = TcpStream::connect(addr).await.unwrap();
    let mut rbuf = BytesMut::with_capacity(65536);
    let alice_uuid = offline_player_uuid("dim_alice");
    send_packet(
        &mut p1_re,
        &rad_core::packet::handshake::Handshake {
            protocol_version: VarInt(777),
            server_address: "127.0.0.1".into(),
            server_port: addr.port(),
            next_state: rad_core::state::ConnectionState::Login,
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut p1_re,
        &rad_core::packet::login::LoginStart {
            username: "dim_alice".into(),
            player_uuid: alice_uuid,
        },
    )
    .await
    .unwrap();
    let req: rad_core::packet::login::LoginPluginRequest =
        read_typed_packet(&mut p1_re, &mut rbuf, 1024 * 1024)
            .await
            .unwrap();
    let payload = encode_velocity_forwarding(
        &ForwardingData {
            address: "127.0.0.1".into(),
            uuid: alice_uuid,
            username: "dim_alice".into(),
            properties: Vec::new(),
        },
        b"server-test-secret",
    )
    .unwrap();
    send_packet(
        &mut p1_re,
        &rad_core::packet::login::LoginPluginResponse {
            message_id: req.message_id,
            successful: true,
            data: payload,
        },
    )
    .await
    .unwrap();
    let _login_ok = read_typed_packet::<rad_core::packet::login::LoginSuccess>(
        &mut p1_re,
        &mut rbuf,
        1024 * 1024,
    )
    .await
    .unwrap();
    send_packet(&mut p1_re, &rad_core::packet::login::LoginAcknowledged)
        .await
        .unwrap();
    let mut saw_nether_login = false;
    for _ in 0..48 {
        let frame = read_raw_frame(&mut p1_re, &mut rbuf, 1024 * 1024)
            .await
            .unwrap();
        if frame.id == rad_core::packet::configuration::ClientboundKnownPacks::ID {
            send_packet(
                &mut p1_re,
                &rad_core::packet::configuration::ServerboundKnownPacks { packs: vec![] },
            )
            .await
            .unwrap();
        } else if frame.id == rad_core::packet::configuration::FinishConfiguration::ID {
            send_packet(
                &mut p1_re,
                &rad_core::packet::configuration::AcknowledgeFinishConfiguration,
            )
            .await
            .unwrap();
        } else if frame.id == PlayLogin::ID {
            let pl = frame.decode_as::<PlayLogin>().unwrap();
            assert_eq!(pl.dimension_name, "minecraft:the_nether");
            assert_eq!(pl.dimension_type.0, 1);
            saw_nether_login = true;
            break;
        }
    }
    assert!(
        saw_nether_login,
        "reconnecting Alice should spawn in minecraft:the_nether"
    );

    drop(p1_re);
    drop(p2);
    accept_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn e2e_dimension_isolation_blocks_containers_furnaces_physics_and_entities() {
    use rad_core::packet::play::{ChatCommand, MoveEntityPosRot, Respawn, SpawnEntity};

    let dir = std::env::temp_dir().join(format!(
        "radbyte_dim_iso_full_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.admins.insert("iso_alice".into());
    cfg.admins.insert("iso_bob".into());
    let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));

    // 1. Place identical chest & furnace coordinates in Overworld vs Nether
    {
        let mut containers = state.containers.write().await;
        containers.ensure_chest_in(crate::world::worldgen::DimensionId::Overworld, 4, 64, 4);
        containers
            .get_chest_mut_in(crate::world::worldgen::DimensionId::Overworld, 4, 64, 4)
            .unwrap()[0] = rad_core::packet::play::SimpleSlot {
            count: 5,
            item_id: rad_core::mc777_data::items::COBBLESTONE,
        };
        containers.ensure_chest_in(crate::world::worldgen::DimensionId::Nether, 4, 64, 4);
        containers
            .get_chest_mut_in(crate::world::worldgen::DimensionId::Nether, 4, 64, 4)
            .unwrap()[0] = rad_core::packet::play::SimpleSlot {
            count: 9,
            item_id: rad_core::mc777_data::items::DIAMOND,
        };
        containers.mark_dirty();

        let mut furnaces = state.furnaces.write().await;
        furnaces
            .ensure_get_mut_in(crate::world::worldgen::DimensionId::Overworld, 5, 64, 5)
            .input_item = rad_core::mc777_data::items::COBBLESTONE;
        furnaces
            .ensure_get_mut_in(crate::world::worldgen::DimensionId::Nether, 5, 64, 5)
            .input_item = rad_core::mc777_data::items::IRON_ORE;
    }

    // 2. Place a solid wall at (10, 64, 10) in Overworld only; verify collision is dimension-scoped
    {
        let mut world = state.world.write().await;
        assert!(world.set_block_in(
            crate::world::worldgen::DimensionId::Overworld,
            10,
            64,
            10,
            rad_core::mc777_data::blocks::STONE,
        ));
        assert!(world.set_block_in(crate::world::worldgen::DimensionId::Nether, 10, 64, 10, 0,));
        assert_eq!(
            crate::world::physics::validate_player_move_with_budget_in(
                &mut world,
                crate::world::worldgen::DimensionId::Overworld,
                (9.2, 64.0, 10.5),
                (10.5, 64.0, 10.5),
                0.2,
                false,
            ),
            crate::world::physics::MoveValidation::CollidedWithBlock
        );
        assert_eq!(
            crate::world::physics::validate_player_move_with_budget_in(
                &mut world,
                crate::world::worldgen::DimensionId::Nether,
                (9.2, 64.0, 10.5),
                (10.5, 64.0, 10.5),
                0.2,
                false,
            ),
            crate::world::physics::MoveValidation::Accepted
        );
    }

    // 3. Persist and reload containers & furnaces to confirm dimension keys survive disk roundtrip
    assert!(state.save_all().await.is_ok());
    let reloaded_c = crate::world::containers::ContainerStore::load_from_dir(&dir);
    let mut reloaded_c = reloaded_c;
    assert_eq!(
        reloaded_c
            .get_chest_mut_in(crate::world::worldgen::DimensionId::Overworld, 4, 64, 4)
            .unwrap()[0]
            .item_id,
        rad_core::mc777_data::items::COBBLESTONE
    );
    assert_eq!(
        reloaded_c
            .get_chest_mut_in(crate::world::worldgen::DimensionId::Nether, 4, 64, 4)
            .unwrap()[0]
            .item_id,
        rad_core::mc777_data::items::DIAMOND
    );
    let reloaded_f = crate::world::sim::FurnaceStore::load_from_dir(&dir);
    assert_eq!(
        reloaded_f
            .get_in(crate::world::worldgen::DimensionId::Overworld, 5, 64, 5)
            .unwrap()
            .input_item,
        rad_core::mc777_data::items::COBBLESTONE
    );
    assert_eq!(
        reloaded_f
            .get_in(crate::world::worldgen::DimensionId::Nether, 5, 64, 5)
            .unwrap()
            .input_item,
        rad_core::mc777_data::items::IRON_ORE
    );

    // 4. Spawn a hostile mob in The Nether; verify Overworld player doesn't see its movement or take damage,
    //    and switching to Nether spawns player with mc777_data::entities::PLAYER (159), not 155.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_state = Arc::clone(&state);
    let accept_task = tokio::spawn(async move {
        for _ in 0..2 {
            if let Ok((stream, peer)) = listener.accept().await {
                let s = Arc::clone(&accept_state);
                tokio::spawn(async move {
                    handle_connection(stream, peer, s).await;
                });
            }
        }
    });

    let (mut p1, mut buf1) = connect_test_player(addr, "iso_alice").await;
    let (mut p2, mut buf2) = connect_test_player(addr, "iso_bob").await;

    // Drain join packets
    for _ in 0..8 {
        let _ = timeout(
            Duration::from_millis(60),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await;
        let _ = timeout(
            Duration::from_millis(60),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await;
    }

    // Spawn zombie in The Nether near spawn
    let mob_eid = 9101;
    state.entities.write().await.spawn_mob_in(
        crate::world::worldgen::DimensionId::Nether,
        mob_eid,
        crate::gameplay::entity::MobKind::Zombie,
        1.5,
        64.0,
        0.5,
    );
    for _ in 0..5 {
        step_world_tick(&state).await;
    }

    // Overworld Bob must NOT receive MoveEntityPosRot for the Nether zombie
    while let Some(frame) = timeout(
        Duration::from_millis(80),
        read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
    )
    .await
    .ok()
    .flatten()
    {
        if frame.id == MoveEntityPosRot::ID {
            let m = frame.decode_as::<MoveEntityPosRot>().unwrap();
            assert_ne!(
                m.entity_id.0, mob_eid,
                "Overworld player must not receive Nether mob updates"
            );
        }
    }

    // Alice switches to Nether, then Bob switches to Nether and must see SpawnEntity with entities::PLAYER (159)
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "dimension nether".into(),
        },
    )
    .await
    .unwrap();
    let mut alice_respawned = false;
    for _ in 0..48 {
        if let Some(frame) = timeout(
            Duration::from_millis(300),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == Respawn::ID
        {
            alice_respawned = true;
            break;
        }
    }
    assert!(alice_respawned, "Alice should receive Respawn into Nether");

    send_packet(
        &mut p2,
        &ChatCommand {
            command: "dimension nether".into(),
        },
    )
    .await
    .unwrap();
    let mut saw_player_spawn_159 = false;
    for _ in 0..48 {
        if let Some(frame) = timeout(
            Duration::from_millis(300),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == SpawnEntity::ID
        {
            let sp = frame.decode_as::<SpawnEntity>().unwrap();
            if sp.entity_type.0 == rad_core::mc777_data::entities::PLAYER {
                saw_player_spawn_159 = true;
                break;
            }
        }
    }
    assert!(
        saw_player_spawn_159,
        "Dimension transfer must spawn remote players using mc777_data::entities::PLAYER (159)"
    );

    drop(p1);
    drop(p2);
    accept_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn e2e_authoritative_bow_projectile_hits_mob_damages_and_despawns() {
    use rad_core::packet::play::{RemoveEntities, SetCarriedItem, SpawnEntity, UseItem};

    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.game_mode = 0;
    let state = Arc::new(ServerState::new(cfg));

    // Spawn a zombie directly along +Z from spawn (0, 64, 5)
    let zombie_eid = 8801;
    state.entities.write().await.spawn_mob_in(
        crate::world::worldgen::DimensionId::Overworld,
        zombie_eid,
        crate::gameplay::entity::MobKind::Zombie,
        0.0,
        64.0,
        5.0,
    );

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_state = Arc::clone(&state);
    let accept_task = tokio::spawn(async move {
        if let Ok((stream, peer)) = listener.accept().await {
            handle_connection(stream, peer, accept_state).await;
        }
    });

    let (mut p1, mut buf1) = connect_test_player(addr, "archer_test").await;
    for _ in 0..8 {
        let _ = timeout(
            Duration::from_millis(60),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await;
    }

    // Give bow + 1 arrow via playerdata-free direct inventory: select hotbar slot 0 with bow?
    // First test firing with empty bow (no arrow in survival) -> no projectile spawned.
    send_packet(&mut p1, &SetCarriedItem { slot: 0 })
        .await
        .unwrap();
    send_packet(
        &mut p1,
        &UseItem {
            hand: VarInt(0),
            sequence: VarInt(1),
            yaw: 0.0,
            pitch: 0.0,
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(state.entities.read().await.projectile_count(), 0);

    // Preload a second player with bow + 1 arrow via SavedPlayerData and verify full bow -> arrow -> mob damage -> despawn
    drop(p1);
    accept_task.abort();

    let dir = std::env::temp_dir().join(format!(
        "radbyte_bow_e2e_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg2 = test_server_config(ServerForwardingMode::Modern);
    cfg2.game_mode = 0;
    let state2 = Arc::new(ServerState::new_with_world_dir(cfg2, Some(dir.clone())));
    let mut inv_slots = vec![
        SavedSlot {
            count: 0,
            item_id: 0,
            ..Default::default()
        };
        46
    ];
    inv_slots[36] = SavedSlot {
        count: 1,
        item_id: rad_core::mc777_data::items::BOW,
        ..Default::default()
    };
    inv_slots[37] = SavedSlot {
        count: 1,
        item_id: rad_core::mc777_data::items::ARROW,
        ..Default::default()
    };
    let archer_uuid = offline_player_uuid("archer_bow");
    state2
        .save_player_data(
            archer_uuid,
            &SavedPlayerData {
                x: 0.0,
                y: 64.0,
                z: 0.0,
                yaw: 0.0,
                pitch: 0.0,
                hp: 20.0,
                food: 20,
                saturation: 5.0,
                game_mode: 0,
                inventory: inv_slots,
                selected_hotbar: 0,
                dimension: crate::world::worldgen::DimensionId::Overworld,
                stats: PlayerStats::default(),
            },
        )
        .await
        .unwrap();

    state2.entities.write().await.spawn_mob_in(
        crate::world::worldgen::DimensionId::Overworld,
        zombie_eid,
        crate::gameplay::entity::MobKind::Zombie,
        0.0,
        64.0,
        4.0,
    );

    let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let accept_state2 = Arc::clone(&state2);
    let accept_task2 = tokio::spawn(async move {
        if let Ok((stream, peer)) = listener2.accept().await {
            handle_connection(stream, peer, accept_state2).await;
        }
    });
    let (mut p2, mut buf2) = connect_test_player(addr2, "archer_bow").await;
    for _ in 0..8 {
        let _ = timeout(
            Duration::from_millis(60),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await;
    }

    // Fire bow along +Z (yaw=0.0, pitch=0.0)
    send_packet(
        &mut p2,
        &UseItem {
            hand: VarInt(0),
            sequence: VarInt(2),
            yaw: 0.0,
            pitch: 0.0,
        },
    )
    .await
    .unwrap();

    let mut proj_eid = None;
    for _ in 0..24 {
        if let Some(frame) = timeout(
            Duration::from_millis(200),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == SpawnEntity::ID
        {
            let sp = frame.decode_as::<SpawnEntity>().unwrap();
            if sp.entity_type.0 == rad_core::mc777_data::entities::ARROW {
                proj_eid = Some(sp.entity_id.0);
                break;
            }
        }
    }
    let proj_eid = proj_eid.expect("bow UseItem should spawn tracked arrow");
    assert_eq!(state2.entities.read().await.projectile_count(), 1);

    let hp_before = state2.entities.read().await.get(zombie_eid).unwrap().hp;
    for _ in 0..10 {
        step_world_tick(&state2).await;
    }
    let hp_after = state2.entities.read().await.get(zombie_eid).unwrap().hp;
    assert!(
        hp_after < hp_before,
        "authoritative arrow trajectory must hit and damage zombie"
    );
    assert_eq!(
        state2.entities.read().await.projectile_count(),
        0,
        "arrow must despawn after hitting zombie"
    );

    let mut saw_remove = false;
    while let Some(frame) = timeout(
        Duration::from_millis(120),
        read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
    )
    .await
    .ok()
    .flatten()
    {
        if frame.id == RemoveEntities::ID {
            let rem = frame.decode_as::<RemoveEntities>().unwrap();
            if rem.entity_ids.iter().any(|id| id.0 == proj_eid) {
                saw_remove = true;
                break;
            }
        }
    }
    assert!(
        saw_remove,
        "clients must receive RemoveEntities for impacted arrow"
    );

    drop(p2);
    accept_task2.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn hardcore_login_sets_flag_and_respawns_in_spectator() {
    let mut cfg = test_server_config(ServerForwardingMode::None);
    cfg.hardcore = true;
    cfg.difficulty = 3;
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
            username: "HardcoreOne".into(),
            player_uuid: Uuid::from_u128(301),
        },
    )
    .await
    .unwrap();

    let mut buf = BytesMut::with_capacity(8192);
    let _success: LoginSuccess = read_typed_packet(&mut client, &mut buf, 1024 * 1024)
        .await
        .unwrap();
    send_packet(&mut client, &LoginAcknowledged).await.unwrap();

    let mut play_login = None;
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
    assert!(play_login.is_hardcore, "PlayLogin.is_hardcore must be true");

    // Mark dead, then request respawn
    tokio::time::sleep(Duration::from_millis(30)).await;
    if let Some(p) = state.players.write().await.get_mut(&play_login.entity_id) {
        p.dead = true;
        p.hp = 0.0;
    }
    send_packet(
        &mut client,
        &ClientCommand {
            action_id: VarInt(0),
        },
    )
    .await
    .unwrap();

    let respawn = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let frame = read_raw_frame(&mut client, &mut buf, 1024 * 1024)
                .await
                .unwrap();
            if frame.id == Respawn::ID {
                return frame.decode_as::<Respawn>().unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        respawn.game_mode, 3,
        "hardcore respawn must switch to Spectator (3)"
    );

    drop(client);
    let _ = accept.await;
}
