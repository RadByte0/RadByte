//! Redstone wiring, pistons, switches, repeaters, TNT, and block placement orientation.

use super::*;

/// Redstone wire connection shape for each horizontal direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum WireConnection {
    /// Connected up onto an adjacent block (`up`).
    Up = 0,
    /// Connected horizontally or down (`side`).
    Side = 1,
    /// Not connected (`none`).
    None = 2,
}

impl WireConnection {
    const fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Up,
            1 => Self::Side,
            _ => Self::None,
        }
    }
}

/// Decoded `minecraft:redstone_wire` block state (`5567..=6862`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedstoneWireState {
    /// East connection.
    pub east: WireConnection,
    /// North connection.
    pub north: WireConnection,
    /// Signal power (`0..=15`).
    pub power: u8,
    /// South connection.
    pub south: WireConnection,
    /// West connection.
    pub west: WireConnection,
}

impl RedstoneWireState {
    /// Decodes a `minecraft:redstone_wire` state ID using `regs`.
    #[must_use]
    pub const fn from_state_id_with(state: u16, regs: &Version26_3Registries) -> Option<Self> {
        let min = regs.blocks.redstone_wire.min_state;
        let max = regs.blocks.redstone_wire.max_state;
        if state < min || state > max {
            return None;
        }
        let mut idx = state - min;
        let west = WireConnection::from_u8((idx % 3) as u8);
        idx /= 3;
        let south = WireConnection::from_u8((idx % 3) as u8);
        idx /= 3;
        let power = (idx % 16) as u8;
        idx /= 16;
        let north = WireConnection::from_u8((idx % 3) as u8);
        idx /= 3;
        let east = WireConnection::from_u8((idx % 3) as u8);
        Some(Self {
            east,
            north,
            power,
            south,
            west,
        })
    }

    /// Encodes back into a `minecraft:redstone_wire` state ID using `regs`.
    #[must_use]
    pub const fn to_state_id_with(self, regs: &Version26_3Registries) -> u16 {
        let p = if self.power > 15 { 15 } else { self.power } as u16;
        regs.blocks.redstone_wire.min_state
            + (self.east as u16) * 432
            + (self.north as u16) * 144
            + p * 9
            + (self.south as u16) * 3
            + (self.west as u16)
    }
}

pub(crate) fn piston_info_with(
    state: u16,
    regs: &Version26_3Registries,
) -> Option<(bool, bool, usize)> {
    let block = regs.block_by_state(state)?;
    let sticky = match block.name.as_str() {
        "minecraft:sticky_piston" => true,
        "minecraft:piston" => false,
        _ => return None,
    };
    let rel = usize::from(state - block.min_state);
    let extended = rel < 6;
    let facing_idx = rel % 6;
    Some((sticky, extended, facing_idx))
}

const fn piston_facing_offset(facing_idx: usize) -> (i32, i32, i32) {
    match facing_idx {
        0 => (0, 0, -1), // north
        1 => (1, 0, 0),  // east
        2 => (0, 0, 1),  // south
        3 => (-1, 0, 0), // west
        4 => (0, 1, 0),  // up
        _ => (0, -1, 0), // down
    }
}

fn is_piston_immovable_with(state: u16, regs: &Version26_3Registries) -> bool {
    if state == 0 {
        return false;
    }
    if let Some((_, extended, _)) = piston_info_with(state, regs) {
        return extended;
    }
    matches!(
        regs.block_by_state(state).map(|b| b.name).as_deref(),
        Some(
            "minecraft:bedrock"
                | "minecraft:obsidian"
                | "minecraft:crying_obsidian"
                | "minecraft:reinforced_deepslate"
                | "minecraft:barrier"
                | "minecraft:piston_head"
                | "minecraft:chest"
                | "minecraft:trapped_chest"
                | "minecraft:barrel"
                | "minecraft:furnace"
                | "minecraft:hopper"
                | "minecraft:dispenser"
                | "minecraft:dropper"
        )
    )
}

/// Computes the oriented block state ID using `regs` when a player places `raw_state`.
pub(crate) fn orient_block_on_place_with(
    raw_state: u16,
    yaw: f32,
    pitch: f32,
    face: i32,
    cursor_y: f32,
    regs: &Version26_3Registries,
) -> u16 {
    if is_redstone_wire_with(raw_state, regs) {
        return regs.blocks.redstone_wire.default_state;
    }
    let Some(block) = regs.block_by_state(raw_state) else {
        return raw_state;
    };

    // Player horizontal look direction: north=0, south=1, west=2, east=3
    let y = yaw.rem_euclid(360.0);
    let player_nswe: u16 = if !(45.0..315.0).contains(&y) {
        1 // south (+Z)
    } else if y < 135.0 {
        2 // west (-X)
    } else if y < 225.0 {
        0 // north (-Z)
    } else {
        3 // east (+X)
    };
    let opposite_nswe: u16 = match player_nswe {
        0 => 1,
        1 => 0,
        2 => 3,
        _ => 2,
    };

    // 6-way direction: north=0, east=1, south=2, west=3, up=4, down=5
    let player_6way: u16 = if pitch > 45.0 {
        5 // down
    } else if pitch < -45.0 {
        4 // up
    } else {
        match player_nswe {
            0 => 0,
            3 => 1,
            1 => 2,
            _ => 3,
        }
    };
    let opposite_6way: u16 = match player_6way {
        0 => 2,
        1 => 3,
        2 => 0,
        3 => 1,
        4 => 5,
        _ => 4,
    };

    let name = block.name.as_str();
    let min = block.min_state;

    // Wall torches when placed on a horizontal face (2..=5)
    if (2..=5).contains(&face) {
        let wall_facing = (face - 2) as u16; // 2->0(north), 3->1(south), 4->2(west), 5->3(east)
        match name {
            "minecraft:torch" => {
                if let Some(wb) = regs.block_by_name("minecraft:wall_torch") {
                    return wb.min_state + wall_facing;
                }
            }
            "minecraft:soul_torch" => {
                if let Some(wb) = regs.block_by_name("minecraft:soul_wall_torch") {
                    return wb.min_state + wall_facing;
                }
            }
            "minecraft:redstone_torch" => {
                if let Some(wb) = regs.block_by_name("minecraft:redstone_wall_torch") {
                    return wb.min_state + wall_facing * 2; // lit=true (+0)
                }
            }
            _ => {}
        }
    }

    if name.ends_with("_stairs") {
        let top = face == 0 || (face >= 2 && cursor_y > 0.5);
        let half = if top { 0 } else { 1 };
        return min + player_nswe * 20 + half * 10 + 1;
    }
    if name.ends_with("_trapdoor") {
        let top = face == 0 || (face >= 2 && cursor_y > 0.5);
        let half = if top { 0 } else { 1 };
        return min + opposite_nswe * 16 + half * 8 + 4 + 2 + 1;
    }
    if name.ends_with("_door") {
        return min + player_nswe * 16 + 8 + 2 + 1;
    }
    if name.ends_with("_fence_gate") {
        return min + player_nswe * 8 + 4 + 2 + 1;
    }
    if name == "minecraft:chest" || name == "minecraft:trapped_chest" {
        return min + opposite_nswe * 6 + 1;
    }
    if name == "minecraft:furnace"
        || name == "minecraft:blast_furnace"
        || name == "minecraft:smoker"
    {
        return min + opposite_nswe * 2 + 1;
    }
    if name == "minecraft:piston" || name == "minecraft:sticky_piston" {
        return min + 6 + opposite_6way;
    }
    if name == "minecraft:dispenser" || name == "minecraft:dropper" {
        return min + opposite_6way * 2 + 1;
    }
    if name == "minecraft:observer" {
        return min + player_6way * 2 + 1;
    }
    if name == "minecraft:repeater" {
        return min + opposite_nswe * 4 + 2 + 1;
    }
    if name == "minecraft:comparator" {
        return min + opposite_nswe * 4 + 1;
    }
    if name == "minecraft:lever" || name.ends_with("_button") {
        let (attach_face, facing) = match face {
            0 => (2u16, player_nswe),           // ceiling
            1 => (0u16, player_nswe),           // floor
            2..=5 => (1u16, (face - 2) as u16), // wall
            _ => (0u16, player_nswe),
        };
        return min + attach_face * 8 + facing * 2 + 1;
    }

    raw_state
}

pub(crate) fn is_redstone_wire_with(state: u16, regs: &Version26_3Registries) -> bool {
    (regs.blocks.redstone_wire.min_state..=regs.blocks.redstone_wire.max_state).contains(&state)
}

// Only blocks with verified floor-only placement are handled here. Blocks
// attached to walls or with block entities need their own orientation logic.
pub(super) fn wall_torch_support_offset_and_item_with(
    state: u16,
    regs: &Version26_3Registries,
) -> Option<((i32, i32, i32), i32)> {
    let block = regs.block_by_state(state)?;
    let (facing_idx, drop_name) = match block.name.as_str() {
        "minecraft:wall_torch" => (state.checked_sub(block.min_state)?, "minecraft:torch"),
        "minecraft:soul_wall_torch" => {
            (state.checked_sub(block.min_state)?, "minecraft:soul_torch")
        }
        "minecraft:redstone_wall_torch" => (
            state.checked_sub(block.min_state)? / 2,
            "minecraft:redstone_torch",
        ),
        _ => return None,
    };
    let offset = match facing_idx {
        0 => (0, 0, 1),  // facing north -> attached to south
        1 => (0, 0, -1), // facing south -> attached to north
        2 => (1, 0, 0),  // facing west -> attached to east
        3 => (-1, 0, 0), // facing east -> attached to west
        _ => return None,
    };
    let item_id = regs.item_by_name(drop_name)?;
    Some((offset, item_id))
}

pub(crate) fn floor_dependent_item_with(state: u16, regs: &Version26_3Registries) -> Option<i32> {
    let block = regs.block_by_state(state)?;
    let name = block.name.as_str();
    if matches!(
        name,
        "minecraft:torch"
            | "minecraft:soul_torch"
            | "minecraft:redstone_torch"
            | "minecraft:rail"
            | "minecraft:powered_rail"
            | "minecraft:detector_rail"
            | "minecraft:activator_rail"
    ) || name.ends_with("_pressure_plate")
    {
        regs.item_by_name(name)
    } else {
        None
    }
}

pub(crate) fn supports_redstone_with(state: u16, regs: &Version26_3Registries) -> bool {
    crate::world::physics::block_collision_boxes_with(state, regs.provider().as_ref())
        .into_iter()
        .flatten()
        .any(|shape| {
            shape.min_x <= 0.0
                && shape.min_z <= 0.0
                && shape.max_x >= 1.0
                && shape.max_z >= 1.0
                && shape.max_y >= 1.0
        })
}

fn is_button_with(state: u16, regs: &Version26_3Registries) -> bool {
    regs.block_by_state(state)
        .is_some_and(|b| b.name.ends_with("_button"))
}

pub(super) fn button_is_powered_with(state: u16, regs: &Version26_3Registries) -> Option<bool> {
    let block = regs.block_by_state(state)?;
    if !block.name.ends_with("_button") {
        return None;
    }
    Some((state - block.min_state).is_multiple_of(2))
}

pub(super) fn button_toggle_state_with(state: u16, regs: &Version26_3Registries) -> Option<u16> {
    let block = regs.block_by_state(state)?;
    if !block.name.ends_with("_button") {
        return None;
    }
    let rel = state - block.min_state;
    Some(if rel.is_multiple_of(2) {
        state + 1
    } else {
        state - 1
    })
}

fn lever_is_powered_with(state: u16, regs: &Version26_3Registries) -> Option<bool> {
    if (regs.blocks.lever.min_state..=regs.blocks.lever.max_state).contains(&state) {
        Some((state - regs.blocks.lever.min_state).is_multiple_of(2))
    } else {
        None
    }
}

fn lever_toggle_state_with(state: u16, regs: &Version26_3Registries) -> Option<u16> {
    if (regs.blocks.lever.min_state..=regs.blocks.lever.max_state).contains(&state) {
        let offset = state - regs.blocks.lever.min_state;
        Some(regs.blocks.lever.min_state + (offset ^ 1))
    } else {
        None
    }
}

pub(super) fn repeater_info_with(
    state: u16,
    regs: &Version26_3Registries,
) -> Option<(u8, usize, bool, bool)> {
    // minecraft:repeater: 64 states starting at regs.blocks.repeater.min_state
    // properties: delay(1..=4, step 16), facing(north=0,south=1,west=2,east=3, step 4),
    // locked(true=0,false=1, step 2), powered(true=0,false=1, step 1)
    let repeater = regs.block_by_name("minecraft:repeater")?;
    let min = repeater.min_state;
    let max = repeater.max_state;
    if !(min..=max).contains(&state) {
        return None;
    }
    let rel = state - min;
    let delay = ((rel / 16) + 1) as u8;
    let facing = ((rel % 16) / 4) as usize;
    let locked = ((rel % 4) / 2) == 0;
    let powered = rel.is_multiple_of(2);
    Some((delay, facing, locked, powered))
}

fn repeater_front_dir(facing: usize) -> (i32, i32) {
    match facing {
        0 => (0, -1), // north
        1 => (0, 1),  // south
        2 => (-1, 0), // west
        _ => (1, 0),  // east
    }
}

pub(super) fn powers_neighbor_with(
    state: u16,
    dx: i32,
    dy: i32,
    dz: i32,
    regs: &Version26_3Registries,
) -> bool {
    if is_power_source_with(state, regs) {
        return true;
    }
    if let Some((_, facing, _, powered)) = repeater_info_with(state, regs) {
        return powered && dy == 0 && repeater_front_dir(facing) == (dx, dz);
    }
    false
}

fn is_power_source_with(state: u16, regs: &Version26_3Registries) -> bool {
    if lever_is_powered_with(state, regs) == Some(true)
        || button_is_powered_with(state, regs) == Some(true)
        || state == regs.blocks.redstone_torch.default_state
    {
        return true;
    }
    matches!(
        regs.block_by_state(state).map(|b| b.name).as_deref(),
        Some("minecraft:redstone_block" | "minecraft:redstone_wall_torch")
    )
}

fn is_redstone_connectable_with(state: u16, regs: &Version26_3Registries) -> bool {
    if is_redstone_wire_with(state, regs)
        || (regs.blocks.lever.min_state..=regs.blocks.lever.max_state).contains(&state)
        || state == regs.blocks.redstone_torch.default_state
    {
        return true;
    }
    matches!(
        regs.block_by_state(state).map(|b| b.name).as_deref(),
        Some(
            "minecraft:redstone_block"
                | "minecraft:redstone_wall_torch"
                | "minecraft:repeater"
                | "minecraft:comparator"
                | "minecraft:observer"
                | "minecraft:target"
        )
    )
}

fn is_solid_for_wire_with(state: u16, regs: &Version26_3Registries) -> bool {
    !regs.is_air_state(state) && !regs.is_fluid_state(state) && !is_redstone_wire_with(state, regs)
}

fn compute_wire_connections(
    world: &mut WorldState,
    dim: DimensionId,
    x: i32,
    y: i32,
    z: i32,
) -> (
    WireConnection,
    WireConnection,
    WireConnection,
    WireConnection,
) {
    let regs = std::sync::Arc::clone(world.registries());
    let above_solid = is_solid_for_wire_with(world.get_block_in(dim, x, y + 1, z), &regs);
    let conn_dir = |world: &mut WorldState, dx: i32, dz: i32| -> WireConnection {
        let side_blk = world.get_block_in(dim, x + dx, y, z + dz);
        if is_redstone_connectable_with(side_blk, &regs) {
            return WireConnection::Side;
        }
        if !above_solid
            && is_solid_for_wire_with(side_blk, &regs)
            && is_redstone_wire_with(world.get_block_in(dim, x + dx, y + 1, z + dz), &regs)
        {
            return WireConnection::Up;
        }
        if !is_solid_for_wire_with(side_blk, &regs)
            && is_redstone_wire_with(world.get_block_in(dim, x + dx, y - 1, z + dz), &regs)
        {
            return WireConnection::Side;
        }
        WireConnection::None
    };

    let mut north = conn_dir(world, 0, -1);
    let mut east = conn_dir(world, 1, 0);
    let mut south = conn_dir(world, 0, 1);
    let mut west = conn_dir(world, -1, 0);

    // Vanilla straight-line extension when connected on only one side
    let n_on = north != WireConnection::None;
    let e_on = east != WireConnection::None;
    let s_on = south != WireConnection::None;
    let w_on = west != WireConnection::None;
    if n_on && !e_on && !s_on && !w_on {
        south = WireConnection::Side;
    } else if s_on && !n_on && !e_on && !w_on {
        north = WireConnection::Side;
    } else if e_on && !n_on && !s_on && !w_on {
        west = WireConnection::Side;
    } else if w_on && !n_on && !e_on && !s_on {
        east = WireConnection::Side;
    }

    (east, north, south, west)
}

impl WorldSim {
    /// Toggles an isolated redstone wire between small dot (`none` on all sides) and large cross (`side` on all sides).
    pub fn toggle_redstone_wire_dot_cross_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<u16> {
        let regs = std::sync::Arc::clone(world.registries());
        let cur = world.get_block_in(dim, x, y, z);
        let wire = RedstoneWireState::from_state_id_with(cur, &regs)?;
        let auto_conn = compute_wire_connections(world, dim, x, y, z);
        if auto_conn
            != (
                WireConnection::None,
                WireConnection::None,
                WireConnection::None,
                WireConnection::None,
            )
        {
            return None;
        }
        let is_dot = wire.east == WireConnection::None
            && wire.north == WireConnection::None
            && wire.south == WireConnection::None
            && wire.west == WireConnection::None;
        let conn = if is_dot {
            WireConnection::Side
        } else {
            WireConnection::None
        };
        let next = RedstoneWireState {
            east: conn,
            north: conn,
            south: conn,
            west: conn,
            power: wire.power,
        }
        .to_state_id_with(&regs);
        if world.set_block_in(dim, x, y, z, next) {
            Some(next)
        } else {
            None
        }
    }

    /// Extends or retracts a piston at `(x, y, z)` in `dim` based on adjacent redstone power.
    pub fn update_piston_at_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        let regs = std::sync::Arc::clone(world.registries());
        let cur = world.get_block_in(dim, x, y, z);
        let Some((sticky, extended, facing_idx)) = piston_info_with(cur, &regs) else {
            return Vec::new();
        };
        let (dx, dy, dz) = piston_facing_offset(facing_idx);
        let powered = [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ]
        .into_iter()
        .filter(|&(ox, oy, oz)| (ox, oy, oz) != (dx, dy, dz))
        .any(|(ox, oy, oz)| {
            let ny = y + oy;
            if ny < MIN_WORLD_Y {
                return false;
            }
            let ns = world.get_block_in(dim, x + ox, ny, z + oz);
            is_power_source_with(ns, &regs)
                || RedstoneWireState::from_state_id_with(ns, &regs).is_some_and(|w| w.power > 0)
        });

        let mut updates = Vec::new();
        if powered && !extended {
            let mut chain = Vec::new();
            for step in 1..=13 {
                let cx = x + dx * step;
                let cy = y + dy * step;
                let cz = z + dz * step;
                if cy < MIN_WORLD_Y {
                    return Vec::new();
                }
                let cs = world.get_block_in(dim, cx, cy, cz);
                if cs == 0 {
                    break;
                }
                if is_piston_immovable_with(cs, &regs) || chain.len() >= 12 {
                    return Vec::new();
                }
                chain.push((cx, cy, cz, cs));
            }
            for &(cx, cy, cz, cs) in chain.iter().rev() {
                let tx = cx + dx;
                let ty = cy + dy;
                let tz = cz + dz;
                if world.set_block_in(dim, tx, ty, tz, cs) {
                    updates.push((tx, ty, tz, cs));
                }
            }
            let head_base = regs
                .block_by_name("minecraft:piston_head")
                .map(|b| b.min_state)
                .unwrap_or(2385);
            let head_state = head_base + (facing_idx as u16) * 4 + 2 + u16::from(sticky);
            let hx = x + dx;
            let hy = y + dy;
            let hz = z + dz;
            if world.set_block_in(dim, hx, hy, hz, head_state) {
                updates.push((hx, hy, hz, head_state));
            }
            let ext_state = cur - 6;
            if world.set_block_in(dim, x, y, z, ext_state) {
                updates.push((x, y, z, ext_state));
            }
        } else if !powered && extended {
            let hx = x + dx;
            let hy = y + dy;
            let hz = z + dz;
            if hy >= MIN_WORLD_Y
                && regs
                    .block_by_state(world.get_block_in(dim, hx, hy, hz))
                    .is_some_and(|b| b.name == "minecraft:piston_head")
                && world.set_block_in(dim, hx, hy, hz, 0)
            {
                updates.push((hx, hy, hz, 0));
            }
            let ret_state = cur + 6;
            if world.set_block_in(dim, x, y, z, ret_state) {
                updates.push((x, y, z, ret_state));
            }
            if sticky {
                let px = x + dx * 2;
                let py = y + dy * 2;
                let pz = z + dz * 2;
                if py >= MIN_WORLD_Y {
                    let pull_state = world.get_block_in(dim, px, py, pz);
                    if pull_state != 0
                        && !is_piston_immovable_with(pull_state, &regs)
                        && world.set_block_in(dim, px, py, pz, 0)
                        && world.set_block_in(dim, hx, hy, hz, pull_state)
                    {
                        updates.push((px, py, pz, 0));
                        updates.push((hx, hy, hz, pull_state));
                    }
                }
            }
        }
        updates
    }

    /// Toggles a wooden door (both halves), trapdoor, or fence gate at `(x, y, z)` in `dim`.
    pub fn toggle_openable_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        let regs = std::sync::Arc::clone(world.registries());
        let cur = world.get_block_in(dim, x, y, z);
        let Some(block) = regs.block_by_state(cur) else {
            return Vec::new();
        };
        let name = block.name.as_str();
        let rel = cur - block.min_state;
        if name.ends_with("_trapdoor") && name != "minecraft:iron_trapdoor" {
            let next = block.min_state + (rel ^ 4);
            if world.set_block_in(dim, x, y, z, next) {
                return vec![(x, y, z, next)];
            }
        } else if name.ends_with("_fence_gate") {
            let next = block.min_state + (rel ^ 2);
            if world.set_block_in(dim, x, y, z, next) {
                return vec![(x, y, z, next)];
            }
        } else if name.ends_with("_door") && name != "minecraft:iron_door" {
            let next = block.min_state + (rel ^ 2);
            let mut updates = Vec::new();
            if world.set_block_in(dim, x, y, z, next) {
                updates.push((x, y, z, next));
            }
            let other_y = if (rel / 8).is_multiple_of(2) {
                y - 1 // upper -> lower
            } else {
                y + 1 // lower -> upper
            };
            if other_y >= MIN_WORLD_Y {
                let other_cur = world.get_block_in(dim, x, other_y, z);
                if let Some(other_block) = regs.block_by_state(other_cur)
                    && other_block.name == name
                {
                    let other_next =
                        other_block.min_state + ((other_cur - other_block.min_state) ^ 2);
                    if world.set_block_in(dim, x, other_y, z, other_next) {
                        updates.push((x, other_y, z, other_next));
                    }
                }
            }
            return updates;
        }
        Vec::new()
    }

    /// Presses a button at `(x, y, z)` in `dim`, scheduling its automatic release.
    pub fn press_button_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
        cur_tick: u64,
    ) -> Vec<(i32, i32, i32, u16)> {
        let regs = std::sync::Arc::clone(world.registries());
        let cur = world.get_block_in(dim, x, y, z);
        if !is_button_with(cur, &regs) || button_is_powered_with(cur, &regs) != Some(false) {
            return Vec::new();
        }
        let Some(on_state) = button_toggle_state_with(cur, &regs) else {
            return Vec::new();
        };
        if !world.set_block_in(dim, x, y, z, on_state) {
            return Vec::new();
        }
        let delay = regs
            .block_by_state(cur)
            .map(|b| {
                if b.name == "minecraft:stone_button"
                    || b.name == "minecraft:polished_blackstone_button"
                {
                    20
                } else {
                    30
                }
            })
            .unwrap_or(20);
        self.push(ScheduledEntry {
            due_tick: cur_tick.saturating_add(delay),
            dim,
            x,
            y,
            z,
            action: SimAction::ButtonRelease,
        });
        let mut updates = vec![(x, y, z, on_state)];
        updates.extend(self.update_redstone_around_in(world, dim, x, y, z));
        updates
    }

    /// Toggles a lever at `(x, y, z)` in `dim` and propagates redstone power through connected wire and lamps.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn toggle_lever_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        let regs = std::sync::Arc::clone(world.registries());
        let cur = world.get_block_in(dim, x, y, z);
        let Some(next_lever) = lever_toggle_state_with(cur, &regs) else {
            return Vec::new();
        };
        let mut updates = Vec::new();
        if world.set_block_in(dim, x, y, z, next_lever) {
            updates.push((x, y, z, next_lever));
        }
        updates.extend(self.update_redstone_around_in(world, dim, x, y, z));
        updates
    }

    /// Recalculates redstone wire connections, signal power (`0..=15`), and adjacent lamps
    /// around `(x, y, z)` in `dim`, bounded to `512` wires per update.
    pub fn update_redstone_around_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        let regs = std::sync::Arc::clone(world.registries());
        let lamp_off = regs.blocks.redstone_lamp.default_state;
        let lamp_on = regs.blocks.redstone_lamp.min_state;
        let mut wires = std::collections::HashSet::new();
        let mut lamps = std::collections::HashSet::new();
        let mut repeaters = std::collections::HashSet::new();
        let mut explore = VecDeque::new();

        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let bx = x + dx;
                    let by = y + dy;
                    let bz = z + dz;
                    if by < MIN_WORLD_Y {
                        continue;
                    }
                    let blk = world.get_block_in(dim, bx, by, bz);
                    if is_redstone_wire_with(blk, &regs) && wires.insert((bx, by, bz)) {
                        explore.push_back((bx, by, bz));
                    } else if blk == lamp_off || blk == lamp_on {
                        lamps.insert((bx, by, bz));
                    } else if repeater_info_with(blk, &regs).is_some() {
                        repeaters.insert((bx, by, bz));
                    }
                }
            }
        }

        // Collect connected wire component (bounded to 512 wires)
        while let Some((wx, wy, wz)) = explore.pop_front() {
            if wires.len() >= 512 {
                break;
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                for dy in [-1, 0, 1] {
                    let nx = wx + dx;
                    let ny = wy + dy;
                    let nz = wz + dz;
                    if ny < MIN_WORLD_Y {
                        continue;
                    }
                    let blk = world.get_block_in(dim, nx, ny, nz);
                    if is_redstone_wire_with(blk, &regs) && wires.insert((nx, ny, nz)) {
                        explore.push_back((nx, ny, nz));
                    } else if dy == 0 && (blk == lamp_off || blk == lamp_on) {
                        lamps.insert((nx, ny, nz));
                    } else if dy == 0 && repeater_info_with(blk, &regs).is_some() {
                        repeaters.insert((nx, ny, nz));
                    }
                }
            }
            for dy in [-1, 1] {
                let blk = world.get_block_in(dim, wx, wy + dy, wz);
                if blk == lamp_off || blk == lamp_on {
                    lamps.insert((wx, wy + dy, wz));
                }
            }
        }

        // Multi-source BFS for signal power (0..=15)
        let mut power_map: HashMap<(i32, i32, i32), u8> = HashMap::with_capacity(wires.len());
        let mut pq = VecDeque::new();
        for &(wx, wy, wz) in &wires {
            let mut max_src = 0u8;
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 0, 1),
                (0, 0, -1),
                (0, 1, 0),
                (0, -1, 0),
            ] {
                if powers_neighbor_with(
                    world.get_block_in(dim, wx + dx, wy + dy, wz + dz),
                    -dx,
                    -dy,
                    -dz,
                    &regs,
                ) {
                    max_src = 15;
                    break;
                }
            }
            power_map.insert((wx, wy, wz), max_src);
            if max_src > 1 {
                pq.push_back((wx, wy, wz, max_src));
            }
        }

        while let Some((wx, wy, wz, p)) = pq.pop_front() {
            if p <= 1 {
                continue;
            }
            let next_p = p - 1;
            let above_solid =
                is_solid_for_wire_with(world.get_block_in(dim, wx, wy + 1, wz), &regs);
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let side_blk = world.get_block_in(dim, wx + dx, wy, wz + dz);
                let candidates = [
                    (wx + dx, wy, wz + dz, is_redstone_wire_with(side_blk, &regs)),
                    (
                        wx + dx,
                        wy + 1,
                        wz + dz,
                        !above_solid
                            && is_solid_for_wire_with(side_blk, &regs)
                            && is_redstone_wire_with(
                                world.get_block_in(dim, wx + dx, wy + 1, wz + dz),
                                &regs,
                            ),
                    ),
                    (
                        wx + dx,
                        wy - 1,
                        wz + dz,
                        !is_solid_for_wire_with(side_blk, &regs)
                            && is_redstone_wire_with(
                                world.get_block_in(dim, wx + dx, wy - 1, wz + dz),
                                &regs,
                            ),
                    ),
                ];
                for (nx, ny, nz, ok) in candidates {
                    if ok
                        && let Some(cur_p) = power_map.get_mut(&(nx, ny, nz))
                        && next_p > *cur_p
                    {
                        *cur_p = next_p;
                        pq.push_back((nx, ny, nz, next_p));
                    }
                }
            }
        }

        let mut updates = Vec::new();
        for &(wx, wy, wz) in &wires {
            let cur = world.get_block_in(dim, wx, wy, wz);
            let (mut east, mut north, mut south, mut west) =
                compute_wire_connections(world, dim, wx, wy, wz);
            if (east, north, south, west)
                == (
                    WireConnection::None,
                    WireConnection::None,
                    WireConnection::None,
                    WireConnection::None,
                )
            {
                let cur_is_dot =
                    RedstoneWireState::from_state_id_with(cur, &regs).is_some_and(|w| {
                        w.east == WireConnection::None
                            && w.north == WireConnection::None
                            && w.south == WireConnection::None
                            && w.west == WireConnection::None
                    });
                if !cur_is_dot {
                    east = WireConnection::Side;
                    north = WireConnection::Side;
                    south = WireConnection::Side;
                    west = WireConnection::Side;
                }
            }
            let power = power_map.get(&(wx, wy, wz)).copied().unwrap_or(0);
            let target = RedstoneWireState {
                east,
                north,
                power,
                south,
                west,
            }
            .to_state_id_with(&regs);
            if cur != target && world.set_block_in(dim, wx, wy, wz, target) {
                updates.push((wx, wy, wz, target));
            }
        }

        for (lx, ly, lz) in lamps {
            let mut lit = false;
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 0, 1),
                (0, 0, -1),
                (0, 1, 0),
                (0, -1, 0),
            ] {
                let nb = world.get_block_in(dim, lx + dx, ly + dy, lz + dz);
                if powers_neighbor_with(nb, -dx, -dy, -dz, &regs) {
                    lit = true;
                    break;
                }
                if let Some(ws) = RedstoneWireState::from_state_id_with(nb, &regs)
                    && ws.power > 0
                {
                    lit = true;
                    break;
                }
            }
            let target = if lit { lamp_on } else { lamp_off };
            let cur = world.get_block_in(dim, lx, ly, lz);
            if cur != target && world.set_block_in(dim, lx, ly, lz, target) {
                updates.push((lx, ly, lz, target));
            }
        }

        for (dx, dy, dz) in [
            (0, 0, 0),
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            let py = y + dy;
            if py >= MIN_WORLD_Y
                && piston_info_with(world.get_block_in(dim, x + dx, py, z + dz), &regs).is_some()
            {
                updates.extend(self.update_piston_at_in(world, dim, x + dx, py, z + dz));
            }
        }

        for (rx, ry, rz) in repeaters {
            let blk = world.get_block_in(dim, rx, ry, rz);
            if let Some((delay, _, _, _)) = repeater_info_with(blk, &regs) {
                self.push(ScheduledEntry {
                    due_tick: self.last_tick.saturating_add(u64::from(delay) * 2),
                    dim,
                    x: rx,
                    y: ry,
                    z: rz,
                    action: SimAction::RepeaterTick,
                });
            }
        }

        updates
    }

    /// Cycles a `minecraft:repeater` delay (1 -> 2 -> 3 -> 4 -> 1) and returns the new block state.
    pub fn repeater_cycle_delay_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<u16> {
        let regs = std::sync::Arc::clone(world.registries());
        let state = world.get_block_in(dim, x, y, z);
        let (delay, _, _, _) = repeater_info_with(state, &regs)?;
        let rel = state - 8704;
        let next_delay_idx = u16::from(delay % 4);
        let next = 8704 + next_delay_idx * 16 + (rel % 16);
        if world.set_block_in(dim, x, y, z, next) {
            self.push(ScheduledEntry {
                due_tick: self.last_tick.saturating_add(u64::from(delay % 4 + 1) * 2),
                dim,
                x,
                y,
                z,
                action: SimAction::RepeaterTick,
            });
            Some(next)
        } else {
            None
        }
    }

    /// Toggles a `minecraft:comparator` mode (`compare` <-> `subtract`) and returns the new block state.
    pub fn comparator_toggle_mode_in(
        &mut self,
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<u16> {
        let state = world.get_block_in(dim, x, y, z);
        if !(13030..=13045).contains(&state) {
            return None;
        }
        let next = 13030 + ((state - 13030) ^ 2);
        if world.set_block_in(dim, x, y, z, next) {
            Some(next)
        } else {
            None
        }
    }

    /// Overworld wrapper for [`Self::toggle_lever_in`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn toggle_lever(
        &mut self,
        world: &mut WorldState,
        x: i32,
        y: i32,
        z: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        self.toggle_lever_in(world, DimensionId::Overworld, x, y, z)
    }

    /// Detonates TNT at `(x, y, z)` in `dim`, carving blocks within `radius` (preserving bedrock `85`).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn detonate_tnt_in(
        world: &mut WorldState,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        let mut updates = Vec::new();
        let r = radius.clamp(1, 6);
        let r2 = r * r;
        for dx in -r..=r {
            for dy in -r..=r {
                for dz in -r..=r {
                    if dx * dx + dy * dy + dz * dz > r2 {
                        continue;
                    }
                    let bx = x + dx;
                    let by = y + dy;
                    let bz = z + dz;
                    if by <= MIN_WORLD_Y {
                        continue;
                    }
                    let bedrock = world.registries().blocks.bedrock.default_state;
                    let blk = world.get_block_in(dim, bx, by, bz);
                    if blk != 0 && blk != bedrock && world.set_block_in(dim, bx, by, bz, 0) {
                        updates.push((bx, by, bz, 0));
                    }
                }
            }
        }
        updates
    }

    /// Overworld wrapper for [`Self::detonate_tnt_in`].
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn detonate_tnt(
        world: &mut WorldState,
        x: i32,
        y: i32,
        z: i32,
        radius: i32,
    ) -> Vec<(i32, i32, i32, u16)> {
        Self::detonate_tnt_in(world, DimensionId::Overworld, x, y, z, radius)
    }
}
