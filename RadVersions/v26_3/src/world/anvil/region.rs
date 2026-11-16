//! Region .mca file I/O, storage format inspection, and vanilla Anvil import/export.

use super::*;

/// Underlying on-disk chunk payload encoding format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum StorageFormat {
    /// RadServer compact `RAD1` section snapshot format.
    Rad1,
    /// Vanilla Minecraft Anvil Named-NBT chunk format.
    VanillaAnvil,
    /// Unrecognized or corrupt payload.
    Unknown,
}

/// Inspects raw decompressed chunk payload bytes to distinguish `RAD1` from vanilla NBT.
#[allow(dead_code)]
#[must_use]
pub fn detect_storage_format(raw: &[u8]) -> StorageFormat {
    if raw.len() >= 4 && &raw[..4] == b"RAD1" {
        StorageFormat::Rad1
    } else if raw.first() == Some(&0x0A) {
        StorageFormat::VanillaAnvil
    } else {
        StorageFormat::Unknown
    }
}

/// Full vanilla Anvil NBT chunk import preserving metadata, block entities, and biomes.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct VanillaImportChunk {
    /// Imported voxel sections.
    pub column: ChunkColumn,
    /// `DataVersion` tag if present in vanilla NBT root.
    pub data_version: Option<i32>,
    /// Preserved vanilla `block_entities` compound tags.
    pub block_entities: Vec<std::collections::BTreeMap<String, rad_core::nbt::NbtTag>>,
    /// Preserved biome palette names per section `(section_y, palette_names)`.
    pub section_biomes: Vec<(i8, Vec<String>)>,
}

pub(super) fn vanilla_block_entry_to_state_with(
    name: &str,
    props: Option<&std::collections::BTreeMap<String, rad_core::nbt::NbtTag>>,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
) -> Option<u16> {
    use rad_core::nbt::NbtTag;

    let key = name.strip_prefix("minecraft:").unwrap_or(name);
    match key {
        "air" | "cave_air" | "void_air" => provider
            .block_by_name("minecraft:air")
            .map(|b| b.default_state)
            .or(Some(0)),
        _ => {
            let full_name = if name.contains(':') {
                name.to_owned()
            } else {
                format!("minecraft:{name}")
            };
            let def = provider.block_by_name(&full_name)?;
            let Some(props) = props else {
                return Some(def.default_state);
            };
            let prop_str = |k: &str| match props.get(k) {
                Some(NbtTag::String(s)) => Some(s.as_str()),
                _ => None,
            };
            let candidate = match key {
                "wheat" => prop_str("age")
                    .and_then(|s| s.parse::<u16>().ok())
                    .map(|age| def.min_state + age.min(7)),
                "water" => prop_str("level")
                    .and_then(|s| s.parse::<u16>().ok())
                    .map(|lvl| def.min_state + lvl.min(15)),
                "redstone_lamp" => match prop_str("lit") {
                    Some("true") => Some(def.min_state),
                    Some("false") => Some(def.max_state),
                    _ => None,
                },
                "redstone_wire" => prop_str("power")
                    .and_then(|s| s.parse::<u16>().ok())
                    .map(|p| {
                        if p > 0 {
                            def.max_state
                        } else {
                            def.default_state
                        }
                    }),
                "oak_log" | "spruce_log" | "birch_log" => match prop_str("axis") {
                    Some("x") => Some(def.min_state),
                    Some("y") => Some(def.default_state),
                    Some("z") => Some(def.max_state),
                    _ => None,
                },
                "lever" => match prop_str("powered") {
                    Some("true") => Some(def.default_state + 1),
                    Some("false") => Some(def.default_state),
                    _ => None,
                },
                _ => None,
            };
            Some(
                candidate
                    .filter(|&st| st >= def.min_state && st <= def.max_state)
                    .unwrap_or(def.default_state),
            )
        }
    }
}

#[allow(dead_code)]
fn state_to_vanilla_block_entry_with(
    state_id: u16,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
) -> std::result::Result<std::collections::BTreeMap<String, rad_core::nbt::NbtTag>, AnvilDecodeError>
{
    use rad_core::nbt::NbtTag;
    use std::collections::BTreeMap;

    let def = provider
        .block_by_state(state_id)
        .ok_or(AnvilDecodeError::ForeignOrCorrupt)?;
    let key = def.name.strip_prefix("minecraft:").unwrap_or(&def.name);
    let mut entry = BTreeMap::new();
    entry.insert("Name".to_owned(), NbtTag::String(def.name.clone()));

    let mut props = BTreeMap::new();
    match key {
        "wheat" => {
            let age = state_id.saturating_sub(def.min_state).min(7);
            props.insert("age".to_owned(), NbtTag::String(age.to_string()));
        }
        "water" => {
            let lvl = state_id.saturating_sub(def.min_state).min(15);
            props.insert("level".to_owned(), NbtTag::String(lvl.to_string()));
        }
        "redstone_lamp" => {
            let lit = if state_id == def.min_state {
                "true"
            } else {
                "false"
            };
            props.insert("lit".to_owned(), NbtTag::String(lit.to_owned()));
        }
        "redstone_wire" => {
            let power = if state_id == def.max_state { "15" } else { "0" };
            props.insert("power".to_owned(), NbtTag::String(power.to_owned()));
        }
        "oak_log" | "spruce_log" | "birch_log" => {
            let axis = if state_id == def.min_state {
                "x"
            } else if state_id == def.max_state {
                "z"
            } else {
                "y"
            };
            props.insert("axis".to_owned(), NbtTag::String(axis.to_owned()));
        }
        "lever" => {
            let powered = if state_id == def.default_state + 1 {
                "true"
            } else {
                "false"
            };
            props.insert("powered".to_owned(), NbtTag::String(powered.to_owned()));
        }
        _ => {}
    }
    if !props.is_empty() {
        entry.insert("Properties".to_owned(), NbtTag::Compound(props));
    }
    Ok(entry)
}

#[allow(dead_code)]
pub(super) fn encode_section_block_states_with(
    section: &ChunkSection,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
) -> std::result::Result<std::collections::BTreeMap<String, rad_core::nbt::NbtTag>, AnvilDecodeError>
{
    use rad_core::nbt::NbtTag;
    use std::collections::{BTreeMap, HashMap};

    let mut palette_states: Vec<u16> = Vec::new();
    let mut palette_tags: Vec<NbtTag> = Vec::new();
    let mut bs = BTreeMap::new();

    match section {
        ChunkSection::Uniform(state) => {
            palette_tags.push(NbtTag::Compound(state_to_vanilla_block_entry_with(
                *state, provider,
            )?));
            bs.insert("palette".to_owned(), NbtTag::List(palette_tags));
        }
        ChunkSection::Blocks(arr) => {
            let mut state_to_idx: HashMap<u16, usize> = HashMap::new();
            let mut indices = Vec::with_capacity(4096);
            for &st in arr.iter() {
                let idx = if let Some(&existing) = state_to_idx.get(&st) {
                    existing
                } else {
                    let next = palette_states.len();
                    palette_states.push(st);
                    palette_tags.push(NbtTag::Compound(state_to_vanilla_block_entry_with(
                        st, provider,
                    )?));
                    state_to_idx.insert(st, next);
                    next
                };
                indices.push(idx);
            }
            bs.insert("palette".to_owned(), NbtTag::List(palette_tags));
            if palette_states.len() > 1 {
                let bits = ((palette_states.len() - 1).ilog2() as usize + 1).max(4);
                let per_long = 64 / bits;
                let num_longs = 4096usize.div_ceil(per_long);
                let mut longs = vec![0i64; num_longs];
                for (block_idx, &pal_idx) in indices.iter().enumerate() {
                    let long_idx = block_idx / per_long;
                    let bit_off = (block_idx % per_long) * bits;
                    longs[long_idx] |= ((pal_idx as u64) << bit_off) as i64;
                }
                bs.insert("data".to_owned(), NbtTag::LongArray(longs));
            }
        }
    }

    Ok(bs)
}

/// Anvil `.mca` region file helper.
pub struct AnvilRegion;

impl AnvilRegion {
    fn region_path_in(world_dir: &Path, subdir: &str, chunk_x: i32, chunk_z: i32) -> PathBuf {
        let rx = chunk_x.div_euclid(32);
        let rz = chunk_z.div_euclid(32);
        world_dir.join(subdir).join(format!("r.{rx}.{rz}.mca"))
    }

    #[cfg(test)]
    fn region_path(world_dir: &Path, chunk_x: i32, chunk_z: i32) -> PathBuf {
        Self::region_path_in(world_dir, "region", chunk_x, chunk_z)
    }

    fn header_index(chunk_x: i32, chunk_z: i32) -> usize {
        let lx = chunk_x.rem_euclid(32) as usize;
        let lz = chunk_z.rem_euclid(32) as usize;
        (lx + lz * 32) * 4
    }

    /// Reads raw compressed chunk payload from `<world_dir>/region/*.mca`, distinguishing absent (`Ok(None)`)
    /// from corrupt/oversized sectors (`Err(AnvilDecodeError::ForeignOrCorrupt)`).
    pub fn read_chunk_payload_checked(
        world_dir: &Path,
        chunk_x: i32,
        chunk_z: i32,
    ) -> std::result::Result<Option<Vec<u8>>, AnvilDecodeError> {
        Self::read_subdir_chunk_payload_checked(world_dir, "region", chunk_x, chunk_z)
    }

    /// Reads raw compressed chunk payload from `<world_dir>/<subdir>/*.mca`.
    pub fn read_subdir_chunk_payload_checked(
        world_dir: &Path,
        subdir: &str,
        chunk_x: i32,
        chunk_z: i32,
    ) -> std::result::Result<Option<Vec<u8>>, AnvilDecodeError> {
        let path = Self::region_path_in(world_dir, subdir, chunk_x, chunk_z);
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(AnvilDecodeError::ForeignOrCorrupt),
        };
        let mut header = [0u8; HEADER_BYTES];
        if file.read_exact(&mut header).is_err() {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }

        let idx = Self::header_index(chunk_x, chunk_z);
        let offset_sectors = ((u32::from(header[idx]) << 16)
            | (u32::from(header[idx + 1]) << 8)
            | u32::from(header[idx + 2])) as u64;
        let sector_count = header[idx + 3];
        if offset_sectors == 0 && sector_count == 0 {
            return Ok(None);
        }
        if offset_sectors < 2 || sector_count == 0 {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }

        if file
            .seek(SeekFrom::Start(offset_sectors * SECTOR_BYTES as u64))
            .is_err()
        {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        let mut len_buf = [0u8; 5];
        if file.read_exact(&mut len_buf).is_err() {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        let declared_len =
            u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;
        if declared_len == 0 {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        let payload_len = declared_len - 1;
        let compression_type = len_buf[4];
        if compression_type != 2 || payload_len + 5 > usize::from(sector_count) * SECTOR_BYTES {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        let mut payload = vec![0u8; payload_len];
        if file.read_exact(&mut payload).is_err() {
            return Err(AnvilDecodeError::ForeignOrCorrupt);
        }
        Ok(Some(payload))
    }

    /// Reads all valid zlib-compressed chunk payloads from a single `.mca` file in one pass.
    pub fn read_all_payloads_from_file(path: &Path) -> Vec<Vec<u8>> {
        let Ok(mut file) = File::open(path) else {
            return Vec::new();
        };
        let mut header = [0u8; HEADER_BYTES];
        if file.read_exact(&mut header).is_err() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for idx in (0..4096).step_by(4) {
            let offset_sectors = ((u32::from(header[idx]) << 16)
                | (u32::from(header[idx + 1]) << 8)
                | u32::from(header[idx + 2])) as u64;
            let sector_count = header[idx + 3];
            if offset_sectors < 2 || sector_count == 0 {
                continue;
            }
            if file
                .seek(SeekFrom::Start(offset_sectors * SECTOR_BYTES as u64))
                .is_err()
            {
                continue;
            }
            let mut len_buf = [0u8; 5];
            if file.read_exact(&mut len_buf).is_err() {
                continue;
            }
            let declared_len =
                u32::from_be_bytes([len_buf[0], len_buf[1], len_buf[2], len_buf[3]]) as usize;
            if declared_len == 0 {
                continue;
            }
            let payload_len = declared_len - 1;
            let compression_type = len_buf[4];
            if compression_type != 2 || payload_len + 5 > usize::from(sector_count) * SECTOR_BYTES {
                continue;
            }
            let mut payload = vec![0u8; payload_len];
            if file.read_exact(&mut payload).is_ok() {
                out.push(payload);
            }
        }
        out
    }

    /// Reads raw compressed chunk payload from `.mca` (returns `None` if absent).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn read_chunk_payload(world_dir: &Path, chunk_x: i32, chunk_z: i32) -> Option<Vec<u8>> {
        Self::read_chunk_payload_checked(world_dir, chunk_x, chunk_z)
            .ok()
            .flatten()
    }

    /// Writes a zlib-compressed chunk payload into `<world_dir>/region/r.<rx>.<rz>.mca`, reusing in-place
    /// sectors when large enough and rejecting payloads exceeding 255 sectors (`1020 KiB`).
    pub fn write_chunk_payload(
        world_dir: &Path,
        chunk_x: i32,
        chunk_z: i32,
        zlib_payload: &[u8],
    ) -> std::io::Result<()> {
        Self::write_subdir_chunk_payload(world_dir, "region", chunk_x, chunk_z, zlib_payload)
    }

    /// Writes a zlib-compressed chunk payload into `<world_dir>/<subdir>/r.<rx>.<rz>.mca`.
    pub fn write_subdir_chunk_payload(
        world_dir: &Path,
        subdir: &str,
        chunk_x: i32,
        chunk_z: i32,
        zlib_payload: &[u8],
    ) -> std::io::Result<()> {
        let total_chunk_bytes = 5 + zlib_payload.len();
        let needed_sectors_usize = total_chunk_bytes.div_ceil(SECTOR_BYTES).max(1);
        let needed_sectors = u8::try_from(needed_sectors_usize).map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "chunk payload exceeds 255 Anvil sectors",
            )
        })?;

        let path = Self::region_path_in(world_dir, subdir, chunk_x, chunk_z);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let file_len = file.metadata()?.len() as usize;
        if file_len != 0 && file_len < HEADER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "truncated Anvil region header",
            ));
        }
        let mut header = [0u8; HEADER_BYTES];
        if file_len >= HEADER_BYTES {
            file.seek(SeekFrom::Start(0))?;
            file.read_exact(&mut header)?;
        } else {
            file.set_len(HEADER_BYTES as u64)?;
        }

        let idx = Self::header_index(chunk_x, chunk_z);
        let existing_offset = (u32::from(header[idx]) << 16)
            | (u32::from(header[idx + 1]) << 8)
            | u32::from(header[idx + 2]);
        let existing_sectors = header[idx + 3];

        let sector_offset = if existing_offset >= 2 && existing_sectors >= needed_sectors {
            existing_offset
        } else {
            u32::try_from(file_len.max(HEADER_BYTES).div_ceil(SECTOR_BYTES)).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Anvil region offset too large",
                )
            })?
        };
        if sector_offset > 0x00FF_FFFF {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Anvil region offset exceeds 24 bits",
            ));
        }

        file.seek(SeekFrom::Start(
            u64::from(sector_offset) * SECTOR_BYTES as u64,
        ))?;
        let declared_len = (1 + zlib_payload.len()) as u32;
        file.write_all(&declared_len.to_be_bytes())?;
        file.write_all(&[2u8])?; // zlib
        file.write_all(zlib_payload)?;
        let rem = total_chunk_bytes % SECTOR_BYTES;
        if rem != 0 {
            let pad = vec![0u8; SECTOR_BYTES - rem];
            file.write_all(&pad)?;
        }

        header[idx] = ((sector_offset >> 16) & 0xFF) as u8;
        header[idx + 1] = ((sector_offset >> 8) & 0xFF) as u8;
        header[idx + 2] = (sector_offset & 0xFF) as u8;
        header[idx + 3] = needed_sectors;

        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header)?;
        file.flush()?;
        Ok(())
    }
}

#[inline]
pub(super) fn set_nibble(arr: &mut [u8; 2048], x: usize, y: usize, z: usize, val: u8) {
    let idx = (y << 8) | (z << 4) | x;
    let byte = idx >> 1;
    let v = val & 0x0F;
    if idx & 1 == 0 {
        arr[byte] = (arr[byte] & 0xF0) | v;
    } else {
        arr[byte] = (arr[byte] & 0x0F) | (v << 4);
    }
}

#[inline]
pub(super) fn get_nibble(arr: &[u8], x: usize, y: usize, z: usize) -> u8 {
    let idx = (y << 8) | (z << 4) | x;
    let b = arr[idx >> 1];
    if idx & 1 == 0 {
        b & 0x0F
    } else {
        (b >> 4) & 0x0F
    }
}

/// Returns block light emission level (`0..=15`) for block `state_id` using `provider`.
#[must_use]
pub fn block_light_emission_with(
    state_id: u16,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
) -> u8 {
    let Some(def) = provider.block_by_state(state_id) else {
        return 0;
    };
    match def.name.as_str() {
        "minecraft:glowstone"
        | "minecraft:lava"
        | "minecraft:fire"
        | "minecraft:sea_lantern"
        | "minecraft:shroomlight"
        | "minecraft:jack_o_lantern"
        | "minecraft:beacon"
        | "minecraft:end_rod"
        | "minecraft:lantern" => 15,
        "minecraft:redstone_lamp" if state_id == def.min_state => 15,
        "minecraft:torch" | "minecraft:wall_torch" | "minecraft:end_portal" => 14,
        "minecraft:nether_portal" => 11,
        "minecraft:soul_torch" | "minecraft:soul_wall_torch" | "minecraft:soul_lantern" => 10,
        "minecraft:redstone_torch" if state_id == def.min_state => 7,
        "minecraft:brewing_stand" => 1,
        _ => 0,
    }
}

/// Returns light opacity (`0` transparent, `1` translucent, `15` opaque) for block `state_id` using `provider`.
#[must_use]
pub fn block_light_opacity_with(
    state_id: u16,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
) -> u8 {
    if state_id == 0 {
        return 0;
    }
    let Some(def) = provider.block_by_state(state_id) else {
        return 15;
    };
    match def.name.as_str() {
        "minecraft:air"
        | "minecraft:cave_air"
        | "minecraft:void_air"
        | "minecraft:glass"
        | "minecraft:torch"
        | "minecraft:wall_torch"
        | "minecraft:redstone_torch"
        | "minecraft:redstone_wire"
        | "minecraft:lever"
        | "minecraft:wheat"
        | "minecraft:fire"
        | "minecraft:nether_portal"
        | "minecraft:end_portal"
        | "minecraft:pink_petals"
        | "minecraft:wildflowers"
        | "minecraft:leaf_litter"
        | "minecraft:bush"
        | "minecraft:firefly_bush"
        | "minecraft:short_dry_grass"
        | "minecraft:tall_dry_grass"
        | "minecraft:cactus_flower"
        | "minecraft:lily_pad"
        | "minecraft:glow_lichen"
        | "minecraft:sculk_vein" => 0,
        "minecraft:water" | "minecraft:ice" | "minecraft:cobweb" => 1,
        name if name.ends_with("_leaves") => 1,
        name if name.ends_with("_glass") || name.ends_with("_pane") => 0,
        _ => {
            // Non-full cubes (grass, flowers, ferns, rails, fences, slabs, stairs, etc.) do not block skylight.
            match rad_version::shapes::block_collision_shape_with(state_id, provider) {
                Some(shape) => {
                    let boxes = shape.as_boxes();
                    if boxes.len() == 1 && boxes[0] == rad_version::shapes::BlockBox::FULL {
                        15
                    } else {
                        0
                    }
                }
                None => 15,
            }
        }
    }
}

/// Precomputed per-block-state light opacity and emission lookup tables (`O(1)` indexing).
#[derive(Debug, Clone)]
pub struct LightTables {
    opacity: Vec<u8>,
    emission: Vec<u8>,
}

impl LightTables {
    /// Builds lookup tables covering all block states registered in `provider` (up to `32768`).
    #[must_use]
    pub fn from_provider(provider: &dyn rad_core::registry_provider::RegistryProvider) -> Self {
        const TABLE_LEN: usize = 32_768;
        let mut opacity = Vec::with_capacity(TABLE_LEN);
        let mut emission = Vec::with_capacity(TABLE_LEN);
        for raw in 0..TABLE_LEN {
            let state = raw as u16;
            opacity.push(block_light_opacity_with(state, provider));
            emission.push(block_light_emission_with(state, provider));
        }
        Self { opacity, emission }
    }

    /// Returns precomputed light opacity (`0`, `1`, or `15`) for `state_id`.
    #[inline]
    #[must_use]
    pub fn opacity(&self, state_id: u16) -> u8 {
        self.opacity
            .get(usize::from(state_id))
            .copied()
            .unwrap_or(15)
    }

    /// Returns precomputed light emission (`0..=15`) for `state_id`.
    #[inline]
    #[must_use]
    pub fn emission(&self, state_id: u16) -> u8 {
        self.emission
            .get(usize::from(state_id))
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod storage_boundary_tests {
    use super::*;
    static NEXT_DIRECTORY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temporary_world() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "radbyte-region-review-{}-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(path.join("region")).unwrap();
        path
    }

    #[test]
    fn rejects_payload_crossing_sector_and_zero_declared_length() {
        let world = temporary_world();
        let path = AnvilRegion::region_path(&world, 0, 0);
        let mut bytes = vec![0; HEADER_BYTES + 2 * SECTOR_BYTES];
        bytes[2] = 2;
        bytes[3] = 1;
        bytes[HEADER_BYTES..HEADER_BYTES + 4].copy_from_slice(&4093u32.to_be_bytes());
        bytes[HEADER_BYTES + 4] = 2;
        std::fs::write(&path, &bytes).unwrap();
        assert!(AnvilRegion::read_chunk_payload_checked(&world, 0, 0).is_err());
        bytes[HEADER_BYTES..HEADER_BYTES + 4].fill(0);
        std::fs::write(&path, &bytes).unwrap();
        assert!(AnvilRegion::read_chunk_payload_checked(&world, 0, 0).is_err());
        std::fs::remove_dir_all(world).unwrap();
    }

    #[test]
    fn appends_after_partial_sector_without_overwriting_existing_bytes() {
        let world = temporary_world();
        AnvilRegion::write_chunk_payload(&world, 0, 0, b"first").unwrap();
        let path = AnvilRegion::region_path(&world, 0, 0);
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        file.write_all(&[0xAB]).unwrap();
        drop(file);
        AnvilRegion::write_chunk_payload(&world, 1, 0, b"second").unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap()[HEADER_BYTES + SECTOR_BYTES],
            0xAB
        );
        assert_eq!(
            AnvilRegion::read_chunk_payload_checked(&world, 0, 0)
                .unwrap()
                .unwrap(),
            b"first"
        );
        assert_eq!(
            AnvilRegion::read_chunk_payload_checked(&world, 1, 0)
                .unwrap()
                .unwrap(),
            b"second"
        );
        std::fs::remove_dir_all(world).unwrap();
    }

    #[test]
    fn refuses_to_overwrite_truncated_region_header() {
        let world = temporary_world();
        let path = AnvilRegion::region_path(&world, 0, 0);
        std::fs::write(&path, b"existing-damaged-header").unwrap();
        assert!(AnvilRegion::write_chunk_payload(&world, 0, 0, b"chunk").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"existing-damaged-header");
        std::fs::remove_dir_all(world).unwrap();
    }
}
