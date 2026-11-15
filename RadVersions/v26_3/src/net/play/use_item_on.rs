//! UseItemOn packet handler: block interactions, containers, redstone, and block placement.

use super::*;
use crate::world::worldgen::DimensionId;

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_use_item_on(
    state: &Arc<ServerState>,
    entity_id: i32,
    player_uuid: Uuid,
    username: &str,
    cur_x: f64,
    cur_y: f64,
    cur_z: f64,
    cur_yaw: f32,
    cur_pitch: f32,
    inventory: &Arc<std::sync::Mutex<PlayerInventory>>,
    tx: &OutboundSender,
    open_chest_pos: &mut Option<(DimensionId, i32, i32, i32)>,
    open_furnace_pos: &mut Option<(DimensionId, i32, i32, i32)>,
    use_on: UseItemOn,
) {
    let cfg = &state.config;
    let (dead, mode, hp) = {
        let players = state.players.read().await;
        players
            .get(&entity_id)
            .map_or((true, cfg.game_mode, 0.0), |p| (p.dead, p.game_mode, p.hp))
    };
    let place_pos = offset_block_pos(use_on.location, use_on.face.0);
    let dx = f64::from(place_pos.x) + 0.5 - cur_x;
    let dy = f64::from(place_pos.y) + 0.5 - (cur_y + 1.62);
    let dz = f64::from(place_pos.z) + 0.5 - cur_z;
    let within_reach = dx * dx + dy * dy + dz * dz <= 36.0
        && place_pos.x.abs() <= cfg.max_world_size
        && place_pos.z.abs() <= cfg.max_world_size;
    let held = inventory.lock().unwrap().held_item();
    let player_box = Aabb::from_player(cur_x, cur_y, cur_z);
    let overlaps_player = player_box.max_x > f64::from(place_pos.x)
        && player_box.min_x < f64::from(place_pos.x + 1)
        && player_box.max_y > f64::from(place_pos.y)
        && player_box.min_y < f64::from(place_pos.y + 1)
        && player_box.max_z > f64::from(place_pos.z)
        && player_box.min_z < f64::from(place_pos.z + 1);

    let cur_dim = state
        .players
        .read()
        .await
        .get(&entity_id)
        .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
            p.dimension
        });
    let clicked_state = {
        let mut world = state.world.write().await;
        world.get_block_in(
            cur_dim,
            use_on.location.x,
            use_on.location.y,
            use_on.location.z,
        )
    };
    let cdx = f64::from(use_on.location.x) + 0.5 - cur_x;
    let cdy = f64::from(use_on.location.y) + 0.5 - (cur_y + 1.62);
    let cdz = f64::from(use_on.location.z) + 0.5 - cur_z;
    let clicked_within_reach = cdx * cdx + cdy * cdy + cdz * cdz <= 36.0
        && use_on.location.x.abs() <= cfg.max_world_size
        && use_on.location.z.abs() <= cfg.max_world_size;

    if !dead
        && mode != 3
        && clicked_within_reach
        && clicked_state == state.registries.blocks.crafting_table.default_state
    {
        *open_chest_pos = None;
        *open_furnace_pos = None;
        let loaded_recipes = state.recipes.read().await.clone();
        {
            inventory.lock().unwrap().custom_recipes = loaded_recipes;
        }
        if let Some(p) = state.players.write().await.get_mut(&entity_id) {
            p.open_chest = None;
        }
        if let Ok(open_bytes) = encode_to_bytes(&OpenScreen {
            window_id: VarInt(crate::gameplay::inventory::CRAFTING_WINDOW_ID),
            window_type: VarInt(state.registries.menus.crafting),
            title: rad_core::nbt::NbtTag::String("Crafting".into()),
        }) {
            let _ = tx.try_send(open_bytes);
        }
        let craft_pkt = {
            inventory
                .lock()
                .unwrap()
                .to_crafting_container_set_content(crate::gameplay::inventory::CRAFTING_WINDOW_ID)
        };
        if let Ok(content_bytes) = encode_to_bytes(&craft_pkt) {
            let _ = tx.try_send(content_bytes);
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && (state.registries.blocks.lever.min_state..=state.registries.blocks.lever.max_state)
            .contains(&clicked_state)
    {
        let updates = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.toggle_lever_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
            )
        };
        for (bx, by, bz, new_state) in updates {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: BlockPos::new(bx, by, bz),
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if clicked_within_reach
        && use_on.face.0 == 1
        && crate::world::sim::is_redstone_wire_with(clicked_state, &state.registries)
    {
        let toggled = {
            let mut world = state.world.write().await;
            state
                .world_sim
                .write()
                .await
                .toggle_redstone_wire_dot_cross_in(
                    &mut world,
                    cur_dim,
                    use_on.location.x,
                    use_on.location.y,
                    use_on.location.z,
                )
        };
        if let Some(new_state) = toggled {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: use_on.location,
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| b.name == "minecraft:comparator")
    {
        let updated = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.comparator_toggle_mode_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
            )
        };
        if let Some(new_state) = updated {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: use_on.location,
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| b.name == "minecraft:repeater")
    {
        let updated = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.repeater_cycle_delay_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
            )
        };
        if let Some(new_state) = updated {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: use_on.location,
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| b.name.ends_with("_button"))
    {
        let cur_tick = state.tick_count.load(std::sync::atomic::Ordering::Relaxed);
        let updates = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.press_button_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
                cur_tick,
            )
        };
        for (bx, by, bz, new_state) in updates {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: BlockPos::new(bx, by, bz),
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| {
                (b.name.ends_with("_door") && b.name != "minecraft:iron_door")
                    || (b.name.ends_with("_trapdoor") && b.name != "minecraft:iron_trapdoor")
                    || b.name.ends_with("_fence_gate")
            })
    {
        let updates = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.toggle_openable_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
            )
        };
        for (bx, by, bz, new_state) in updates {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: BlockPos::new(bx, by, bz),
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && clicked_state == state.registries.blocks.tnt.default_state
    {
        let updates = {
            let mut world = state.world.write().await;
            crate::world::sim::WorldSim::detonate_tnt_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
                3,
            )
        };
        for (bx, by, bz, new_state) in updates {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: BlockPos::new(bx, by, bz),
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if clicked_within_reach
        && use_on.face.0 == 1
        && crate::world::sim::is_redstone_wire_with(clicked_state, &state.registries)
    {
        let toggled = {
            let mut world = state.world.write().await;
            state
                .world_sim
                .write()
                .await
                .toggle_redstone_wire_dot_cross_in(
                    &mut world,
                    cur_dim,
                    use_on.location.x,
                    use_on.location.y,
                    use_on.location.z,
                )
        };
        if let Some(new_state) = toggled {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: use_on.location,
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| b.name == "minecraft:comparator")
    {
        let updated = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.comparator_toggle_mode_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
            )
        };
        if let Some(new_state) = updated {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: use_on.location,
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| b.name == "minecraft:repeater")
    {
        let updated = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.repeater_cycle_delay_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
            )
        };
        if let Some(new_state) = updated {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: use_on.location,
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if clicked_within_reach
        && state
            .registries
            .block_by_state(clicked_state)
            .is_some_and(|b| b.name.ends_with("_button"))
    {
        let cur_tick = state.tick_count.load(std::sync::atomic::Ordering::Relaxed);
        let updates = {
            let mut world = state.world.write().await;
            state.world_sim.write().await.press_button_in(
                &mut world,
                cur_dim,
                use_on.location.x,
                use_on.location.y,
                use_on.location.z,
                cur_tick,
            )
        };
        for (bx, by, bz, new_state) in updates {
            state
                .broadcast_in_dimension_except(
                    cur_dim,
                    &BlockUpdate {
                        pos: BlockPos::new(bx, by, bz),
                        block_state: VarInt(i32::from(new_state)),
                    },
                    None,
                )
                .await;
        }
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && clicked_state == state.registries.blocks.furnace.default_state
    {
        *open_chest_pos = None;
        let fpos = (
            cur_dim,
            use_on.location.x,
            use_on.location.y,
            use_on.location.z,
        );
        *open_furnace_pos = Some(fpos);
        let fsnap = {
            let mut furnaces = state.furnaces.write().await;
            furnaces
                .ensure_get_mut_in(fpos.0, fpos.1, fpos.2, fpos.3)
                .clone()
        };
        let mut batch = BytesMut::with_capacity(512);
        let wid = VarInt(crate::gameplay::inventory::FURNACE_WINDOW_ID);
        let _ = encode_packet(
            &OpenScreen {
                window_id: wid,
                window_type: VarInt(state.registries.menus.furnace),
                title: rad_core::nbt::NbtTag::String("Furnace".into()),
            },
            &mut batch,
        );
        let _ = encode_packet(
            &inventory.lock().unwrap().to_furnace_container_set_content(
                crate::gameplay::inventory::FURNACE_WINDOW_ID,
                rad_core::packet::play::SimpleSlot {
                    count: fsnap.input_count,
                    item_id: fsnap.input_item,
                },
                rad_core::packet::play::SimpleSlot {
                    count: fsnap.fuel_count,
                    item_id: fsnap.fuel_item,
                },
                fsnap.output_slot(),
            ),
            &mut batch,
        );
        let _ = encode_packet(
            &rad_core::packet::play::ContainerSetData {
                window_id: wid,
                property: 0,
                value: fsnap.burn_time as i16,
            },
            &mut batch,
        );
        let _ = encode_packet(
            &rad_core::packet::play::ContainerSetData {
                window_id: wid,
                property: 1,
                value: 300,
            },
            &mut batch,
        );
        let _ = encode_packet(
            &rad_core::packet::play::ContainerSetData {
                window_id: wid,
                property: 2,
                value: fsnap.cook_progress as i16,
            },
            &mut batch,
        );
        let _ = encode_packet(
            &rad_core::packet::play::ContainerSetData {
                window_id: wid,
                property: 3,
                value: 200,
            },
            &mut batch,
        );
        let _ = tx.try_send(batch.freeze());
    } else if !dead
        && mode != 3
        && clicked_within_reach
        && clicked_state == state.registries.blocks.chest.default_state
    {
        *open_furnace_pos = None;
        let cpos = (
            cur_dim,
            use_on.location.x,
            use_on.location.y,
            use_on.location.z,
        );
        let chest_data = {
            let mut containers = state.containers.write().await;
            containers.ensure_chest_in(cpos.0, cpos.1, cpos.2, cpos.3);
            let slots = containers
                .get_chest_mut_in(cpos.0, cpos.1, cpos.2, cpos.3)
                .cloned();
            let comps = containers
                .get_chest_components_in(cpos.0, cpos.1, cpos.2, cpos.3)
                .map(<[rad_core::packet::play::ItemComponentPatch]>::to_vec)
                .unwrap_or_default();
            slots.map(|s| (s, comps))
        };
        if let Some((slots, comps)) = chest_data {
            *open_chest_pos = Some(cpos);
            if let Some(p) = state.players.write().await.get_mut(&entity_id) {
                p.open_chest = Some(cpos);
            }
            if let Ok(open_bytes) = encode_to_bytes(&OpenScreen {
                window_id: VarInt(1),
                window_type: VarInt(2),
                title: rad_core::nbt::NbtTag::String("Chest".into()),
            }) {
                let _ = tx.try_send(open_bytes);
            }
            if let Ok(content_bytes) = {
                let inv_guard = inventory.lock().unwrap();
                encode_to_bytes(&inv_guard.to_chest_container_set_content_stacks(1, &slots, &comps))
            } {
                let _ = tx.try_send(content_bytes);
            }
        }
    } else if !dead
        && mode != 2
        && mode != 3
        && within_reach
        && held.count > 0
        && let Some(raw_block_state) =
            crate::gameplay::inventory::item_id_to_block_state_with(held.item_id, &state.registries)
    {
        let block_state = crate::world::sim::orient_block_on_place_with(
            raw_block_state,
            cur_yaw,
            cur_pitch,
            use_on.face.0,
            use_on.cursor_y,
            &state.registries,
        );
        let is_solid_place = crate::world::physics::block_collision_boxes_with(
            block_state,
            state.registries.provider().as_ref(),
        )
        .is_some_and(|boxes| !boxes.is_empty());
        let target_state =
            state
                .world
                .write()
                .await
                .get_block_in(cur_dim, place_pos.x, place_pos.y, place_pos.z);
        let target_replaceable = state.registries.is_air_state(target_state)
            || state.registries.is_fluid_state(target_state);
        let has_support =
            if crate::world::sim::is_redstone_wire_with(block_state, &state.registries)
                || crate::world::sim::floor_dependent_item_with(block_state, &state.registries)
                    .is_some()
            {
                place_pos.y > crate::world::anvil::MIN_WORLD_Y
                    && crate::world::sim::supports_redstone_with(
                        state.world.write().await.get_block_in(
                            cur_dim,
                            place_pos.x,
                            place_pos.y - 1,
                            place_pos.z,
                        ),
                        &state.registries,
                    )
            } else {
                true
            };
        if target_replaceable && has_support && (!is_solid_place || !overlaps_player) {
            let p_ctx = ServerPlayerContext {
                entity_id,
                username: username.to_owned(),
                x: cur_x,
                y: cur_y,
                z: cur_z,
                hp,
            };
            let (res, actions) = state.radons.read().await.dispatch_block_place(
                &p_ctx,
                place_pos.x,
                place_pos.y,
                place_pos.z,
                block_state,
            );
            state.execute_radon_actions(actions).await;

            if res == ServerEventResult::Allow {
                let cur_dim = state
                    .players
                    .read()
                    .await
                    .get(&entity_id)
                    .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
                        p.dimension
                    });
                if is_spawn_protected(cfg, cur_dim, place_pos)
                    && state.permission_level(username, Some(player_uuid)).await < 1
                {
                    let prev = state.world.write().await.get_block_in(
                        cur_dim,
                        place_pos.x,
                        place_pos.y,
                        place_pos.z,
                    );
                    if let Ok(bytes) = encode_to_bytes(&BlockUpdate {
                        pos: place_pos,
                        block_state: VarInt(i32::from(prev)),
                    }) {
                        let _ = tx.try_send(bytes);
                    }
                    let _ = tx.try_send(
                        encode_to_bytes(&AcknowledgeBlockChange {
                            sequence: use_on.sequence,
                        })
                        .unwrap_or_default(),
                    );
                    return;
                }
                let changed = {
                    let mut world = state.world.write().await;
                    let ok = world.set_block_in(
                        cur_dim,
                        place_pos.x,
                        place_pos.y,
                        place_pos.z,
                        block_state,
                    );
                    if ok {
                        let cur_tick = state.tick_count.load(Ordering::Relaxed);
                        state.world_sim.write().await.on_block_changed_in(
                            &mut world,
                            cur_dim,
                            place_pos.x,
                            place_pos.y,
                            place_pos.z,
                            cur_tick,
                        );
                    }
                    ok
                };
                if changed {
                    if state
                        .registries
                        .block_by_state(block_state)
                        .is_some_and(|b| b.name.ends_with("_door"))
                    {
                        let upper_state = block_state.saturating_sub(8);
                        let upper_y = place_pos.y + 1;
                        let upper_placed = {
                            let mut world = state.world.write().await;
                            let cur_up =
                                world.get_block_in(cur_dim, place_pos.x, upper_y, place_pos.z);
                            if state.registries.is_air_state(cur_up)
                                || state.registries.is_fluid_state(cur_up)
                            {
                                let ok = world.set_block_in(
                                    cur_dim,
                                    place_pos.x,
                                    upper_y,
                                    place_pos.z,
                                    upper_state,
                                );
                                if ok {
                                    let cur_tick = state.tick_count.load(Ordering::Relaxed);
                                    state.world_sim.write().await.on_block_changed_in(
                                        &mut world,
                                        cur_dim,
                                        place_pos.x,
                                        upper_y,
                                        place_pos.z,
                                        cur_tick,
                                    );
                                }
                                ok
                            } else {
                                false
                            }
                        };
                        if upper_placed {
                            state
                                .broadcast_in_dimension_except(
                                    cur_dim,
                                    &BlockUpdate {
                                        pos: BlockPos::new(place_pos.x, upper_y, place_pos.z),
                                        block_state: VarInt(i32::from(upper_state)),
                                    },
                                    None,
                                )
                                .await;
                        }
                    }
                    state
                        .player_stats
                        .write()
                        .await
                        .entry(player_uuid)
                        .or_default()
                        .blocks_placed += 1;
                    if block_state == state.registries.blocks.furnace.default_state {
                        state.furnaces.write().await.insert_in(
                            cur_dim,
                            place_pos.x,
                            place_pos.y,
                            place_pos.z,
                            crate::world::sim::FurnaceEntity::default(),
                        );
                    }
                    if block_state == state.registries.blocks.chest.default_state {
                        state.containers.write().await.ensure_chest_in(
                            cur_dim,
                            place_pos.x,
                            place_pos.y,
                            place_pos.z,
                        );
                    }
                    if mode == 0 && inventory.lock().unwrap().consume_held_item() {
                        sync_player_inventory(state, entity_id, inventory, tx).await;
                    }
                    state
                        .broadcast_in_dimension_except(
                            cur_dim,
                            &BlockUpdate {
                                pos: place_pos,
                                block_state: VarInt(i32::from(block_state)),
                            },
                            None,
                        )
                        .await;
                    let rs_updates = {
                        let mut world = state.world.write().await;
                        state.world_sim.write().await.update_redstone_around_in(
                            &mut world,
                            cur_dim,
                            place_pos.x,
                            place_pos.y,
                            place_pos.z,
                        )
                    };
                    for (ux, uy, uz, ustate) in rs_updates {
                        state
                            .broadcast_in_dimension_except(
                                cur_dim,
                                &BlockUpdate {
                                    pos: BlockPos {
                                        x: ux,
                                        y: uy,
                                        z: uz,
                                    },
                                    block_state: VarInt(i32::from(ustate)),
                                },
                                None,
                            )
                            .await;
                    }
                }
            }
        }
    }
    if let Ok(ack_bytes) = encode_to_bytes(&AcknowledgeBlockChange {
        sequence: use_on.sequence,
    }) {
        let _ = tx.try_send(ack_bytes);
    }
}
