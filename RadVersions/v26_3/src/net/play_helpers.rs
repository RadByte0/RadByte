//! Play-state movement validation, chunk streaming sync, inventory sync, and pick-item helpers.

use super::*;

pub(super) async fn sync_player_inventory(
    state: &ServerState,
    entity_id: i32,
    inventory: &std::sync::Mutex<PlayerInventory>,
    tx: &OutboundSender,
) {
    let (saved_slots, selected_hotbar, pkt) = {
        let inv_guard = inventory.lock().unwrap();
        let saved_slots = inv_guard
            .slots
            .iter()
            .zip(inv_guard.slot_components.iter())
            .map(|(s, c)| SavedSlot {
                count: s.count,
                item_id: s.item_id,
                components: c.clone(),
            })
            .collect();
        let pkt = inv_guard.to_container_set_content_stacks();
        (saved_slots, inv_guard.selected_hotbar, pkt)
    };
    if let Some(p) = state.players.write().await.get_mut(&entity_id) {
        p.inventory = saved_slots;
        p.selected_hotbar = selected_hotbar;
    }
    if let Ok(bytes) = encode_to_bytes(&pkt) {
        let _ = tx.try_send(bytes);
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_player_move(
    state: &ServerState,
    entity_id: i32,
    tx: &OutboundSender,
    loaded_chunks: &mut HashSet<(i32, i32)>,
    cur_x: &mut f64,
    cur_y: &mut f64,
    cur_z: &mut f64,
    cur_yaw: &mut f32,
    cur_pitch: &mut f32,
    cur_cx: &mut i32,
    cur_cz: &mut i32,
    new_pos: Option<(f64, f64, f64)>,
    new_rot: Option<(f32, f32)>,
    elapsed_secs: f64,
) {
    // Ignore stale client movement packets while a server-initiated teleport is unconfirmed.
    if let Some(p) = state.players.read().await.get(&entity_id)
        && p.pending_teleport_id.is_some()
    {
        return;
    }

    let (nx, ny, nz) = new_pos.unwrap_or((*cur_x, *cur_y, *cur_z));
    let (nyaw, npitch) = new_rot.unwrap_or((*cur_yaw, *cur_pitch));

    let (player_mode, cur_dim) = state.players.read().await.get(&entity_id).map_or(
        (
            state.config.game_mode,
            crate::world::worldgen::DimensionId::Overworld,
        ),
        |p| (p.game_mode, p.dimension),
    );

    let on_ground = {
        let mut world = state.world.write().await;
        if new_pos.is_some() {
            let max_world = f64::from(state.config.max_world_size);
            let within_world = nx.abs() <= max_world && nz.abs() <= max_world;
            let allow_flight = player_mode == 1 || player_mode == 3;
            let noclip = player_mode == 3;
            let validation = crate::world::physics::validate_player_move_with_pose_in_with(
                &mut world,
                cur_dim,
                (*cur_x, *cur_y, *cur_z),
                (nx, ny, nz),
                elapsed_secs,
                allow_flight,
                noclip,
                1.8,
                state.registries.provider().as_ref(),
            );
            if !within_world || validation != MoveValidation::Accepted {
                if let Some(p) = state.players.write().await.get_mut(&entity_id) {
                    p.pending_teleport_id = Some(99);
                }
                if let Ok(bytes) = encode_to_bytes(&SynchronizePlayerPosition {
                    teleport_id: VarInt(99),
                    x: *cur_x,
                    y: *cur_y,
                    z: *cur_z,
                    vx: 0.0,
                    vy: 0.0,
                    vz: 0.0,
                    yaw: *cur_yaw,
                    pitch: *cur_pitch,
                    flags: 0,
                }) {
                    let _ = tx.try_send(bytes);
                }
                return;
            }
        }
        crate::world::physics::is_player_on_ground_in_with(
            &mut world,
            cur_dim,
            nx,
            ny,
            nz,
            state.registries.provider().as_ref(),
        )
    };

    let dx_raw = ((nx * 4096.0) - (*cur_x * 4096.0)).round() as i64;
    let dy_raw = ((ny * 4096.0) - (*cur_y * 4096.0)).round() as i64;
    let dz_raw = ((nz * 4096.0) - (*cur_z * 4096.0)).round() as i64;
    let delta_i16 = (
        i16::try_from(dx_raw).ok(),
        i16::try_from(dy_raw).ok(),
        i16::try_from(dz_raw).ok(),
    );

    *cur_x = nx;
    *cur_y = ny;
    *cur_z = nz;
    *cur_yaw = nyaw;
    *cur_pitch = npitch;

    let mut fall_damage = 0.0f32;
    if let Some(session) = state.players.write().await.get_mut(&entity_id) {
        let was_on_ground = session.on_ground;
        session.x = nx;
        session.y = ny;
        session.z = nz;
        session.yaw = nyaw;
        session.pitch = npitch;
        session.on_ground = on_ground;

        if !on_ground {
            let peak = session.fall_start_y.map_or(ny, |prev| prev.max(ny));
            session.fall_start_y = Some(peak);
        } else {
            if !was_on_ground && let Some(start_y) = session.fall_start_y.take() {
                fall_damage = compute_fall_damage(start_y - ny);
            }
            session.fall_start_y = None;
        }
    }

    if fall_damage > 0.0 {
        apply_player_damage(state, entity_id, fall_damage, nyaw, None).await;
    }

    let cur_dim = state
        .players
        .read()
        .await
        .get(&entity_id)
        .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
            p.dimension
        });
    let yaw_byte = deg_to_angle(nyaw);
    let pitch_byte = deg_to_angle(npitch);
    if let (Some(dx), Some(dy), Some(dz)) = delta_i16 {
        state
            .broadcast_in_dimension_except(
                cur_dim,
                &MoveEntityPosRot {
                    entity_id: VarInt(entity_id),
                    steps: Vec::new(),
                    dx,
                    dy,
                    dz,
                    yaw: yaw_byte,
                    pitch: pitch_byte,
                    on_ground,
                },
                Some(entity_id),
            )
            .await;
    } else {
        state
            .broadcast_in_dimension_except(
                cur_dim,
                &rad_core::packet::play::TeleportEntity {
                    entity_id: VarInt(entity_id),
                    x: nx,
                    y: ny,
                    z: nz,
                    vx: 0.0,
                    vy: 0.0,
                    vz: 0.0,
                    yaw: nyaw,
                    pitch: npitch,
                    flags: 0,
                    on_ground,
                },
                Some(entity_id),
            )
            .await;
    }
    if new_rot.is_some() {
        state
            .broadcast_in_dimension_except(
                cur_dim,
                &RotateHead {
                    entity_id: VarInt(entity_id),
                    head_yaw: yaw_byte,
                },
                Some(entity_id),
            )
            .await;
    }

    let new_cx = (nx / 16.0).floor() as i32;
    let new_cz = (nz / 16.0).floor() as i32;
    sync_player_chunks(
        state,
        tx,
        cur_dim,
        loaded_chunks,
        cur_cx,
        cur_cz,
        new_cx,
        new_cz,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn sync_player_chunks(
    state: &ServerState,
    tx: &OutboundSender,
    dim: crate::world::worldgen::DimensionId,
    loaded_chunks: &mut HashSet<(i32, i32)>,
    cur_cx: &mut i32,
    cur_cz: &mut i32,
    new_cx: i32,
    new_cz: i32,
) {
    if new_cx != *cur_cx || new_cz != *cur_cz {
        *cur_cx = new_cx;
        *cur_cz = new_cz;
        let desired: HashSet<(i32, i32)> =
            chunks_in_radius(new_cx, new_cz, state.config.view_distance)
                .into_iter()
                .collect();
        let mut batch = BytesMut::with_capacity(4096);
        let _ = encode_packet(
            &SetChunkCacheCenter {
                chunk_x: VarInt(new_cx),
                chunk_z: VarInt(new_cz),
            },
            &mut batch,
        );
        for &(cx, cz) in loaded_chunks.difference(&desired) {
            let _ = encode_packet(
                &UnloadChunk {
                    chunk_x: cx,
                    chunk_z: cz,
                },
                &mut batch,
            );
        }
        if !tx.send_wait(batch.freeze()).await {
            return;
        }
        let to_load: Vec<_> = chunks_in_radius(new_cx, new_cz, state.config.view_distance)
            .into_iter()
            .filter(|pos| !loaded_chunks.contains(pos))
            .collect();
        for group in to_load.chunks(4) {
            let mut batch = BytesMut::new();
            let mut world = state.world.write().await;
            for &(cx, cz) in group {
                let _ = encode_packet(&world.chunk_packet_in(dim, cx, cz), &mut batch);
            }
            drop(world);
            if !tx.send_wait(batch.freeze()).await {
                return;
            }
        }
        *loaded_chunks = desired;
    }
}

pub(super) fn is_spawn_protected(
    cfg: &ServerConfig,
    dim: crate::world::worldgen::DimensionId,
    pos: BlockPos,
) -> bool {
    if cfg.spawn_protection <= 0 || dim != crate::world::worldgen::DimensionId::Overworld {
        return false;
    }
    let sx = cfg.spawn_x.floor() as i32;
    let sz = cfg.spawn_z.floor() as i32;
    (pos.x - sx).abs().max((pos.z - sz).abs()) <= cfg.spawn_protection
}

pub(super) fn block_state_to_pick_item_id(
    block_state: u16,
    regs: &crate::registries::Version26_3Registries,
) -> Option<i32> {
    if block_state == 0 {
        return None;
    }
    let block = regs.block_by_state(block_state)?;
    let item_name = match block.name.as_str() {
        "minecraft:redstone_wire" => "minecraft:redstone",
        "minecraft:wall_torch" => "minecraft:torch",
        "minecraft:soul_wall_torch" => "minecraft:soul_torch",
        "minecraft:redstone_wall_torch" => "minecraft:redstone_torch",
        "minecraft:water" | "minecraft:flowing_water" => "minecraft:water_bucket",
        "minecraft:lava" | "minecraft:flowing_lava" => "minecraft:lava_bucket",
        other => other,
    };
    regs.item_by_name(item_name)
}
