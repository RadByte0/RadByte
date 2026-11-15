//! Persisted 27-slot Chest container block entities for `RadServer`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;

use rad_core::packet::play::{ItemComponentPatch, SimpleSlot};
use serde::{Deserialize, Serialize};

use crate::world::worldgen::DimensionId;

/// Number of slots in a single Chest container.
pub const CHEST_SLOTS: usize = 27;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedSlotEntry {
    count: i32,
    item_id: i32,
    #[serde(default, skip_serializing_if = "ItemComponentPatch::is_empty")]
    components: ItemComponentPatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedContainerRecord {
    #[serde(default)]
    dimension: DimensionId,
    x: i32,
    y: i32,
    z: i32,
    slots: Vec<SavedSlotEntry>,
}

/// Authoritative in-memory store of Chest containers keyed by `(dimension, x, y, z)`.
#[derive(Debug, Default)]
pub struct ContainerStore {
    chests: HashMap<(DimensionId, i32, i32, i32), Vec<SimpleSlot>>,
    chest_components: HashMap<(DimensionId, i32, i32, i32), Vec<ItemComponentPatch>>,
    dirty: bool,
    revision: u64,
    load_error: Option<String>,
}

impl ContainerStore {
    /// Loads `<world_dir>/containers.json` if present.
    pub fn load_from_dir(world_dir: &Path) -> Self {
        match Self::try_load_from_dir(world_dir) {
            Ok(store) => store,
            Err(error) => {
                tracing::error!(world = %world_dir.display(), %error, "cannot load containers; saving is blocked");
                Self {
                    load_error: Some(error.to_string()),
                    ..Self::default()
                }
            }
        }
    }

    /// Loads containers without replacing unreadable or invalid data with defaults.
    pub fn try_load_from_dir(world_dir: &Path) -> std::io::Result<Self> {
        let path = world_dir.join("containers.json");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error),
        };
        const MAX_BYTES: u64 = 32 * 1024 * 1024;
        if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "containers.json must be a regular file no larger than 32 MiB",
            ));
        }
        let mut raw = String::new();
        file.take(MAX_BYTES + 1).read_to_string(&mut raw)?;
        if raw.len() as u64 > MAX_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "containers.json exceeds 32 MiB",
            ));
        }
        let invalid =
            |message: String| std::io::Error::new(std::io::ErrorKind::InvalidData, message);
        let records = serde_json::from_str::<Vec<SavedContainerRecord>>(&raw)
            .map_err(|error| invalid(format!("invalid containers.json: {error}")))?;
        let mut chests = HashMap::with_capacity(records.len());
        let mut chest_components = HashMap::with_capacity(records.len());
        for rec in records {
            if rec.slots.len() > CHEST_SLOTS
                || chests.contains_key(&(rec.dimension, rec.x, rec.y, rec.z))
            {
                return Err(invalid("duplicate container or too many slots".into()));
            }
            let mut slots = vec![SimpleSlot::default(); CHEST_SLOTS];
            let mut comps = vec![ItemComponentPatch::default(); CHEST_SLOTS];
            for (i, entry) in rec.slots.into_iter().enumerate() {
                if !(0..=64).contains(&entry.count)
                    || entry.item_id < 0
                    || (entry.count > 0 && entry.item_id == 0)
                {
                    return Err(invalid("invalid saved container slot".into()));
                }
                if entry.count > 0 && entry.item_id > 0 {
                    slots[i] = SimpleSlot {
                        count: entry.count.min(64),
                        item_id: entry.item_id,
                    };
                    comps[i] = entry.components;
                }
            }
            chests.insert((rec.dimension, rec.x, rec.y, rec.z), slots);
            chest_components.insert((rec.dimension, rec.x, rec.y, rec.z), comps);
        }
        Ok(Self {
            chests,
            chest_components,
            dirty: false,
            revision: 0,
            load_error: None,
        })
    }

    /// Ensures a 27-slot chest entry exists at `(dim, x, y, z)`.
    pub fn ensure_chest_in(&mut self, dim: DimensionId, x: i32, y: i32, z: i32) {
        if let std::collections::hash_map::Entry::Vacant(e) = self.chests.entry((dim, x, y, z)) {
            e.insert(vec![SimpleSlot::default(); CHEST_SLOTS]);
            self.chest_components.insert(
                (dim, x, y, z),
                vec![ItemComponentPatch::default(); CHEST_SLOTS],
            );
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
    }

    /// Ensures a 27-slot chest entry exists at `(x, y, z)` in Overworld.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn ensure_chest(&mut self, x: i32, y: i32, z: i32) {
        self.ensure_chest_in(DimensionId::Overworld, x, y, z);
    }

    /// Removes a chest container in `dim` when its block is broken.
    pub fn remove_chest_in(
        &mut self,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<Vec<SimpleSlot>> {
        let removed = self.chests.remove(&(dim, x, y, z));
        self.chest_components.remove(&(dim, x, y, z));
        if removed.is_some() {
            self.dirty = true;
            self.revision = self.revision.wrapping_add(1);
        }
        removed
    }

    /// Removes a chest container in Overworld when its block is broken.
    #[allow(dead_code)]
    pub fn remove_chest(&mut self, x: i32, y: i32, z: i32) -> Option<Vec<SimpleSlot>> {
        self.remove_chest_in(DimensionId::Overworld, x, y, z)
    }

    /// Returns a mutable reference to the 27 slots of the chest at `(dim, x, y, z)`.
    pub fn get_chest_mut_in(
        &mut self,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<&mut Vec<SimpleSlot>> {
        self.chests.get_mut(&(dim, x, y, z))
    }

    /// Returns mutable references to both slots and component patches of the chest at `(dim, x, y, z)`.
    pub fn get_chest_and_components_mut_in(
        &mut self,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<(&mut Vec<SimpleSlot>, &mut Vec<ItemComponentPatch>)> {
        let key = (dim, x, y, z);
        if !self.chests.contains_key(&key) {
            return None;
        }
        let comps = self
            .chest_components
            .entry(key)
            .or_insert_with(|| vec![ItemComponentPatch::default(); CHEST_SLOTS]);
        let slots = self.chests.get_mut(&key)?;
        Some((slots, comps))
    }

    /// Returns a slice of component patches for the chest at `(dim, x, y, z)`.
    #[must_use]
    pub fn get_chest_components_in(
        &self,
        dim: DimensionId,
        x: i32,
        y: i32,
        z: i32,
    ) -> Option<&[ItemComponentPatch]> {
        self.chest_components
            .get(&(dim, x, y, z))
            .map(Vec::as_slice)
    }

    /// Returns a mutable reference to the 27 slots of the chest at `(x, y, z)` in Overworld.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn get_chest_mut(&mut self, x: i32, y: i32, z: i32) -> Option<&mut Vec<SimpleSlot>> {
        self.get_chest_mut_in(DimensionId::Overworld, x, y, z)
    }

    /// Marks container state dirty after slot mutations.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Returns `(revision, json_bytes)` of all chests for atomic async saving when dirty.
    pub fn snapshot_if_dirty(&self) -> std::io::Result<Option<(u64, Vec<u8>)>> {
        if let Some(error) = &self.load_error {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("container loading failed; refusing save: {error}"),
            ));
        }
        if !self.dirty {
            return Ok(None);
        }
        let mut records: Vec<SavedContainerRecord> = self
            .chests
            .iter()
            .map(|(&(dimension, x, y, z), slots)| {
                let comps = self.chest_components.get(&(dimension, x, y, z));
                SavedContainerRecord {
                    dimension,
                    x,
                    y,
                    z,
                    slots: slots
                        .iter()
                        .enumerate()
                        .map(|(i, s)| SavedSlotEntry {
                            count: s.count,
                            item_id: s.item_id,
                            components: comps.and_then(|c| c.get(i)).cloned().unwrap_or_default(),
                        })
                        .collect(),
                }
            })
            .collect();
        records.sort_by(|a, b| {
            (a.dimension.resource_name(), a.x, a.y, a.z).cmp(&(
                b.dimension.resource_name(),
                b.x,
                b.y,
                b.z,
            ))
        });
        let bytes = serde_json::to_vec_pretty(&records).map_err(std::io::Error::other)?;
        Ok(Some((self.revision, bytes)))
    }

    /// Clears the dirty flag after a confirmed save only if no newer mutation occurred.
    pub fn mark_saved(&mut self, expected_rev: u64) {
        if self.revision == expected_rev {
            self.dirty = false;
        }
    }

    /// Atomically writes serialized container JSON to `<world_dir>/containers.json`.
    pub fn write_snapshot_atomic(world_dir: &Path, json_bytes: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(world_dir)?;
        let path = world_dir.join("containers.json");
        let tmp_path = world_dir.join(format!(
            "containers.json.{}",
            crate::net::unique_tmp_suffix()
        ));
        {
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(json_bytes)?;
            f.sync_all()?;
        }
        std::fs::rename(&tmp_path, &path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_saved_data_is_preserved_and_blocks_future_snapshots() {
        let dir = std::env::temp_dir().join(format!(
            "rad-container-invalid-{}-{}",
            std::process::id(),
            crate::net::unique_tmp_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("containers.json");
        let record = r#"{"x":0,"y":64,"z":0,"slots":[]}"#;
        for contents in [
            "{broken".to_owned(),
            format!("[{record},{record}]"),
            r#"[{"x":0,"y":64,"z":0,"slots":[{"count":100,"item_id":1}]}]"#.to_owned(),
        ] {
            std::fs::write(&path, &contents).unwrap();
            assert!(ContainerStore::try_load_from_dir(&dir).is_err());
            let mut store = ContainerStore::load_from_dir(&dir);
            store.ensure_chest(1, 64, 1);
            assert!(store.snapshot_if_dirty().is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
            assert!(!dir.join("containers.json.corrupt").exists());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn container_store_atomic_roundtrip() {
        let dir = std::env::temp_dir().join("rad_containers_test");
        let _ = std::fs::remove_dir_all(&dir);

        let mut store = ContainerStore::default();
        store.ensure_chest(-3, 64, -5);
        store.get_chest_mut(-3, 64, -5).unwrap()[0] = SimpleSlot {
            count: 12,
            item_id: 25,
        };
        store.mark_dirty();
        let (rev, snap) = store.snapshot_if_dirty().unwrap().unwrap();
        ContainerStore::write_snapshot_atomic(&dir, &snap).unwrap();
        // Simulate a concurrent mutation before mark_saved(rev)
        store.get_chest_mut(-3, 64, -5).unwrap()[1] = SimpleSlot {
            count: 3,
            item_id: 63,
        };
        store.mark_dirty();
        store.mark_saved(rev);
        assert!(
            store.snapshot_if_dirty().unwrap().is_some(),
            "dirty flag must remain true when a newer mutation occurred"
        );
        let (rev2, snap2) = store.snapshot_if_dirty().unwrap().unwrap();
        ContainerStore::write_snapshot_atomic(&dir, &snap2).unwrap();
        store.mark_saved(rev2);
        assert!(store.snapshot_if_dirty().unwrap().is_none());

        let loaded = ContainerStore::load_from_dir(&dir);
        assert_eq!(
            loaded
                .chests
                .get(&(DimensionId::Overworld, -3, 64, -5))
                .unwrap()[0],
            SimpleSlot {
                count: 12,
                item_id: 25,
            }
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
