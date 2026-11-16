//! Play-state packet loop, movement validation, and world interactions.

mod containers;
mod mining;
mod use_item_on;

use super::play_helpers::{
    block_state_to_pick_item_id, is_spawn_protected, sync_player_chunks, sync_player_inventory,
};
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_play(
    stream: TcpStream,
    mut read_buf: BytesMut,
    peer_addr: SocketAddr,
    player_uuid: Uuid,
    username: String,
    properties: Vec<GameProfileProperty>,
    client_view_distance: Option<i32>,
    state: Arc<ServerState>,
) {
    let cfg = &state.config;
    let mut effective_view_distance = client_view_distance
        .unwrap_or(cfg.view_distance)
        .clamp(2, cfg.view_distance);
    let entity_id = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
    let (mut reader, mut writer) = stream.into_split();
    let (tx, mut rx, mut cancel_rx) = outbound_channel(4096, MAX_PLAYER_OUTBOUND_BYTES);

    let mut writer_cancel = cancel_rx.clone();
    let writer_tx = tx.clone();
    let writer_task = tokio::spawn(async move {
        loop {
            let maybe_bytes = tokio::select! {
                biased;
                _ = async { let _ = writer_cancel.wait_for(|&c| c).await; } => None,
                b = rx.recv() => b,
            };
            let Some(bytes) = maybe_bytes else {
                break;
            };
            match tokio::time::timeout(Duration::from_secs(10), writer.write_all(&bytes)).await {
                Ok(Ok(())) => {}
                _ => {
                    writer_tx.disconnect();
                    break;
                }
            }
        }
    });

    let saved = state.load_player_data(player_uuid).await;
    if let Some(ref s) = saved {
        state
            .player_dimensions
            .write()
            .await
            .insert(player_uuid, s.dimension);
        state
            .player_stats
            .write()
            .await
            .insert(player_uuid, s.stats);
    }
    let init_x = saved.as_ref().map_or(cfg.spawn_x, |s| s.x);
    let init_y = saved.as_ref().map_or(cfg.spawn_y, |s| s.y);
    let init_z = saved.as_ref().map_or(cfg.spawn_z, |s| s.z);
    let init_yaw = saved.as_ref().map_or(0.0, |s| s.yaw);
    let init_pitch = saved.as_ref().map_or(0.0, |s| s.pitch);
    let init_hp = saved.as_ref().map_or(20.0, |s| s.hp);
    let init_food = saved.as_ref().map_or(20, |s| s.food);
    let init_sat = saved.as_ref().map_or(5.0, |s| s.saturation);
    let init_mode = if cfg.force_gamemode {
        cfg.game_mode
    } else {
        saved.as_ref().map_or(cfg.game_mode, |s| s.game_mode)
    };
    let init_dim = saved
        .as_ref()
        .map_or(crate::world::worldgen::DimensionId::Overworld, |s| {
            s.dimension
        });

    let spawn_cx = (init_x / 16.0).floor() as i32;
    let spawn_cz = (init_z / 16.0).floor() as i32;

    // 1. Initial Play login + chunk streaming + spawn
    let mut init_batch = BytesMut::with_capacity(8192);
    let play_login = PlayLogin {
        entity_id,
        is_hardcore: cfg.hardcore,
        dimension_names: vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ],
        max_players: VarInt(cfg.max_players),
        view_distance: VarInt(effective_view_distance),
        simulation_distance: VarInt(cfg.simulation_distance),
        reduced_debug_info: false,
        enable_respawn_screen: true,
        do_limited_crafting: false,
        dimension_type: VarInt(init_dim.type_id()),
        dimension_name: init_dim.resource_name().into(),
        hashed_seed: 0,
        game_mode: init_mode,
        previous_game_mode: -1,
        is_debug: false,
        is_flat: !cfg.world_type.eq_ignore_ascii_case("seeded")
            && !state
                .world
                .read()
                .await
                .world_dir()
                .is_some_and(|d| d.join("dimensions").is_dir()),
        portal_cooldown: VarInt(0),
        sea_level: VarInt(64),
        online_mode: false,
        enforces_secure_chat: false,
    };
    if encode_packet(&play_login, &mut init_batch).is_err() {
        writer_task.abort();
        return;
    }
    let _ = encode_packet(
        &SetChunkCacheRadius {
            view_distance: VarInt(effective_view_distance),
        },
        &mut init_batch,
    );
    let _ = encode_packet(
        &SetChunkCacheCenter {
            chunk_x: VarInt(spawn_cx),
            chunk_z: VarInt(spawn_cz),
        },
        &mut init_batch,
    );
    let _ = encode_packet(
        &SetDefaultSpawnPosition {
            dimension_name: init_dim.resource_name().into(),
            location: BlockPos::new(cfg.spawn_x as i32, cfg.spawn_y as i32, cfg.spawn_z as i32),
            yaw: 0.0,
            pitch: 0.0,
        },
        &mut init_batch,
    );
    let (game_time, day_time) = state.sync_level_time().await;
    let _ = encode_packet(
        &rad_core::packet::play::SetTime::overworld(game_time, day_time, 1.0),
        &mut init_batch,
    );
    let _ = encode_packet(
        &GameEvent {
            event: GameEvent::START_WAITING_FOR_CHUNKS,
            value: 0.0,
        },
        &mut init_batch,
    );

    let mut inventory = PlayerInventory::new_with_registries(Arc::clone(&state.registries));
    let mut open_chest_pos: Option<(crate::world::worldgen::DimensionId, i32, i32, i32)> = None;
    let mut open_furnace_pos: Option<(crate::world::worldgen::DimensionId, i32, i32, i32)> = None;
    let mut last_projectile_tick: Option<u64> = None;
    inventory.custom_recipes = state.recipes.read().await.clone();
    if let Some(ref s) = saved {
        if !s.inventory.is_empty() {
            for (idx, saved_slot) in s.inventory.iter().enumerate().take(inventory.slots.len()) {
                inventory.slots[idx] = rad_core::packet::play::SimpleSlot {
                    count: saved_slot.count,
                    item_id: saved_slot.item_id,
                };
                inventory.slot_components[idx] = saved_slot.components.clone();
            }
        }
        inventory.set_selected_hotbar(i16::from(s.selected_hotbar));
    }
    let inventory = Arc::new(std::sync::Mutex::new(inventory));
    state
        .live_inventories
        .write()
        .await
        .insert(entity_id, Arc::clone(&inventory));
    let perm_level = state.permission_level(&username, Some(player_uuid)).await;
    let _ = encode_packet(
        &PermissionEvent {
            entity_id,
            level: perm_level,
        },
        &mut init_batch,
    );
    let _ = encode_packet(
        &commands::build_declare_commands_for_level(perm_level),
        &mut init_batch,
    );
    {
        let inv_guard = inventory.lock().unwrap();
        let _ = encode_packet(
            &inv_guard.to_container_set_content_stacks(),
            &mut init_batch,
        );
        let _ = encode_packet(
            &SetHeldSlot {
                slot: VarInt(i32::from(inv_guard.selected_hotbar)),
            },
            &mut init_batch,
        );
    }
    let _ = encode_packet(
        &SetHealth {
            health: init_hp,
            food: VarInt(init_food),
            food_saturation: init_sat,
        },
        &mut init_batch,
    );
    let _ = encode_packet(
        &SynchronizePlayerPosition {
            teleport_id: VarInt(1),
            x: init_x,
            y: init_y,
            z: init_z,
            vx: 0.0,
            vy: 0.0,
            vz: 0.0,
            yaw: init_yaw,
            pitch: init_pitch,
            flags: 0,
        },
        &mut init_batch,
    );

    let _ = tx.try_send(init_batch.freeze());

    let mut loaded_chunks: HashSet<(i32, i32)> = HashSet::new();
    let mut pending_outer_chunks: std::collections::VecDeque<(i32, i32)> =
        std::collections::VecDeque::new();
    {
        let immediate_r = effective_view_distance.min(4);
        let all_coords = chunks_in_radius(spawn_cx, spawn_cz, effective_view_distance);
        let mut immediate_coords = Vec::new();
        for (cx, cz) in all_coords {
            if (cx - spawn_cx).abs().max((cz - spawn_cz).abs()) <= immediate_r {
                immediate_coords.push((cx, cz));
            } else {
                pending_outer_chunks.push_back((cx, cz));
            }
        }
        state
            .ensure_chunks_extras_loaded(init_dim, &immediate_coords)
            .await;
        for chunk_group in immediate_coords.chunks(16) {
            let mut chunk_batch = BytesMut::with_capacity(16 * 4096);
            let mut world = state.world.write().await;
            for &(cx, cz) in chunk_group {
                let chunk_pkt = world.chunk_packet_in(init_dim, cx, cz);
                if encode_packet(&chunk_pkt, &mut chunk_batch).is_ok() {
                    loaded_chunks.insert((cx, cz));
                }
            }
            drop(world);
            if !chunk_batch.is_empty() && !tx.send_wait(chunk_batch.freeze()).await {
                info!(
                    %peer_addr,
                    %username,
                    reason = "outbound_closed_during_initial_chunks",
                    "player disconnected before completing join"
                );
                writer_task.abort();
                return;
            }
        }
    }

    let mut post_batch = BytesMut::with_capacity(512);
    if let Some(ref msg) = cfg.welcome_message {
        let _ = encode_packet(
            &SystemChatMessage {
                content: styled_text(msg, "aqua"),
                overlay: false,
            },
            &mut post_batch,
        );
    }
    if !tx.send_wait(post_batch.freeze()).await {
        info!(
            %peer_addr,
            %username,
            reason = "outbound_closed_before_spawn",
            "player disconnected before completing join"
        );
        writer_task.abort();
        return;
    }

    // 2. Register player & sync tab-list / existing players before join hook
    let (inv_snapshot, selected_hotbar) = {
        let inv_guard = inventory.lock().unwrap();
        let snap = inv_guard
            .slots
            .iter()
            .zip(inv_guard.slot_components.iter())
            .map(|(s, c)| SavedSlot {
                count: s.count,
                item_id: s.item_id,
                components: c.clone(),
            })
            .collect();
        (snap, inv_guard.selected_hotbar)
    };
    let (chest_sync_tx, mut chest_sync_rx) = tokio::sync::mpsc::unbounded_channel();
    let session = PlayerSession {
        entity_id,
        uuid: player_uuid,
        username: username.clone(),
        properties: properties.clone(),
        x: init_x,
        y: init_y,
        z: init_z,
        yaw: init_yaw,
        pitch: init_pitch,
        hp: init_hp,
        food: init_food,
        saturation: init_sat,
        on_ground: true,
        fall_start_y: None,
        dead: init_hp <= 0.0,
        game_mode: init_mode,
        dimension: init_dim,
        inventory: inv_snapshot,
        selected_hotbar,
        open_chest: None,
        chest_sync_tx: Some(chest_sync_tx),
        pending_teleport_id: Some(1),
        pending_keep_alive: None,
        last_activity: Instant::now(),
        tx: tx.clone(),
    };

    {
        let mut ent_batch = BytesMut::with_capacity(4096);
        let max_chunk_dist = cfg.view_distance.max(1);
        let entities = state.entities.read().await;
        for mob in entities.mobs_in_dimension(init_dim) {
            let mcx = (mob.x.floor() as i32).div_euclid(16);
            let mcz = (mob.z.floor() as i32).div_euclid(16);
            if (mcx - spawn_cx).abs() <= max_chunk_dist && (mcz - spawn_cz).abs() <= max_chunk_dist
            {
                let _ = encode_packet(
                    &mob.to_spawn_packet_with(&state.registries.entities),
                    &mut ent_batch,
                );
            }
        }
        for item in entities.dropped_items_in_dimension(init_dim) {
            let icx = (item.x.floor() as i32).div_euclid(16);
            let icz = (item.z.floor() as i32).div_euclid(16);
            if (icx - spawn_cx).abs() <= max_chunk_dist && (icz - spawn_cz).abs() <= max_chunk_dist
            {
                let _ = encode_packet(
                    &item.to_spawn_packet_with(&state.registries.entities),
                    &mut ent_batch,
                );
                let _ = encode_packet(&item.to_metadata_packet(), &mut ent_batch);
            }
        }
        drop(entities);
        if !ent_batch.is_empty() && !tx.send_wait(ent_batch.freeze()).await {
            writer_task.abort();
            return;
        }
    }

    {
        let mut players = state.players.write().await;
        // Send existing players to the newcomer + newcomer to existing players
        let mut all_entries = Vec::with_capacity(players.len() + 1);
        for other in players.values() {
            all_entries.push(PlayerInfoAddEntry {
                uuid: other.uuid,
                name: other.username.clone(),
                properties: other.properties.clone(),
                game_mode: VarInt(i32::from(other.game_mode)),
                listed: true,
                latency: VarInt(0),
            });
            if other.dimension == init_dim
                && let Ok(spawn_bytes) = encode_to_bytes(&SpawnEntity {
                    entity_id: VarInt(other.entity_id),
                    uuid: other.uuid,
                    entity_type: VarInt(state.registries.entities.player),
                    x: other.x,
                    y: other.y,
                    z: other.z,
                    velocity: LpVec3::ZERO,
                    pitch: deg_to_angle(other.pitch),
                    yaw: deg_to_angle(other.yaw),
                    head_yaw: deg_to_angle(other.yaw),
                    data: VarInt(0),
                })
            {
                let _ = tx.try_send(spawn_bytes);
            }
        }
        all_entries.push(PlayerInfoAddEntry {
            uuid: player_uuid,
            name: username.clone(),
            properties: properties.clone(),
            game_mode: VarInt(i32::from(init_mode)),
            listed: true,
            latency: VarInt(0),
        });
        if let Ok(info_bytes) = encode_to_bytes(&PlayerInfoUpdate {
            entries: all_entries,
        }) {
            let _ = tx.try_send(info_bytes);
        }

        if let (Ok(new_info), Ok(new_spawn)) = (
            encode_to_bytes(&PlayerInfoUpdate {
                entries: vec![PlayerInfoAddEntry {
                    uuid: player_uuid,
                    name: username.clone(),
                    properties,
                    game_mode: VarInt(i32::from(init_mode)),
                    listed: true,
                    latency: VarInt(0),
                }],
            }),
            encode_to_bytes(&SpawnEntity {
                entity_id: VarInt(entity_id),
                uuid: player_uuid,
                entity_type: VarInt(state.registries.entities.player),
                x: session.x,
                y: session.y,
                z: session.z,
                velocity: LpVec3::ZERO,
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: VarInt(0),
            }),
        ) {
            for other in players.values() {
                let _ = other.tx.try_send(new_info.clone());
                if other.dimension == init_dim {
                    let _ = other.tx.try_send(new_spawn.clone());
                }
            }
        }

        players.insert(entity_id, session);
    }

    // Dispatch join hook after player registration with loaded stats so teleport/kick actions apply.
    {
        let join_ctx = ServerPlayerContext {
            entity_id,
            username: username.clone(),
            x: init_x,
            y: init_y,
            z: init_z,
            hp: init_hp,
        };
        let join_actions = state.radons.read().await.dispatch_join(&join_ctx);
        state.execute_radon_actions(join_actions).await;
    }

    info!(%peer_addr, %username, entity_id, "player joined RadServer");
    state
        .broadcast_except(
            &SystemChatMessage {
                content: translated_text(
                    "multiplayer.player.joined",
                    NbtTag::String(username.clone()),
                    "yellow",
                ),
                overlay: false,
            },
            None,
        )
        .await;

    let mut cur_x = init_x;
    let mut cur_y = init_y;
    let mut cur_z = init_z;
    let mut cur_yaw = init_yaw;
    let mut cur_pitch = init_pitch;
    let mut cur_cx = spawn_cx;
    let mut cur_cz = spawn_cz;
    let mut last_move_at = Instant::now();
    let mut mining_target = mining::MiningAttempt::default();
    let mut disconnect_reason = "unknown";

    loop {
        if !pending_outer_chunks.is_empty() {
            let mut batch_coords = Vec::with_capacity(16);
            while batch_coords.len() < 16 {
                let Some((cx, cz)) = pending_outer_chunks.pop_front() else {
                    break;
                };
                if !loaded_chunks.contains(&(cx, cz))
                    && (cx - cur_cx).abs().max((cz - cur_cz).abs()) <= effective_view_distance
                {
                    batch_coords.push((cx, cz));
                }
            }
            if !batch_coords.is_empty() {
                let active_dim = state
                    .players
                    .read()
                    .await
                    .get(&entity_id)
                    .map_or(init_dim, |p| p.dimension);
                state
                    .ensure_chunks_extras_loaded(active_dim, &batch_coords)
                    .await;
                let mut chunk_batch = BytesMut::with_capacity(batch_coords.len() * 4096);
                {
                    let mut world = state.world.write().await;
                    for &(cx, cz) in &batch_coords {
                        let chunk_pkt = world.chunk_packet_in(active_dim, cx, cz);
                        if encode_packet(&chunk_pkt, &mut chunk_batch).is_ok() {
                            loaded_chunks.insert((cx, cz));
                        }
                    }
                }
                if !chunk_batch.is_empty() && !tx.send_wait(chunk_batch.freeze()).await {
                    disconnect_reason = "outbound_closed_during_outer_chunks";
                    break;
                }
            }
        }

        let mut sync_pos_opt = None;
        let frame_opt = tokio::select! {
            biased;
            _ = cancel_rx.wait_for(|&c| c) => {
                disconnect_reason = "cancelled_or_write_failed";
                break;
            }
            Some(sync_pos) = chest_sync_rx.recv() => {
                sync_pos_opt = Some(sync_pos);
                None
            }
            res = tokio::time::timeout(
                Duration::from_secs(cfg.play_read_timeout_secs),
                read_raw_frame(&mut reader, &mut read_buf, cfg.max_packet_size),
            ) => match res {
                Ok(Some(f)) => Some(f),
                Ok(None) => {
                    disconnect_reason = "client_eof_or_frame_error";
                    break;
                }
                Err(_) => {
                    disconnect_reason = "read_timeout";
                    break;
                }
            },
            () = tokio::task::yield_now(), if !pending_outer_chunks.is_empty() => {
                continue;
            }
        };
        if let Some(sync_pos) = sync_pos_opt {
            if open_chest_pos == Some(sync_pos) {
                let mut containers = state.containers.write().await;
                if let Some((cslots, ccomps)) = containers
                    .get_chest_and_components_mut_in(sync_pos.0, sync_pos.1, sync_pos.2, sync_pos.3)
                    && let Ok(bytes) = {
                        let inv_guard = inventory.lock().unwrap();
                        encode_to_bytes(
                            &inv_guard.to_chest_container_set_content_stacks(1, cslots, ccomps),
                        )
                    }
                {
                    let _ = tx.try_send(bytes);
                }
            }
            continue;
        }
        let Some(frame) = frame_opt else {
            break;
        };
        if frame.id != KeepAliveServerbound::ID
            && frame.id != ConfirmTeleportation::ID
            && frame.id != MovePlayerStatusOnly::ID
            && let Some(p) = state.players.write().await.get_mut(&entity_id)
        {
            p.last_activity = Instant::now();
        }
        if frame.id == KeepAliveServerbound::ID {
            if let Ok(ka) = frame.decode_as::<KeepAliveServerbound>() {
                let mut players = state.players.write().await;
                if let Some(p) = players.get_mut(&entity_id) {
                    if let Some((expected, _)) = p.pending_keep_alive
                        && ka.id != expected
                    {
                        debug!(%username, expected, got = ka.id, "mismatched keep-alive id");
                        disconnect_reason = "keep_alive_mismatch";
                        break;
                    }
                    p.pending_keep_alive = None;
                }
                debug!(%username, id = ka.id, "server keep-alive ack");
            }
        } else if frame.id == ConfirmTeleportation::ID {
            if let Ok(tp) = frame.decode_as::<ConfirmTeleportation>() {
                let confirmed_pos = {
                    let mut players = state.players.write().await;
                    if let Some(p) = players.get_mut(&entity_id)
                        && p.pending_teleport_id == Some(tp.teleport_id.0)
                    {
                        p.pending_teleport_id = None;
                        cur_x = p.x;
                        cur_y = p.y;
                        cur_z = p.z;
                        cur_yaw = p.yaw;
                        cur_pitch = p.pitch;
                        Some((p.x, p.y, p.z, p.yaw, p.pitch, p.on_ground, p.dimension))
                    } else {
                        None
                    }
                };
                if let Some((px, py, pz, pyaw, ppitch, on_ground, cur_dim)) = confirmed_pos {
                    let new_cx = (px / 16.0).floor() as i32;
                    let new_cz = (pz / 16.0).floor() as i32;
                    sync_player_chunks(
                        &state,
                        &tx,
                        cur_dim,
                        &mut loaded_chunks,
                        &mut cur_cx,
                        &mut cur_cz,
                        new_cx,
                        new_cz,
                        effective_view_distance,
                        false,
                    )
                    .await;
                    state
                        .broadcast_in_dimension_except(
                            cur_dim,
                            &rad_core::packet::play::TeleportEntity {
                                entity_id: VarInt(entity_id),
                                x: px,
                                y: py,
                                z: pz,
                                vx: 0.0,
                                vy: 0.0,
                                vz: 0.0,
                                yaw: pyaw,
                                pitch: ppitch,
                                flags: 0,
                                on_ground,
                            },
                            Some(entity_id),
                        )
                        .await;
                }
            }
        } else if frame.id == MovePlayerPos::ID {
            if let Ok(mv) = frame.decode_as::<MovePlayerPos>() {
                let elapsed = last_move_at.elapsed().as_secs_f64();
                last_move_at = Instant::now();
                handle_player_move(
                    &state,
                    entity_id,
                    &tx,
                    &mut loaded_chunks,
                    &mut cur_x,
                    &mut cur_y,
                    &mut cur_z,
                    &mut cur_yaw,
                    &mut cur_pitch,
                    &mut cur_cx,
                    &mut cur_cz,
                    Some((mv.x, mv.y, mv.z)),
                    None,
                    elapsed,
                    effective_view_distance,
                )
                .await;
            }
        } else if frame.id == MovePlayerPosRot::ID {
            if let Ok(mv) = frame.decode_as::<MovePlayerPosRot>() {
                let elapsed = last_move_at.elapsed().as_secs_f64();
                last_move_at = Instant::now();
                handle_player_move(
                    &state,
                    entity_id,
                    &tx,
                    &mut loaded_chunks,
                    &mut cur_x,
                    &mut cur_y,
                    &mut cur_z,
                    &mut cur_yaw,
                    &mut cur_pitch,
                    &mut cur_cx,
                    &mut cur_cz,
                    Some((mv.x, mv.y, mv.z)),
                    Some((mv.yaw, mv.pitch)),
                    elapsed,
                    effective_view_distance,
                )
                .await;
            }
        } else if frame.id == MovePlayerRot::ID {
            if let Ok(mv) = frame.decode_as::<MovePlayerRot>() {
                let elapsed = last_move_at.elapsed().as_secs_f64();
                last_move_at = Instant::now();
                handle_player_move(
                    &state,
                    entity_id,
                    &tx,
                    &mut loaded_chunks,
                    &mut cur_x,
                    &mut cur_y,
                    &mut cur_z,
                    &mut cur_yaw,
                    &mut cur_pitch,
                    &mut cur_cx,
                    &mut cur_cz,
                    None,
                    Some((mv.yaw, mv.pitch)),
                    elapsed,
                    effective_view_distance,
                )
                .await;
            }
        } else if frame.id == MovePlayerStatusOnly::ID {
            if let Ok(_mv) = frame.decode_as::<MovePlayerStatusOnly>() {
                let elapsed = last_move_at.elapsed().as_secs_f64();
                last_move_at = Instant::now();
                handle_player_move(
                    &state,
                    entity_id,
                    &tx,
                    &mut loaded_chunks,
                    &mut cur_x,
                    &mut cur_y,
                    &mut cur_z,
                    &mut cur_yaw,
                    &mut cur_pitch,
                    &mut cur_cx,
                    &mut cur_cz,
                    None,
                    None,
                    elapsed,
                    effective_view_distance,
                )
                .await;
            }
        } else if frame.id == 0x0E
            && let Some(vd) = decode_client_view_distance(&frame.payload)
        {
            let new_vd = vd.clamp(2, cfg.view_distance);
            if new_vd != effective_view_distance {
                effective_view_distance = new_vd;
                pending_outer_chunks.clear();
                if let Ok(bytes) = encode_to_bytes(&SetChunkCacheRadius {
                    view_distance: VarInt(effective_view_distance),
                }) {
                    let _ = tx.try_send(bytes);
                }
                let cur_dim = state
                    .players
                    .read()
                    .await
                    .get(&entity_id)
                    .map_or(init_dim, |p| p.dimension);
                let (target_cx, target_cz) = (cur_cx, cur_cz);
                sync_player_chunks(
                    &state,
                    &tx,
                    cur_dim,
                    &mut loaded_chunks,
                    &mut cur_cx,
                    &mut cur_cz,
                    target_cx,
                    target_cz,
                    effective_view_distance,
                    true,
                )
                .await;
            }
        } else if frame.id == Attack::ID {
            let target_id = frame.decode_as::<Attack>().ok().map(|a| a.entity_id.0);
            if let Some(target_id) = target_id {
                let cur_dim = state
                    .players
                    .read()
                    .await
                    .get(&entity_id)
                    .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
                        p.dimension
                    });
                let attack_dmg = crate::gameplay::inventory::tool_attack_damage_with(
                    inventory.lock().unwrap().held_item().item_id,
                    &state.registries.items,
                );
                // Check mob attack first (must be in same dimension)
                let (mob_kind, mob_hit) = {
                    let mut entities = state.entities.write().await;
                    if let Some(mob) = entities.get(target_id)
                        && mob.dimension == cur_dim
                    {
                        let kind = mob.kind;
                        let dx = mob.x - cur_x;
                        let dy = mob.y - cur_y;
                        let dz = mob.z - cur_z;
                        let hit = if dx * dx + dy * dy + dz * dz <= 36.0 {
                            entities.damage_mob(target_id, attack_dmg, dx, dz)
                        } else {
                            None
                        };
                        (Some(kind), hit)
                    } else {
                        (None, None)
                    }
                };
                if let Some(outcome) = mob_hit {
                    if outcome.killed {
                        state
                            .player_stats
                            .write()
                            .await
                            .entry(player_uuid)
                            .or_default()
                            .mobs_killed += 1;
                        if mob_kind == Some(crate::gameplay::entity::MobKind::Pig)
                            && inventory
                                .lock()
                                .unwrap()
                                .give_item(state.registries.items.cooked_porkchop, 1)
                        {
                            sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                        }
                        state
                            .broadcast_in_dimension_except(
                                cur_dim,
                                &RemoveEntities {
                                    entity_ids: vec![VarInt(target_id)],
                                },
                                None,
                            )
                            .await;
                    } else {
                        state
                            .broadcast_in_dimension_except(
                                cur_dim,
                                &HurtAnimation {
                                    entity_id: VarInt(target_id),
                                    yaw: cur_yaw,
                                },
                                None,
                            )
                            .await;
                        state
                            .broadcast_in_dimension_except(
                                cur_dim,
                                &SetEntityMotion {
                                    entity_id: VarInt(target_id),
                                    velocity: LpVec3::new(
                                        f64::from(outcome.knockback_vx) / 8000.0,
                                        f64::from(outcome.knockback_vy) / 8000.0,
                                        f64::from(outcome.knockback_vz) / 8000.0,
                                    ),
                                },
                                None,
                            )
                            .await;
                    }
                } else if target_id != entity_id && state.config.pvp {
                    // PvP attack (must be in same dimension)
                    let kb = {
                        let players = state.players.read().await;
                        players.get(&target_id).and_then(|t| {
                            if t.dimension != cur_dim {
                                return None;
                            }
                            let dx = t.x - cur_x;
                            let dy = t.y - cur_y;
                            let dz = t.z - cur_z;
                            if dx * dx + dy * dy + dz * dz <= 36.0 {
                                let len = (dx * dx + dz * dz).sqrt().max(0.001);
                                Some(((dx / len) * 0.35, 0.28, (dz / len) * 0.35))
                            } else {
                                None
                            }
                        })
                    };
                    if let Some(kb) = kb {
                        apply_player_damage(&state, target_id, 4.0, cur_yaw, Some(kb)).await;
                    }
                }
            }
        } else if frame.id == ClientCommand::ID {
            if let Ok(cmd) = frame.decode_as::<ClientCommand>()
                && cmd.action_id.0 == 0
            {
                let mut should_respawn = false;
                if let Some(p) = state.players.write().await.get_mut(&entity_id)
                    && p.dead
                {
                    p.hp = 20.0;
                    p.food = 20;
                    p.saturation = 5.0;
                    p.dead = false;
                    p.fall_start_y = None;
                    p.on_ground = true;
                    p.x = cfg.spawn_x;
                    p.y = cfg.spawn_y;
                    p.z = cfg.spawn_z;
                    p.dimension = crate::world::worldgen::DimensionId::Overworld;
                    if cfg.hardcore {
                        p.game_mode = 3;
                    }
                    should_respawn = true;
                }
                if should_respawn {
                    let respawn_mode = if cfg.hardcore { 3 } else { cfg.game_mode };
                    state
                        .player_dimensions
                        .write()
                        .await
                        .insert(player_uuid, crate::world::worldgen::DimensionId::Overworld);
                    cur_x = cfg.spawn_x;
                    cur_y = cfg.spawn_y;
                    cur_z = cfg.spawn_z;
                    let mut batch = BytesMut::with_capacity(256);
                    let _ = encode_packet(
                        &Respawn {
                            dimension_type: VarInt(0),
                            dimension_name: "minecraft:overworld".into(),
                            hashed_seed: 0,
                            game_mode: respawn_mode,
                            previous_game_mode: -1,
                            is_debug: false,
                            is_flat: true,
                            death_location: None,
                            portal_cooldown: VarInt(0),
                            sea_level: VarInt(63),
                            data_kept: 0,
                        },
                        &mut batch,
                    );
                    let _ = encode_packet(
                        &SynchronizePlayerPosition {
                            teleport_id: VarInt(2),
                            x: cfg.spawn_x,
                            y: cfg.spawn_y,
                            z: cfg.spawn_z,
                            vx: 0.0,
                            vy: 0.0,
                            vz: 0.0,
                            yaw: 0.0,
                            pitch: 0.0,
                            flags: 0,
                        },
                        &mut batch,
                    );
                    let _ = encode_packet(
                        &SetHealth {
                            health: 20.0,
                            food: VarInt(20),
                            food_saturation: 5.0,
                        },
                        &mut batch,
                    );
                    let _ = tx.try_send(batch.freeze());
                }
            }
        } else if frame.id == SetCarriedItem::ID {
            if let Ok(carried) = frame.decode_as::<SetCarriedItem>() {
                let selected = {
                    let mut inv_guard = inventory.lock().unwrap();
                    inv_guard.set_selected_hotbar(carried.slot);
                    inv_guard.selected_hotbar
                };
                if let Some(p) = state.players.write().await.get_mut(&entity_id) {
                    p.selected_hotbar = selected;
                }
            }
        } else if frame.id == SetCreativeModeSlot::ID {
            if let Ok(update) = frame.decode_as::<SetCreativeModeSlot>() {
                let creative_dim = state
                    .players
                    .read()
                    .await
                    .get(&entity_id)
                    .filter(|p| !p.dead && p.game_mode == 1)
                    .map(|p| p.dimension);
                if let Some(dim) = creative_dim
                    && update.slot == -1
                    && (1..=64).contains(&update.item.count)
                    && state
                        .registries
                        .item_name_by_id(update.item.item_id)
                        .is_some()
                {
                    let yaw = f64::from(cur_yaw).to_radians();
                    let pitch = f64::from(cur_pitch).to_radians();
                    let _ = spawn_and_broadcast_dropped_item(
                        &state,
                        dim,
                        [cur_x, cur_y + 1.32, cur_z],
                        [
                            -yaw.sin() * pitch.cos() * 0.3,
                            -pitch.sin() * 0.3 + 0.1,
                            yaw.cos() * pitch.cos() * 0.3,
                        ],
                        update.item,
                        40,
                    )
                    .await;
                } else if creative_dim.is_some() && update.slot >= 1 {
                    let changed = inventory
                        .lock()
                        .unwrap()
                        .set_creative_stack(update.slot as usize, update.item);
                    if changed {
                        sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                    }
                } else if creative_dim.is_none() {
                    sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                }
            }
        } else if frame.id == PlayerAction::ID {
            if let Ok(action) = frame.decode_as::<PlayerAction>() {
                if action.status.0 == 6 {
                    inventory.lock().unwrap().swap_offhand();
                    sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                } else if action.status.0 == 3 || action.status.0 == 4 {
                    let full_stack = action.status.0 == 3;
                    let maybe_drop = inventory
                        .lock()
                        .unwrap()
                        .peek_drop_selected_hotbar(full_stack);
                    if let Some(drop_stack) = maybe_drop {
                        let (pdim, px, py, pz, pyaw, ppitch) = {
                            let players = state.players.read().await;
                            players.get(&entity_id).map_or(
                                (
                                    crate::world::worldgen::DimensionId::Overworld,
                                    cur_x,
                                    cur_y,
                                    cur_z,
                                    0.0f32,
                                    0.0f32,
                                ),
                                |p| (p.dimension, p.x, p.y, p.z, p.yaw, p.pitch),
                            )
                        };
                        let yaw_rad = f64::from(pyaw).to_radians();
                        let pitch_rad = f64::from(ppitch).to_radians();
                        let vx = -yaw_rad.sin() * pitch_rad.cos() * 0.3;
                        let vy = -pitch_rad.sin() * 0.3 + 0.1;
                        let vz = yaw_rad.cos() * pitch_rad.cos() * 0.3;
                        if spawn_and_broadcast_dropped_item(
                            &state,
                            pdim,
                            [px, py + 1.32, pz],
                            [vx, vy, vz],
                            drop_stack,
                            40,
                        )
                        .await
                        .is_some()
                        {
                            inventory.lock().unwrap().drop_selected_hotbar(full_stack);
                        } else {
                            tracing::warn!(
                                player = %username,
                                "refused inventory drop because world item entity spawn failed"
                            );
                        }
                        sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                    }
                } else if action.status.0 == 0 || action.status.0 == 1 || action.status.0 == 2 {
                    let pos = action.location;
                    let (dead, mode, hp) = {
                        let players = state.players.read().await;
                        players
                            .get(&entity_id)
                            .map_or((true, cfg.game_mode, 0.0), |p| (p.dead, p.game_mode, p.hp))
                    };
                    let dx = f64::from(pos.x) + 0.5 - cur_x;
                    let dy = f64::from(pos.y) + 0.5 - (cur_y + 1.62);
                    let dz = f64::from(pos.z) + 0.5 - cur_z;
                    let within_reach = dx * dx + dy * dy + dz * dz <= 36.0
                        && pos.x.abs() <= cfg.max_world_size
                        && pos.z.abs() <= cfg.max_world_size;
                    let can_break = !dead && mode != 2 && mode != 3 && within_reach;

                    if action.status.0 == 1 {
                        mining_target.cancel();
                    } else if can_break {
                        let dim = state
                            .players
                            .read()
                            .await
                            .get(&entity_id)
                            .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
                                p.dimension
                            });
                        let current = state
                            .world
                            .write()
                            .await
                            .get_block_in(dim, pos.x, pos.y, pos.z);
                        let packaged_block = state
                            .version_package
                            .block_name_by_state(current)
                            .and_then(|name| state.version_package.block_by_name(name));
                        let unbreakable =
                            mode == 0
                                && (packaged_block
                                    .and_then(|block| block.mining.as_ref())
                                    .is_some_and(|data| data.hardness < 0.0)
                                    || state.registries.block_by_state(current).is_some_and(
                                        |block| mining::survival_unbreakable(&block.name),
                                    ));
                        let held_item = {
                            let inv = inventory.lock().unwrap();
                            let index = crate::gameplay::inventory::HOTBAR_START_SLOT
                                + usize::from(inv.selected_hotbar.min(8));
                            inv.slots
                                .get(index)
                                .filter(|slot| slot.count > 0)
                                .map(|slot| slot.item_id)
                        };
                        let block = state.registries.block_by_state(current);
                        let block_id = block.as_ref().map_or(0, |entry| entry.block_id);
                        let block_name = block.as_ref().map_or("", |entry| entry.name.as_str());
                        let item_name =
                            held_item.and_then(|id| state.registries.item_name_by_id(id));
                        let minimum = packaged_block
                            .and_then(|block| block.mining.as_ref())
                            .and_then(|data| mining::packaged_duration(data, item_name))
                            .unwrap_or_else(|| mining::minimum_duration(block_name, item_name));
                        let should_break = if unbreakable {
                            mining_target.cancel();
                            false
                        } else if mode == 1 {
                            action.status.0 == 0
                        } else if action.status.0 == 0 {
                            if minimum.is_zero() && current != 0 {
                                mining_target.cancel();
                                true
                            } else {
                                mining_target.start_block(pos, block_id, current, Instant::now());
                                false
                            }
                        } else {
                            mining_target.finish_block(
                                pos,
                                block_id,
                                current,
                                Instant::now(),
                                minimum,
                            )
                        };

                        // The client predicts a completed break. If the server rejects it,
                        // restore the authoritative block before acknowledging the action.
                        if !should_break && (action.status.0 == 2 || unbreakable) {
                            let dim = state
                                .players
                                .read()
                                .await
                                .get(&entity_id)
                                .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
                                    p.dimension
                                });
                            let current = state
                                .world
                                .write()
                                .await
                                .get_block_in(dim, pos.x, pos.y, pos.z);
                            if let Ok(bytes) = encode_to_bytes(&BlockUpdate {
                                pos,
                                block_state: VarInt(i32::from(current)),
                            }) {
                                let _ = tx.try_send(bytes);
                            }
                        }

                        if should_break {
                            let cur_dim = state
                                .players
                                .read()
                                .await
                                .get(&entity_id)
                                .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
                                    p.dimension
                                });
                            if is_spawn_protected(cfg, cur_dim, pos)
                                && state.permission_level(&username, Some(player_uuid)).await < 1
                            {
                                let prev = state
                                    .world
                                    .write()
                                    .await
                                    .get_block_in(cur_dim, pos.x, pos.y, pos.z);
                                if let Ok(bytes) = encode_to_bytes(&BlockUpdate {
                                    pos,
                                    block_state: VarInt(i32::from(prev)),
                                }) {
                                    let _ = tx.try_send(bytes);
                                }
                                let _ = tx.try_send(
                                    encode_to_bytes(&AcknowledgeBlockChange {
                                        sequence: action.sequence,
                                    })
                                    .unwrap_or_default(),
                                );
                                continue;
                            }
                            let prev_state = state
                                .world
                                .write()
                                .await
                                .get_block_in(cur_dim, pos.x, pos.y, pos.z);
                            if prev_state != 0 {
                                let p_ctx = ServerPlayerContext {
                                    entity_id,
                                    username: username.clone(),
                                    x: cur_x,
                                    y: cur_y,
                                    z: cur_z,
                                    hp,
                                };
                                let (res, actions) =
                                    state.radons.read().await.dispatch_block_break(
                                        &p_ctx, pos.x, pos.y, pos.z, prev_state,
                                    );
                                state.execute_radon_actions(actions).await;

                                if res == ServerEventResult::Allow {
                                    let changed = {
                                        let mut world = state.world.write().await;
                                        let ok =
                                            world.set_block_in(cur_dim, pos.x, pos.y, pos.z, 0);
                                        if ok {
                                            let cur_tick = state.tick_count.load(Ordering::Relaxed);
                                            state.world_sim.write().await.on_block_changed_in(
                                                &mut world, cur_dim, pos.x, pos.y, pos.z, cur_tick,
                                            );
                                        }
                                        ok
                                    };
                                    if changed {
                                        state
                                            .player_stats
                                            .write()
                                            .await
                                            .entry(player_uuid)
                                            .or_default()
                                            .blocks_broken += 1;
                                        let broken_name = state
                                            .registries
                                            .block_by_state(prev_state)
                                            .map(|b| b.name.clone());
                                        if matches!(
                                            broken_name.as_deref(),
                                            Some("minecraft:obsidian" | "minecraft:nether_portal")
                                        ) {
                                            let portal_clears = {
                                                let mut world = state.world.write().await;
                                                crate::world::portals::invalidate_broken_nether_portal(
                                                    &mut world, cur_dim, pos.x, pos.y, pos.z,
                                                )
                                            };
                                            for (px, py, pz, st) in portal_clears {
                                                state
                                                    .broadcast_in_dimension_except(
                                                        cur_dim,
                                                        &BlockUpdate {
                                                            pos: BlockPos::new(px, py, pz),
                                                            block_state: VarInt(i32::from(st)),
                                                        },
                                                        None,
                                                    )
                                                    .await;
                                            }
                                        }
                                        if broken_name.as_deref() == Some("minecraft:furnace") {
                                            let removed = state
                                                .furnaces
                                                .write()
                                                .await
                                                .remove_in(cur_dim, pos.x, pos.y, pos.z);
                                            if open_furnace_pos
                                                == Some((cur_dim, pos.x, pos.y, pos.z))
                                            {
                                                open_furnace_pos = None;
                                            }
                                            if let Some(f) = removed {
                                                let cur_tick =
                                                    state.tick_count.load(Ordering::Relaxed);
                                                for (item_id, count) in [
                                                    (f.input_item, f.input_count),
                                                    (f.fuel_item, f.fuel_count),
                                                    (f.output_item, f.output_count),
                                                ] {
                                                    if item_id > 0 && count > 0 {
                                                        let (drop_pos, drop_vel) =
                                                            random_block_drop_spawn(
                                                                pos, cur_tick, entity_id,
                                                            );
                                                        let _ = spawn_and_broadcast_dropped_item(
                                                            &state,
                                                            cur_dim,
                                                            drop_pos,
                                                            drop_vel,
                                                            rad_core::packet::play::ItemStack {
                                                                count,
                                                                item_id,
                                                                components: Default::default(),
                                                            },
                                                            10,
                                                        )
                                                        .await;
                                                    }
                                                }
                                            }
                                        }
                                        if broken_name.as_deref() == Some("minecraft:chest") {
                                            let removed = state
                                                .containers
                                                .write()
                                                .await
                                                .remove_chest_in(cur_dim, pos.x, pos.y, pos.z);
                                            if open_chest_pos
                                                == Some((cur_dim, pos.x, pos.y, pos.z))
                                            {
                                                open_chest_pos = None;
                                                open_furnace_pos = None;
                                            }
                                            if let Some(slots) = removed {
                                                let cur_tick =
                                                    state.tick_count.load(Ordering::Relaxed);
                                                for slot in slots {
                                                    if slot.item_id > 0 && slot.count > 0 {
                                                        let (drop_pos, drop_vel) =
                                                            random_block_drop_spawn(
                                                                pos, cur_tick, entity_id,
                                                            );
                                                        let _ = spawn_and_broadcast_dropped_item(
                                                            &state,
                                                            cur_dim,
                                                            drop_pos,
                                                            drop_vel,
                                                            rad_core::packet::play::ItemStack {
                                                                count: slot.count,
                                                                item_id: slot.item_id,
                                                                components: Default::default(),
                                                            },
                                                            10,
                                                        )
                                                        .await;
                                                    }
                                                }
                                            }
                                        }
                                        if mode == 0
                                            && packaged_block
                                                .and_then(|block| block.mining.as_ref())
                                                .and_then(|data| {
                                                    mining::can_harvest(data, item_name)
                                                })
                                                .unwrap_or_else(|| {
                                                    mining::fallback_can_harvest(
                                                        block_name, item_name,
                                                    )
                                                })
                                            && let Some(item_id) =
                                                crate::gameplay::inventory::block_state_to_item_id_with(
                                                    prev_state,
                                                    &state.registries,
                                                )
                                        {
                                            let cur_tick = state.tick_count.load(Ordering::Relaxed);
                                            let (drop_pos, drop_vel) =
                                                random_block_drop_spawn(pos, cur_tick, entity_id);
                                            let _ = spawn_and_broadcast_dropped_item(
                                                &state,
                                                cur_dim,
                                                drop_pos,
                                                drop_vel,
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
                                            .broadcast_in_dimension_except(
                                                cur_dim,
                                                &BlockUpdate {
                                                    pos,
                                                    block_state: VarInt(0),
                                                },
                                                None,
                                            )
                                            .await;
                                        if let Some(door_name) = state
                                            .registries
                                            .block_by_state(prev_state)
                                            .map(|b| b.name.clone())
                                            .filter(|n| {
                                                n.ends_with("_door")
                                                    || matches!(
                                                        n.as_str(),
                                                        "minecraft:tall_grass"
                                                            | "minecraft:large_fern"
                                                            | "minecraft:sunflower"
                                                            | "minecraft:lilac"
                                                            | "minecraft:rose_bush"
                                                            | "minecraft:peony"
                                                            | "minecraft:pitcher_plant"
                                                            | "minecraft:tall_seagrass"
                                                    )
                                            })
                                        {
                                            for other_y in [pos.y + 1, pos.y - 1] {
                                                let other_state =
                                                    state.world.write().await.get_block_in(
                                                        cur_dim, pos.x, other_y, pos.z,
                                                    );
                                                if state
                                                    .registries
                                                    .block_by_state(other_state)
                                                    .is_some_and(|b| b.name == door_name)
                                                {
                                                    let cur_tick = state
                                                        .tick_count
                                                        .load(std::sync::atomic::Ordering::Relaxed);
                                                    let cleared = {
                                                        let mut world = state.world.write().await;
                                                        let ok = world.set_block_in(
                                                            cur_dim, pos.x, other_y, pos.z, 0,
                                                        );
                                                        if ok {
                                                            state
                                                                .world_sim
                                                                .write()
                                                                .await
                                                                .on_block_changed_in(
                                                                    &mut world, cur_dim, pos.x,
                                                                    other_y, pos.z, cur_tick,
                                                                );
                                                        }
                                                        ok
                                                    };
                                                    if cleared {
                                                        state
                                                            .broadcast_in_dimension_except(
                                                                cur_dim,
                                                                &BlockUpdate {
                                                                    pos: BlockPos::new(
                                                                        pos.x, other_y, pos.z,
                                                                    ),
                                                                    block_state: VarInt(0),
                                                                },
                                                                None,
                                                            )
                                                            .await;
                                                    }
                                                }
                                            }
                                        }
                                        if let Some((px, py, pz)) =
                                            crate::world::sim::piston_partner_pos(
                                                prev_state,
                                                pos.x,
                                                pos.y,
                                                pos.z,
                                                &state.registries,
                                            )
                                        {
                                            let cleared = {
                                                let mut world = state.world.write().await;
                                                let partner_state =
                                                    world.get_block_in(cur_dim, px, py, pz);
                                                let is_piston_part = state
                                                    .registries
                                                    .block_by_state(partner_state)
                                                    .is_some_and(|b| {
                                                        matches!(
                                                            b.name.as_str(),
                                                            "minecraft:piston_head"
                                                                | "minecraft:piston"
                                                                | "minecraft:sticky_piston"
                                                        )
                                                    });
                                                is_piston_part
                                                    && world.set_block_in(cur_dim, px, py, pz, 0)
                                            };
                                            if cleared {
                                                state
                                                    .broadcast_in_dimension_except(
                                                        cur_dim,
                                                        &BlockUpdate {
                                                            pos: BlockPos::new(px, py, pz),
                                                            block_state: VarInt(0),
                                                        },
                                                        None,
                                                    )
                                                    .await;
                                            }
                                        }
                                        let rs_updates = {
                                            let mut world = state.world.write().await;
                                            state.world_sim.write().await.update_redstone_around_in(
                                                &mut world, cur_dim, pos.x, pos.y, pos.z,
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
                    } else {
                        mining_target.cancel();
                        let dim = state
                            .players
                            .read()
                            .await
                            .get(&entity_id)
                            .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
                                p.dimension
                            });
                        let current = state
                            .world
                            .write()
                            .await
                            .get_block_in(dim, pos.x, pos.y, pos.z);
                        if let Ok(bytes) = encode_to_bytes(&BlockUpdate {
                            pos,
                            block_state: VarInt(i32::from(current)),
                        }) {
                            let _ = tx.try_send(bytes);
                        }
                    }

                    if let Ok(ack_bytes) = encode_to_bytes(&AcknowledgeBlockChange {
                        sequence: action.sequence,
                    }) {
                        let _ = tx.try_send(ack_bytes);
                    }
                }
            }
        } else if frame.id == rad_core::packet::play::PickItemFromBlock::ID {
            if let Ok(pick) = frame.decode_as::<rad_core::packet::play::PickItemFromBlock>() {
                let player = state
                    .players
                    .read()
                    .await
                    .get(&entity_id)
                    .map(|p| (p.dimension, p.game_mode, p.dead));
                if let Some((dim, mode, false)) = player {
                    let dx = f64::from(pick.location.x) + 0.5 - cur_x;
                    let dy = f64::from(pick.location.y) + 0.5 - (cur_y + 1.62);
                    let dz = f64::from(pick.location.z) + 0.5 - cur_z;
                    if dx * dx + dy * dy + dz * dz <= 36.0 {
                        let block = state.world.write().await.get_block_in(
                            dim,
                            pick.location.x,
                            pick.location.y,
                            pick.location.z,
                        );
                        if let Some(item_id) = block_state_to_pick_item_id(block, &state.registries)
                        {
                            let selected = inventory
                                .lock()
                                .unwrap()
                                .pick_block_item(item_id, mode == 1);
                            if let Some(slot) = selected {
                                sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                                if let Ok(packet) = encode_to_bytes(&SetHeldSlot {
                                    slot: VarInt(i32::from(slot)),
                                }) {
                                    let _ = tx.try_send(packet);
                                }
                            }
                        }
                    }
                }
            }
        } else if frame.id == UseItemOn::ID {
            if let Ok(use_on) = frame.decode_as::<UseItemOn>() {
                use_item_on::handle_use_item_on(
                    &state,
                    entity_id,
                    player_uuid,
                    &username,
                    cur_x,
                    cur_y,
                    cur_z,
                    cur_yaw,
                    cur_pitch,
                    &inventory,
                    &tx,
                    &mut open_chest_pos,
                    &mut open_furnace_pos,
                    use_on,
                )
                .await;
            }
        } else if frame.id == UseItem::ID {
            if let Ok(use_item) = frame.decode_as::<UseItem>() {
                let held = inventory.lock().unwrap().held_item();
                if let Some((add_food, add_sat)) = crate::gameplay::inventory::food_nutrition_with(
                    held.item_id,
                    &state.registries.items,
                ) {
                    let updated_health = {
                        let mut players = state.players.write().await;
                        if let Some(p) = players.get_mut(&entity_id)
                            && !p.dead
                            && p.food < 20
                        {
                            p.food = (p.food + add_food).min(20);
                            p.saturation = (p.saturation + add_sat).min(p.food as f32);
                            Some((p.hp, p.food, p.saturation))
                        } else {
                            None
                        }
                    };
                    if let Some((hp, food, saturation)) = updated_health {
                        if inventory.lock().unwrap().consume_held_item() {
                            sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                        }
                        if let Ok(bytes) = encode_to_bytes(&SetHealth {
                            health: hp,
                            food: VarInt(food),
                            food_saturation: saturation,
                        }) {
                            let _ = tx.try_send(bytes);
                        }
                    }
                } else if held.count > 0
                    && (held.item_id == state.registries.items.snowball
                        || held.item_id == state.registries.items.bow)
                {
                    let cur_tick = state.tick_count.load(Ordering::Relaxed);
                    let (dead, cur_dim, pmode) = state.players.read().await.get(&entity_id).map_or(
                        (
                            true,
                            crate::world::worldgen::DimensionId::Overworld,
                            state.config.game_mode,
                        ),
                        |p| (p.dead, p.dimension, p.game_mode),
                    );
                    if !dead
                        && last_projectile_tick
                            .is_none_or(|prev| cur_tick.saturating_sub(prev) >= 4)
                    {
                        let is_arrow = held.item_id == state.registries.items.bow;
                        let can_fire = if is_arrow {
                            if pmode == 1 {
                                true
                            } else if inventory
                                .lock()
                                .unwrap()
                                .consume_item_by_id(state.registries.items.arrow, 1)
                            {
                                sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                                true
                            } else {
                                false
                            }
                        } else if pmode == 1 {
                            true
                        } else if inventory.lock().unwrap().consume_held_item() {
                            sync_player_inventory(&state, entity_id, &inventory, &tx).await;
                            true
                        } else {
                            false
                        };
                        if can_fire {
                            last_projectile_tick = Some(cur_tick);
                            let yaw_rad = f64::from(use_item.yaw).to_radians();
                            let pitch_rad = f64::from(use_item.pitch).to_radians();
                            let speed = if is_arrow { 2.0 } else { 1.5 };
                            let vx = -yaw_rad.sin() * pitch_rad.cos() * speed;
                            let vy = -pitch_rad.sin() * speed;
                            let vz = yaw_rad.cos() * pitch_rad.cos() * speed;
                            let proj_eid = state.next_entity_id.fetch_add(1, Ordering::Relaxed);
                            let proj_kind = if is_arrow {
                                crate::gameplay::entity::ProjectileKind::Arrow
                            } else {
                                crate::gameplay::entity::ProjectileKind::Snowball
                            };
                            let spawn_pkt = {
                                let mut entities = state.entities.write().await;
                                entities
                                    .spawn_projectile_in(
                                        cur_dim,
                                        proj_eid,
                                        proj_kind,
                                        entity_id,
                                        [cur_x, cur_y + 1.5, cur_z],
                                        [vx, vy, vz],
                                    )
                                    .to_spawn_packet_with(&state.registries.entities)
                            };
                            state
                                .broadcast_in_dimension_except(cur_dim, &spawn_pkt, None)
                                .await;
                        }
                    }
                }
                if let Ok(ack_bytes) = encode_to_bytes(&AcknowledgeBlockChange {
                    sequence: use_item.sequence,
                }) {
                    let _ = tx.try_send(ack_bytes);
                }
            }
        } else if frame.id == ContainerClick::ID {
            if let Ok(click) = frame.decode_as::<ContainerClick>() {
                containers::handle_container_click(
                    &state,
                    entity_id,
                    player_uuid,
                    cur_x,
                    cur_y,
                    cur_z,
                    &inventory,
                    &tx,
                    &mut open_chest_pos,
                    &mut open_furnace_pos,
                    click,
                )
                .await;
            }
        } else if frame.id == ContainerClose::ID {
            if let Ok(close) = frame.decode_as::<ContainerClose>() {
                containers::handle_container_close(
                    &state,
                    entity_id,
                    &inventory,
                    &tx,
                    &mut open_chest_pos,
                    &mut open_furnace_pos,
                    close,
                )
                .await;
            }
        } else if frame.id == ServerboundChat::ID {
            if let Ok(chat) = frame.decode_as::<ServerboundChat>() {
                let text = chat.message.trim();
                if !text.is_empty() {
                    let p_ctx = ServerPlayerContext {
                        entity_id,
                        username: username.clone(),
                        x: cur_x,
                        y: cur_y,
                        z: cur_z,
                        hp: 20.0,
                    };
                    let (res, actions) = state.radons.read().await.dispatch_chat(&p_ctx, text);
                    state.execute_radon_actions(actions).await;
                    if res == ServerEventResult::Allow {
                        state.broadcast_chat(format!("<{username}> {text}")).await;
                    }
                }
            }
        } else if frame.id == ChangeGameMode::ID {
            if let Ok(request) = frame.decode_as::<ChangeGameMode>()
                && (0..=3).contains(&request.0.0)
                && state.permission_level(&username, Some(player_uuid)).await >= 2
            {
                commands::handle_server_command(
                    &state,
                    &tx,
                    &username,
                    &format!("gamemode {}", request.0.0),
                    Some(&inventory),
                )
                .await;
            }
        } else if frame.id == ChatCommand::ID
            && let Ok(cmd) = frame.decode_as::<ChatCommand>()
        {
            handle_server_command(&state, &tx, &username, &cmd.command, Some(&inventory)).await;
        } else if frame.id == 0x08
            && let Ok(command) = {
                use rad_core::types::Decode as _;
                let mut slice = frame.payload.as_ref();
                String::decode(&mut slice)
            }
        {
            handle_server_command(&state, &tx, &username, &command, Some(&inventory)).await;
        } else if frame.id == CommandSuggestionsRequest::ID
            && let Ok(req) = frame.decode_as::<CommandSuggestionsRequest>()
        {
            commands::handle_command_suggestions(&state, &tx, &username, &req).await;
        }
    }

    tx.disconnect();
    // Cleanup on disconnect
    state.live_inventories.write().await.remove(&entity_id);
    let removed = state.players.write().await.remove(&entity_id);
    if let Some(p) = removed {
        let dim = state
            .player_dimensions
            .write()
            .await
            .remove(&player_uuid)
            .unwrap_or_default();
        let stats = state
            .player_stats
            .write()
            .await
            .remove(&player_uuid)
            .unwrap_or_default();
        if let Err(e) = state
            .save_player_data(
                player_uuid,
                &SavedPlayerData {
                    x: p.x,
                    y: p.y,
                    z: p.z,
                    yaw: p.yaw,
                    pitch: p.pitch,
                    hp: p.hp,
                    food: p.food,
                    saturation: p.saturation,
                    game_mode: p.game_mode,
                    inventory: p.inventory,
                    selected_hotbar: p.selected_hotbar,
                    dimension: dim,
                    stats,
                },
            )
            .await
        {
            tracing::warn!(uuid = %player_uuid, error = %e, "failed to save playerdata on disconnect");
        }
        let quit_ctx = ServerPlayerContext {
            entity_id,
            username: p.username,
            x: p.x,
            y: p.y,
            z: p.z,
            hp: p.hp,
        };
        let quit_actions = state.radons.read().await.dispatch_quit(&quit_ctx);
        state.execute_radon_actions(quit_actions).await;
    }
    state
        .broadcast_except(
            &RemoveEntities {
                entity_ids: vec![VarInt(entity_id)],
            },
            Some(entity_id),
        )
        .await;
    state
        .broadcast_except(
            &PlayerInfoRemove {
                uuids: vec![player_uuid],
            },
            Some(entity_id),
        )
        .await;
    state
        .broadcast_except(
            &SystemChatMessage {
                content: translated_text(
                    "multiplayer.player.left",
                    NbtTag::String(username.clone()),
                    "yellow",
                ),
                overlay: false,
            },
            None,
        )
        .await;
    writer_task.abort();
    info!(%peer_addr, %username, reason = disconnect_reason, "player left RadServer");
}

/// Computes a Vanilla-like randomized spawn position and scatter velocity for a broken block drop.
fn random_block_drop_spawn(pos: BlockPos, tick_count: u64, entity_id: i32) -> ([f64; 3], [f64; 3]) {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0_u64, |d| d.subsec_nanos() as u64);
    let mut seed = tick_count
        ^ nanos.wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (i64::from(pos.x) as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
        ^ (i64::from(pos.y) as u64).wrapping_mul(0x94D0_49BB_1331_11EB)
        ^ (i64::from(pos.z) as u64).wrapping_mul(0x517C_C1B7_2722_0A95)
        ^ (i64::from(entity_id) as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);

    let mut next_unit = || -> f64 {
        seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f64) / ((1_u64 << 53) as f64)
    };

    let ox = (next_unit() - 0.5) * 0.5;
    let oy = (next_unit() - 0.5) * 0.5;
    let oz = (next_unit() - 0.5) * 0.5;
    let vx = (next_unit() - 0.5) * 0.2;
    let vy = 0.2;
    let vz = (next_unit() - 0.5) * 0.2;

    (
        [
            f64::from(pos.x) + 0.5 + ox,
            f64::from(pos.y) + 0.5 + oy,
            f64::from(pos.z) + 0.5 + oz,
        ],
        [vx, vy, vz],
    )
}

#[cfg(test)]
mod block_drop_tests {
    use super::*;

    #[test]
    fn block_drop_spawn_stays_within_vanilla_scatter_bounds() {
        let pos = BlockPos::new(10, 64, -5);
        for tick in 0..32_u64 {
            let (p, v) = random_block_drop_spawn(pos, tick, 1);
            assert!((10.25..=10.75).contains(&p[0]));
            assert!((64.25..=64.75).contains(&p[1]));
            assert!((-4.75..=-4.25).contains(&p[2]));
            assert!((-0.1..=0.1).contains(&v[0]));
            assert!((v[1] - 0.2).abs() < f64::EPSILON);
            assert!((-0.1..=0.1).contains(&v[2]));
        }
    }
}
