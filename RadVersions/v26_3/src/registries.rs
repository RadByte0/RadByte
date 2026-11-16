//! Resolved Minecraft 26.3 (`protocol 777`) registries backed by a [`RegistryProvider`].

use std::sync::Arc;

use rad_core::registry_provider::{
    BlockRegistryEntry, LoadedPackageRegistryProvider, RegistryProvider, RegistryProviderError,
    RegistryTagSet, TagDefinition, VersionMetadata,
};
use rad_core::version_package::LoadedVersionPackage;

/// [`RegistryProvider`] adapter owning an `Arc<LoadedVersionPackage>`.
#[derive(Debug, Clone)]
pub struct ArcPackageRegistryProvider(pub Arc<LoadedVersionPackage>);

impl RegistryProvider for ArcPackageRegistryProvider {
    fn metadata(&self) -> VersionMetadata {
        LoadedPackageRegistryProvider::new(&self.0).metadata()
    }

    fn block_by_name(&self, name: &str) -> Option<BlockRegistryEntry> {
        LoadedPackageRegistryProvider::new(&self.0).block_by_name(name)
    }

    fn block_by_state(&self, state: u16) -> Option<BlockRegistryEntry> {
        LoadedPackageRegistryProvider::new(&self.0).block_by_state(state)
    }

    fn item_by_name(&self, name: &str) -> Option<i32> {
        LoadedPackageRegistryProvider::new(&self.0).item_by_name(name)
    }

    fn item_name_by_id(&self, id: i32) -> Option<&str> {
        self.0.item_name_by_id(id)
    }

    fn entity_by_name(&self, name: &str) -> Option<i32> {
        LoadedPackageRegistryProvider::new(&self.0).entity_by_name(name)
    }

    fn menu_by_name(&self, name: &str) -> Option<i32> {
        LoadedPackageRegistryProvider::new(&self.0).menu_by_name(name)
    }

    fn block_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        LoadedPackageRegistryProvider::new(&self.0).block_tags()
    }

    fn item_tags(&self) -> Result<Vec<TagDefinition>, RegistryProviderError> {
        LoadedPackageRegistryProvider::new(&self.0).item_tags()
    }

    fn core_registry_tags(&self) -> Result<Vec<RegistryTagSet>, RegistryProviderError> {
        LoadedPackageRegistryProvider::new(&self.0).core_registry_tags()
    }
}

/// Pre-resolved block registry entries required by the 26.3 runtime.
#[derive(Debug, Clone)]
pub struct BlockIds {
    /// `minecraft:air`
    pub air: BlockRegistryEntry,
    /// Optional `minecraft:cave_air`
    pub cave_air: Option<BlockRegistryEntry>,
    /// Optional `minecraft:void_air`
    pub void_air: Option<BlockRegistryEntry>,
    /// `minecraft:bedrock`
    pub bedrock: BlockRegistryEntry,
    /// `minecraft:chest`
    pub chest: BlockRegistryEntry,
    /// `minecraft:crafting_table`
    pub crafting_table: BlockRegistryEntry,
    /// `minecraft:end_stone`
    pub end_stone: BlockRegistryEntry,
    /// `minecraft:fire`
    pub fire: BlockRegistryEntry,
    /// `minecraft:furnace`
    pub furnace: BlockRegistryEntry,
    /// `minecraft:gravel`
    pub gravel: BlockRegistryEntry,
    /// Optional `minecraft:lava`
    pub lava: Option<BlockRegistryEntry>,
    /// Optional `minecraft:bubble_column`
    pub bubble_column: Option<BlockRegistryEntry>,
    /// `minecraft:lever`
    pub lever: BlockRegistryEntry,
    /// `minecraft:oak_planks`
    pub oak_planks: BlockRegistryEntry,
    /// `minecraft:redstone_lamp`
    pub redstone_lamp: BlockRegistryEntry,
    /// `minecraft:redstone_torch`
    pub redstone_torch: BlockRegistryEntry,
    /// `minecraft:redstone_wire`
    pub redstone_wire: BlockRegistryEntry,
    /// `minecraft:sand`
    pub sand: BlockRegistryEntry,
    /// `minecraft:tnt`
    pub tnt: BlockRegistryEntry,
    /// `minecraft:water`
    pub water: BlockRegistryEntry,
    /// `minecraft:wheat`
    pub wheat: BlockRegistryEntry,
}

/// Pre-resolved item numeric IDs required by the 26.3 runtime.
#[derive(Debug, Clone, Copy)]
pub struct ItemIds {
    /// `minecraft:apple`
    pub apple: i32,
    /// `minecraft:arrow`
    pub arrow: i32,
    /// `minecraft:bow`
    pub bow: i32,
    /// `minecraft:bread`
    pub bread: i32,
    /// `minecraft:chest`
    pub chest: i32,
    /// `minecraft:coal`
    pub coal: i32,
    /// `minecraft:cooked_porkchop`
    pub cooked_porkchop: i32,
    /// `minecraft:crafting_table`
    pub crafting_table: i32,
    /// `minecraft:furnace`
    pub furnace: i32,
    /// `minecraft:oak_log`
    pub oak_log: i32,
    /// `minecraft:oak_planks`
    pub oak_planks: i32,
    /// `minecraft:redstone`
    pub redstone: i32,
    /// `minecraft:snowball`
    pub snowball: i32,
    /// `minecraft:stick`
    pub stick: i32,
    /// `minecraft:torch`
    pub torch: i32,
    /// `minecraft:wooden_pickaxe`
    pub wooden_pickaxe: i32,
}

/// Pre-resolved entity type numeric IDs required by the 26.3 runtime.
#[derive(Debug, Clone, Copy)]
pub struct EntityIds {
    /// `minecraft:arrow`
    pub arrow: i32,
    /// `minecraft:chicken`
    pub chicken: i32,
    /// `minecraft:cow`
    pub cow: i32,
    /// `minecraft:creeper`
    pub creeper: i32,
    /// `minecraft:enderman`
    pub enderman: i32,
    /// `minecraft:iron_golem`
    pub iron_golem: i32,
    /// `minecraft:item`
    pub item: i32,
    /// `minecraft:pig`
    pub pig: i32,
    /// `minecraft:player`
    pub player: i32,
    /// `minecraft:sheep`
    pub sheep: i32,
    /// `minecraft:skeleton`
    pub skeleton: i32,
    /// `minecraft:snowball`
    pub snowball: i32,
    /// `minecraft:spider`
    pub spider: i32,
    /// `minecraft:villager`
    pub villager: i32,
    /// `minecraft:zombie`
    pub zombie: i32,
}

/// Pre-resolved menu type numeric IDs required by the 26.3 runtime.
#[derive(Debug, Clone, Copy)]
pub struct MenuIds {
    /// `minecraft:crafting`
    pub crafting: i32,
    /// `minecraft:furnace`
    pub furnace: i32,
}

/// Resolved 26.3 registries backed by a [`RegistryProvider`].
#[derive(Clone)]
pub struct Version26_3Registries {
    provider: Arc<dyn RegistryProvider>,
    /// Resolved block entries.
    pub blocks: BlockIds,
    /// Resolved item IDs.
    pub items: ItemIds,
    /// Resolved entity type IDs.
    pub entities: EntityIds,
    /// Resolved menu type IDs.
    pub menus: MenuIds,
}

impl std::fmt::Debug for Version26_3Registries {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Version26_3Registries")
            .field("metadata", &self.provider.metadata())
            .finish()
    }
}

fn require_block(
    provider: &dyn RegistryProvider,
    name: &str,
) -> Result<BlockRegistryEntry, RegistryProviderError> {
    provider
        .block_by_name(name)
        .ok_or_else(|| RegistryProviderError::MissingData(format!("missing block '{name}'")))
}

fn require_item(provider: &dyn RegistryProvider, name: &str) -> Result<i32, RegistryProviderError> {
    provider
        .item_by_name(name)
        .ok_or_else(|| RegistryProviderError::MissingData(format!("missing item '{name}'")))
}

fn require_entity(
    provider: &dyn RegistryProvider,
    name: &str,
) -> Result<i32, RegistryProviderError> {
    provider
        .entity_by_name(name)
        .ok_or_else(|| RegistryProviderError::MissingData(format!("missing entity '{name}'")))
}

fn require_menu(provider: &dyn RegistryProvider, name: &str) -> Result<i32, RegistryProviderError> {
    provider
        .menu_by_name(name)
        .ok_or_else(|| RegistryProviderError::MissingData(format!("missing menu '{name}'")))
}

/// Returns `true` if `state` falls within `entry`'s inclusive state range.
#[inline]
pub fn state_in_entry(entry: &BlockRegistryEntry, state: u16) -> bool {
    state >= entry.min_state && state <= entry.max_state
}

impl Version26_3Registries {
    /// Resolves and validates all required 26.3 registries from `provider`.
    ///
    /// When `allow_synthetic` is `false`, synthetic test fixtures are rejected.
    pub fn from_provider(
        provider: Arc<dyn RegistryProvider>,
        allow_synthetic: bool,
    ) -> Result<Self, RegistryProviderError> {
        provider.validate(!allow_synthetic)?;

        let blocks = BlockIds {
            air: require_block(provider.as_ref(), "minecraft:air")?,
            cave_air: provider.block_by_name("minecraft:cave_air"),
            void_air: provider.block_by_name("minecraft:void_air"),
            bedrock: require_block(provider.as_ref(), "minecraft:bedrock")?,
            chest: require_block(provider.as_ref(), "minecraft:chest")?,
            crafting_table: require_block(provider.as_ref(), "minecraft:crafting_table")?,
            end_stone: require_block(provider.as_ref(), "minecraft:end_stone")?,
            fire: require_block(provider.as_ref(), "minecraft:fire")?,
            furnace: require_block(provider.as_ref(), "minecraft:furnace")?,
            gravel: require_block(provider.as_ref(), "minecraft:gravel")?,
            lava: provider.block_by_name("minecraft:lava"),
            bubble_column: provider.block_by_name("minecraft:bubble_column"),
            lever: require_block(provider.as_ref(), "minecraft:lever")?,
            oak_planks: require_block(provider.as_ref(), "minecraft:oak_planks")?,
            redstone_lamp: require_block(provider.as_ref(), "minecraft:redstone_lamp")?,
            redstone_torch: require_block(provider.as_ref(), "minecraft:redstone_torch")?,
            redstone_wire: require_block(provider.as_ref(), "minecraft:redstone_wire")?,
            sand: require_block(provider.as_ref(), "minecraft:sand")?,
            tnt: require_block(provider.as_ref(), "minecraft:tnt")?,
            water: require_block(provider.as_ref(), "minecraft:water")?,
            wheat: require_block(provider.as_ref(), "minecraft:wheat")?,
        };

        let items = ItemIds {
            apple: require_item(provider.as_ref(), "minecraft:apple")?,
            arrow: require_item(provider.as_ref(), "minecraft:arrow")?,
            bow: require_item(provider.as_ref(), "minecraft:bow")?,
            bread: require_item(provider.as_ref(), "minecraft:bread")?,
            chest: require_item(provider.as_ref(), "minecraft:chest")?,
            coal: require_item(provider.as_ref(), "minecraft:coal")?,
            cooked_porkchop: require_item(provider.as_ref(), "minecraft:cooked_porkchop")?,
            crafting_table: require_item(provider.as_ref(), "minecraft:crafting_table")?,
            furnace: require_item(provider.as_ref(), "minecraft:furnace")?,
            oak_log: require_item(provider.as_ref(), "minecraft:oak_log")?,
            oak_planks: require_item(provider.as_ref(), "minecraft:oak_planks")?,
            redstone: require_item(provider.as_ref(), "minecraft:redstone")?,
            snowball: require_item(provider.as_ref(), "minecraft:snowball")?,
            stick: require_item(provider.as_ref(), "minecraft:stick")?,
            torch: require_item(provider.as_ref(), "minecraft:torch")?,
            wooden_pickaxe: require_item(provider.as_ref(), "minecraft:wooden_pickaxe")?,
        };

        let pig = require_entity(provider.as_ref(), "minecraft:pig")?;
        let zombie = require_entity(provider.as_ref(), "minecraft:zombie")?;
        let entities = EntityIds {
            arrow: require_entity(provider.as_ref(), "minecraft:arrow")?,
            chicken: provider.entity_by_name("minecraft:chicken").unwrap_or(pig),
            cow: provider.entity_by_name("minecraft:cow").unwrap_or(pig),
            creeper: provider
                .entity_by_name("minecraft:creeper")
                .unwrap_or(zombie),
            enderman: provider
                .entity_by_name("minecraft:enderman")
                .unwrap_or(zombie),
            iron_golem: provider
                .entity_by_name("minecraft:iron_golem")
                .unwrap_or(pig),
            item: require_entity(provider.as_ref(), "minecraft:item")?,
            pig,
            player: require_entity(provider.as_ref(), "minecraft:player")?,
            sheep: provider.entity_by_name("minecraft:sheep").unwrap_or(pig),
            skeleton: require_entity(provider.as_ref(), "minecraft:skeleton")?,
            snowball: require_entity(provider.as_ref(), "minecraft:snowball")?,
            spider: provider
                .entity_by_name("minecraft:spider")
                .unwrap_or(zombie),
            villager: provider.entity_by_name("minecraft:villager").unwrap_or(pig),
            zombie,
        };

        let menus = MenuIds {
            crafting: require_menu(provider.as_ref(), "minecraft:crafting")?,
            furnace: require_menu(provider.as_ref(), "minecraft:furnace")?,
        };

        Ok(Self {
            provider,
            blocks,
            items,
            entities,
            menus,
        })
    }

    /// Returns the underlying [`RegistryProvider`].
    pub fn provider(&self) -> &Arc<dyn RegistryProvider> {
        &self.provider
    }

    /// Returns `true` if `state` represents air (`air`, `cave_air`, or `void_air`).
    #[inline]
    pub fn is_air_state(&self, state: u16) -> bool {
        state_in_entry(&self.blocks.air, state)
            || self
                .blocks
                .cave_air
                .as_ref()
                .is_some_and(|e| state_in_entry(e, state))
            || self
                .blocks
                .void_air
                .as_ref()
                .is_some_and(|e| state_in_entry(e, state))
    }

    /// Returns `true` if `state` represents a fluid (`water`, `lava`, or `bubble_column`).
    #[inline]
    pub fn is_fluid_state(&self, state: u16) -> bool {
        state_in_entry(&self.blocks.water, state)
            || self
                .blocks
                .lava
                .as_ref()
                .is_some_and(|e| state_in_entry(e, state))
            || self
                .blocks
                .bubble_column
                .as_ref()
                .is_some_and(|e| state_in_entry(e, state))
    }

    /// Looks up a block entry by resource name.
    pub fn block_by_name(&self, name: &str) -> Option<BlockRegistryEntry> {
        self.provider.block_by_name(name)
    }

    /// Looks up a block entry by global block state ID.
    pub fn block_by_state(&self, state: u16) -> Option<BlockRegistryEntry> {
        self.provider.block_by_state(state)
    }

    /// Looks up an item ID by resource name.
    pub fn item_by_name(&self, name: &str) -> Option<i32> {
        self.provider.item_by_name(name)
    }

    /// Looks up an item resource name by numeric ID.
    pub fn item_name_by_id(&self, id: i32) -> Option<&str> {
        self.provider.item_name_by_id(id)
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Synthetic26_3TestProvider {
    blocks: Vec<BlockRegistryEntry>,
    items: std::collections::HashMap<String, i32>,
    item_names: std::collections::HashMap<i32, String>,
    entities: std::collections::HashMap<String, i32>,
    menus: std::collections::HashMap<String, i32>,
}

#[cfg(test)]
impl RegistryProvider for Synthetic26_3TestProvider {
    fn metadata(&self) -> rad_core::registry_provider::VersionMetadata {
        rad_core::registry_provider::VersionMetadata {
            game_version: "26.3".to_owned(),
            protocol_version: 777,
            world_data_version: 4786,
            source: "synthetic-26-3-test".to_owned(),
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

    fn block_tags(
        &self,
    ) -> Result<Vec<rad_core::registry_provider::TagDefinition>, RegistryProviderError> {
        Ok(vec![rad_core::registry_provider::TagDefinition {
            name: "minecraft:mineable/pickaxe".to_owned(),
            members: vec!["minecraft:furnace".to_owned()],
        }])
    }

    fn item_tags(
        &self,
    ) -> Result<Vec<rad_core::registry_provider::TagDefinition>, RegistryProviderError> {
        Ok(vec![rad_core::registry_provider::TagDefinition {
            name: "minecraft:planks".to_owned(),
            members: vec!["minecraft:oak_planks".to_owned()],
        }])
    }

    fn core_registry_tags(
        &self,
    ) -> Result<Vec<rad_core::registry_provider::RegistryTagSet>, RegistryProviderError> {
        Ok(vec![rad_core::registry_provider::RegistryTagSet {
            registry_id: "minecraft:damage_type".to_owned(),
            tags: vec![rad_core::registry_provider::TagDefinition {
                name: "minecraft:is_fire".to_owned(),
                members: vec!["minecraft:in_fire".to_owned()],
            }],
        }])
    }
}

#[cfg(test)]
pub(crate) fn synthetic_26_3_test_provider() -> Synthetic26_3TestProvider {
    use std::collections::HashMap;

    let block_names = [
        "minecraft:air",
        "minecraft:bedrock",
        "minecraft:chest",
        "minecraft:crafting_table",
        "minecraft:end_stone",
        "minecraft:fire",
        "minecraft:furnace",
        "minecraft:gravel",
        "minecraft:lever",
        "minecraft:oak_planks",
        "minecraft:redstone_lamp",
        "minecraft:redstone_torch",
        "minecraft:redstone_wire",
        "minecraft:sand",
        "minecraft:tnt",
        "minecraft:water",
        "minecraft:wheat",
    ];
    let mut blocks = Vec::new();
    let mut next_state = 0u16;
    for (idx, name) in block_names.iter().enumerate() {
        let min_state = next_state;
        // Give multi-state blocks (water, lever, redstone_*, wheat) a small range.
        let count = if *name == "minecraft:water"
            || *name == "minecraft:redstone_wire"
            || *name == "minecraft:wheat"
        {
            16
        } else if *name == "minecraft:lever"
            || *name == "minecraft:redstone_lamp"
            || *name == "minecraft:redstone_torch"
        {
            4
        } else {
            1
        };
        let max_state = min_state + count - 1;
        blocks.push(BlockRegistryEntry {
            block_id: idx as u16,
            name: (*name).to_owned(),
            default_state: min_state,
            min_state,
            max_state,
        });
        next_state = max_state + 1;
    }

    let item_names_list = [
        "minecraft:apple",
        "minecraft:arrow",
        "minecraft:bow",
        "minecraft:bread",
        "minecraft:chest",
        "minecraft:coal",
        "minecraft:cooked_porkchop",
        "minecraft:crafting_table",
        "minecraft:furnace",
        "minecraft:oak_log",
        "minecraft:oak_planks",
        "minecraft:redstone",
        "minecraft:snowball",
        "minecraft:stick",
        "minecraft:torch",
        "minecraft:wooden_pickaxe",
    ];
    let mut items = HashMap::new();
    let mut item_names = HashMap::new();
    for (idx, name) in item_names_list.iter().enumerate() {
        let id = idx as i32 + 1;
        items.insert((*name).to_owned(), id);
        item_names.insert(id, (*name).to_owned());
    }

    let mut entities = HashMap::new();
    for (idx, name) in [
        "minecraft:arrow",
        "minecraft:item",
        "minecraft:pig",
        "minecraft:player",
        "minecraft:skeleton",
        "minecraft:snowball",
        "minecraft:zombie",
    ]
    .iter()
    .enumerate()
    {
        entities.insert((*name).to_owned(), idx as i32 + 1);
    }

    let mut menus = HashMap::new();
    menus.insert("minecraft:crafting".to_owned(), 1);
    menus.insert("minecraft:furnace".to_owned(), 2);

    Synthetic26_3TestProvider {
        blocks,
        items,
        item_names,
        entities,
        menus,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rad_core::registry_provider::{
        RegistryTagSet, SyntheticFixtureRegistryProvider, TagDefinition, VersionMetadata,
    };

    #[derive(Debug)]
    struct EmptyProvider;

    impl RegistryProvider for EmptyProvider {
        fn metadata(&self) -> VersionMetadata {
            VersionMetadata {
                game_version: "26.3".to_string(),
                protocol_version: 777,
                world_data_version: 4786,
                source: "empty-test".to_string(),
                is_synthetic_fixture: false,
            }
        }

        fn block_by_name(&self, _name: &str) -> Option<BlockRegistryEntry> {
            None
        }

        fn block_by_state(&self, _state: u16) -> Option<BlockRegistryEntry> {
            None
        }

        fn item_by_name(&self, _name: &str) -> Option<i32> {
            None
        }

        fn item_name_by_id(&self, _id: i32) -> Option<&str> {
            None
        }

        fn entity_by_name(&self, _name: &str) -> Option<i32> {
            None
        }

        fn menu_by_name(&self, _name: &str) -> Option<i32> {
            None
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
    fn rejects_synthetic_when_real_data_required() {
        let err = Version26_3Registries::from_provider(
            Arc::new(SyntheticFixtureRegistryProvider::minimal_valid()),
            false,
        )
        .unwrap_err();
        assert!(matches!(err, RegistryProviderError::MissingData(_)));
    }

    #[test]
    fn rejects_missing_entries_on_empty_provider() {
        let err = Version26_3Registries::from_provider(Arc::new(EmptyProvider), true).unwrap_err();
        assert!(matches!(err, RegistryProviderError::MissingData(_)));
    }

    #[test]
    fn loads_minimal_synthetic_fixture_when_allowed() {
        let regs =
            Version26_3Registries::from_provider(Arc::new(synthetic_26_3_test_provider()), true)
                .expect("synthetic fixture should resolve required 26.3 entries");
        assert!(regs.is_air_state(regs.blocks.air.default_state));
        assert!(regs.is_fluid_state(regs.blocks.water.default_state));
        assert!(!regs.is_air_state(regs.blocks.bedrock.default_state));
    }
}
