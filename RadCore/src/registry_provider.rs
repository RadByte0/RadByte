//! Typed registry and tag provider abstraction separating RadByte code from
//! bundled or external Minecraft version data.
//!
//! This module allows protocol, configuration, and server components to query
//! block states, items, entities, menus, configuration tags, and version
//! metadata through [`RegistryProvider`] without hard-coding compile-time
//! `mc777_data` tables.

use std::collections::{HashMap, HashSet};

use thiserror::Error;

use crate::version_package::LoadedVersionPackage;

/// Maximum number of entries allowed in a single registry or tag list when
/// validating external/synthetic inputs.
pub const MAX_REGISTRY_ENTRIES: usize = 250_000;

/// Metadata describing a registry data source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionMetadata {
    /// Target game version string (or synthetic fixture marker).
    pub game_version: String,
    /// Protocol version number (`0` for synthetic non-protocol fixtures).
    pub protocol_version: i32,
    /// World data version number (`0` for synthetic non-world fixtures).
    pub world_data_version: i32,
    /// Human-readable origin of the data.
    pub source: String,
    /// `true` when this provider only holds synthetic unit-test fixtures and
    /// MUST NOT be treated as real Minecraft 26.3 data.
    pub is_synthetic_fixture: bool,
}

/// Block registry entry with state range bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRegistryEntry {
    /// Numeric block registry ID.
    pub block_id: u16,
    /// Namespaced block identifier (e.g. `"minecraft:stone"` or `"test:stone"`).
    pub name: String,
    /// Default block state ID.
    pub default_state: u16,
    /// Minimum valid block state ID for this block.
    pub min_state: u16,
    /// Maximum valid block state ID for this block.
    pub max_state: u16,
}

/// A named tag and its member identifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagDefinition {
    /// Namespaced tag identifier.
    pub name: String,
    /// Namespaced member identifiers belonging to this tag.
    pub members: Vec<String>,
}

/// All tags for a single registry (e.g. `"minecraft:damage_type"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryTagSet {
    /// Registry identifier.
    pub registry_id: String,
    /// Tags belonging to `registry_id`.
    pub tags: Vec<TagDefinition>,
}

/// Errors returned by a [`RegistryProvider`] or validator.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RegistryProviderError {
    /// Required registry or tag data is not available in this build/runtime.
    #[error("missing registry data: {0}")]
    MissingData(String),
    /// Registry data failed structural or bounds validation.
    #[error("invalid registry data: {0}")]
    ValidationError(String),
}

/// Abstraction over version-specific registry and tag data.
pub trait RegistryProvider: Send + Sync {
    /// Returns metadata describing the data source.
    fn metadata(&self) -> VersionMetadata;

    /// Looks up a block entry by namespaced name.
    fn block_by_name(&self, name: &str) -> Option<BlockRegistryEntry>;

    /// Looks up a block entry by numeric block state ID.
    fn block_by_state(&self, state: u16) -> Option<BlockRegistryEntry>;

    /// Looks up an item ID by namespaced name.
    fn item_by_name(&self, name: &str) -> Option<i32>;

    /// Looks up an item namespaced name by numeric ID.
    fn item_name_by_id(&self, id: i32) -> Option<&str>;

    /// Looks up an entity type ID by namespaced name.
    fn entity_by_name(&self, name: &str) -> Option<i32>;

    /// Looks up a menu/screen handler ID by namespaced name.
    fn menu_by_name(&self, name: &str) -> Option<i32>;

    /// Returns block tags if provided by this source.
    fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError>;

    /// Returns item tags if provided by this source.
    fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError>;

    /// Returns non-block/item registry tags if provided by this source.
    fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError>;

    /// Validates that required registries are present and well-formed.
    ///
    /// When `require_real_data` is `true`, synthetic test fixtures are rejected.
    fn validate(&self, require_real_data: bool) -> Result<(), RegistryProviderError> {
        let meta = self.metadata();
        if meta.game_version.trim().is_empty() {
            return Err(RegistryProviderError::ValidationError(
                "game_version must not be empty".into(),
            ));
        }
        if meta.source.trim().is_empty() {
            return Err(RegistryProviderError::ValidationError(
                "source must not be empty".into(),
            ));
        }
        if require_real_data && meta.is_synthetic_fixture {
            return Err(RegistryProviderError::MissingData(
                "synthetic test fixtures cannot be used as real game data".into(),
            ));
        }
        if require_real_data && meta.protocol_version <= 0 {
            return Err(RegistryProviderError::ValidationError(
                "protocol_version must be positive for real game data".into(),
            ));
        }
        Ok(())
    }
}

/// [`RegistryProvider`] backed by a runtime-loaded [`LoadedVersionPackage`].
///
/// This provider reads block, item, entity, and menu definitions loaded from
/// disk (`versions/<version>/data/*.json`) without depending on compile-time
/// `mc777_data` tables.
pub struct LoadedPackageRegistryProvider<'a> {
    package: &'a LoadedVersionPackage,
}

impl<'a> LoadedPackageRegistryProvider<'a> {
    /// Creates a provider wrapping an already-validated [`LoadedVersionPackage`].
    #[must_use]
    pub fn new(package: &'a LoadedVersionPackage) -> Self {
        Self { package }
    }
}

impl RegistryProvider for LoadedPackageRegistryProvider<'_> {
    fn metadata(&self) -> VersionMetadata {
        let m = self.package.manifest();
        VersionMetadata {
            game_version: m.game_version.clone(),
            protocol_version: m.protocol_version,
            world_data_version: m.world_data_version,
            source: m.source.clone(),
            is_synthetic_fixture: false,
        }
    }

    fn block_by_name(&self, name: &str) -> Option<BlockRegistryEntry> {
        self.package
            .block_by_name(name)
            .map(|b| BlockRegistryEntry {
                block_id: b.block_id,
                name: b.name.clone(),
                default_state: b.default_state,
                min_state: b.min_state,
                max_state: b.max_state,
            })
    }

    fn block_by_state(&self, state: u16) -> Option<BlockRegistryEntry> {
        let name = self.package.block_name_by_state(state)?;
        self.block_by_name(name)
    }

    fn item_by_name(&self, name: &str) -> Option<i32> {
        self.package.item_id_by_name(name)
    }

    fn item_name_by_id(&self, id: i32) -> Option<&str> {
        self.package.item_name_by_id(id)
    }

    fn entity_by_name(&self, name: &str) -> Option<i32> {
        self.package.entity_id_by_name(name)
    }

    fn menu_by_name(&self, name: &str) -> Option<i32> {
        self.package.menu_id_by_name(name)
    }

    fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        self.package
            .block_tags()
            .map(<[TagDefinition]>::to_vec)
            .ok_or_else(|| {
                RegistryProviderError::MissingData(
                    "block configuration tags are not packaged in LoadedVersionPackage".into(),
                )
            })
    }

    fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        self.package
            .item_tags()
            .map(<[TagDefinition]>::to_vec)
            .ok_or_else(|| {
                RegistryProviderError::MissingData(
                    "item configuration tags are not packaged in LoadedVersionPackage".into(),
                )
            })
    }

    fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError> {
        self.package
            .core_registry_tags()
            .map(<[RegistryTagSet]>::to_vec)
            .ok_or_else(|| {
                RegistryProviderError::MissingData(
                    "core registry configuration tags are not packaged in LoadedVersionPackage"
                        .into(),
                )
            })
    }
}

/// Small, explicitly synthetic fixture provider for unit tests.
///
/// Provenance: hand-written synthetic test entries (`test:*`), **not** extracted
/// from Minecraft and **not** valid for real Minecraft 26.3 client logins.
#[derive(Debug, Clone)]
pub struct SyntheticFixtureRegistryProvider {
    blocks: Vec<BlockRegistryEntry>,
    items: HashMap<String, i32>,
    item_names: HashMap<i32, String>,
    entities: HashMap<String, i32>,
    menus: HashMap<String, i32>,
    block_tags: Vec<TagDefinition>,
    item_tags: Vec<TagDefinition>,
    core_tags: Vec<RegistryTagSet>,
}

impl SyntheticFixtureRegistryProvider {
    /// Creates a minimal valid synthetic fixture with `test:air`, `test:stone`,
    /// `test:stick`, `test:pig`, and `test:menu`.
    #[must_use]
    pub fn minimal_valid() -> Self {
        let mut items = HashMap::new();
        items.insert("test:air".to_owned(), 0);
        items.insert("test:stone".to_owned(), 1);
        items.insert("test:stick".to_owned(), 2);

        let item_names = items.iter().map(|(k, &v)| (v, k.clone())).collect();

        let mut entities = HashMap::new();
        entities.insert("test:pig".to_owned(), 1);

        let mut menus = HashMap::new();
        menus.insert("test:menu".to_owned(), 0);

        Self {
            blocks: vec![
                BlockRegistryEntry {
                    block_id: 0,
                    name: "test:air".to_owned(),
                    default_state: 0,
                    min_state: 0,
                    max_state: 0,
                },
                BlockRegistryEntry {
                    block_id: 1,
                    name: "test:stone".to_owned(),
                    default_state: 1,
                    min_state: 1,
                    max_state: 1,
                },
            ],
            items,
            item_names,
            entities,
            menus,
            block_tags: vec![TagDefinition {
                name: "test:solid".to_owned(),
                members: vec!["test:stone".to_owned()],
            }],
            item_tags: vec![TagDefinition {
                name: "test:sticks".to_owned(),
                members: vec!["test:stick".to_owned()],
            }],
            core_tags: vec![RegistryTagSet {
                registry_id: "minecraft:damage_type".to_owned(),
                tags: vec![TagDefinition {
                    name: "minecraft:is_fire".to_owned(),
                    members: vec!["minecraft:in_fire".to_owned()],
                }],
            }],
        }
    }

    /// Validates all internal tables for duplicates, empty registries,
    /// out-of-bounds state ranges, overlapping block states, or oversized lists.
    pub fn validate_tables(&self) -> Result<(), RegistryProviderError> {
        if self.blocks.is_empty()
            || self.items.is_empty()
            || self.entities.is_empty()
            || self.menus.is_empty()
        {
            return Err(RegistryProviderError::ValidationError(
                "blocks, items, entities, and menus must all be non-empty".into(),
            ));
        }
        if self.blocks.len() > MAX_REGISTRY_ENTRIES
            || self.items.len() > MAX_REGISTRY_ENTRIES
            || self.block_tags.len() > MAX_REGISTRY_ENTRIES
            || self.item_tags.len() > MAX_REGISTRY_ENTRIES
        {
            return Err(RegistryProviderError::ValidationError(
                "registry entry count exceeds MAX_REGISTRY_ENTRIES".into(),
            ));
        }
        let mut seen_names = HashSet::new();
        let mut seen_ids = HashSet::new();
        for b in &self.blocks {
            if b.name.trim().is_empty() {
                return Err(RegistryProviderError::ValidationError(
                    "block name must not be empty".into(),
                ));
            }
            if !seen_names.insert(&b.name) {
                return Err(RegistryProviderError::ValidationError(format!(
                    "duplicate block name '{}'",
                    b.name
                )));
            }
            if !seen_ids.insert(b.block_id) {
                return Err(RegistryProviderError::ValidationError(format!(
                    "duplicate block_id {}",
                    b.block_id
                )));
            }
            if !(b.min_state <= b.default_state && b.default_state <= b.max_state) {
                return Err(RegistryProviderError::ValidationError(format!(
                    "invalid state range for '{}': min={}, default={}, max={}",
                    b.name, b.min_state, b.default_state, b.max_state
                )));
            }
        }
        let mut ranges: Vec<(u16, u16, &str)> = self
            .blocks
            .iter()
            .map(|b| (b.min_state, b.max_state, b.name.as_str()))
            .collect();
        ranges.sort_by_key(|r| (r.0, r.1));
        for w in ranges.windows(2) {
            if w[1].0 <= w[0].1 {
                return Err(RegistryProviderError::ValidationError(format!(
                    "overlapping block state ranges between '{}' ({}..={}) and '{}' ({}..={})",
                    w[0].2, w[0].0, w[0].1, w[1].2, w[1].0, w[1].1
                )));
            }
        }
        Ok(())
    }
}

impl RegistryProvider for SyntheticFixtureRegistryProvider {
    fn metadata(&self) -> VersionMetadata {
        VersionMetadata {
            game_version: "0.0-synthetic".to_owned(),
            protocol_version: 0,
            world_data_version: 0,
            source: "synthetic-unit-test-fixture".to_owned(),
            is_synthetic_fixture: true,
        }
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
        self.items.get(name).copied()
    }

    fn item_name_by_id(&self, id: i32) -> Option<&str> {
        self.item_names.get(&id).map(String::as_str)
    }

    fn entity_by_name(&self, name: &str) -> Option<i32> {
        self.entities.get(name).copied()
    }

    fn menu_by_name(&self, name: &str) -> Option<i32> {
        self.menus.get(name).copied()
    }

    fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        Ok(self.block_tags.clone())
    }

    fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        Ok(self.item_tags.clone())
    }

    fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError> {
        Ok(self.core_tags.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_fixture_validates_and_rejects_real_data_claim() {
        let fixture = SyntheticFixtureRegistryProvider::minimal_valid();
        assert!(fixture.validate_tables().is_ok());
        assert!(fixture.validate(false).is_ok());
        let err = fixture.validate(true).unwrap_err();
        assert!(matches!(err, RegistryProviderError::MissingData(_)));
    }

    #[test]
    fn synthetic_fixture_rejects_invalid_state_ranges_and_overlaps() {
        let mut bad = SyntheticFixtureRegistryProvider::minimal_valid();
        bad.blocks[1].default_state = 99;
        assert!(matches!(
            bad.validate_tables(),
            Err(RegistryProviderError::ValidationError(_))
        ));

        let mut overlap = SyntheticFixtureRegistryProvider::minimal_valid();
        overlap.blocks[0].max_state = 1;
        assert!(matches!(
            overlap.validate_tables(),
            Err(RegistryProviderError::ValidationError(_))
        ));
    }

    #[test]
    fn synthetic_fixture_rejects_empty_and_excessive_tables() {
        let mut empty = SyntheticFixtureRegistryProvider::minimal_valid();
        empty.items.clear();
        assert!(matches!(
            empty.validate_tables(),
            Err(RegistryProviderError::ValidationError(_))
        ));

        let mut huge = SyntheticFixtureRegistryProvider::minimal_valid();
        huge.block_tags = (0..=MAX_REGISTRY_ENTRIES)
            .map(|i| TagDefinition {
                name: format!("test:t{i}"),
                members: Vec::new(),
            })
            .collect();
        assert!(matches!(
            huge.validate_tables(),
            Err(RegistryProviderError::ValidationError(_))
        ));
    }
}
