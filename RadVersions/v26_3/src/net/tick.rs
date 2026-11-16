//! Fixed-rate 20 TPS server tick loop, player survival tick, and world simulation step.

use super::*;

/// Spawns the fixed-rate server tick loop (default `20 TPS` = `50 ms` per tick) with shutdown cancellation.
pub fn spawn_tick_loop_with_shutdown(
    state: Arc<ServerState>,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    let tps = u64::from(state.config.tps.max(1));
    let tick_ns = 1_000_000_000 / tps;
    let keepalive_every_ticks = tps * 15;
    let autosave_every_ticks = tps.saturating_mul(state.config.autosave_interval_secs);

    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_nanos(tick_ns));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut ka_seq: i64 = 1;

        loop {
            tokio::select! {
                biased;
                _ = async { let _ = shutdown_rx.wait_for(|&s| s).await; } => {
                    break;
                }
                _ = ticker.tick() => {}
            }
            if *shutdown_rx.borrow() {
                break;
            }
            let start = Instant::now();
            let tick_num = state.tick_count.fetch_add(1, Ordering::Relaxed) + 1;

            if tick_num.is_multiple_of(keepalive_every_ticks) {
                ka_seq = ka_seq.wrapping_add(1);
                step_keep_alive(&state, ka_seq, Duration::from_secs(30)).await;
            }
            if tick_num.is_multiple_of(tps) {
                if state.config.player_idle_timeout > 0 {
                    let idle_mins = u64::try_from(state.config.player_idle_timeout).unwrap_or(0);
                    step_idle_timeouts(&state, Duration::from_secs(idle_mins.saturating_mul(60)))
                        .await;
                }
                let (game_time, day_time) = state.sync_level_time().await;
                state
                    .broadcast_except(
                        &rad_core::packet::play::SetTime::overworld(game_time, day_time, 1.0),
                        None,
                    )
                    .await;
            }

            let sim_area = state.simulation_area().await;
            step_world_tick_in_area(&state, &sim_area).await;
            step_player_portals(&state).await;
            let (sim_updates, fb_spawns, fb_moves, fb_removed, support_drops) = {
                let mut world = state.world.write().await;
                let mut sim = state.world_sim.write().await;
                let mut updates = sim.step_in_area(&mut world, tick_num, &sim_area);
                let previous_y: HashMap<_, _> = sim
                    .falling_blocks
                    .iter()
                    .map(|fb| (fb.entity_id, fb.y))
                    .collect();
                let (fb_updates, fb_removed) = sim.tick_falling_blocks(&mut world, tick_num);
                updates.extend(fb_updates);
                let fb_spawns: Vec<_> = sim
                    .falling_blocks
                    .iter()
                    .filter(|fb| fb.age_ticks == 1)
                    .map(|fb| {
                        (
                            fb.dim,
                            fb.entity_id,
                            fb.uuid,
                            fb.x,
                            fb.y,
                            fb.z,
                            fb.block_state,
                        )
                    })
                    .collect();
                let fb_moves: Vec<_> = sim
                    .falling_blocks
                    .iter()
                    .filter(|fb| fb.age_ticks > 1)
                    .filter_map(|fb| {
                        previous_y.get(&fb.entity_id).and_then(|old| {
                            i16::try_from(((fb.y * 4096.0).round() - (old * 4096.0).round()) as i64)
                                .ok()
                                .map(|dy| (fb.dim, fb.entity_id, dy))
                        })
                    })
                    .collect();
                let support_drops = sim.take_support_drops();
                (updates, fb_spawns, fb_moves, fb_removed, support_drops)
            };
            for (dim, entity_id, uuid, x, y, z, block_state) in fb_spawns {
                state
                    .broadcast_in_dimension_except(
                        dim,
                        &rad_core::packet::play::SpawnEntity {
                            entity_id: VarInt(entity_id),
                            uuid,
                            entity_type: VarInt(52),
                            x,
                            y,
                            z,
                            pitch: 0,
                            yaw: 0,
                            head_yaw: 0,
                            data: VarInt(i32::from(block_state)),
                            velocity: rad_core::types::LpVec3::ZERO,
                        },
                        None,
                    )
                    .await;
            }
            for (dim, entity_id, dy) in fb_moves {
                state
                    .broadcast_in_dimension_except(
                        dim,
                        &MoveEntityPosRot {
                            entity_id: VarInt(entity_id),
                            steps: Vec::new(),
                            dx: 0,
                            dy,
                            dz: 0,
                            yaw: 0,
                            pitch: 0,
                            on_ground: false,
                        },
                        None,
                    )
                    .await;
            }
            if !fb_removed.is_empty() {
                state
                    .broadcast_except(
                        &RemoveEntities {
                            entity_ids: fb_removed.into_iter().map(VarInt).collect(),
                        },
                        None,
                    )
                    .await;
            }
            for (dim, bx, by, bz, bstate) in sim_updates {
                state
                    .broadcast_in_dimension_except(
                        dim,
                        &BlockUpdate {
                            pos: BlockPos::new(bx, by, bz),
                            block_state: VarInt(i32::from(bstate)),
                        },
                        None,
                    )
                    .await;
            }
            for (dim, x, y, z, item_id) in support_drops {
                let _ = spawn_and_broadcast_dropped_item(
                    &state,
                    dim,
                    [f64::from(x) + 0.5, f64::from(y) + 0.2, f64::from(z) + 0.5],
                    [0.0, 0.0, 0.0],
                    rad_core::packet::play::ItemStack {
                        count: 1,
                        item_id,
                        components: Default::default(),
                    },
                    10,
                )
                .await;
            }
            state
                .furnaces
                .write()
                .await
                .tick_in_area_with(&sim_area, state.registries.provider().as_ref());
            let regen_interval = tps.saturating_mul(4).max(1);
            if tick_num.is_multiple_of(regen_interval) {
                step_player_survival_tick(&state, tick_num).await;
            }
            let tick_actions = state.radons.read().await.dispatch_tick(tick_num);
            if !tick_actions.is_empty() {
                state.execute_radon_actions(tick_actions).await;
            }
            if autosave_every_ticks > 0
                && tick_num.is_multiple_of(autosave_every_ticks)
                && let Err(e) = state.save_all().await
            {
                tracing::warn!(error = %e, "periodic world autosave failed");
            }

            let elapsed_us = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
            state.last_tick_us.store(elapsed_us, Ordering::Relaxed);
        }
    })
}

/// Spawns the fixed-rate server tick loop without an external shutdown handle.
#[cfg_attr(not(test), allow(dead_code))]
pub fn spawn_tick_loop(state: Arc<ServerState>) -> tokio::task::JoinHandle<()> {
    let (_tx, rx) = tokio::sync::watch::channel(false);
    spawn_tick_loop_with_shutdown(state, rx)
}

/// Periodically updates survival natural regeneration, saturation drain, and starvation.
pub async fn step_player_survival_tick(state: &ServerState, tick_num: u64) {
    let difficulty = state.config.difficulty;
    let mut updates: Vec<(OutboundSender, f32, i32, f32)> = Vec::new();
    {
        let mut players = state.players.write().await;
        for p in players.values_mut() {
            if p.dead || (p.game_mode != 0 && p.game_mode != 2) {
                continue;
            }
            let mut changed = false;
            if difficulty == 0 {
                // Peaceful: restore health and hunger passively.
                if p.hp < 20.0 || p.food < 20 {
                    p.hp = (p.hp + 1.0).min(20.0);
                    p.food = 20;
                    p.saturation = 5.0;
                    changed = true;
                }
            } else if p.food >= 18 && p.hp < 20.0 {
                p.hp = (p.hp + 1.0).min(20.0);
                if p.saturation > 0.0 {
                    p.saturation = (p.saturation - 1.5).max(0.0);
                } else {
                    p.food = (p.food - 1).max(0);
                }
                changed = true;
            } else if tick_num.is_multiple_of(1200) && p.food > 0 {
                if p.saturation > 0.0 {
                    p.saturation = (p.saturation - 1.0).max(0.0);
                } else {
                    p.food = (p.food - 1).max(0);
                }
                changed = true;
            } else if p.food == 0 {
                let min_hp = match difficulty {
                    1 => 10.0,
                    2 => 1.0,
                    _ => 0.0,
                };
                if p.hp > min_hp {
                    p.hp = (p.hp - 1.0).max(min_hp);
                    if p.hp <= 0.0 {
                        p.dead = true;
                    }
                    changed = true;
                }
            }
            if changed {
                updates.push((p.tx.clone(), p.hp, p.food, p.saturation));
            }
        }
    }
    for (tx, health, food, food_saturation) in updates {
        if let Ok(bytes) = encode_to_bytes(&SetHealth {
            health,
            food: VarInt(food),
            food_saturation,
        }) {
            let _ = tx.try_send(bytes);
        }
    }
}

impl ServerState {
    /// Builds the current active [`crate::world::sim::SimulationArea`] from all connected players' positions and dimensions.
    pub async fn simulation_area(&self) -> crate::world::sim::SimulationArea {
        let players = self.players.read().await;
        let coords: Vec<_> = players.values().map(|p| (p.dimension, p.x, p.z)).collect();
        crate::world::sim::SimulationArea::new(self.config.simulation_distance, coords)
    }
}

/// Advances mob AI/physics and broadcasts entity events for a single server tick.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn step_world_tick(state: &ServerState) {
    let sim_area = state.simulation_area().await;
    step_world_tick_in_area(state, &sim_area).await;
}

/// Advances mob AI/physics within `sim_area` and broadcasts entity events for a single server tick.
pub async fn step_world_tick_in_area(
    state: &ServerState,
    sim_area: &crate::world::sim::SimulationArea,
) {
    let player_targets: Vec<(
        i32,
        crate::world::worldgen::DimensionId,
        f64,
        f64,
        f64,
        bool,
    )> = {
        let players = state.players.read().await;
        players
            .values()
            .map(|p| {
                (
                    p.entity_id,
                    p.dimension,
                    p.x,
                    p.y,
                    p.z,
                    !p.dead && (p.game_mode == 0 || p.game_mode == 2),
                )
            })
            .collect()
    };

    let events = {
        let mut world = state.world.write().await;
        let mut entities = state.entities.write().await;
        entities.tick_in_area(&mut world, &player_targets, sim_area)
    };

    for event in events {
        match event {
            MobTickEvent::Moved {
                dimension,
                entity_id,
                dx,
                dy,
                dz,
                yaw,
                pitch,
                on_ground,
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &MoveEntityPosRot {
                            entity_id: VarInt(entity_id),
                            steps: Vec::new(),
                            dx,
                            dy,
                            dz,
                            yaw,
                            pitch,
                            on_ground,
                        },
                        None,
                    )
                    .await;
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &RotateHead {
                            entity_id: VarInt(entity_id),
                            head_yaw: yaw,
                        },
                        None,
                    )
                    .await;
            }
            MobTickEvent::AttackedPlayer {
                dimension: _,
                player_entity_id,
                damage,
                knockback_x,
                knockback_z,
                yaw,
            } => {
                let scaled_damage = match state.config.difficulty {
                    0 => 0.0,
                    1 => (damage * 0.5 + 1.0).max(1.0),
                    2 => damage,
                    _ => damage * 1.5,
                };
                if scaled_damage > 0.0 {
                    apply_player_damage(
                        state,
                        player_entity_id,
                        scaled_damage,
                        yaw,
                        Some((knockback_x, 0.28, knockback_z)),
                    )
                    .await;
                }
            }
            MobTickEvent::ProjectileMoved {
                dimension,
                entity_id,
                dx,
                dy,
                dz,
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &MoveEntityPosRot {
                            entity_id: VarInt(entity_id),
                            steps: Vec::new(),
                            dx,
                            dy,
                            dz,
                            yaw: 0,
                            pitch: 0,
                            on_ground: false,
                        },
                        None,
                    )
                    .await;
            }
            MobTickEvent::ProjectileDespawned {
                dimension,
                entity_id,
            }
            | MobTickEvent::MobKilled {
                dimension,
                entity_id,
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &RemoveEntities {
                            entity_ids: vec![VarInt(entity_id)],
                        },
                        None,
                    )
                    .await;
            }
            MobTickEvent::MobDamaged {
                dimension,
                entity_id,
                knockback_vx,
                knockback_vy,
                knockback_vz,
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &HurtAnimation {
                            entity_id: VarInt(entity_id),
                            yaw: 0.0,
                        },
                        None,
                    )
                    .await;
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &SetEntityMotion {
                            entity_id: VarInt(entity_id),
                            velocity: LpVec3::new(
                                f64::from(knockback_vx) / 8000.0,
                                f64::from(knockback_vy) / 8000.0,
                                f64::from(knockback_vz) / 8000.0,
                            ),
                        },
                        None,
                    )
                    .await;
            }
            MobTickEvent::ItemMoved {
                dimension,
                entity_id,
                dx,
                dy,
                dz,
                on_ground,
                ..
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &MoveEntityPosRot {
                            entity_id: VarInt(entity_id),
                            steps: Vec::new(),
                            dx,
                            dy,
                            dz,
                            yaw: 0,
                            pitch: 0,
                            on_ground,
                        },
                        None,
                    )
                    .await;
            }
            MobTickEvent::ItemMerged {
                dimension,
                kept_entity_id,
                removed_entity_id,
                stack,
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &RemoveEntities {
                            entity_ids: vec![VarInt(removed_entity_id)],
                        },
                        None,
                    )
                    .await;
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &rad_core::packet::play::SetEntityData {
                            entity_id: VarInt(kept_entity_id),
                            entries: vec![rad_core::packet::play::MetadataEntry {
                                index: 8,
                                value: rad_core::packet::play::MetadataValue::ItemStack(stack),
                            }],
                        },
                        None,
                    )
                    .await;
            }
            MobTickEvent::ItemDespawned {
                dimension,
                entity_id,
            } => {
                state
                    .broadcast_in_dimension_except(
                        dimension,
                        &RemoveEntities {
                            entity_ids: vec![VarInt(entity_id)],
                        },
                        None,
                    )
                    .await;
            }
        }
    }

    // Atomic player pickup pass for ready dropped items (pickup_delay_ticks == 0).
    let pickup_players: Vec<(
        i32,
        crate::world::worldgen::DimensionId,
        f64,
        f64,
        f64,
        OutboundSender,
    )> = {
        let players = state.players.read().await;
        players
            .values()
            .filter(|p| !p.dead && p.game_mode != 3)
            .map(|p| (p.entity_id, p.dimension, p.x, p.y, p.z, p.tx.clone()))
            .collect()
    };

    if !pickup_players.is_empty() {
        enum PickupOutcome {
            Removed {
                dim: crate::world::worldgen::DimensionId,
                item_eid: i32,
                player_eid: i32,
                count: i32,
                player_tx: OutboundSender,
                inv_packet: rad_core::packet::play::ContainerSetContentStacks,
                saved_inv: Vec<SavedSlot>,
            },
            Partial {
                dim: crate::world::worldgen::DimensionId,
                item_eid: i32,
                player_eid: i32,
                count: i32,
                remaining: rad_core::packet::play::ItemStack,
                player_tx: OutboundSender,
                inv_packet: rad_core::packet::play::ContainerSetContentStacks,
                saved_inv: Vec<SavedSlot>,
            },
        }

        let mut outcomes = Vec::new();
        {
            let live_invs = state.live_inventories.read().await;
            let mut entities = state.entities.write().await;
            let item_ids: Vec<i32> = entities
                .all_dropped_items()
                .filter(|item| item.pickup_delay_ticks == 0)
                .map(|item| item.entity_id)
                .collect();

            for iid in item_ids {
                let Some(item_pos) = entities
                    .get_dropped_item_mut(iid)
                    .map(|it| (it.dimension, it.x, it.y, it.z))
                else {
                    continue;
                };
                for &(pid, pdim, px, py, pz, ref ptx) in &pickup_players {
                    if pdim != item_pos.0 {
                        continue;
                    }
                    if (item_pos.1 - px).abs() > 1.25
                        || (item_pos.3 - pz).abs() > 1.25
                        || item_pos.2 < py - 0.5
                        || item_pos.2 > py + 2.0
                    {
                        continue;
                    }
                    let Some(inv_lock) = live_invs.get(&pid) else {
                        continue;
                    };
                    let Ok(mut inv) = inv_lock.lock() else {
                        continue;
                    };
                    let Some(item) = entities.get_dropped_item_mut(iid) else {
                        break;
                    };
                    let picked = inv.insert_item_stack(&mut item.stack);
                    if picked <= 0 {
                        continue;
                    }
                    let inv_packet = inv.to_container_set_content_stacks();
                    let saved_inv: Vec<SavedSlot> = inv
                        .slots
                        .iter()
                        .zip(inv.slot_components.iter())
                        .map(|(s, c)| SavedSlot {
                            count: s.count,
                            item_id: s.item_id,
                            components: c.clone(),
                        })
                        .collect();
                    if item.stack.count <= 0 {
                        entities.remove_dropped_item(iid);
                        outcomes.push(PickupOutcome::Removed {
                            dim: pdim,
                            item_eid: iid,
                            player_eid: pid,
                            count: picked,
                            player_tx: ptx.clone(),
                            inv_packet,
                            saved_inv,
                        });
                    } else {
                        let remaining = item.stack.clone();
                        outcomes.push(PickupOutcome::Partial {
                            dim: pdim,
                            item_eid: iid,
                            player_eid: pid,
                            count: picked,
                            remaining,
                            player_tx: ptx.clone(),
                            inv_packet,
                            saved_inv,
                        });
                    }
                    break;
                }
            }
        }

        if !outcomes.is_empty() {
            let mut players = state.players.write().await;
            for outcome in &outcomes {
                let (pid, saved) = match outcome {
                    PickupOutcome::Removed {
                        player_eid,
                        saved_inv,
                        ..
                    }
                    | PickupOutcome::Partial {
                        player_eid,
                        saved_inv,
                        ..
                    } => (*player_eid, saved_inv),
                };
                if let Some(p) = players.get_mut(&pid) {
                    p.inventory = saved.clone();
                }
            }
            drop(players);

            for outcome in outcomes {
                match outcome {
                    PickupOutcome::Removed {
                        dim,
                        item_eid,
                        player_eid,
                        count,
                        player_tx,
                        inv_packet,
                        ..
                    } => {
                        state
                            .broadcast_in_dimension_except(
                                dim,
                                &rad_core::packet::play::TakeItemEntity {
                                    collected_entity_id: VarInt(item_eid),
                                    collector_entity_id: VarInt(player_eid),
                                    pickup_item_count: VarInt(count),
                                },
                                None,
                            )
                            .await;
                        state
                            .broadcast_in_dimension_except(
                                dim,
                                &RemoveEntities {
                                    entity_ids: vec![VarInt(item_eid)],
                                },
                                None,
                            )
                            .await;
                        if let Ok(bytes) = encode_to_bytes(&inv_packet) {
                            let _ = player_tx.try_send(bytes);
                        }
                    }
                    PickupOutcome::Partial {
                        dim,
                        item_eid,
                        player_eid,
                        count,
                        remaining,
                        player_tx,
                        inv_packet,
                        ..
                    } => {
                        state
                            .broadcast_in_dimension_except(
                                dim,
                                &rad_core::packet::play::TakeItemEntity {
                                    collected_entity_id: VarInt(item_eid),
                                    collector_entity_id: VarInt(player_eid),
                                    pickup_item_count: VarInt(count),
                                },
                                None,
                            )
                            .await;
                        state
                            .broadcast_in_dimension_except(
                                dim,
                                &rad_core::packet::play::SetEntityData {
                                    entity_id: VarInt(item_eid),
                                    entries: vec![rad_core::packet::play::MetadataEntry {
                                        index: 8,
                                        value: rad_core::packet::play::MetadataValue::ItemStack(
                                            remaining,
                                        ),
                                    }],
                                },
                                None,
                            )
                            .await;
                        if let Ok(bytes) = encode_to_bytes(&inv_packet) {
                            let _ = player_tx.try_send(bytes);
                        }
                    }
                }
            }
        }
    }
}

/// Spawns a dropped item entity in `dimension` and broadcasts `SpawnEntity` + `SetEntityData`
/// to all players in that dimension. Returns `Some(entity_id)` on success, `None` if invalid/empty.
pub async fn spawn_and_broadcast_dropped_item(
    state: &ServerState,
    dimension: crate::world::worldgen::DimensionId,
    pos: [f64; 3],
    vel: [f64; 3],
    stack: rad_core::packet::play::ItemStack,
    pickup_delay_ticks: u16,
) -> Option<i32> {
    if stack.item_id <= 0 || stack.count <= 0 {
        return None;
    }
    let eid = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
    let (spawn_pkt, meta_pkt) = {
        let mut entities = state.entities.write().await;
        let item =
            entities.spawn_dropped_item_in(dimension, eid, pos, vel, stack, pickup_delay_ticks)?;
        (
            item.to_spawn_packet_with(&state.registries.entities),
            item.to_metadata_packet(),
        )
    };
    state
        .broadcast_in_dimension_except(dimension, &spawn_pkt, None)
        .await;
    state
        .broadcast_in_dimension_except(dimension, &meta_pkt, None)
        .await;
    Some(eid)
}

pub(super) async fn apply_player_damage(
    state: &ServerState,
    entity_id: i32,
    amount: f32,
    attack_yaw: f32,
    knockback: Option<(f64, f64, f64)>,
) {
    if amount <= 0.0 {
        return;
    }
    let damage_ctx = {
        let players = state.players.read().await;
        players.get(&entity_id).and_then(|p| {
            if p.dead || (p.game_mode != 0 && p.game_mode != 2) {
                None
            } else {
                Some(ServerPlayerContext {
                    entity_id: p.entity_id,
                    username: p.username.clone(),
                    x: p.x,
                    y: p.y,
                    z: p.z,
                    hp: p.hp,
                })
            }
        })
    };
    if let Some(ref ctx) = damage_ctx {
        let (res, actions) = state.radons.read().await.dispatch_damage(ctx, amount);
        state.execute_radon_actions(actions).await;
        if res == ServerEventResult::Cancel {
            return;
        }
    }
    let update = {
        let mut players = state.players.write().await;
        let Some(p) = players.get_mut(&entity_id) else {
            return;
        };
        if p.dead || (p.game_mode != 0 && p.game_mode != 2) {
            return;
        }
        p.hp = (p.hp - amount).max(0.0);
        if p.hp <= 0.0 {
            p.dead = true;
        }
        Some((
            p.hp,
            p.food,
            p.saturation,
            p.tx.clone(),
            p.username.clone(),
            p.dead,
            p.dimension,
        ))
    };

    let Some((hp, food, saturation, tx, username, died, player_dim)) = update else {
        return;
    };

    if let Ok(bytes) = encode_to_bytes(&SetHealth {
        health: hp,
        food: VarInt(food),
        food_saturation: saturation,
    }) {
        let _ = tx.try_send(bytes);
    }

    state
        .broadcast_in_dimension_except(
            player_dim,
            &HurtAnimation {
                entity_id: VarInt(entity_id),
                yaw: attack_yaw,
            },
            None,
        )
        .await;

    if let Some((kx, ky, kz)) = knockback {
        state
            .broadcast_in_dimension_except(
                player_dim,
                &SetEntityMotion {
                    entity_id: VarInt(entity_id),
                    velocity: LpVec3::new(kx, ky, kz),
                },
                None,
            )
            .await;
    }

    if died {
        state.broadcast_chat(format!("{username} died.")).await;
    }
}

/// Checks if any active player is standing in a Nether or End portal and transitions dimensions.
pub async fn step_player_portals(state: &ServerState) {
    use crate::registries::state_in_entry;
    use crate::world::worldgen::DimensionId;

    let nether_portal = state.registries.block_by_name("minecraft:nether_portal");
    let end_portal = state.registries.block_by_name("minecraft:end_portal");
    if nether_portal.is_none() && end_portal.is_none() {
        return;
    }

    let snapshot: Vec<(i32, DimensionId, f64, f64, f64, u8, bool)> = {
        let players = state.players.read().await;
        players
            .iter()
            .map(|(&eid, p)| (eid, p.dimension, p.x, p.y, p.z, p.game_mode, p.dead))
            .collect()
    };

    type PortalTransition = (
        i32,
        DimensionId,
        (f64, f64, f64),
        crate::world::portals::PortalBlockUpdates,
    );
    let mut transitions: Vec<PortalTransition> = Vec::new();

    {
        let mut world = state.world.write().await;
        let mut pstate = state.portal_state.write().await;

        // Prune disconnected players.
        pstate.retain(|eid, _| snapshot.iter().any(|(id, ..)| id == eid));

        for (eid, dim, x, y, z, mode, dead) in snapshot {
            let entry = pstate.entry(eid).or_insert((0, 0));
            if entry.1 > 0 {
                entry.1 -= 1;
            }
            if dead {
                entry.0 = 0;
                continue;
            }
            let bx = x.floor() as i32;
            let by = y.floor() as i32;
            let bz = z.floor() as i32;
            let feet_block = world.get_block_in(dim, bx, by, bz);

            if end_portal
                .as_ref()
                .is_some_and(|ep| state_in_entry(ep, feet_block))
            {
                if entry.1 == 0 {
                    entry.0 = 0;
                    entry.1 = 40;
                    if dim == DimensionId::End {
                        let spawn = (
                            state.config.spawn_x,
                            state.config.spawn_y,
                            state.config.spawn_z,
                        );
                        transitions.push((eid, DimensionId::Overworld, spawn, Vec::new()));
                    } else {
                        let updates = crate::world::portals::ensure_end_spawn_platform(&mut world);
                        transitions.push((eid, DimensionId::End, (100.5, 49.0, 0.5), updates));
                    }
                }
                continue;
            }

            if nether_portal
                .as_ref()
                .is_some_and(|np| state_in_entry(np, feet_block))
                && matches!(dim, DimensionId::Overworld | DimensionId::Nether)
            {
                if entry.1 == 0 {
                    let threshold = if mode == 1 { 1 } else { 80 };
                    entry.0 = entry.0.saturating_add(1);
                    if entry.0 >= threshold {
                        entry.0 = 0;
                        entry.1 = 80;
                        let target_dim = if dim == DimensionId::Overworld {
                            DimensionId::Nether
                        } else {
                            DimensionId::Overworld
                        };
                        let (tx, tz) = if dim == DimensionId::Overworld {
                            (bx.div_euclid(8), bz.div_euclid(8))
                        } else {
                            (bx.saturating_mul(8), bz.saturating_mul(8))
                        };
                        let (target_pos, updates) =
                            crate::world::portals::find_or_create_nether_portal(
                                &mut world, target_dim, tx, by, tz,
                            );
                        transitions.push((eid, target_dim, target_pos, updates));
                    }
                }
            } else {
                entry.0 = 0;
            }
        }
    }

    for (eid, target_dim, target_pos, updates) in transitions {
        state
            .switch_player_dimension_at(eid, target_dim, Some(target_pos))
            .await;
        for (ux, uy, uz, st) in updates {
            state
                .broadcast_in_dimension_except(
                    target_dim,
                    &BlockUpdate {
                        pos: BlockPos::new(ux, uy, uz),
                        block_state: VarInt(i32::from(st)),
                    },
                    None,
                )
                .await;
        }
    }
}
