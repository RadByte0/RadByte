use super::*;

#[test]
fn removing_support_breaks_redstone_and_updates_neighbors_once() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();
    assert!(world.set_block(0, 64, 0, mc_blocks::STONE));
    assert!(world.set_block(1, 64, 0, mc_blocks::STONE));
    assert!(world.set_block(0, 65, 0, mc_blocks::REDSTONE_WIRE));
    assert!(world.set_block(1, 65, 0, mc_blocks::REDSTONE_WIRE));
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, 0, 65, 0);
    assert!(world.set_block(0, 64, 0, 0));
    sim.on_block_changed(&mut world, 0, 64, 0, 0);
    let changes = sim.step(&mut world, 1);
    assert!(mc_blocks::is_air_state(world.get_block(0, 65, 0)));
    assert!(is_redstone_wire(world.get_block(1, 65, 0)));
    assert!(
        changes
            .iter()
            .any(|&(_, x, y, z, s)| (x, y, z, s) == (0, 65, 0, 0))
    );
    assert_eq!(
        sim.take_support_drops(),
        vec![(
            DimensionId::Overworld,
            0,
            65,
            0,
            rad_core::mc777_data::items::REDSTONE
        )]
    );
    sim.step(&mut world, 2);
    assert!(sim.take_support_drops().is_empty());
}

#[test]
fn support_updates_floor_blocks_without_breaking_unrelated_neighbors() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();
    for (x, name) in [
        (0, "minecraft:torch"),
        (2, "minecraft:rail"),
        (4, "minecraft:stone_pressure_plate"),
    ] {
        let block = mc_blocks::by_name(name).unwrap().default_state;
        assert!(world.set_block(x, 64, 0, mc_blocks::STONE));
        assert!(world.set_block(x, 65, 0, block));
        assert!(world.set_block(x, 64, 0, 0));
        sim.on_block_changed(&mut world, x, 64, 0, 0);
    }
    assert!(world.set_block(8, 64, 0, mc_blocks::STONE));
    assert!(world.set_block(8, 65, 0, mc_blocks::TORCH));
    sim.step(&mut world, 1);
    for x in [0, 2, 4] {
        assert!(mc_blocks::is_air_state(world.get_block(x, 65, 0)));
    }
    assert_eq!(world.get_block(8, 65, 0), mc_blocks::TORCH);
    assert_eq!(sim.take_support_drops().len(), 3);
    sim.step(&mut world, 2);
    assert!(sim.take_support_drops().is_empty());
}

#[test]
fn gravity_blocks_include_red_sand_powder_and_water() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();
    let red_sand = mc_blocks::by_name("minecraft:red_sand")
        .unwrap()
        .default_state;
    let powder = mc_blocks::by_name("minecraft:white_concrete_powder")
        .unwrap()
        .default_state;
    for (x, block) in [(2, red_sand), (5, powder), (8, GRAVEL_BLOCK_STATE)] {
        assert!(world.set_block(x, 64, 2, WATER_SOURCE_STATE));
        assert!(world.set_block(x, 65, 2, block));
        sim.on_block_changed(&mut world, x, 65, 2, 0);
    }
    for tick in 1..=20 {
        sim.step(&mut world, tick);
        sim.tick_falling_blocks(&mut world, tick);
    }
    assert_eq!(world.get_block(2, 64, 2), red_sand);
    assert_eq!(
        world.get_block(5, 64, 2),
        mc_blocks::by_name("minecraft:white_concrete")
            .unwrap()
            .default_state
    );
    assert_eq!(world.get_block(8, 64, 2), GRAVEL_BLOCK_STATE);
}

#[test]
fn simulation_area_handles_boundaries_players_and_dimensions() {
    let area = SimulationArea::new(
        1,
        [
            (DimensionId::Overworld, -0.01, 0.0),
            (DimensionId::Nether, 160.0, 0.0),
        ],
    );
    assert!(area.contains_block(DimensionId::Overworld, -32, 0));
    assert!(area.contains_block(DimensionId::Overworld, 15, 31));
    assert!(!area.contains_block(DimensionId::Overworld, 16, 0));
    assert!(area.contains_block(DimensionId::Nether, 160, 0));
    assert!(!area.contains_block(DimensionId::End, 160, 0));
    assert!(!SimulationArea::new(1, []).contains_block(DimensionId::Overworld, 0, 0));
}

#[test]
fn furnace_filter_pauses_and_resumes_without_losing_state() {
    let mut store = FurnaceStore::default();
    let furnace = FurnaceEntity {
        burn_time: 10,
        ..Default::default()
    };
    store.insert_in(DimensionId::Overworld, 0, 64, 0, furnace.clone());
    store.insert_in(DimensionId::Overworld, 160, 64, 0, furnace);
    let area = SimulationArea::new(1, [(DimensionId::Overworld, 0.0, 0.0)]);
    store.tick_where(|dim, x, z| area.contains_block(dim, x, z));
    assert_eq!(
        store
            .get_in(DimensionId::Overworld, 0, 64, 0)
            .unwrap()
            .burn_time,
        9
    );
    assert_eq!(
        store
            .get_in(DimensionId::Overworld, 160, 64, 0)
            .unwrap()
            .burn_time,
        10
    );
    let area = SimulationArea::new(1, [(DimensionId::Overworld, 160.0, 0.0)]);
    store.tick_where(|dim, x, z| area.contains_block(dim, x, z));
    assert_eq!(
        store
            .get_in(DimensionId::Overworld, 160, 64, 0)
            .unwrap()
            .burn_time,
        9
    );
}

#[test]
fn gravity_sand_falls_until_solid_ground() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    // Flat ground is at y=63 (stone/grass), so y=64, 65, 66 are air.
    assert!(world.set_block(2, 66, 2, SAND_BLOCK_STATE));
    sim.on_block_changed(&mut world, 2, 66, 2, 0);

    for t in 1..=20u64 {
        sim.step(&mut world, t);
        sim.tick_falling_blocks(&mut world, t);
    }
    assert_eq!(world.get_block(2, 66, 2), 0);
    assert_eq!(world.get_block(2, 65, 2), 0);
    assert_eq!(world.get_block(2, 64, 2), SAND_BLOCK_STATE);
}

#[test]
fn water_spreads_down_and_horizontally_bounded() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    assert!(world.set_block(5, 65, 5, WATER_SOURCE_STATE));
    sim.on_block_changed(&mut world, 5, 65, 5, 0);

    for t in 1..=30u64 {
        sim.step(&mut world, t);
    }
    assert_eq!(world.get_block(5, 64, 5), WATER_FLOW_STATE);
    assert_eq!(world.get_block(6, 64, 5), WATER_FLOW_STATE);
    assert_eq!(world.get_block(4, 64, 5), WATER_FLOW_STATE);
    // Bounded spread does not flood 10 blocks away
    assert_eq!(world.get_block(15, 64, 5), 0);
}

#[test]
fn crop_grows_deterministically_to_maturity() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    assert!(world.set_block(3, 64, 3, WHEAT_STAGE_0));
    sim.on_block_changed(&mut world, 3, 64, 3, 0);

    for t in 1..=200u64 {
        sim.step(&mut world, t);
    }
    assert_eq!(world.get_block(3, 64, 3), WHEAT_STAGE_MAX);
}

#[test]
fn furnace_smelts_and_resumes_after_reload() {
    let dir = std::env::temp_dir().join(format!(
        "rad_furnace_test_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let mut store = FurnaceStore::default();
    store.insert(
        1,
        64,
        1,
        FurnaceEntity {
            input_item: 62, // cobblestone
            input_count: 2,
            fuel_item: rad_core::mc777_data::items::OAK_PLANKS,
            fuel_count: 1,
            output_item: 0,
            output_count: 0,
            burn_time: 0,
            cook_progress: 0,
        },
    );

    // Run 100 ticks (halfway through first smelt), then persist
    for _ in 0..100 {
        store.tick_all();
    }
    let (rev, snap) = store.snapshot_if_dirty().unwrap();
    FurnaceStore::write_snapshot_atomic(&dir, &snap).unwrap();
    store.mark_saved(rev);

    // Reload and run 105 more ticks -> first stone completes
    let mut reloaded = FurnaceStore::load_from_dir(&dir);
    for _ in 0..105 {
        reloaded.tick_all();
    }
    let f = reloaded.get(1, 64, 1).unwrap();
    assert_eq!(
        f.output_slot(),
        SimpleSlot {
            count: 1,
            item_id: 1
        }
    );
    assert_eq!(f.input_count, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn redstone_lever_propagates_power_and_tnt_carves_crater() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    world.set_block(0, 64, 0, LEVER_OFF_STATE);
    world.set_block(1, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(2, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(3, 64, 0, REDSTONE_LAMP_OFF);

    let on_updates = sim.toggle_lever(&mut world, 0, 64, 0);
    assert!(on_updates.contains(&(3, 64, 0, REDSTONE_LAMP_ON)));
    let wire2 = RedstoneWireState::from_state_id(world.get_block(2, 64, 0)).unwrap();
    assert_eq!(wire2.power, 14);
    assert_eq!(wire2.west, WireConnection::Side);
    assert_eq!(wire2.east, WireConnection::Side);
    assert_eq!(world.get_block(3, 64, 0), REDSTONE_LAMP_ON);

    let off_updates = sim.toggle_lever(&mut world, 0, 64, 0);
    assert!(off_updates.contains(&(3, 64, 0, REDSTONE_LAMP_OFF)));
    assert_eq!(world.get_block(3, 64, 0), REDSTONE_LAMP_OFF);

    world.set_block(10, 64, 10, TNT_BLOCK_STATE);
    let blast = WorldSim::detonate_tnt(&mut world, 10, 64, 10, 2);
    assert!(!blast.is_empty());
    assert_eq!(world.get_block(10, 64, 10), 0);
    assert_eq!(world.get_block(10, 63, 10), 0);
}

#[test]
fn redstone_wire_shapes_straight_corner_t_junction_and_diagonal_isolation() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    // Diagonal isolation: (0, 64, 0) and (1, 64, 1) must not connect.
    world.set_block(0, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(1, 64, 1, REDSTONE_WIRE_UNPOWERED);
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, 0, 64, 0);
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, 1, 64, 1);
    let diag_a = RedstoneWireState::from_state_id(world.get_block(0, 64, 0)).unwrap();
    let diag_b = RedstoneWireState::from_state_id(world.get_block(1, 64, 1)).unwrap();
    assert_eq!(
        (diag_a.east, diag_a.north, diag_a.south, diag_a.west),
        (
            WireConnection::None,
            WireConnection::None,
            WireConnection::None,
            WireConnection::None
        )
    );
    assert_eq!(
        (diag_b.east, diag_b.north, diag_b.south, diag_b.west),
        (
            WireConnection::None,
            WireConnection::None,
            WireConnection::None,
            WireConnection::None
        )
    );

    // Straight line along X: (10, 64, 0), (11, 64, 0), (12, 64, 0)
    world.set_block(10, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(11, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(12, 64, 0, REDSTONE_WIRE_UNPOWERED);
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, 11, 64, 0);
    let straight = RedstoneWireState::from_state_id(world.get_block(11, 64, 0)).unwrap();
    assert_eq!(straight.east, WireConnection::Side);
    assert_eq!(straight.west, WireConnection::Side);
    assert_eq!(straight.north, WireConnection::None);
    assert_eq!(straight.south, WireConnection::None);

    // Corner + T-junction: add (11, 64, -1) (north of 11,64,0) -> T-junction at (11,64,0)
    world.set_block(11, 64, -1, REDSTONE_WIRE_UNPOWERED);
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, 11, 64, -1);
    let tjunc = RedstoneWireState::from_state_id(world.get_block(11, 64, 0)).unwrap();
    assert_eq!(tjunc.east, WireConnection::Side);
    assert_eq!(tjunc.west, WireConnection::Side);
    assert_eq!(tjunc.north, WireConnection::Side);
    assert_eq!(tjunc.south, WireConnection::None);
}

#[test]
fn redstone_wire_step_up_and_step_down_connections_and_power() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    // Lever at (-1, 64, 0), wire at (0, 64, 0), solid stone at (1, 64, 0), wire at (1, 65, 0)
    world.set_block(-1, 64, 0, LEVER_OFF_STATE);
    world.set_block(0, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(1, 64, 0, 1); // stone
    world.set_block(1, 65, 0, REDSTONE_WIRE_UNPOWERED);

    sim.toggle_lever(&mut world, -1, 64, 0);
    let lower = RedstoneWireState::from_state_id(world.get_block(0, 64, 0)).unwrap();
    let upper = RedstoneWireState::from_state_id(world.get_block(1, 65, 0)).unwrap();
    assert_eq!(lower.east, WireConnection::Up);
    assert_eq!(lower.power, 15);
    assert_eq!(upper.west, WireConnection::Side);
    assert_eq!(upper.power, 14);
}

#[test]
fn redstone_wire_power_attenuation_0_to_15() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    world.set_block(0, 64, 0, LEVER_OFF_STATE);
    for x in 1..=16 {
        world.set_block(x, 64, 0, REDSTONE_WIRE_UNPOWERED);
    }
    sim.toggle_lever(&mut world, 0, 64, 0);

    for x in 1..=15 {
        let w = RedstoneWireState::from_state_id(world.get_block(x, 64, 0)).unwrap();
        assert_eq!(w.power, (16 - x) as u8, "unexpected power at x={x}");
    }
    let w16 = RedstoneWireState::from_state_id(world.get_block(16, 64, 0)).unwrap();
    assert_eq!(w16.power, 0);
}

#[test]
fn redstone_wire_multi_source_and_cycle_keeps_power_when_one_turns_off() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    // 2x2 wire loop at (1,64,0), (2,64,0), (2,64,1), (1,64,1)
    // Lever A at (0,64,0), Lever B at (3,64,1)
    world.set_block(0, 64, 0, LEVER_OFF_STATE);
    world.set_block(3, 64, 1, LEVER_OFF_STATE);
    world.set_block(1, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(2, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(2, 64, 1, REDSTONE_WIRE_UNPOWERED);
    world.set_block(1, 64, 1, REDSTONE_WIRE_UNPOWERED);

    sim.toggle_lever(&mut world, 0, 64, 0);
    sim.toggle_lever(&mut world, 3, 64, 1);
    assert!(
        RedstoneWireState::from_state_id(world.get_block(1, 64, 0))
            .unwrap()
            .power
            > 0
    );
    assert!(
        RedstoneWireState::from_state_id(world.get_block(2, 64, 1))
            .unwrap()
            .power
            > 0
    );

    // Turn off Lever A: loop must remain powered by Lever B
    sim.toggle_lever(&mut world, 0, 64, 0);
    assert!(
        RedstoneWireState::from_state_id(world.get_block(1, 64, 0))
            .unwrap()
            .power
            > 0
    );

    // Turn off Lever B: loop must unpower
    sim.toggle_lever(&mut world, 3, 64, 1);
    assert_eq!(
        RedstoneWireState::from_state_id(world.get_block(1, 64, 0))
            .unwrap()
            .power,
        0
    );
}

#[test]
fn redstone_wire_cross_dimension_isolation_and_negative_coords() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    world.set_block_in(DimensionId::Overworld, -2, 64, -1, LEVER_OFF_STATE);
    world.set_block_in(DimensionId::Overworld, -1, 64, -1, REDSTONE_WIRE_UNPOWERED);
    world.set_block_in(DimensionId::Nether, -1, 64, -1, REDSTONE_WIRE_UNPOWERED);

    sim.toggle_lever_in(&mut world, DimensionId::Overworld, -2, 64, -1);
    assert_eq!(
        RedstoneWireState::from_state_id(world.get_block_in(DimensionId::Overworld, -1, 64, -1))
            .unwrap()
            .power,
        15
    );
    assert_eq!(
        RedstoneWireState::from_state_id(world.get_block_in(DimensionId::Nether, -1, 64, -1))
            .unwrap()
            .power,
        0
    );
}

#[test]
fn wall_torch_drops_when_supporting_wall_is_broken() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    let wall_torch = mc_blocks::by_name("minecraft:wall_torch")
        .unwrap()
        .min_state; // facing=north -> attached to south (z + 1)
    world.set_block(0, 64, 1, 1); // stone wall at z=1
    world.set_block(0, 64, 0, wall_torch);

    // Break the supporting wall at (0, 64, 1)
    world.set_block(0, 64, 1, 0);
    sim.on_block_changed(&mut world, 0, 64, 1, 0);
    let updates = sim.step(&mut world, 1);
    assert!(updates.contains(&(DimensionId::Overworld, 0, 64, 0, 0)));
    assert_eq!(world.get_block(0, 64, 0), 0);
    let drops = std::mem::take(&mut sim.pending_block_drops);
    assert_eq!(drops.len(), 1);
    assert_eq!(
        drops[0].4,
        rad_core::mc777_data::items::by_name("minecraft:torch").unwrap()
    );
}

#[test]
fn button_press_powers_redstone_and_releases_after_delay() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    let stone_btn = mc_blocks::by_name("minecraft:stone_button")
        .unwrap()
        .default_state;
    world.set_block(0, 64, 0, stone_btn);
    world.set_block(1, 64, 0, REDSTONE_WIRE_UNPOWERED);
    world.set_block(2, 64, 0, REDSTONE_LAMP_OFF);

    let on_updates = sim.press_button_in(&mut world, DimensionId::Overworld, 0, 64, 0, 10);
    assert!(!on_updates.is_empty());
    assert_eq!(
        button_is_powered_with(world.get_block(0, 64, 0), world.registries()),
        Some(true)
    );
    assert_eq!(world.get_block(2, 64, 0), REDSTONE_LAMP_ON);

    // Before 20 ticks: still powered
    sim.step(&mut world, 25);
    assert_eq!(
        button_is_powered_with(world.get_block(0, 64, 0), world.registries()),
        Some(true)
    );

    // At tick 30 (10 + 20): releases and turns off lamp
    sim.step(&mut world, 30);
    assert_eq!(
        button_is_powered_with(world.get_block(0, 64, 0), world.registries()),
        Some(false)
    );
    assert_eq!(world.get_block(2, 64, 0), REDSTONE_LAMP_OFF);
}

#[test]
fn door_trapdoor_and_fence_gate_toggle_open() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    let door_min = mc_blocks::by_name("minecraft:oak_door").unwrap().min_state;
    let door_upper = door_min + 2; // closed upper (open=false is +2)
    let door_lower = door_min + 8 + 2; // closed lower
    world.set_block(0, 64, 0, door_lower);
    world.set_block(0, 65, 0, door_upper);
    let door_updates = sim.toggle_openable_in(&mut world, DimensionId::Overworld, 0, 64, 0);
    assert_eq!(door_updates.len(), 2);
    assert_eq!(world.get_block(0, 64, 0), door_min + 8);
    assert_eq!(world.get_block(0, 65, 0), door_min);

    let trap = mc_blocks::by_name("minecraft:oak_trapdoor")
        .unwrap()
        .default_state;
    world.set_block(2, 64, 0, trap);
    let trap_updates = sim.toggle_openable_in(&mut world, DimensionId::Overworld, 2, 64, 0);
    assert_eq!(trap_updates.len(), 1);
    assert_ne!(world.get_block(2, 64, 0), trap);

    let gate = mc_blocks::by_name("minecraft:oak_fence_gate")
        .unwrap()
        .default_state;
    world.set_block(4, 64, 0, gate);
    let gate_updates = sim.toggle_openable_in(&mut world, DimensionId::Overworld, 4, 64, 0);
    assert_eq!(gate_updates.len(), 1);
    assert_ne!(world.get_block(4, 64, 0), gate);
}

#[test]
fn piston_push_and_sticky_piston_pull() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();

    // Sticky piston facing east (facing_idx = 1 -> min_state + 6 + 1)
    let sticky_min = mc_blocks::by_name("minecraft:sticky_piston")
        .unwrap()
        .min_state;
    let sticky_east = sticky_min + 6 + 1;
    world.set_block(0, 64, 0, sticky_east);
    world.set_block(1, 64, 0, 1); // stone at x=1
    world.set_block(-1, 64, 0, LEVER_OFF_STATE);

    // Power lever -> piston extends and pushes stone to x=2
    sim.toggle_lever_in(&mut world, DimensionId::Overworld, -1, 64, 0);
    sim.update_piston_at_in(&mut world, DimensionId::Overworld, 0, 64, 0);
    assert_eq!(world.get_block(0, 64, 0), sticky_min + 1); // extended
    assert_eq!(
        mc_blocks::by_state(world.get_block(1, 64, 0)).map(|b| b.name),
        Some("minecraft:piston_head")
    );
    assert_eq!(world.get_block(2, 64, 0), 1);

    // Unpower lever -> sticky piston retracts and pulls stone back to x=1
    sim.toggle_lever_in(&mut world, DimensionId::Overworld, -1, 64, 0);
    sim.update_piston_at_in(&mut world, DimensionId::Overworld, 0, 64, 0);
    assert_eq!(world.get_block(0, 64, 0), sticky_east);
    assert_eq!(world.get_block(1, 64, 0), 1);
    assert_eq!(world.get_block(2, 64, 0), 0);
}

#[test]
fn piston_repeater_indirect_power_and_partner_cleanup() {
    let mut world = new_test_world();
    let mut sim = WorldSim::default();

    let piston_min = mc_blocks::by_name("minecraft:piston").unwrap().min_state;
    let piston_east = piston_min + 6 + 1;
    world.set_block(0, 64, 0, piston_east);
    world.set_block(1, 64, 0, 1); // stone in front

    // Powered repeater at (-1, 64, 0) pointing east (facing=3, powered=true -> min + 3*4 + 2 + 0 = min + 14)
    let rep_min = mc_blocks::by_name("minecraft:repeater").unwrap().min_state;
    world.set_block(-1, 64, 0, rep_min + 14);
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, -1, 64, 0);
    assert_eq!(world.get_block(0, 64, 0), piston_min + 1); // extended
    assert_eq!(world.get_block(2, 64, 0), 1); // pushed stone

    let head_state = world.get_block(1, 64, 0);
    assert_eq!(
        crate::world::sim::piston_partner_pos(piston_min + 1, 0, 64, 0, world.registries()),
        Some((1, 64, 0))
    );
    assert_eq!(
        crate::world::sim::piston_partner_pos(head_state, 1, 64, 0, world.registries()),
        Some((0, 64, 0))
    );

    // Unpower repeater and test indirect power via solid block + lever
    world.set_block(-1, 64, 0, 0);
    sim.update_redstone_around_in(&mut world, DimensionId::Overworld, -1, 64, 0);
    assert_eq!(world.get_block(0, 64, 0), piston_east);

    world.set_block(-1, 64, 0, 1); // solid stone next to piston
    world.set_block(-2, 64, 0, LEVER_OFF_STATE);
    sim.toggle_lever_in(&mut world, DimensionId::Overworld, -2, 64, 0);
    assert_eq!(world.get_block(0, 64, 0), piston_min + 1); // indirectly powered!
}

#[test]
fn orient_block_on_place_handles_pistons_stairs_furnace_lever_and_wall_torch() {
    let furnace = mc_blocks::by_name("minecraft:furnace").unwrap();
    // Player looking south (yaw=0) -> furnace faces north (min + 0*2 + 1)
    assert_eq!(
        orient_block_on_place(furnace.default_state, 0.0, 0.0, 1, 0.5),
        furnace.min_state + 1
    );
    // Player looking north (yaw=180) -> furnace faces south (min + 1*2 + 1)
    assert_eq!(
        orient_block_on_place(furnace.default_state, 180.0, 0.0, 1, 0.5),
        furnace.min_state + 3
    );

    // Piston looking down (pitch=80) -> piston faces up (4)
    let piston = mc_blocks::by_name("minecraft:piston").unwrap();
    assert_eq!(
        orient_block_on_place(piston.default_state, 0.0, 80.0, 1, 0.5),
        piston.min_state + 6 + 4
    );

    // Torch placed on north face (face=2) -> wall_torch facing north
    let torch = mc_blocks::by_name("minecraft:torch").unwrap();
    let wall_torch = mc_blocks::by_name("minecraft:wall_torch").unwrap();
    assert_eq!(
        orient_block_on_place(torch.default_state, 0.0, 0.0, 2, 0.5),
        wall_torch.min_state
    );
}

#[test]
fn simulation_area_suspend_resume_nonblocking_tickets_and_persistence() {
    let dir = std::env::temp_dir().join(format!("rad_sim_area_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let mut world = WorldState::new(Some(dir.clone()));
    let mut sim = WorldSim::default();

    // Active sand in chunk (0, 0) and inactive sand in far chunk (40, 40) + negative chunk (-2, -2)
    world.set_block(2, 66, 2, SAND_BLOCK_STATE);
    world.set_block(2, 65, 2, 0);
    world.set_block(2, 64, 2, 1);
    sim.on_block_changed_in(&mut world, DimensionId::Overworld, 2, 66, 2, 0);

    world.set_block(-20, 66, -20, SAND_BLOCK_STATE);
    world.set_block(-20, 65, -20, 0);
    world.set_block(-20, 64, -20, 1);
    sim.on_block_changed_in(&mut world, DimensionId::Overworld, -20, 66, -20, 0);

    // Flood 300 due tasks into an inactive chunk (100, 100) without loading chunk (100, 100)!
    let loaded_before = world.loaded_chunk_count();
    for i in 0..300 {
        sim.push(ScheduledEntry {
            due_tick: 2,
            dim: DimensionId::Overworld,
            x: 1600 + (i % 16),
            y: 65 + (i / 16),
            z: 1600,
            action: SimAction::CropGrow,
        });
    }

    // Player at (0.5, 0.5) with radius=1: chunk (0,0) active; (-2,-2) and (100,100) inactive
    let area_p1 = SimulationArea::new(1, [(DimensionId::Overworld, 0.5, 0.5)]);
    assert!(area_p1.contains_chunk(DimensionId::Overworld, -1, -1));
    assert!(!area_p1.contains_chunk(DimensionId::Overworld, -2, -2));
    assert!(!area_p1.contains_chunk(DimensionId::Nether, 0, 0));

    // Step ticks 1..=3: active sand falls, 300 inactive tasks are suspended WITHOUT loading chunk (100,100)
    // and WITHOUT blocking active sand at (2, 66, 2)!
    for t in 1..=12 {
        sim.step_in_area(&mut world, t, &area_p1);
        sim.tick_falling_blocks(&mut world, t);
    }
    assert_eq!(world.loaded_chunk_count(), loaded_before);
    assert_eq!(world.get_block(2, 65, 2), SAND_BLOCK_STATE);
    assert_eq!(world.get_block(-20, 66, -20), SAND_BLOCK_STATE);
    assert!(sim.suspended_count() >= 301);

    // Save sim_queue.json, reload from disk, and verify suspended tasks survive restart
    let (rev, snap) = sim.snapshot_if_dirty(3).unwrap();
    WorldSim::write_snapshot_atomic(&dir, &snap).unwrap();
    sim.mark_saved(rev);

    let mut reloaded = WorldSim::load_from_dir(&dir);
    assert_eq!(reloaded.suspended_count(), sim.suspended_count());

    // Move player near (-20, -20): suspended sand at (-20, 66, -20) resumes and falls!
    let area_neg = SimulationArea::new(1, [(DimensionId::Overworld, -20.5, -20.5)]);
    for t in 4..=15 {
        reloaded.step_in_area(&mut world, t, &area_neg);
        reloaded.tick_falling_blocks(&mut world, t);
    }
    assert_eq!(world.get_block(-20, 65, -20), SAND_BLOCK_STATE);

    // Ticket keeps Nether chunk (0, 0) active even with no players
    let ticket_area = SimulationArea::new(1, []).with_tickets([SimulationTicket {
        dimension: DimensionId::Nether,
        chunk_x: 0,
        chunk_z: 0,
        radius: 0,
    }]);
    assert!(!ticket_area.is_empty());
    assert!(ticket_area.contains_chunk(DimensionId::Nether, 0, 0));
    assert!(!ticket_area.contains_chunk(DimensionId::Overworld, 0, 0));

    // Corrupt sim_queue.json is quarantined safely
    std::fs::write(dir.join("sim_queue.json"), b"{not valid json").unwrap();
    let recovered = WorldSim::load_from_dir(&dir);
    assert_eq!(recovered.pending_count(), 0);
    assert!(dir.join("sim_queue.json.corrupt").exists());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn furnace_pauses_outside_simulation_area_and_resumes_inside() {
    let mut store = FurnaceStore::default();
    store.insert_in(
        DimensionId::Overworld,
        40,
        64,
        0,
        FurnaceEntity {
            input_item: rad_core::mc777_data::items::COBBLESTONE,
            input_count: 1,
            fuel_item: rad_core::mc777_data::items::COAL,
            fuel_count: 1,
            ..Default::default()
        },
    );
    // Chunk (2, 0) at x=40; player at (0.5, 0.5) with radius=1 covers chunks -1..=1
    let far = SimulationArea::new(1, [(DimensionId::Overworld, 0.5, 0.5)]);
    for _ in 0..50 {
        store.tick_in_area(&far);
    }
    let paused = store.get_in(DimensionId::Overworld, 40, 64, 0).unwrap();
    assert_eq!(paused.cook_progress, 0);
    assert_eq!(paused.burn_time, 0);

    // Second player near (40.5, 0.5) overlaps/activates chunk (2, 0)
    let near = SimulationArea::new(
        1,
        [
            (DimensionId::Overworld, 0.5, 0.5),
            (DimensionId::Overworld, 40.5, 0.5),
        ],
    );
    for _ in 0..200 {
        store.tick_in_area(&near);
    }
    let done = store.get_in(DimensionId::Overworld, 40, 64, 0).unwrap();
    assert_eq!(done.output_count, 1);
}

#[test]
fn repeater_delays_signal_and_cycles_delay() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();
    for x in 0..=4 {
        world.set_block(x, 63, 0, mc_blocks::STONE);
    }
    // Repeater at (2, 64, 0) facing east (facing=3 -> state 8719 unpowered delay=1)
    let rep_default = 8704 + 3 * 4 + 2 + 1; // delay=1, facing=east(3), locked=false(2), powered=false(1) => 8719
    assert_eq!(
        repeater_info_with(rep_default, world.registries()),
        Some((1, 3, false, false))
    );
    world.set_block(2, 64, 0, rep_default);
    // Output wire at (3, 64, 0)
    world.set_block(
        3,
        64,
        0,
        RedstoneWireState {
            east: WireConnection::Side,
            north: WireConnection::Side,
            power: 0,
            south: WireConnection::Side,
            west: WireConnection::Side,
        }
        .to_state_id(),
    );
    // Power source at back (1, 64, 0)
    let redstone_block = mc_blocks::by_name("minecraft:redstone_block")
        .unwrap()
        .default_state;
    world.set_block(1, 64, 0, redstone_block);
    sim.on_block_changed(&mut world, 2, 64, 0, 1);

    // Before 2 ticks have passed, repeater and output wire are still unpowered
    sim.step(&mut world, 2);
    assert_eq!(world.get_block(2, 64, 0), rep_default);
    assert_eq!(
        RedstoneWireState::from_state_id(world.get_block(3, 64, 0))
            .unwrap()
            .power,
        0
    );

    // At tick 3 (1 + 1*2), repeater turns on and powers output wire to 15
    sim.step(&mut world, 3);
    assert_eq!(
        repeater_info_with(world.get_block(2, 64, 0), world.registries()),
        Some((1, 3, false, true))
    );
    assert_eq!(
        RedstoneWireState::from_state_id(world.get_block(3, 64, 0))
            .unwrap()
            .power,
        15
    );

    // Cycle delay: 1 -> 2 -> 3 -> 4 -> 1
    let d2 = sim
        .repeater_cycle_delay_in(&mut world, DimensionId::Overworld, 2, 64, 0)
        .unwrap();
    assert_eq!(
        repeater_info_with(d2, world.registries()),
        Some((2, 3, false, true))
    );
    let d3 = sim
        .repeater_cycle_delay_in(&mut world, DimensionId::Overworld, 2, 64, 0)
        .unwrap();
    assert_eq!(
        repeater_info_with(d3, world.registries()),
        Some((3, 3, false, true))
    );
    let d4 = sim
        .repeater_cycle_delay_in(&mut world, DimensionId::Overworld, 2, 64, 0)
        .unwrap();
    assert_eq!(
        repeater_info_with(d4, world.registries()),
        Some((4, 3, false, true))
    );
    let d1 = sim
        .repeater_cycle_delay_in(&mut world, DimensionId::Overworld, 2, 64, 0)
        .unwrap();
    assert_eq!(
        repeater_info_with(d1, world.registries()),
        Some((1, 3, false, true))
    );
}

#[test]
fn falling_block_breaks_into_item_when_landing_on_non_supporting_block() {
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();
    let stone = mc_blocks::by_name("minecraft:stone").unwrap().default_state;
    let torch = mc_blocks::by_name("minecraft:torch").unwrap().default_state;
    assert!(world.set_block(4, 63, 4, stone));
    assert!(world.set_block(4, 64, 4, torch));
    assert!(world.set_block(4, 67, 4, SAND_BLOCK_STATE));
    sim.on_block_changed(&mut world, 4, 67, 4, 0);

    for tick in 1..=30 {
        sim.step(&mut world, tick);
        sim.tick_falling_blocks(&mut world, tick);
    }

    assert_eq!(world.get_block(4, 64, 4), torch);
    assert_eq!(world.get_block(4, 65, 4), 0);
    let drops = sim.take_support_drops();
    let sand_item = rad_core::mc777_data::items::SAND;
    assert!(
        drops
            .iter()
            .any(|&(dim, x, y, z, item_id)| dim == DimensionId::Overworld
                && x == 4
                && y == 64
                && z == 4
                && item_id == sand_item),
        "expected sand item drop at torch position, got {drops:?}"
    );
}

#[test]
fn falling_block_persists_across_sim_snapshot_reload() {
    let dir = std::env::temp_dir().join(format!(
        "radbyte_sim_fb_persist_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut world = WorldState::new(None);
    let mut sim = WorldSim::default();
    let stone = mc_blocks::by_name("minecraft:stone").unwrap().default_state;
    assert!(world.set_block(2, 64, 2, stone));
    assert!(world.set_block(2, 75, 2, SAND_BLOCK_STATE));
    sim.on_block_changed(&mut world, 2, 75, 2, 0);

    for tick in 1..=5 {
        sim.step(&mut world, tick);
        sim.tick_falling_blocks(&mut world, tick);
    }
    assert_eq!(sim.falling_blocks.len(), 1);
    assert_eq!(world.get_block(2, 75, 2), 0);

    let (rev, snap) = sim
        .snapshot_if_dirty(5)
        .expect("mid-air falling block should mark sim dirty");
    WorldSim::write_snapshot_atomic(&dir, &snap).unwrap();
    sim.mark_saved(rev);

    let mut reloaded = WorldSim::load_from_dir(&dir);
    assert_eq!(reloaded.falling_blocks.len(), 1);
    for tick in 6..=40 {
        reloaded.step(&mut world, tick);
        reloaded.tick_falling_blocks(&mut world, tick);
    }
    assert_eq!(world.get_block(2, 65, 2), SAND_BLOCK_STATE);

    let _ = std::fs::remove_dir_all(&dir);
}
