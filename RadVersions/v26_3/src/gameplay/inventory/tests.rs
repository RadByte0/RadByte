use super::*;

#[test]
fn pick_block_updates_live_inventory_and_never_creates_in_survival() {
    let mut inv = PlayerInventory::default();
    let item = mc_items::by_name("minecraft:diamond_block").unwrap();
    assert_eq!(inv.pick_block_item(item, false), None);
    let slot = inv.pick_block_item(item, true).unwrap();
    assert_eq!(inv.selected_hotbar, slot);
    assert_eq!(inv.held_item().item_id, item);
    assert_eq!(inv.pick_block_item(item, false), Some(slot));
}

#[test]
fn hotbar_selection_and_block_mapping_work() {
    let mut inv = PlayerInventory::default();
    assert_eq!(inv.held_item().item_id, 1);
    inv.set_selected_hotbar(2);
    assert_eq!(inv.held_item().item_id, mc_items::DIRT);
    assert_eq!(
        item_id_to_block_state(inv.held_item().item_id),
        Some(mc_blocks::DIRT)
    );
    assert_eq!(item_id_to_block_state(STICK_ITEM_ID), None);
    assert!(inv.consume_held_item());
    assert_eq!(inv.held_item().count, 63);
    assert!(inv.give_block_drop(1)); // stone -> cobblestone (62)
}

#[test]
fn crafting_2x2_and_chest_container_clicks_conserve_items() {
    let mut inv = PlayerInventory::default();
    // Move 4 oak_planks from hotbar slot 40 (16 planks) into crafting grid 1..=4
    for grid_slot in 1..=4 {
        let sid = inv.state_id;
        assert_eq!(inv.apply_container_click(0, sid, 40, 1, 0, None), Ok(false)); // pickup half (8) -> wait: button 1 picks up half on empty cursor, let's use button 0 then right-click 1 into grid
        let _ = grid_slot;
    }

    // Fresh inventory test for exact 2x2 chest craft: place 1 plank in each of slots 1..=4
    let mut inv = PlayerInventory::default();
    let s1 = inv.state_id;
    assert_eq!(inv.apply_container_click(0, s1, 40, 0, 0, None), Ok(false)); // pick 16 planks
    for grid_slot in 1..=4 {
        let sid = inv.state_id;
        assert_eq!(
            inv.apply_container_click(0, sid, grid_slot, 1, 0, None),
            Ok(false)
        ); // place 1 plank
    }
    let sid = inv.state_id;
    assert_eq!(inv.apply_container_click(0, sid, 40, 0, 0, None), Ok(false)); // return remaining 12 planks

    assert_eq!(
        inv.slots[0],
        SimpleSlot {
            count: 1,
            item_id: CRAFTING_TABLE_ITEM_ID
        }
    );
    // Take crafted crafting_table via shift-click -> consumes 1 from each grid slot
    let sid = inv.state_id;
    assert_eq!(inv.apply_container_click(0, sid, 0, 0, 1, None), Ok(false));
    assert_eq!(inv.slots[0], SimpleSlot::default());
    assert_eq!(inv.slots[41].item_id, CRAFTING_TABLE_ITEM_ID);
    assert_eq!(inv.slots[41].count, 1);

    // Shift-click 12 planks from player slot into open chest (window 1)
    let mut chest = vec![SimpleSlot::default(); 27];
    let sid = inv.state_id;
    // In chest window, hotbar slot 40 is index 27 + (40 - 9) = 58
    assert_eq!(
        inv.apply_container_click(1, sid, 58, 0, 1, Some(&mut chest)),
        Ok(true)
    );
    assert_eq!(inv.slots[40], SimpleSlot::default());
    assert_eq!(
        chest[0],
        SimpleSlot {
            count: 12,
            item_id: OAK_PLANKS_ITEM_ID
        }
    );
}

#[test]
fn crafting_3x3_table_supports_pickaxe_furnace_and_close_return() {
    let mut inv = PlayerInventory::default();
    // 8 cobblestone (slot 39 in hotbar -> index 10 + (39 - 9) = 40 in crafting window) around perimeter -> Furnace
    let sid = inv.state_id;
    assert_eq!(
        inv.apply_container_click(CRAFTING_WINDOW_ID, sid, 40, 0, 0, None),
        Ok(false)
    );
    for grid_slot in [1i16, 2, 3, 4, 6, 7, 8, 9] {
        let sid = inv.state_id;
        assert_eq!(
            inv.apply_container_click(CRAFTING_WINDOW_ID, sid, grid_slot, 1, 0, None),
            Ok(false)
        );
    }
    let sid = inv.state_id;
    assert_eq!(
        inv.apply_container_click(CRAFTING_WINDOW_ID, sid, 40, 0, 0, None),
        Ok(false)
    );
    assert_eq!(
        inv.craft3x3[0],
        SimpleSlot {
            count: 1,
            item_id: FURNACE_ITEM_ID
        }
    );
    let sid = inv.state_id;
    assert_eq!(
        inv.apply_container_click(CRAFTING_WINDOW_ID, sid, 0, 0, 1, None),
        Ok(false)
    );
    assert_eq!(inv.slots[41].item_id, FURNACE_ITEM_ID);
    assert_eq!(inv.slots[41].count, 1);

    // Unconsumed item in grid slot 1 returns to inventory on close_craft3x3
    inv.craft3x3[1] = SimpleSlot {
        count: 3,
        item_id: STICK_ITEM_ID,
    };
    inv.close_craft3x3();
    assert_eq!(inv.craft3x3[1], SimpleSlot::default());
    assert_eq!(
        inv.slots
            .iter()
            .filter(|s| s.item_id == STICK_ITEM_ID)
            .map(|s| s.count)
            .sum::<i32>(),
        3
    );
}

#[test]
fn click_modes_number_swap_double_click_and_offhand_conserve_items() {
    let mut inv = PlayerInventory::default();
    let mut chest = vec![SimpleSlot::default(); 27];
    inv.slots[10] = SimpleSlot {
        count: 16,
        item_id: 1,
    };
    inv.slots[11] = SimpleSlot {
        count: 20,
        item_id: 1,
    };
    chest[0] = SimpleSlot {
        count: 10,
        item_id: 1,
    };

    // Total stone before: 64 (slot 36 default) + 16 + 20 + 10 = 110
    let total_stone = |i: &PlayerInventory, c: &[SimpleSlot]| -> i32 {
        let mut sum = i.carried.count * i32::from(i.carried.item_id == 1);
        sum += i
            .slots
            .iter()
            .filter(|s| s.item_id == 1)
            .map(|s| s.count)
            .sum::<i32>();
        sum += c
            .iter()
            .filter(|s| s.item_id == 1)
            .map(|s| s.count)
            .sum::<i32>();
        sum
    };
    assert_eq!(total_stone(&inv, &chest), 110);

    // Mode 2: swap slot 10 with hotbar button 1 (slot 37)
    let sid = inv.state_id;
    assert_eq!(inv.apply_container_click(0, sid, 10, 1, 2, None), Ok(false));
    assert_eq!(inv.slots[37].count, 16);
    assert_eq!(total_stone(&inv, &chest), 110);

    // Mode 2 with offhand (button 40): swap slot 37 with offhand (slot 45)
    let sid = inv.state_id;
    assert_eq!(
        inv.apply_container_click(0, sid, 37, 40, 2, None),
        Ok(false)
    );
    assert_eq!(inv.slots[45].count, 16);

    // swap_offhand() with selected hotbar 0 (slot 36: 64)
    inv.swap_offhand();
    assert_eq!(inv.slots[36].count, 16);
    assert_eq!(inv.slots[45].count, 64);

    // Pick up slot 36 (16 stone) then double-click (mode 6) while chest is open
    let sid = inv.state_id;
    assert_eq!(
        inv.apply_container_click(1, sid, 27 + 27, 0, 0, Some(&mut chest)),
        Ok(false)
    );
    assert_eq!(inv.carried.count, 16);
    let sid = inv.state_id;
    assert_eq!(
        inv.apply_container_click(1, sid, 0, 0, 6, Some(&mut chest)),
        Ok(true)
    );
    // Carried collects 10 from chest + 20 from slot 11 -> 16 + 10 + 20 = 46
    assert_eq!(inv.carried.count, 46);
    assert_eq!(total_stone(&inv, &chest), 110);
}

#[test]
fn datapack_json_recipes_load_and_craft() {
    let dir = std::env::temp_dir().join(format!(
        "rad_datapack_recipe_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let recipes_dir = dir.join("datapacks").join("recipes");
    std::fs::create_dir_all(&recipes_dir).unwrap();

    let recipe = DataPackRecipe {
        id: "radbyte:stone_to_chest".into(),
        ingredients: [1, 1, 0, 1, 1, 0, 0, 0, 0],
        result_item_id: CHEST_ITEM_ID,
        result_count: 2,
    };
    std::fs::write(
        recipes_dir.join("stone_to_iron.json"),
        serde_json::to_string_pretty(&recipe).unwrap(),
    )
    .unwrap();

    let loaded = load_custom_recipes_from_dir(&dir);
    assert_eq!(loaded, vec![recipe]);

    let mut grid = [SimpleSlot::default(); 9];
    for idx in [0, 1, 3, 4] {
        grid[idx] = SimpleSlot {
            count: 1,
            item_id: 1,
        };
    }
    let out = match_custom_recipe(&grid, &loaded);
    assert_eq!(
        out,
        SimpleSlot {
            count: 2,
            item_id: CHEST_ITEM_ID,
        }
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn all_inventory_operations_preserve_and_conserve_item_components() {
    let mut inv = PlayerInventory::default();
    for s in &mut inv.slots {
        *s = SimpleSlot::default();
    }
    let patch_a = ItemComponentPatch {
        custom_name: Some("Relic A".into()),
        damage: Some(2),
        max_damage: Some(100),
        repair_cost: Some(1),
        unbreakable: false,
        removed: Vec::new(),
    };
    let patch_b = ItemComponentPatch {
        custom_name: Some("Relic B".into()),
        ..Default::default()
    };

    // Seed 6 items with patch_a in slot 9, 4 items with patch_a in slot 10, and 3 items with patch_b in slot 11
    inv.slots[9] = SimpleSlot {
        count: 6,
        item_id: OAK_PLANKS_ITEM_ID,
    };
    inv.slot_components[9] = patch_a.clone();
    inv.slots[10] = SimpleSlot {
        count: 4,
        item_id: OAK_PLANKS_ITEM_ID,
    };
    inv.slot_components[10] = patch_a.clone();
    inv.slots[11] = SimpleSlot {
        count: 3,
        item_id: OAK_PLANKS_ITEM_ID,
    };
    inv.slot_components[11] = patch_b.clone();

    let mut chest_slots = vec![SimpleSlot::default(); 27];
    let mut chest_comps = vec![ItemComponentPatch::default(); 27];

    // 1. Right-click half-stack split on slot 9 (in window 0) -> 3 in slot 9, 3 on cursor with patch_a
    inv.apply_container_click_with_components(0, inv.state_id, 9, 1, 0, None)
        .unwrap();
    assert_eq!(inv.slots[9].count, 3);
    assert_eq!(inv.slot_components[9], patch_a);
    assert_eq!(inv.carried.count, 3);
    assert_eq!(inv.carried_components, patch_a);

    // 2. Right-click place 1 into slot 12 -> slot 12 gets count=1 + patch_a, cursor has 2
    inv.apply_container_click_with_components(0, inv.state_id, 12, 1, 0, None)
        .unwrap();
    assert_eq!(inv.slots[12].count, 1);
    assert_eq!(inv.slot_components[12], patch_a);
    assert_eq!(inv.carried.count, 2);

    // 3. Double-click collect (mode 6) on empty slot 13 -> collects slot 9 (3), slot 10 (4), slot 12 (1) => 10 total,
    //    WITHOUT touching slot 11 (patch_b must stay isolated)!
    inv.apply_container_click_with_components(0, inv.state_id, 13, 0, 6, None)
        .unwrap();
    assert_eq!(inv.carried.count, 10);
    assert_eq!(inv.carried_components, patch_a);
    assert_eq!(inv.slots[9].count, 0);
    assert!(inv.slot_components[9].is_empty());
    assert_eq!(inv.slots[11].count, 3);
    assert_eq!(inv.slot_components[11], patch_b);

    // 4. Left-click place cursor (10 x patch_a) into chest slot 0 (window 1)
    inv.apply_container_click_with_components(
        1,
        inv.state_id,
        0,
        0,
        0,
        Some((&mut chest_slots, Some(&mut chest_comps))),
    )
    .unwrap();
    assert_eq!(inv.carried.count, 0);
    assert!(inv.carried_components.is_empty());
    assert_eq!(chest_slots[0].count, 10);
    assert_eq!(chest_comps[0], patch_a);

    // 5. Hotbar number swap (mode 2, button 0 -> hotbar 36) on chest slot 0
    inv.apply_container_click_with_components(
        1,
        inv.state_id,
        0,
        0,
        2,
        Some((&mut chest_slots, Some(&mut chest_comps))),
    )
    .unwrap();
    assert_eq!(chest_slots[0].count, 0);
    assert!(chest_comps[0].is_empty());
    assert_eq!(inv.slots[36].count, 10);
    assert_eq!(inv.slot_components[36], patch_a);

    // 6. Offhand swap (hotbar 0 -> offhand 45) preserves patch_a
    inv.set_selected_hotbar(0);
    inv.swap_offhand();
    assert_eq!(inv.slots[45].count, 10);
    assert_eq!(inv.slot_components[45], patch_a);
    inv.swap_offhand();

    // 7. Drop 1 (mode 4, button 0) leaves 9 with patch_a; Drop stack (mode 4, button 1) clears slot & patch
    inv.apply_container_click_with_components(0, inv.state_id, 36, 0, 4, None)
        .unwrap();
    assert_eq!(inv.slots[36].count, 9);
    assert_eq!(inv.slot_components[36], patch_a);
    inv.apply_container_click_with_components(0, inv.state_id, 36, 1, 4, None)
        .unwrap();
    assert_eq!(inv.slots[36].count, 0);
    assert!(inv.slot_components[36].is_empty());
}

#[test]
fn drop_selected_hotbar_handles_single_and_full_stack() {
    let mut inv = PlayerInventory::default();
    inv.set_selected_hotbar(2);
    inv.set_slot(38, SimpleSlot::default());
    assert!(!inv.drop_selected_hotbar(false));
    inv.set_slot(
        38,
        SimpleSlot {
            count: 5,
            item_id: 1,
        },
    );
    assert!(inv.drop_selected_hotbar(false));
    assert_eq!(inv.slots[38].count, 4);
    assert!(inv.drop_selected_hotbar(true));
    assert_eq!(inv.slots[38].count, 0);
    assert_eq!(inv.slots[38].item_id, 0);
}
