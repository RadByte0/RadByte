//! World simulation: scheduled block ticks, gravity blocks, fluid spread, crops, redstone, and furnaces.

mod furnace;
mod redstone;

pub use furnace::FurnaceEntity;
pub use furnace::FurnaceStore;
pub use redstone::{RedstoneWireState, WireConnection};
use redstone::{
    button_is_powered_with, button_toggle_state_with, powers_neighbor_with, repeater_info_with,
    wall_torch_support_offset_and_item_with,
};
pub(crate) use redstone::{
    floor_dependent_item_with, is_redstone_wire_with, orient_block_on_place_with,
    supports_redstone_with,
};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::Path;

use rad_core::packet::play::SimpleSlot;
use serde::{Deserialize, Serialize};

use crate::registries::Version26_3Registries;
use crate::world::WorldState;
use crate::world::anvil::MIN_WORLD_Y;
use crate::world::worldgen::DimensionId;

fn is_gravity_block(state: u16, regs: &Version26_3Registries) -> bool {
    regs.block_by_state(state).is_some_and(|block| {
        let name = block.name.strip_prefix("minecraft:").unwrap_or(&block.name);
        matches!(
            name,
            "sand" | "red_sand" | "gravel" | "anvil" | "chipped_anvil" | "damaged_anvil"
        ) || name.ends_with("_concrete_powder")
    })
}

fn is_fall_through(state: u16, regs: &Version26_3Registries) -> bool {
    regs.is_air_state(state) || regs.is_fluid_state(state)
}

const MAX_FLUID_SPREAD_DEPTH: u8 = 3;
const MAX_SCHEDULED_PER_TICK: usize = 256;
const MAX_SCANNED_PER_TICK: usize = 4096;
const MAX_PENDING_SIM_TASKS: usize = 65_536;
const MAX_SUSPENDED_PER_CHUNK: usize = 512;

/// Optional chunk-level simulation ticket (for forced-loaded or temporary portal regions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulationTicket {
    /// Target dimension.
    pub dimension: DimensionId,
    /// Ticket center chunk X.
    pub chunk_x: i32,
    /// Ticket center chunk Z.
    pub chunk_z: i32,
    /// Chebyshev chunk radius around `(chunk_x, chunk_z)`.
    pub radius: i32,
}

/// Player-centered and ticket-backed simulation regions, separated by dimension.
#[derive(Debug, Default, Clone)]
pub struct SimulationArea {
    centers: Vec<(DimensionId, i32, i32)>,
    radius: i32,
    tickets: Vec<SimulationTicket>,
    all_active: bool,
}

impl SimulationArea {
    /// Builds a simulation area from `radius` (in chunks) and player positions `(dim, x, z)`.
    pub fn new(radius: i32, players: impl IntoIterator<Item = (DimensionId, f64, f64)>) -> Self {
        let centers = players
            .into_iter()
            .filter_map(|(dim, x, z)| {
                if !x.is_finite() || !z.is_finite() {
                    return None;
                }
                Some((dim, (x / 16.0).floor() as i32, (z / 16.0).floor() as i32))
            })
            .collect();
        Self {
            centers,
            radius: radius.max(0),
            tickets: Vec::new(),
            all_active: false,
        }
    }

    /// Attaches explicit chunk-level simulation tickets to this area.
    #[must_use]
    #[allow(dead_code)]
    pub fn with_tickets(mut self, tickets: impl IntoIterator<Item = SimulationTicket>) -> Self {
        self.tickets.extend(tickets);
        self
    }

    /// Unbounded area used by legacy callers/tests that do not filter by players.
    #[must_use]
    pub fn all_active() -> Self {
        Self {
            all_active: true,
            ..Self::default()
        }
    }

    /// Returns `true` if no player centers or tickets are active.
    #[must_use]
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        !self.all_active && self.centers.is_empty() && self.tickets.is_empty()
    }

    /// Returns `true` if chunk `(cx, cz)` in `dim` is within the active simulation area.
    /// Does not load any chunks.
    #[must_use]
    pub fn contains_chunk(&self, dim: DimensionId, cx: i32, cz: i32) -> bool {
        if self.all_active {
            return true;
        }
        let in_player = self.centers.iter().any(|&(player_dim, px, pz)| {
            player_dim == dim
                && i64::from(cx).abs_diff(i64::from(px)) <= self.radius as u64
                && i64::from(cz).abs_diff(i64::from(pz)) <= self.radius as u64
        });
        if in_player {
            return true;
        }
        self.tickets.iter().any(|t| {
            let r = t.radius.max(0) as u64;
            t.dimension == dim
                && i64::from(cx).abs_diff(i64::from(t.chunk_x)) <= r
                && i64::from(cz).abs_diff(i64::from(t.chunk_z)) <= r
        })
    }

    /// Returns `true` if block `(x, z)` in `dim` lies inside an active simulation chunk.
    #[must_use]
    pub fn contains_block(&self, dim: DimensionId, x: i32, z: i32) -> bool {
        self.contains_chunk(dim, x.div_euclid(16), z.div_euclid(16))
    }

    /// Returns `true` if entity position `(x, z)` in `dim` lies inside an active simulation chunk.
    #[must_use]
    pub fn contains_pos(&self, dim: DimensionId, x: f64, z: f64) -> bool {
        if self.all_active {
            return true;
        }
        if !x.is_finite() || !z.is_finite() {
            return false;
        }
        self.contains_chunk(dim, (x / 16.0).floor() as i32, (z / 16.0).floor() as i32)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum SimAction {
    GravityFall,
    FluidSpread { depth: u8 },
    CropGrow,
    FireExtinguish,
    WireSupportCheck,
    FloorSupportCheck,
    WallSupportCheck,
    ButtonRelease,
    RepeaterTick,
}

impl SimAction {
    const fn kind_id(self) -> u8 {
        match self {
            Self::GravityFall => 0,
            Self::FluidSpread { .. } => 1,
            Self::CropGrow => 2,
            Self::FireExtinguish => 3,
            Self::WireSupportCheck => 4,
            Self::FloorSupportCheck => 5,
            Self::WallSupportCheck => 6,
            Self::ButtonRelease => 7,
            Self::RepeaterTick => 8,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ScheduledEntry {
    due_tick: u64,
    dim: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    action: SimAction,
}

type SimQueue = BTreeMap<u64, VecDeque<(DimensionId, i32, i32, i32, SimAction)>>;
type SuspendedSimQueue = HashMap<(DimensionId, i32, i32), VecDeque<(i32, i32, i32, SimAction)>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedScheduledTask {
    #[serde(default)]
    dimension: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    delay: u64,
    action: SimAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedSuspendedTask {
    #[serde(default)]
    dimension: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    action: SimAction,
}

/// Atomic persistence snapshot for [`WorldSim`] (`world/sim_queue.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SimSnapshot {
    #[serde(default)]
    scheduled: Vec<SavedScheduledTask>,
    #[serde(default)]
    suspended: Vec<SavedSuspendedTask>,
    #[serde(default)]
    falling_blocks: Vec<FallingBlockEntity>,
}

/// Maximum number of active falling-block entities per world.
pub const MAX_FALLING_BLOCKS: usize = 256;

/// Server-side falling block entity (`minecraft:falling_block`, entity type `52`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FallingBlockEntity {
    /// Network entity ID.
    pub entity_id: i32,
    /// Entity UUID.
    pub uuid: uuid::Uuid,
    /// Dimension of the falling block.
    pub dim: DimensionId,
    /// World X coordinate.
    pub x: f64,
    /// World Y coordinate.
    pub y: f64,
    /// World Z coordinate.
    pub z: f64,
    /// Vertical velocity (blocks/tick).
    pub vy: f64,
    /// Block state ID carried by this falling block.
    pub block_state: u16,
    /// Age in ticks since spawn.
    pub age_ticks: u32,
}

/// Deterministic scheduled-tick simulation engine for world blocks.
#[derive(Debug, Default)]
pub struct WorldSim {
    queue: SimQueue,
    suspended: SuspendedSimQueue,
    pending_keys: HashSet<(DimensionId, i32, i32, i32, u8)>,
    total_pending: usize,
    dirty: bool,
    revision: u64,
    /// Active falling-block entities.
    pub falling_blocks: Vec<FallingBlockEntity>,
    next_falling_entity_id: i32,
    pending_block_drops: Vec<(DimensionId, i32, i32, i32, i32)>,
    last_tick: u64,
}

impl WorldSim {
    /// Schedules a simulation tick for `(x, y, z)` and its immediate 6 neighbors in `dim`.
    pub fn on_block_changed_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
        cur_tick: u64,
    ) {
        self.last_tick = cur_tick;
        let coords = [
            (x, y, z),
            (x, y + 1, z),
            (x, y - 1, z),
            (x + 1, y, z),
            (x - 1, y, z),
            (x, y, z + 1),
            (x, y, z - 1),
        ];
        for (bx, by, bz) in coords {
            if by < MIN_WORLD_Y {
                continue;
            }
            let regs = std::sync::Arc::clone(world.registries());
            let state = world.get_block_in(dim, bx, by, bz);
            let water_source = regs.blocks.water.default_state;
            let fire_state = regs.blocks.fire.default_state;
            let wheat_min = regs.blocks.wheat.min_state;
            let wheat_max = regs.blocks.wheat.max_state;
            match state {
                s if is_redstone_wire_with(s, &regs) => {
                    self.push(ScheduledEntry {
                        due_tick: cur_tick.saturating_add(1),
                        dim,
                        x: bx,
                        y: by,
                        z: bz,
                        action: SimAction::WireSupportCheck,
                    });
                }
                s if floor_dependent_item_with(s, &regs).is_some() => {
                    self.push(ScheduledEntry {
                        due_tick: cur_tick.saturating_add(1),
                        dim,
                        x: bx,
                        y: by,
                        z: bz,
                        action: SimAction::FloorSupportCheck,
                    });
                }
                s if wall_torch_support_offset_and_item_with(s, &regs).is_some() => {
                    self.push(ScheduledEntry {
                        due_tick: cur_tick.saturating_add(1),
                        dim,
                        x: bx,
                        y: by,
                        z: bz,
                        action: SimAction::WallSupportCheck,
                    });
                }
                s if is_gravity_block(s, &regs) => {
                    if by > MIN_WORLD_Y
                        && is_fall_through(world.get_block_in(dim, bx, by - 1, bz), &regs)
                    {
                        self.push(ScheduledEntry {
                            due_tick: cur_tick + 2,
                            dim,
                            x: bx,
                            y: by,
                            z: bz,
                            action: SimAction::GravityFall,
                        });
                    }
                }
                s if s == water_source => {
                    self.push(ScheduledEntry {
                        due_tick: cur_tick + 4,
                        dim,
                        x: bx,
                        y: by,
                        z: bz,
                        action: SimAction::FluidSpread { depth: 0 },
                    });
                }
                s if s == fire_state => {
                    if by > MIN_WORLD_Y && world.get_block_in(dim, bx, by - 1, bz) == 0 {
                        self.push(ScheduledEntry {
                            due_tick: cur_tick + 3,
                            dim,
                            x: bx,
                            y: by,
                            z: bz,
                            action: SimAction::FireExtinguish,
                        });
                    }
                }
                s if (wheat_min..wheat_max).contains(&s) => {
                    self.push(ScheduledEntry {
                        due_tick: cur_tick + 20,
                        dim,
                        x: bx,
                        y: by,
                        z: bz,
                        action: SimAction::CropGrow,
                    });
                }
                _ => {}
            }
            if let Some((delay, _, _, _)) = repeater_info_with(state, &regs) {
                self.push(ScheduledEntry {
                    due_tick: cur_tick.saturating_add(u64::from(delay) * 2),
                    dim,
                    x: bx,
                    y: by,
                    z: bz,
                    action: SimAction::RepeaterTick,
                });
            }
        }
    }

    /// Overworld wrapper for [`Self::on_block_changed_in`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn on_block_changed(
        &mut self,
        world: &mut WorldState,
        x: i32,
        y: i32,
        z: i32,
        cur_tick: u64,
    ) {
        self.on_block_changed_in(world, DimensionId::Overworld, x, y, z, cur_tick);
    }

    fn push(&mut self, entry: ScheduledEntry) {
        if self.total_pending >= MAX_PENDING_SIM_TASKS {
            return;
        }
        let key = (entry.dim, entry.x, entry.y, entry.z, entry.action.kind_id());
        if !self.pending_keys.insert(key) {
            return;
        }
        self.total_pending += 1;
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        self.queue.entry(entry.due_tick).or_default().push_back((
            entry.dim,
            entry.x,
            entry.y,
            entry.z,
            entry.action,
        ));
    }

    /// Loads `<world_dir>/sim_queue.json` if present, quarantining corrupt files safely.
    #[must_use]
    pub fn load_from_dir(world_dir: &Path) -> Self {
        let path = world_dir.join("sim_queue.json");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let snap = match serde_json::from_str::<SimSnapshot>(&raw) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "corrupt sim_queue.json quarantined"
                );
                let _ = std::fs::rename(&path, world_dir.join("sim_queue.json.corrupt"));
                return Self::default();
            }
        };
        let mut sim = Self::default();
        for task in snap.scheduled {
            if sim.total_pending >= MAX_PENDING_SIM_TASKS || task.y < MIN_WORLD_Y {
                continue;
            }
            let key = (
                task.dimension,
                task.x,
                task.y,
                task.z,
                task.action.kind_id(),
            );
            if !sim.pending_keys.insert(key) {
                continue;
            }
            sim.total_pending += 1;
            sim.queue.entry(task.delay).or_default().push_back((
                task.dimension,
                task.x,
                task.y,
                task.z,
                task.action,
            ));
        }
        for task in snap.suspended {
            if sim.total_pending >= MAX_PENDING_SIM_TASKS || task.y < MIN_WORLD_Y {
                continue;
            }
            let chunk_key = (task.dimension, task.x.div_euclid(16), task.z.div_euclid(16));
            let bucket = sim.suspended.entry(chunk_key).or_default();
            if bucket.len() >= MAX_SUSPENDED_PER_CHUNK {
                continue;
            }
            let key = (
                task.dimension,
                task.x,
                task.y,
                task.z,
                task.action.kind_id(),
            );
            if !sim.pending_keys.insert(key) {
                continue;
            }
            sim.total_pending += 1;
            bucket.push_back((task.x, task.y, task.z, task.action));
        }
        sim.falling_blocks = snap.falling_blocks;
        if let Some(max_id) = sim
            .falling_blocks
            .iter()
            .filter_map(|fb| fb.entity_id.checked_sub(900_000))
            .max()
        {
            sim.next_falling_entity_id = max_id.saturating_add(1);
        }
        sim
    }

    /// Snapshots pending and suspended simulation tasks if dirty since last save.
    #[must_use]
    pub fn snapshot_if_dirty(&self, cur_tick: u64) -> Option<(u64, SimSnapshot)> {
        if !self.dirty && self.falling_blocks.is_empty() {
            return None;
        }
        let mut scheduled = Vec::new();
        for (&due_tick, bucket) in &self.queue {
            let delay = due_tick.saturating_sub(cur_tick);
            for &(dimension, x, y, z, action) in bucket {
                scheduled.push(SavedScheduledTask {
                    dimension,
                    x,
                    y,
                    z,
                    delay,
                    action,
                });
            }
        }
        let mut suspended = Vec::new();
        for (&(dimension, _, _), bucket) in &self.suspended {
            for &(x, y, z, action) in bucket {
                suspended.push(SavedSuspendedTask {
                    dimension,
                    x,
                    y,
                    z,
                    action,
                });
            }
        }
        Some((
            self.revision,
            SimSnapshot {
                scheduled,
                suspended,
                falling_blocks: self.falling_blocks.clone(),
            },
        ))
    }

    /// Clears the dirty flag after save succeeds only if no newer mutation occurred.
    pub fn mark_saved(&mut self, expected_rev: u64) {
        if self.revision == expected_rev {
            self.dirty = false;
        }
    }

    /// Atomically writes `world/sim_queue.json`.
    pub fn write_snapshot_atomic(world_dir: &Path, snap: &SimSnapshot) -> std::io::Result<()> {
        std::fs::create_dir_all(world_dir)?;
        let path = world_dir.join("sim_queue.json");
        let tmp = world_dir.join(format!("sim_queue.json.{}.tmp", std::process::id()));
        let json = serde_json::to_string_pretty(snap)
            .map_err(|e| std::io::Error::other(format!("sim_queue serialize failed: {e}")))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Returns total number of pending (queued + suspended) tasks.
    #[cfg(test)]
    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.total_pending
    }

    /// Returns number of tasks currently suspended outside active simulation areas.
    #[cfg(test)]
    #[must_use]
    pub fn suspended_count(&self) -> usize {
        self.suspended.values().map(VecDeque::len).sum()
    }

    /// Executes all scheduled updates due at or before `cur_tick` (all chunks active).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn step(
        &mut self,
        world: &mut WorldState,
        cur_tick: u64,
    ) -> Vec<(DimensionId, i32, i32, i32, u16)> {
        self.step_in_area(world, cur_tick, &SimulationArea::all_active())
    }

    /// Executes due scheduled updates within `area`, suspending inactive chunks without
    /// loading them and without letting deferred tasks starve active chunks.
    pub fn step_in_area(
        &mut self,
        world: &mut WorldState,
        cur_tick: u64,
        area: &SimulationArea,
    ) -> Vec<(DimensionId, i32, i32, i32, u16)> {
        self.last_tick = cur_tick;
        // Resume suspended chunks that have entered the active simulation area.
        let active_chunks: Vec<(DimensionId, i32, i32)> = self
            .suspended
            .keys()
            .copied()
            .filter(|&(dim, cx, cz)| area.contains_chunk(dim, cx, cz))
            .collect();
        if !active_chunks.is_empty() {
            let bucket = self.queue.entry(cur_tick).or_default();
            for chunk_key in active_chunks {
                if let Some(tasks) = self.suspended.remove(&chunk_key) {
                    for (x, y, z, action) in tasks {
                        bucket.push_back((chunk_key.0, x, y, z, action));
                    }
                    self.dirty = true;
                    self.revision = self.revision.wrapping_add(1);
                }
            }
        }

        let mut updates = Vec::new();
        let mut active_processed = 0usize;
        let mut scanned = 0usize;

        while active_processed < MAX_SCHEDULED_PER_TICK && scanned < MAX_SCANNED_PER_TICK {
            let Some(&first_tick) = self.queue.keys().next() else {
                break;
            };
            if first_tick > cur_tick {
                break;
            }
            let Some(mut bucket) = self.queue.remove(&first_tick) else {
                break;
            };
            while let Some((dim, x, y, z, action)) = bucket.pop_front() {
                scanned += 1;
                if !area.contains_block(dim, x, z) {
                    let chunk_key = (dim, x.div_euclid(16), z.div_euclid(16));
                    let sus_bucket = self.suspended.entry(chunk_key).or_default();
                    if sus_bucket.len() < MAX_SUSPENDED_PER_CHUNK {
                        sus_bucket.push_back((x, y, z, action));
                    } else {
                        self.pending_keys.remove(&(dim, x, y, z, action.kind_id()));
                        self.total_pending = self.total_pending.saturating_sub(1);
                    }
                    self.dirty = true;
                    self.revision = self.revision.wrapping_add(1);
                    if scanned >= MAX_SCANNED_PER_TICK {
                        break;
                    }
                    continue;
                }

                self.pending_keys.remove(&(dim, x, y, z, action.kind_id()));
                self.total_pending = self.total_pending.saturating_sub(1);
                self.dirty = true;
                self.revision = self.revision.wrapping_add(1);
                active_processed += 1;
                match action {
                    SimAction::GravityFall => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let cur = world.get_block_in(dim, x, y, z);
                        let below = world.get_block_in(dim, x, y - 1, z);
                        if is_gravity_block(cur, &regs)
                            && y > MIN_WORLD_Y
                            && is_fall_through(below, &regs)
                            && self.falling_blocks.len() < MAX_FALLING_BLOCKS
                            && world.set_block_in(dim, x, y, z, 0)
                        {
                            updates.push((dim, x, y, z, 0));
                            self.next_falling_entity_id =
                                self.next_falling_entity_id.wrapping_sub(1).min(-100_000);
                            let entity_id = self.next_falling_entity_id;
                            self.falling_blocks.push(FallingBlockEntity {
                                entity_id,
                                uuid: uuid::Uuid::from_u128(
                                    (entity_id as u32 as u128)
                                        | 0x4641_4c4c_424c_4b00_0000_0000_0000_0000_u128,
                                ),
                                dim,
                                x: f64::from(x) + 0.5,
                                y: f64::from(y),
                                z: f64::from(z) + 0.5,
                                vy: 0.0,
                                block_state: cur,
                                age_ticks: 0,
                            });
                            self.on_block_changed_in(world, dim, x, y, z, cur_tick);
                        }
                    }
                    SimAction::FluidSpread { depth } => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let water_source = regs.blocks.water.default_state;
                        let water_flow =
                            (regs.blocks.water.default_state + 1).min(regs.blocks.water.max_state);
                        let cur = world.get_block_in(dim, x, y, z);
                        if cur != water_source && cur != water_flow {
                            continue;
                        }
                        if y > MIN_WORLD_Y && world.get_block_in(dim, x, y - 1, z) == 0 {
                            if world.set_block_in(dim, x, y - 1, z, water_flow) {
                                updates.push((dim, x, y - 1, z, water_flow));
                                self.push(ScheduledEntry {
                                    due_tick: cur_tick + 4,
                                    dim,
                                    x,
                                    y: y - 1,
                                    z,
                                    action: SimAction::FluidSpread { depth: 0 },
                                });
                            }
                        } else if depth < MAX_FLUID_SPREAD_DEPTH {
                            for (nx, nz) in [(x + 1, z), (x - 1, z), (x, z + 1), (x, z - 1)] {
                                if world.get_block_in(dim, nx, y, nz) == 0
                                    && world.set_block_in(dim, nx, y, nz, water_flow)
                                {
                                    updates.push((dim, nx, y, nz, water_flow));
                                    self.push(ScheduledEntry {
                                        due_tick: cur_tick + 4,
                                        dim,
                                        x: nx,
                                        y,
                                        z: nz,
                                        action: SimAction::FluidSpread { depth: depth + 1 },
                                    });
                                }
                            }
                        }
                    }
                    SimAction::CropGrow => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let wheat_min = regs.blocks.wheat.min_state;
                        let wheat_max = regs.blocks.wheat.max_state;
                        let cur = world.get_block_in(dim, x, y, z);
                        if (wheat_min..wheat_max).contains(&cur)
                            && world.set_block_in(dim, x, y, z, cur + 1)
                        {
                            updates.push((dim, x, y, z, cur + 1));
                            if cur + 1 < wheat_max {
                                self.push(ScheduledEntry {
                                    due_tick: cur_tick + 20,
                                    dim,
                                    x,
                                    y,
                                    z,
                                    action: SimAction::CropGrow,
                                });
                            }
                        }
                    }
                    SimAction::FireExtinguish => {
                        let fire_state = world.registries().blocks.fire.default_state;
                        if world.get_block_in(dim, x, y, z) == fire_state
                            && (y <= MIN_WORLD_Y || world.get_block_in(dim, x, y - 1, z) == 0)
                            && world.set_block_in(dim, x, y, z, 0)
                        {
                            updates.push((dim, x, y, z, 0));
                        }
                    }
                    SimAction::WireSupportCheck => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let wire = world.get_block_in(dim, x, y, z);
                        if is_redstone_wire_with(wire, &regs)
                            && (y <= MIN_WORLD_Y
                                || !supports_redstone_with(
                                    world.get_block_in(dim, x, y - 1, z),
                                    &regs,
                                ))
                            && world.set_block_in(dim, x, y, z, 0)
                        {
                            updates.push((dim, x, y, z, 0));
                            self.pending_block_drops
                                .push((dim, x, y, z, regs.items.redstone));
                            self.on_block_changed_in(world, dim, x, y, z, cur_tick);
                            updates.extend(
                                self.update_redstone_around_in(world, dim, x, y, z)
                                    .into_iter()
                                    .map(|(bx, by, bz, s)| (dim, bx, by, bz, s)),
                            );
                        }
                    }
                    SimAction::WallSupportCheck => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let state = world.get_block_in(dim, x, y, z);
                        if let Some(((dx, dy, dz), item_id)) =
                            wall_torch_support_offset_and_item_with(state, &regs)
                        {
                            let support_y = y + dy;
                            let supported = support_y >= MIN_WORLD_Y
                                && supports_redstone_with(
                                    world.get_block_in(dim, x + dx, support_y, z + dz),
                                    &regs,
                                );
                            if !supported && world.set_block_in(dim, x, y, z, 0) {
                                updates.push((dim, x, y, z, 0));
                                self.pending_block_drops.push((dim, x, y, z, item_id));
                                self.on_block_changed_in(world, dim, x, y, z, cur_tick);
                                updates.extend(
                                    self.update_redstone_around_in(world, dim, x, y, z)
                                        .into_iter()
                                        .map(|(bx, by, bz, s)| (dim, bx, by, bz, s)),
                                );
                            }
                        }
                    }
                    SimAction::ButtonRelease => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let state = world.get_block_in(dim, x, y, z);
                        if button_is_powered_with(state, &regs) == Some(true)
                            && let Some(off_state) = button_toggle_state_with(state, &regs)
                            && world.set_block_in(dim, x, y, z, off_state)
                        {
                            updates.push((dim, x, y, z, off_state));
                            updates.extend(
                                self.update_redstone_around_in(world, dim, x, y, z)
                                    .into_iter()
                                    .map(|(bx, by, bz, s)| (dim, bx, by, bz, s)),
                            );
                        }
                    }
                    SimAction::RepeaterTick => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let state = world.get_block_in(dim, x, y, z);
                        if let Some((_delay, facing, _locked, powered)) =
                            repeater_info_with(state, &regs)
                        {
                            let (bx, bz) = match facing {
                                0 => (x, z + 1), // facing north -> input from south
                                1 => (x, z - 1), // facing south -> input from north
                                2 => (x + 1, z), // facing west -> input from east
                                _ => (x - 1, z), // facing east -> input from west
                            };
                            let back = world.get_block_in(dim, bx, y, bz);
                            let should_power = powers_neighbor_with(back, x - bx, 0, z - bz, &regs)
                                || RedstoneWireState::from_state_id_with(back, &regs)
                                    .is_some_and(|w| w.power > 0);
                            if should_power != powered {
                                let next = if should_power { state - 1 } else { state + 1 };
                                if world.set_block_in(dim, x, y, z, next) {
                                    updates.push((dim, x, y, z, next));
                                    updates.extend(
                                        self.update_redstone_around_in(world, dim, x, y, z)
                                            .into_iter()
                                            .map(|(rx, ry, rz, s)| (dim, rx, ry, rz, s)),
                                    );
                                }
                            }
                        }
                    }
                    SimAction::FloorSupportCheck => {
                        let regs = std::sync::Arc::clone(world.registries());
                        let state = world.get_block_in(dim, x, y, z);
                        if let Some(item_id) = floor_dependent_item_with(state, &regs)
                            && (y <= MIN_WORLD_Y
                                || !supports_redstone_with(
                                    world.get_block_in(dim, x, y - 1, z),
                                    &regs,
                                ))
                            && world.set_block_in(dim, x, y, z, 0)
                        {
                            updates.push((dim, x, y, z, 0));
                            self.pending_block_drops.push((dim, x, y, z, item_id));
                            self.on_block_changed_in(world, dim, x, y, z, cur_tick);
                            updates.extend(
                                self.update_redstone_around_in(world, dim, x, y, z)
                                    .into_iter()
                                    .map(|(bx, by, bz, s)| (dim, bx, by, bz, s)),
                            );
                        }
                    }
                }
                if active_processed >= MAX_SCHEDULED_PER_TICK || scanned >= MAX_SCANNED_PER_TICK {
                    break;
                }
            }
            if !bucket.is_empty() {
                self.queue.insert(first_tick, bucket);
            }
        }
        updates
    }

    /// Takes item drops produced when support-dependent blocks break.
    pub fn take_support_drops(&mut self) -> Vec<(DimensionId, i32, i32, i32, i32)> {
        std::mem::take(&mut self.pending_block_drops)
    }

    /// Advances all active falling-block entities by one tick, handling gravity, water hardening, and landing.
    #[allow(clippy::type_complexity)]
    pub fn tick_falling_blocks(
        &mut self,
        world: &mut WorldState,
        cur_tick: u64,
    ) -> (Vec<(DimensionId, i32, i32, i32, u16)>, Vec<i32>) {
        let mut block_updates = Vec::new();
        let mut removed_ids = Vec::new();
        let mut remaining = Vec::with_capacity(self.falling_blocks.len());

        let entities = std::mem::take(&mut self.falling_blocks);
        for mut fb in entities {
            fb.age_ticks = fb.age_ticks.saturating_add(1);
            fb.vy -= 0.04;
            fb.vy *= 0.98;
            fb.y += fb.vy;

            let bx = fb.x.floor() as i32;
            let by = fb.y.floor() as i32;
            let bz = fb.z.floor() as i32;

            if by < MIN_WORLD_Y || fb.age_ticks > 600 {
                removed_ids.push(fb.entity_id);
                continue;
            }

            let regs = std::sync::Arc::clone(world.registries());
            let at_pos = world.get_block_in(fb.dim, bx, by, bz);
            let hardened = regs.block_by_state(fb.block_state).and_then(|block| {
                let in_water = regs
                    .block_by_state(at_pos)
                    .is_some_and(|b| b.name == "minecraft:water");
                in_water
                    .then(|| block.name.strip_suffix("_concrete_powder"))
                    .flatten()
                    .and_then(|color| regs.block_by_name(&format!("{color}_concrete")))
                    .map(|b| b.default_state)
            });
            if let Some(concrete_state) = hardened {
                if world.set_block_in(fb.dim, bx, by, bz, concrete_state) {
                    block_updates.push((fb.dim, bx, by, bz, concrete_state));
                    self.on_block_changed_in(world, fb.dim, bx, by, bz, cur_tick);
                }
                removed_ids.push(fb.entity_id);
                continue;
            }

            if !is_fall_through(at_pos, &regs) {
                let supports_top = rad_version::shapes::block_supports_top_face_with(
                    at_pos,
                    regs.provider().as_ref(),
                )
                .unwrap_or(true);
                if !supports_top {
                    let empty_collision = rad_version::shapes::block_collision_shape_with(
                        at_pos,
                        regs.provider().as_ref(),
                    )
                    .is_some_and(|s| s.as_boxes().is_empty());
                    let drop_y = if empty_collision { by } else { by + 1 };
                    if let Some(item_id) = regs
                        .block_by_state(fb.block_state)
                        .and_then(|b| regs.item_by_name(&b.name))
                    {
                        self.pending_block_drops
                            .push((fb.dim, bx, drop_y, bz, item_id));
                    }
                    self.dirty = true;
                    self.revision = self.revision.wrapping_add(1);
                    removed_ids.push(fb.entity_id);
                    continue;
                }
                let land_y = by + 1;
                let land_cur = world.get_block_in(fb.dim, bx, land_y, bz);
                if is_fall_through(land_cur, &regs)
                    && world.set_block_in(fb.dim, bx, land_y, bz, fb.block_state)
                {
                    block_updates.push((fb.dim, bx, land_y, bz, fb.block_state));
                    self.on_block_changed_in(world, fb.dim, bx, land_y, bz, cur_tick);
                }
                self.dirty = true;
                self.revision = self.revision.wrapping_add(1);
                removed_ids.push(fb.entity_id);
                continue;
            }

            remaining.push(fb);
        }

        self.falling_blocks = remaining;
        (block_updates, removed_ids)
    }
}
