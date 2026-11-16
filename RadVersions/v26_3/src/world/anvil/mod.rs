//! Chunk storage, compact RAD1 & vanilla Anvil .mca region serialization, and protocol 777 chunk encoding.

mod region;

pub use region::{
    AnvilRegion, LightTables, StorageFormat, VanillaImportChunk, detect_storage_format,
};
use region::{get_nibble, set_nibble, vanilla_block_entry_to_state_with};

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use bytes::{BufMut, BytesMut};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use rad_core::packet::play::{HeightmapEntry, LevelChunkWithLight};
use rad_core::types::{Encode, VarInt};

use crate::world::OVERWORLD_SECTIONS;

/// Minimum world Y coordinate in standard Overworld.
pub const MIN_WORLD_Y: i32 = -64;
/// Maximum exclusive world Y coordinate in standard Overworld.
pub const MAX_WORLD_Y: i32 = 320;
const SECTOR_BYTES: usize = 4096;
const HEADER_BYTES: usize = 8192;
const MAX_CHUNK_STORAGE_RAW_BYTES: usize = 1024 * 1024;
const DIRECT_PALETTE_BPE: u8 = 15;

/// Error returned when reading or decoding an `.mca` chunk payload fails validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnvilDecodeError {
    /// Non-empty chunk slot had corrupt zlib, exceeded limits, or was not `RAD1` format.
    ForeignOrCorrupt,
}

/// Memory-efficient 16x16x16 chunk section.
#[derive(Debug, Clone)]
pub enum ChunkSection {
    /// All 4,096 blocks share the same block state ID.
    Uniform(u16),
    /// Per-block state IDs indexed by `(y << 8) | (z << 4) | x`.
    Blocks(Box<[u16; 4096]>),
}

impl ChunkSection {
    /// Gets the block state ID at local coordinates `(0..16, 0..16, 0..16)`.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u16 {
        let idx = (y << 8) | (z << 4) | x;
        match self {
            Self::Uniform(id) => *id,
            Self::Blocks(arr) => arr[idx],
        }
    }

    /// Sets the block state ID at local coordinates `(0..16, 0..16, 0..16)`.
    pub fn set(&mut self, x: usize, y: usize, z: usize, state_id: u16) {
        let idx = (y << 8) | (z << 4) | x;
        match self {
            Self::Uniform(cur) if *cur == state_id => {}
            Self::Uniform(cur) => {
                let mut arr = Box::new([*cur; 4096]);
                arr[idx] = state_id;
                *self = Self::Blocks(arr);
            }
            Self::Blocks(arr) => {
                arr[idx] = state_id;
            }
        }
    }

    fn non_air_count(&self) -> i16 {
        match self {
            Self::Uniform(0) => 0,
            Self::Uniform(_) => 4096,
            Self::Blocks(arr) => arr.iter().filter(|&&id| id != 0).count() as i16,
        }
    }

    fn encode_paletted(&self, out: &mut BytesMut) {
        out.put_i16(self.non_air_count());
        out.put_i16(0); // fluid_count (777+)
        match self {
            Self::Uniform(id) => {
                out.put_u8(0); // single-valued palette (no long array follows in 777)
                let _ = VarInt(i32::from(*id)).encode(out);
            }
            Self::Blocks(arr) => {
                let mut palette: Vec<u16> = Vec::new();
                let mut map: HashMap<u16, usize> = HashMap::new();
                for &id in arr.iter() {
                    map.entry(id).or_insert_with(|| {
                        let idx = palette.len();
                        palette.push(id);
                        idx
                    });
                }
                if palette.len() == 1 {
                    out.put_u8(0);
                    let _ = VarInt(i32::from(palette[0])).encode(out);
                } else if palette.len() > 256 {
                    // Direct (global) palette for >256 distinct block states in a section.
                    let bpe = DIRECT_PALETTE_BPE;
                    out.put_u8(bpe);
                    let entries_per_long = 64 / usize::from(bpe);
                    let long_count = 4096usize.div_ceil(entries_per_long);
                    for l in 0..long_count {
                        let mut word: u64 = 0;
                        for j in 0..entries_per_long {
                            let block_idx = l * entries_per_long + j;
                            if block_idx < 4096 {
                                word |= u64::from(arr[block_idx]) << (j * usize::from(bpe));
                            }
                        }
                        out.put_u64(word);
                    }
                } else {
                    let bpe = ((palette.len() - 1).ilog2() as u8 + 1).clamp(4, 8);
                    out.put_u8(bpe);
                    let _ = VarInt(palette.len() as i32).encode(out);
                    for id in palette {
                        let _ = VarInt(i32::from(id)).encode(out);
                    }
                    let entries_per_long = 64 / usize::from(bpe);
                    let long_count = 4096usize.div_ceil(entries_per_long);
                    for l in 0..long_count {
                        let mut word: u64 = 0;
                        for j in 0..entries_per_long {
                            let block_idx = l * entries_per_long + j;
                            if block_idx < 4096 {
                                let p_idx = map[&arr[block_idx]] as u64;
                                word |= p_idx << (j * usize::from(bpe));
                            }
                        }
                        out.put_u64(word);
                    }
                }
            }
        }

        // Biomes PalettedContainer (single-valued plains = 0, no long array follows in 777)
        out.put_u8(0);
        let _ = VarInt(0).encode(out);
    }
}

/// Preserved Vanilla Anvil `SkyLight` and `BlockLight` nibbles for all 26 light sections (`-1..=24`).
#[derive(Debug, Clone)]
pub struct VanillaChunkLight {
    /// 26 sky light nibble arrays (`2048` bytes each).
    pub sky_sections: Vec<[u8; 2048]>,
    /// 26 block light nibble arrays (`2048` bytes each).
    pub block_sections: Vec<[u8; 2048]>,
}

/// Full 16x384x16 chunk column.
#[derive(Debug, Clone)]
pub struct ChunkColumn {
    /// 24 sections from `Y = -64` to `Y = 320`.
    pub sections: Vec<ChunkSection>,
    /// Vanilla Anvil `block_entities` NBT compounds for this chunk.
    pub block_entities: Vec<std::collections::BTreeMap<String, rad_core::nbt::NbtTag>>,
    /// Vanilla Anvil `block_ticks` NBT compounds for this chunk.
    pub block_ticks: Vec<std::collections::BTreeMap<String, rad_core::nbt::NbtTag>>,
    /// Preserved Vanilla light nibbles when loaded from `.mca` and unmodified.
    pub vanilla_light: Option<Box<VanillaChunkLight>>,
    /// Whether this chunk has unsaved modifications.
    pub dirty: bool,
    /// Monotonic mutation revision for race-free async saves.
    pub revision: u64,
}

impl ChunkColumn {
    /// Creates a standard flat chunk (`Y = -64..48` stone, `Y = 48..64` grass/surface, `64..` air).
    #[must_use]
    pub fn flat() -> Self {
        let mut sections = Vec::with_capacity(OVERWORLD_SECTIONS);
        for idx in 0..OVERWORLD_SECTIONS {
            let state = if idx == 7 {
                9
            } else if idx < 7 {
                1
            } else {
                0
            };
            sections.push(ChunkSection::Uniform(state));
        }
        Self {
            sections,
            block_entities: Vec::new(),
            block_ticks: Vec::new(),
            vanilla_light: None,
            dirty: false,
            revision: 0,
        }
    }

    /// Creates a [`ChunkColumn`] from pre-built sections.
    #[must_use]
    pub fn from_sections(sections: Vec<ChunkSection>) -> Self {
        Self {
            sections,
            block_entities: Vec::new(),
            block_ticks: Vec::new(),
            vanilla_light: None,
            dirty: false,
            revision: 0,
        }
    }

    /// Clears the dirty flag on this chunk column.
    pub fn clear_dirty(&mut self) {
        self.dirty = false;
    }

    /// Gets block state ID at chunk-local `(x, y, z)` (`x, z in 0..16`, `y in -64..320`).
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn get_block(&self, x: usize, y: i32, z: usize) -> u16 {
        if !(MIN_WORLD_Y..MAX_WORLD_Y).contains(&y) || x >= 16 || z >= 16 {
            return 0;
        }
        let sec_idx = ((y - MIN_WORLD_Y) / 16) as usize;
        let local_y = ((y - MIN_WORLD_Y) % 16) as usize;
        self.sections[sec_idx].get(x, local_y, z)
    }

    /// Sets block state ID at chunk-local `(x, y, z)` and marks the chunk dirty.
    pub fn set_block(&mut self, x: usize, y: i32, z: usize, state_id: u16) -> bool {
        if !(MIN_WORLD_Y..MAX_WORLD_Y).contains(&y) || x >= 16 || z >= 16 {
            return false;
        }
        let sec_idx = ((y - MIN_WORLD_Y) / 16) as usize;
        let local_y = ((y - MIN_WORLD_Y) % 16) as usize;
        self.sections[sec_idx].set(x, local_y, z, state_id);
        self.vanilla_light = None;
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    /// Serializes this chunk column into a [`LevelChunkWithLight`] packet using `provider`.
    #[must_use]
    pub fn to_packet_with(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> LevelChunkWithLight {
        let tables = LightTables::from_provider(provider);
        self.to_packet_with_tables(chunk_x, chunk_z, &tables)
    }

    /// Serializes this chunk column into a [`LevelChunkWithLight`] packet using precomputed `tables`.
    #[must_use]
    pub fn to_packet_with_tables(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        tables: &LightTables,
    ) -> LevelChunkWithLight {
        self.to_packet_with_tables_and_neighbors(chunk_x, chunk_z, tables, [None, None, None, None])
    }

    /// Serializes this chunk column using precomputed `tables` and optional `[west, east, north, south]` neighbors for cross-chunk lighting.
    #[must_use]
    pub fn to_packet_with_tables_and_neighbors(
        &self,
        chunk_x: i32,
        chunk_z: i32,
        tables: &LightTables,
        neighbors: [Option<&Self>; 4],
    ) -> LevelChunkWithLight {
        let mut data = BytesMut::with_capacity(1024);
        for sec in &self.sections {
            sec.encode_paletted(&mut data);
        }

        let total_light_sections = OVERWORLD_SECTIONS + 2; // 26 sections: below(-1), 0..23, above(24)
        if !self.dirty
            && let Some(vl) = &self.vanilla_light
        {
            let mut sky_light_mask: u32 = 0;
            let mut empty_sky_light_mask: u32 = 0;
            let mut sky_lights = Vec::new();
            for (idx, arr) in vl.sky_sections.iter().enumerate() {
                if arr.iter().all(|&b| b == 0) {
                    empty_sky_light_mask |= 1u32 << idx;
                } else {
                    sky_light_mask |= 1u32 << idx;
                    sky_lights.push(arr.to_vec());
                }
            }
            let mut block_light_mask: u32 = 0;
            let mut empty_block_light_mask: u32 = 0;
            let mut block_lights = Vec::new();
            for (idx, arr) in vl.block_sections.iter().enumerate() {
                if arr.iter().all(|&b| b == 0) {
                    empty_block_light_mask |= 1u32 << idx;
                } else {
                    block_light_mask |= 1u32 << idx;
                    block_lights.push(arr.to_vec());
                }
            }
            let motion_blocking = self.packed_motion_blocking_heightmap();
            return LevelChunkWithLight {
                chunk_x,
                chunk_z,
                heightmaps: vec![HeightmapEntry {
                    kind: VarInt(4),
                    data: motion_blocking,
                }],
                data: data.to_vec(),
                sky_light_mask: sky_light_mask.to_le_bytes().to_vec(),
                block_light_mask: block_light_mask.to_le_bytes().to_vec(),
                empty_sky_light_mask: empty_sky_light_mask.to_le_bytes().to_vec(),
                empty_block_light_mask: empty_block_light_mask.to_le_bytes().to_vec(),
                sky_lights,
                block_lights,
            };
        }
        let mut sky_sections: Vec<[u8; 2048]> = vec![[0u8; 2048]; total_light_sections];
        sky_sections[total_light_sections - 1].fill(0xFF); // above-world sky is always 15

        let mut min_active_sec = OVERWORLD_SECTIONS;

        // 1. Column-by-column vertical skylight pass from top down.
        for z in 0..16usize {
            for x in 0..16usize {
                let mut light: u8 = 15;
                for sec_idx in (0..OVERWORLD_SECTIONS).rev() {
                    if light == 0 {
                        break;
                    }
                    let light_sec_idx = sec_idx + 1;
                    if let ChunkSection::Uniform(state) = self.sections[sec_idx] {
                        let opacity = tables.opacity(state);
                        if opacity == 0 {
                            for ly in 0..16usize {
                                set_nibble(&mut sky_sections[light_sec_idx], x, ly, z, light);
                            }
                            continue;
                        } else if opacity >= 15 {
                            min_active_sec = min_active_sec.min(sec_idx);
                            break;
                        }
                    }
                    min_active_sec = min_active_sec.min(sec_idx);
                    for ly in (0..16usize).rev() {
                        let state = self.sections[sec_idx].get(x, ly, z);
                        let opacity = tables.opacity(state);
                        if opacity >= 15 {
                            light = 0;
                            break;
                        }
                        if opacity > 0 {
                            light = light.saturating_sub(opacity);
                        }
                        set_nibble(&mut sky_sections[light_sec_idx], x, ly, z, light);
                    }
                }
            }
        }

        // 2. 3D horizontal/vertical skylight flood-fill around canopies, overhangs, and cave mouths.
        if min_active_sec < OVERWORLD_SECTIONS {
            let mut sky_queue = std::collections::VecDeque::new();
            let start_sec = min_active_sec.saturating_sub(1);
            for sec_idx in start_sec..OVERWORLD_SECTIONS {
                let light_sec_idx = sec_idx + 1;
                if matches!(self.sections[sec_idx], ChunkSection::Uniform(0))
                    && sec_idx > min_active_sec + 1
                {
                    continue;
                }
                for ly in 0..16usize {
                    let y = MIN_WORLD_Y + (sec_idx as i32) * 16 + ly as i32;
                    for z in 0..16usize {
                        for x in 0..16usize {
                            let cur = get_nibble(&sky_sections[light_sec_idx], x, ly, z);
                            if cur <= 1 {
                                continue;
                            }
                            let mut needs_spread = false;
                            for (dx, dy, dz) in [
                                (1i32, 0i32, 0i32),
                                (-1, 0, 0),
                                (0, 0, 1),
                                (0, 0, -1),
                                (0, -1, 0),
                            ] {
                                let nx = x as i32 + dx;
                                let ny = y + dy;
                                let nz = z as i32 + dz;
                                if !(0..16).contains(&nx)
                                    || !(0..16).contains(&nz)
                                    || !(MIN_WORLD_Y..MAX_WORLD_Y).contains(&ny)
                                {
                                    continue;
                                }
                                let nsec = ((ny - MIN_WORLD_Y) / 16) as usize;
                                let nly = ((ny - MIN_WORLD_Y) % 16) as usize;
                                let nopacity = tables.opacity(self.sections[nsec].get(
                                    nx as usize,
                                    nly,
                                    nz as usize,
                                ));
                                if nopacity < 15 {
                                    let ncur = get_nibble(
                                        &sky_sections[nsec + 1],
                                        nx as usize,
                                        nly,
                                        nz as usize,
                                    );
                                    if cur.saturating_sub(nopacity.max(1)) > ncur {
                                        needs_spread = true;
                                        break;
                                    }
                                }
                            }
                            if needs_spread {
                                sky_queue.push_back((x as i32, y, z as i32, cur));
                            }
                        }
                    }
                }
            }

            // Seed cross-chunk skylight from adjacent chunks' Vanilla light borders when available.
            let border_specs: [(usize, usize, usize, bool); 4] = [
                (0, 15, 0, true),  // west neighbor (cx-1): its x=15 -> our x=0
                (1, 0, 15, true),  // east neighbor (cx+1): its x=0 -> our x=15
                (2, 15, 0, false), // north neighbor (cz-1): its z=15 -> our z=0
                (3, 0, 15, false), // south neighbor (cz+1): its z=0 -> our z=15
            ];
            for (n_idx, n_coord, our_coord, along_z) in border_specs {
                let Some(nb) = neighbors[n_idx] else {
                    continue;
                };
                let Some(vl) = nb.vanilla_light.as_deref() else {
                    continue;
                };
                for sec_idx in start_sec..OVERWORLD_SECTIONS {
                    for ly in 0..16usize {
                        let y = MIN_WORLD_Y + (sec_idx as i32) * 16 + ly as i32;
                        for t in 0..16usize {
                            let (nx, nz, ox, oz) = if along_z {
                                (n_coord, t, our_coord, t)
                            } else {
                                (t, n_coord, t, our_coord)
                            };
                            let nlight = get_nibble(&vl.sky_sections[sec_idx + 1], nx, ly, nz);
                            if nlight <= 1 {
                                continue;
                            }
                            let opacity = tables.opacity(self.sections[sec_idx].get(ox, ly, oz));
                            if opacity >= 15 {
                                continue;
                            }
                            let next_level = nlight.saturating_sub(opacity.max(1));
                            let cur = get_nibble(&sky_sections[sec_idx + 1], ox, ly, oz);
                            if next_level > cur {
                                set_nibble(&mut sky_sections[sec_idx + 1], ox, ly, oz, next_level);
                                sky_queue.push_back((ox as i32, y, oz as i32, next_level));
                            }
                        }
                    }
                }
            }

            while let Some((x, y, z, level)) = sky_queue.pop_front() {
                if level <= 1 {
                    continue;
                }
                for (dx, dy, dz) in [
                    (1, 0, 0),
                    (-1, 0, 0),
                    (0, 1, 0),
                    (0, -1, 0),
                    (0, 0, 1),
                    (0, 0, -1),
                ] {
                    let nx = x + dx;
                    let ny = y + dy;
                    let nz = z + dz;
                    if !(0..16).contains(&nx)
                        || !(0..16).contains(&nz)
                        || !(MIN_WORLD_Y..MAX_WORLD_Y).contains(&ny)
                    {
                        continue;
                    }
                    let nsec = ((ny - MIN_WORLD_Y) / 16) as usize;
                    let nly = ((ny - MIN_WORLD_Y) % 16) as usize;
                    let nstate = self.sections[nsec].get(nx as usize, nly, nz as usize);
                    let opacity = tables.opacity(nstate);
                    if opacity >= 15 {
                        continue;
                    }
                    let next_level = level.saturating_sub(opacity.max(1));
                    if next_level == 0 {
                        continue;
                    }
                    let cur = get_nibble(&sky_sections[nsec + 1], nx as usize, nly, nz as usize);
                    if next_level > cur {
                        set_nibble(
                            &mut sky_sections[nsec + 1],
                            nx as usize,
                            nly,
                            nz as usize,
                            next_level,
                        );
                        sky_queue.push_back((nx, ny, nz, next_level));
                    }
                }
            }
        }

        let mut sky_light_mask: u32 = 0;
        let mut empty_sky_light_mask: u32 = 0;
        let mut sky_lights = Vec::new();
        for (idx, arr) in sky_sections.into_iter().enumerate() {
            if arr.iter().all(|&b| b == 0) {
                empty_sky_light_mask |= 1u32 << idx;
            } else {
                sky_light_mask |= 1u32 << idx;
                sky_lights.push(arr.to_vec());
            }
        }

        // Dynamic block light BFS when any light emitter is present in this chunk.
        let mut block_sections: Vec<[u8; 2048]> = vec![[0u8; 2048]; total_light_sections];
        let mut queue = std::collections::VecDeque::new();
        for (sec_idx, sec) in self.sections.iter().enumerate() {
            let has_emitter = match sec {
                ChunkSection::Uniform(s) => tables.emission(*s) > 0,
                ChunkSection::Blocks(_) => true,
            };
            if !has_emitter {
                continue;
            }
            for ly in 0..16usize {
                for z in 0..16usize {
                    for x in 0..16usize {
                        let emit = tables.emission(sec.get(x, ly, z));
                        if emit > 0 {
                            let y = MIN_WORLD_Y + (sec_idx as i32) * 16 + ly as i32;
                            set_nibble(&mut block_sections[sec_idx + 1], x, ly, z, emit);
                            queue.push_back((x as i32, y, z as i32, emit));
                        }
                    }
                }
            }
        }

        // Seed cross-chunk block light from adjacent chunks' Vanilla light borders when available.
        let border_specs: [(usize, usize, usize, bool); 4] = [
            (0, 15, 0, true),
            (1, 0, 15, true),
            (2, 15, 0, false),
            (3, 0, 15, false),
        ];
        for (n_idx, n_coord, our_coord, along_z) in border_specs {
            let Some(nb) = neighbors[n_idx] else {
                continue;
            };
            let Some(vl) = nb.vanilla_light.as_deref() else {
                continue;
            };
            for sec_idx in 0..OVERWORLD_SECTIONS {
                if vl.block_sections[sec_idx + 1].iter().all(|&b| b == 0) {
                    continue;
                }
                for ly in 0..16usize {
                    let y = MIN_WORLD_Y + (sec_idx as i32) * 16 + ly as i32;
                    for t in 0..16usize {
                        let (nx, nz, ox, oz) = if along_z {
                            (n_coord, t, our_coord, t)
                        } else {
                            (t, n_coord, t, our_coord)
                        };
                        let nlight = get_nibble(&vl.block_sections[sec_idx + 1], nx, ly, nz);
                        if nlight <= 1 {
                            continue;
                        }
                        let opacity = tables.opacity(self.sections[sec_idx].get(ox, ly, oz));
                        if opacity >= 15 {
                            continue;
                        }
                        let next_level = nlight.saturating_sub(1 + opacity);
                        let cur = get_nibble(&block_sections[sec_idx + 1], ox, ly, oz);
                        if next_level > cur {
                            set_nibble(&mut block_sections[sec_idx + 1], ox, ly, oz, next_level);
                            queue.push_back((ox as i32, y, oz as i32, next_level));
                        }
                    }
                }
            }
        }

        while let Some((x, y, z, level)) = queue.pop_front() {
            if level <= 1 {
                continue;
            }
            for (dx, dy, dz) in [
                (1, 0, 0),
                (-1, 0, 0),
                (0, 1, 0),
                (0, -1, 0),
                (0, 0, 1),
                (0, 0, -1),
            ] {
                let nx = x + dx;
                let ny = y + dy;
                let nz = z + dz;
                if !(0..16).contains(&nx)
                    || !(0..16).contains(&nz)
                    || !(MIN_WORLD_Y..MAX_WORLD_Y).contains(&ny)
                {
                    continue;
                }
                let nsec = ((ny - MIN_WORLD_Y) / 16) as usize;
                let nly = ((ny - MIN_WORLD_Y) % 16) as usize;
                let nstate = self.sections[nsec].get(nx as usize, nly, nz as usize);
                let opacity = tables.opacity(nstate);
                if opacity >= 15 {
                    continue;
                }
                let next_level = level.saturating_sub(1 + opacity);
                if next_level == 0 {
                    continue;
                }
                let cur = get_nibble(&block_sections[nsec + 1], nx as usize, nly, nz as usize);
                if next_level > cur {
                    set_nibble(
                        &mut block_sections[nsec + 1],
                        nx as usize,
                        nly,
                        nz as usize,
                        next_level,
                    );
                    queue.push_back((nx, ny, nz, next_level));
                }
            }
        }

        let mut block_light_mask: u32 = 0;
        let mut empty_block_light_mask: u32 = 0;
        let mut block_lights = Vec::new();
        for (idx, arr) in block_sections.into_iter().enumerate() {
            if arr.iter().all(|&b| b == 0) {
                empty_block_light_mask |= 1u32 << idx;
            } else {
                block_light_mask |= 1u32 << idx;
                block_lights.push(arr.to_vec());
            }
        }

        let motion_blocking = self.packed_motion_blocking_heightmap();
        let heightmaps = vec![HeightmapEntry {
            kind: VarInt(4), // MOTION_BLOCKING
            data: motion_blocking,
        }];

        LevelChunkWithLight {
            chunk_x,
            chunk_z,
            heightmaps,
            data: data.to_vec(),
            sky_light_mask: sky_light_mask.to_le_bytes().to_vec(),
            block_light_mask: block_light_mask.to_le_bytes().to_vec(),
            empty_sky_light_mask: empty_sky_light_mask.to_le_bytes().to_vec(),
            empty_block_light_mask: empty_block_light_mask.to_le_bytes().to_vec(),
            sky_lights,
            block_lights,
        }
    }

    fn packed_motion_blocking_heightmap(&self) -> Vec<i64> {
        // 9 bits per column (values 0..384), 7 columns per i64 -> 37 i64s for 256 columns.
        let mut longs = vec![0i64; 37];
        for z in 0..16 {
            for x in 0..16 {
                let mut height_val: u64 = 0;
                for y in (MIN_WORLD_Y..MAX_WORLD_Y).rev() {
                    if self.get_block(x, y, z) != 0 {
                        height_val = (y - MIN_WORLD_Y + 1) as u64;
                        break;
                    }
                }
                let col_idx = z * 16 + x;
                let long_idx = col_idx / 7;
                let bit_offset = (col_idx % 7) * 9;
                longs[long_idx] |= ((height_val & 0x1FF) as i64) << bit_offset;
            }
        }
        longs
    }

    /// Encodes modified block sections to compact zlib-compressed bytes for `.mca` storage.
    pub fn encode_storage_bytes(&self) -> std::io::Result<Vec<u8>> {
        let mut raw = BytesMut::with_capacity(4096);
        raw.put_u32(0x5241_4431); // "RAD1" section snapshot header
        for sec in &self.sections {
            match sec {
                ChunkSection::Uniform(id) => {
                    raw.put_u8(0);
                    raw.put_u16(*id);
                }
                ChunkSection::Blocks(arr) => {
                    raw.put_u8(1);
                    for &id in arr.iter() {
                        raw.put_u16(id);
                    }
                }
            }
        }
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&raw)?;
        enc.finish()
    }

    /// Decodes a chunk from zlib-compressed `.mca` payload bytes with bounded decompression using `provider`.
    pub fn decode_storage_bytes_checked_with(
        compressed: &[u8],
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> std::result::Result<Self, AnvilDecodeError> {
        let mut dec = ZlibDecoder::new(compressed).take(MAX_CHUNK_STORAGE_RAW_BYTES as u64 + 1);
        let mut raw = Vec::new();
        if dec.read_to_end(&mut raw).is_err() || raw.len() > MAX_CHUNK_STORAGE_RAW_BYTES {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        if raw.first() == Some(&0x0A) {
            return Self::decode_vanilla_nbt_chunk_with(&raw, provider);
        }
        if raw.len() < 4 || &raw[..4] != b"RAD1" {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        let mut cursor = 4usize;
        let mut sections = Vec::with_capacity(OVERWORLD_SECTIONS);
        for _ in 0..OVERWORLD_SECTIONS {
            let tag = *raw.get(cursor).ok_or(AnvilDecodeError::ForeignOrCorrupt)?;
            cursor += 1;
            if tag == 0 {
                let slice = raw
                    .get(cursor..cursor + 2)
                    .ok_or(AnvilDecodeError::ForeignOrCorrupt)?;
                let bytes: [u8; 2] = slice
                    .try_into()
                    .map_err(|_| AnvilDecodeError::ForeignOrCorrupt)?;
                cursor += 2;
                sections.push(ChunkSection::Uniform(u16::from_be_bytes(bytes)));
            } else if tag == 1 {
                let mut arr = Box::new([0u16; 4096]);
                for slot in arr.iter_mut() {
                    let slice = raw
                        .get(cursor..cursor + 2)
                        .ok_or(AnvilDecodeError::ForeignOrCorrupt)?;
                    let bytes: [u8; 2] = slice
                        .try_into()
                        .map_err(|_| AnvilDecodeError::ForeignOrCorrupt)?;
                    cursor += 2;
                    *slot = u16::from_be_bytes(bytes);
                }
                sections.push(ChunkSection::Blocks(arr));
            } else {
                return Err(AnvilDecodeError::ForeignOrCorrupt);
            }
        }
        Ok(Self {
            sections,
            block_entities: Vec::new(),
            block_ticks: Vec::new(),
            vanilla_light: None,
            dirty: false,
            revision: 0,
        })
    }

    fn decode_vanilla_nbt_chunk_with(
        raw: &[u8],
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> std::result::Result<Self, AnvilDecodeError> {
        Self::decode_vanilla_nbt_chunk_with_metadata_and_provider(raw, provider)
            .map(|imp| imp.column)
    }

    /// Decodes a vanilla Anvil NBT chunk using `provider` while preserving `DataVersion`, `block_entities`, and `biomes`.
    pub fn decode_vanilla_nbt_chunk_with_metadata_and_provider(
        raw: &[u8],
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> std::result::Result<VanillaImportChunk, AnvilDecodeError> {
        use rad_core::nbt::{NbtTag, decode_named_nbt};

        let mut cursor = raw;
        let Ok((_, NbtTag::Compound(root))) = decode_named_nbt(&mut cursor) else {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        };
        let Some(NbtTag::List(nbt_sections)) = root.get("sections") else {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        };
        let data_version = match root.get("DataVersion") {
            Some(NbtTag::Int(v)) => Some(*v),
            _ => None,
        };
        let block_entities = match root.get("block_entities") {
            Some(NbtTag::List(list)) => list
                .iter()
                .filter_map(|tag| match tag {
                    NbtTag::Compound(map) => Some(map.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let block_ticks = match root.get("block_ticks") {
            Some(NbtTag::List(list)) => list
                .iter()
                .filter_map(|tag| match tag {
                    NbtTag::Compound(map) => Some(map.clone()),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };

        let mut section_biomes = Vec::new();
        let mut sections = vec![ChunkSection::Uniform(0); OVERWORLD_SECTIONS];
        let total_light_sections = OVERWORLD_SECTIONS + 2;
        let mut vanilla_sky = vec![[0u8; 2048]; total_light_sections];
        vanilla_sky[total_light_sections - 1].fill(0xFF);
        let mut vanilla_block = vec![[0u8; 2048]; total_light_sections];
        let mut has_vanilla_light = false;
        for sec_tag in nbt_sections {
            let NbtTag::Compound(sec) = sec_tag else {
                continue;
            };
            let Some(NbtTag::Byte(y_idx)) = sec.get("Y") else {
                continue;
            };
            let sec_pos = i32::from(*y_idx) + 4;
            if !(0..OVERWORLD_SECTIONS as i32).contains(&sec_pos) {
                continue;
            }
            if let Some(NbtTag::ByteArray(bytes)) = sec.get("SkyLight")
                && bytes.len() == 2048
            {
                for (dst, &src) in vanilla_sky[sec_pos as usize + 1].iter_mut().zip(bytes) {
                    *dst = src as u8;
                }
                has_vanilla_light = true;
            }
            if let Some(NbtTag::ByteArray(bytes)) = sec.get("BlockLight")
                && bytes.len() == 2048
            {
                for (dst, &src) in vanilla_block[sec_pos as usize + 1].iter_mut().zip(bytes) {
                    *dst = src as u8;
                }
                has_vanilla_light = true;
            }
            if let Some(NbtTag::Compound(biomes)) = sec.get("biomes")
                && let Some(NbtTag::List(b_pal)) = biomes.get("palette")
            {
                let names: Vec<String> = b_pal
                    .iter()
                    .filter_map(|t| match t {
                        NbtTag::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                if !names.is_empty() {
                    section_biomes.push((*y_idx, names));
                }
            }
            let Some(NbtTag::Compound(bs)) = sec.get("block_states") else {
                continue;
            };
            let Some(NbtTag::List(palette_tags)) = bs.get("palette") else {
                continue;
            };
            if palette_tags.is_empty() {
                continue;
            }
            let mut palette = Vec::with_capacity(palette_tags.len());
            for entry in palette_tags {
                let state = match entry {
                    NbtTag::String(name) => vanilla_block_entry_to_state_with(name, None, provider)
                        .ok_or(AnvilDecodeError::ForeignOrCorrupt)?,
                    NbtTag::Compound(map) => {
                        let name = match map
                            .get("Name")
                            .or_else(|| map.get("id"))
                            .or_else(|| map.get(""))
                        {
                            Some(NbtTag::String(name)) => name,
                            _ => return Err(AnvilDecodeError::ForeignOrCorrupt),
                        };
                        let props = match map.get("Properties").or_else(|| map.get("properties")) {
                            Some(NbtTag::Compound(p)) => Some(p),
                            _ => None,
                        };
                        vanilla_block_entry_to_state_with(name, props, provider)
                            .ok_or(AnvilDecodeError::ForeignOrCorrupt)?
                    }
                    _ => return Err(AnvilDecodeError::ForeignOrCorrupt),
                };
                palette.push(state);
            }
            if palette.len() == 1 {
                sections[sec_pos as usize] = ChunkSection::Uniform(palette[0]);
                continue;
            }
            let Some(NbtTag::LongArray(longs)) = bs.get("data") else {
                return Err(AnvilDecodeError::ForeignOrCorrupt);
            };
            let bits = ((palette.len() - 1).ilog2() as usize + 1).max(4);
            let per_long = 64 / bits;
            let mask = (1u64 << bits) - 1;
            let mut arr = Box::new([0u16; 4096]);
            for (idx, slot) in arr.iter_mut().enumerate() {
                let long_idx = idx / per_long;
                let bit_off = (idx % per_long) * bits;
                let word = longs
                    .get(long_idx)
                    .copied()
                    .ok_or(AnvilDecodeError::ForeignOrCorrupt)? as u64;
                let pal_idx = ((word >> bit_off) & mask) as usize;
                *slot = *palette
                    .get(pal_idx)
                    .ok_or(AnvilDecodeError::ForeignOrCorrupt)?;
            }
            sections[sec_pos as usize] = ChunkSection::Blocks(arr);
        }

        let vanilla_light = if has_vanilla_light {
            Some(Box::new(VanillaChunkLight {
                sky_sections: vanilla_sky,
                block_sections: vanilla_block,
            }))
        } else {
            None
        };

        Ok(VanillaImportChunk {
            column: Self {
                sections,
                block_entities: block_entities.clone(),
                block_ticks,
                vanilla_light,
                dirty: false,
                revision: 0,
            },
            data_version,
            block_entities,
            section_biomes,
        })
    }

    /// Exports this [`ChunkColumn`] into standard vanilla Anvil Named-NBT bytes using `provider`,
    /// preserving all unknown root and section fields from `template_raw_nbt` when supplied.
    #[allow(dead_code)]
    pub fn export_to_vanilla_nbt_with(
        &self,
        template_raw_nbt: Option<&[u8]>,
        chunk_x: i32,
        chunk_z: i32,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> std::result::Result<Vec<u8>, AnvilDecodeError> {
        use rad_core::nbt::{NbtTag, decode_named_nbt, encode_named_nbt};
        use std::collections::BTreeMap;

        let (root_name, mut root) = if let Some(raw) = template_raw_nbt {
            let mut cursor = raw;
            match decode_named_nbt(&mut cursor) {
                Ok((name, NbtTag::Compound(map))) => (name, map),
                _ => return Err(AnvilDecodeError::ForeignOrCorrupt),
            }
        } else {
            let mut map = BTreeMap::new();
            map.insert("DataVersion".to_owned(), NbtTag::Int(4790));
            map.insert("xPos".to_owned(), NbtTag::Int(chunk_x));
            map.insert("yPos".to_owned(), NbtTag::Int(-4));
            map.insert("zPos".to_owned(), NbtTag::Int(chunk_z));
            map.insert("LastUpdate".to_owned(), NbtTag::Long(0));
            map.insert("InhabitedTime".to_owned(), NbtTag::Long(0));
            map.insert(
                "Status".to_owned(),
                NbtTag::String("minecraft:full".to_owned()),
            );
            (String::new(), map)
        };

        let existing_sections = match root.remove("sections") {
            Some(NbtTag::List(list)) => list,
            _ => Vec::new(),
        };
        let mut by_y: BTreeMap<i8, BTreeMap<String, NbtTag>> = BTreeMap::new();
        for tag in existing_sections {
            if let NbtTag::Compound(sec) = tag
                && let Some(NbtTag::Byte(y_idx)) = sec.get("Y")
            {
                by_y.insert(*y_idx, sec);
            }
        }

        let mut out_sections = Vec::new();
        for (sec_idx, section) in self.sections.iter().enumerate().take(OVERWORLD_SECTIONS) {
            let y_idx = (sec_idx as i32 - 4) as i8;
            let has_existing = by_y.contains_key(&y_idx);
            let is_empty_air = matches!(section, ChunkSection::Uniform(0));
            if !has_existing && is_empty_air {
                continue;
            }
            let bs = region::encode_section_block_states_with(section, provider)?;
            let mut sec_map = by_y.remove(&y_idx).unwrap_or_default();
            sec_map.insert("Y".to_owned(), NbtTag::Byte(y_idx));
            sec_map.insert("block_states".to_owned(), NbtTag::Compound(bs));
            if !sec_map.contains_key("biomes") {
                let mut biomes = BTreeMap::new();
                biomes.insert(
                    "palette".to_owned(),
                    NbtTag::List(vec![NbtTag::String("minecraft:plains".to_owned())]),
                );
                sec_map.insert("biomes".to_owned(), NbtTag::Compound(biomes));
            }
            out_sections.push(NbtTag::Compound(sec_map));
        }

        // Preserve any out-of-range sections from the original template untouched.
        for (_, extra_sec) in by_y {
            out_sections.push(NbtTag::Compound(extra_sec));
        }

        root.insert("sections".to_owned(), NbtTag::List(out_sections));
        if !self.block_entities.is_empty() {
            root.insert(
                "block_entities".to_owned(),
                NbtTag::List(
                    self.block_entities
                        .iter()
                        .cloned()
                        .map(NbtTag::Compound)
                        .collect(),
                ),
            );
        } else if template_raw_nbt.is_none() {
            root.remove("block_entities");
        }
        if !self.block_ticks.is_empty() {
            root.insert(
                "block_ticks".to_owned(),
                NbtTag::List(
                    self.block_ticks
                        .iter()
                        .cloned()
                        .map(NbtTag::Compound)
                        .collect(),
                ),
            );
        } else if template_raw_nbt.is_none() {
            root.remove("block_ticks");
        }

        let mut out = BytesMut::new();
        encode_named_nbt(&root_name, &NbtTag::Compound(root), &mut out)
            .map_err(|_| AnvilDecodeError::ForeignOrCorrupt)?;
        Ok(out.to_vec())
    }

    /// Exports this [`ChunkColumn`] as a zlib-compressed vanilla Anvil `.mca` payload using `provider`.
    #[allow(dead_code)]
    pub fn export_to_vanilla_storage_bytes_with(
        &self,
        template_raw_nbt: Option<&[u8]>,
        chunk_x: i32,
        chunk_z: i32,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> std::result::Result<Vec<u8>, AnvilDecodeError> {
        let raw_nbt =
            self.export_to_vanilla_nbt_with(template_raw_nbt, chunk_x, chunk_z, provider)?;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&raw_nbt)
            .map_err(|_| AnvilDecodeError::ForeignOrCorrupt)?;
        enc.finish().map_err(|_| AnvilDecodeError::ForeignOrCorrupt)
    }
}
