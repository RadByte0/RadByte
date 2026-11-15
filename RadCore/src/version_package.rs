//! Versioned Minecraft data package (`versions/<version>/version.json`) loader and validator.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::registry_provider::{RegistryTagSet, TagDefinition};

/// Current `version.json` manifest schema version.
pub const VERSION_PACKAGE_SCHEMA_VERSION: u32 = 1;
/// Current version-package data format version.
pub const VERSION_PACKAGE_FORMAT_VERSION: u32 = 1;
/// Current C ABI version for loadable native Minecraft version libraries (`versions/<ver>/*.dll|*.so|*.dylib`).
pub const RAD_VERSION_ABI_VERSION: u32 = 1;
/// Maximum allowed `version.json` file size (`256 KiB`).
pub const MAX_VERSION_MANIFEST_BYTES: u64 = 256 * 1024;
/// Maximum allowed individual data file size (`32 MiB`).
pub const MAX_VERSION_DATA_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// Maximum allowed native version library size (`256 MiB`).
pub const MAX_VERSION_LIBRARY_BYTES: u64 = 256 * 1024 * 1024;

/// C ABI status code: operation succeeded.
pub const RAD_VERSION_OK: i32 = 0;
/// C ABI status code: invalid argument or null pointer.
pub const RAD_VERSION_ERR_INVALID_ARG: i32 = -1;
/// C ABI status code: output buffer too small (`*out_len` receives required size).
pub const RAD_VERSION_ERR_BUFFER_TOO_SMALL: i32 = -2;
/// C ABI status code: requested entry or packet was not found.
pub const RAD_VERSION_ERR_NOT_FOUND: i32 = -3;
/// C ABI status code: internal runtime/server error.
pub const RAD_VERSION_ERR_INTERNAL: i32 = -4;

/// C ABI metadata returned by `rad_version_info` from a loaded version library.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RadVersionInfo {
    /// Struct size in bytes (`std::mem::size_of::<RadVersionInfo>()`), for ABI safety.
    pub struct_size: u32,
    /// C ABI version (`RAD_VERSION_ABI_VERSION`).
    pub abi_version: u32,
    /// Minecraft network protocol version (e.g. `777`).
    pub protocol_version: i32,
    /// World Anvil data version (e.g. `4790`).
    pub world_data_version: i32,
    /// NUL-terminated UTF-8 game version string (e.g. `"26.3"`).
    pub game_version: [u8; 32],
    /// NUL-terminated UTF-8 target OS (`std::env::consts::OS`).
    pub target_os: [u8; 32],
    /// NUL-terminated UTF-8 target architecture (`std::env::consts::ARCH`).
    pub target_arch: [u8; 32],
}

/// C ABI startup options passed by `rad-server` into `rad_version_run_server`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RadServerStartOptions {
    /// Struct size in bytes (`std::mem::size_of::<RadServerStartOptions>()`).
    pub struct_size: u32,
    /// C ABI version (`RAD_VERSION_ABI_VERSION`).
    pub abi_version: u32,
    /// UTF-8 pointer to `rad.properties` path (not required to be NUL-terminated).
    pub config_path_ptr: *const u8,
    /// Byte length of `config_path_ptr`.
    pub config_path_len: usize,
    /// UTF-8 pointer to `versions` root directory path.
    pub versions_dir_ptr: *const u8,
    /// Byte length of `versions_dir_ptr`.
    pub versions_dir_len: usize,
}

/// Metadata for the native dynamic library (`rad_version_<ver>.dll` / `.so` / `.dylib`) inside `version.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionLibraryMeta {
    /// Relative POSIX path inside the version directory (e.g. `"rad_version_26_3.dll"`).
    pub path: String,
    /// C ABI version exported by the library (must match `RAD_VERSION_ABI_VERSION`).
    pub abi_version: u32,
    /// Target OS (`"windows"`, `"linux"`, `"macos"`).
    pub target_os: String,
    /// Target architecture (`"x86_64"`, `"aarch64"`, etc.).
    pub target_arch: String,
    /// Exact byte size of the library file.
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 digest of the library file.
    pub sha256: String,
}

const REQUIRED_FILE_IDS: [&str; 4] = ["items", "blocks", "entities", "menus"];

/// Errors returned while validating or loading a version package.
#[derive(Debug)]
pub struct VersionPackageError {
    path: PathBuf,
    reason: String,
}

impl VersionPackageError {
    fn new(path: impl Into<PathBuf>, reason: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            reason: reason.into(),
        }
    }

    /// Returns the path associated with the failure.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl fmt::Display for VersionPackageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.reason)
    }
}

impl std::error::Error for VersionPackageError {}

/// Single file entry inside `version.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionPackageFile {
    /// Logical dataset identifier (`"items"`, `"blocks"`, `"entities"`, `"menus"`).
    pub id: String,
    /// Relative POSIX path inside the version directory (e.g. `"data/items.json"`).
    pub path: String,
    /// Exact byte size of the file.
    pub size_bytes: u64,
    /// Lowercase hex SHA-256 digest of the file.
    pub sha256: String,
    /// Expected number of registry entries in the file.
    pub entry_count: usize,
}

/// Manifest schema for `versions/<game_version>/version.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionManifest {
    /// Manifest schema version (must be `1`).
    pub schema_version: u32,
    /// Minecraft Java game version (e.g. `"26.3"`).
    pub game_version: String,
    /// Minecraft network protocol ID (e.g. `777`).
    pub protocol_version: i32,
    /// RadByte data-package format version (must be `1`).
    pub package_format_version: u32,
    /// World Anvil data version expected for this package.
    pub world_data_version: i32,
    /// Provenance source string (without secrets or PII).
    pub source: String,
    /// UTC generation timestamp.
    pub generated_utc: String,
    /// Optional human-readable notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Optional native executable version library (`rad_version_<ver>.dll` / `.so` / `.dylib`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub library: Option<VersionLibraryMeta>,
    /// Data files belonging to this version package.
    pub files: Vec<VersionPackageFile>,
}

/// Runtime block definition loaded from `data/blocks.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockPackageDef {
    /// Numeric block registry ID.
    pub block_id: u16,
    /// Namespaced block ID (e.g. `"minecraft:stone"`).
    pub name: String,
    /// Default block state ID.
    pub default_state: u16,
    /// Minimum block state ID.
    pub min_state: u16,
    /// Maximum block state ID.
    pub max_state: u16,
}

#[derive(Deserialize)]
struct ItemsJsonFile {
    entries: Vec<ItemJsonEntry>,
}

#[derive(Deserialize)]
struct ItemJsonEntry {
    name: String,
    id: i32,
}

#[derive(Deserialize)]
struct BlocksJsonFile {
    blocks: Vec<BlockJsonEntry>,
}

#[derive(Deserialize)]
struct BlockJsonEntry {
    block_id: u16,
    name: String,
    default_state: u16,
    min_state: u16,
    max_state: u16,
}

#[derive(Deserialize)]
struct EntitiesJsonFile {
    entries: Vec<NamedIdJsonEntry>,
}

#[derive(Deserialize)]
struct MenusJsonFile {
    entries: Vec<NamedIdJsonEntry>,
}

#[derive(Deserialize)]
struct NamedIdJsonEntry {
    id: String,
    id_num: i32,
}

#[derive(Deserialize)]
struct TagsJsonFile {
    #[serde(default)]
    block_tags: Vec<TagJsonEntry>,
    #[serde(default)]
    item_tags: Vec<TagJsonEntry>,
    #[serde(default)]
    core_registry_tags: Vec<CoreRegistryTagSetJson>,
}

#[derive(Deserialize)]
struct TagJsonEntry {
    name: String,
    members: Vec<String>,
}

#[derive(Deserialize)]
struct CoreRegistryTagSetJson {
    registry_id: String,
    tags: Vec<TagJsonEntry>,
}

/// Validated runtime Minecraft version package loaded from `versions/<version>/`.
#[derive(Debug, Clone)]
pub struct LoadedVersionPackage {
    manifest: VersionManifest,
    package_dir: PathBuf,
    items_by_name: HashMap<String, i32>,
    item_names_by_id: HashMap<i32, String>,
    blocks_by_name: HashMap<String, BlockPackageDef>,
    blocks_by_state: Vec<BlockPackageDef>,
    entities_by_name: HashMap<String, i32>,
    menus_by_name: HashMap<String, i32>,
    block_tags: Option<Vec<TagDefinition>>,
    item_tags: Option<Vec<TagDefinition>>,
    core_registry_tags: Option<Vec<RegistryTagSet>>,
}

impl LoadedVersionPackage {
    /// Loads and validates `<versions_root>/<version>/version.json` and all referenced data files.
    pub fn load_from_versions_dir(
        versions_root: &Path,
        version: &str,
    ) -> Result<Self, VersionPackageError> {
        if !is_safe_version_segment(version) {
            return Err(VersionPackageError::new(
                versions_root,
                format!("unsafe version directory name '{version}'"),
            ));
        }
        ensure_not_symlink(versions_root, "versions directory")?;
        let canonical_root = std::fs::canonicalize(versions_root).map_err(|e| {
            VersionPackageError::new(
                versions_root,
                format!("cannot resolve versions directory: {e}"),
            )
        })?;
        let package_dir = versions_root.join(version);
        let pkg = Self::load_from_package_dir(&package_dir)?;
        let canonical_pkg = std::fs::canonicalize(&package_dir).map_err(|e| {
            VersionPackageError::new(
                &package_dir,
                format!("cannot resolve package directory: {e}"),
            )
        })?;
        if !canonical_pkg.starts_with(&canonical_root) {
            return Err(VersionPackageError::new(
                &package_dir,
                "package directory escapes versions root",
            ));
        }
        if pkg.manifest.game_version != version {
            return Err(VersionPackageError::new(
                package_dir.join("version.json"),
                format!(
                    "manifest game_version '{}' does not match requested version '{version}'",
                    pkg.manifest.game_version
                ),
            ));
        }
        Ok(pkg)
    }

    /// Loads and validates `<package_dir>/version.json` and all referenced data files.
    pub fn load_from_package_dir(package_dir: &Path) -> Result<Self, VersionPackageError> {
        ensure_not_symlink(package_dir, "version package directory")?;
        let canonical_pkg = std::fs::canonicalize(package_dir).map_err(|e| {
            VersionPackageError::new(
                package_dir,
                format!("cannot resolve package directory: {e}"),
            )
        })?;
        let manifest_path = package_dir.join("version.json");
        ensure_not_symlink(&manifest_path, "version.json")?;
        let manifest_bytes =
            read_bounded_file(&manifest_path, MAX_VERSION_MANIFEST_BYTES, "version.json")?;
        let manifest: VersionManifest = serde_json::from_slice(&manifest_bytes).map_err(|e| {
            VersionPackageError::new(&manifest_path, format!("invalid version.json: {e}"))
        })?;

        validate_manifest_header(&manifest, &manifest_path)?;

        let mut seen_ids = HashSet::new();
        let mut seen_paths = HashSet::new();
        let mut resolved_files: HashMap<String, (PathBuf, VersionPackageFile)> = HashMap::new();

        for entry in &manifest.files {
            if !seen_ids.insert(entry.id.as_str()) {
                return Err(VersionPackageError::new(
                    &manifest_path,
                    format!("duplicate file id '{}'", entry.id),
                ));
            }
            validate_relative_data_path(&entry.path, &manifest_path)?;
            if !seen_paths.insert(entry.path.as_str()) {
                return Err(VersionPackageError::new(
                    &manifest_path,
                    format!("duplicate file path '{}'", entry.path),
                ));
            }
            if !is_valid_sha256_hex(&entry.sha256) {
                return Err(VersionPackageError::new(
                    &manifest_path,
                    format!("invalid sha256 digest for '{}'", entry.id),
                ));
            }
            if entry.size_bytes > MAX_VERSION_DATA_FILE_BYTES {
                return Err(VersionPackageError::new(
                    &manifest_path,
                    format!(
                        "file '{}' declared size {} exceeds limit {}",
                        entry.path, entry.size_bytes, MAX_VERSION_DATA_FILE_BYTES
                    ),
                ));
            }
            let full_path = package_dir.join(&entry.path);
            ensure_not_symlink(&full_path, &entry.path)?;
            let canonical_file = std::fs::canonicalize(&full_path).map_err(|e| {
                VersionPackageError::new(
                    &full_path,
                    format!("cannot resolve '{}': {e}", entry.path),
                )
            })?;
            if !canonical_file.starts_with(&canonical_pkg) {
                return Err(VersionPackageError::new(
                    &full_path,
                    format!("data file '{}' escapes package directory", entry.path),
                ));
            }
            resolved_files.insert(entry.id.clone(), (full_path, entry.clone()));
        }

        for req in REQUIRED_FILE_IDS {
            if !resolved_files.contains_key(req) {
                return Err(VersionPackageError::new(
                    &manifest_path,
                    format!("missing required dataset '{req}' in version.json"),
                ));
            }
        }

        let items_bytes =
            read_and_verify_data_file(&resolved_files["items"].0, &resolved_files["items"].1)?;
        let blocks_bytes =
            read_and_verify_data_file(&resolved_files["blocks"].0, &resolved_files["blocks"].1)?;
        let entities_bytes = read_and_verify_data_file(
            &resolved_files["entities"].0,
            &resolved_files["entities"].1,
        )?;
        let menus_bytes =
            read_and_verify_data_file(&resolved_files["menus"].0, &resolved_files["menus"].1)?;

        let items_json: ItemsJsonFile = serde_json::from_slice(&items_bytes).map_err(|e| {
            VersionPackageError::new(&resolved_files["items"].0, format!("invalid JSON: {e}"))
        })?;
        let blocks_json: BlocksJsonFile = serde_json::from_slice(&blocks_bytes).map_err(|e| {
            VersionPackageError::new(&resolved_files["blocks"].0, format!("invalid JSON: {e}"))
        })?;
        let entities_json: EntitiesJsonFile =
            serde_json::from_slice(&entities_bytes).map_err(|e| {
                VersionPackageError::new(
                    &resolved_files["entities"].0,
                    format!("invalid JSON: {e}"),
                )
            })?;
        let menus_json: MenusJsonFile = serde_json::from_slice(&menus_bytes).map_err(|e| {
            VersionPackageError::new(&resolved_files["menus"].0, format!("invalid JSON: {e}"))
        })?;

        let (items_by_name, item_names_by_id) = parse_items(
            items_json,
            &resolved_files["items"].0,
            resolved_files["items"].1.entry_count,
        )?;
        let blocks_by_name = parse_blocks(
            blocks_json,
            &resolved_files["blocks"].0,
            resolved_files["blocks"].1.entry_count,
        )?;
        let entities_by_name = parse_named_ids(
            entities_json.entries,
            &resolved_files["entities"].0,
            resolved_files["entities"].1.entry_count,
            "entity",
        )?;
        let menus_by_name = parse_named_ids(
            menus_json.entries,
            &resolved_files["menus"].0,
            resolved_files["menus"].1.entry_count,
            "menu",
        )?;

        let (block_tags, item_tags, core_registry_tags) = if let Some((tags_path, tags_meta)) =
            resolved_files.get("tags")
        {
            let tags_bytes = read_and_verify_data_file(tags_path, tags_meta)?;
            let tags_json: TagsJsonFile = serde_json::from_slice(&tags_bytes)
                .map_err(|e| VersionPackageError::new(tags_path, format!("invalid JSON: {e}")))?;
            let (bt, it, ct) = parse_tags(
                tags_json,
                tags_path,
                tags_meta.entry_count,
                &blocks_by_name,
                &items_by_name,
            )?;
            (Some(bt), Some(it), Some(ct))
        } else {
            (None, None, None)
        };

        let mut blocks_by_state: Vec<BlockPackageDef> = blocks_by_name.values().cloned().collect();
        blocks_by_state.sort_unstable_by_key(|b| b.min_state);
        Ok(Self {
            manifest,
            package_dir: package_dir.to_path_buf(),
            items_by_name,
            item_names_by_id,
            blocks_by_name,
            blocks_by_state,
            entities_by_name,
            menus_by_name,
            block_tags,
            item_tags,
            core_registry_tags,
        })
    }

    /// Returns the loaded manifest.
    #[must_use]
    pub fn manifest(&self) -> &VersionManifest {
        &self.manifest
    }

    /// Returns the package directory path.
    #[must_use]
    pub fn package_dir(&self) -> &Path {
        &self.package_dir
    }

    /// Validates and resolves the native version library (`rad_version_<ver>.dll` / `.so` / `.dylib`).
    pub fn verify_and_resolve_library_path(&self) -> Result<PathBuf, VersionPackageError> {
        let manifest_path = self.package_dir.join("version.json");
        let lib = self.manifest.library.as_ref().ok_or_else(|| {
            VersionPackageError::new(
                &manifest_path,
                "version.json is missing required 'library' entry for native version module",
            )
        })?;
        if lib.abi_version != RAD_VERSION_ABI_VERSION {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!(
                    "unsupported library abi_version {} (expected {})",
                    lib.abi_version, RAD_VERSION_ABI_VERSION
                ),
            ));
        }
        if lib.target_os != std::env::consts::OS {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!(
                    "library target_os '{}' does not match host '{}'",
                    lib.target_os,
                    std::env::consts::OS
                ),
            ));
        }
        if lib.target_arch != std::env::consts::ARCH {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!(
                    "library target_arch '{}' does not match host '{}'",
                    lib.target_arch,
                    std::env::consts::ARCH
                ),
            ));
        }
        if !is_safe_relative_library_path(&lib.path) {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!("unsafe or invalid library path '{}'", lib.path),
            ));
        }
        if !is_valid_sha256_hex(&lib.sha256) {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!("invalid library sha256 '{}'", lib.sha256),
            ));
        }
        if lib.size_bytes == 0 || lib.size_bytes > MAX_VERSION_LIBRARY_BYTES {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!(
                    "library size_bytes {} outside allowed bounds (1..={})",
                    lib.size_bytes, MAX_VERSION_LIBRARY_BYTES
                ),
            ));
        }

        let expected_ext = match lib.target_os.as_str() {
            "windows" => ".dll",
            "linux" => ".so",
            "macos" => ".dylib",
            other => {
                return Err(VersionPackageError::new(
                    &manifest_path,
                    format!("unsupported library target_os '{other}'"),
                ));
            }
        };
        if !lib.path.ends_with(expected_ext) {
            return Err(VersionPackageError::new(
                &manifest_path,
                format!(
                    "library '{}' must end with '{expected_ext}' for target_os '{}'",
                    lib.path, lib.target_os
                ),
            ));
        }

        let canonical_pkg = std::fs::canonicalize(&self.package_dir).map_err(|e| {
            VersionPackageError::new(
                &self.package_dir,
                format!("cannot resolve package directory: {e}"),
            )
        })?;
        let lib_path = self.package_dir.join(&lib.path);
        ensure_not_symlink(&lib_path, "native version library")?;
        let canonical_lib = std::fs::canonicalize(&lib_path).map_err(|e| {
            VersionPackageError::new(
                &lib_path,
                format!("failed to resolve native version library: {e}"),
            )
        })?;
        if !canonical_lib.starts_with(&canonical_pkg) {
            return Err(VersionPackageError::new(
                &lib_path,
                "native version library escapes package directory",
            ));
        }
        let mut file = File::open(&lib_path).map_err(|e| {
            VersionPackageError::new(
                &lib_path,
                format!("failed to open native version library: {e}"),
            )
        })?;
        let meta = file.metadata().map_err(|e| {
            VersionPackageError::new(
                &lib_path,
                format!("failed to stat native version library: {e}"),
            )
        })?;
        if meta.len() != lib.size_bytes {
            return Err(VersionPackageError::new(
                &lib_path,
                format!(
                    "library size mismatch: expected {} bytes, found {} bytes",
                    lib.size_bytes,
                    meta.len()
                ),
            ));
        }

        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).map_err(|e| {
                VersionPackageError::new(
                    &lib_path,
                    format!("failed reading native version library: {e}"),
                )
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual_sha256 = format!("{:x}", hasher.finalize());
        if actual_sha256 != lib.sha256 {
            return Err(VersionPackageError::new(
                &lib_path,
                format!(
                    "library sha256 mismatch: expected {}, got {}",
                    lib.sha256, actual_sha256
                ),
            ));
        }

        Ok(lib_path)
    }

    /// Returns the game version string (e.g. `"26.3"`).
    #[must_use]
    pub fn game_version(&self) -> &str {
        &self.manifest.game_version
    }

    /// Returns the protocol version number (e.g. `777`).
    #[must_use]
    pub fn protocol_version(&self) -> i32 {
        self.manifest.protocol_version
    }

    /// Resolves a namespaced or short item name (e.g. `"stone"` or `"minecraft:stone"`) to its numeric ID.
    #[must_use]
    pub fn item_id_by_name(&self, raw: &str) -> Option<i32> {
        let key = normalize_namespaced(raw)?;
        self.items_by_name.get(&key).copied()
    }

    /// Resolves a numeric item ID to its namespaced name.
    #[must_use]
    pub fn item_name_by_id(&self, id: i32) -> Option<&str> {
        self.item_names_by_id.get(&id).map(String::as_str)
    }

    /// Looks up a block definition by namespaced or short name.
    #[must_use]
    pub fn block_by_name(&self, raw: &str) -> Option<&BlockPackageDef> {
        let key = normalize_namespaced(raw)?;
        self.blocks_by_name.get(&key)
    }

    /// Resolves a block state ID to its namespaced block name.
    #[must_use]
    pub fn block_name_by_state(&self, state_id: u16) -> Option<&str> {
        let idx = self
            .blocks_by_state
            .partition_point(|b| b.min_state <= state_id);
        let b = self.blocks_by_state.get(idx.checked_sub(1)?)?;
        (state_id <= b.max_state).then_some(b.name.as_str())
    }

    /// Resolves an entity type ID by namespaced or short name.
    #[must_use]
    pub fn entity_id_by_name(&self, raw: &str) -> Option<i32> {
        let key = normalize_namespaced(raw)?;
        self.entities_by_name.get(&key).copied()
    }

    /// Resolves a menu type ID by namespaced or short name.
    #[must_use]
    pub fn menu_id_by_name(&self, raw: &str) -> Option<i32> {
        let key = normalize_namespaced(raw)?;
        self.menus_by_name.get(&key).copied()
    }

    /// Returns packaged block tags if `data/tags.json` was provided.
    #[must_use]
    pub fn block_tags(&self) -> Option<&[TagDefinition]> {
        self.block_tags.as_deref()
    }

    /// Returns packaged item tags if `data/tags.json` was provided.
    #[must_use]
    pub fn item_tags(&self) -> Option<&[TagDefinition]> {
        self.item_tags.as_deref()
    }

    /// Returns packaged core registry tags if `data/tags.json` was provided.
    #[must_use]
    pub fn core_registry_tags(&self) -> Option<&[RegistryTagSet]> {
        self.core_registry_tags.as_deref()
    }
}

fn normalize_namespaced(raw: &str) -> Option<String> {
    let trimmed = raw.trim().to_ascii_lowercase();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains(':') {
        Some(trimmed)
    } else {
        Some(format!("minecraft:{trimmed}"))
    }
}

fn is_safe_version_segment(seg: &str) -> bool {
    !seg.is_empty()
        && seg != "."
        && seg != ".."
        && !seg.contains("..")
        && !seg.contains('/')
        && !seg.contains('\\')
        && !seg.contains(':')
        && !seg.contains('\0')
}

fn is_safe_relative_library_path(rel: &str) -> bool {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains("..")
        || rel.contains('\\')
        || rel.contains(':')
        || rel.contains('\0')
    {
        return false;
    }
    rel.ends_with(".dll") || rel.ends_with(".so") || rel.ends_with(".dylib")
}

fn validate_relative_data_path(rel: &str, manifest_path: &Path) -> Result<(), VersionPackageError> {
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.starts_with('\\')
        || rel.contains("..")
        || rel.contains('\\')
        || rel.contains(':')
        || rel.contains('\0')
        || !rel.starts_with("data/")
    {
        return Err(VersionPackageError::new(
            manifest_path,
            format!("unsafe or non-relative data path '{rel}'"),
        ));
    }
    Ok(())
}

fn ensure_not_symlink(path: &Path, label: &str) -> Result<(), VersionPackageError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.file_type().is_symlink() {
                return Err(VersionPackageError::new(
                    path,
                    format!("{label} must not be a symlink"),
                ));
            }
            Ok(())
        }
        Err(e) => Err(VersionPackageError::new(
            path,
            format!("cannot access {label}: {e}"),
        )),
    }
}

fn validate_manifest_header(
    manifest: &VersionManifest,
    manifest_path: &Path,
) -> Result<(), VersionPackageError> {
    if manifest.schema_version != VERSION_PACKAGE_SCHEMA_VERSION {
        return Err(VersionPackageError::new(
            manifest_path,
            format!(
                "unsupported schema_version {} (expected {})",
                manifest.schema_version, VERSION_PACKAGE_SCHEMA_VERSION
            ),
        ));
    }
    if manifest.package_format_version != VERSION_PACKAGE_FORMAT_VERSION {
        return Err(VersionPackageError::new(
            manifest_path,
            format!(
                "unsupported package_format_version {} (expected {})",
                manifest.package_format_version, VERSION_PACKAGE_FORMAT_VERSION
            ),
        ));
    }
    if !is_safe_version_segment(&manifest.game_version) {
        return Err(VersionPackageError::new(
            manifest_path,
            format!(
                "invalid game_version '{}' in version.json",
                manifest.game_version
            ),
        ));
    }
    if manifest.protocol_version <= 0 {
        return Err(VersionPackageError::new(
            manifest_path,
            format!(
                "invalid protocol_version {} in version.json",
                manifest.protocol_version
            ),
        ));
    }
    if manifest.world_data_version <= 0 {
        return Err(VersionPackageError::new(
            manifest_path,
            format!(
                "invalid world_data_version {} in version.json",
                manifest.world_data_version
            ),
        ));
    }
    if manifest.source.trim().is_empty() {
        return Err(VersionPackageError::new(
            manifest_path,
            "missing provenance source in version.json",
        ));
    }
    Ok(())
}

fn is_valid_sha256_hex(hex: &str) -> bool {
    hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn read_bounded_file(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, VersionPackageError> {
    let meta = std::fs::metadata(path)
        .map_err(|e| VersionPackageError::new(path, format!("cannot access {label}: {e}")))?;
    if !meta.is_file() {
        return Err(VersionPackageError::new(
            path,
            format!("{label} is not a regular file"),
        ));
    }
    if meta.len() > max_bytes {
        return Err(VersionPackageError::new(
            path,
            format!(
                "{label} size {} exceeds maximum {max_bytes} bytes",
                meta.len()
            ),
        ));
    }
    let file = File::open(path)
        .map_err(|e| VersionPackageError::new(path, format!("failed to open {label}: {e}")))?;
    let mut buf = Vec::with_capacity(meta.len() as usize);
    file.take(max_bytes + 1)
        .read_to_end(&mut buf)
        .map_err(|e| VersionPackageError::new(path, format!("failed to read {label}: {e}")))?;
    if buf.len() as u64 > max_bytes {
        return Err(VersionPackageError::new(
            path,
            format!("{label} exceeds maximum {max_bytes} bytes"),
        ));
    }
    Ok(buf)
}

fn read_and_verify_data_file(
    path: &Path,
    entry: &VersionPackageFile,
) -> Result<Vec<u8>, VersionPackageError> {
    let bytes = read_bounded_file(path, MAX_VERSION_DATA_FILE_BYTES, &entry.path)?;
    if bytes.len() as u64 != entry.size_bytes {
        return Err(VersionPackageError::new(
            path,
            format!(
                "size mismatch for '{}': expected {} bytes, got {} bytes",
                entry.path,
                entry.size_bytes,
                bytes.len()
            ),
        ));
    }
    let digest = Sha256::digest(&bytes);
    let actual_hex = format!("{digest:x}");
    if actual_hex != entry.sha256 {
        return Err(VersionPackageError::new(
            path,
            format!(
                "sha256 mismatch for '{}': expected {}, got {actual_hex}",
                entry.path, entry.sha256
            ),
        ));
    }
    Ok(bytes)
}

type ItemMaps = (HashMap<String, i32>, HashMap<i32, String>);

fn parse_items(
    file: ItemsJsonFile,
    path: &Path,
    expected_count: usize,
) -> Result<ItemMaps, VersionPackageError> {
    if file.entries.len() != expected_count {
        return Err(VersionPackageError::new(
            path,
            format!(
                "item entry_count mismatch: manifest={}, actual={}",
                expected_count,
                file.entries.len()
            ),
        ));
    }
    let mut by_name = HashMap::with_capacity(file.entries.len());
    let mut by_id = HashMap::with_capacity(file.entries.len());
    for entry in file.entries {
        if entry.name.trim().is_empty() || entry.id < 0 {
            return Err(VersionPackageError::new(
                path,
                format!("invalid item entry '{}' ({})", entry.name, entry.id),
            ));
        }
        if by_name.insert(entry.name.clone(), entry.id).is_some() {
            return Err(VersionPackageError::new(
                path,
                format!("duplicate item name '{}'", entry.name),
            ));
        }
        if by_id.insert(entry.id, entry.name).is_some() {
            return Err(VersionPackageError::new(
                path,
                format!("duplicate item id {}", entry.id),
            ));
        }
    }
    Ok((by_name, by_id))
}

fn parse_blocks(
    file: BlocksJsonFile,
    path: &Path,
    expected_count: usize,
) -> Result<HashMap<String, BlockPackageDef>, VersionPackageError> {
    if file.blocks.len() != expected_count {
        return Err(VersionPackageError::new(
            path,
            format!(
                "block entry_count mismatch: manifest={}, actual={}",
                expected_count,
                file.blocks.len()
            ),
        ));
    }
    let mut by_name = HashMap::with_capacity(file.blocks.len());
    let mut seen_ids = HashSet::with_capacity(file.blocks.len());
    for entry in file.blocks {
        if entry.name.trim().is_empty()
            || entry.min_state > entry.default_state
            || entry.default_state > entry.max_state
        {
            return Err(VersionPackageError::new(
                path,
                format!("invalid block entry '{}'", entry.name),
            ));
        }
        if !seen_ids.insert(entry.block_id) {
            return Err(VersionPackageError::new(
                path,
                format!(
                    "duplicate block_id {} in blocks.json ('{}')",
                    entry.block_id, entry.name
                ),
            ));
        }
        if by_name
            .insert(
                entry.name.clone(),
                BlockPackageDef {
                    block_id: entry.block_id,
                    name: entry.name,
                    default_state: entry.default_state,
                    min_state: entry.min_state,
                    max_state: entry.max_state,
                },
            )
            .is_some()
        {
            return Err(VersionPackageError::new(
                path,
                "duplicate block name in blocks.json",
            ));
        }
    }
    let mut ranges: Vec<_> = by_name.values().collect();
    ranges.sort_unstable_by_key(|block| block.min_state);
    for pair in ranges.windows(2) {
        if pair[1].min_state <= pair[0].max_state {
            return Err(VersionPackageError::new(
                path,
                format!(
                    "overlapping block state ranges for '{}' and '{}'",
                    pair[0].name, pair[1].name
                ),
            ));
        }
    }
    Ok(by_name)
}

fn parse_named_ids(
    entries: Vec<NamedIdJsonEntry>,
    path: &Path,
    expected_count: usize,
    kind: &str,
) -> Result<HashMap<String, i32>, VersionPackageError> {
    if entries.len() != expected_count {
        return Err(VersionPackageError::new(
            path,
            format!(
                "{kind} entry_count mismatch: manifest={}, actual={}",
                expected_count,
                entries.len()
            ),
        ));
    }
    let mut by_name = HashMap::with_capacity(entries.len());
    let mut seen_ids = HashSet::with_capacity(entries.len());
    for entry in entries {
        if entry.id.trim().is_empty() || entry.id_num < 0 {
            return Err(VersionPackageError::new(
                path,
                format!("invalid {kind} entry '{}' ({})", entry.id, entry.id_num),
            ));
        }
        if by_name.insert(entry.id.clone(), entry.id_num).is_some() {
            return Err(VersionPackageError::new(
                path,
                format!("duplicate {kind} name '{}'", entry.id),
            ));
        }
        if !seen_ids.insert(entry.id_num) {
            return Err(VersionPackageError::new(
                path,
                format!("duplicate {kind} numeric id {}", entry.id_num),
            ));
        }
    }
    Ok(by_name)
}

type ParsedTags = (Vec<TagDefinition>, Vec<TagDefinition>, Vec<RegistryTagSet>);

fn parse_tags(
    file: TagsJsonFile,
    path: &Path,
    expected_count: usize,
    blocks_by_name: &HashMap<String, BlockPackageDef>,
    items_by_name: &HashMap<String, i32>,
) -> Result<ParsedTags, VersionPackageError> {
    let actual_count = file.block_tags.len()
        + file.item_tags.len()
        + file
            .core_registry_tags
            .iter()
            .map(|r| r.tags.len())
            .sum::<usize>();
    if actual_count != expected_count {
        return Err(VersionPackageError::new(
            path,
            format!("tags entry_count mismatch: manifest={expected_count}, actual={actual_count}"),
        ));
    }

    let mut block_tags = Vec::with_capacity(file.block_tags.len());
    for t in file.block_tags {
        if t.name.trim().is_empty() {
            return Err(VersionPackageError::new(path, "empty block tag name"));
        }
        for member in &t.members {
            if !blocks_by_name.contains_key(member) {
                return Err(VersionPackageError::new(
                    path,
                    format!("block tag '{}' references unknown block '{member}'", t.name),
                ));
            }
        }
        block_tags.push(TagDefinition {
            name: t.name,
            members: t.members,
        });
    }

    let mut item_tags = Vec::with_capacity(file.item_tags.len());
    for t in file.item_tags {
        if t.name.trim().is_empty() {
            return Err(VersionPackageError::new(path, "empty item tag name"));
        }
        for member in &t.members {
            if !items_by_name.contains_key(member) {
                return Err(VersionPackageError::new(
                    path,
                    format!("item tag '{}' references unknown item '{member}'", t.name),
                ));
            }
        }
        item_tags.push(TagDefinition {
            name: t.name,
            members: t.members,
        });
    }

    let mut core_tags = Vec::with_capacity(file.core_registry_tags.len());
    for set in file.core_registry_tags {
        if set.registry_id.trim().is_empty() {
            return Err(VersionPackageError::new(path, "empty registry_id in tags"));
        }
        let mut tags = Vec::with_capacity(set.tags.len());
        for t in set.tags {
            if t.name.trim().is_empty() {
                return Err(VersionPackageError::new(path, "empty core tag name"));
            }
            tags.push(TagDefinition {
                name: t.name,
                members: t.members,
            });
        }
        core_tags.push(RegistryTagSet {
            registry_id: set.registry_id,
            tags,
        });
    }

    Ok((block_tags, item_tags, core_tags))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_overlapping_block_state_ranges() {
        let file: BlocksJsonFile = serde_json::from_str(
            r#"{"blocks":[{"block_id":0,"name":"test:a","default_state":0,"min_state":0,"max_state":2},{"block_id":1,"name":"test:b","default_state":2,"min_state":2,"max_state":3}]}"#,
        ).unwrap();
        assert!(parse_blocks(file, Path::new("synthetic-blocks.json"), 2).is_err());
    }

    #[test]
    fn rejects_duplicate_block_id() {
        let file: BlocksJsonFile = serde_json::from_str(
            r#"{"blocks":[{"block_id":0,"name":"test:a","default_state":0,"min_state":0,"max_state":0},{"block_id":0,"name":"test:b","default_state":1,"min_state":1,"max_state":1}]}"#,
        ).unwrap();
        assert!(parse_blocks(file, Path::new("synthetic-blocks.json"), 2).is_err());
    }

    #[test]
    fn accepts_adjacent_block_state_ranges() {
        let file: BlocksJsonFile = serde_json::from_str(
            r#"{"blocks":[{"block_id":0,"name":"test:a","default_state":0,"min_state":0,"max_state":1},{"block_id":1,"name":"test:b","default_state":2,"min_state":2,"max_state":3}]}"#,
        ).unwrap();
        assert!(parse_blocks(file, Path::new("synthetic-blocks.json"), 2).is_ok());
    }

    #[test]
    fn loads_synthetic_package_via_registry_provider_and_rejects_missing_or_corrupt() {
        use crate::registry_provider::{LoadedPackageRegistryProvider, RegistryProvider};

        let root =
            std::env::temp_dir().join(format!("rad_verpkg_synth_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let pkg_dir = root.join("versions/0.0-synth");
        let data_dir = pkg_dir.join("data");
        std::fs::create_dir_all(&data_dir).unwrap();

        let items_json =
            br#"{"entries":[{"id":0,"name":"test:air"},{"id":1,"name":"test:stone"}]}"#;
        let blocks_json = br#"{"blocks":[{"block_id":0,"name":"test:air","default_state":0,"min_state":0,"max_state":0},{"block_id":1,"name":"test:stone","default_state":1,"min_state":1,"max_state":1}]}"#;
        let entities_json = br#"{"entries":[{"id":"test:pig","id_num":1}]}"#;
        let menus_json = br#"{"entries":[{"id":"test:menu","id_num":0}]}"#;

        let files_spec: [(&str, &str, &[u8], usize); 4] = [
            ("items", "data/items.json", items_json, 2),
            ("blocks", "data/blocks.json", blocks_json, 2),
            ("entities", "data/entities.json", entities_json, 1),
            ("menus", "data/menus.json", menus_json, 1),
        ];

        let mut files = Vec::new();
        for (id, rel, bytes, count) in files_spec {
            std::fs::write(pkg_dir.join(rel), bytes).unwrap();
            files.push(VersionPackageFile {
                id: id.to_owned(),
                path: rel.to_owned(),
                size_bytes: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
                entry_count: count,
            });
        }
        let manifest = VersionManifest {
            schema_version: VERSION_PACKAGE_SCHEMA_VERSION,
            game_version: "0.0-synth".to_owned(),
            protocol_version: 999,
            package_format_version: VERSION_PACKAGE_FORMAT_VERSION,
            world_data_version: 1,
            source: "synthetic-unit-test-fixture".to_owned(),
            generated_utc: "2026-11-15T00:00:00Z".to_owned(),
            notes: Some("Synthetic unit test fixture; not Minecraft data".to_owned()),
            library: None,
            files,
        };
        std::fs::write(
            pkg_dir.join("version.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let loaded =
            LoadedVersionPackage::load_from_versions_dir(&root.join("versions"), "0.0-synth")
                .expect("valid synthetic package must load");
        let provider = LoadedPackageRegistryProvider::new(&loaded);
        assert_eq!(provider.item_by_name("test:stone"), Some(1));
        assert_eq!(provider.block_by_state(1).unwrap().block_id, 1);
        assert_eq!(provider.block_by_state(1).unwrap().name, "test:stone");
        assert!(provider.block_tags().is_err());

        // Missing directory must fail cleanly
        assert!(
            LoadedVersionPackage::load_from_versions_dir(&root.join("versions"), "26.3").is_err()
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
