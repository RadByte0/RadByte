//! Seeded terrain, resource veins, trees, and multi-dimension chunk generation (M6 & M7).

use serde::{Deserialize, Serialize};

use crate::world::OVERWORLD_SECTIONS;
use crate::world::anvil::{ChunkColumn, ChunkSection, MIN_WORLD_Y};

const SECTION_HEIGHT: usize = 16;

/// Minecraft dimension identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum DimensionId {
    /// Standard Overworld (`minecraft:overworld`).
    #[default]
    Overworld = 0,
    /// The Nether (`minecraft:the_nether`).
    Nether = 1,
    /// The End (`minecraft:the_end`).
    End = 2,
}

impl DimensionId {
    /// Parses a dimension name (`overworld`, `nether`, `end`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "overworld" | "minecraft:overworld" | "0" => Some(Self::Overworld),
            "nether" | "the_nether" | "minecraft:the_nether" | "1" => Some(Self::Nether),
            "end" | "the_end" | "minecraft:the_end" | "2" => Some(Self::End),
            _ => None,
        }
    }

    /// Returns the Minecraft resource location for this dimension.
    #[must_use]
    pub const fn resource_name(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
        }
    }

    /// Returns the numeric `dimension_type` registry index matching `minimal_777_registries()`.
    #[must_use]
    pub const fn type_id(self) -> i32 {
        match self {
            Self::Overworld => 0,
            Self::Nether => 1,
            Self::End => 2,
        }
    }

    /// Returns the Anvil subdirectory relative to `world_dir` (`None` for Overworld).
    #[must_use]
    pub const fn anvil_subdir(self) -> Option<&'static str> {
        match self {
            Self::Overworld => None,
            Self::Nether => Some("DIM-1"),
            Self::End => Some("DIM1"),
        }
    }
}

/// World chunk generation strategy.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WorldGenerator {
    /// Deterministic flat survival testbed (`y = 63` grass).
    #[default]
    Flat,
    /// Seeded procedural terrain with biomes, water, caves/ores, and trees.
    Seeded(i64),
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

fn hash2d(seed: i64, x: i32, z: i32) -> f64 {
    let key = (seed as u64)
        ^ ((x as i64 as u64).wrapping_mul(0x517C_C1B7_2722_0A95))
        ^ ((z as i64 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let h = splitmix64(key);
    (h & 0xFFFF_FFFF) as f64 / f64::from(u32::MAX)
}

fn value_noise_2d(seed: i64, wx: f64, wz: f64, scale: f64) -> f64 {
    let sx = wx / scale;
    let sz = wz / scale;
    let x0 = sx.floor() as i32;
    let z0 = sz.floor() as i32;
    let fx = sx - f64::from(x0);
    let fz = sz - f64::from(z0);
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uz = fz * fz * (3.0 - 2.0 * fz);

    let v00 = hash2d(seed, x0, z0);
    let v10 = hash2d(seed, x0 + 1, z0);
    let v01 = hash2d(seed, x0, z0 + 1);
    let v11 = hash2d(seed, x0 + 1, z0 + 1);

    let a = v00 + (v10 - v00) * ux;
    let b = v01 + (v11 - v01) * ux;
    a + (b - a) * uz
}

fn value_noise_3d(seed: i64, x: f64, y: f64, z: f64, scale: f64) -> f64 {
    let sx = x / scale;
    let sy = y / scale;
    let sz = z / scale;
    let x0 = sx.floor() as i64;
    let y0 = sy.floor() as i64;
    let z0 = sz.floor() as i64;
    let fx = sx - x0 as f64;
    let fy = sy - y0 as f64;
    let fz = sz - z0 as f64;
    let ux = fx * fx * (3.0 - 2.0 * fx);
    let uy = fy * fy * (3.0 - 2.0 * fy);
    let uz = fz * fz * (3.0 - 2.0 * fz);

    let corner = |ix: i64, iy: i64, iz: i64| -> f64 {
        let h = splitmix64(
            (seed as u64)
                ^ (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                ^ (iy as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9)
                ^ (iz as u64).wrapping_mul(0x94D0_49BB_1331_11EB),
        );
        ((h >> 11) as f64) / ((1u64 << 53) as f64)
    };

    let c000 = corner(x0, y0, z0);
    let c100 = corner(x0 + 1, y0, z0);
    let c010 = corner(x0, y0 + 1, z0);
    let c110 = corner(x0 + 1, y0 + 1, z0);
    let c001 = corner(x0, y0, z0 + 1);
    let c101 = corner(x0 + 1, y0, z0 + 1);
    let c011 = corner(x0, y0 + 1, z0 + 1);
    let c111 = corner(x0 + 1, y0 + 1, z0 + 1);

    let x00 = c000 + (c100 - c000) * ux;
    let x10 = c010 + (c110 - c010) * ux;
    let x01 = c001 + (c101 - c001) * ux;
    let x11 = c011 + (c111 - c011) * ux;
    let y0v = x00 + (x10 - x00) * uy;
    let y1v = x01 + (x11 - x01) * uy;
    y0v + (y1v - y0v) * uz
}

/// Resolved block-state palette for world generation, decoupled from compile-time `mc777_data`.
#[derive(Debug, Clone, Copy)]
pub struct WorldgenPalette {
    /// Air state ID.
    pub air: u16,
    /// Bedrock state ID.
    pub bedrock: u16,
    /// Stone state ID.
    pub stone: u16,
    /// Deepslate state ID.
    pub deepslate: u16,
    /// Dirt state ID.
    pub dirt: u16,
    /// Grass block state ID.
    pub grass_block: u16,
    /// Sand state ID.
    pub sand: u16,
    /// Sandstone state ID.
    pub sandstone: u16,
    /// Water source state ID.
    pub water: u16,
    /// Lava source state ID.
    pub lava: u16,
    /// Coal ore state ID.
    pub coal_ore: u16,
    /// Iron ore state ID.
    pub iron_ore: u16,
    /// Copper ore state ID.
    pub copper_ore: u16,
    /// Gold ore state ID.
    pub gold_ore: u16,
    /// Redstone ore state ID.
    pub redstone_ore: u16,
    /// Lapis ore state ID.
    pub lapis_ore: u16,
    /// Diamond ore state ID.
    pub diamond_ore: u16,
    /// Deepslate diamond ore state ID.
    pub deepslate_diamond_ore: u16,
    /// Oak log state ID.
    pub oak_log: u16,
    /// Oak leaves state ID.
    pub oak_leaves: u16,
    /// Birch log state ID.
    pub birch_log: u16,
    /// Birch leaves state ID.
    pub birch_leaves: u16,
    /// Spruce log state ID.
    pub spruce_log: u16,
    /// Spruce leaves state ID.
    pub spruce_leaves: u16,
    /// Oak planks state ID.
    pub oak_planks: u16,
    /// Cobblestone state ID.
    pub cobblestone: u16,
    /// Mossy cobblestone state ID.
    pub mossy_cobblestone: u16,
    /// Obsidian state ID.
    pub obsidian: u16,
    /// Crying obsidian state ID.
    pub crying_obsidian: u16,
    /// Gold block state ID.
    pub gold_block: u16,
    /// Chest state ID.
    pub chest: u16,
    /// Monster spawner state ID.
    pub spawner: u16,
    /// Short grass state ID.
    pub short_grass: u16,
    /// Dandelion state ID.
    pub dandelion: u16,
    /// Poppy state ID.
    pub poppy: u16,
    /// Cactus state ID.
    pub cactus: u16,
    /// TNT state ID.
    pub tnt: u16,
    /// Orange terracotta state ID.
    pub orange_terracotta: u16,
    /// Blue terracotta state ID.
    pub blue_terracotta: u16,
    /// Crafting table state ID.
    pub crafting_table: u16,
    /// Furnace state ID.
    pub furnace: u16,
    /// Glass state ID.
    pub glass: u16,
    /// Nether bricks state ID.
    pub nether_bricks: u16,
    /// Nether quartz ore state ID.
    pub nether_quartz_ore: u16,
    /// Nether gold ore state ID.
    pub nether_gold_ore: u16,
    /// Glowstone state ID.
    pub glowstone: u16,
    /// Netherrack state ID.
    pub netherrack: u16,
    /// End stone state ID.
    pub end_stone: u16,
}

impl WorldgenPalette {
    /// Resolves all worldgen block states from `provider`, failing loudly if any core block is missing.
    pub fn from_provider(
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> Result<Self, rad_core::registry_provider::RegistryProviderError> {
        let req = |name: &str| -> Result<u16, rad_core::registry_provider::RegistryProviderError> {
            provider
                .block_by_name(name)
                .map(|b| b.default_state)
                .ok_or_else(|| {
                    rad_core::registry_provider::RegistryProviderError::MissingData(format!(
                        "missing worldgen block '{name}'"
                    ))
                })
        };
        let opt = |name: &str, fallback: u16| -> u16 {
            provider
                .block_by_name(name)
                .map_or(fallback, |b| b.default_state)
        };
        let air = req("minecraft:air")?;
        let bedrock = req("minecraft:bedrock")?;
        let stone = req("minecraft:stone")?;
        let dirt = req("minecraft:dirt")?;
        let grass_block = req("minecraft:grass_block")?;
        let sand = req("minecraft:sand")?;
        let water = req("minecraft:water")?;
        let coal_ore = req("minecraft:coal_ore")?;
        let iron_ore = req("minecraft:iron_ore")?;
        let diamond_ore = req("minecraft:diamond_ore")?;
        let oak_log = req("minecraft:oak_log")?;
        let oak_leaves = req("minecraft:oak_leaves")?;
        let netherrack = req("minecraft:netherrack")?;
        let end_stone = req("minecraft:end_stone")?;
        let cobblestone = opt("minecraft:cobblestone", stone);
        let mossy_cobblestone = opt("minecraft:mossy_cobblestone", cobblestone);
        let obsidian = opt("minecraft:obsidian", stone);
        Ok(Self {
            air,
            bedrock,
            stone,
            deepslate: opt("minecraft:deepslate", stone),
            dirt,
            grass_block,
            sand,
            sandstone: opt("minecraft:sandstone", sand),
            water,
            lava: opt("minecraft:lava", air),
            coal_ore,
            iron_ore,
            copper_ore: opt("minecraft:copper_ore", iron_ore),
            gold_ore: opt("minecraft:gold_ore", iron_ore),
            redstone_ore: opt("minecraft:redstone_ore", coal_ore),
            lapis_ore: opt("minecraft:lapis_ore", coal_ore),
            diamond_ore,
            deepslate_diamond_ore: opt("minecraft:deepslate_diamond_ore", diamond_ore),
            oak_log,
            oak_leaves,
            birch_log: opt("minecraft:birch_log", oak_log),
            birch_leaves: opt("minecraft:birch_leaves", oak_leaves),
            spruce_log: opt("minecraft:spruce_log", oak_log),
            spruce_leaves: opt("minecraft:spruce_leaves", oak_leaves),
            oak_planks: opt("minecraft:oak_planks", oak_log),
            cobblestone,
            mossy_cobblestone,
            obsidian,
            crying_obsidian: opt("minecraft:crying_obsidian", obsidian),
            gold_block: opt("minecraft:gold_block", stone),
            chest: opt("minecraft:chest", opt("minecraft:oak_planks", oak_log)),
            spawner: opt("minecraft:spawner", mossy_cobblestone),
            short_grass: opt("minecraft:short_grass", air),
            dandelion: opt("minecraft:dandelion", air),
            poppy: opt("minecraft:poppy", air),
            cactus: opt("minecraft:cactus", air),
            tnt: opt("minecraft:tnt", sand),
            orange_terracotta: opt("minecraft:orange_terracotta", sand),
            blue_terracotta: opt("minecraft:blue_terracotta", sand),
            crafting_table: opt(
                "minecraft:crafting_table",
                opt("minecraft:oak_planks", oak_log),
            ),
            furnace: opt("minecraft:furnace", cobblestone),
            glass: opt("minecraft:glass", air),
            nether_bricks: opt("minecraft:nether_bricks", netherrack),
            nether_quartz_ore: opt("minecraft:nether_quartz_ore", netherrack),
            nether_gold_ore: opt("minecraft:nether_gold_ore", netherrack),
            glowstone: opt("minecraft:glowstone", netherrack),
            netherrack,
            end_stone,
        })
    }
}

/// Generates a deterministic seeded [`ChunkColumn`] for `(cx, cz)` in `dim` using `palette`.
#[must_use]
pub fn generate_seeded_chunk_with(
    seed: i64,
    dim: DimensionId,
    cx: i32,
    cz: i32,
    palette: &WorldgenPalette,
) -> ChunkColumn {
    let mut col =
        ChunkColumn::from_sections(vec![ChunkSection::Uniform(palette.air); OVERWORLD_SECTIONS]);
    match dim {
        DimensionId::Overworld => generate_overworld(&mut col, seed, cx, cz, palette),
        DimensionId::Nether => generate_nether(&mut col, seed, cx, cz, palette),
        DimensionId::End => generate_end(&mut col, seed, cx, cz, palette),
    }
    col.clear_dirty();
    col
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverworldBiome {
    Plains,
    Forest,
    BirchForest,
    Taiga,
    Desert,
    Mountains,
    Ocean,
}

fn sample_overworld_biome(seed: i64, wx: i32, wz: i32) -> (OverworldBiome, i32) {
    let fx = f64::from(wx);
    let fz = f64::from(wz);
    let continental = value_noise_2d(seed ^ 0x1111_2222, fx, fz, 128.0);
    let temp = value_noise_2d(seed ^ 0x3333_4444, fx, fz, 160.0);
    let humid = value_noise_2d(seed ^ 0x5555_6666, fx, fz, 160.0);
    let detail1 = value_noise_2d(seed, fx, fz, 48.0);
    let detail2 = value_noise_2d(seed ^ 0x1234_5678, fx, fz, 16.0);

    if continental < 0.30 {
        let depth = ((0.30 - continental) * 48.0).round() as i32;
        return (OverworldBiome::Ocean, (60 - depth).max(38));
    }
    if continental > 0.72 {
        let peak = ((continental - 0.72) * 110.0).round() as i32;
        return (
            OverworldBiome::Mountains,
            66 + peak + ((detail1 * 12.0 + detail2 * 5.0).round() as i32),
        );
    }
    let base_y = 60 + ((detail1 * 10.0 + detail2 * 4.0).round() as i32);
    let biome = if temp > 0.66 && humid < 0.45 {
        OverworldBiome::Desert
    } else if temp < 0.35 {
        OverworldBiome::Taiga
    } else if humid > 0.62 {
        OverworldBiome::Forest
    } else if humid > 0.48 {
        OverworldBiome::BirchForest
    } else {
        OverworldBiome::Plains
    };
    (biome, base_y)
}

fn place_canopy_tree(
    col: &mut ChunkColumn,
    lx: usize,
    base_y: i32,
    lz: usize,
    log: u16,
    leaves: u16,
    height: i32,
) {
    for ty in 1..=height {
        col.set_block(lx, base_y + ty, lz, log);
    }
    for dy in (height - 2)..=height {
        let radius: isize = if dy == height { 1 } else { 2 };
        for dx in -radius..=radius {
            for dz in -radius..=radius {
                if dx.abs() == 2 && dz.abs() == 2 && dy == height - 1 {
                    continue;
                }
                let tx = lx as isize + dx;
                let tz = lz as isize + dz;
                if (0..16).contains(&tx)
                    && (0..16).contains(&tz)
                    && !(dx == 0 && dz == 0 && dy < height)
                {
                    col.set_block(tx as usize, base_y + dy, tz as usize, leaves);
                }
            }
        }
    }
    col.set_block(lx, base_y + height + 1, lz, leaves);
}

fn generate_ruined_portal(
    col: &mut ChunkColumn,
    palette: &WorldgenPalette,
    base_x: usize,
    base_y: i32,
    base_z: usize,
    hash: u64,
) {
    for dx in 0..5usize {
        for dz in 0..4usize {
            let block = if (dx + dz + (hash as usize)).is_multiple_of(3) {
                palette.netherrack
            } else {
                palette.stone
            };
            col.set_block(base_x + dx, base_y, base_z + dz, block);
        }
    }
    for dy in 1..=5 {
        let b_left = if (dy as u64 + hash).is_multiple_of(4) {
            palette.crying_obsidian
        } else {
            palette.obsidian
        };
        col.set_block(base_x + 1, base_y + dy, base_z + 1, b_left);
        if dy <= 4 {
            col.set_block(base_x + 4, base_y + dy, base_z + 1, palette.obsidian);
        }
    }
    for dx in 1..=3 {
        col.set_block(base_x + dx, base_y + 5, base_z + 1, palette.obsidian);
    }
    col.set_block(base_x + 2, base_y + 1, base_z + 1, palette.obsidian);
    col.set_block(base_x + 3, base_y + 1, base_z + 1, palette.obsidian);
    col.set_block(base_x + 2, base_y + 6, base_z + 1, palette.gold_block);
    col.set_block(base_x, base_y + 1, base_z + 2, palette.chest);
}

fn generate_dungeon_room(
    col: &mut ChunkColumn,
    palette: &WorldgenPalette,
    base_x: usize,
    base_y: i32,
    base_z: usize,
    hash: u64,
) {
    for dx in 0..7usize {
        for dz in 0..7usize {
            let floor = if ((dx * 7 + dz) as u64 ^ hash).is_multiple_of(3) {
                palette.mossy_cobblestone
            } else {
                palette.cobblestone
            };
            col.set_block(base_x + dx, base_y, base_z + dz, floor);
            col.set_block(base_x + dx, base_y + 5, base_z + dz, palette.cobblestone);
            for dy in 1..=4 {
                let is_wall = dx == 0 || dx == 6 || dz == 0 || dz == 6;
                if is_wall {
                    col.set_block(base_x + dx, base_y + dy, base_z + dz, palette.cobblestone);
                } else {
                    col.set_block(base_x + dx, base_y + dy, base_z + dz, palette.air);
                }
            }
        }
    }
    col.set_block(base_x + 3, base_y + 1, base_z + 3, palette.spawner);
    col.set_block(base_x + 1, base_y + 1, base_z + 1, palette.chest);
    col.set_block(base_x + 5, base_y + 1, base_z + 5, palette.chest);
}

fn generate_desert_well(
    col: &mut ChunkColumn,
    palette: &WorldgenPalette,
    base_x: usize,
    base_y: i32,
    base_z: usize,
) {
    for dx in 0..5usize {
        for dz in 0..5usize {
            col.set_block(base_x + dx, base_y, base_z + dz, palette.sandstone);
            let corner = (dx == 1 || dx == 3) && (dz == 1 || dz == 3);
            let rim = (1..=3).contains(&dx) && (1..=3).contains(&dz);
            if corner {
                col.set_block(base_x + dx, base_y + 1, base_z + dz, palette.sandstone);
                col.set_block(base_x + dx, base_y + 2, base_z + dz, palette.sandstone);
                col.set_block(base_x + dx, base_y + 3, base_z + dz, palette.sandstone);
            } else if rim {
                col.set_block(base_x + dx, base_y + 1, base_z + dz, palette.water);
                col.set_block(base_x + dx, base_y + 3, base_z + dz, palette.sandstone);
            }
        }
    }
    col.set_block(base_x + 2, base_y + 4, base_z + 2, palette.sandstone);
}

fn generate_desert_pyramid(
    col: &mut ChunkColumn,
    palette: &WorldgenPalette,
    base_x: usize,
    base_y: i32,
    base_z: usize,
) {
    // 9x9 sandstone pyramid with underground TNT chamber & 4 loot chests
    for layer in 0..5usize {
        let size = 9 - layer * 2;
        let ox = base_x + layer;
        let oz = base_z + layer;
        for dx in 0..size {
            for dz in 0..size {
                let wall = layer == 0 || dx == 0 || dz == 0 || dx + 1 == size || dz + 1 == size;
                let block = if wall {
                    if layer == 2 && (dx == size / 2 || dz == size / 2) {
                        palette.orange_terracotta
                    } else {
                        palette.sandstone
                    }
                } else {
                    palette.air
                };
                col.set_block(ox + dx, base_y + layer as i32, oz + dz, block);
            }
        }
    }
    // Decorative floor & hidden chamber below
    col.set_block(base_x + 4, base_y, base_z + 4, palette.blue_terracotta);
    for dy in 1..=5 {
        for dx in 3..=5 {
            for dz in 3..=5 {
                let wall = dx == 3 || dx == 5 || dz == 3 || dz == 5;
                let b = if dy == 5 || wall {
                    palette.sandstone
                } else {
                    palette.air
                };
                col.set_block(base_x + dx, base_y - dy, base_z + dz, b);
            }
        }
    }
    for dx in 3..=5 {
        for dz in 3..=5 {
            col.set_block(base_x + dx, base_y - 5, base_z + dz, palette.tnt);
        }
    }
    col.set_block(base_x + 3, base_y - 4, base_z + 4, palette.chest);
    col.set_block(base_x + 5, base_y - 4, base_z + 4, palette.chest);
    col.set_block(base_x + 4, base_y - 4, base_z + 3, palette.chest);
    col.set_block(base_x + 4, base_y - 4, base_z + 5, palette.chest);
}

fn generate_village_house(
    col: &mut ChunkColumn,
    palette: &WorldgenPalette,
    base_x: usize,
    base_y: i32,
    base_z: usize,
) {
    // 5x5 oak & cobblestone village house with crafting table, furnace & chest
    for dx in 0..5usize {
        for dz in 0..5usize {
            col.set_block(base_x + dx, base_y, base_z + dz, palette.cobblestone);
            let corner = (dx == 0 || dx == 4) && (dz == 0 || dz == 4);
            let wall = dx == 0 || dx == 4 || dz == 0 || dz == 4;
            for dy in 1..=3 {
                let block = if corner {
                    palette.oak_log
                } else if wall {
                    if dx == 2 && dz == 0 && (dy == 1 || dy == 2) {
                        palette.air // doorway
                    } else if dy == 2 && ((dx == 2 && dz == 4) || (dz == 2 && (dx == 0 || dx == 4)))
                    {
                        palette.glass
                    } else {
                        palette.oak_planks
                    }
                } else {
                    palette.air
                };
                col.set_block(base_x + dx, base_y + dy, base_z + dz, block);
            }
            col.set_block(base_x + dx, base_y + 4, base_z + dz, palette.oak_planks);
        }
    }
    col.set_block(base_x + 1, base_y + 1, base_z + 3, palette.crafting_table);
    col.set_block(base_x + 2, base_y + 1, base_z + 3, palette.furnace);
    col.set_block(base_x + 3, base_y + 1, base_z + 3, palette.chest);
}

/// Generates deterministic vanilla-style structure loot for a newly opened worldgen chest.
pub fn structure_chest_loot_with(
    seed: i64,
    x: i32,
    y: i32,
    z: i32,
    regs: &crate::registries::Version26_3Registries,
) -> Vec<rad_core::packet::play::SimpleSlot> {
    let mut slots = vec![rad_core::packet::play::SimpleSlot::default(); 27];
    let loot_names = [
        ("minecraft:bread", 3),
        ("minecraft:coal", 6),
        ("minecraft:iron_ingot", 4),
        ("minecraft:gold_ingot", 3),
        ("minecraft:golden_apple", 1),
        ("minecraft:obsidian", 2),
        ("minecraft:flint_and_steel", 1),
        ("minecraft:bone", 5),
        ("minecraft:string", 4),
        ("minecraft:gunpowder", 4),
        ("minecraft:emerald", 2),
        ("minecraft:diamond", 1),
        ("minecraft:arrow", 8),
        ("minecraft:torch", 6),
    ];
    let mut h = splitmix64(
        (seed as u64)
            ^ ((x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
            ^ ((y as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9))
            ^ ((z as u64).wrapping_mul(0x94D0_49BB_1331_11EB)),
    );
    let num_items = 4 + (h % 5) as usize;
    for _ in 0..num_items {
        h = splitmix64(h);
        let slot_idx = (h as usize) % 27;
        let (item_name, max_count) = loot_names[((h >> 8) as usize) % loot_names.len()];
        if let Some(item_id) = regs.item_by_name(item_name) {
            let count = 1 + (((h >> 16) as i32).abs() % max_count);
            slots[slot_idx] = rad_core::packet::play::SimpleSlot { count, item_id };
        }
    }
    slots
}

fn generate_overworld(
    col: &mut ChunkColumn,
    seed: i64,
    cx: i32,
    cz: i32,
    palette: &WorldgenPalette,
) {
    const SEA_LEVEL: i32 = 62;
    let mut center_biome = OverworldBiome::Plains;
    let mut center_surface = 64;

    for lz in 0..16usize {
        for lx in 0..16usize {
            let wx = cx * 16 + lx as i32;
            let wz = cz * 16 + lz as i32;
            let (biome, surface_y) = sample_overworld_biome(seed, wx, wz);
            if lx == 8 && lz == 8 {
                center_biome = biome;
                center_surface = surface_y;
            }

            col.set_block(lx, MIN_WORLD_Y, lz, palette.bedrock);
            for y in (MIN_WORLD_Y + 1)..=(surface_y - 4) {
                let is_deepslate = y < -4 || (y <= 0 && ((wx + y + wz) & 1) == 0);
                let cave_val = if y < surface_y - 6 && y > MIN_WORLD_Y + 3 {
                    value_noise_3d(
                        seed ^ 0xCAFE_BABE,
                        f64::from(wx),
                        f64::from(y) * 1.4,
                        f64::from(wz),
                        18.0,
                    )
                } else {
                    0.5
                };
                if cave_val > 0.71 {
                    let block = if y <= -52 { palette.lava } else { palette.air };
                    col.set_block(lx, y, lz, block);
                    continue;
                }

                let ore_h = splitmix64(
                    (seed as u64)
                        ^ ((wx as u64).wrapping_mul(73856093))
                        ^ ((y as u64).wrapping_mul(19349663))
                        ^ ((wz as u64).wrapping_mul(83492791)),
                );
                let block = if y <= 12 && (ore_h & 0x7F) == 1 {
                    if is_deepslate {
                        palette.deepslate_diamond_ore
                    } else {
                        palette.diamond_ore
                    }
                } else if (-32..=32).contains(&y) && (ore_h & 0x7F) == 5 {
                    palette.gold_ore
                } else if y <= 15 && (ore_h & 0x3F) == 7 {
                    palette.redstone_ore
                } else if (-32..=32).contains(&y) && (ore_h & 0xFF) == 9 {
                    palette.lapis_ore
                } else if y < 56 && (ore_h & 0x2F) == 2 {
                    palette.iron_ore
                } else if (0..=80).contains(&y) && (ore_h & 0x2F) == 11 {
                    palette.copper_ore
                } else if (0..72).contains(&y) && (ore_h & 0x1F) == 3 {
                    palette.coal_ore
                } else if is_deepslate {
                    palette.deepslate
                } else {
                    palette.stone
                };
                col.set_block(lx, y, lz, block);
            }

            for y in (surface_y - 3).max(MIN_WORLD_Y + 1)..surface_y {
                let sub = if biome == OverworldBiome::Desert || surface_y <= SEA_LEVEL + 1 {
                    if y == surface_y - 3 && biome == OverworldBiome::Desert {
                        palette.sandstone
                    } else {
                        palette.sand
                    }
                } else if biome == OverworldBiome::Mountains && surface_y > 86 {
                    palette.stone
                } else {
                    palette.dirt
                };
                col.set_block(lx, y, lz, sub);
            }
            let top = if biome == OverworldBiome::Desert || surface_y <= SEA_LEVEL + 1 {
                palette.sand
            } else if biome == OverworldBiome::Mountains && surface_y > 86 {
                palette.stone
            } else {
                palette.grass_block
            };
            col.set_block(lx, surface_y, lz, top);

            if surface_y < SEA_LEVEL {
                for y in (surface_y + 1)..=SEA_LEVEL {
                    col.set_block(lx, y, lz, palette.water);
                }
            } else {
                let surf_h = splitmix64(
                    (seed as u64) ^ ((wx as u64).wrapping_mul(31)) ^ (wz as u64).wrapping_mul(97),
                );
                let can_place_tree = (2..14).contains(&lx) && (2..14).contains(&lz);
                match biome {
                    OverworldBiome::Forest
                    | OverworldBiome::BirchForest
                    | OverworldBiome::Taiga
                        if can_place_tree && surf_h.is_multiple_of(37) =>
                    {
                        let (log, leaves, h) = if biome == OverworldBiome::BirchForest {
                            (
                                palette.birch_log,
                                palette.birch_leaves,
                                5 + (surf_h % 2) as i32,
                            )
                        } else if biome == OverworldBiome::Taiga {
                            (
                                palette.spruce_log,
                                palette.spruce_leaves,
                                6 + (surf_h % 2) as i32,
                            )
                        } else {
                            (palette.oak_log, palette.oak_leaves, 4 + (surf_h % 2) as i32)
                        };
                        place_canopy_tree(col, lx, surface_y, lz, log, leaves, h);
                    }
                    OverworldBiome::Plains if can_place_tree && surf_h.is_multiple_of(131) => {
                        place_canopy_tree(
                            col,
                            lx,
                            surface_y,
                            lz,
                            palette.oak_log,
                            palette.oak_leaves,
                            4,
                        );
                    }
                    OverworldBiome::Desert
                        if surf_h.is_multiple_of(73) && palette.cactus != palette.air =>
                    {
                        let ch = 1 + (surf_h % 3) as i32;
                        for cy in 1..=ch {
                            col.set_block(lx, surface_y + cy, lz, palette.cactus);
                        }
                    }
                    OverworldBiome::Plains
                    | OverworldBiome::Forest
                    | OverworldBiome::BirchForest
                        if top == palette.grass_block =>
                    {
                        if surf_h.is_multiple_of(11) && palette.short_grass != palette.air {
                            col.set_block(lx, surface_y + 1, lz, palette.short_grass);
                        } else if surf_h.is_multiple_of(43) && palette.dandelion != palette.air {
                            col.set_block(lx, surface_y + 1, lz, palette.dandelion);
                        } else if surf_h.is_multiple_of(59) && palette.poppy != palette.air {
                            col.set_block(lx, surface_y + 1, lz, palette.poppy);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let chunk_h = splitmix64(
        (seed as u64)
            ^ ((cx as u64).wrapping_mul(0x9E37_79B9))
            ^ ((cz as u64).wrapping_mul(0xBF58_476D)),
    );
    if center_surface > SEA_LEVEL + 1 && chunk_h.is_multiple_of(64) {
        generate_ruined_portal(col, palette, 5, center_surface, 6, chunk_h);
    } else if center_biome == OverworldBiome::Desert
        && center_surface > SEA_LEVEL + 1
        && chunk_h.is_multiple_of(79)
    {
        generate_desert_pyramid(col, palette, 3, center_surface, 3);
    } else if center_biome == OverworldBiome::Desert
        && center_surface > SEA_LEVEL + 1
        && chunk_h.is_multiple_of(41)
    {
        generate_desert_well(col, palette, 5, center_surface, 5);
    } else if matches!(
        center_biome,
        OverworldBiome::Plains | OverworldBiome::Forest | OverworldBiome::Taiga
    ) && center_surface > SEA_LEVEL + 1
        && chunk_h.is_multiple_of(53)
    {
        generate_village_house(col, palette, 5, center_surface, 5);
    }
    if (chunk_h >> 8).is_multiple_of(48) {
        let dy = 12 + ((chunk_h >> 16) % 24) as i32;
        generate_dungeon_room(col, palette, 4, dy, 4, chunk_h);
    }
}

fn generate_nether(col: &mut ChunkColumn, seed: i64, cx: i32, cz: i32, palette: &WorldgenPalette) {
    let chunk_h = splitmix64(
        (seed as u64)
            ^ ((cx as u64).wrapping_mul(0x9E37_79B9))
            ^ ((cz as u64).wrapping_mul(0xBF58_476D)),
    );
    for lz in 0..16usize {
        for lx in 0..16usize {
            let wx = cx * 16 + lx as i32;
            let wz = cz * 16 + lz as i32;
            let floor_h = 30
                + (value_noise_2d(seed ^ 0x4E45_5448, f64::from(wx), f64::from(wz), 24.0) * 14.0)
                    .round() as i32;
            col.set_block(lx, 0, lz, palette.bedrock);
            for y in 1..=floor_h {
                let h = splitmix64(
                    (seed as u64)
                        ^ ((wx as u64).wrapping_mul(73856093))
                        ^ ((y as u64).wrapping_mul(19349663))
                        ^ ((wz as u64).wrapping_mul(83492791)),
                );
                let block = if (h & 0x3F) == 1 {
                    palette.nether_quartz_ore
                } else if (h & 0x7F) == 2 {
                    palette.nether_gold_ore
                } else {
                    palette.netherrack
                };
                col.set_block(lx, y, lz, block);
            }
            for y in 118..127 {
                col.set_block(lx, y, lz, palette.netherrack);
            }
            if ((wx + wz) & 7) == 0 && (chunk_h & 3) == 0 {
                col.set_block(lx, 117, lz, palette.glowstone);
                col.set_block(lx, 116, lz, palette.glowstone);
            }
            col.set_block(lx, 127, lz, palette.bedrock);
        }
    }
    // Nether fortress bridge segment
    if chunk_h.is_multiple_of(48) {
        for lx in 0..16usize {
            for lz in 5..=10usize {
                let is_rail = lz == 5 || lz == 10;
                col.set_block(lx, 56, lz, palette.nether_bricks);
                if is_rail && lx.is_multiple_of(2) {
                    col.set_block(lx, 57, lz, palette.nether_bricks);
                }
            }
        }
        for y in 30..56 {
            col.set_block(4, y, 5, palette.nether_bricks);
            col.set_block(4, y, 10, palette.nether_bricks);
            col.set_block(11, y, 5, palette.nether_bricks);
            col.set_block(11, y, 10, palette.nether_bricks);
        }
    }
}

fn generate_end(col: &mut ChunkColumn, seed: i64, cx: i32, cz: i32, palette: &WorldgenPalette) {
    for lz in 0..16usize {
        for lx in 0..16usize {
            let wx = f64::from(cx * 16 + lx as i32);
            let wz = f64::from(cz * 16 + lz as i32);
            let dist = (wx * wx + wz * wz).sqrt();
            let island_noise = value_noise_2d(seed ^ 0x454E_4400, wx, wz, 32.0);
            if dist < 48.0 || island_noise > 0.62 {
                let top = 56 + ((island_noise * 6.0).round() as i32);
                for y in 46..=top {
                    col.set_block(lx, y, lz, palette.end_stone);
                }
            }
            // Obsidian pillars arranged in a ring around the main island
            for idx in 0..10 {
                let angle = f64::from(idx) * (std::f64::consts::TAU / 10.0);
                let px = (angle.cos() * 34.0).round();
                let pz = (angle.sin() * 34.0).round();
                let dx = wx - px;
                let dz = wz - pz;
                let radius = 1.5 + f64::from(idx % 3) * 0.6;
                if dx * dx + dz * dz <= radius * radius {
                    let top_y = 76 + (idx * 3);
                    for y in 52..=top_y {
                        col.set_block(lx, y, lz, palette.obsidian);
                    }
                }
            }
        }
    }
}

const _: usize = SECTION_HEIGHT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worldgen_palette_rejects_incomplete_provider_and_generates_with_synthetic_palette() {
        let incomplete =
            rad_core::registry_provider::SyntheticFixtureRegistryProvider::minimal_valid();
        assert!(WorldgenPalette::from_provider(&incomplete).is_err());

        let palette = WorldgenPalette {
            air: 0,
            bedrock: 1,
            stone: 2,
            deepslate: 2,
            dirt: 3,
            grass_block: 4,
            sand: 5,
            sandstone: 5,
            water: 6,
            lava: 0,
            coal_ore: 7,
            iron_ore: 8,
            copper_ore: 8,
            gold_ore: 8,
            redstone_ore: 7,
            lapis_ore: 7,
            diamond_ore: 9,
            deepslate_diamond_ore: 9,
            oak_log: 10,
            oak_leaves: 11,
            birch_log: 10,
            birch_leaves: 11,
            spruce_log: 10,
            spruce_leaves: 11,
            oak_planks: 10,
            cobblestone: 2,
            mossy_cobblestone: 2,
            obsidian: 2,
            crying_obsidian: 2,
            gold_block: 2,
            chest: 10,
            spawner: 2,
            short_grass: 0,
            dandelion: 0,
            poppy: 0,
            cactus: 0,
            tnt: 2,
            orange_terracotta: 2,
            blue_terracotta: 2,
            crafting_table: 10,
            furnace: 2,
            glass: 0,
            nether_bricks: 12,
            nether_quartz_ore: 12,
            nether_gold_ore: 12,
            glowstone: 12,
            netherrack: 12,
            end_stone: 13,
        };
        let nether = generate_seeded_chunk_with(42, DimensionId::Nether, 0, 0, &palette);
        assert_eq!(nether.get_block(0, 0, 0), palette.bedrock);
        assert_eq!(nether.get_block(0, 127, 0), palette.bedrock);
        let end = generate_seeded_chunk_with(42, DimensionId::End, 0, 0, &palette);
        assert_eq!(end.get_block(0, 50, 0), palette.end_stone);
        assert_eq!(end.get_block(0, 0, 0), palette.air);

        let overworld = generate_seeded_chunk_with(42, DimensionId::Overworld, 0, 0, &palette);
        assert_eq!(overworld.get_block(0, MIN_WORLD_Y, 0), palette.bedrock);
    }
}
