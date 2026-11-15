//! ContainerClick and ContainerClose packet handlers.

use super::*;
use crate::world::worldgen::DimensionId;

#[allow(clippy::too_many_arguments)]
pub(super) async fn handle_container_click(
    state: &Arc<ServerState>,
    entity_id: i32,
    _player_uuid: Uuid,
    cur_x: f64,
    cur_y: f64,
    cur_z: f64,
    inventory: &Arc<std::sync::Mutex<PlayerInventory>>,
    tx: &OutboundSender,
    open_chest_pos: &mut Option<(DimensionId, i32, i32, i32)>,
    open_furnace_pos: &mut Option<(DimensionId, i32, i32, i32)>,
    click: ContainerClick,
) {
    let is_craft3x3 = click.window_id.0 == crate::gameplay::inventory::CRAFTING_WINDOW_ID;
    let is_furnace = click.window_id.0 == crate::gameplay::inventory::FURNACE_WINDOW_ID;
    let cur_dim = state
        .players
        .read()
        .await
        .get(&entity_id)
        .map_or(crate::world::worldgen::DimensionId::Overworld, |p| {
            p.dimension
        });
    if is_furnace {
        if let Some(fpos) = (*open_furnace_pos)
            && fpos.0 == cur_dim
        {
            let mut furnaces = state.furnaces.write().await;
            if let Some(f) = furnaces.get_mut_in(fpos.0, fpos.1, fpos.2, fpos.3) {
                let mut fslots = [
                    rad_core::packet::play::SimpleSlot {
                        count: f.input_count,
                        item_id: f.input_item,
                    },
                    rad_core::packet::play::SimpleSlot {
                        count: f.fuel_count,
                        item_id: f.fuel_item,
                    },
                    f.output_slot(),
                ];
                let furnace_result = {
                    let mut inv_guard = inventory.lock().unwrap();
                    let res = inv_guard.apply_furnace_click(
                        click.state_id.0,
                        click.slot,
                        click.button,
                        click.mode.0,
                        &mut fslots,
                    );
                    let pkt = encode_to_bytes(&inv_guard.to_furnace_container_set_content(
                        click.window_id.0,
                        fslots[0],
                        fslots[1],
                        fslots[2],
                    ));
                    (res, pkt)
                };
                if let Ok(modified) = furnace_result.0
                    && modified
                {
                    f.input_count = fslots[0].count;
                    f.input_item = fslots[0].item_id;
                    f.fuel_count = fslots[1].count;
                    f.fuel_item = fslots[1].item_id;
                    f.output_count = fslots[2].count;
                    f.output_item = fslots[2].item_id;
                    furnaces.mark_dirty();
                }
                if let Ok(bytes) = furnace_result.1 {
                    let _ = tx.try_send(bytes);
                }
            }
        } else {
            (*open_furnace_pos) = None;
        }
        sync_player_inventory(state, entity_id, inventory, tx).await;
        return;
    }
    if (*open_chest_pos).is_some_and(|pos| pos.0 != cur_dim) {
        (*open_chest_pos) = None;
    }
    let mut containers = if click.window_id.0 > 0 && !is_craft3x3 {
        Some(state.containers.write().await)
    } else {
        None
    };
    let click_res = {
        let chest_ref = containers.as_deref_mut().and_then(|c| {
            let pos = (*open_chest_pos)?;
            c.get_chest_and_components_mut_in(pos.0, pos.1, pos.2, pos.3)
                .map(|(s, comp)| (s, Some(comp)))
        });
        inventory.lock().unwrap().apply_container_click_with_drop(
            click.window_id.0,
            click.state_id.0,
            click.slot,
            click.button,
            click.mode.0,
            chest_ref,
        )
    };
    match click_res {
        Ok((chest_modified, maybe_dropped)) => {
            if chest_modified && let Some(ref mut c) = containers {
                c.mark_dirty();
            }
            if is_craft3x3
                && let Ok(bytes) = {
                    let inv_guard = inventory.lock().unwrap();
                    encode_to_bytes(&inv_guard.to_crafting_container_set_content(click.window_id.0))
                }
            {
                let _ = tx.try_send(bytes);
            } else if let Some(ref mut c) = containers
                && let Some(pos) = (*open_chest_pos)
                && let Some((chest_slots, chest_comps)) =
                    c.get_chest_and_components_mut_in(pos.0, pos.1, pos.2, pos.3)
            {
                if let Ok(bytes) = {
                    let inv_guard = inventory.lock().unwrap();
                    encode_to_bytes(&inv_guard.to_chest_container_set_content_stacks(
                        click.window_id.0,
                        chest_slots,
                        chest_comps,
                    ))
                } {
                    let _ = tx.try_send(bytes);
                }
                if chest_modified {
                    let other_viewers: Vec<_> = {
                        let players = state.players.read().await;
                        players
                            .iter()
                            .filter(|&(&eid, p)| eid != entity_id && p.open_chest == Some(pos))
                            .filter_map(|(_, p)| p.chest_sync_tx.clone())
                            .collect()
                    };
                    for sync_tx in other_viewers {
                        let _ = sync_tx.send(pos);
                    }
                }
            }
            drop(containers);
            if let Some(mut drop_stack) = maybe_dropped {
                let (pdim, px, py, pz, pyaw, ppitch) = {
                    let players = state.players.read().await;
                    players
                        .get(&entity_id)
                        .map_or((cur_dim, cur_x, cur_y, cur_z, 0.0f32, 0.0f32), |p| {
                            (p.dimension, p.x, p.y, p.z, p.yaw, p.pitch)
                        })
                };
                let yaw_rad = f64::from(pyaw).to_radians();
                let pitch_rad = f64::from(ppitch).to_radians();
                let vx = -yaw_rad.sin() * pitch_rad.cos() * 0.3;
                let vy = -pitch_rad.sin() * 0.3 + 0.1;
                let vz = yaw_rad.cos() * pitch_rad.cos() * 0.3;
                if spawn_and_broadcast_dropped_item(
                    state,
                    pdim,
                    [px, py + 1.32, pz],
                    [vx, vy, vz],
                    drop_stack.clone(),
                    40,
                )
                .await
                .is_none()
                {
                    inventory.lock().unwrap().insert_item_stack(&mut drop_stack);
                }
            }
            sync_player_inventory(state, entity_id, inventory, tx).await;
        }
        Err(()) => {
            if is_craft3x3
                && let Ok(bytes) = {
                    let inv_guard = inventory.lock().unwrap();
                    encode_to_bytes(&inv_guard.to_crafting_container_set_content(click.window_id.0))
                }
            {
                let _ = tx.try_send(bytes);
            } else if let Some(ref mut c) = containers
                && let Some(pos) = (*open_chest_pos)
                && let Some((chest_slots, chest_comps)) =
                    c.get_chest_and_components_mut_in(pos.0, pos.1, pos.2, pos.3)
                && let Ok(bytes) = {
                    let inv_guard = inventory.lock().unwrap();
                    encode_to_bytes(&inv_guard.to_chest_container_set_content_stacks(
                        click.window_id.0,
                        chest_slots,
                        chest_comps,
                    ))
                }
            {
                let _ = tx.try_send(bytes);
            }
            drop(containers);
            sync_player_inventory(state, entity_id, inventory, tx).await;
        }
    }
}

pub(super) async fn handle_container_close(
    state: &Arc<ServerState>,
    entity_id: i32,
    inventory: &Arc<std::sync::Mutex<PlayerInventory>>,
    tx: &OutboundSender,
    open_chest_pos: &mut Option<(DimensionId, i32, i32, i32)>,
    open_furnace_pos: &mut Option<(DimensionId, i32, i32, i32)>,
    close: ContainerClose,
) {
    if close.window_id.0 > 0 {
        if close.window_id.0 == crate::gameplay::inventory::CRAFTING_WINDOW_ID {
            inventory.lock().unwrap().close_craft3x3();
            sync_player_inventory(state, entity_id, inventory, tx).await;
        }
        *open_chest_pos = None;
        *open_furnace_pos = None;
        if let Some(p) = state.players.write().await.get_mut(&entity_id) {
            p.open_chest = None;
        }
    }
}
