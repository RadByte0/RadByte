//! State-dependent Minecraft Java 26.3 (`protocol 777`) voxel shapes for collision,
//! interaction/outline, and support checks.

use std::sync::LazyLock;

use rad_core::registry_provider::RegistryProvider;

/// Local unit-block AABB (`0.0..=1.0` typical, up to `1.5` for fences/walls/gates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockBox {
    /// Minimum local X.
    pub min_x: f64,
    /// Minimum local Y.
    pub min_y: f64,
    /// Minimum local Z.
    pub min_z: f64,
    /// Maximum local X.
    pub max_x: f64,
    /// Maximum local Y.
    pub max_y: f64,
    /// Maximum local Z.
    pub max_z: f64,
}

impl BlockBox {
    /// Full unit cube (`0.0..=1.0` on all axes).
    pub const FULL: Self = Self {
        min_x: 0.0,
        min_y: 0.0,
        min_z: 0.0,
        max_x: 1.0,
        max_y: 1.0,
        max_z: 1.0,
    };

    /// Constructs a local AABB from pixel coordinates (`0..=16`, up to `24` for 1.5-tall blocks).
    #[must_use]
    pub const fn px(x0: u8, y0: u8, z0: u8, x1: u8, y1: u8, z1: u8) -> Self {
        Self {
            min_x: x0 as f64 / 16.0,
            min_y: y0 as f64 / 16.0,
            min_z: z0 as f64 / 16.0,
            max_x: x1 as f64 / 16.0,
            max_y: y1 as f64 / 16.0,
            max_z: z1 as f64 / 16.0,
        }
    }

    /// Returns `true` when coordinates are finite and properly ordered (`min <= max`).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.min_x.is_finite()
            && self.min_y.is_finite()
            && self.min_z.is_finite()
            && self.max_x.is_finite()
            && self.max_y.is_finite()
            && self.max_z.is_finite()
            && self.min_x <= self.max_x
            && self.min_y <= self.max_y
            && self.min_z <= self.max_z
    }
}

/// Typed voxel shape representation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VoxelShape {
    /// Empty shape (no boxes).
    Empty,
    /// Full `1x1x1` unit cube.
    Full,
    /// One or more local AABBs.
    Boxes(&'static [BlockBox]),
}

impl VoxelShape {
    /// Returns the underlying slice of local AABBs (`&[]` for `Empty`).
    #[must_use]
    pub fn as_boxes(self) -> &'static [BlockBox] {
        static FULL_SLICE: [BlockBox; 1] = [BlockBox::FULL];
        match self {
            Self::Empty => &[],
            Self::Full => &FULL_SLICE,
            Self::Boxes(b) => b,
        }
    }

    /// Returns `true` if the shape contains no boxes.
    #[must_use]
    pub fn is_empty(self) -> bool {
        match self {
            Self::Empty => true,
            Self::Full => false,
            Self::Boxes(b) => b.is_empty(),
        }
    }

    /// Returns `true` if the shape is the full `1x1x1` unit cube.
    #[must_use]
    pub const fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }
}

static BOTTOM_HALF: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 8, 16)];
static TOP_HALF: [BlockBox; 1] = [BlockBox::px(0, 8, 0, 16, 16, 16)];
static TRAPDOOR_BOTTOM: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 3, 16)];
static TRAPDOOR_TOP: [BlockBox; 1] = [BlockBox::px(0, 13, 0, 16, 16, 16)];
static CARPET_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 1, 16)];
static REPEATER_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 2, 16)];
static PRESSURE_PLATE_BOX: [BlockBox; 1] = [BlockBox::px(1, 0, 1, 15, 1, 15)];
static RAIL_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 2, 16)];
static TORCH_BOX: [BlockBox; 1] = [BlockBox::px(6, 0, 6, 10, 10, 10)];
static LEVER_BOX: [BlockBox; 1] = [BlockBox::px(4, 0, 4, 12, 10, 12)];
static BUTTON_BOX: [BlockBox; 1] = [BlockBox::px(5, 0, 6, 11, 2, 10)];
static PLANT_BOX: [BlockBox; 1] = [BlockBox::px(2, 0, 2, 14, 13, 14)];

static CHEST_BOX: [BlockBox; 1] = [BlockBox::px(1, 0, 1, 15, 14, 15)];
static FARMLAND_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 15, 16)];
static SOUL_SAND_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 14, 16)];
static HONEY_BOX: [BlockBox; 1] = [BlockBox::px(1, 0, 1, 15, 15, 15)];
static CACTUS_BOX: [BlockBox; 1] = [BlockBox::px(1, 0, 1, 15, 16, 15)];
static BAMBOO_BOX: [BlockBox; 1] = [BlockBox::px(6, 0, 6, 10, 16, 10)];
static LILY_PAD_BOX: [BlockBox; 1] = [BlockBox::px(1, 0, 1, 15, 1, 15)];
static BED_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 9, 16)];
static STONECUTTER_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 9, 16)];
static CAMPFIRE_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 7, 16)];
static DAYLIGHT_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 6, 16)];
static ENCHANTING_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 12, 16)];
static END_PORTAL_FRAME_BOX: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 13, 16)];
static LECTERN_BOX: [BlockBox; 2] = [
    BlockBox::px(0, 0, 0, 16, 2, 16),
    BlockBox::px(4, 2, 4, 12, 14, 12),
];
static BELL_BOX: [BlockBox; 1] = [BlockBox::px(4, 0, 4, 12, 16, 12)];
static LANTERN_STANDING_BOX: [BlockBox; 1] = [BlockBox::px(5, 0, 5, 11, 9, 11)];
static LANTERN_HANGING_BOX: [BlockBox; 1] = [BlockBox::px(5, 1, 5, 11, 10, 11)];
static FLOWER_POT_BOX: [BlockBox; 1] = [BlockBox::px(5, 0, 5, 11, 6, 11)];
static HEAD_BOX: [BlockBox; 1] = [BlockBox::px(4, 0, 4, 12, 8, 12)];
static BREWING_STAND_BOX: [BlockBox; 2] = [
    BlockBox::px(1, 0, 1, 15, 2, 15),
    BlockBox::px(7, 0, 7, 9, 14, 9),
];
static HOPPER_BOXES: [BlockBox; 5] = [
    BlockBox::px(0, 10, 0, 16, 11, 16),
    BlockBox::px(0, 11, 0, 2, 16, 16),
    BlockBox::px(14, 11, 0, 16, 16, 16),
    BlockBox::px(2, 11, 0, 14, 16, 2),
    BlockBox::px(2, 11, 14, 14, 16, 16),
];
static CAULDRON_BOXES: [BlockBox; 5] = [
    BlockBox::px(0, 0, 0, 16, 4, 16),
    BlockBox::px(0, 4, 0, 2, 16, 16),
    BlockBox::px(14, 4, 0, 16, 16, 16),
    BlockBox::px(2, 4, 0, 14, 16, 2),
    BlockBox::px(2, 4, 14, 14, 16, 16),
];
static SCAFFOLDING_TOP: [BlockBox; 1] = [BlockBox::px(0, 14, 0, 16, 16, 16)];
static SCAFFOLDING_BOTTOM_TOP: [BlockBox; 2] = [
    BlockBox::px(0, 0, 0, 16, 2, 16),
    BlockBox::px(0, 14, 0, 16, 16, 16),
];
static ANVIL_NS: [BlockBox; 1] = [BlockBox::px(2, 0, 0, 14, 16, 16)];
static ANVIL_WE: [BlockBox; 1] = [BlockBox::px(0, 0, 2, 16, 16, 14)];

static WALL_NORTH: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 16, 3)];
static WALL_SOUTH: [BlockBox; 1] = [BlockBox::px(0, 0, 13, 16, 16, 16)];
static WALL_WEST: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 3, 16, 16)];
static WALL_EAST: [BlockBox; 1] = [BlockBox::px(13, 0, 0, 16, 16, 16)];

static GATE_NS: [BlockBox; 1] = [BlockBox::px(0, 0, 6, 16, 24, 10)];
static GATE_WE: [BlockBox; 1] = [BlockBox::px(6, 0, 0, 10, 24, 16)];

static SNOW_COLLISION_LAYERS: [[BlockBox; 1]; 7] = [
    [BlockBox::px(0, 0, 0, 16, 2, 16)],  // layers=2
    [BlockBox::px(0, 0, 0, 16, 4, 16)],  // layers=3
    [BlockBox::px(0, 0, 0, 16, 6, 16)],  // layers=4
    [BlockBox::px(0, 0, 0, 16, 8, 16)],  // layers=5
    [BlockBox::px(0, 0, 0, 16, 10, 16)], // layers=6
    [BlockBox::px(0, 0, 0, 16, 12, 16)], // layers=7
    [BlockBox::px(0, 0, 0, 16, 14, 16)], // layers=8
];
static SNOW_LAYER_1_INTERACT: [BlockBox; 1] = [BlockBox::px(0, 0, 0, 16, 2, 16)];

static CAKE_BOXES: [[BlockBox; 1]; 7] = [
    [BlockBox::px(1, 0, 1, 15, 8, 15)],
    [BlockBox::px(3, 0, 1, 15, 8, 15)],
    [BlockBox::px(5, 0, 1, 15, 8, 15)],
    [BlockBox::px(7, 0, 1, 15, 8, 15)],
    [BlockBox::px(9, 0, 1, 15, 8, 15)],
    [BlockBox::px(11, 0, 1, 15, 8, 15)],
    [BlockBox::px(13, 0, 1, 15, 8, 15)],
];

static AXIS_X_THIN: [BlockBox; 1] = [BlockBox::px(0, 6, 6, 16, 10, 10)];
static AXIS_Y_THIN: [BlockBox; 1] = [BlockBox::px(6, 0, 6, 10, 16, 10)];
static AXIS_Z_THIN: [BlockBox; 1] = [BlockBox::px(6, 6, 0, 10, 10, 16)];

// Precomputed 40 stair shapes: facing(0=N,1=S,2=W,3=E) * 10 + half(0=top,1=bottom) * 5 + shape(0..5)
static STAIR_SHAPES: LazyLock<Vec<Vec<BlockBox>>> = LazyLock::new(|| {
    let mut table = Vec::with_capacity(40);
    for facing in 0..4u8 {
        for half in 0..2u8 {
            let base = if half == 0 {
                BlockBox::px(0, 8, 0, 16, 16, 16)
            } else {
                BlockBox::px(0, 0, 0, 16, 8, 16)
            };
            let (sy0, sy1) = if half == 0 { (0, 8) } else { (8, 16) };
            // Half-block step on each side: N(z:0..8), S(z:8..16), W(x:0..8), E(x:8..16)
            let half_step = |dir: u8| -> BlockBox {
                match dir {
                    0 => BlockBox::px(0, sy0, 0, 16, sy1, 8),  // North
                    1 => BlockBox::px(0, sy0, 8, 16, sy1, 16), // South
                    2 => BlockBox::px(0, sy0, 0, 8, sy1, 16),  // West
                    _ => BlockBox::px(8, sy0, 0, 16, sy1, 16), // East
                }
            };
            // Quadrants: NW, NE, SW, SE
            let quad = |north: bool, west: bool| -> BlockBox {
                let (x0, x1) = if west { (0, 8) } else { (8, 16) };
                let (z0, z1) = if north { (0, 8) } else { (8, 16) };
                BlockBox::px(x0, sy0, z0, x1, sy1, z1)
            };
            // Left/right relative to facing (0=N,1=S,2=W,3=E)
            let (left_q, right_q, inner_left_q, inner_right_q) = match facing {
                0 => (
                    quad(true, true),
                    quad(true, false),
                    quad(false, true),
                    quad(false, false),
                ), // facing N: front=N, left=W, right=E
                1 => (
                    quad(false, false),
                    quad(false, true),
                    quad(true, false),
                    quad(true, true),
                ), // facing S: front=S, left=E, right=W
                2 => (
                    quad(false, true),
                    quad(true, true),
                    quad(false, false),
                    quad(true, false),
                ), // facing W: front=W, left=S, right=N
                _ => (
                    quad(true, false),
                    quad(false, false),
                    quad(true, true),
                    quad(false, true),
                ), // facing E: front=E, left=N, right=S
            };

            for shape in 0..5u8 {
                let boxes = match shape {
                    0 => vec![base, half_step(facing)],                // straight
                    1 => vec![base, half_step(facing), inner_left_q],  // inner_left
                    2 => vec![base, half_step(facing), inner_right_q], // inner_right
                    3 => vec![base, left_q],                           // outer_left
                    _ => vec![base, right_q],                          // outer_right
                };
                table.push(boxes);
            }
        }
    }
    table
});

// Precomputed 16 fence shapes: mask bit0=N, bit1=E, bit2=S, bit3=W
static FENCE_SHAPES: LazyLock<[Vec<BlockBox>; 16]> = LazyLock::new(|| {
    std::array::from_fn(|mask| {
        let mut v = vec![BlockBox::px(6, 0, 6, 10, 24, 10)];
        if (mask & 1) != 0 {
            v.push(BlockBox::px(6, 0, 0, 10, 24, 6));
        }
        if (mask & 2) != 0 {
            v.push(BlockBox::px(10, 0, 6, 16, 24, 10));
        }
        if (mask & 4) != 0 {
            v.push(BlockBox::px(6, 0, 10, 10, 24, 16));
        }
        if (mask & 8) != 0 {
            v.push(BlockBox::px(0, 0, 6, 6, 24, 10));
        }
        v
    })
});

// Precomputed 32 wall shapes: bit4=up, bit0=N, bit1=E, bit2=S, bit3=W
static WALL_SHAPES: LazyLock<[Vec<BlockBox>; 32]> = LazyLock::new(|| {
    std::array::from_fn(|mask| {
        let mut v = Vec::new();
        if (mask & 16) != 0 {
            v.push(BlockBox::px(4, 0, 4, 12, 24, 12));
        }
        if (mask & 1) != 0 {
            v.push(BlockBox::px(5, 0, 0, 11, 24, 8));
        }
        if (mask & 2) != 0 {
            v.push(BlockBox::px(8, 0, 5, 16, 24, 11));
        }
        if (mask & 4) != 0 {
            v.push(BlockBox::px(5, 0, 8, 11, 24, 16));
        }
        if (mask & 8) != 0 {
            v.push(BlockBox::px(0, 0, 5, 8, 24, 11));
        }
        if v.is_empty() {
            v.push(BlockBox::px(4, 0, 4, 12, 24, 12));
        }
        v
    })
});

// Precomputed 16 pane/iron_bars shapes: bit0=N, bit1=E, bit2=S, bit3=W
static PANE_SHAPES: LazyLock<[Vec<BlockBox>; 16]> = LazyLock::new(|| {
    std::array::from_fn(|mask| {
        let mut v = vec![BlockBox::px(7, 0, 7, 9, 16, 9)];
        if (mask & 1) != 0 {
            v.push(BlockBox::px(7, 0, 0, 9, 16, 7));
        }
        if (mask & 2) != 0 {
            v.push(BlockBox::px(9, 0, 7, 16, 16, 9));
        }
        if (mask & 4) != 0 {
            v.push(BlockBox::px(7, 0, 9, 9, 16, 16));
        }
        if (mask & 8) != 0 {
            v.push(BlockBox::px(0, 0, 7, 7, 16, 9));
        }
        v
    })
});

// Precomputed 6 piston_head shapes (facing N,E,S,W,U,D)
static PISTON_HEAD_SHAPES: [[BlockBox; 2]; 6] = [
    [
        BlockBox::px(0, 0, 0, 16, 16, 4),
        BlockBox::px(6, 6, 4, 10, 10, 16),
    ], // North
    [
        BlockBox::px(12, 0, 0, 16, 16, 16),
        BlockBox::px(0, 6, 6, 12, 10, 10),
    ], // East
    [
        BlockBox::px(0, 0, 12, 16, 16, 16),
        BlockBox::px(6, 6, 0, 10, 10, 12),
    ], // South
    [
        BlockBox::px(0, 0, 0, 4, 16, 16),
        BlockBox::px(4, 6, 6, 16, 10, 10),
    ], // West
    [
        BlockBox::px(0, 12, 0, 16, 16, 16),
        BlockBox::px(6, 0, 6, 10, 12, 10),
    ], // Up
    [
        BlockBox::px(0, 0, 0, 16, 4, 16),
        BlockBox::px(6, 4, 6, 10, 16, 10),
    ], // Down
];

fn is_non_solid_name(name: &str) -> bool {
    name.ends_with("_sapling")
        || name.ends_with("_button")
        || name.ends_with("_pressure_plate")
        || name.ends_with("_sign")
        || name.ends_with("_banner")
        || name.ends_with("_rail")
        || name.ends_with("_coral")
        || name.ends_with("_coral_fan")
        || name.ends_with("_coral_wall_fan")
        || name.ends_with("_tulip")
        || name.ends_with("_vines")
        || name.ends_with("_vines_plant")
        || matches!(
            name,
            "minecraft:rail"
                | "minecraft:short_grass"
                | "minecraft:tall_grass"
                | "minecraft:fern"
                | "minecraft:large_fern"
                | "minecraft:dead_bush"
                | "minecraft:seagrass"
                | "minecraft:tall_seagrass"
                | "minecraft:kelp"
                | "minecraft:kelp_plant"
                | "minecraft:sugar_cane"
                | "minecraft:carrots"
                | "minecraft:potatoes"
                | "minecraft:beetroots"
                | "minecraft:wheat"
                | "minecraft:melon_stem"
                | "minecraft:pumpkin_stem"
                | "minecraft:attached_melon_stem"
                | "minecraft:attached_pumpkin_stem"
                | "minecraft:nether_wart"
                | "minecraft:sweet_berry_bush"
                | "minecraft:dandelion"
                | "minecraft:poppy"
                | "minecraft:blue_orchid"
                | "minecraft:allium"
                | "minecraft:azure_bluet"
                | "minecraft:oxeye_daisy"
                | "minecraft:cornflower"
                | "minecraft:lily_of_the_valley"
                | "minecraft:wither_rose"
                | "minecraft:sunflower"
                | "minecraft:lilac"
                | "minecraft:rose_bush"
                | "minecraft:peony"
                | "minecraft:brown_mushroom"
                | "minecraft:red_mushroom"
                | "minecraft:crimson_fungus"
                | "minecraft:warped_fungus"
                | "minecraft:crimson_roots"
                | "minecraft:warped_roots"
                | "minecraft:nether_sprouts"
                | "minecraft:vine"
                | "minecraft:glow_lichen"
                | "minecraft:cobweb"
                | "minecraft:powder_snow"
                | "minecraft:tripwire"
                | "minecraft:tripwire_hook"
                | "minecraft:moving_piston"
                | "minecraft:bubble_column"
                | "minecraft:spore_blossom"
                | "minecraft:hanging_roots"
                | "minecraft:small_dripleaf"
                | "minecraft:sculk_vein"
                | "minecraft:pink_petals"
                | "minecraft:leaf_litter"
                | "minecraft:wildflowers"
                | "minecraft:bush"
                | "minecraft:firefly_bush"
                | "minecraft:short_dry_grass"
                | "minecraft:tall_dry_grass"
                | "minecraft:cactus_flower"
                | "minecraft:bamboo_sapling"
        )
}

fn is_air_or_fluid_name(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "test:air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:bubble_column"
    )
}

/// Computes the collision [`VoxelShape`] for a resolved block entry without
/// requiring compile-time `mc777_data` tables.
#[must_use]
pub fn block_collision_shape_with(
    state_id: u16,
    provider: &dyn RegistryProvider,
) -> Option<VoxelShape> {
    let def = provider.block_by_state(state_id)?;
    Some(block_collision_shape_for_entry(
        state_id,
        &def.name,
        def.min_state,
        def.max_state,
    ))
}

fn block_collision_shape_for_entry(
    state_id: u16,
    name: &str,
    min_state: u16,
    _max_state: u16,
) -> VoxelShape {
    if is_air_or_fluid_name(name)
        || matches!(
            name,
            "minecraft:nether_portal"
                | "minecraft:end_portal"
                | "minecraft:torch"
                | "minecraft:wall_torch"
                | "minecraft:redstone_torch"
                | "minecraft:redstone_wire"
                | "minecraft:lever"
                | "minecraft:wheat"
                | "minecraft:fire"
        )
    {
        return VoxelShape::Empty;
    }

    let rel = state_id.saturating_sub(min_state);

    if is_non_solid_name(name)
        || name == "minecraft:soul_torch"
        || name == "minecraft:soul_wall_torch"
        || name == "minecraft:redstone_wall_torch"
        || name == "minecraft:copper_torch"
        || name == "minecraft:copper_wall_torch"
    {
        return VoxelShape::Empty;
    }

    if name.ends_with("_carpet") {
        return VoxelShape::Boxes(&CARPET_BOX);
    }

    if name == "minecraft:snow" {
        return if rel == 0 {
            VoxelShape::Empty
        } else {
            let idx = (rel as usize - 1).min(SNOW_COLLISION_LAYERS.len() - 1);
            VoxelShape::Boxes(&SNOW_COLLISION_LAYERS[idx])
        };
    }

    if name.ends_with("_slab") {
        return match rel / 2 {
            0 => VoxelShape::Boxes(&TOP_HALF),
            1 => VoxelShape::Boxes(&BOTTOM_HALF),
            _ => VoxelShape::Full,
        };
    }

    if name.ends_with("_stairs") {
        let facing = (rel / 20).min(3) as usize;
        let half = ((rel / 10) % 2) as usize;
        let shape = ((rel / 2) % 5) as usize;
        let idx = facing * 10 + half * 5 + shape;
        return VoxelShape::Boxes(STAIR_SHAPES[idx].as_slice());
    }

    if name.ends_with("_fence") {
        // properties: east(16), north(8), south(4), waterlogged(2), west(1), 0=true, 1=false
        let east = usize::from((rel / 16).is_multiple_of(2));
        let north = usize::from((rel / 8).is_multiple_of(2));
        let south = usize::from((rel / 4).is_multiple_of(2));
        let west = usize::from(rel.is_multiple_of(2));
        let mask = north | (east << 1) | (south << 2) | (west << 3);
        return VoxelShape::Boxes(FENCE_SHAPES[mask].as_slice());
    }

    if name.ends_with("_wall") {
        // properties: east(108), north(36), south(12), up(6), waterlogged(3), west(1)
        let east = usize::from((rel / 108) != 0);
        let north = usize::from(!(rel / 36).is_multiple_of(3));
        let south = usize::from(!(rel / 12).is_multiple_of(3));
        let up = usize::from((rel / 6).is_multiple_of(2));
        let west = usize::from(!(rel % 3).is_multiple_of(3));
        let mask = north | (east << 1) | (south << 2) | (west << 3) | (up << 4);
        return VoxelShape::Boxes(WALL_SHAPES[mask].as_slice());
    }

    if name.ends_with("_fence_gate") {
        // properties: facing(8), in_wall(4), open(2), powered(1); open: 0=true, 1=false
        let open = (rel / 2).is_multiple_of(2);
        if open {
            return VoxelShape::Empty;
        }
        let facing = (rel / 8).min(3);
        return if facing < 2 {
            VoxelShape::Boxes(&GATE_NS)
        } else {
            VoxelShape::Boxes(&GATE_WE)
        };
    }

    if name.ends_with("_door") {
        // properties: facing(16: N,S,W,E), half(8), hinge(4: 0=left,1=right), open(2: 0=true,1=false), powered(1)
        let facing = (rel / 16).min(3);
        let hinge_right = ((rel / 4) % 2) == 1;
        let open = (rel / 2).is_multiple_of(2);
        let boxes = match (facing, open, hinge_right) {
            (0, false, _) => &WALL_SOUTH,
            (0, true, false) => &WALL_WEST,
            (0, true, true) => &WALL_EAST,
            (1, false, _) => &WALL_NORTH,
            (1, true, false) => &WALL_EAST,
            (1, true, true) => &WALL_WEST,
            (2, false, _) => &WALL_EAST,
            (2, true, false) => &WALL_SOUTH,
            (2, true, true) => &WALL_NORTH,
            (_, false, _) => &WALL_WEST,
            (_, true, false) => &WALL_NORTH,
            (_, true, true) => &WALL_SOUTH,
        };
        return VoxelShape::Boxes(boxes);
    }

    if name.ends_with("_trapdoor") {
        // properties: facing(16), half(8: 0=top,1=bottom), open(4: 0=true,1=false), powered(2), waterlogged(1)
        let facing = (rel / 16).min(3);
        let top = (rel / 8).is_multiple_of(2);
        let open = (rel / 4).is_multiple_of(2);
        if !open {
            return VoxelShape::Boxes(if top { &TRAPDOOR_TOP } else { &TRAPDOOR_BOTTOM });
        }
        return VoxelShape::Boxes(match facing {
            0 => &WALL_SOUTH,
            1 => &WALL_NORTH,
            2 => &WALL_EAST,
            _ => &WALL_WEST,
        });
    }

    if name.ends_with("_glass_pane") || name == "minecraft:iron_bars" {
        let east = usize::from((rel / 16).is_multiple_of(2));
        let north = usize::from((rel / 8).is_multiple_of(2));
        let south = usize::from((rel / 4).is_multiple_of(2));
        let west = usize::from(rel.is_multiple_of(2));
        let mask = north | (east << 1) | (south << 2) | (west << 3);
        return VoxelShape::Boxes(PANE_SHAPES[mask].as_slice());
    }

    if name == "minecraft:ladder" {
        let facing = (rel / 2).min(3);
        return VoxelShape::Boxes(match facing {
            0 => &WALL_SOUTH,
            1 => &WALL_NORTH,
            2 => &WALL_EAST,
            _ => &WALL_WEST,
        });
    }

    if name == "minecraft:scaffolding" {
        let bottom = (rel / 16) == 0;
        return VoxelShape::Boxes(if bottom {
            &SCAFFOLDING_BOTTOM_TOP
        } else {
            &SCAFFOLDING_TOP
        });
    }

    if name == "minecraft:piston_head" {
        let facing = (rel / 4).min(5) as usize;
        return VoxelShape::Boxes(&PISTON_HEAD_SHAPES[facing]);
    }

    if name == "minecraft:repeater" || name == "minecraft:comparator" {
        return VoxelShape::Boxes(&REPEATER_BOX);
    }

    if name.ends_with("_chest")
        || name == "minecraft:chest"
        || name == "minecraft:trapped_chest"
        || name == "minecraft:ender_chest"
    {
        return VoxelShape::Boxes(&CHEST_BOX);
    }

    if name == "minecraft:hopper" {
        return VoxelShape::Boxes(&HOPPER_BOXES);
    }

    if name == "minecraft:cauldron"
        || name == "minecraft:water_cauldron"
        || name == "minecraft:lava_cauldron"
        || name == "minecraft:powder_snow_cauldron"
    {
        return VoxelShape::Boxes(&CAULDRON_BOXES);
    }

    if name == "minecraft:lectern" {
        return VoxelShape::Boxes(&LECTERN_BOX);
    }

    if name == "minecraft:bell" {
        return VoxelShape::Boxes(&BELL_BOX);
    }

    if name == "minecraft:farmland" || name == "minecraft:dirt_path" {
        return VoxelShape::Boxes(&FARMLAND_BOX);
    }

    if name == "minecraft:soul_sand" {
        return VoxelShape::Boxes(&SOUL_SAND_BOX);
    }

    if name == "minecraft:honey_block" {
        return VoxelShape::Boxes(&HONEY_BOX);
    }

    if name == "minecraft:cactus" {
        return VoxelShape::Boxes(&CACTUS_BOX);
    }

    if name == "minecraft:bamboo" {
        return VoxelShape::Boxes(&BAMBOO_BOX);
    }

    if name == "minecraft:lily_pad" {
        return VoxelShape::Boxes(&LILY_PAD_BOX);
    }

    if name.ends_with("_bed") {
        return VoxelShape::Boxes(&BED_BOX);
    }

    if name == "minecraft:cake" {
        let idx = (rel as usize).min(CAKE_BOXES.len() - 1);
        return VoxelShape::Boxes(&CAKE_BOXES[idx]);
    }

    if matches!(
        name,
        "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil"
    ) {
        let facing = rel.min(3);
        return VoxelShape::Boxes(if facing < 2 { &ANVIL_NS } else { &ANVIL_WE });
    }

    if name == "minecraft:stonecutter" {
        return VoxelShape::Boxes(&STONECUTTER_BOX);
    }

    if name == "minecraft:campfire" || name == "minecraft:soul_campfire" {
        return VoxelShape::Boxes(&CAMPFIRE_BOX);
    }

    if name == "minecraft:daylight_detector" {
        return VoxelShape::Boxes(&DAYLIGHT_BOX);
    }

    if name == "minecraft:enchanting_table" {
        return VoxelShape::Boxes(&ENCHANTING_BOX);
    }

    if name == "minecraft:end_portal_frame" {
        return VoxelShape::Boxes(&END_PORTAL_FRAME_BOX);
    }

    if name.ends_with("_lantern") {
        // properties: hanging(2: 0=true,1=false), waterlogged(1)
        let hanging = (rel / 2) == 0;
        return VoxelShape::Boxes(if hanging {
            &LANTERN_HANGING_BOX
        } else {
            &LANTERN_STANDING_BOX
        });
    }

    if name.ends_with("_chain") || name == "minecraft:chain" {
        // axis: 0=x, 1=y, 2=z (waterlogged stride 1)
        let axis = (rel / 2).min(2);
        return VoxelShape::Boxes(match axis {
            0 => &AXIS_X_THIN,
            1 => &AXIS_Y_THIN,
            _ => &AXIS_Z_THIN,
        });
    }

    if name == "minecraft:end_rod" || name == "minecraft:lightning_rod" {
        let facing = if name == "minecraft:lightning_rod" {
            (rel / 4).min(5)
        } else {
            rel.min(5)
        };
        return VoxelShape::Boxes(match facing {
            0 | 2 => &AXIS_Z_THIN, // N / S
            1 | 3 => &AXIS_X_THIN, // E / W
            _ => &AXIS_Y_THIN,     // U / D
        });
    }

    if name == "minecraft:flower_pot" || name.starts_with("minecraft:potted_") {
        return VoxelShape::Boxes(&FLOWER_POT_BOX);
    }

    if name.ends_with("_head") || name.ends_with("_skull") {
        return VoxelShape::Boxes(&HEAD_BOX);
    }

    if name == "minecraft:brewing_stand" {
        return VoxelShape::Boxes(&BREWING_STAND_BOX);
    }

    VoxelShape::Full
}

/// Computes the outline/interaction [`VoxelShape`] for `state_id` using a [`RegistryProvider`].
#[must_use]
pub fn block_interaction_shape_with(
    state_id: u16,
    provider: &dyn RegistryProvider,
) -> Option<VoxelShape> {
    let def = provider.block_by_state(state_id)?;
    let name = def.name.as_str();
    if is_air_or_fluid_name(name) {
        return Some(VoxelShape::Empty);
    }
    if name == "minecraft:redstone_wire" {
        return Some(VoxelShape::Boxes(&REPEATER_BOX));
    }
    if matches!(
        name,
        "minecraft:torch" | "minecraft:wall_torch" | "minecraft:redstone_torch"
    ) {
        return Some(VoxelShape::Boxes(&TORCH_BOX));
    }
    if name == "minecraft:lever" {
        return Some(VoxelShape::Boxes(&LEVER_BOX));
    }
    if name == "minecraft:snow" && state_id == def.min_state {
        return Some(VoxelShape::Boxes(&SNOW_LAYER_1_INTERACT));
    }
    if name.ends_with("_fence_gate") {
        let rel = state_id.saturating_sub(def.min_state);
        let facing = (rel / 8).min(3);
        return Some(VoxelShape::Boxes(if facing < 2 {
            &GATE_NS
        } else {
            &GATE_WE
        }));
    }
    if name.ends_with("_button") {
        return Some(VoxelShape::Boxes(&BUTTON_BOX));
    }
    if name.ends_with("_pressure_plate") {
        return Some(VoxelShape::Boxes(&PRESSURE_PLATE_BOX));
    }
    if name.ends_with("_rail") || name == "minecraft:rail" {
        return Some(VoxelShape::Boxes(&RAIL_BOX));
    }
    if is_non_solid_name(name) {
        return Some(VoxelShape::Boxes(&PLANT_BOX));
    }

    Some(block_collision_shape_for_entry(
        state_id,
        name,
        def.min_state,
        def.max_state,
    ))
}

/// Returns `true` if the top surface of `state_id` provides full-width redstone/floor support (`max_y >= 1.0`).
#[must_use]
pub fn block_supports_top_face_with(
    state_id: u16,
    provider: &dyn RegistryProvider,
) -> Option<bool> {
    let shape = block_collision_shape_with(state_id, provider)?;
    Some(shape.as_boxes().iter().any(|b| {
        b.min_x <= 0.0 && b.min_z <= 0.0 && b.max_x >= 1.0 && b.max_z >= 1.0 && b.max_y >= 1.0
    }))
}

#[cfg(test)]
mod tests {
    use rad_core::registry_provider::SyntheticFixtureRegistryProvider;

    use super::*;

    #[test]
    fn shapes_work_with_synthetic_provider_and_report_missing_states() {
        let provider = SyntheticFixtureRegistryProvider::minimal_valid();
        assert_eq!(
            block_collision_shape_with(0, &provider),
            Some(VoxelShape::Empty)
        );
        assert_eq!(
            block_collision_shape_with(1, &provider),
            Some(VoxelShape::Full)
        );
        assert_eq!(
            block_interaction_shape_with(1, &provider),
            Some(VoxelShape::Full)
        );
        assert_eq!(block_supports_top_face_with(1, &provider), Some(true));
        assert_eq!(block_supports_top_face_with(0, &provider), Some(false));
        assert_eq!(block_collision_shape_with(999, &provider), None);
        assert_eq!(block_interaction_shape_with(999, &provider), None);
        assert_eq!(block_supports_top_face_with(999, &provider), None);
    }
}
