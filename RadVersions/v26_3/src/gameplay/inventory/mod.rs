//! Player inventory, hotbar, crafting grid, container sync, and item-block mappings.

mod recipes;

pub use recipes::{DataPackRecipe, load_custom_recipes_from_dir, match_custom_recipe};

use std::sync::Arc;

use rad_core::packet::play::{
    ContainerSetContent, ContainerSetContentStacks, ItemComponentPatch, ItemStack, SimpleSlot,
};
use rad_core::types::VarInt;

use crate::registries::{ItemIds, Version26_3Registries};

/// Standard player inventory slot count (`46` slots in MC 1.21+ / 26.3).
pub const PLAYER_INVENTORY_SLOTS: usize = 46;
/// Hotbar start index in `ContainerSetContent` for window `0`.
pub const HOTBAR_START_SLOT: usize = 36;
/// Window ID for an open 3x3 Crafting Table screen.
pub const CRAFTING_WINDOW_ID: i32 = 2;
/// Window ID for an open Furnace screen.
pub const FURNACE_WINDOW_ID: i32 = 3;

/// Per-player inventory state.
#[derive(Debug, Clone)]
pub struct PlayerInventory {
    /// All 46 inventory slots (`0` = 2x2 craft result, `1..=4` = 2x2 grid, `9..=35` = main, `36..=44` = hotbar).
    pub slots: Vec<SimpleSlot>,
    /// Per-slot item data component patches (`46` entries).
    pub slot_components: Vec<ItemComponentPatch>,
    /// Transient 3x3 Crafting Table window slots (`0` = result, `1..=9` = 3x3 grid).
    pub craft3x3: [SimpleSlot; 10],
    /// Custom recipes loaded from `<world_dir>/datapacks/recipes/*.json`.
    pub custom_recipes: Vec<DataPackRecipe>,
    /// Item currently carried on the cursor.
    pub carried: SimpleSlot,
    /// Data components on the carried cursor item.
    pub carried_components: ItemComponentPatch,
    /// Selected hotbar index (`0..=8`).
    pub selected_hotbar: u8,
    /// Monotonic container state ID.
    pub state_id: i32,
    /// Resolved 26.3 registries used for item/block lookups and crafting.
    pub registries: Arc<Version26_3Registries>,
}

impl PlayerInventory {
    /// Creates a new [`PlayerInventory`] backed by `registries`.
    #[must_use]
    pub fn new_with_registries(registries: Arc<Version26_3Registries>) -> Self {
        let mut slots = vec![SimpleSlot::default(); PLAYER_INVENTORY_SLOTS];
        // Starter building & crafting resources in hotbar slots 0..=4
        slots[HOTBAR_START_SLOT] = SimpleSlot {
            count: 64,
            item_id: 1, // stone
        };
        slots[HOTBAR_START_SLOT + 1] = SimpleSlot {
            count: 64,
            item_id: 54, // grass_block (777)
        };
        slots[HOTBAR_START_SLOT + 2] = SimpleSlot {
            count: 64,
            item_id: 55, // dirt (777)
        };
        slots[HOTBAR_START_SLOT + 3] = SimpleSlot {
            count: 64,
            item_id: 62, // cobblestone (777)
        };
        slots[HOTBAR_START_SLOT + 4] = SimpleSlot {
            count: 16,
            item_id: registries.items.oak_planks, // oak_planks for 2x2/3x3 crafting
        };
        Self {
            slots,
            slot_components: vec![ItemComponentPatch::default(); PLAYER_INVENTORY_SLOTS],
            craft3x3: [SimpleSlot::default(); 10],
            custom_recipes: Vec::new(),
            carried: SimpleSlot::default(),
            carried_components: ItemComponentPatch::default(),
            selected_hotbar: 0,
            state_id: 1,
            registries,
        }
    }
}

impl PlayerInventory {
    /// Updates the selected hotbar slot (`0..=8`).
    pub fn set_selected_hotbar(&mut self, slot: i16) {
        if (0..=8).contains(&slot) {
            self.selected_hotbar = slot as u8;
        }
    }

    /// Returns the currently held main-hand item slot.
    #[must_use]
    pub fn held_item(&self) -> SimpleSlot {
        let idx = HOTBAR_START_SLOT + usize::from(self.selected_hotbar.min(8));
        self.slots.get(idx).copied().unwrap_or_default()
    }

    /// Builds a [`ContainerSetContent`] packet for syncing the player inventory.
    #[must_use]
    pub fn to_container_set_content(&self) -> ContainerSetContent {
        ContainerSetContent {
            window_id: VarInt(0),
            state_id: VarInt(self.state_id),
            slots: self.slots.clone(),
            carried: self.carried,
        }
    }

    /// Builds a [`ContainerSetContentStacks`] packet for syncing player inventory with components.
    #[must_use]
    pub fn to_container_set_content_stacks(&self) -> ContainerSetContentStacks {
        let slots = self
            .slots
            .iter()
            .enumerate()
            .map(|(i, s)| ItemStack {
                count: s.count,
                item_id: s.item_id,
                components: self.slot_components.get(i).cloned().unwrap_or_default(),
            })
            .collect();
        ContainerSetContentStacks {
            window_id: VarInt(0),
            state_id: VarInt(self.state_id),
            slots,
            carried: ItemStack {
                count: self.carried.count,
                item_id: self.carried.item_id,
                components: self.carried_components.clone(),
            },
        }
    }

    /// Builds a [`ContainerSetContent`] packet for an open 27-slot chest window (`window_id >= 1`).
    #[allow(dead_code)]
    #[must_use]
    pub fn to_chest_container_set_content(
        &self,
        window_id: i32,
        chest_slots: &[SimpleSlot],
    ) -> ContainerSetContent {
        let mut slots = Vec::with_capacity(27 + 36);
        for i in 0..27 {
            slots.push(chest_slots.get(i).copied().unwrap_or_default());
        }
        // Chest screen appends main inventory (9..36) + hotbar (36..45)
        slots.extend_from_slice(&self.slots[9..45]);
        ContainerSetContent {
            window_id: VarInt(window_id),
            state_id: VarInt(self.state_id),
            slots,
            carried: self.carried,
        }
    }

    /// Builds a [`ContainerSetContentStacks`] packet for an open 27-slot chest window with full components.
    #[must_use]
    pub fn to_chest_container_set_content_stacks(
        &self,
        window_id: i32,
        chest_slots: &[SimpleSlot],
        chest_components: &[ItemComponentPatch],
    ) -> ContainerSetContentStacks {
        let mut slots = Vec::with_capacity(27 + 36);
        for i in 0..27 {
            let s = chest_slots.get(i).copied().unwrap_or_default();
            let c = if s.count > 0 {
                chest_components.get(i).cloned().unwrap_or_default()
            } else {
                ItemComponentPatch::default()
            };
            slots.push(ItemStack {
                count: s.count,
                item_id: s.item_id,
                components: c,
            });
        }
        for (idx, s) in self.slots[9..45].iter().enumerate() {
            let pidx = 9 + idx;
            let c = if s.count > 0 {
                self.slot_components.get(pidx).cloned().unwrap_or_default()
            } else {
                ItemComponentPatch::default()
            };
            slots.push(ItemStack {
                count: s.count,
                item_id: s.item_id,
                components: c,
            });
        }
        ContainerSetContentStacks {
            window_id: VarInt(window_id),
            state_id: VarInt(self.state_id),
            slots,
            carried: ItemStack {
                count: self.carried.count,
                item_id: self.carried.item_id,
                components: self.carried_components.clone(),
            },
        }
    }

    /// Builds a [`ContainerSetContent`] packet for an open 3x3 Crafting Table (`window_id == 2`).
    #[must_use]
    pub fn to_crafting_container_set_content(&self, window_id: i32) -> ContainerSetContent {
        let mut slots = Vec::with_capacity(10 + 36);
        slots.extend_from_slice(&self.craft3x3);
        slots.extend_from_slice(&self.slots[9..45]);
        ContainerSetContent {
            window_id: VarInt(window_id),
            state_id: VarInt(self.state_id),
            slots,
            carried: self.carried,
        }
    }

    /// Builds a [`ContainerSetContent`] packet for an open Furnace (`window_id == 3`).
    #[must_use]
    pub fn to_furnace_container_set_content(
        &self,
        window_id: i32,
        input: SimpleSlot,
        fuel: SimpleSlot,
        output: SimpleSlot,
    ) -> ContainerSetContent {
        let mut slots = Vec::with_capacity(3 + 36);
        slots.push(input);
        slots.push(fuel);
        slots.push(output);
        slots.extend_from_slice(&self.slots[9..45]);
        ContainerSetContent {
            window_id: VarInt(window_id),
            state_id: VarInt(self.state_id),
            slots,
            carried: self.carried,
        }
    }

    /// Processes a click on an open Furnace window (`0`=input, `1`=fuel, `2`=output, `3..=38`=player).
    /// Returns `Ok(true)` if the furnace slots were modified.
    #[allow(clippy::result_unit_err)]
    pub fn apply_furnace_click(
        &mut self,
        state_id: i32,
        slot: i16,
        button: i8,
        mode: i32,
        furnace_slots: &mut [SimpleSlot; 3],
    ) -> Result<bool, ()> {
        if state_id != self.state_id {
            return Err(());
        }
        let Ok(slot_idx) = usize::try_from(slot) else {
            return Err(());
        };
        if slot_idx >= 3 + 36 {
            return Err(());
        }
        let mut modified = false;
        if mode == 1 {
            if slot_idx < 3 {
                let item = furnace_slots[slot_idx];
                if !is_empty(item) && self.insert_into_player_main_or_hotbar(item) {
                    furnace_slots[slot_idx] = SimpleSlot::default();
                    modified = true;
                    self.state_id = self.state_id.wrapping_add(1);
                }
            } else {
                let pidx = 9 + (slot_idx - 3);
                let item = self.slots[pidx];
                if !is_empty(item) {
                    // Try fuel slot if it's fuel, else input slot
                    let target = if crate::world::sim::FurnaceEntity::fuel_ticks_with(
                        item.item_id,
                        self.registries.provider().as_ref(),
                    )
                    .is_some()
                        && is_empty(furnace_slots[1])
                    {
                        1
                    } else {
                        0
                    };
                    if is_empty(furnace_slots[target]) {
                        furnace_slots[target] = item;
                        self.slots[pidx] = SimpleSlot::default();
                        modified = true;
                        self.state_id = self.state_id.wrapping_add(1);
                    } else if furnace_slots[target].item_id == item.item_id
                        && furnace_slots[target].count < 64
                    {
                        let add = (64 - furnace_slots[target].count).min(item.count);
                        furnace_slots[target].count += add;
                        self.slots[pidx].count -= add;
                        if self.slots[pidx].count <= 0 {
                            self.slots[pidx] = SimpleSlot::default();
                        }
                        modified = true;
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                }
            }
            return Ok(modified);
        }
        if mode == 0 {
            if slot_idx < 3 {
                let before = furnace_slots[slot_idx];
                if slot_idx == 2 {
                    // Output slot: only take items out, never place into output
                    if !is_empty(before)
                        && (is_empty(self.carried)
                            || (self.carried.item_id == before.item_id
                                && self.carried.count + before.count <= 64))
                    {
                        if is_empty(self.carried) {
                            self.carried = before;
                        } else {
                            self.carried.count += before.count;
                        }
                        furnace_slots[2] = SimpleSlot::default();
                        modified = true;
                    }
                } else {
                    apply_pickup_or_place(&mut furnace_slots[slot_idx], &mut self.carried, button);
                    modified = furnace_slots[slot_idx] != before;
                }
            } else {
                let pidx = 9 + (slot_idx - 3);
                apply_pickup_or_place(&mut self.slots[pidx], &mut self.carried, button);
            }
            self.state_id = self.state_id.wrapping_add(1);
        } else if mode == 2
            && let Some(swap_idx) = hotbar_swap_target(button)
        {
            if slot_idx < 3 {
                if slot_idx != 2 || is_empty(self.slots[swap_idx]) {
                    std::mem::swap(&mut furnace_slots[slot_idx], &mut self.slots[swap_idx]);
                    modified = true;
                }
            } else {
                let pidx = 9 + (slot_idx - 3);
                self.slots.swap(pidx, swap_idx);
            }
            self.state_id = self.state_id.wrapping_add(1);
        }
        Ok(modified)
    }

    /// Returns unconsumed items in the 3x3 Crafting Table grid back to player inventory.
    pub fn close_craft3x3(&mut self) {
        for idx in 1..=9 {
            let item = self.craft3x3[idx];
            if !is_empty(item) {
                let _ = self.insert_into_player_main_or_hotbar(item);
                self.craft3x3[idx] = SimpleSlot::default();
            }
        }
        self.craft3x3[0] = SimpleSlot::default();
        self.state_id = self.state_id.wrapping_add(1);
    }

    /// Recomputes 3x3 Crafting Table output (`craft3x3[0]`) from grid `craft3x3[1..=9]`.
    pub fn update_craft3x3_result(&mut self) {
        let items = &self.registries.items;
        let g = &self.craft3x3[1..=9];
        let has = |i: usize, id: i32| g[i].item_id == id && g[i].count > 0;
        let emp = |i: usize| g[i].count <= 0 || g[i].item_id <= 0;
        let ring = |id: i32| [0, 1, 2, 3, 5, 6, 7, 8].iter().all(|&i| has(i, id)) && emp(4);

        let result = if ring(items.oak_planks) {
            SimpleSlot {
                count: 1,
                item_id: items.chest,
            }
        } else if ring(62) {
            SimpleSlot {
                count: 1,
                item_id: items.furnace,
            }
        } else if has(0, items.oak_planks)
            && has(1, items.oak_planks)
            && has(2, items.oak_planks)
            && emp(3)
            && has(4, items.stick)
            && emp(5)
            && emp(6)
            && has(7, items.stick)
            && emp(8)
        {
            SimpleSlot {
                count: 1,
                item_id: items.wooden_pickaxe,
            }
        } else if has(0, items.oak_planks)
            && has(1, items.oak_planks)
            && has(3, items.oak_planks)
            && has(4, items.oak_planks)
            && [2, 5, 6, 7, 8].iter().all(|&i| emp(i))
        {
            SimpleSlot {
                count: 1,
                item_id: items.crafting_table,
            }
        } else if has(0, items.oak_planks)
            && has(3, items.oak_planks)
            && [1, 2, 4, 5, 6, 7, 8].iter().all(|&i| emp(i))
        {
            SimpleSlot {
                count: 4,
                item_id: items.stick,
            }
        } else if !self.custom_recipes.is_empty() {
            let grid: [SimpleSlot; 9] = g.try_into().unwrap_or([SimpleSlot::default(); 9]);
            match_custom_recipe(&grid, &self.custom_recipes)
        } else {
            SimpleSlot::default()
        };
        self.craft3x3[0] = result;
    }

    fn consume_craft3x3_ingredients(&mut self) {
        PlayerInventory::consume_craft_slice(&mut self.craft3x3[1..=9]);
    }

    /// Recomputes 2x2 crafting output (slot `0`) from crafting grid slots `1..=4`.
    pub fn update_craft_result(&mut self) {
        let g = [self.slots[1], self.slots[2], self.slots[3], self.slots[4]];
        let non_empty: Vec<(usize, SimpleSlot)> = g
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, s)| !is_empty(*s))
            .collect();
        let items = &self.registries.items;
        let result = if g
            .iter()
            .all(|s| s.item_id == items.oak_planks && s.count > 0)
        {
            SimpleSlot {
                count: 1,
                item_id: items.crafting_table,
            }
        } else if non_empty.len() == 1 && non_empty[0].1.item_id == items.oak_log {
            SimpleSlot {
                count: 4,
                item_id: items.oak_planks,
            }
        } else if (g[0].item_id == items.oak_planks
            && g[0].count > 0
            && g[2].item_id == items.oak_planks
            && g[2].count > 0
            && g[1].count == 0
            && g[3].count == 0)
            || (g[1].item_id == items.oak_planks
                && g[1].count > 0
                && g[3].item_id == items.oak_planks
                && g[3].count > 0
                && g[0].count == 0
                && g[2].count == 0)
        {
            SimpleSlot {
                count: 4,
                item_id: items.stick,
            }
        } else if (g[0].item_id == items.coal
            && g[0].count > 0
            && g[2].item_id == items.stick
            && g[2].count > 0
            && g[1].count == 0
            && g[3].count == 0)
            || (g[1].item_id == items.coal
                && g[1].count > 0
                && g[3].item_id == items.stick
                && g[3].count > 0
                && g[0].count == 0
                && g[2].count == 0)
        {
            SimpleSlot {
                count: 4,
                item_id: items.torch,
            }
        } else {
            SimpleSlot::default()
        };
        self.slots[0] = result;
    }

    /// Applies an authoritative `ContainerClick` to player inventory or an open 27-slot chest.
    ///
    /// Returns `Ok(changed_chest)` if valid, or `Err(())` when `state_id` / bounds fail.
    #[cfg_attr(not(test), allow(dead_code))]
    #[allow(clippy::result_unit_err)]
    pub fn apply_container_click(
        &mut self,
        window_id: i32,
        expected_state_id: i32,
        slot: i16,
        button: i8,
        mode: i32,
        open_chest: Option<&mut Vec<SimpleSlot>>,
    ) -> Result<bool, ()> {
        self.apply_container_click_with_components(
            window_id,
            expected_state_id,
            slot,
            button,
            mode,
            open_chest.map(|c| (c, None)),
        )
    }

    /// Applies an authoritative `ContainerClick` while preserving [`ItemComponentPatch`] metadata.
    #[allow(clippy::result_unit_err)]
    pub fn apply_container_click_with_components(
        &mut self,
        window_id: i32,
        expected_state_id: i32,
        slot: i16,
        button: i8,
        mode: i32,
        open_chest: Option<(&mut Vec<SimpleSlot>, Option<&mut Vec<ItemComponentPatch>>)>,
    ) -> Result<bool, ()> {
        self.apply_container_click_with_drop(
            window_id,
            expected_state_id,
            slot,
            button,
            mode,
            open_chest,
        )
        .map(|(chest_modified, _)| chest_modified)
    }

    /// Applies an authoritative `ContainerClick`, returning `(chest_modified, dropped_stack)`.
    #[allow(clippy::result_unit_err)]
    pub fn apply_container_click_with_drop(
        &mut self,
        window_id: i32,
        expected_state_id: i32,
        slot: i16,
        button: i8,
        mode: i32,
        open_chest: Option<(&mut Vec<SimpleSlot>, Option<&mut Vec<ItemComponentPatch>>)>,
    ) -> Result<(bool, Option<ItemStack>), ()> {
        if expected_state_id != self.state_id {
            return Err(());
        }
        if slot == -999 {
            // Clicking outside drops carried stack
            let mut dropped = None;
            if mode == 0 && !is_empty(self.carried) {
                if button == 1 {
                    dropped = Some(ItemStack {
                        item_id: self.carried.item_id,
                        count: 1,
                        components: self.carried_components.clone(),
                    });
                    self.carried.count -= 1;
                    if self.carried.count <= 0 {
                        self.carried = SimpleSlot::default();
                        self.carried_components = ItemComponentPatch::default();
                    }
                } else {
                    dropped = Some(ItemStack {
                        item_id: self.carried.item_id,
                        count: self.carried.count,
                        components: std::mem::take(&mut self.carried_components),
                    });
                    self.carried = SimpleSlot::default();
                }
                self.state_id = self.state_id.wrapping_add(1);
            }
            return Ok((false, dropped));
        }
        let Ok(slot_idx) = usize::try_from(slot) else {
            return Err(());
        };

        // Player 2x2 craft result slot (window 0, slot 0)
        if window_id == 0 && slot_idx == 0 {
            self.update_craft_result();
            let crafted = self.slots[0];
            if is_empty(crafted) {
                return Ok((false, None));
            }
            if mode == 1 {
                if self.insert_into_player_main_or_hotbar(crafted) {
                    self.consume_craft_ingredients();
                    self.update_craft_result();
                    self.state_id = self.state_id.wrapping_add(1);
                }
                return Ok((false, None));
            }
            if mode == 0
                && (is_empty(self.carried)
                    || (self.carried.item_id == crafted.item_id
                        && self.carried.count + crafted.count <= 64))
            {
                if is_empty(self.carried) {
                    self.carried = crafted;
                } else {
                    self.carried.count += crafted.count;
                }
                self.consume_craft_ingredients();
                self.update_craft_result();
                self.state_id = self.state_id.wrapping_add(1);
            }
            return Ok((false, None));
        }

        if window_id == CRAFTING_WINDOW_ID {
            if slot_idx >= 10 + 36 {
                return Err(());
            }
            if slot_idx == 0 {
                self.update_craft3x3_result();
                let crafted = self.craft3x3[0];
                if is_empty(crafted) {
                    return Ok((false, None));
                }
                if mode == 1 {
                    if self.insert_into_player_main_or_hotbar(crafted) {
                        self.consume_craft3x3_ingredients();
                        self.update_craft3x3_result();
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                    return Ok((false, None));
                }
                if mode == 0
                    && (is_empty(self.carried)
                        || (self.carried.item_id == crafted.item_id
                            && self.carried.count + crafted.count <= 64))
                {
                    if is_empty(self.carried) {
                        self.carried = crafted;
                    } else {
                        self.carried.count += crafted.count;
                    }
                    self.consume_craft3x3_ingredients();
                    self.update_craft3x3_result();
                    self.state_id = self.state_id.wrapping_add(1);
                }
                return Ok((false, None));
            }
            if mode == 1 {
                if slot_idx < 10 {
                    let item = self.craft3x3[slot_idx];
                    if !is_empty(item) && self.insert_into_player_main_or_hotbar(item) {
                        self.craft3x3[slot_idx] = SimpleSlot::default();
                        self.update_craft3x3_result();
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                } else {
                    let pidx = slot_idx - 10 + 9;
                    let item = self.slots[pidx];
                    if !is_empty(item) && insert_into_range(&mut self.craft3x3, 1..10, item) {
                        self.slots[pidx] = SimpleSlot::default();
                        self.update_craft3x3_result();
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                }
                return Ok((false, None));
            }
            if mode == 0 {
                if slot_idx < 10 {
                    apply_pickup_or_place(&mut self.craft3x3[slot_idx], &mut self.carried, button);
                    self.update_craft3x3_result();
                } else {
                    let pidx = slot_idx - 10 + 9;
                    apply_pickup_or_place(&mut self.slots[pidx], &mut self.carried, button);
                }
                self.state_id = self.state_id.wrapping_add(1);
            } else if mode == 2
                && let Some(swap_idx) = hotbar_swap_target(button)
            {
                if slot_idx < 10 {
                    std::mem::swap(&mut self.craft3x3[slot_idx], &mut self.slots[swap_idx]);
                    self.update_craft3x3_result();
                } else {
                    let pidx = slot_idx - 10 + 9;
                    self.slots.swap(pidx, swap_idx);
                }
                self.state_id = self.state_id.wrapping_add(1);
            }
            return Ok((false, None));
        }

        let mut chest_modified = false;
        if window_id > 0 {
            let Some((chest, mut chest_comps)) = open_chest else {
                return Err(());
            };
            if slot_idx >= 27 + 36 {
                return Err(());
            }
            if mode == 1 {
                // Shift-click between chest (0..27) and player (27..63 -> slots 9..45)
                if slot_idx < 27 {
                    let item = chest[slot_idx];
                    let comp = chest_comps
                        .as_deref()
                        .and_then(|cc| cc.get(slot_idx))
                        .cloned()
                        .unwrap_or_default();
                    if !is_empty(item)
                        && self.insert_into_player_main_or_hotbar_with_comp(item, comp)
                    {
                        chest[slot_idx] = SimpleSlot::default();
                        if let Some(cc) = chest_comps.as_deref_mut()
                            && let Some(c) = cc.get_mut(slot_idx)
                        {
                            *c = ItemComponentPatch::default();
                        }
                        chest_modified = true;
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                } else {
                    let player_idx = 9 + (slot_idx - 27);
                    let item = self.slots[player_idx];
                    let comp = self
                        .slot_components
                        .get(player_idx)
                        .cloned()
                        .unwrap_or_default();
                    if !is_empty(item)
                        && insert_into_slice_with_comp(
                            chest,
                            chest_comps.as_deref_mut(),
                            item,
                            comp,
                        )
                    {
                        self.slots[player_idx] = SimpleSlot::default();
                        if let Some(c) = self.slot_components.get_mut(player_idx) {
                            *c = ItemComponentPatch::default();
                        }
                        chest_modified = true;
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                }
                return Ok((chest_modified, None));
            }
            if mode == 0 {
                if slot_idx < 27 {
                    chest_modified = true;
                    let mut dummy_comp = ItemComponentPatch::default();
                    let target_comp = chest_comps
                        .as_deref_mut()
                        .and_then(|cc| cc.get_mut(slot_idx))
                        .unwrap_or(&mut dummy_comp);
                    apply_pickup_or_place_with_comp(
                        &mut chest[slot_idx],
                        target_comp,
                        &mut self.carried,
                        &mut self.carried_components,
                        button,
                    );
                } else {
                    let pidx = 9 + (slot_idx - 27);
                    apply_pickup_or_place_with_comp(
                        &mut self.slots[pidx],
                        &mut self.slot_components[pidx],
                        &mut self.carried,
                        &mut self.carried_components,
                        button,
                    );
                }
                self.state_id = self.state_id.wrapping_add(1);
                return Ok((chest_modified, None));
            }
            if mode == 2
                && let Some(swap_idx) = hotbar_swap_target(button)
            {
                if slot_idx < 27 {
                    std::mem::swap(&mut chest[slot_idx], &mut self.slots[swap_idx]);
                    if let Some(cc) = chest_comps
                        && let Some(c) = cc.get_mut(slot_idx)
                    {
                        std::mem::swap(c, &mut self.slot_components[swap_idx]);
                    }
                    chest_modified = true;
                } else {
                    let pidx = 9 + (slot_idx - 27);
                    self.slots.swap(pidx, swap_idx);
                    self.slot_components.swap(pidx, swap_idx);
                }
                self.state_id = self.state_id.wrapping_add(1);
                return Ok((chest_modified, None));
            }
            if mode == 6 && !is_empty(self.carried) && self.carried.count < 64 {
                chest_modified |= collect_matching_with_comp(
                    &mut self.carried,
                    &self.carried_components,
                    chest,
                    chest_comps.as_deref_mut().map(Vec::as_mut_slice),
                );
                collect_matching_with_comp(
                    &mut self.carried,
                    &self.carried_components,
                    &mut self.slots[9..45],
                    Some(&mut self.slot_components[9..45]),
                );
                self.state_id = self.state_id.wrapping_add(1);
                return Ok((chest_modified, None));
            }
            if mode == 4 {
                let mut dropped = None;
                if slot_idx < 27 && !is_empty(chest[slot_idx]) {
                    let comp = chest_comps
                        .as_ref()
                        .and_then(|cc| cc.get(slot_idx))
                        .cloned()
                        .unwrap_or_default();
                    if button == 1 {
                        dropped = Some(ItemStack {
                            item_id: chest[slot_idx].item_id,
                            count: chest[slot_idx].count,
                            components: comp,
                        });
                        chest[slot_idx] = SimpleSlot::default();
                    } else {
                        dropped = Some(ItemStack {
                            item_id: chest[slot_idx].item_id,
                            count: 1,
                            components: comp,
                        });
                        chest[slot_idx].count -= 1;
                        if chest[slot_idx].count <= 0 {
                            chest[slot_idx] = SimpleSlot::default();
                        }
                    }
                    if is_empty(chest[slot_idx])
                        && let Some(cc) = chest_comps
                        && let Some(c) = cc.get_mut(slot_idx)
                    {
                        *c = ItemComponentPatch::default();
                    }
                    chest_modified = true;
                    self.state_id = self.state_id.wrapping_add(1);
                } else if slot_idx >= 27 {
                    let pidx = 9 + (slot_idx - 27);
                    if !is_empty(self.slots[pidx]) {
                        let comp = self.slot_components[pidx].clone();
                        if button == 1 {
                            dropped = Some(ItemStack {
                                item_id: self.slots[pidx].item_id,
                                count: self.slots[pidx].count,
                                components: comp,
                            });
                            self.slots[pidx] = SimpleSlot::default();
                        } else {
                            dropped = Some(ItemStack {
                                item_id: self.slots[pidx].item_id,
                                count: 1,
                                components: comp,
                            });
                            self.slots[pidx].count -= 1;
                            if self.slots[pidx].count <= 0 {
                                self.slots[pidx] = SimpleSlot::default();
                            }
                        }
                        if is_empty(self.slots[pidx]) {
                            self.slot_components[pidx] = ItemComponentPatch::default();
                        }
                        self.state_id = self.state_id.wrapping_add(1);
                    }
                }
                return Ok((chest_modified, dropped));
            }
            return Ok((false, None));
        }

        // Player inventory window_id == 0, slot_idx in 1..46
        if slot_idx >= PLAYER_INVENTORY_SLOTS {
            return Err(());
        }
        if mode == 1 {
            let item = self.slots[slot_idx];
            if !is_empty(item) {
                let comp = self.slot_components[slot_idx].clone();
                let target_range = if (HOTBAR_START_SLOT..45).contains(&slot_idx) {
                    9..HOTBAR_START_SLOT
                } else {
                    HOTBAR_START_SLOT..45
                };
                if insert_into_range_with_comp(
                    &mut self.slots,
                    &mut self.slot_components,
                    target_range,
                    item,
                    comp,
                ) {
                    self.slots[slot_idx] = SimpleSlot::default();
                    self.slot_components[slot_idx] = ItemComponentPatch::default();
                }
                if (1..=4).contains(&slot_idx) {
                    self.update_craft_result();
                }
                self.state_id = self.state_id.wrapping_add(1);
            }
            return Ok((false, None));
        }
        if mode == 0 {
            apply_pickup_or_place_with_comp(
                &mut self.slots[slot_idx],
                &mut self.slot_components[slot_idx],
                &mut self.carried,
                &mut self.carried_components,
                button,
            );
            if (1..=4).contains(&slot_idx) {
                self.update_craft_result();
            }
            self.state_id = self.state_id.wrapping_add(1);
        } else if mode == 2
            && slot_idx >= 1
            && let Some(swap_idx) = hotbar_swap_target(button)
        {
            self.slots.swap(slot_idx, swap_idx);
            self.slot_components.swap(slot_idx, swap_idx);
            if (1..=4).contains(&slot_idx) || (1..=4).contains(&swap_idx) {
                self.update_craft_result();
            }
            self.state_id = self.state_id.wrapping_add(1);
        } else if mode == 6 && !is_empty(self.carried) && self.carried.count < 64 {
            collect_matching_with_comp(
                &mut self.carried,
                &self.carried_components,
                &mut self.slots[9..45],
                Some(&mut self.slot_components[9..45]),
            );
            self.state_id = self.state_id.wrapping_add(1);
        } else if mode == 4 && !is_empty(self.slots[slot_idx]) {
            let comp = self.slot_components[slot_idx].clone();
            let dropped = if button == 1 {
                let d = ItemStack {
                    item_id: self.slots[slot_idx].item_id,
                    count: self.slots[slot_idx].count,
                    components: comp,
                };
                self.slots[slot_idx] = SimpleSlot::default();
                d
            } else {
                let d = ItemStack {
                    item_id: self.slots[slot_idx].item_id,
                    count: 1,
                    components: comp,
                };
                self.slots[slot_idx].count -= 1;
                if self.slots[slot_idx].count <= 0 {
                    self.slots[slot_idx] = SimpleSlot::default();
                }
                d
            };
            if is_empty(self.slots[slot_idx]) {
                self.slot_components[slot_idx] = ItemComponentPatch::default();
            }
            if (1..=4).contains(&slot_idx) {
                self.update_craft_result();
            }
            self.state_id = self.state_id.wrapping_add(1);
            return Ok((false, Some(dropped)));
        }
        Ok((false, None))
    }

    fn consume_craft_slice(slots: &mut [SimpleSlot]) {
        for slot in slots {
            if slot.count > 0 {
                slot.count -= 1;
                if slot.count == 0 {
                    slot.item_id = 0;
                }
            }
        }
    }

    fn consume_craft_ingredients(&mut self) {
        Self::consume_craft_slice(&mut self.slots[1..=4]);
    }

    fn insert_into_player_main_or_hotbar(&mut self, item: SimpleSlot) -> bool {
        insert_into_range(&mut self.slots, HOTBAR_START_SLOT..45, item)
            || insert_into_range(&mut self.slots, 9..HOTBAR_START_SLOT, item)
    }

    fn insert_into_player_main_or_hotbar_with_comp(
        &mut self,
        item: SimpleSlot,
        comp: ItemComponentPatch,
    ) -> bool {
        insert_into_range_with_comp(
            &mut self.slots,
            &mut self.slot_components,
            HOTBAR_START_SLOT..45,
            item,
            comp.clone(),
        ) || insert_into_range_with_comp(
            &mut self.slots,
            &mut self.slot_components,
            9..HOTBAR_START_SLOT,
            item,
            comp,
        )
    }

    /// Swaps the currently selected hotbar slot with the offhand slot (`45`).
    pub fn swap_offhand(&mut self) {
        let main_idx = HOTBAR_START_SLOT + usize::from(self.selected_hotbar.min(8));
        self.slots.swap(main_idx, 45);
        self.slot_components.swap(main_idx, 45);
        self.state_id = self.state_id.wrapping_add(1);
    }

    /// Inserts `count` of `item_id` into hotbar or main inventory, returning `true` if added.
    pub fn give_item(&mut self, item_id: i32, count: i32) -> bool {
        if item_id <= 0 || count <= 0 {
            return false;
        }
        let ok = self.insert_into_player_main_or_hotbar(SimpleSlot {
            count: count.min(64),
            item_id,
        });
        if ok {
            self.state_id = self.state_id.wrapping_add(1);
        }
        ok
    }

    /// Consumes one item from the currently selected hotbar slot. Returns `false` if empty.
    pub fn consume_held_item(&mut self) -> bool {
        let idx = HOTBAR_START_SLOT + usize::from(self.selected_hotbar.min(8));
        let Some(slot) = self.slots.get_mut(idx) else {
            return false;
        };
        if slot.count <= 0 || slot.item_id <= 0 {
            return false;
        }
        slot.count -= 1;
        if slot.count == 0 {
            slot.item_id = 0;
        }
        self.state_id = self.state_id.wrapping_add(1);
        true
    }

    /// Consumes `amount` of `item_id` across hotbar/main inventory slots (returns `false` if insufficient).
    pub fn consume_item_by_id(&mut self, item_id: i32, amount: i32) -> bool {
        if amount <= 0 {
            return true;
        }
        let total: i32 = self.slots[9..PLAYER_INVENTORY_SLOTS]
            .iter()
            .filter(|s| s.item_id == item_id && s.count > 0)
            .map(|s| s.count)
            .sum();
        if total < amount {
            return false;
        }
        let mut remaining = amount;
        for slot in &mut self.slots[9..PLAYER_INVENTORY_SLOTS] {
            if remaining == 0 {
                break;
            }
            if slot.item_id == item_id && slot.count > 0 {
                let take = remaining.min(slot.count);
                slot.count -= take;
                remaining -= take;
                if slot.count == 0 {
                    slot.item_id = 0;
                }
            }
        }
        self.state_id = self.state_id.wrapping_add(1);
        true
    }

    /// Adds a single block drop item to an existing matching stack or first empty hotbar/main slot.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn give_block_drop(&mut self, block_state: u16) -> bool {
        let Some(item_id) = block_state_to_item_id_with(block_state, &self.registries) else {
            return false;
        };
        for idx in HOTBAR_START_SLOT..PLAYER_INVENTORY_SLOTS {
            if let Some(slot) = self.slots.get_mut(idx)
                && slot.item_id == item_id
                && slot.count > 0
                && slot.count < 64
            {
                slot.count += 1;
                self.state_id = self.state_id.wrapping_add(1);
                return true;
            }
        }
        for idx in 9..PLAYER_INVENTORY_SLOTS {
            if let Some(slot) = self.slots.get_mut(idx)
                && slot.item_id == item_id
                && slot.count > 0
                && slot.count < 64
            {
                slot.count += 1;
                self.state_id = self.state_id.wrapping_add(1);
                return true;
            }
        }
        for idx in (HOTBAR_START_SLOT..PLAYER_INVENTORY_SLOTS).chain(9..HOTBAR_START_SLOT) {
            if let Some(slot) = self.slots.get_mut(idx)
                && slot.count <= 0
            {
                *slot = SimpleSlot { count: 1, item_id };
                self.state_id = self.state_id.wrapping_add(1);
                return true;
            }
        }
        false
    }

    /// Peeks the [`ItemStack`] (including [`ItemComponentPatch`]) that would be dropped
    /// by `Q` (`full_stack == false`) or `Ctrl+Q` (`full_stack == true`) without mutating the inventory.
    #[must_use]
    pub fn peek_drop_selected_hotbar(&self, full_stack: bool) -> Option<ItemStack> {
        let idx = HOTBAR_START_SLOT + usize::from(self.selected_hotbar.min(8));
        let slot = self.slots.get(idx)?;
        if slot.count <= 0 || slot.item_id <= 0 {
            return None;
        }
        let count = if full_stack || slot.count <= 1 {
            slot.count
        } else {
            1
        };
        Some(ItemStack {
            item_id: slot.item_id,
            count,
            components: self.slot_components.get(idx).cloned().unwrap_or_default(),
        })
    }

    /// Inserts as much of `stack` as possible into hotbar (`36..45`) then main inventory (`9..36`),
    /// preserving [`ItemComponentPatch`] and only stacking onto slots with identical components.
    /// Decrements `stack.count` by the amount inserted and returns that amount.
    pub fn insert_item_stack(&mut self, stack: &mut ItemStack) -> i32 {
        if stack.item_id <= 0 || stack.count <= 0 {
            return 0;
        }
        let max_stack = if stack.components.damage.is_some() {
            1
        } else {
            64
        };
        let mut inserted = 0;

        // Pass 1: merge into existing matching stacks (hotbar first, then main inventory)
        for idx in (HOTBAR_START_SLOT..45).chain(9..HOTBAR_START_SLOT) {
            if stack.count <= 0 {
                break;
            }
            let slot = self.slots[idx];
            if slot.item_id == stack.item_id
                && slot.count > 0
                && slot.count < max_stack
                && self.slot_components[idx] == stack.components
            {
                let add = (max_stack - slot.count).min(stack.count);
                self.slots[idx].count += add;
                stack.count -= add;
                inserted += add;
            }
        }

        // Pass 2: place into empty slots (hotbar first, then main inventory)
        for idx in (HOTBAR_START_SLOT..45).chain(9..HOTBAR_START_SLOT) {
            if stack.count <= 0 {
                break;
            }
            if is_empty(self.slots[idx]) {
                let add = stack.count.min(max_stack);
                self.slots[idx] = SimpleSlot {
                    count: add,
                    item_id: stack.item_id,
                };
                self.slot_components[idx] = stack.components.clone();
                stack.count -= add;
                inserted += add;
            }
        }

        if inserted > 0 {
            self.state_id = self.state_id.wrapping_add(1);
        }
        inserted
    }

    /// Drops 1 item (`full_stack == false`, `Q`) or the entire stack (`full_stack == true`, `Ctrl+Q`)
    /// from the selected hotbar slot, returning `true` if the slot changed.
    pub fn drop_selected_hotbar(&mut self, full_stack: bool) -> bool {
        let idx = HOTBAR_START_SLOT + usize::from(self.selected_hotbar.min(8));
        let Some(slot) = self.slots.get_mut(idx) else {
            return false;
        };
        if slot.count <= 0 || slot.item_id <= 0 {
            return false;
        }
        if full_stack || slot.count <= 1 {
            *slot = SimpleSlot::default();
            if let Some(comp) = self.slot_components.get_mut(idx) {
                *comp = ItemComponentPatch::default();
            }
        } else {
            slot.count -= 1;
        }
        self.state_id = self.state_id.wrapping_add(1);
        true
    }

    /// Applies a Creative-mode slot update (`SetCreativeModeSlot`).
    #[allow(dead_code)]
    pub fn set_slot(&mut self, slot_idx: usize, item: SimpleSlot) -> bool {
        let Some(slot) = self.slots.get_mut(slot_idx) else {
            return false;
        };
        *slot = if item.count <= 0 || item.item_id <= 0 {
            SimpleSlot::default()
        } else {
            SimpleSlot {
                count: item.count.min(64),
                item_id: item.item_id,
            }
        };
        self.state_id = self.state_id.wrapping_add(1);
        true
    }

    /// Applies a validated creative inventory update, retaining item components.
    pub fn set_creative_stack(&mut self, slot_idx: usize, item: ItemStack) -> bool {
        if !(1..PLAYER_INVENTORY_SLOTS).contains(&slot_idx)
            || item.count < 0
            || item.count > 64
            || (item.count > 0 && self.registries.item_name_by_id(item.item_id).is_none())
        {
            return false;
        }
        self.slots[slot_idx] = item.to_simple();
        self.slot_components[slot_idx] = if item.count == 0 {
            ItemComponentPatch::default()
        } else {
            item.components
        };
        self.state_id = self.state_id.wrapping_add(1);
        true
    }

    /// Selects an existing block item or creates it in creative mode.
    pub fn pick_block_item(&mut self, item_id: i32, creative: bool) -> Option<u8> {
        self.registries.item_name_by_id(item_id)?;
        if let Some(index) = (0..9).find(|&i| {
            self.slots[HOTBAR_START_SLOT + i].count > 0
                && self.slots[HOTBAR_START_SLOT + i].item_id == item_id
        }) {
            self.selected_hotbar = index as u8;
        } else if let Some(index) = (9..HOTBAR_START_SLOT)
            .find(|&i| self.slots[i].count > 0 && self.slots[i].item_id == item_id)
        {
            let target = HOTBAR_START_SLOT + usize::from(self.selected_hotbar.min(8));
            self.slots.swap(index, target);
            self.slot_components.swap(index, target);
            self.state_id = self.state_id.wrapping_add(1);
        } else if creative {
            let index = (0..9)
                .find(|&i| self.slots[HOTBAR_START_SLOT + i].count <= 0)
                .unwrap_or(usize::from(self.selected_hotbar.min(8)));
            self.selected_hotbar = index as u8;
            self.slots[HOTBAR_START_SLOT + index] = SimpleSlot { count: 1, item_id };
            self.slot_components[HOTBAR_START_SLOT + index] = ItemComponentPatch::default();
            self.state_id = self.state_id.wrapping_add(1);
        } else {
            return None;
        }
        Some(self.selected_hotbar)
    }
}

fn hotbar_swap_target(button: i8) -> Option<usize> {
    match button {
        0..=8 => Some(HOTBAR_START_SLOT + usize::from(button as u8)),
        40 => Some(45),
        _ => None,
    }
}

fn collect_matching_with_comp(
    carried: &mut SimpleSlot,
    carried_comp: &ItemComponentPatch,
    slots: &mut [SimpleSlot],
    mut slot_comps: Option<&mut [ItemComponentPatch]>,
) -> bool {
    let mut changed = false;
    for (idx, slot) in slots.iter_mut().enumerate() {
        if carried.count >= 64 {
            break;
        }
        let same_comp = slot_comps
            .as_deref()
            .and_then(|sc| sc.get(idx))
            .is_none_or(|c| c == carried_comp);
        if !is_empty(*slot) && slot.item_id == carried.item_id && same_comp {
            let take = (64 - carried.count).min(slot.count);
            carried.count += take;
            slot.count -= take;
            if slot.count <= 0 {
                *slot = SimpleSlot::default();
                if let Some(sc) = slot_comps.as_deref_mut()
                    && let Some(c) = sc.get_mut(idx)
                {
                    *c = ItemComponentPatch::default();
                }
            }
            changed = true;
        }
    }
    changed
}

/// Returns `(food, saturation)` restored when consuming `item_id` using `items`, or `None` if not edible.
#[must_use]
pub const fn food_nutrition_with(item_id: i32, items: &ItemIds) -> Option<(i32, f32)> {
    if item_id == items.apple {
        Some((4, 2.4))
    } else if item_id == items.bread {
        Some((5, 6.0))
    } else if item_id == items.cooked_porkchop {
        Some((8, 12.8))
    } else {
        None
    }
}

/// Returns melee attack damage for the held `item_id` using `items`.
#[must_use]
pub const fn tool_attack_damage_with(item_id: i32, items: &ItemIds) -> f32 {
    if item_id == items.wooden_pickaxe {
        7.0
    } else if item_id == items.stick {
        5.5
    } else {
        5.0
    }
}

/// Maps a held item ID to a placeable block state ID using `regs` (`None` if not a block).
#[must_use]
pub fn item_id_to_block_state_with(item_id: i32, regs: &Version26_3Registries) -> Option<u16> {
    let name = regs.item_name_by_id(item_id)?;
    let block_name = match name {
        "minecraft:air" | "minecraft:water_bucket" | "minecraft:lava_bucket" => return None,
        "minecraft:redstone" => "minecraft:redstone_wire",
        "minecraft:wheat_seeds" => "minecraft:wheat",
        "minecraft:carrot" => "minecraft:carrots",
        "minecraft:potato" => "minecraft:potatoes",
        "minecraft:beetroot_seeds" => "minecraft:beetroots",
        "minecraft:melon_seeds" => "minecraft:melon_stem",
        "minecraft:pumpkin_seeds" => "minecraft:pumpkin_stem",
        "minecraft:sweet_berries" => "minecraft:sweet_berry_bush",
        "minecraft:string" => "minecraft:tripwire",
        "minecraft:cocoa_beans" => "minecraft:cocoa",
        "minecraft:kelp" => "minecraft:kelp_plant",
        other => other,
    };
    regs.block_by_name(block_name).map(|b| b.default_state)
}

/// Maps a broken block state ID to its Survival item drop using `regs` (`None` if unbreakable/no drop).
#[must_use]
pub fn block_state_to_item_id_with(block_state: u16, regs: &Version26_3Registries) -> Option<i32> {
    if regs.is_air_state(block_state) || regs.is_fluid_state(block_state) {
        return None;
    }
    let def = regs.block_by_state(block_state)?;
    let drop_name = match def.name.as_str() {
        "minecraft:bedrock"
        | "minecraft:fire"
        | "minecraft:soul_fire"
        | "minecraft:nether_portal"
        | "minecraft:end_portal"
        | "minecraft:barrier"
        | "minecraft:command_block"
        | "minecraft:chain_command_block"
        | "minecraft:repeating_command_block"
        | "minecraft:structure_block"
        | "minecraft:jigsaw"
        | "minecraft:light"
        | "minecraft:glass"
        | "minecraft:glass_pane"
        | "minecraft:tinted_glass"
        | "minecraft:ice"
        | "minecraft:packed_ice"
        | "minecraft:blue_ice"
        | "minecraft:spawner"
        | "minecraft:piston_head"
        | "minecraft:moving_piston"
        | "minecraft:tall_grass"
        | "minecraft:short_grass"
        | "minecraft:fern"
        | "minecraft:large_fern"
        | "minecraft:dead_bush"
        | "minecraft:cobweb"
        | "minecraft:infested_stone"
        | "minecraft:infested_cobblestone"
        | "minecraft:infested_stone_bricks"
        | "minecraft:infested_mossy_stone_bricks"
        | "minecraft:infested_cracked_stone_bricks"
        | "minecraft:infested_chiseled_stone_bricks"
        | "minecraft:infested_deepslate" => return None,
        "minecraft:stone" => "minecraft:cobblestone",
        "minecraft:deepslate" => "minecraft:cobbled_deepslate",
        "minecraft:grass_block"
        | "minecraft:podzol"
        | "minecraft:mycelium"
        | "minecraft:farmland"
        | "minecraft:dirt_path" => "minecraft:dirt",
        "minecraft:redstone_wire" => "minecraft:redstone",
        "minecraft:wall_torch" => "minecraft:torch",
        "minecraft:redstone_wall_torch" => "minecraft:redstone_torch",
        "minecraft:soul_wall_torch" => "minecraft:soul_torch",
        "minecraft:copper_wall_torch" => "minecraft:copper_torch",
        "minecraft:bamboo_sapling" => "minecraft:bamboo",
        "minecraft:cocoa" => "minecraft:cocoa_beans",
        "minecraft:sweet_berry_bush" => "minecraft:sweet_berries",
        "minecraft:carrots" => "minecraft:carrot",
        "minecraft:potatoes" => "minecraft:potato",
        "minecraft:beetroots" => "minecraft:beetroot",
        "minecraft:coal_ore" | "minecraft:deepslate_coal_ore" => "minecraft:coal",
        "minecraft:iron_ore" | "minecraft:deepslate_iron_ore" => "minecraft:raw_iron",
        "minecraft:copper_ore" | "minecraft:deepslate_copper_ore" => "minecraft:raw_copper",
        "minecraft:gold_ore" | "minecraft:deepslate_gold_ore" => "minecraft:raw_gold",
        "minecraft:redstone_ore" | "minecraft:deepslate_redstone_ore" => "minecraft:redstone",
        "minecraft:lapis_ore" | "minecraft:deepslate_lapis_ore" => "minecraft:lapis_lazuli",
        "minecraft:emerald_ore" | "minecraft:deepslate_emerald_ore" => "minecraft:emerald",
        "minecraft:diamond_ore" | "minecraft:deepslate_diamond_ore" => "minecraft:diamond",
        "minecraft:nether_quartz_ore" => "minecraft:quartz",
        "minecraft:nether_gold_ore" => "minecraft:gold_nugget",
        "minecraft:clay" => "minecraft:clay_ball",
        "minecraft:glowstone" => "minecraft:glowstone_dust",
        "minecraft:sea_lantern" => "minecraft:prismarine_crystals",
        "minecraft:bookshelf" => "minecraft:book",
        "minecraft:melon" => "minecraft:melon_slice",
        "minecraft:snow" | "minecraft:snow_block" => "minecraft:snowball",
        other => other,
    };
    regs.item_by_name(drop_name)
}

const fn is_empty(slot: SimpleSlot) -> bool {
    slot.count <= 0 || slot.item_id <= 0
}

fn apply_pickup_or_place(target: &mut SimpleSlot, carried: &mut SimpleSlot, button: i8) {
    let mut t_comp = ItemComponentPatch::default();
    let mut c_comp = ItemComponentPatch::default();
    apply_pickup_or_place_with_comp(target, &mut t_comp, carried, &mut c_comp, button);
}

fn apply_pickup_or_place_with_comp(
    target: &mut SimpleSlot,
    target_comp: &mut ItemComponentPatch,
    carried: &mut SimpleSlot,
    carried_comp: &mut ItemComponentPatch,
    button: i8,
) {
    if is_empty(*carried) {
        if is_empty(*target) {
            return;
        }
        if button == 1 && target.count > 1 {
            let take = (target.count + 1) / 2;
            *carried = SimpleSlot {
                count: take,
                item_id: target.item_id,
            };
            *carried_comp = target_comp.clone();
            target.count -= take;
        } else {
            *carried = *target;
            *carried_comp = std::mem::take(target_comp);
            *target = SimpleSlot::default();
        }
        return;
    }

    if is_empty(*target) {
        if button == 1 {
            *target = SimpleSlot {
                count: 1,
                item_id: carried.item_id,
            };
            *target_comp = carried_comp.clone();
            carried.count -= 1;
            if carried.count <= 0 {
                *carried = SimpleSlot::default();
                *carried_comp = ItemComponentPatch::default();
            }
        } else {
            *target = *carried;
            *target_comp = std::mem::take(carried_comp);
            *carried = SimpleSlot::default();
        }
        return;
    }

    if target.item_id == carried.item_id && *target_comp == *carried_comp {
        let space = 64 - target.count;
        if space > 0 {
            let move_n = if button == 1 {
                1
            } else {
                carried.count.min(space)
            };
            target.count += move_n;
            carried.count -= move_n;
            if carried.count <= 0 {
                *carried = SimpleSlot::default();
                *carried_comp = ItemComponentPatch::default();
            }
        }
    } else if button == 0 {
        std::mem::swap(target, carried);
        std::mem::swap(target_comp, carried_comp);
    }
}

fn insert_into_slice_with_comp(
    slice: &mut [SimpleSlot],
    mut slice_comps: Option<&mut Vec<ItemComponentPatch>>,
    mut item: SimpleSlot,
    comp: ItemComponentPatch,
) -> bool {
    for (idx, slot) in slice.iter_mut().enumerate() {
        let same_comp = slice_comps
            .as_deref()
            .and_then(|sc| sc.get(idx))
            .is_none_or(|c| *c == comp);
        if !is_empty(*slot) && slot.item_id == item.item_id && same_comp && slot.count < 64 {
            let add = item.count.min(64 - slot.count);
            slot.count += add;
            item.count -= add;
            if item.count == 0 {
                return true;
            }
        }
    }
    for (idx, slot) in slice.iter_mut().enumerate() {
        if is_empty(*slot) {
            *slot = item;
            if let Some(sc) = slice_comps.as_deref_mut()
                && let Some(c) = sc.get_mut(idx)
            {
                *c = comp;
            }
            return true;
        }
    }
    false
}

fn insert_into_range_with_comp(
    slots: &mut [SimpleSlot],
    slot_comps: &mut [ItemComponentPatch],
    range: std::ops::Range<usize>,
    mut item: SimpleSlot,
    comp: ItemComponentPatch,
) -> bool {
    for idx in range.clone() {
        if let Some(slot) = slots.get_mut(idx)
            && !is_empty(*slot)
            && slot.item_id == item.item_id
            && slot_comps.get(idx).is_none_or(|c| *c == comp)
            && slot.count < 64
        {
            let add = item.count.min(64 - slot.count);
            slot.count += add;
            item.count -= add;
            if item.count == 0 {
                return true;
            }
        }
    }
    for idx in range {
        if let Some(slot) = slots.get_mut(idx)
            && is_empty(*slot)
        {
            *slot = item;
            if let Some(c) = slot_comps.get_mut(idx) {
                *c = comp;
            }
            return true;
        }
    }
    false
}

fn insert_into_range(
    slots: &mut [SimpleSlot],
    range: std::ops::Range<usize>,
    mut item: SimpleSlot,
) -> bool {
    for idx in range.clone() {
        if let Some(slot) = slots.get_mut(idx)
            && !is_empty(*slot)
            && slot.item_id == item.item_id
            && slot.count < 64
        {
            let add = item.count.min(64 - slot.count);
            slot.count += add;
            item.count -= add;
            if item.count == 0 {
                return true;
            }
        }
    }
    for idx in range {
        if let Some(slot) = slots.get_mut(idx)
            && is_empty(*slot)
        {
            *slot = item;
            return true;
        }
    }
    false
}
