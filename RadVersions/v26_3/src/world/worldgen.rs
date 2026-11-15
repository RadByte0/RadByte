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

/// Resolved block-state palette for world generation, decoupled from compile-time `mc777_data`.
#[derive(Debug, Clone, Copy)]
pub struct WorldgenPalette {
    /// Air state ID.
    pub air: u16,
    /// Bedrock state ID.
    pub bedrock: u16,
    /// Stone state ID.
    pub stone: u16,
    /// Dirt state ID.
    pub dirt: u16,
    /// Grass block state ID.
    pub grass_block: u16,
    /// Sand state ID.
    pub sand: u16,
    /// Water source state ID.
    pub water: u16,
    /// Coal ore state ID.
    pub coal_ore: u16,
    /// Iron ore state ID.
    pub iron_ore: u16,
    /// Diamond ore state ID.
    pub diamond_ore: u16,
    /// Oak log state ID.
    pub oak_log: u16,
    /// Oak leaves state ID.
    pub oak_leaves: u16,
    /// Netherrack state ID.
    pub netherrack: u16,
    /// End stone state ID.
    pub end_stone: u16,
}

impl WorldgenPalette {
    /// Resolves all worldgen block states from `provider`, failing loudly if any block is missing.
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
        Ok(Self {
            air: req("minecraft:air")?,
            bedrock: req("minecraft:bedrock")?,
            stone: req("minecraft:stone")?,
            dirt: req("minecraft:dirt")?,
            grass_block: req("minecraft:grass_block")?,
            sand: req("minecraft:sand")?,
            water: req("minecraft:water")?,
            coal_ore: req("minecraft:coal_ore")?,
            iron_ore: req("minecraft:iron_ore")?,
            diamond_ore: req("minecraft:diamond_ore")?,
            oak_log: req("minecraft:oak_log")?,
            oak_leaves: req("minecraft:oak_leaves")?,
            netherrack: req("minecraft:netherrack")?,
            end_stone: req("minecraft:end_stone")?,
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

fn generate_overworld(
    col: &mut ChunkColumn,
    seed: i64,
    cx: i32,
    cz: i32,
    palette: &WorldgenPalette,
) {
    const SEA_LEVEL: i32 = 62;

    for lz in 0..16usize {
        for lx in 0..16usize {
            let wx = cx * 16 + lx as i32;
            let wz = cz * 16 + lz as i32;
            let n1 = value_noise_2d(seed, f64::from(wx), f64::from(wz), 48.0);
            let n2 = value_noise_2d(seed ^ 0x1234_5678, f64::from(wx), f64::from(wz), 16.0);
            let surface_y = 57 + ((n1 * 12.0 + n2 * 5.0).round() as i32);

            col.set_block(lx, MIN_WORLD_Y, lz, palette.bedrock);
            for y in (MIN_WORLD_Y + 1)..=(surface_y - 4) {
                // Deterministic cave / ore pocket check
                let cave_h = splitmix64(
                    (seed as u64)
                        ^ ((wx as u64).wrapping_mul(73856093))
                        ^ ((y as u64).wrapping_mul(19349663))
                        ^ ((wz as u64).wrapping_mul(83492791)),
                );
                let block = if (10..=40).contains(&y) && (cave_h & 0x3F) == 0 {
                    palette.air
                } else if y <= 16 && (cave_h & 0xFF) == 1 {
                    palette.diamond_ore
                } else if y < 48 && (cave_h & 0x3F) == 2 {
                    palette.iron_ore
                } else if y < 56 && (cave_h & 0x1F) == 3 {
                    palette.coal_ore
                } else {
                    palette.stone
                };
                col.set_block(lx, y, lz, block);
            }
            for y in (surface_y - 3).max(MIN_WORLD_Y + 1)..surface_y {
                let sub = if surface_y <= SEA_LEVEL + 1 {
                    palette.sand
                } else {
                    palette.dirt
                };
                col.set_block(lx, y, lz, sub);
            }
            let top = if surface_y <= SEA_LEVEL + 1 {
                palette.sand
            } else {
                palette.grass_block
            };
            col.set_block(lx, surface_y, lz, top);

            if surface_y < SEA_LEVEL {
                for y in (surface_y + 1)..=SEA_LEVEL {
                    col.set_block(lx, y, lz, palette.water);
                }
            } else if (2..14).contains(&lx) && (2..14).contains(&lz) {
                let tree_h = splitmix64(
                    (seed as u64) ^ ((wx as u64).wrapping_mul(31)) ^ (wz as u64).wrapping_mul(97),
                );
                if tree_h.is_multiple_of(89) {
                    for ty in 1..=4 {
                        col.set_block(lx, surface_y + ty, lz, palette.oak_log);
                    }
                    for dx in -1isize..=1 {
                        for dz in -1isize..=1 {
                            let tx = (lx as isize + dx) as usize;
                            let tz = (lz as isize + dz) as usize;
                            if !(dx == 0 && dz == 0) {
                                col.set_block(tx, surface_y + 3, tz, palette.oak_leaves);
                                col.set_block(tx, surface_y + 4, tz, palette.oak_leaves);
                            }
                        }
                    }
                    col.set_block(lx, surface_y + 5, lz, palette.oak_leaves);
                }
            }
        }
    }
}

fn generate_nether(col: &mut ChunkColumn, seed: i64, cx: i32, cz: i32, palette: &WorldgenPalette) {
    for lz in 0..16usize {
        for lx in 0..16usize {
            let wx = cx * 16 + lx as i32;
            let wz = cz * 16 + lz as i32;
            let floor_h = 30
                + (value_noise_2d(seed ^ 0x4E45_5448, f64::from(wx), f64::from(wz), 24.0) * 14.0)
                    .round() as i32;
            col.set_block(lx, 0, lz, palette.bedrock);
            for y in 1..=floor_h {
                col.set_block(lx, y, lz, palette.netherrack);
            }
            for y in 118..127 {
                col.set_block(lx, y, lz, palette.netherrack);
            }
            col.set_block(lx, 127, lz, palette.bedrock);
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
            dirt: 3,
            grass_block: 4,
            sand: 5,
            water: 6,
            coal_ore: 7,
            iron_ore: 8,
            diamond_ore: 9,
            oak_log: 10,
            oak_leaves: 11,
            netherrack: 12,
            end_stone: 13,
        };
        let nether = generate_seeded_chunk_with(42, DimensionId::Nether, 0, 0, &palette);
        assert_eq!(nether.get_block(0, 0, 0), palette.bedrock);
        assert_eq!(nether.get_block(0, 127, 0), palette.bedrock);
        let end = generate_seeded_chunk_with(42, DimensionId::End, 0, 0, &palette);
        assert_eq!(end.get_block(0, 50, 0), palette.end_stone);
        assert_eq!(end.get_block(0, 0, 0), palette.air);
    }
}
