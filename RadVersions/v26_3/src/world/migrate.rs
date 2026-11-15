//! Safe legacy world/playerdata migration to Minecraft 26.3 (`protocol 777`) IDs.
//!
//! Supports `dry_run`, automatic backup creation before writes, semantic diff reporting,
//! and explicit quarantine of ambiguous legacy records (e.g. legacy block state `190`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::registries::ItemIds;

/// Current RadByte world schema version for protocol 777.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Stored schema marker `<world_dir>/schema_version.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldSchemaMarker {
    /// Schema version (`2` = protocol 777 registry-backed IDs).
    pub schema_version: u32,
    /// Minecraft protocol version (`777`).
    pub protocol_version: i32,
}

/// Overall migration outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationStatus {
    /// World already at current schema version.
    Clean,
    /// Dry-run or applied migration completed without ambiguous records.
    Migrated,
    /// Ambiguous records detected and quarantined instead of guessed.
    QuarantinedAmbiguous,
}

/// Detailed report produced by [`migrate_legacy_world`].
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MigrationReport {
    /// Migration status.
    pub status: MigrationStatus,
    /// Backup directory created when `dry_run == false`.
    pub backup_dir: Option<PathBuf>,
    /// Human-readable semantic diff lines.
    pub diff_summary: Vec<String>,
    /// Quarantined ambiguous entries that were NOT auto-rewritten.
    pub quarantined_records: Vec<String>,
}

/// Maps unambiguous pre-777 prototype item IDs using resolved `items`.
#[must_use]
pub const fn map_legacy_item_id_with(old_id: i32, items: &ItemIds) -> Option<i32> {
    match old_id {
        25 => Some(items.oak_planks),
        204 => Some(items.chest),
        205 => Some(items.crafting_table),
        206 => Some(items.furnace),
        700 => Some(items.stick),
        720 => Some(items.wooden_pickaxe),
        793 => Some(items.apple),
        818 => Some(items.bread),
        824 => Some(items.cooked_porkchop),
        _ => None,
    }
}

/// Migrates a legacy `world_dir` using `items` with backup, dry-run, and ambiguity quarantine.
pub fn migrate_legacy_world_with(
    world_dir: &Path,
    dry_run: bool,
    items: &ItemIds,
) -> std::io::Result<MigrationReport> {
    let marker_path = world_dir.join("schema_version.json");
    if let Ok(raw) = std::fs::read_to_string(&marker_path)
        && let Ok(marker) = serde_json::from_str::<WorldSchemaMarker>(&raw)
        && marker.schema_version >= CURRENT_SCHEMA_VERSION
        && marker.protocol_version == rad_core::PROTOCOL_VERSION
    {
        return Ok(MigrationReport {
            status: MigrationStatus::Clean,
            backup_dir: None,
            diff_summary: Vec::new(),
            quarantined_records: Vec::new(),
        });
    }

    let mut backup_dir = None;
    if !dry_run && world_dir.exists() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let bdir = world_dir.join("backups").join(format!("pre-777-{ts}"));
        std::fs::create_dir_all(&bdir)?;
        for sub in ["containers.json", "furnaces.json", "level.dat"] {
            let src = world_dir.join(sub);
            if src.exists() {
                std::fs::copy(&src, bdir.join(sub))?;
            }
        }
        for dir_rel in ["playerdata", "region", "DIM-1/region", "DIM1/region"] {
            let src_dir = world_dir.join(dir_rel);
            if src_dir.exists() {
                let dst_dir = bdir.join(dir_rel);
                std::fs::create_dir_all(&dst_dir)?;
                for entry in std::fs::read_dir(&src_dir)?.flatten() {
                    let p = entry.path();
                    if p.is_file()
                        && let Some(name) = p.file_name()
                    {
                        std::fs::copy(&p, dst_dir.join(name))?;
                    }
                }
            }
        }
        backup_dir = Some(bdir);
    }

    let mut diff_summary = Vec::new();
    let mut quarantined_records = Vec::new();

    // Check playerdata/*.json
    let pd_dir = world_dir.join("playerdata");
    if pd_dir.exists() {
        for entry in std::fs::read_dir(&pd_dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path)?;
            let mut val: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
            let mut file_changed = false;
            if let Some(slots) = val.get_mut("inventory").and_then(|v| v.as_array_mut()) {
                for (idx, slot) in slots.iter_mut().enumerate() {
                    let count = slot.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
                    let iid = slot.get("item_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                    if count > 0
                        && iid > 0
                        && let Some(new_id) = map_legacy_item_id_with(iid, items)
                    {
                        diff_summary.push(format!(
                            "{}: slot[{idx}] item_id {iid} -> {new_id}",
                            path.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        slot["item_id"] = serde_json::json!(new_id);
                        file_changed = true;
                    }
                }
            }
            if file_changed && !dry_run {
                std::fs::write(&path, serde_json::to_string_pretty(&val)?)?;
            }
        }
    }

    // Check containers.json
    let containers_path = world_dir.join("containers.json");
    if containers_path.exists() {
        let raw = std::fs::read_to_string(&containers_path)?;
        if let Ok(mut records) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
            let mut changed = false;
            for (r_idx, rec) in records.iter_mut().enumerate() {
                if let Some(slots) = rec.get_mut("slots").and_then(|v| v.as_array_mut()) {
                    for (s_idx, slot) in slots.iter_mut().enumerate() {
                        let count = slot.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
                        let iid = slot.get("item_id").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        if count > 0
                            && iid > 0
                            && let Some(new_id) = map_legacy_item_id_with(iid, items)
                        {
                            diff_summary.push(format!(
                                "containers.json: chest[{r_idx}] slot[{s_idx}] item_id {iid} -> {new_id}"
                            ));
                            slot["item_id"] = serde_json::json!(new_id);
                            changed = true;
                        }
                    }
                }
            }
            if changed && !dry_run {
                std::fs::write(&containers_path, serde_json::to_string_pretty(&records)?)?;
            }
        }
    }

    // Check furnaces.json
    let furnaces_path = world_dir.join("furnaces.json");
    if furnaces_path.exists() {
        let raw = std::fs::read_to_string(&furnaces_path)?;
        if let Ok(mut records) = serde_json::from_str::<Vec<serde_json::Value>>(&raw) {
            let mut changed = false;
            for (r_idx, rec) in records.iter_mut().enumerate() {
                if let Some(f) = rec.get_mut("furnace").and_then(|v| v.as_object_mut()) {
                    for (count_key, item_key) in [
                        ("input_count", "input_item"),
                        ("fuel_count", "fuel_item"),
                        ("output_count", "output_item"),
                    ] {
                        let count = f.get(count_key).and_then(|v| v.as_i64()).unwrap_or(0);
                        let iid = f.get(item_key).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        if count > 0
                            && iid > 0
                            && let Some(new_id) = map_legacy_item_id_with(iid, items)
                        {
                            diff_summary.push(format!(
                                "furnaces.json: furnace[{r_idx}] {item_key} {iid} -> {new_id}"
                            ));
                            f.insert(item_key.to_owned(), serde_json::json!(new_id));
                            changed = true;
                        }
                    }
                }
            }
            if changed && !dry_run {
                std::fs::write(&furnaces_path, serde_json::to_string_pretty(&records)?)?;
            }
        }
    }

    // Scan Anvil region/*.mca across Overworld, DIM-1, and DIM1
    for (dim_label, rel) in [
        ("overworld", "region"),
        ("nether", "DIM-1/region"),
        ("end", "DIM1/region"),
    ] {
        let reg_dir = world_dir.join(rel);
        if !reg_dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(&reg_dir)?.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("mca") {
                continue;
            }
            let meta = std::fs::metadata(&p)?;
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if meta.len() < 8192 {
                quarantined_records.push(format!(
                    "{dim_label}/{name}: truncated Anvil header ({} bytes); quarantined",
                    meta.len()
                ));
            } else {
                diff_summary.push(format!("scanned {dim_label}/{name} ({} bytes)", meta.len()));
            }
        }
    }

    // Check legacy block state marker file if present (e.g., ambiguous state 190 chest/tnt)
    let legacy_blocks = world_dir.join("legacy_block_states.json");
    if legacy_blocks.exists() {
        let raw = std::fs::read_to_string(&legacy_blocks)?;
        if let Ok(states) = serde_json::from_str::<Vec<u16>>(&raw) {
            for st in states {
                if st == 190 {
                    quarantined_records.push(
                        "legacy_block_states.json: ambiguous state 190 (collided chest/TNT); quarantined without guessing".into(),
                    );
                }
            }
        }
    }

    let status = if quarantined_records.is_empty() {
        if !dry_run {
            let marker = WorldSchemaMarker {
                schema_version: CURRENT_SCHEMA_VERSION,
                protocol_version: rad_core::PROTOCOL_VERSION,
            };
            std::fs::write(&marker_path, serde_json::to_string_pretty(&marker)?)?;
        }
        MigrationStatus::Migrated
    } else {
        MigrationStatus::QuarantinedAmbiguous
    };

    Ok(MigrationReport {
        status,
        backup_dir,
        diff_summary,
        quarantined_records,
    })
}

/// Restores backed-up files/regions from `backup_dir` back into `world_dir` and removes `schema_version.json`.
#[allow(dead_code)]
pub fn rollback_migration(world_dir: &Path, backup_dir: &Path) -> std::io::Result<()> {
    if !backup_dir.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("backup directory missing: {}", backup_dir.display()),
        ));
    }
    for sub in ["containers.json", "furnaces.json", "level.dat"] {
        let src = backup_dir.join(sub);
        if src.exists() {
            std::fs::copy(&src, world_dir.join(sub))?;
        }
    }
    for dir_rel in ["playerdata", "region", "DIM-1/region", "DIM1/region"] {
        let src_dir = backup_dir.join(dir_rel);
        if src_dir.exists() {
            let dst_dir = world_dir.join(dir_rel);
            std::fs::create_dir_all(&dst_dir)?;
            for entry in std::fs::read_dir(&src_dir)?.flatten() {
                let p = entry.path();
                if p.is_file()
                    && let Some(name) = p.file_name()
                {
                    std::fs::copy(&p, dst_dir.join(name))?;
                }
            }
        }
    }
    let marker_path = world_dir.join("schema_version.json");
    if marker_path.exists() {
        std::fs::remove_file(&marker_path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_migration_dry_run_backup_and_quarantines_ambiguous_id() {
        let dir = std::env::temp_dir().join(format!(
            "rad_migrate_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let pd = dir.join("playerdata");
        std::fs::create_dir_all(&pd).unwrap();
        let player_file = pd.join("test.json");
        std::fs::write(
            &player_file,
            r#"{"inventory":[{"count":16,"item_id":25},{"count":1,"item_id":204}]}"#,
        )
        .unwrap();

        let items = crate::registries::Version26_3Registries::from_provider(
            std::sync::Arc::new(crate::registries::synthetic_26_3_test_provider()),
            true,
        )
        .unwrap()
        .items;

        // Dry-run reports semantic diff without modifying disk
        let dry = migrate_legacy_world_with(&dir, true, &items).unwrap();
        assert_eq!(dry.status, MigrationStatus::Migrated);
        assert!(dry.backup_dir.is_none());
        assert_eq!(dry.diff_summary.len(), 2);
        assert!(
            std::fs::read_to_string(&player_file)
                .unwrap()
                .contains("\"item_id\":25")
        );

        // Apply migration creates backup and rewrites to resolved 777 IDs
        let applied = migrate_legacy_world_with(&dir, false, &items).unwrap();
        assert_eq!(applied.status, MigrationStatus::Migrated);
        assert!(applied.backup_dir.unwrap().exists());
        let updated = std::fs::read_to_string(&player_file).unwrap();
        assert!(updated.contains(&items.oak_planks.to_string()));
        assert!(updated.contains(&items.chest.to_string()));

        // Second run is Clean
        let second = migrate_legacy_world_with(&dir, false, &items).unwrap();
        assert_eq!(second.status, MigrationStatus::Clean);

        // Ambiguous legacy state 190 is quarantined when schema marker absent
        std::fs::remove_file(dir.join("schema_version.json")).unwrap();
        std::fs::write(dir.join("legacy_block_states.json"), "[190]").unwrap();
        let quar = migrate_legacy_world_with(&dir, false, &items).unwrap();
        assert_eq!(quar.status, MigrationStatus::QuarantinedAmbiguous);
        assert_eq!(quar.quarantined_records.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn migration_scans_containers_furnaces_regions_and_supports_rollback() {
        let dir = std::env::temp_dir().join(format!(
            "rad_migrate_full_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("DIM-1/region")).unwrap();
        std::fs::write(
            dir.join("containers.json"),
            r#"[{"x":1,"y":64,"z":1,"slots":[{"count":4,"item_id":25}]}]"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("furnaces.json"),
            r#"[{"x":2,"y":64,"z":2,"furnace":{"input_item":25,"input_count":2,"fuel_item":700,"fuel_count":1,"output_item":0,"output_count":0,"burn_time":0,"cook_progress":0}}]"#,
        )
        .unwrap();
        std::fs::write(dir.join("DIM-1/region/r.0.0.mca"), vec![0u8; 8192]).unwrap();

        let items = crate::registries::Version26_3Registries::from_provider(
            std::sync::Arc::new(crate::registries::synthetic_26_3_test_provider()),
            true,
        )
        .unwrap()
        .items;
        let applied = migrate_legacy_world_with(&dir, false, &items).unwrap();
        assert_eq!(applied.status, MigrationStatus::Migrated);
        let backup = applied.backup_dir.unwrap();
        assert!(backup.join("DIM-1/region/r.0.0.mca").exists());
        assert!(
            std::fs::read_to_string(dir.join("containers.json"))
                .unwrap()
                .contains(&items.oak_planks.to_string())
        );
        assert!(
            std::fs::read_to_string(dir.join("furnaces.json"))
                .unwrap()
                .contains(&items.stick.to_string())
        );

        // Rollback restores original legacy JSONs and removes schema marker
        rollback_migration(&dir, &backup).unwrap();
        assert!(!dir.join("schema_version.json").exists());
        assert!(
            std::fs::read_to_string(dir.join("containers.json"))
                .unwrap()
                .contains("\"item_id\": 25")
                || std::fs::read_to_string(dir.join("containers.json"))
                    .unwrap()
                    .contains("\"item_id\":25")
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
