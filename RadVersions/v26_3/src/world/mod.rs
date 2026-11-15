//! Flat world chunk generation, Anvil persistence, and voxel physics.

pub mod anvil;
pub mod containers;
pub mod migrate;
pub mod physics;
pub mod sim;
pub mod worldgen;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::BytesMut;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use rad_core::nbt::{NbtTag, decode_named_nbt, encode_named_nbt};
use rad_core::packet::play::LevelChunkWithLight;
use tracing::warn;

use self::anvil::{AnvilRegion, ChunkColumn};
use crate::registries::Version26_3Registries;

/// Number of 16x16x16 sections in standard Overworld (`Y = -64..320`).
pub const OVERWORLD_SECTIONS: usize = 24;
/// Default maximum number of chunk columns held in memory before LRU eviction.
pub const DEFAULT_MAX_CACHED_CHUNKS: usize = 2048;

/// Snapshot of a dirty chunk `(chunk_x, chunk_z, revision, zlib_payload)` ready for async disk write.
/// `(dimension, chunk_x, chunk_z, revision, encoded_storage_bytes)` snapshot for async disk saving.
pub type DirtyChunkSnapshot = (worldgen::DimensionId, i32, i32, u64, Vec<u8>);

/// Returns the Anvil directory for a given dimension under `base`.
#[must_use]
pub fn dimension_dir_for(base: &Path, dim: worldgen::DimensionId) -> PathBuf {
    match dim.anvil_subdir() {
        Some(sub) => base.join(sub),
        None => base.to_path_buf(),
    }
}

/// Vanilla-compatible `level.dat` world metadata stored as gzipped Named NBT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelMetadata {
    /// World name (`LevelName`).
    pub level_name: String,
    /// World generation seed (`RandomSeed`).
    pub random_seed: i64,
    /// Total world ticks elapsed (`Time`).
    pub time: i64,
    /// Current time of day in ticks (`DayTime`).
    pub day_time: i64,
    /// World spawn X coordinate (`SpawnX`).
    pub spawn_x: i32,
    /// World spawn Y coordinate (`SpawnY`).
    pub spawn_y: i32,
    /// World spawn Z coordinate (`SpawnZ`).
    pub spawn_z: i32,
    /// Default game mode (`GameType`).
    pub game_type: i32,
    /// Minecraft data version (`DataVersion`).
    pub data_version: i32,
}

impl Default for LevelMetadata {
    fn default() -> Self {
        Self {
            level_name: "world".to_owned(),
            random_seed: 0,
            time: 0,
            day_time: 1000,
            spawn_x: 0,
            spawn_y: 64,
            spawn_z: 0,
            game_type: 0,
            data_version: 4790,
        }
    }
}

impl LevelMetadata {
    /// Loads `<world_dir>/level.dat` (gzipped Named NBT), returning defaults if missing or invalid.
    #[must_use]
    pub fn load_from_dir(world_dir: &Path) -> Self {
        let path = world_dir.join("level.dat");
        let Ok(compressed) = std::fs::read(path) else {
            return Self::default();
        };
        let mut dec = GzDecoder::new(compressed.as_slice()).take(4 * 1024 * 1024);
        let mut raw = Vec::new();
        if dec.read_to_end(&mut raw).is_err() {
            return Self::default();
        }
        let mut cursor = raw.as_slice();
        let Ok((_, NbtTag::Compound(root))) = decode_named_nbt(&mut cursor) else {
            return Self::default();
        };
        let Some(NbtTag::Compound(data)) = root.get("Data") else {
            return Self::default();
        };
        let def = Self::default();
        Self {
            level_name: match data.get("LevelName") {
                Some(NbtTag::String(s)) => s.clone(),
                _ => def.level_name,
            },
            random_seed: match data.get("RandomSeed") {
                Some(NbtTag::Long(v)) => *v,
                _ => def.random_seed,
            },
            time: match data.get("Time") {
                Some(NbtTag::Long(v)) => *v,
                _ => def.time,
            },
            day_time: match data.get("DayTime") {
                Some(NbtTag::Long(v)) => *v,
                _ => def.day_time,
            },
            spawn_x: match data.get("SpawnX") {
                Some(NbtTag::Int(v)) => *v,
                _ => def.spawn_x,
            },
            spawn_y: match data.get("SpawnY") {
                Some(NbtTag::Int(v)) => *v,
                _ => def.spawn_y,
            },
            spawn_z: match data.get("SpawnZ") {
                Some(NbtTag::Int(v)) => *v,
                _ => def.spawn_z,
            },
            game_type: match data.get("GameType") {
                Some(NbtTag::Int(v)) => *v,
                _ => def.game_type,
            },
            data_version: match data.get("DataVersion") {
                Some(NbtTag::Int(v)) => *v,
                _ => def.data_version,
            },
        }
    }

    /// Atomically writes `level.dat` as gzipped Named NBT to `<world_dir>/level.dat`.
    pub fn write_atomic(&self, world_dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(world_dir)?;
        let mut data = BTreeMap::new();
        data.insert(
            "LevelName".to_owned(),
            NbtTag::String(self.level_name.clone()),
        );
        data.insert("RandomSeed".to_owned(), NbtTag::Long(self.random_seed));
        data.insert("Time".to_owned(), NbtTag::Long(self.time));
        data.insert("DayTime".to_owned(), NbtTag::Long(self.day_time));
        data.insert("SpawnX".to_owned(), NbtTag::Int(self.spawn_x));
        data.insert("SpawnY".to_owned(), NbtTag::Int(self.spawn_y));
        data.insert("SpawnZ".to_owned(), NbtTag::Int(self.spawn_z));
        data.insert("GameType".to_owned(), NbtTag::Int(self.game_type));
        data.insert("DataVersion".to_owned(), NbtTag::Int(self.data_version));

        let mut root = BTreeMap::new();
        root.insert("Data".to_owned(), NbtTag::Compound(data));

        let mut nbt_bytes = BytesMut::new();
        encode_named_nbt("", &NbtTag::Compound(root), &mut nbt_bytes)
            .map_err(|e| std::io::Error::other(format!("level.dat NBT encode failed: {e}")))?;

        let mut enc = GzEncoder::new(Vec::new(), Compression::fast());
        enc.write_all(&nbt_bytes)?;
        let gz = enc.finish()?;

        let path = world_dir.join("level.dat");
        let tmp = world_dir.join(format!("level.dat.{}.tmp", std::process::id()));
        std::fs::write(&tmp, gz)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Mutable world chunk store backed by flat generation and optional Anvil `.mca` persistence.
pub struct WorldState {
    world_dir: Option<PathBuf>,
    registries: Arc<Version26_3Registries>,
    worldgen_palette: Option<worldgen::WorldgenPalette>,
    chunks: HashMap<(worldgen::DimensionId, i32, i32), ChunkColumn>,
    protected_chunks: HashSet<(worldgen::DimensionId, i32, i32)>,
    access_order: HashMap<(worldgen::DimensionId, i32, i32), u64>,
    access_counter: u64,
    max_cached_chunks: usize,
    generator: worldgen::WorldGenerator,
}

impl WorldState {
    /// Creates a new [`WorldState`] backed by `registries`. When `world_dir` is `Some`, dirty chunks are persisted to `.mca`.
    #[must_use]
    pub fn new_with_registries(
        world_dir: Option<PathBuf>,
        registries: Arc<Version26_3Registries>,
    ) -> Self {
        let worldgen_palette =
            worldgen::WorldgenPalette::from_provider(registries.provider().as_ref()).ok();
        Self {
            world_dir,
            registries,
            worldgen_palette,
            chunks: HashMap::new(),
            protected_chunks: HashSet::new(),
            access_order: HashMap::new(),
            access_counter: 0,
            max_cached_chunks: DEFAULT_MAX_CACHED_CHUNKS,
            generator: worldgen::WorldGenerator::Flat,
        }
    }

    /// Returns the resolved 26.3 registries associated with this world.
    #[must_use]
    pub fn registries(&self) -> &Arc<Version26_3Registries> {
        &self.registries
    }

    /// Configures the world generator used for unpersisted chunks.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn with_generator(mut self, generator: worldgen::WorldGenerator) -> Self {
        self.generator = generator;
        self
    }

    /// Generates or loads a chunk packet for the requested dimension.
    #[allow(dead_code)]
    pub fn dimension_chunk_packet(
        &mut self,
        dim: worldgen::DimensionId,
        cx: i32,
        cz: i32,
    ) -> LevelChunkWithLight {
        self.chunk_packet_in(dim, cx, cz)
    }

    /// Ensures chunk `(cx, cz)` in `dim` is loaded and returns its network packet.
    pub fn chunk_packet_in(
        &mut self,
        dim: worldgen::DimensionId,
        cx: i32,
        cz: i32,
    ) -> LevelChunkWithLight {
        let registries = Arc::clone(&self.registries);
        self.get_or_load_mut(dim, cx, cz)
            .to_packet_with(cx, cz, registries.provider().as_ref())
    }

    /// Configures the maximum in-memory chunk cache capacity (clamped to >= 1).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_max_cached_chunks(&mut self, max: usize) {
        self.max_cached_chunks = max.max(1);
    }

    /// Returns the number of chunks currently cached in memory.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn cached_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Returns the root world directory when persistence is enabled.
    #[must_use]
    pub fn world_dir(&self) -> Option<&Path> {
        self.world_dir.as_deref()
    }

    /// Ensures Overworld chunk `(cx, cz)` is loaded and returns its network packet.
    #[allow(dead_code)]
    pub fn chunk_packet(&mut self, cx: i32, cz: i32) -> LevelChunkWithLight {
        self.chunk_packet_in(worldgen::DimensionId::Overworld, cx, cz)
    }

    /// Reads the block state ID at Overworld coordinates `(x, y, z)`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_block(&mut self, x: i32, y: i32, z: i32) -> u16 {
        self.get_block_in(worldgen::DimensionId::Overworld, x, y, z)
    }

    /// Returns the number of currently loaded chunks in memory.
    #[cfg(test)]
    #[must_use]
    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Reads the block state ID at `(x, y, z)` in `dim`.
    pub fn get_block_in(&mut self, dim: worldgen::DimensionId, x: i32, y: i32, z: i32) -> u16 {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let lx = x.rem_euclid(16) as usize;
        let lz = z.rem_euclid(16) as usize;
        self.get_or_load_mut(dim, cx, cz).get_block(lx, y, lz)
    }

    /// Sets the block state ID at Overworld coordinates `(x, y, z)`.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state_id: u16) -> bool {
        self.set_block_in(worldgen::DimensionId::Overworld, x, y, z, state_id)
    }

    /// Sets the block state ID at `(x, y, z)` in `dim`.
    pub fn set_block_in(
        &mut self,
        dim: worldgen::DimensionId,
        x: i32,
        y: i32,
        z: i32,
        state_id: u16,
    ) -> bool {
        let cx = x.div_euclid(16);
        let cz = z.div_euclid(16);
        let lx = x.rem_euclid(16) as usize;
        let lz = z.rem_euclid(16) as usize;
        let _ = self.get_or_load_mut(dim, cx, cz);
        if self.protected_chunks.contains(&(dim, cx, cz)) {
            return false;
        }
        self.get_or_load_mut(dim, cx, cz)
            .set_block(lx, y, lz, state_id)
    }

    /// Snapshots dirty non-protected chunks `(dim, cx, cz, revision, encoded_bytes)` for async saving.
    pub fn snapshot_dirty_chunks(&self) -> std::io::Result<Vec<DirtyChunkSnapshot>> {
        if self.world_dir.is_none() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for (&(dim, cx, cz), col) in &self.chunks {
            if col.dirty && !self.protected_chunks.contains(&(dim, cx, cz)) {
                out.push((dim, cx, cz, col.revision, col.encode_storage_bytes()?));
            }
        }
        Ok(out)
    }

    /// Marks chunks clean only if their in-memory revision still matches `expected_revision`.
    pub fn mark_chunks_saved(&mut self, saved: &[(worldgen::DimensionId, i32, i32, u64)]) {
        for &(dim, cx, cz, expected_rev) in saved {
            if let Some(col) = self.chunks.get_mut(&(dim, cx, cz))
                && col.revision == expected_rev
            {
                col.dirty = false;
            }
        }
    }

    /// Saves all dirty chunks to `.mca` files under `world_dir`. Returns saved chunk count.
    #[allow(dead_code)]
    pub fn save_dirty_chunks(&mut self) -> std::io::Result<usize> {
        let Some(ref dir) = self.world_dir else {
            return Ok(0);
        };
        let mut saved = 0usize;
        for (&(dim, cx, cz), col) in &mut self.chunks {
            if col.dirty && !self.protected_chunks.contains(&(dim, cx, cz)) {
                let bytes = col.encode_storage_bytes()?;
                let dim_dir = dimension_dir_for(dir, dim);
                AnvilRegion::write_chunk_payload(&dim_dir, cx, cz, &bytes)?;
                col.dirty = false;
                saved += 1;
            }
        }
        Ok(saved)
    }

    fn get_or_load_mut(
        &mut self,
        dim: worldgen::DimensionId,
        cx: i32,
        cz: i32,
    ) -> &mut ChunkColumn {
        self.access_counter = self.access_counter.wrapping_add(1);
        self.access_order.insert((dim, cx, cz), self.access_counter);

        if !self.chunks.contains_key(&(dim, cx, cz)) {
            self.evict_if_full();
            let (col, protected) = load_or_generate(
                self.world_dir.as_deref(),
                self.generator,
                self.worldgen_palette.as_ref(),
                self.registries.provider().as_ref(),
                dim,
                cx,
                cz,
            );
            if protected {
                self.protected_chunks.insert((dim, cx, cz));
            }
            self.chunks.insert((dim, cx, cz), col);
        }
        self.chunks
            .get_mut(&(dim, cx, cz))
            .expect("chunk was just inserted")
    }

    fn evict_if_full(&mut self) {
        while self.chunks.len() >= self.max_cached_chunks {
            let oldest = self
                .chunks
                .keys()
                .min_by_key(|coord| self.access_order.get(*coord).copied().unwrap_or(0))
                .copied();
            let Some(coord) = oldest else {
                break;
            };
            self.access_order.remove(&coord);
            if let Some(col) = self.chunks.remove(&coord)
                && col.dirty
                && !self.protected_chunks.contains(&coord)
                && let Some(ref dir) = self.world_dir
                && let Ok(bytes) = col.encode_storage_bytes()
            {
                let dim_dir = dimension_dir_for(dir, coord.0);
                if let Err(err) =
                    AnvilRegion::write_chunk_payload(&dim_dir, coord.1, coord.2, &bytes)
                {
                    warn!(
                        dim = ?coord.0,
                        cx = coord.1,
                        cz = coord.2,
                        %err,
                        "failed to flush evicted dirty chunk"
                    );
                }
            }
        }
    }
}

fn default_chunk(
    generator: worldgen::WorldGenerator,
    palette: Option<&worldgen::WorldgenPalette>,
    dim: worldgen::DimensionId,
    cx: i32,
    cz: i32,
) -> ChunkColumn {
    match (generator, dim, palette) {
        (worldgen::WorldGenerator::Flat, worldgen::DimensionId::Overworld, _) => {
            ChunkColumn::flat()
        }
        (worldgen::WorldGenerator::Flat, _, Some(pal)) => {
            worldgen::generate_seeded_chunk_with(0, dim, cx, cz, pal)
        }
        (worldgen::WorldGenerator::Seeded(seed), _, Some(pal)) => {
            worldgen::generate_seeded_chunk_with(seed, dim, cx, cz, pal)
        }
        (_, _, None) => ChunkColumn::flat(),
    }
}

fn load_or_generate(
    world_dir: Option<&Path>,
    generator: worldgen::WorldGenerator,
    palette: Option<&worldgen::WorldgenPalette>,
    provider: &dyn rad_core::registry_provider::RegistryProvider,
    dim: worldgen::DimensionId,
    cx: i32,
    cz: i32,
) -> (ChunkColumn, bool) {
    let Some(base_dir) = world_dir else {
        return (default_chunk(generator, palette, dim, cx, cz), false);
    };
    let dir = dimension_dir_for(base_dir, dim);
    match AnvilRegion::read_chunk_payload_checked(&dir, cx, cz) {
        Ok(Some(payload)) => {
            match ChunkColumn::decode_storage_bytes_checked_with(&payload, provider) {
                Ok(col) => (col, false),
                Err(_) => {
                    warn!(
                        ?dim,
                        cx, cz, "chunk in .mca is foreign or corrupt; protecting from overwrite"
                    );
                    (default_chunk(generator, palette, dim, cx, cz), true)
                }
            }
        }
        Ok(None) => (default_chunk(generator, palette, dim, cx, cz), false),
        Err(_) => {
            warn!(
                ?dim,
                cx, cz, "chunk header/sector in .mca is corrupt; protecting from overwrite"
            );
            (default_chunk(generator, palette, dim, cx, cz), true)
        }
    }
}

/// Returns `(chunk_x, chunk_z)` coordinates within `radius` sorted closest-first.
#[must_use]
pub fn chunks_in_radius(center_x: i32, center_z: i32, radius: i32) -> Vec<(i32, i32)> {
    let r = radius.clamp(1, 32);
    let side = (r * 2 + 1) as usize;
    let mut coords = Vec::with_capacity(side * side);
    for dx in -r..=r {
        for dz in -r..=r {
            coords.push((center_x + dx, center_z + dz));
        }
    }
    coords.sort_by_key(|&(cx, cz)| {
        let dx = i64::from(cx - center_x);
        let dz = i64::from(cz - center_z);
        dx * dx + dz * dz
    });
    coords
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_chunk_and_radius_ordering_work() {
        let provider = crate::registries::synthetic_26_3_test_provider();
        let pkt = ChunkColumn::flat().to_packet_with(0, 0, &provider);
        assert_eq!(pkt.chunk_x, 0);
        assert_eq!(pkt.chunk_z, 0);
        assert!(!pkt.data.is_empty());
        let sky_mask = u32::from_le_bytes(pkt.sky_light_mask.clone().try_into().unwrap());
        let empty_sky_mask =
            u32::from_le_bytes(pkt.empty_sky_light_mask.clone().try_into().unwrap());
        assert_eq!(sky_mask.count_ones() as usize, pkt.sky_lights.len());
        assert_eq!(
            sky_mask.count_ones() + empty_sky_mask.count_ones(),
            (OVERWORLD_SECTIONS + 2) as u32
        );

        let ring = chunks_in_radius(0, 0, 2);
        assert_eq!(ring.len(), 25);
        assert_eq!(ring[0], (0, 0));
        assert_eq!(chunks_in_radius(0, 0, 20).len(), 41 * 41);
    }

    #[test]
    fn lru_eviction_flushes_dirty_chunks_and_bounds_cache() {
        let dir = std::env::temp_dir().join("rad_world_lru_test");
        let _ = std::fs::remove_dir_all(&dir);

        let regs = Arc::new(
            Version26_3Registries::from_provider(
                Arc::new(crate::registries::synthetic_26_3_test_provider()),
                true,
            )
            .unwrap(),
        );
        let mut world = WorldState::new_with_registries(Some(dir.clone()), regs);
        world.set_max_cached_chunks(2);

        assert!(world.set_block(1, 64, 1, 7)); // chunk (0, 0)
        assert!(world.set_block(17, 64, 1, 8)); // chunk (1, 0)
        assert!(world.set_block(33, 64, 1, 9)); // chunk (2, 0) -> evicts (0, 0) to disk
        assert!(world.cached_chunk_count() <= 2);

        // Reload (0, 0) from disk and confirm dirty mutation was flushed on eviction
        assert_eq!(world.get_block(1, 64, 1), 7);
        assert!(world.cached_chunk_count() <= 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_or_foreign_mca_chunk_is_protected_from_overwrite() {
        let dir = std::env::temp_dir().join("rad_world_foreign_mca_test");
        let _ = std::fs::remove_dir_all(&dir);

        // Write foreign/corrupt zlib payload into chunk (0, 0)
        let foreign = b"foreign-anvil-nbt-payload";
        AnvilRegion::write_chunk_payload(&dir, 0, 0, foreign).unwrap();

        let regs = Arc::new(
            Version26_3Registries::from_provider(
                Arc::new(crate::registries::synthetic_26_3_test_provider()),
                true,
            )
            .unwrap(),
        );
        let mut world = WorldState::new_with_registries(Some(dir.clone()), regs);
        // Touching chunk (0, 0) marks it protected and refuses overwrite
        assert!(!world.set_block(2, 64, 2, 5));
        assert_eq!(world.save_dirty_chunks().unwrap(), 0);

        let preserved = AnvilRegion::read_chunk_payload_checked(&dir, 0, 0)
            .unwrap()
            .unwrap();
        assert_eq!(preserved.as_slice(), foreign);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn level_dat_gzip_nbt_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "rad_level_dat_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let meta = LevelMetadata {
            level_name: "survival_test".into(),
            random_seed: 424242,
            time: 1200,
            day_time: 6000,
            spawn_x: -10,
            spawn_y: 72,
            spawn_z: 35,
            game_type: 0,
            data_version: 4790,
        };
        meta.write_atomic(&dir).unwrap();
        let loaded = LevelMetadata::load_from_dir(&dir);
        assert_eq!(loaded, meta);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
