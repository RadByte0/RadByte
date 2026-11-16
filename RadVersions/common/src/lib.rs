//! Version descriptors, capabilities, voxel shapes, movement physics, and world
//! compatibility/backup safety for RadByte.
//!
//! Currently only **Minecraft Java 26.3 (`protocol 777`)** is supported. Future versions
//! must not be added as empty placeholders; each version requires verified packet layouts,
//! registries, mechanics, and tests.

#![warn(missing_docs)]

pub mod physics;
pub mod shapes;

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use rad_core::nbt::{NbtTag, decode_named_nbt};
use rad_core::version_package::RAD_VERSION_ABI_VERSION;
use rad_core::{MINECRAFT_VERSION, PROTOCOL_VERSION};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// World Anvil `DataVersion` for Minecraft Java 26.3.
pub const WORLD_DATA_VERSION_26_3: i32 = 4790;

/// Current RadByte world storage schema version (`radbyte_world_profile.json`).
pub const WORLD_STORAGE_SCHEMA_VERSION: u32 = 1;

/// Filename storing the explicit mechanics & data profile inside a world directory.
pub const WORLD_PROFILE_FILE: &str = "radbyte_world_profile.json";

/// Maximum compressed `level.dat` size read during compatibility inspection (`4 MiB`).
const MAX_LEVEL_DAT_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum uncompressed `level.dat` NBT payload (`16 MiB`).
const MAX_LEVEL_DAT_DECOMPRESSED_BYTES: u64 = 16 * 1024 * 1024;

/// Feature and protocol capabilities advertised by a [`VersionDescriptor`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionCapabilities {
    /// Supports Server List Ping (`Status`).
    pub status: bool,
    /// Supports `Login` handshake, UUID assignment, and encryption/compression.
    pub login: bool,
    /// Supports `Configuration` phase (known packs, registries, finish).
    pub configuration: bool,
    /// Supports `Play` state.
    pub play: bool,
    /// Supports Velocity modern forwarding.
    pub velocity_forwarding: bool,
    /// Minimum buildable Y coordinate in Overworld.
    pub min_world_y: i32,
    /// Total buildable height in Overworld.
    pub world_height: i32,
}

/// Authoritative descriptor separating game, protocol, registry/data, storage schema, and C ABI versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VersionDescriptor {
    /// Stable mechanics profile identifier (e.g. `"mc_26_3"`).
    pub profile_id: &'static str,
    /// Minecraft Java game version string (e.g. `"26.3"`).
    pub game_version: &'static str,
    /// Network protocol version (e.g. `777`).
    pub protocol_version: i32,
    /// World Anvil `DataVersion` (e.g. `4790`).
    pub world_data_version: i32,
    /// Registry/data snapshot version identifier.
    pub registry_version: &'static str,
    /// RadByte world storage schema version.
    pub storage_schema_version: u32,
    /// C ABI version for native version libraries.
    pub abi_version: u32,
    /// Supported protocol/gameplay capabilities.
    pub capabilities: VersionCapabilities,
}

/// Single supported version descriptor: Minecraft Java 26.3 (`protocol 777`).
pub const MC_26_3: VersionDescriptor = VersionDescriptor {
    profile_id: "mc_26_3",
    game_version: MINECRAFT_VERSION,
    protocol_version: PROTOCOL_VERSION,
    world_data_version: WORLD_DATA_VERSION_26_3,
    registry_version: "mc777",
    storage_schema_version: WORLD_STORAGE_SCHEMA_VERSION,
    abi_version: RAD_VERSION_ABI_VERSION,
    capabilities: VersionCapabilities {
        status: true,
        login: true,
        configuration: true,
        play: true,
        velocity_forwarding: true,
        min_world_y: -64,
        world_height: 384,
    },
};

static SUPPORTED_VERSIONS: [VersionDescriptor; 1] = [MC_26_3];

/// Returns all actually implemented and supported version descriptors.
#[must_use]
pub fn supported_versions() -> &'static [VersionDescriptor] {
    &SUPPORTED_VERSIONS
}

/// Finds a supported version descriptor by network `protocol_version`.
#[must_use]
pub fn find_by_protocol(protocol_version: i32) -> Option<&'static VersionDescriptor> {
    SUPPORTED_VERSIONS
        .iter()
        .find(|v| v.protocol_version == protocol_version)
}

/// Finds a supported version descriptor by Minecraft `game_version`.
#[must_use]
pub fn find_by_game_version(game_version: &str) -> Option<&'static VersionDescriptor> {
    SUPPORTED_VERSIONS
        .iter()
        .find(|v| v.game_version == game_version)
}

/// Persisted world mechanics & data profile (`<world>/radbyte_world_profile.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldMechanicsProfile {
    /// Mechanics profile identifier (e.g. `"mc_26_3"`).
    pub profile_id: String,
    /// Minecraft game version that created/last saved this world.
    pub game_version: String,
    /// Network protocol version associated with the mechanics profile.
    pub protocol_version: i32,
    /// Anvil `DataVersion` stored in world chunks/`level.dat`.
    pub world_data_version: i32,
    /// RadByte world storage schema version.
    pub storage_schema_version: u32,
}

impl WorldMechanicsProfile {
    /// Builds a profile matching `descriptor`.
    #[must_use]
    pub fn from_descriptor(descriptor: &VersionDescriptor) -> Self {
        Self {
            profile_id: descriptor.profile_id.to_owned(),
            game_version: descriptor.game_version.to_owned(),
            protocol_version: descriptor.protocol_version,
            world_data_version: descriptor.world_data_version,
            storage_schema_version: descriptor.storage_schema_version,
        }
    }

    /// Reads `<world_dir>/radbyte_world_profile.json` if present (rejects symlinks).
    pub fn load_if_present(world_dir: &Path) -> Result<Option<Self>, WorldProfileError> {
        let path = world_dir.join(WORLD_PROFILE_FILE);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(WorldProfileError::Corrupt(format!(
                        "{} must not be a symlink",
                        path.display()
                    )));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(WorldProfileError::Io(e)),
        }
        let raw = fs::read_to_string(&path)?;
        let parsed: Self = serde_json::from_str(&raw)?;
        Ok(Some(parsed))
    }

    /// Atomically writes `<world_dir>/radbyte_world_profile.json`.
    pub fn write_atomic(&self, world_dir: &Path) -> Result<(), WorldProfileError> {
        fs::create_dir_all(world_dir)?;
        let path = world_dir.join(WORLD_PROFILE_FILE);
        let tmp = world_dir.join(format!("{WORLD_PROFILE_FILE}.tmp"));
        let json = serde_json::to_string_pretty(self)?;
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Compatibility check result for opening a world directory with a target [`VersionDescriptor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorldCompatibility {
    /// World profile exists and matches the active version descriptor.
    Compatible,
    /// New or completely empty directory with no existing world files; safe to initialize.
    EmptyNewWorld,
    /// Unmarked legacy RadByte world whose `level.dat` `DataVersion` matches `target.world_data_version`.
    ConfirmedLegacyRadByteWorld,
    /// Unmarked directory containing existing files without sufficient metadata to confirm compatibility.
    UnmarkedInsufficientData {
        /// Explanation of what is missing/ambiguous.
        reason: String,
    },
    /// World belongs to a different or newer/incompatible mechanics profile, game/data version, or schema.
    Incompatible {
        /// Human-readable explanation of the mismatch.
        reason: String,
    },
    /// Metadata (`radbyte_world_profile.json` or `level.dat`) is corrupt or unreadable.
    CorruptMetadata {
        /// Human-readable error details.
        reason: String,
    },
}

/// Errors while reading, validating, or backing up a world profile.
#[derive(Debug, Error)]
pub enum WorldProfileError {
    /// Filesystem error.
    #[error("world profile I/O error: {0}")]
    Io(#[from] io::Error),
    /// Corrupt or unrecognized profile JSON.
    #[error("invalid world profile JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Corrupt or unsafe world metadata/path.
    #[error("corrupt or unsafe world metadata: {0}")]
    Corrupt(String),
    /// Unsafe backup source/destination path.
    #[error("unsafe backup path: {0}")]
    UnsafeBackupPath(String),
}

fn read_level_dat_data_version(level_dat: &Path) -> Result<Option<i32>, String> {
    let meta = fs::symlink_metadata(level_dat).map_err(|e| e.to_string())?;
    if meta.file_type().is_symlink() {
        return Err("level.dat must not be a symlink".to_owned());
    }
    if meta.len() > MAX_LEVEL_DAT_BYTES {
        return Err(format!("level.dat exceeds {} bytes", MAX_LEVEL_DAT_BYTES));
    }
    let file = File::open(level_dat).map_err(|e| e.to_string())?;
    let mut decoder = GzDecoder::new(file).take(MAX_LEVEL_DAT_DECOMPRESSED_BYTES + 1);
    let mut raw = Vec::new();
    decoder.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    if raw.len() as u64 > MAX_LEVEL_DAT_DECOMPRESSED_BYTES {
        return Err("decompressed level.dat exceeds size limit".to_owned());
    }
    let mut slice = raw.as_slice();
    let (_, tag) = decode_named_nbt(&mut slice).map_err(|e| e.to_string())?;
    let NbtTag::Compound(root) = tag else {
        return Err("level.dat root tag is not a Compound".to_owned());
    };
    let Some(NbtTag::Compound(data)) = root.get("Data") else {
        return Err("level.dat missing Data compound".to_owned());
    };
    match data.get("DataVersion") {
        Some(NbtTag::Int(dv)) => Ok(Some(*dv)),
        Some(_) => Err("level.dat Data.DataVersion is not Int".to_owned()),
        None => Ok(None),
    }
}

fn dir_has_any_entries(dir: &Path) -> io::Result<bool> {
    if !dir.exists() {
        return Ok(false);
    }
    Ok(fs::read_dir(dir)?.next().is_some())
}

/// Checks whether `world_dir` is compatible with `target` without mutating any files in `world_dir`.
pub fn check_world_compatibility(
    world_dir: &Path,
    target: &VersionDescriptor,
) -> Result<WorldCompatibility, WorldProfileError> {
    if let Ok(meta) = fs::symlink_metadata(world_dir)
        && meta.file_type().is_symlink()
    {
        return Ok(WorldCompatibility::CorruptMetadata {
            reason: format!(
                "world directory '{}' must not be a symlink",
                world_dir.display()
            ),
        });
    }

    let profile = match WorldMechanicsProfile::load_if_present(world_dir) {
        Ok(p) => p,
        Err(e) => {
            return Ok(WorldCompatibility::CorruptMetadata {
                reason: e.to_string(),
            });
        }
    };

    if let Some(profile) = profile {
        if profile.profile_id != target.profile_id
            || profile.game_version != target.game_version
            || profile.protocol_version != target.protocol_version
            || profile.world_data_version != target.world_data_version
        {
            return Ok(WorldCompatibility::Incompatible {
                reason: format!(
                    "world profile '{}' (game {}, protocol {}, data_version {}) does not match server '{}' (game {}, protocol {}, data_version {}); use an explicit migration tool on a backup copy",
                    profile.profile_id,
                    profile.game_version,
                    profile.protocol_version,
                    profile.world_data_version,
                    target.profile_id,
                    target.game_version,
                    target.protocol_version,
                    target.world_data_version,
                ),
            });
        }
        if profile.storage_schema_version > target.storage_schema_version {
            return Ok(WorldCompatibility::Incompatible {
                reason: format!(
                    "world storage schema {} is newer than supported schema {}; refusing unsafe downgrade",
                    profile.storage_schema_version, target.storage_schema_version
                ),
            });
        }
        return Ok(WorldCompatibility::Compatible);
    }

    // No radbyte_world_profile.json: distinguish empty new world vs unmarked existing world.
    if !dir_has_any_entries(world_dir)? {
        return Ok(WorldCompatibility::EmptyNewWorld);
    }

    let level_dat = world_dir.join("level.dat");
    if !level_dat.exists() {
        // Check if only empty standard subdirs exist (e.g. region/ and playerdata/ created before first save)
        let has_world_payload = [
            "region",
            "dimensions",
            "playerdata",
            "players",
            "DIM-1",
            "DIM1",
            "entities",
        ]
        .iter()
        .any(|sub| dir_has_any_entries(&world_dir.join(sub)).unwrap_or(true));
        if !has_world_payload
            && !world_dir.join("sim_queue.json").exists()
            && !world_dir.join("containers.json").exists()
        {
            return Ok(WorldCompatibility::EmptyNewWorld);
        }
        return Ok(WorldCompatibility::UnmarkedInsufficientData {
            reason: format!(
                "world directory '{}' contains existing data without '{}' or 'level.dat'; refusing to auto-stamp as {} ({})",
                world_dir.display(),
                WORLD_PROFILE_FILE,
                target.game_version,
                target.profile_id
            ),
        });
    }

    match read_level_dat_data_version(&level_dat) {
        Ok(Some(dv))
            if dv == target.world_data_version
                || (target.world_data_version == WORLD_DATA_VERSION_26_3 && dv == 5023) =>
        {
            Ok(WorldCompatibility::ConfirmedLegacyRadByteWorld)
        }
        Ok(Some(dv)) => Ok(WorldCompatibility::Incompatible {
            reason: format!(
                "unmarked world 'level.dat' DataVersion {} does not match required {} ({}); refusing silent conversion",
                dv, target.world_data_version, target.game_version
            ),
        }),
        Ok(None) => Ok(WorldCompatibility::UnmarkedInsufficientData {
            reason: format!(
                "unmarked world 'level.dat' has no DataVersion; cannot verify compatibility with {}",
                target.game_version
            ),
        }),
        Err(reason) => Ok(WorldCompatibility::CorruptMetadata {
            reason: format!("failed to inspect 'level.dat': {reason}"),
        }),
    }
}

fn normalize_for_comparison(path: &Path) -> Result<PathBuf, WorldProfileError> {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut existing_prefix = abs.as_path();
    let mut suffix_parts: Vec<std::ffi::OsString> = Vec::new();
    while !existing_prefix.exists() {
        let Some(name) = existing_prefix.file_name() else {
            break;
        };
        suffix_parts.push(name.to_os_string());
        let Some(parent) = existing_prefix.parent() else {
            break;
        };
        existing_prefix = parent;
    }
    let mut resolved = fs::canonicalize(existing_prefix)?;
    for part in suffix_parts.into_iter().rev() {
        resolved.push(part);
    }
    // Reject any unresolved `..` components
    if resolved.components().any(|c| c == Component::ParentDir) {
        return Err(WorldProfileError::UnsafeBackupPath(format!(
            "path '{}' contains parent traversal",
            path.display()
        )));
    }
    Ok(resolved)
}

fn ensure_no_symlinks_in_existing_prefix(path: &Path) -> Result<(), WorldProfileError> {
    let mut cur = PathBuf::new();
    for comp in path.components() {
        cur.push(comp);
        if matches!(comp, Component::Normal(_))
            && let Ok(meta) = fs::symlink_metadata(&cur)
            && meta.file_type().is_symlink()
        {
            return Err(WorldProfileError::UnsafeBackupPath(format!(
                "path component '{}' is a symlink",
                cur.display()
            )));
        }
    }
    Ok(())
}

/// Recursively copies `src_world` into an empty/new external `backup_dir`.
///
/// Refuses symlinks, identical source/destination, destination inside source,
/// source inside destination, and non-empty destinations to prevent data loss.
pub fn backup_world_dir(src_world: &Path, backup_dir: &Path) -> Result<PathBuf, WorldProfileError> {
    if !src_world.is_dir() {
        return Err(WorldProfileError::UnsafeBackupPath(format!(
            "source '{}' is not an existing directory",
            src_world.display()
        )));
    }
    ensure_no_symlinks_in_existing_prefix(src_world)?;
    ensure_no_symlinks_in_existing_prefix(backup_dir)?;

    let src_norm = normalize_for_comparison(src_world)?;
    let dst_norm = normalize_for_comparison(backup_dir)?;

    if src_norm == dst_norm {
        return Err(WorldProfileError::UnsafeBackupPath(
            "backup destination must not be the same as source world".to_owned(),
        ));
    }
    if dst_norm.starts_with(&src_norm) {
        return Err(WorldProfileError::UnsafeBackupPath(
            "backup destination must not be inside source world".to_owned(),
        ));
    }
    if src_norm.starts_with(&dst_norm) {
        return Err(WorldProfileError::UnsafeBackupPath(
            "backup destination must not be an ancestor of source world".to_owned(),
        ));
    }
    if dir_has_any_entries(&dst_norm)? {
        return Err(WorldProfileError::UnsafeBackupPath(format!(
            "backup destination '{}' already exists and is not empty",
            backup_dir.display()
        )));
    }

    copy_dir_recursive(&src_norm, &dst_norm)?;
    Ok(dst_norm)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), WorldProfileError> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.file_type().is_symlink() {
            return Err(WorldProfileError::UnsafeBackupPath(format!(
                "refusing to copy symlink '{}'",
                path.display()
            )));
        }
        let dst_path = dst.join(entry.file_name());
        if meta.is_dir() {
            copy_dir_recursive(&path, &dst_path)?;
        } else if meta.is_file() {
            fs::copy(&path, &dst_path)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use rad_core::nbt::encode_named_nbt;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "radbyte_radversion_safe_{name}_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_test_level_dat(world_dir: &Path, data_version: Option<i32>) {
        let mut data = BTreeMap::new();
        if let Some(dv) = data_version {
            data.insert("DataVersion".to_owned(), NbtTag::Int(dv));
        }
        data.insert(
            "LevelName".to_owned(),
            NbtTag::String("TestWorld".to_owned()),
        );
        let mut root = BTreeMap::new();
        root.insert("Data".to_owned(), NbtTag::Compound(data));
        let mut raw_nbt = Vec::new();
        encode_named_nbt("", &NbtTag::Compound(root), &mut raw_nbt).unwrap();
        let mut gz = GzEncoder::new(Vec::new(), Compression::default());
        io::Write::write_all(&mut gz, &raw_nbt).unwrap();
        let compressed = gz.finish().unwrap();
        fs::write(world_dir.join("level.dat"), compressed).unwrap();
    }

    #[test]
    fn mc_26_3_descriptor_and_lookup_work() {
        assert_eq!(supported_versions().len(), 1);
        assert_eq!(find_by_protocol(777), Some(&MC_26_3));
        assert_eq!(find_by_game_version("26.3"), Some(&MC_26_3));
        assert_eq!(find_by_protocol(776), None);
        assert_eq!(find_by_protocol(778), None);
        assert_eq!(find_by_game_version("26.4"), None);
        assert_eq!(MC_26_3.world_data_version, 4790);
        assert_eq!(MC_26_3.abi_version, 1);
    }

    #[test]
    fn world_compatibility_handles_all_states_without_mutating_source() {
        let empty = temp_dir("empty");
        assert_eq!(
            check_world_compatibility(&empty, &MC_26_3).unwrap(),
            WorldCompatibility::EmptyNewWorld
        );
        // Must not create files during compatibility check
        assert!(!empty.join(WORLD_PROFILE_FILE).exists());

        // Confirmed legacy RadByte world (level.dat with DataVersion 4790, no profile json)
        let legacy = temp_dir("legacy");
        write_test_level_dat(&legacy, Some(4790));
        assert_eq!(
            check_world_compatibility(&legacy, &MC_26_3).unwrap(),
            WorldCompatibility::ConfirmedLegacyRadByteWorld
        );
        assert!(!legacy.join(WORLD_PROFILE_FILE).exists());

        // Unmarked directory with region data but no level.dat -> UnmarkedInsufficientData
        let unmarked = temp_dir("unmarked");
        fs::create_dir_all(unmarked.join("region")).unwrap();
        fs::write(unmarked.join("region").join("r.0.0.mca"), b"dummy").unwrap();
        assert!(matches!(
            check_world_compatibility(&unmarked, &MC_26_3).unwrap(),
            WorldCompatibility::UnmarkedInsufficientData { .. }
        ));
        assert!(!unmarked.join(WORLD_PROFILE_FILE).exists());

        // Corrupt level.dat -> CorruptMetadata
        let corrupt = temp_dir("corrupt");
        fs::write(corrupt.join("level.dat"), b"not-gzip-nbt").unwrap();
        assert!(matches!(
            check_world_compatibility(&corrupt, &MC_26_3).unwrap(),
            WorldCompatibility::CorruptMetadata { .. }
        ));

        // Profile mismatches: game_version, protocol_version, world_data_version, storage_schema_version
        let prof_dir = temp_dir("mismatches");
        let base = WorldMechanicsProfile::from_descriptor(&MC_26_3);
        base.write_atomic(&prof_dir).unwrap();
        assert_eq!(
            check_world_compatibility(&prof_dir, &MC_26_3).unwrap(),
            WorldCompatibility::Compatible
        );

        let mut bad_game = base.clone();
        bad_game.game_version = "26.4".to_owned();
        bad_game.write_atomic(&prof_dir).unwrap();
        assert!(matches!(
            check_world_compatibility(&prof_dir, &MC_26_3).unwrap(),
            WorldCompatibility::Incompatible { .. }
        ));

        let mut bad_dv = base.clone();
        bad_dv.world_data_version = 4800;
        bad_dv.write_atomic(&prof_dir).unwrap();
        assert!(matches!(
            check_world_compatibility(&prof_dir, &MC_26_3).unwrap(),
            WorldCompatibility::Incompatible { .. }
        ));

        let mut newer_schema = base;
        newer_schema.storage_schema_version = 2;
        newer_schema.write_atomic(&prof_dir).unwrap();
        assert!(matches!(
            check_world_compatibility(&prof_dir, &MC_26_3).unwrap(),
            WorldCompatibility::Incompatible { .. }
        ));

        // Corrupt JSON
        fs::write(prof_dir.join(WORLD_PROFILE_FILE), b"{bad json").unwrap();
        assert!(matches!(
            check_world_compatibility(&prof_dir, &MC_26_3).unwrap(),
            WorldCompatibility::CorruptMetadata { .. }
        ));

        for d in [empty, legacy, unmarked, corrupt, prof_dir] {
            let _ = fs::remove_dir_all(d);
        }
    }

    #[test]
    fn backup_world_dir_rejects_unsafe_paths_and_restores_cleanly() {
        let src = temp_dir("bk_src");
        WorldMechanicsProfile::from_descriptor(&MC_26_3)
            .write_atomic(&src)
            .unwrap();
        write_test_level_dat(&src, Some(4790));

        // Same path rejected
        assert!(matches!(
            backup_world_dir(&src, &src),
            Err(WorldProfileError::UnsafeBackupPath(_))
        ));
        // Inside source rejected
        assert!(matches!(
            backup_world_dir(&src, &src.join("nested_backup")),
            Err(WorldProfileError::UnsafeBackupPath(_))
        ));
        // Ancestor of source rejected
        assert!(matches!(
            backup_world_dir(&src, src.parent().unwrap()),
            Err(WorldProfileError::UnsafeBackupPath(_))
        ));

        // Valid external backup succeeds
        let dst = temp_dir("bk_dst");
        let _ = fs::remove_dir_all(&dst);
        backup_world_dir(&src, &dst).unwrap();
        assert!(dst.join(WORLD_PROFILE_FILE).exists());
        assert!(dst.join("level.dat").exists());

        // Non-empty destination rejected on second backup
        assert!(matches!(
            backup_world_dir(&src, &dst),
            Err(WorldProfileError::UnsafeBackupPath(_))
        ));

        // Restore into fresh target recovers Compatible world
        let restored = temp_dir("bk_restored");
        let _ = fs::remove_dir_all(&restored);
        backup_world_dir(&dst, &restored).unwrap();
        assert_eq!(
            check_world_compatibility(&restored, &MC_26_3).unwrap(),
            WorldCompatibility::Compatible
        );

        let _ = fs::remove_dir_all(&src);
        let _ = fs::remove_dir_all(&dst);
        let _ = fs::remove_dir_all(&restored);
    }
}
