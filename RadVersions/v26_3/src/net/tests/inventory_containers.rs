use super::*;

#[tokio::test]
async fn creative_inventory_updates_server_and_creative_drop_spawns_item() {
    let state = Arc::new(ServerState::new(test_server_config(
        ServerForwardingMode::Modern,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server_state = Arc::clone(&state);
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        handle_connection(stream, peer, server_state).await;
    });
    let (mut stream, _) = connect_test_player(addr, "CreativeTester").await;
    let player_id = *state.players.read().await.keys().next().unwrap();
    state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .game_mode = 1;
    let item = rad_core::packet::play::ItemStack {
        count: 5,
        item_id: rad_core::mc777_data::items::by_name("minecraft:diamond_block").unwrap(),
        components: Default::default(),
    };
    send_packet(
        &mut stream,
        &SetCreativeModeSlot {
            slot: 36,
            item: item.clone(),
        },
    )
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state
                .players
                .read()
                .await
                .get(&player_id)
                .is_some_and(|p| p.inventory[36].item_id == item.item_id)
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("creative slot update was not applied");
    assert!(crate::gameplay::inventory::item_id_to_block_state(item.item_id).is_some());
    send_packet(&mut stream, &SetCreativeModeSlot { slot: -1, item })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if state.entities.read().await.dropped_item_count() > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("creative drop was not spawned");
    state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .game_mode = 0;
    let forged = rad_core::packet::play::ItemStack {
        count: 64,
        item_id: rad_core::mc777_data::items::DIRT,
        components: Default::default(),
    };
    send_packet(
        &mut stream,
        &SetCreativeModeSlot {
            slot: 36,
            item: forged,
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_ne!(
        state.players.read().await[&player_id].inventory[36].item_id,
        rad_core::mc777_data::items::DIRT
    );
    drop(stream);
    task.abort();
}

#[tokio::test]
async fn pick_block_packet_updates_live_creative_inventory() {
    let state = Arc::new(ServerState::new(test_server_config(
        ServerForwardingMode::Modern,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Arc::clone(&state);
    let task = tokio::spawn(async move {
        let (stream, peer) = listener.accept().await.unwrap();
        handle_connection(stream, peer, server).await;
    });
    let (mut client, _) = connect_test_player(addr, "PickTester").await;
    let player_id = *state.players.read().await.keys().next().unwrap();
    state
        .players
        .write()
        .await
        .get_mut(&player_id)
        .unwrap()
        .game_mode = 1;
    let state_id = rad_core::mc777_data::blocks::by_name("minecraft:diamond_block")
        .unwrap()
        .default_state;
    state.world.write().await.set_block(0, 64, 2, state_id);
    assert_eq!(rad_core::packet::play::PickItemFromBlock::ID, 0x24);
    send_packet(
        &mut client,
        &rad_core::packet::play::PickItemFromBlock {
            location: BlockPos::new(0, 64, 2),
            include_data: false,
        },
    )
    .await
    .unwrap();
    let item_id = rad_core::mc777_data::items::by_name("minecraft:diamond_block").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let inv = state
                .live_inventories
                .read()
                .await
                .get(&player_id)
                .cloned()
                .unwrap();
            if inv.lock().unwrap().held_item().item_id == item_id {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("pick block must update the live selected item");
    drop(client);
    task.abort();
}

#[tokio::test]
async fn e2e_furnace_ui_smelts_and_persists_across_reload() {
    use rad_core::packet::play::{
        ContainerClick, ContainerClose, ContainerSetContent, ContainerSetData, OpenScreen,
        UseItemOn,
    };

    let dir = std::env::temp_dir().join(format!(
        "radbyte_g04_furnace_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.admins.insert("smelt_alice".into());
    let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
    {
        let mut world = state.world.write().await;
        world.set_block(2, 64, 2, crate::gameplay::inventory::FURNACE_BLOCK_STATE);
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let accept_state = Arc::clone(&state);
    let accept_task = tokio::spawn(async move {
        if let Ok((stream, peer)) = listener.accept().await {
            handle_connection(stream, peer, accept_state).await;
        }
    });

    let (mut p1, mut buf1) = connect_test_player(addr, "smelt_alice").await;
    let _ = timeout(
        Duration::from_millis(120),
        read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
    )
    .await;

    // Right-click the placed furnace at (2, 64, 2)
    send_packet(
        &mut p1,
        &UseItemOn {
            hand: VarInt(0),
            location: BlockPos::new(2, 64, 2),
            face: VarInt(1),
            cursor_x: 0.5,
            cursor_y: 0.5,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: VarInt(1),
        },
    )
    .await
    .unwrap();

    let mut saw_open = false;
    let mut saw_content = false;
    let mut saw_data = false;
    let mut state_id = 0i32;
    for _ in 0..32 {
        if let Some(frame) = timeout(
            Duration::from_millis(300),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
        {
            if frame.id == OpenScreen::ID {
                let o = frame.decode_as::<OpenScreen>().unwrap();
                assert_eq!(o.window_id.0, crate::gameplay::inventory::FURNACE_WINDOW_ID);
                assert_eq!(o.window_type.0, rad_core::mc777_data::menus::FURNACE);
                saw_open = true;
            } else if frame.id == ContainerSetContent::ID {
                let c = frame.decode_as::<ContainerSetContent>().unwrap();
                if c.window_id.0 == crate::gameplay::inventory::FURNACE_WINDOW_ID {
                    assert_eq!(c.slots.len(), 39);
                    state_id = c.state_id.0;
                    saw_content = true;
                }
            } else if frame.id == ContainerSetData::ID {
                let d = frame.decode_as::<ContainerSetData>().unwrap();
                if d.window_id.0 == crate::gameplay::inventory::FURNACE_WINDOW_ID {
                    saw_data = true;
                }
            }
            if saw_open && saw_content && saw_data {
                break;
            }
        }
    }
    assert!(
        saw_open && saw_content && saw_data,
        "Furnace UI should send OpenScreen, ContainerSetContent, and ContainerSetData"
    );

    // Put cobblestone + planks into furnace via FurnaceStore and verify ContainerClick output take
    {
        let mut furnaces = state.furnaces.write().await;
        {
            let f = furnaces.get_mut(2, 64, 2).unwrap();
            f.input_item = rad_core::mc777_data::items::COBBLESTONE;
            f.input_count = 1;
            f.fuel_item = rad_core::mc777_data::items::OAK_PLANKS;
            f.fuel_count = 1;
        }
        for _ in 0..205 {
            furnaces.tick_all();
        }
        let f = furnaces.get(2, 64, 2).unwrap();
        assert_eq!(f.output_item, rad_core::mc777_data::items::STONE);
        assert_eq!(f.output_count, 1);
    }

    // Shift-click furnace output slot (slot 2) to take smelted stone
    send_packet(
        &mut p1,
        &ContainerClick {
            window_id: VarInt(crate::gameplay::inventory::FURNACE_WINDOW_ID),
            state_id: VarInt(state_id),
            slot: 2,
            button: 0,
            mode: VarInt(1),
            changed_slots: Vec::new(),
            carried_item: rad_core::packet::play::SimpleSlot::default(),
        },
    )
    .await
    .unwrap();

    let mut took_stone = false;
    for _ in 0..24 {
        if let Some(frame) = timeout(
            Duration::from_millis(300),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == ContainerSetContent::ID
        {
            let c = frame.decode_as::<ContainerSetContent>().unwrap();
            if c.window_id.0 == 0
                && c.slots
                    .iter()
                    .any(|s| s.item_id == rad_core::mc777_data::items::STONE && s.count == 1)
            {
                took_stone = true;
                break;
            }
        }
    }
    assert!(
        took_stone,
        "Player inventory should receive smelted stone from furnace output click"
    );

    send_packet(
        &mut p1,
        &ContainerClose {
            window_id: VarInt(crate::gameplay::inventory::FURNACE_WINDOW_ID),
        },
    )
    .await
    .unwrap();

    drop(p1);
    accept_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn e2e_block4_components_runtime_recipe_reload_and_two_viewer_chest_sync() {
    use bytes::BytesMut;
    use rad_core::packet::play::{
        ChatCommand, ContainerClick, ContainerClose, ContainerSetContent, ItemComponentPatch,
        ItemStack, OpenScreen, SimpleSlot, UseItemOn,
    };

    // 1. ItemStack with data components roundtrips wire + persistence
    let stack = ItemStack {
        count: 1,
        item_id: rad_core::mc777_data::items::WOODEN_PICKAXE,
        components: ItemComponentPatch {
            custom_name: Some("Starter Pick".into()),
            damage: Some(12),
            max_damage: Some(59),
            repair_cost: Some(1),
            unbreakable: false,
            removed: vec![7],
        },
    };
    let mut wire = BytesMut::new();
    stack.encode_slot(&mut wire).unwrap();
    let mut slice = wire.freeze();
    let decoded_stack = ItemStack::decode_slot(&mut slice).unwrap();
    assert_eq!(decoded_stack, stack);

    // 2. Runtime custom recipe loaded on startup + /reload + real 3x3 crafting + two-viewer chest sync
    let dir = std::env::temp_dir().join(format!(
        "radbyte_block4_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let recipes_dir = dir.join("datapacks").join("recipes");
    std::fs::create_dir_all(&recipes_dir).unwrap();
    let custom_recipe = crate::gameplay::inventory::DataPackRecipe {
        id: "radbyte:cobble_to_iron".into(),
        ingredients: [
            rad_core::mc777_data::items::COBBLESTONE,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ],
        result_item_id: rad_core::mc777_data::items::IRON_INGOT,
        result_count: 2,
    };
    std::fs::write(
        recipes_dir.join("cobble_to_iron.json"),
        serde_json::to_string_pretty(&custom_recipe).unwrap(),
    )
    .unwrap();

    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.admins.insert("viewer_a".into());
    let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
    assert_eq!(state.recipes.read().await.len(), 1);

    // Place a chest at (2, 64, 2) and crafting table at (3, 64, 2)
    {
        let mut world = state.world.write().await;
        world.set_block_in(
            crate::world::worldgen::DimensionId::Overworld,
            2,
            64,
            2,
            crate::gameplay::inventory::CHEST_BLOCK_STATE,
        );
        world.set_block_in(
            crate::world::worldgen::DimensionId::Overworld,
            3,
            64,
            2,
            crate::gameplay::inventory::CRAFTING_TABLE_BLOCK_STATE,
        );
    }

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

    let (mut p1, mut buf1) = connect_test_player(addr, "viewer_a").await;
    let (mut p2, mut buf2) = connect_test_player(addr, "viewer_b").await;
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

    // Both players open the same chest at (2, 64, 2)
    let open_chest_pkt = UseItemOn {
        hand: VarInt(0),
        location: BlockPos::new(2, 64, 2),
        face: VarInt(1),
        cursor_x: 0.5,
        cursor_y: 1.0,
        cursor_z: 0.5,
        inside_block: false,
        world_border_hit: false,
        sequence: VarInt(1),
    };
    send_packet(&mut p1, &open_chest_pkt).await.unwrap();
    send_packet(&mut p2, &open_chest_pkt).await.unwrap();

    let mut p1_chest_sid = 0;
    for _ in 0..20 {
        if let Some(frame) = timeout(
            Duration::from_millis(200),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == ContainerSetContent::ID
        {
            let c = frame.decode_as::<ContainerSetContent>().unwrap();
            if c.window_id.0 == 1 {
                p1_chest_sid = c.state_id.0;
                break;
            }
        }
    }
    assert!(p1_chest_sid > 0, "viewer_a should open chest window 1");
    // Wait until viewer_b also finishes opening chest window 1
    let mut p2_opened = false;
    for _ in 0..20 {
        if let Some(frame) = timeout(
            Duration::from_millis(200),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == ContainerSetContent::ID
        {
            let c = frame.decode_as::<ContainerSetContent>().unwrap();
            if c.window_id.0 == 1 {
                p2_opened = true;
                break;
            }
        }
    }
    assert!(p2_opened, "viewer_b should open chest window 1");

    // viewer_a shift-clicks cobblestone (hotbar slot 3 = inventory slot 39 -> chest window slot 27 + (39 - 9) = 57)
    send_packet(
        &mut p1,
        &ContainerClick {
            window_id: VarInt(1),
            state_id: VarInt(p1_chest_sid),
            slot: 57,
            button: 0,
            mode: VarInt(1),
            changed_slots: Vec::new(),
            carried_item: SimpleSlot::default(),
        },
    )
    .await
    .unwrap();

    // viewer_b MUST receive updated ContainerSetContent for window 1 with cobblestone in slot 0
    let mut p2_saw_update = false;
    for _ in 0..24 {
        if let Some(frame) = timeout(
            Duration::from_millis(250),
            read_raw_frame(&mut p2, &mut buf2, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == ContainerSetContent::ID
        {
            let c = frame.decode_as::<ContainerSetContent>().unwrap();
            if c.window_id.0 == 1
                && c.slots[0].item_id == rad_core::mc777_data::items::COBBLESTONE
                && c.slots[0].count == 64
            {
                p2_saw_update = true;
                break;
            }
        }
    }
    assert!(
        p2_saw_update,
        "second chest viewer must immediately receive ContainerSetContent updates"
    );

    send_packet(
        &mut p1,
        &ContainerClose {
            window_id: VarInt(1),
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut p2,
        &ContainerClose {
            window_id: VarInt(1),
        },
    )
    .await
    .unwrap();

    // Test /reload reloads custom recipes and crafting table uses them
    let recipe2 = crate::gameplay::inventory::DataPackRecipe {
        id: "radbyte:stone_to_diamond".into(),
        ingredients: [rad_core::mc777_data::items::STONE, 0, 0, 0, 0, 0, 0, 0, 0],
        result_item_id: rad_core::mc777_data::items::DIAMOND,
        result_count: 1,
    };
    std::fs::write(
        recipes_dir.join("stone_to_diamond.json"),
        serde_json::to_string_pretty(&recipe2).unwrap(),
    )
    .unwrap();
    send_packet(
        &mut p1,
        &ChatCommand {
            command: "reload".into(),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(state.recipes.read().await.len(), 2);

    // viewer_a opens Crafting Table at (3, 64, 2) and crafts stone -> diamond via reloaded recipe
    send_packet(
        &mut p1,
        &UseItemOn {
            hand: VarInt(0),
            location: BlockPos::new(3, 64, 2),
            face: VarInt(1),
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: VarInt(2),
        },
    )
    .await
    .unwrap();

    let mut craft_sid = 0;
    let mut saw_craft_screen = false;
    for _ in 0..24 {
        if let Some(frame) = timeout(
            Duration::from_millis(200),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
        {
            if frame.id == OpenScreen::ID {
                let s = frame.decode_as::<OpenScreen>().unwrap();
                if s.window_id.0 == crate::gameplay::inventory::CRAFTING_WINDOW_ID {
                    saw_craft_screen = true;
                }
            } else if frame.id == ContainerSetContent::ID {
                let c = frame.decode_as::<ContainerSetContent>().unwrap();
                if c.window_id.0 == crate::gameplay::inventory::CRAFTING_WINDOW_ID {
                    craft_sid = c.state_id.0;
                    if saw_craft_screen {
                        break;
                    }
                }
            }
        }
    }
    assert!(saw_craft_screen && craft_sid > 0);

    // Shift-click hotbar slot 0 (stone, window slot 10+27 = 37) into 3x3 grid slot 1 -> diamond result!
    send_packet(
        &mut p1,
        &ContainerClick {
            window_id: VarInt(crate::gameplay::inventory::CRAFTING_WINDOW_ID),
            state_id: VarInt(craft_sid),
            slot: 37,
            button: 0,
            mode: VarInt(1),
            changed_slots: Vec::new(),
            carried_item: SimpleSlot::default(),
        },
    )
    .await
    .unwrap();

    let mut crafted_diamond = false;
    for _ in 0..24 {
        if let Some(frame) = timeout(
            Duration::from_millis(200),
            read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
        )
        .await
        .ok()
        .flatten()
            && frame.id == ContainerSetContent::ID
        {
            let c = frame.decode_as::<ContainerSetContent>().unwrap();
            if c.window_id.0 == crate::gameplay::inventory::CRAFTING_WINDOW_ID
                && c.slots[0].item_id == rad_core::mc777_data::items::DIAMOND
                && c.slots[0].count == 1
            {
                crafted_diamond = true;
                break;
            }
        }
    }
    assert!(
        crafted_diamond,
        "3x3 crafting table should match runtime reloaded datapack recipe"
    );

    drop(p1);
    drop(p2);
    accept_task.abort();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn e2e_item_components_chest_restart_roundtrip_preserves_patch() {
    use rad_core::packet::play::{ContainerSetContentStacks, ItemComponentPatch, SimpleSlot};

    let dir = std::env::temp_dir().join("rad_e2e_item_comp_chest_restart");
    let _ = std::fs::remove_dir_all(&dir);
    let expected_patch = ItemComponentPatch {
        custom_name: Some("Excalibur".into()),
        damage: Some(12),
        max_damage: Some(250),
        repair_cost: Some(3),
        unbreakable: true,
        removed: Vec::new(),
    };

    // Phase 1: Start server #1, seed player with component-rich sword in hotbar slot 0 (inv 36),
    // open chest, shift-click sword into chest, save_all, and shut down.
    {
        let cfg = test_server_config(ServerForwardingMode::Modern);
        let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
        state.world.write().await.set_block_in(
            crate::world::worldgen::DimensionId::Overworld,
            2,
            64,
            2,
            crate::gameplay::inventory::CHEST_BLOCK_STATE,
        );

        // Pre-seed playerdata for "comp_user" with empty main/hotbar except slot 36 = Iron Sword with patch.
        let uuid = rad_core::security::crypto::offline_player_uuid("comp_user");
        let mut seeded_inv = vec![
            SavedSlot {
                count: 0,
                item_id: 0,
                components: ItemComponentPatch::default(),
            };
            crate::gameplay::inventory::PLAYER_INVENTORY_SLOTS
        ];
        seeded_inv[36] = SavedSlot {
            count: 1,
            item_id: rad_core::mc777_data::items::IRON_SWORD,
            components: expected_patch.clone(),
        };
        let pdata = SavedPlayerData {
            dimension: crate::world::worldgen::DimensionId::Overworld,
            x: 0.5,
            y: 64.0,
            z: 0.5,
            yaw: 0.0,
            pitch: 0.0,
            hp: 20.0,
            food: 20,
            saturation: 5.0,
            game_mode: 0,
            selected_hotbar: 0,
            inventory: seeded_inv,
            stats: Default::default(),
        };
        state.save_player_data(uuid, &pdata).await.unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_state = Arc::clone(&state);
        let accept_task = tokio::spawn(async move {
            if let Ok((stream, peer)) = listener.accept().await {
                handle_connection(stream, peer, accept_state).await;
            }
        });

        let (mut p1, mut buf1) = connect_test_player(addr, "comp_user").await;

        // Open chest at (2, 64, 2) -> initial ContainerSetContentStacks (window 1) includes player's
        // hotbar slot 0 at window slot 54 with full ItemComponentPatch loaded from playerdata.
        send_packet(
            &mut p1,
            &UseItemOn {
                hand: VarInt(0),
                location: BlockPos::new(2, 64, 2),
                face: VarInt(1),
                cursor_x: 0.5,
                cursor_y: 1.0,
                cursor_z: 0.5,
                inside_block: false,
                world_border_hit: false,
                sequence: VarInt(1),
            },
        )
        .await
        .unwrap();

        let mut chest_sid = 0;
        let mut saw_initial_patch = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1 {
                    chest_sid = c.state_id.0;
                    if c.slots[54].item_id == rad_core::mc777_data::items::IRON_SWORD
                        && c.slots[54].components == expected_patch
                    {
                        saw_initial_patch = true;
                    }
                    break;
                }
            }
        }
        assert!(chest_sid > 0);
        assert!(
            saw_initial_patch,
            "player must receive ItemComponentPatch over wire on chest open"
        );

        // Shift-click hotbar slot 0 (inv 36 -> chest window slot 27 + (36 - 9) = 54) into chest
        send_packet(
            &mut p1,
            &ContainerClick {
                window_id: VarInt(1),
                state_id: VarInt(chest_sid),
                slot: 54,
                button: 0,
                mode: VarInt(1),
                changed_slots: Vec::new(),
                carried_item: SimpleSlot::default(),
            },
        )
        .await
        .unwrap();

        let mut saw_in_chest = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1
                    && c.slots[0].item_id == rad_core::mc777_data::items::IRON_SWORD
                    && c.slots[0].components == expected_patch
                {
                    saw_in_chest = true;
                    break;
                }
            }
        }
        assert!(
            saw_in_chest,
            "chest slot 0 must hold item with full components"
        );

        send_packet(
            &mut p1,
            &ContainerClose {
                window_id: VarInt(1),
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(p1);
        tokio::time::sleep(Duration::from_millis(50)).await;
        state.save_all().await.unwrap();
        accept_task.abort();
    }

    // Phase 2: Fresh ServerState from same world_dir -> reopen chest -> shift-click back -> verify patch!
    {
        let cfg = test_server_config(ServerForwardingMode::Modern);
        let state2 = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_state = Arc::clone(&state2);
        let accept_task = tokio::spawn(async move {
            if let Ok((stream, peer)) = listener.accept().await {
                handle_connection(stream, peer, accept_state).await;
            }
        });

        let (mut p1, mut buf1) = connect_test_player(addr, "comp_user").await;
        for _ in 0..120 {
            let Some(frame) = timeout(
                Duration::from_millis(60),
                read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
            )
            .await
            .ok()
            .flatten() else {
                break;
            };
            if frame.id == ContainerSetContentStacks::ID {
                break;
            }
        }

        // Open chest at (2, 64, 2) after full server restart
        send_packet(
            &mut p1,
            &UseItemOn {
                hand: VarInt(0),
                location: BlockPos::new(2, 64, 2),
                face: VarInt(1),
                cursor_x: 0.5,
                cursor_y: 1.0,
                cursor_z: 0.5,
                inside_block: false,
                world_border_hit: false,
                sequence: VarInt(1),
            },
        )
        .await
        .unwrap();

        let mut chest_sid = 0;
        let mut chest_has_patch_after_restart = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1 {
                    chest_sid = c.state_id.0;
                    if c.slots[0].item_id == rad_core::mc777_data::items::IRON_SWORD
                        && c.slots[0].components == expected_patch
                    {
                        chest_has_patch_after_restart = true;
                    }
                    break;
                }
            }
        }
        assert!(
            chest_has_patch_after_restart,
            "chest must retain ItemComponentPatch across full server restart"
        );

        // Shift-click chest slot 0 back into player inventory
        send_packet(
            &mut p1,
            &ContainerClick {
                window_id: VarInt(1),
                state_id: VarInt(chest_sid),
                slot: 0,
                button: 0,
                mode: VarInt(1),
                changed_slots: Vec::new(),
                carried_item: SimpleSlot::default(),
            },
        )
        .await
        .unwrap();

        let mut returned_with_patch = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut p1, &mut buf1, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 0
                    && c.slots.iter().any(|s| {
                        s.item_id == rad_core::mc777_data::items::IRON_SWORD
                            && s.components == expected_patch
                    })
                {
                    returned_with_patch = true;
                    break;
                }
            }
        }
        assert!(
            returned_with_patch,
            "withdrawing item from chest after restart must restore ItemComponentPatch to player inventory"
        );

        drop(p1);
        accept_task.abort();
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn e2e_two_player_concurrent_chest_components_and_death_respawn_restart() {
    use rad_core::packet::play::{
        ClientCommand, ContainerClick, ContainerClose, ContainerSetContentStacks,
        ItemComponentPatch, MovePlayerPos, Respawn, SetHealth, SimpleSlot, UseItemOn,
    };

    let dir = std::env::temp_dir().join(format!(
        "rad_two_p_comp_death_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let expected_patch = ItemComponentPatch {
        custom_name: Some("EnchantedApple".into()),
        damage: None,
        max_damage: None,
        repair_cost: Some(3),
        unbreakable: true,
        removed: vec![3],
    };

    // Phase 1: Alice and Bob concurrently split, place, and shift-click component-bearing apples
    // in the same chest, then Alice dies & respawns, then server restarts.
    {
        let mut cfg = test_server_config(ServerForwardingMode::Modern);
        cfg.game_mode = 0;
        let state = Arc::new(ServerState::new_with_world_dir(cfg, Some(dir.clone())));
        state.world.write().await.set_block_in(
            crate::world::worldgen::DimensionId::Overworld,
            2,
            64,
            2,
            crate::gameplay::inventory::CHEST_BLOCK_STATE,
        );

        let alice_uuid = rad_core::security::crypto::offline_player_uuid("comp_alice");
        let mut alice_inv = vec![
            SavedSlot {
                count: 0,
                item_id: 0,
                components: ItemComponentPatch::default(),
            };
            crate::gameplay::inventory::PLAYER_INVENTORY_SLOTS
        ];
        alice_inv[36] = SavedSlot {
            count: 16,
            item_id: rad_core::mc777_data::items::APPLE,
            components: expected_patch.clone(),
        };
        state
            .save_player_data(
                alice_uuid,
                &SavedPlayerData {
                    dimension: crate::world::worldgen::DimensionId::Overworld,
                    x: 0.5,
                    y: 64.0,
                    z: 0.5,
                    yaw: 0.0,
                    pitch: 0.0,
                    hp: 20.0,
                    food: 20,
                    saturation: 5.0,
                    game_mode: 0,
                    selected_hotbar: 0,
                    inventory: alice_inv,
                    stats: Default::default(),
                },
            )
            .await
            .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_state = Arc::clone(&state);
        let accept_task = tokio::spawn(async move {
            loop {
                if let Ok((stream, peer)) = listener.accept().await {
                    let s = Arc::clone(&accept_state);
                    tokio::spawn(handle_connection(stream, peer, s));
                }
            }
        });

        let (mut alice, mut buf_a) = connect_test_player(addr, "comp_alice").await;
        for _ in 0..120 {
            let Some(frame) = timeout(
                Duration::from_millis(60),
                read_raw_frame(&mut alice, &mut buf_a, 1024 * 1024),
            )
            .await
            .ok()
            .flatten() else {
                break;
            };
            if frame.id == ContainerSetContentStacks::ID {
                break;
            }
        }
        let (mut bob, mut buf_b) = connect_test_player(addr, "comp_bob").await;
        for _ in 0..120 {
            let Some(frame) = timeout(
                Duration::from_millis(60),
                read_raw_frame(&mut bob, &mut buf_b, 1024 * 1024),
            )
            .await
            .ok()
            .flatten() else {
                break;
            };
            if frame.id == ContainerSetContentStacks::ID {
                break;
            }
        }

        let open_chest = UseItemOn {
            hand: VarInt(0),
            location: BlockPos::new(2, 64, 2),
            face: VarInt(1),
            cursor_x: 0.5,
            cursor_y: 1.0,
            cursor_z: 0.5,
            inside_block: false,
            world_border_hit: false,
            sequence: VarInt(1),
        };
        send_packet(&mut alice, &open_chest).await.unwrap();
        send_packet(&mut bob, &open_chest).await.unwrap();

        let mut alice_sid = 0;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut alice, &mut buf_a, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1 {
                    alice_sid = c.state_id.0;
                    break;
                }
            }
        }
        let mut bob_sid = 0;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut bob, &mut buf_b, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1 {
                    bob_sid = c.state_id.0;
                    break;
                }
            }
        }
        assert!(alice_sid > 0 && bob_sid > 0);

        // 1. Alice shift-clicks 16 apples (slot 54) into chest slot 0.
        send_packet(
            &mut alice,
            &ContainerClick {
                window_id: VarInt(1),
                state_id: VarInt(alice_sid),
                slot: 54,
                button: 0,
                mode: VarInt(1),
                changed_slots: Vec::new(),
                carried_item: SimpleSlot::default(),
            },
        )
        .await
        .unwrap();

        // Both Alice and Bob must receive the 16 apples with full components in chest slot 0.
        let mut bob_saw_16 = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(200),
                read_raw_frame(&mut bob, &mut buf_b, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1
                    && c.slots[0].count == 16
                    && c.slots[0].item_id == rad_core::mc777_data::items::APPLE
                    && c.slots[0].components == expected_patch
                {
                    bob_sid = c.state_id.0;
                    bob_saw_16 = true;
                    break;
                }
            }
        }
        assert!(
            bob_saw_16,
            "second viewer Bob must see Alice's chest deposit with ItemComponentPatch"
        );

        // 2. Alice right-clicks chest slot 0 (split half: leaves 8, carries 8),
        //    right-clicks chest slot 1 (places 1, carries 7),
        //    left-clicks chest slot 2 (places 7).
        for (slot, button) in [(0i16, 1i8), (1i16, 1i8), (2i16, 0i8)] {
            // Drain Alice's latest state_id
            for _ in 0..8 {
                let Ok(Some(frame)) = timeout(
                    Duration::from_millis(40),
                    read_raw_frame(&mut alice, &mut buf_a, 1024 * 1024),
                )
                .await
                else {
                    break;
                };
                if frame.id == ContainerSetContentStacks::ID {
                    let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                    if c.window_id.0 == 1 {
                        alice_sid = c.state_id.0;
                    }
                }
            }
            send_packet(
                &mut alice,
                &ContainerClick {
                    window_id: VarInt(1),
                    state_id: VarInt(alice_sid),
                    slot,
                    button,
                    mode: VarInt(0),
                    changed_slots: Vec::new(),
                    carried_item: SimpleSlot::default(),
                },
            )
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        // Bob waits for the split result (8 in slot 0, 1 in slot 1, 7 in slot 2)
        let mut bob_saw_split = false;
        for _ in 0..32 {
            if let Some(frame) = timeout(
                Duration::from_millis(150),
                read_raw_frame(&mut bob, &mut buf_b, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1 {
                    bob_sid = c.state_id.0;
                    if c.slots[0].count == 8
                        && c.slots[1].count == 1
                        && c.slots[2].count == 7
                        && c.slots[0].components == expected_patch
                        && c.slots[1].components == expected_patch
                        && c.slots[2].components == expected_patch
                    {
                        bob_saw_split = true;
                        break;
                    }
                }
            }
        }
        assert!(
            bob_saw_split,
            "concurrent viewer Bob must observe 8/1/7 split with intact ItemComponentPatch"
        );

        // 3. Bob shift-clicks chest slot 2 (7 apples) into Bob's inventory.
        send_packet(
            &mut bob,
            &ContainerClick {
                window_id: VarInt(1),
                state_id: VarInt(bob_sid),
                slot: 2,
                button: 0,
                mode: VarInt(1),
                changed_slots: Vec::new(),
                carried_item: SimpleSlot::default(),
            },
        )
        .await
        .unwrap();

        let mut bob_took_7 = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(150),
                read_raw_frame(&mut bob, &mut buf_b, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                let in_chest_player_section = c.window_id.0 == 1
                    && c.slots[2].count == 0
                    && c.slots[27..].iter().any(|s| {
                        s.item_id == rad_core::mc777_data::items::APPLE
                            && s.count == 7
                            && s.components == expected_patch
                    });
                let in_player_window = c.window_id.0 == 0
                    && c.slots.iter().any(|s| {
                        s.item_id == rad_core::mc777_data::items::APPLE
                            && s.count == 7
                            && s.components == expected_patch
                    });
                if in_chest_player_section || in_player_window {
                    bob_took_7 = true;
                    break;
                }
            }
        }
        assert!(
            bob_took_7,
            "Bob must receive 7 apples with intact ItemComponentPatch"
        );

        // Close menus
        send_packet(
            &mut alice,
            &ContainerClose {
                window_id: VarInt(1),
            },
        )
        .await
        .unwrap();
        send_packet(
            &mut bob,
            &ContainerClose {
                window_id: VarInt(1),
            },
        )
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;

        // 4. Alice dies from lethal fall (within speed cap) and respawns via ClientCommand(0); chest items must remain conserved.
        for &(y, flags) in &[
            (69.0, 0x00),
            (74.0, 0x00),
            (79.0, 0x00),
            (84.0, 0x00),
            (89.0, 0x00),
            (84.0, 0x00),
            (79.0, 0x00),
            (74.0, 0x00),
            (69.0, 0x00),
            (64.0, 0x01),
        ] {
            tokio::time::sleep(Duration::from_millis(450)).await;
            send_packet(
                &mut alice,
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
        let mut alice_died_hp = None;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(150),
                read_raw_frame(&mut alice, &mut buf_a, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == SetHealth::ID
            {
                let hp = frame.decode_as::<SetHealth>().unwrap().health;
                if hp <= 0.0 {
                    alice_died_hp = Some(hp);
                    break;
                }
            }
        }
        assert_eq!(
            alice_died_hp,
            Some(0.0),
            "Alice must reach 0.0 HP on lethal fall"
        );

        send_packet(
            &mut alice,
            &ClientCommand {
                action_id: VarInt(0),
            },
        )
        .await
        .unwrap();

        let mut alice_respawned = false;
        let mut alice_hp_after_respawn = None;
        for _ in 0..30 {
            if let Some(frame) = timeout(
                Duration::from_millis(150),
                read_raw_frame(&mut alice, &mut buf_a, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
            {
                if frame.id == Respawn::ID {
                    alice_respawned = true;
                } else if frame.id == SetHealth::ID && alice_respawned {
                    alice_hp_after_respawn = Some(frame.decode_as::<SetHealth>().unwrap().health);
                    break;
                }
            }
        }
        assert!(alice_respawned, "Alice must respawn after lethal fall");
        assert_eq!(alice_hp_after_respawn, Some(20.0));

        drop(alice);
        drop(bob);
        tokio::time::sleep(Duration::from_millis(60)).await;
        state.save_all().await.unwrap();
        accept_task.abort();
    }

    // Phase 2: Full server restart -> verify exact conservation: 8 + 1 in chest, 7 in Bob's inventory!
    {
        let mut cfg2 = test_server_config(ServerForwardingMode::Modern);
        cfg2.game_mode = 0;
        let state2 = Arc::new(ServerState::new_with_world_dir(cfg2, Some(dir.clone())));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let accept_state = Arc::clone(&state2);
        let accept_task = tokio::spawn(async move {
            if let Ok((stream, peer)) = listener.accept().await {
                handle_connection(stream, peer, accept_state).await;
            }
        });

        let (mut bob, mut buf_b) = connect_test_player(addr, "comp_bob").await;

        send_packet(
            &mut bob,
            &UseItemOn {
                hand: VarInt(0),
                location: BlockPos::new(2, 64, 2),
                face: VarInt(1),
                cursor_x: 0.5,
                cursor_y: 1.0,
                cursor_z: 0.5,
                inside_block: false,
                world_border_hit: false,
                sequence: VarInt(1),
            },
        )
        .await
        .unwrap();

        let mut chest_conserved = false;
        let mut bob_has_7_after_restart = false;
        for _ in 0..24 {
            if let Some(frame) = timeout(
                Duration::from_millis(150),
                read_raw_frame(&mut bob, &mut buf_b, 1024 * 1024),
            )
            .await
            .ok()
            .flatten()
                && frame.id == ContainerSetContentStacks::ID
            {
                let c = frame.decode_as::<ContainerSetContentStacks>().unwrap();
                if c.window_id.0 == 1 {
                    let chest_total: i32 = c.slots[..27]
                        .iter()
                        .filter(|s| {
                            s.item_id == rad_core::mc777_data::items::APPLE
                                && s.components == expected_patch
                        })
                        .map(|s| s.count)
                        .sum();
                    if c.slots[0].count == 8 && c.slots[1].count == 1 && chest_total == 9 {
                        chest_conserved = true;
                    }
                    if c.slots[27..].iter().any(|s| {
                        s.item_id == rad_core::mc777_data::items::APPLE
                            && s.count == 7
                            && s.components == expected_patch
                    }) {
                        bob_has_7_after_restart = true;
                    }
                    break;
                }
            }
        }
        assert!(
            bob_has_7_after_restart,
            "Bob must retain 7 apples with ItemComponentPatch after restart"
        );
        assert!(
            chest_conserved,
            "chest must retain 8 + 1 = 9 apples with ItemComponentPatch after restart (total conserved = 16)"
        );

        drop(bob);
        accept_task.abort();
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn dropped_item_e2e_drop_pickup_partial_concurrent_dimension_and_restart() {
    use rad_core::packet::play::{
        ItemComponentPatch, ItemStack, MetadataValue, PlayerAction, SetEntityData, SpawnEntity,
        TakeItemEntity,
    };

    let temp_dir = std::env::temp_dir().join(format!(
        "rad_dropped_item_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&temp_dir);
    let mut cfg = test_server_config(ServerForwardingMode::Modern);
    cfg.level_name = temp_dir.to_string_lossy().into_owned();
    cfg.view_distance = 2;
    cfg.simulation_distance = 2;

    let state = Arc::new(ServerState::new_with_world_dir(
        cfg.clone(),
        Some(temp_dir.clone()),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_state = Arc::clone(&state);
    let server_task = tokio::spawn(async move {
        for _ in 0..3 {
            if let Ok((stream, peer)) = listener.accept().await {
                let s = Arc::clone(&server_state);
                tokio::spawn(async move {
                    let _ = handle_connection(stream, peer, s).await;
                });
            }
        }
    });

    let (mut dropper, mut dropper_buf) = connect_test_player(addr, "Dropper").await;
    let (mut picker, mut picker_buf) = connect_test_player(addr, "Picker").await;
    let (mut nether_bob, mut nether_buf) = connect_test_player(addr, "NetherBob").await;

    // Drain initial join packets for all 3 players
    tokio::time::sleep(Duration::from_millis(80)).await;
    for (sock, buf) in [
        (&mut dropper, &mut dropper_buf),
        (&mut picker, &mut picker_buf),
        (&mut nether_bob, &mut nether_buf),
    ] {
        while tokio::time::timeout(
            Duration::from_millis(20),
            read_raw_frame(sock, buf, 1024 * 1024),
        )
        .await
        .is_ok()
        {}
    }

    // Move NetherBob to Nether and Picker away from (0, 64, 0) initially so Picker doesn't auto-pickup yet
    let (dropper_eid, picker_eid, nether_eid) = {
        let mut players = state.players.write().await;
        let d_eid = players
            .values()
            .find(|p| p.username == "Dropper")
            .unwrap()
            .entity_id;
        let p_eid = players
            .values()
            .find(|p| p.username == "Picker")
            .unwrap()
            .entity_id;
        let n_eid = players
            .values()
            .find(|p| p.username == "NetherBob")
            .unwrap()
            .entity_id;
        if let Some(p) = players.get_mut(&p_eid) {
            p.x = 12.0;
            p.y = 64.0;
            p.z = 12.0;
        }
        if let Some(n) = players.get_mut(&n_eid) {
            n.dimension = crate::world::worldgen::DimensionId::Nether;
            n.x = 0.5;
            n.y = 64.0;
            n.z = 0.5;
        }
        (d_eid, p_eid, n_eid)
    };
    let _ = nether_eid;

    // Give Dropper 10 custom-named planks in selected hotbar slot 36
    let custom_comp = ItemComponentPatch {
        custom_name: Some("Sky Plank".to_owned()),
        ..Default::default()
    };
    {
        let inv_lock = state
            .live_inventories
            .read()
            .await
            .get(&dropper_eid)
            .unwrap()
            .clone();
        let mut inv = inv_lock.lock().unwrap();
        for idx in 9..45 {
            inv.slots[idx] = rad_core::packet::play::SimpleSlot::default();
            inv.slot_components[idx] = ItemComponentPatch::default();
        }
        inv.set_selected_hotbar(0);
        inv.slots[36] = rad_core::packet::play::SimpleSlot {
            count: 10,
            item_id: crate::gameplay::inventory::OAK_PLANKS_ITEM_ID,
        };
        inv.slot_components[36] = custom_comp.clone();
    }

    // 1. Q (status 4) drops 1 item; Ctrl+Q (status 3) drops remaining 9
    send_packet(
        &mut dropper,
        &PlayerAction {
            status: VarInt(4),
            location: BlockPos { x: 0, y: 0, z: 0 },
            face: 0,
            sequence: VarInt(1),
        },
    )
    .await
    .unwrap();
    send_packet(
        &mut dropper,
        &PlayerAction {
            status: VarInt(3),
            location: BlockPos { x: 0, y: 0, z: 0 },
            face: 0,
            sequence: VarInt(2),
        },
    )
    .await
    .unwrap();

    // Verify Picker receives 2 SpawnEntity (entity_type 72) + 2 SetEntityData with custom_name "Sky Plank"
    let mut spawned_items = Vec::new();
    let mut meta_items = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while (spawned_items.len() < 2 || meta_items.len() < 2)
        && tokio::time::Instant::now() < deadline
    {
        if let Ok(Some(frame)) = tokio::time::timeout(
            Duration::from_millis(100),
            read_raw_frame(&mut picker, &mut picker_buf, 1024 * 1024),
        )
        .await
        {
            if frame.id == SpawnEntity::ID
                && let Ok(s) = frame.decode_as::<SpawnEntity>()
                && s.entity_type.0 == rad_core::mc777_data::entities::ITEM
            {
                spawned_items.push(s);
            } else if frame.id == SetEntityData::ID
                && let Ok(m) = frame.decode_as::<SetEntityData>()
            {
                meta_items.push(m);
            }
        }
    }
    assert_eq!(spawned_items.len(), 2);
    assert_eq!(meta_items.len(), 2);
    assert!(meta_items.iter().any(|m| {
        m.entries.iter().any(|e| {
            e.index == 8
                && matches!(
                    &e.value,
                    MetadataValue::ItemStack(st)
                        if st.count == 1 && st.components.custom_name.as_deref() == Some("Sky Plank")
                )
        })
    }));
    assert!(meta_items.iter().any(|m| {
        m.entries.iter().any(|e| {
            e.index == 8
                && matches!(
                    &e.value,
                    MetadataValue::ItemStack(st)
                        if st.count == 9 && st.components.custom_name.as_deref() == Some("Sky Plank")
                )
        })
    }));

    // Verify NetherBob did NOT receive any Overworld dropped item packets
    let mut nether_saw_item = false;
    while let Ok(Some(frame)) = tokio::time::timeout(
        Duration::from_millis(40),
        read_raw_frame(&mut nether_bob, &mut nether_buf, 1024 * 1024),
    )
    .await
    {
        if frame.id == SpawnEntity::ID
            && let Ok(s) = frame.decode_as::<SpawnEntity>()
            && s.entity_type.0 == rad_core::mc777_data::entities::ITEM
        {
            nether_saw_item = true;
        }
    }
    assert!(!nether_saw_item);

    // 2. Full inventory must NOT destroy dropped items; partial pickup leaves remainder
    let item9_eid = {
        let mut entities = state.entities.write().await;
        // Remove the 1-count item so we test clean partial + concurrent pickup on the 9-count stack
        let mut eid9 = None;
        let ids: Vec<i32> = entities.all_dropped_items().map(|i| i.entity_id).collect();
        for id in ids {
            let item = entities.get_dropped_item_mut(id).unwrap();
            item.pickup_delay_ticks = 0;
            if item.stack.count == 1 {
                entities.remove_dropped_item(id);
            } else {
                item.x = 0.5;
                item.y = 64.0;
                item.z = 0.5;
                eid9 = Some(id);
            }
        }
        eid9.unwrap()
    };

    // Move Picker to (0.5, 64.0, 0.5) with completely full inventory
    {
        let mut players = state.players.write().await;
        if let Some(p) = players.get_mut(&picker_eid) {
            p.x = 0.5;
            p.y = 64.0;
            p.z = 0.5;
        }
        if let Some(d) = players.get_mut(&dropper_eid) {
            d.x = 30.0;
            d.z = 30.0;
        }
    }
    {
        let inv_lock = state
            .live_inventories
            .read()
            .await
            .get(&picker_eid)
            .unwrap()
            .clone();
        let mut inv = inv_lock.lock().unwrap();
        for idx in 9..45 {
            inv.slots[idx] = rad_core::packet::play::SimpleSlot {
                count: 64,
                item_id: 1,
            };
        }
    }
    let area = crate::world::sim::SimulationArea::new(
        2,
        [(crate::world::worldgen::DimensionId::Overworld, 0.5, 0.5)],
    );
    step_world_tick_in_area(&state, &area).await;
    // Item must still exist with count 9!
    assert_eq!(
        state
            .entities
            .write()
            .await
            .get_dropped_item_mut(item9_eid)
            .unwrap()
            .stack
            .count,
        9
    );

    // Give Picker space for exactly 3 more "Sky Plank" in slot 36 (61 + 3 = 64)
    {
        let inv_lock = state
            .live_inventories
            .read()
            .await
            .get(&picker_eid)
            .unwrap()
            .clone();
        let mut inv = inv_lock.lock().unwrap();
        inv.slots[36] = rad_core::packet::play::SimpleSlot {
            count: 61,
            item_id: crate::gameplay::inventory::OAK_PLANKS_ITEM_ID,
        };
        inv.slot_components[36] = custom_comp.clone();
    }
    step_world_tick_in_area(&state, &area).await;

    // 3 picked up -> 6 remain in world!
    assert_eq!(
        state
            .entities
            .write()
            .await
            .get_dropped_item_mut(item9_eid)
            .unwrap()
            .stack
            .count,
        6
    );
    let mut saw_take = false;
    while let Ok(Some(frame)) = tokio::time::timeout(
        Duration::from_millis(50),
        read_raw_frame(&mut picker, &mut picker_buf, 1024 * 1024),
    )
    .await
    {
        if frame.id == TakeItemEntity::ID
            && let Ok(t) = frame.decode_as::<TakeItemEntity>()
            && t.collected_entity_id.0 == item9_eid
            && t.collector_entity_id.0 == picker_eid
            && t.pickup_item_count.0 == 3
        {
            saw_take = true;
        }
    }
    assert!(saw_take);

    // 3. Concurrent pickup by 2 eligible players at the exact same coordinates: no duplication!
    {
        let mut players = state.players.write().await;
        if let Some(d) = players.get_mut(&dropper_eid) {
            d.x = 0.5;
            d.y = 64.0;
            d.z = 0.5;
        }
    }
    {
        let inv_lock = state
            .live_inventories
            .read()
            .await
            .get(&picker_eid)
            .unwrap()
            .clone();
        let mut inv = inv_lock.lock().unwrap();
        inv.slots[37] = rad_core::packet::play::SimpleSlot::default();
        inv.slot_components[37] = ItemComponentPatch::default();
    }
    step_world_tick_in_area(&state, &area).await;
    assert_eq!(state.entities.read().await.dropped_item_count(), 0);
    let count_in_dropper: i32 = {
        let inv_lock = state
            .live_inventories
            .read()
            .await
            .get(&dropper_eid)
            .unwrap()
            .clone();
        let inv = inv_lock.lock().unwrap();
        inv.slots[9..45]
            .iter()
            .filter(|s| s.item_id == crate::gameplay::inventory::OAK_PLANKS_ITEM_ID)
            .map(|s| s.count)
            .sum()
    };
    let count_in_picker: i32 = {
        let inv_lock = state
            .live_inventories
            .read()
            .await
            .get(&picker_eid)
            .unwrap()
            .clone();
        let inv = inv_lock.lock().unwrap();
        inv.slots[9..45]
            .iter()
            .filter(|s| s.item_id == crate::gameplay::inventory::OAK_PLANKS_ITEM_ID)
            .map(|s| s.count)
            .sum()
    };
    // Picker started with 61+3=64; total across both players must be exactly 64+6 = 70 (no duplication!)
    assert_eq!(count_in_dropper + count_in_picker, 70);

    // 4. Persistence across save_all and full server restart
    let persisted_eid = spawn_and_broadcast_dropped_item(
        &state,
        crate::world::worldgen::DimensionId::Overworld,
        [2.5, 65.0, 2.5],
        [0.0, 0.0, 0.0],
        ItemStack {
            item_id: crate::gameplay::inventory::STICK_ITEM_ID,
            count: 4,
            components: custom_comp.clone(),
        },
        40,
    )
    .await
    .unwrap();
    state.save_all().await.unwrap();
    drop(dropper);
    drop(picker);
    drop(nether_bob);
    drop(state);
    server_task.abort();

    let state2 = Arc::new(ServerState::new_with_world_dir(cfg, Some(temp_dir.clone())));
    assert_eq!(state2.entities.read().await.dropped_item_count(), 1);
    let listener2 = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let s2 = Arc::clone(&state2);
    let server_task2 = tokio::spawn(async move {
        if let Ok((stream, peer)) = listener2.accept().await {
            let _ = handle_connection(stream, peer, s2).await;
        }
    });

    let (mut late_joiner, mut late_buf) = connect_test_player(addr2, "LateJoiner").await;
    let mut saw_persisted_spawn = false;
    let mut saw_persisted_meta = false;
    let deadline2 = tokio::time::Instant::now() + Duration::from_secs(2);
    while (!saw_persisted_spawn || !saw_persisted_meta) && tokio::time::Instant::now() < deadline2 {
        if let Ok(Some(frame)) = tokio::time::timeout(
            Duration::from_millis(100),
            read_raw_frame(&mut late_joiner, &mut late_buf, 1024 * 1024),
        )
        .await
        {
            if frame.id == SpawnEntity::ID
                && let Ok(s) = frame.decode_as::<SpawnEntity>()
                && s.entity_id.0 == persisted_eid
                && s.entity_type.0 == rad_core::mc777_data::entities::ITEM
            {
                saw_persisted_spawn = true;
            } else if frame.id == SetEntityData::ID
                && let Ok(m) = frame.decode_as::<SetEntityData>()
                && m.entity_id.0 == persisted_eid
                && m.entries.iter().any(|e| {
                    e.index == 8
                        && matches!(
                            &e.value,
                            MetadataValue::ItemStack(st)
                                if st.count == 4 && st.components.custom_name.as_deref() == Some("Sky Plank")
                        )
                })
            {
                saw_persisted_meta = true;
            }
        }
    }
    assert!(saw_persisted_spawn);
    assert!(saw_persisted_meta);
    server_task2.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}
