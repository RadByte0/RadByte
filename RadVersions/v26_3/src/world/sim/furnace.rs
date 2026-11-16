//! Furnace block-entity smelting simulation and persistence.

use super::*;

/// Persistent furnace block-entity state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FurnaceEntity {
    /// Input item slot.
    pub input_item: i32,
    /// Input item count.
    pub input_count: i32,
    /// Fuel item slot.
    pub fuel_item: i32,
    /// Fuel item count.
    pub fuel_count: i32,
    /// Output item slot.
    pub output_item: i32,
    /// Output item count.
    pub output_count: i32,
    /// Remaining burn ticks.
    pub burn_time: u16,
    /// Current smelt progress (`0..200`).
    pub cook_progress: u16,
}

impl FurnaceEntity {
    /// Returns the smelted output item ID for `input_item` using `provider`, if any.
    #[must_use]
    pub fn smelt_result_with(
        input_item: i32,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> Option<i32> {
        let out_name = match provider.item_name_by_id(input_item)? {
            "minecraft:cobblestone" => "minecraft:stone",
            "minecraft:sand" => "minecraft:glass",
            "minecraft:oak_log" => "minecraft:charcoal",
            "minecraft:iron_ore" => "minecraft:iron_ingot",
            "minecraft:porkchop" => "minecraft:cooked_porkchop",
            _ => return None,
        };
        provider.item_by_name(out_name)
    }

    /// Returns burn ticks provided by `fuel_item` using `provider`, if any.
    #[must_use]
    pub fn fuel_ticks_with(
        fuel_item: i32,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> Option<u16> {
        match provider.item_name_by_id(fuel_item)? {
            "minecraft:coal" | "minecraft:charcoal" => Some(1600),
            "minecraft:oak_planks" | "minecraft:oak_log" => Some(300),
            "minecraft:stick" | "test:stick" => Some(100),
            _ => None,
        }
    }

    /// Advances this furnace by one server tick using `provider`. Returns `true` if state changed.
    pub fn tick_with(
        &mut self,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) -> bool {
        let mut changed = false;
        let can_smelt = self.input_count > 0
            && Self::smelt_result_with(self.input_item, provider).is_some_and(|out_id| {
                self.output_count == 0 || (self.output_item == out_id && self.output_count < 64)
            });

        if self.burn_time > 0 {
            self.burn_time -= 1;
            changed = true;
        }

        if self.burn_time == 0
            && can_smelt
            && self.fuel_count > 0
            && let Some(ticks) = Self::fuel_ticks_with(self.fuel_item, provider)
        {
            self.fuel_count -= 1;
            if self.fuel_count == 0 {
                self.fuel_item = 0;
            }
            self.burn_time = ticks;
            changed = true;
        }

        if self.burn_time > 0 && can_smelt {
            self.cook_progress += 1;
            changed = true;
            if self.cook_progress >= 200 {
                self.cook_progress = 0;
                if let Some(out_id) = Self::smelt_result_with(self.input_item, provider) {
                    self.input_count -= 1;
                    if self.input_count == 0 {
                        self.input_item = 0;
                    }
                    self.output_item = out_id;
                    self.output_count += 1;
                }
            }
        } else if self.cook_progress > 0 {
            self.cook_progress = 0;
            changed = true;
        }
        changed
    }

    /// Converts output slot to [`SimpleSlot`].
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub const fn output_slot(&self) -> SimpleSlot {
        SimpleSlot {
            count: self.output_count,
            item_id: self.output_item,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedFurnaceRecord {
    #[serde(default)]
    dimension: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    furnace: FurnaceEntity,
}

/// Persistent store of furnace block entities (`world/furnaces.json`).
#[derive(Debug, Default)]
pub struct FurnaceStore {
    furnaces: HashMap<(DimensionId, i32, i32, i32), FurnaceEntity>,
    dirty: bool,
    revision: u64,
}

impl FurnaceStore {
    /// Loads `<world_dir>/furnaces.json` if present.
    #[must_use]
    pub fn load_from_dir(world_dir: &Path) -> Self {
        let path = world_dir.join("furnaces.json");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        let records = match serde_json::from_str::<Vec<SavedFurnaceRecord>>(&raw) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "corrupt furnaces.json quarantined"
                );
                let _ = std::fs::rename(&path, world_dir.join("furnaces.json.corrupt"));
                return Self::default();
            }
        };
        let mut furnaces = HashMap::with_capacity(records.len());
        for rec in records {
            furnaces.insert((rec.dimension, rec.x, rec.y, rec.z), rec.furnace);
        }
        Self {
            furnaces,
            dirty: false,
            revision: 0,
        }
    }

    /// Inserts or updates a furnace at `(dim, x, y, z)`.
    pub fn insert_in(&mut self, dim: DimensionId, x: i32, y: i32, z: i32, furnace: FurnaceEntity) {
        self.furnaces.insert((dim, x, y, z), furnace);
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Inserts or updates a furnace at `(x, y, z)` in Overworld.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn insert(&mut self, x: i32, y: i32, z: i32, furnace: FurnaceEntity) {
        self.insert_in(DimensionId::Overworld, x, y, z, furnace);
    }

    /// Removes a furnace in `dim` when broken.
    pub fn remove_in(&mut self, dim: DimensionId, x: i32, y: i32, z: i32) -> Option<FurnaceEntity> {
        let removed = self.furnaces.remove(&(dim, x, y, z));
        if removed.is_some() {
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
        removed
    }

    /// Removes a furnace in Overworld when broken.
    #[allow(dead_code)]
    pub fn remove(&mut self, x: i32, y: i32, z: i32) -> Option<FurnaceEntity> {
        self.remove_in(DimensionId::Overworld, x, y, z)
    }

    /// Returns a reference to a furnace at `(dim, x, y, z)`.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn get_in(&self, dim: DimensionId, x: i32, y: i32, z: i32) -> Option<&FurnaceEntity> {
        self.furnaces.get(&(dim, x, y, z))
    }

    /// Returns a reference to a furnace at `(x, y, z)` in Overworld.
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn get(&self, x: i32, y: i32, z: i32) -> Option<&FurnaceEntity> {
        self.get_in(DimensionId::Overworld, x, y, z)
    }

    /// Returns a mutable reference to a furnace at `(dim, x, y, z)`.
    pub fn get_mut_in(
        &mut self,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<&mut FurnaceEntity> {
        self.furnaces.get_mut(&(dim, x, y, z))
    }

    /// Returns a mutable reference to a furnace at `(x, y, z)` in Overworld.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_mut(&mut self, x: i32, y: i32, z: i32) -> Option<&mut FurnaceEntity> {
        self.get_mut_in(DimensionId::Overworld, x, y, z)
    }

    /// Ensures a furnace exists at `(dim, x, y, z)` and returns a mutable reference.
    pub fn ensure_get_mut_in(
        &mut self,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> &mut FurnaceEntity {
        if let std::collections::hash_map::Entry::Vacant(e) = self.furnaces.entry((dim, x, y, z)) {
            e.insert(FurnaceEntity::default());
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
        self.furnaces
            .get_mut(&(dim, x, y, z))
            .expect("furnace just inserted")
    }

    /// Ensures a furnace exists at `(x, y, z)` in Overworld and returns a mutable reference.
    #[allow(dead_code)]
    pub fn ensure_get_mut(&mut self, x: i32, y: i32, z: i32) -> &mut FurnaceEntity {
        self.ensure_get_mut_in(DimensionId::Overworld, x, y, z)
    }

    /// Marks furnace store dirty and advances its revision counter.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Advances only furnaces inside `area` by one tick using `provider`, pausing all others.
    pub fn tick_in_area_with(
        &mut self,
        area: &SimulationArea,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) {
        self.tick_where_with(|dim, x, z| area.contains_block(dim, x, z), provider);
    }

    /// Advances only furnaces selected by the caller's simulation policy using `provider`.
    pub(super) fn tick_where_with(
        &mut self,
        mut active: impl FnMut(DimensionId, i32, i32) -> bool,
        provider: &dyn rad_core::registry_provider::RegistryProvider,
    ) {
        let mut changed = false;
        for (&(dim, x, _, z), f) in &mut self.furnaces {
            if active(dim, x, z) && f.tick_with(provider) {
                changed = true;
            }
        }
        if changed {
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Snapshots all furnaces for atomic disk persistence if dirty.
    #[allow(clippy::type_complexity)]
    #[must_use]
    pub fn snapshot_if_dirty(
        &self,
    ) -> Option<(u64, Vec<(DimensionId, i32, i32, i32, FurnaceEntity)>)> {
        if !self.dirty {
            return None;
        }
        let items = self
            .furnaces
            .iter()
            .map(|(&pos, f)| (pos.0, pos.1, pos.2, pos.3, f.clone()))
            .collect();
        Some((self.revision, items))
    }

    /// Returns a snapshot of all furnace block entities `(dim, x, y, z, entity)`.
    #[must_use]
    pub fn all_furnaces(&self) -> Vec<(DimensionId, i32, i32, i32, FurnaceEntity)> {
        self.furnaces
            .iter()
            .map(|(&(dim, x, y, z), f)| (dim, x, y, z, f.clone()))
            .collect()
    }

    /// Clears the dirty flag after save succeeds only if no newer mutation occurred.
    pub fn mark_saved(&mut self, expected_rev: u64) {
        if self.revision == expected_rev {
            self.dirty = false;
        }
    }

    /// Atomically writes `world/furnaces.json`, or removes it when empty.
    pub fn write_snapshot_atomic(
        world_dir: &Path,
        snap: &[(DimensionId, i32, i32, i32, FurnaceEntity)],
    ) -> std::io::Result<()> {
        let path = world_dir.join("furnaces.json");
        if snap.is_empty() {
            if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            return Ok(());
        }
        std::fs::create_dir_all(world_dir)?;
        let records: Vec<SavedFurnaceRecord> = snap
            .iter()
            .map(|(dimension, x, y, z, f)| SavedFurnaceRecord {
                dimension: *dimension,
                x: *x,
                y: *y,
                z: *z,
                furnace: f.clone(),
            })
            .collect();
        let tmp = world_dir.join(format!("furnaces.json.{}.tmp", std::process::id()));
        let json = serde_json::to_string_pretty(&records)
            .map_err(|e| std::io::Error::other(format!("furnaces serialize failed: {e}")))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}
