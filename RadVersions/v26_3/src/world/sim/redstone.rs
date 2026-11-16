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

pub(crate) fn piston_partner_pos(
    state: u16,
    x: i32,
    y: i32,
    z: i32,
    regs: &Version26_3Registries,
) -> Option<(i32, i32, i32)> {
    if let Some((_, extended, facing_idx)) = piston_info_with(state, regs) {
        if !extended {
            return None;
        }
        let (dx, dy, dz) = piston_facing_offset(facing_idx);
        return Some((x + dx, y + dy, z + dz));
    }
    let block = regs.block_by_state(state)?;
    if block.name == "minecraft:piston_head" {
        let facing_idx = usize::from((state - block.min_state) / 4);
        let (dx, dy, dz) = piston_facing_offset(facing_idx);
        return Some((x - dx, y - dy, z - dz));
    }
    None
}

fn is_piston_replaceable_with(state: u16, regs: &Version26_3Registries) -> bool {
    if state == 0 || regs.is_fluid_state(state) {
        return true;
    }
    matches!(
        regs.block_by_state(state).map(|b| b.name).as_deref(),
        Some(
            "minecraft:short_grass"
                | "minecraft:tall_grass"
                | "minecraft:fern"
                | "minecraft:large_fern"
                | "minecraft:dead_bush"
                | "minecraft:snow"
                | "minecraft:fire"
                | "minecraft:soul_fire"
        )
    )
}

fn is_solid_conductor_with(state: u16, regs: &Version26_3Registries) -> bool {
    state != 0
        && !regs.is_air_state(state)
        && !regs.is_fluid_state(state)
        && !is_redstone_wire_with(state, regs)
        && piston_info_with(state, regs).is_none()
        && !is_power_source_with(state, regs)
        && repeater_info_with(state, regs).is_none()
}

pub(super) fn is_block_strongly_powered_at(
    world: &mut WorldState,
    dim: DimensionId,
    bx: i32,
    by: i32,
    bz: i32,
    regs: &Version26_3Registries,
) -> bool {
    let blk = world.get_block_in(dim, bx, by, bz);
    if !is_solid_conductor_with(blk, regs) {
        return false;
    }
    if by < 319
        && RedstoneWireState::from_state_id_with(world.get_block_in(dim, bx, by + 1, bz), regs)
            .is_some_and(|w| w.power > 0)
    {
        return true;
    }
    for (ox, oy, oz) in [
        (1, 0, 0),
        (-1, 0, 0),
        (0, 1, 0),
        (0, -1, 0),
        (0, 0, 1),
        (0, 0, -1),
    ] {
        let ny = by + oy;
        if !(MIN_WORLD_Y..=319).contains(&ny) {
            continue;
        }
        let ns = world.get_block_in(dim, bx + ox, ny, bz + oz);
        if powers_neighbor_with(ns, -ox, -oy, -oz, regs) {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn is_piston_powered_at(
    world: &mut WorldState,
    dim: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    dx: i32,
    dy: i32,
    dz: i32,
    regs: &Version26_3Registries,
) -> bool {
    let check_pos = |world: &mut WorldState, px: i32, py: i32, pz: i32, is_above: bool| -> bool {
        for (ox, oy, oz) in [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            if !is_above && (ox, oy, oz) == (dx, dy, dz) {
                continue;
            }
            if is_above && (ox, oy, oz) == (0, -1, 0) {
                continue;
            }
            let ny = py + oy;
            if !(MIN_WORLD_Y..=319).contains(&ny) {
                continue;
            }
            let nx = px + ox;
            let nz = pz + oz;
            let ns = world.get_block_in(dim, nx, ny, nz);
            if powers_neighbor_with(ns, -ox, -oy, -oz, regs)
                || RedstoneWireState::from_state_id_with(ns, regs).is_some_and(|w| w.power > 0)
                || (!is_above && is_block_strongly_powered_at(world, dim, nx, ny, nz, regs))
            {
                return true;
            }
        }
        false
    };

    check_pos(world, x, y, z, false) || (y < 319 && check_pos(world, x, y + 1, z, true))
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
            "minecraft:copper_torch" => {
                if let Some(wb) = regs.block_by_name("minecraft:copper_wall_torch") {
                    return wb.min_state + wall_facing;
                }
            }
            _ => {}
        }
    }

    if name.ends_with("_slab") && block.max_state - block.min_state == 5 {
        let top = face == 0 || (face >= 2 && cursor_y > 0.5);
        // type(top=0, bottom=1, double=2), waterlogged(true=0, false=1)
        return min + if top { 1 } else { 3 };
    }
    if block.max_state - block.min_state == 2
        && (name.ends_with("_log")
            || name.ends_with("_wood")
            || name.ends_with("_stem")
            || name.ends_with("_hyphae")
            || matches!(
                name,
                "minecraft:hay_block"
                    | "minecraft:basalt"
                    | "minecraft:polished_basalt"
                    | "minecraft:bone_block"
                    | "minecraft:quartz_pillar"
                    | "minecraft:purpur_pillar"
                    | "minecraft:deepslate"
                    | "minecraft:muddy_mangrove_roots"
            ))
    {
        // axis: x=0, y=1, z=2
        let axis = match face {
            2 | 3 => 2,
            4 | 5 => 0,
            _ => 1,
        };
        return min + axis;
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

pub(super) fn repeater_front_dir(facing: usize) -> (i32, i32) {
    // In Minecraft blockstates, repeater `facing` is the input (back) side; output (front) is opposite.
    match facing {
        0 => (0, 1),  // facing=north -> outputs south
        1 => (0, -1), // facing=south -> outputs north
        2 => (1, 0),  // facing=west -> outputs east
        _ => (-1, 0), // facing=east -> outputs west
    }
}

pub(super) fn repeater_back_dir(facing: usize) -> (i32, i32) {
    let (fx, fz) = repeater_front_dir(facing);
    (-fx, -fz)
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
        let powered = is_piston_powered_at(world, dim, x, y, z, dx, dy, dz, &regs);

        let mut updates = Vec::new();
        if powered && !extended {
            let mut chain = Vec::new();
            for step in 1..=13 {
                let cx = x + dx * step;
                let cy = y + dy * step;
                let cz = z + dz * step;
                if !(MIN_WORLD_Y..=319).contains(&cy) {
                    return Vec::new();
                }
                let cs = world.get_block_in(dim, cx, cy, cz);
                if is_piston_replaceable_with(cs, &regs) {
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
            if (MIN_WORLD_Y..=319).contains(&hy) && world.set_block_in(dim, hx, hy, hz, head_state)
            {
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
            if (MIN_WORLD_Y..=319).contains(&hy)
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
                if (MIN_WORLD_Y..=319).contains(&py) {
                    let pull_state = world.get_block_in(dim, px, py, pz);
                    if !is_piston_replaceable_with(pull_state, &regs)
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
        let tick = self.last_tick;
        for &(ux, uy, uz, _) in &updates {
            self.on_block_changed_in(world, dim, ux, uy, uz, tick);
        }
        // Propagate power if a piston moved a redstone block or wire.
        let mut extra_updates = Vec::new();
        for &(ux, uy, uz, ustate) in &updates {
            if is_power_source_with(ustate, &regs) || is_redstone_wire_with(ustate, &regs) {
                for (ox, oy, oz) in [
                    (1, 0, 0),
                    (-1, 0, 0),
                    (0, 1, 0),
                    (0, -1, 0),
                    (0, 0, 1),
                    (0, 0, -1),
                ] {
                    let ny = uy + oy;
                    if (MIN_WORLD_Y..=319).contains(&ny) {
                        let nb = world.get_block_in(dim, ux + ox, ny, uz + oz);
                        if is_redstone_wire_with(nb, &regs) {
                            extra_updates
                                .extend(self.update_redstone_around_in(world, dim, ux, uy, uz));
                            break;
                        }
                    }
                }
            }
        }
        updates.extend(extra_updates);
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
                let nb = world.get_block_in(dim, wx + dx, wy + dy, wz + dz);
                if powers_neighbor_with(nb, -dx, -dy, -dz, &regs)
                    || is_block_strongly_powered_at(world, dim, wx + dx, wy + dy, wz + dz, &regs)
                {
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

        for dx in -2_i32..=2 {
            for dy in -2_i32..=2 {
                for dz in -2_i32..=2 {
                    if dx.abs() + dy.abs() + dz.abs() > 2 {
                        continue;
                    }
                    let py = y + dy;
                    if (MIN_WORLD_Y..=319).contains(&py)
                        && piston_info_with(world.get_block_in(dim, x + dx, py, z + dz), &regs)
                            .is_some()
                    {
                        updates.extend(self.update_piston_at_in(world, dim, x + dx, py, z + dz));
                    }
                }
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
        let repeater = regs.block_by_name("minecraft:repeater")?;
        let state = world.get_block_in(dim, x, y, z);
        let (delay, _, _, _) = repeater_info_with(state, &regs)?;
        let rel = state - repeater.min_state;
        let next_delay_idx = u16::from(delay % 4);
        let next = repeater.min_state + next_delay_idx * 16 + (rel % 16);
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
        let regs = std::sync::Arc::clone(world.registries());
        let comp = regs.block_by_name("minecraft:comparator")?;
        let state = world.get_block_in(dim, x, y, z);
        if !(comp.min_state..=comp.max_state).contains(&state) {
            return None;
        }
        let next = comp.min_state + ((state - comp.min_state) ^ 2);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registries::{Version26_3Registries, synthetic_26_3_test_provider};
    use crate::world::WorldState;
    use rad_core::registry_provider::{
        BlockRegistryEntry, RegistryProvider, RegistryProviderError, RegistryTagSet, TagDefinition,
        VersionMetadata,
    };
    use std::sync::Arc;

    #[derive(Debug)]
    struct RedstoneTestProvider {
        blocks: Vec<BlockRegistryEntry>,
    }

    impl RedstoneTestProvider {
        fn new() -> Self {
            let base = synthetic_26_3_test_provider();
            let mut blocks = Vec::new();
            let mut next_state = 0u16;
            let extra = [
                ("minecraft:stone", 1u16),
                ("minecraft:redstone_block", 1),
                ("minecraft:repeater", 64),
                ("minecraft:piston", 12),
                ("minecraft:piston_head", 24),
                ("minecraft:sticky_piston", 12),
            ];
            let mut idx = 0u16;
            while let Some(entry) = base.block_by_state(idx) {
                let count = if entry.name == "minecraft:redstone_wire" {
                    1296
                } else {
                    entry.max_state - entry.min_state + 1
                };
                let min_state = next_state;
                let max_state = min_state + count - 1;
                blocks.push(BlockRegistryEntry {
                    block_id: blocks.len() as u16,
                    name: entry.name.clone(),
                    default_state: min_state,
                    min_state,
                    max_state,
                });
                next_state = max_state + 1;
                idx = entry.max_state + 1;
            }
            for (name, count) in extra {
                let min_state = next_state;
                let max_state = min_state + count - 1;
                blocks.push(BlockRegistryEntry {
                    block_id: blocks.len() as u16,
                    name: name.to_string(),
                    default_state: min_state,
                    min_state,
                    max_state,
                });
                next_state = max_state + 1;
            }
            Self { blocks }
        }
    }

    impl RegistryProvider for RedstoneTestProvider {
        fn metadata(&self) -> VersionMetadata {
            synthetic_26_3_test_provider().metadata()
        }

        fn block_by_name(&self, name: &str) -> Option<BlockRegistryEntry> {
            self.blocks.iter().find(|b| b.name == name).cloned()
        }

        fn block_by_state(&self, state: u16) -> Option<BlockRegistryEntry> {
            self.blocks
                .iter()
                .find(|b| state >= b.min_state && state <= b.max_state)
                .cloned()
        }

        fn item_by_name(&self, name: &str) -> Option<i32> {
            synthetic_26_3_test_provider().item_by_name(name)
        }

        fn item_name_by_id(&self, _id: i32) -> Option<&str> {
            None
        }

        fn entity_by_name(&self, name: &str) -> Option<i32> {
            synthetic_26_3_test_provider().entity_by_name(name)
        }

        fn menu_by_name(&self, name: &str) -> Option<i32> {
            synthetic_26_3_test_provider().menu_by_name(name)
        }

        fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
            Ok(Vec::new())
        }

        fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
            Ok(Vec::new())
        }

        fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn repeater_directions_match_vanilla_facing() {
        // facing=north (0): input at north (0, -1), output to south (0, 1)
        assert_eq!(repeater_back_dir(0), (0, -1));
        assert_eq!(repeater_front_dir(0), (0, 1));
        // facing=south (1): input at south (0, 1), output to north (0, -1)
        assert_eq!(repeater_back_dir(1), (0, 1));
        assert_eq!(repeater_front_dir(1), (0, -1));
        // facing=west (2): input at west (-1, 0), output to east (1, 0)
        assert_eq!(repeater_back_dir(2), (-1, 0));
        assert_eq!(repeater_front_dir(2), (1, 0));
        // facing=east (3): input at east (1, 0), output to west (-1, 0)
        assert_eq!(repeater_back_dir(3), (1, 0));
        assert_eq!(repeater_front_dir(3), (-1, 0));
    }

    #[test]
    fn repeater_powers_forward_wire_and_extends_piston() {
        let regs = Arc::new(
            Version26_3Registries::from_provider(Arc::new(RedstoneTestProvider::new()), true)
                .expect("redstone test registries"),
        );
        let mut world = WorldState::new_with_registries(None, Arc::clone(&regs));
        let mut sim = WorldSim::default();

        let stone = regs.block_by_name("minecraft:stone").unwrap().default_state;
        let redstone_block = regs
            .block_by_name("minecraft:redstone_block")
            .unwrap()
            .default_state;
        let wire_unpowered = RedstoneWireState {
            east: WireConnection::None,
            north: WireConnection::None,
            power: 0,
            south: WireConnection::None,
            west: WireConnection::None,
        }
        .to_state_id_with(&regs);
        let rep_min = regs.block_by_name("minecraft:repeater").unwrap().min_state;
        // facing=west (2) -> input from west (-1, 0), outputs east (+1, 0).
        // delay=1 (0), facing=west (2*4=8), locked=false (2), powered=false (1) => offset 11.
        let repeater_west_unpowered = rep_min + 11;
        let piston_min = regs.block_by_name("minecraft:piston").unwrap().min_state;
        // piston: extended=false (6), facing=east (1) => min + 7
        let piston_east = piston_min + 7;

        for x in 0..=8 {
            assert!(world.set_block(x, 63, 0, stone));
        }
        assert!(world.set_block(1, 64, 0, redstone_block));
        assert!(world.set_block(2, 64, 0, wire_unpowered));
        assert!(world.set_block(3, 64, 0, repeater_west_unpowered));
        assert!(world.set_block(4, 64, 0, wire_unpowered));
        assert!(world.set_block(5, 64, 0, piston_east));

        sim.update_redstone_around_in(&mut world, DimensionId::Overworld, 1, 64, 0);
        sim.on_block_changed(&mut world, 3, 64, 0, 0);
        for tick in 1..=6 {
            sim.step(&mut world, tick);
        }

        // Repeater should now be powered (even offset, powered=true=0)
        let rep_state = world.get_block(3, 64, 0);
        assert_eq!((rep_state - rep_min) % 2, 0, "repeater should be powered");

        // Wire in front of repeater (at x=4) should be powered at 15
        let front_wire = RedstoneWireState::from_state_id_with(world.get_block(4, 64, 0), &regs)
            .expect("front block should be redstone wire");
        assert_eq!(front_wire.power, 15);

        // Piston at x=5 should be extended (extended=true => offset < 6) and head at x=6
        let piston_state = world.get_block(5, 64, 0);
        assert!(
            piston_state - piston_min < 6,
            "piston should be extended, got state {piston_state}"
        );
        let head_min = regs
            .block_by_name("minecraft:piston_head")
            .unwrap()
            .min_state;
        let head_max = regs
            .block_by_name("minecraft:piston_head")
            .unwrap()
            .max_state;
        let head_state = world.get_block(6, 64, 0);
        assert!(
            head_state >= head_min && head_state <= head_max,
            "piston head should be placed at x=6"
        );
    }
}
