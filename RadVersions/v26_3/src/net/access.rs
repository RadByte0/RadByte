//! Operator, whitelist and ban persistence.

use super::*;

fn default_op_entry_level() -> u8 {
    4
}

/// Whitelist entry keyed primarily by UUID (when known) with display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhitelistEntry {
    /// Player UUID if verified/known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<Uuid>,
    /// Display username.
    pub name: String,
}

/// Operator entry keyed primarily by UUID (when known) with display name and permission level (`1..=4`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpEntry {
    /// Player UUID if verified/known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<Uuid>,
    /// Display username.
    pub name: String,
    /// Operator permission level (`1..=4`).
    #[serde(default = "default_op_entry_level")]
    pub level: u8,
    /// Paper's per-operator player-limit bypass flag.
    #[serde(default, rename = "bypassesPlayerLimit")]
    pub bypasses_player_limit: bool,
}

/// Persisted whitelist, operator, and ban state (`<level_name>/access_control.json`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccessControl {
    /// Whether whitelist enforcement is active.
    #[serde(default)]
    pub whitelist_enabled: bool,
    /// Legacy lowercase whitelisted usernames (kept in sync for backward compatibility).
    #[serde(default)]
    pub whitelist: std::collections::HashSet<String>,
    /// Lowercase banned usernames.
    #[serde(default)]
    pub banned: std::collections::HashSet<String>,
    /// Legacy lowercase operator usernames (kept in sync for backward compatibility).
    #[serde(default)]
    pub ops: std::collections::HashSet<String>,
    /// UUID-aware whitelist entries.
    #[serde(default)]
    pub whitelist_entries: Vec<WhitelistEntry>,
    /// UUID-aware operator entries with permission levels (`1..=4`).
    #[serde(default)]
    pub op_entries: Vec<OpEntry>,
    #[serde(skip)]
    pub(super) dirty: bool,
    #[serde(skip)]
    pub(super) revision: u64,
    /// Refuse to overwrite operator-managed files that failed to parse.
    #[serde(skip)]
    pub(super) lists_invalid: bool,
}

impl AccessControl {
    fn invalid_lists() -> Self {
        Self {
            whitelist_enabled: true,
            lists_invalid: true,
            ..Self::default()
        }
    }
    // Production uses a relative level-name, so Paper-compatible lists live in
    // the server working directory. Absolute paths keep isolated tests and
    // embedders scoped to the supplied directory.
    fn lists_dir(world_dir: &std::path::Path) -> &std::path::Path {
        if world_dir.is_absolute() {
            world_dir
        } else {
            std::path::Path::new(".")
        }
    }
    /// Loads access control lists from `<world_dir>/access_control.json`, returning defaults if missing.
    #[must_use]
    pub fn load_from_dir(world_dir: &std::path::Path) -> Self {
        let path = world_dir.join("access_control.json");
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}".to_owned(),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot read access control; refusing logins and saves");
                return Self::invalid_lists();
            }
        };
        let mut loaded: Self = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to parse access_control.json; refusing logins and saves"
                );
                return Self::invalid_lists();
            }
        };
        for (filename, is_ops) in [("ops.json", true), ("whitelist.json", false)] {
            let primary = Self::lists_dir(world_dir).join(filename);
            let legacy = world_dir.join(filename);
            let file = if primary.exists() { primary } else { legacy };
            let raw = match std::fs::read_to_string(&file) {
                Ok(raw) => raw,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => {
                    loaded.lists_invalid = true;
                    tracing::warn!(path = %file.display(), %error, "cannot read access list; refusing logins and saves");
                    continue;
                }
            };
            {
                if is_ops {
                    match serde_json::from_str::<Vec<OpEntry>>(&raw) {
                        Ok(entries) => {
                            loaded.op_entries = entries;
                            loaded.ops.clear();
                        }
                        Err(e) => {
                            loaded.lists_invalid = true;
                            tracing::warn!(path = %file.display(), error = %e, "invalid operator list; refusing to overwrite it")
                        }
                    }
                } else {
                    match serde_json::from_str::<Vec<WhitelistEntry>>(&raw) {
                        Ok(entries) => {
                            loaded.whitelist_entries = entries;
                            loaded.whitelist.clear();
                        }
                        Err(e) => {
                            loaded.lists_invalid = true;
                            tracing::warn!(path = %file.display(), error = %e, "invalid whitelist; refusing to overwrite it")
                        }
                    }
                }
            }
        }
        loaded.sync_legacy_entries();
        // Materialize missing Paper-style files on the next save, including
        // when all entries came from a legacy access_control.json.
        if ["ops.json", "whitelist.json"]
            .iter()
            .any(|name| !Self::lists_dir(world_dir).join(name).exists())
        {
            loaded.mark_dirty();
        }
        loaded
    }

    fn sync_legacy_entries(&mut self) {
        for name in &self.whitelist {
            if !self
                .whitelist_entries
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case(name))
            {
                self.whitelist_entries.push(WhitelistEntry {
                    uuid: None,
                    name: name.clone(),
                });
            }
        }
        for entry in &self.whitelist_entries {
            self.whitelist.insert(entry.name.to_ascii_lowercase());
        }
        for name in &self.ops {
            if !self
                .op_entries
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case(name))
            {
                self.op_entries.push(OpEntry {
                    uuid: None,
                    name: name.clone(),
                    level: 4,
                    bypasses_player_limit: false,
                });
            }
        }
        for entry in &mut self.op_entries {
            entry.level = entry.level.clamp(1, 4);
            self.ops.insert(entry.name.to_ascii_lowercase());
        }
    }

    /// Marks access control dirty and advances its revision counter.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.revision = self.revision.wrapping_add(1);
    }

    /// Returns `(revision, cloned snapshot)` when dirty.
    #[must_use]
    pub fn snapshot_if_dirty(&self) -> Option<(u64, Self)> {
        if self.dirty {
            Some((self.revision, self.clone()))
        } else {
            None
        }
    }

    /// Clears the dirty flag after persistence succeeds only if no newer mutation occurred.
    pub fn mark_saved(&mut self, expected_rev: u64) {
        if self.revision == expected_rev {
            self.dirty = false;
        }
    }

    /// Atomically writes an [`AccessControl`] snapshot to `<world_dir>/access_control.json`.
    pub fn write_snapshot_atomic(world_dir: &std::path::Path, snap: &Self) -> std::io::Result<()> {
        if snap.lists_invalid {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid ops.json or whitelist.json; fix it before saving",
            ));
        }
        std::fs::create_dir_all(world_dir)?;
        let path = world_dir.join("access_control.json");
        let tmp = world_dir.join(format!("access_control.json.{}", unique_tmp_suffix()));
        let json = serde_json::to_string_pretty(snap)
            .map_err(|e| std::io::Error::other(format!("access serialize failed: {e}")))?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)?;
        for (name, json) in [
            ("ops.json", serde_json::to_string_pretty(&snap.op_entries)),
            (
                "whitelist.json",
                serde_json::to_string_pretty(&snap.whitelist_entries),
            ),
        ] {
            let lists_dir = Self::lists_dir(world_dir);
            let path = lists_dir.join(name);
            let tmp = lists_dir.join(format!("{name}.{}", unique_tmp_suffix()));
            let json = json.map_err(|e| std::io::Error::other(e.to_string()))?;
            std::fs::write(&tmp, json)?;
            std::fs::rename(&tmp, &path)?;
        }
        Ok(())
    }

    /// Checks whether `username` is banned (case-insensitive).
    #[must_use]
    pub fn is_banned(&self, username: &str) -> bool {
        self.banned.contains(&username.to_ascii_lowercase())
    }

    /// Checks whether `username` is allowed under the current whitelist policy (legacy name-only helper).
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn is_allowed_by_whitelist(&self, username: &str, is_admin: bool) -> bool {
        self.is_allowed_by_whitelist_identity(username, None, is_admin, false)
    }

    /// Checks whitelist allowance using UUID as primary identity when known.
    #[must_use]
    pub fn is_allowed_by_whitelist_identity(
        &self,
        username: &str,
        uuid: Option<Uuid>,
        is_op: bool,
        trusted_identity: bool,
    ) -> bool {
        if self.lists_invalid {
            return false;
        }
        if !self.whitelist_enabled || is_op {
            return true;
        }
        if let Some(u) = uuid.filter(|_| trusted_identity)
            && self.whitelist_entries.iter().any(|e| e.uuid == Some(u))
        {
            return true;
        }
        if self
            .whitelist_entries
            .iter()
            .any(|e| e.name.eq_ignore_ascii_case(username) && e.uuid.is_none())
        {
            return true;
        }
        // Fallback for tests mutating `access.whitelist` directly without `whitelist_entries`.
        if self.whitelist.contains(&username.to_ascii_lowercase()) {
            let bound_other_uuid = self
                .whitelist_entries
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case(username) && e.uuid.is_some());
            return !bound_other_uuid;
        }
        false
    }

    /// Returns operator permission level (`0` if not an op, `1..=4` otherwise).
    #[must_use]
    pub fn op_level_for_identity(
        &self,
        username: &str,
        uuid: Option<Uuid>,
        trusted_identity: bool,
    ) -> u8 {
        if self.lists_invalid {
            return 0;
        }
        if let Some(u) = uuid.filter(|_| trusted_identity)
            && let Some(entry) = self.op_entries.iter().find(|e| e.uuid == Some(u))
        {
            return entry.level.clamp(1, 4);
        }
        if let Some(entry) = self
            .op_entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(username) && e.uuid.is_none())
        {
            return entry.level.clamp(1, 4);
        }
        if self.ops.contains(&username.to_ascii_lowercase()) {
            let bound_other_uuid = self
                .op_entries
                .iter()
                .any(|e| e.name.eq_ignore_ascii_case(username) && e.uuid.is_some());
            if !bound_other_uuid {
                return 4;
            }
        }
        0
    }

    /// Adds or updates a whitelist entry; returns `true` if newly added.
    pub fn add_whitelist(&mut self, uuid: Option<Uuid>, name: String) -> bool {
        if let Some(u) = uuid
            && let Some(existing) = self
                .whitelist_entries
                .iter_mut()
                .find(|e| e.uuid == Some(u))
        {
            let old_key = existing.name.to_ascii_lowercase();
            if existing.name != name {
                self.whitelist.remove(&old_key);
                existing.name = name.clone();
                self.whitelist.insert(name.to_ascii_lowercase());
                self.mark_dirty();
            }
            return false;
        }
        if let Some(existing) = self
            .whitelist_entries
            .iter_mut()
            .find(|e| e.name.eq_ignore_ascii_case(&name))
        {
            if existing.uuid.is_none() && uuid.is_some() {
                existing.uuid = uuid;
                self.mark_dirty();
            }
            return false;
        }
        self.whitelist.insert(name.to_ascii_lowercase());
        self.whitelist_entries.push(WhitelistEntry { uuid, name });
        self.mark_dirty();
        true
    }

    /// Removes a whitelist entry by UUID or username; returns `true` if removed.
    pub fn remove_whitelist(&mut self, target: &str) -> bool {
        let parsed_uuid = Uuid::parse_str(target).ok();
        let before = self.whitelist_entries.len();
        let mut removed_names = Vec::new();
        self.whitelist_entries.retain(|e| {
            let matches = parsed_uuid.is_some_and(|u| e.uuid == Some(u))
                || e.name.eq_ignore_ascii_case(target);
            if matches {
                removed_names.push(e.name.to_ascii_lowercase());
            }
            !matches
        });
        for n in removed_names {
            self.whitelist.remove(&n);
        }
        let legacy_removed = self.whitelist.remove(&target.to_ascii_lowercase());
        let changed = self.whitelist_entries.len() < before || legacy_removed;
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// Adds or updates an operator entry; returns `true` if newly added or level changed.
    pub fn add_op(&mut self, uuid: Option<Uuid>, name: String, level: u8) -> bool {
        let lvl = level.clamp(1, 4);
        if let Some(u) = uuid
            && let Some(existing) = self.op_entries.iter_mut().find(|e| e.uuid == Some(u))
        {
            let mut changed = false;
            if existing.level != lvl {
                existing.level = lvl;
                changed = true;
            }
            if existing.name != name {
                self.ops.remove(&existing.name.to_ascii_lowercase());
                existing.name = name.clone();
                self.ops.insert(name.to_ascii_lowercase());
                changed = true;
            }
            if changed {
                self.mark_dirty();
            }
            return changed;
        }
        if let Some(existing) = self
            .op_entries
            .iter_mut()
            .find(|e| e.name.eq_ignore_ascii_case(&name))
        {
            let mut changed = false;
            if existing.uuid.is_none() && uuid.is_some() {
                existing.uuid = uuid;
                changed = true;
            }
            if existing.level != lvl {
                existing.level = lvl;
                changed = true;
            }
            if changed {
                self.mark_dirty();
            }
            return changed;
        }
        self.ops.insert(name.to_ascii_lowercase());
        self.op_entries.push(OpEntry {
            uuid,
            name,
            level: lvl,
            bypasses_player_limit: false,
        });
        self.mark_dirty();
        true
    }

    /// Removes an operator entry by UUID or username; returns `true` if removed.
    pub fn remove_op(&mut self, target: &str) -> bool {
        let parsed_uuid = Uuid::parse_str(target).ok();
        let before = self.op_entries.len();
        let mut removed_names = Vec::new();
        self.op_entries.retain(|e| {
            let matches = parsed_uuid.is_some_and(|u| e.uuid == Some(u))
                || e.name.eq_ignore_ascii_case(target);
            if matches {
                removed_names.push(e.name.to_ascii_lowercase());
            }
            !matches
        });
        for n in removed_names {
            self.ops.remove(&n);
        }
        let legacy_removed = self.ops.remove(&target.to_ascii_lowercase());
        let changed = self.op_entries.len() < before || legacy_removed;
        if changed {
            self.mark_dirty();
        }
        changed
    }

    /// Records a verified (forwarded) UUID ↔ username pair, updating display names on rename.
    pub fn record_verified_identity(&mut self, uuid: Uuid, username: &str) {
        let mut changed = false;
        let mut old_wl_names = Vec::new();
        for entry in &mut self.whitelist_entries {
            if entry.uuid == Some(uuid) {
                if entry.name != username {
                    old_wl_names.push(entry.name.to_ascii_lowercase());
                    entry.name = username.to_owned();
                    changed = true;
                }
            } else if entry.uuid.is_none() && entry.name.eq_ignore_ascii_case(username) {
                entry.uuid = Some(uuid);
                entry.name = username.to_owned();
                changed = true;
            }
        }
        for old in old_wl_names {
            self.whitelist.remove(&old);
            self.whitelist.insert(username.to_ascii_lowercase());
        }

        let mut old_op_names = Vec::new();
        for entry in &mut self.op_entries {
            if entry.uuid == Some(uuid) {
                if entry.name != username {
                    old_op_names.push(entry.name.to_ascii_lowercase());
                    entry.name = username.to_owned();
                    changed = true;
                }
            } else if entry.uuid.is_none() && entry.name.eq_ignore_ascii_case(username) {
                entry.uuid = Some(uuid);
                entry.name = username.to_owned();
                changed = true;
            }
        }
        for old in old_op_names {
            self.ops.remove(&old);
            self.ops.insert(username.to_ascii_lowercase());
        }

        if changed {
            self.mark_dirty();
        }
    }
}

#[cfg(test)]
mod identity_review_tests {
    use super::*;

    #[test]
    fn bound_uuid_requires_trusted_identity_and_cannot_fall_back_to_name() {
        let id = Uuid::from_u128(42);
        let mut access = AccessControl {
            whitelist_enabled: true,
            ..Default::default()
        };
        access.add_op(Some(id), "Owner".into(), 4);
        access.add_whitelist(Some(id), "Owner".into());
        assert_eq!(access.op_level_for_identity("Impostor", Some(id), false), 0);
        assert!(!access.is_allowed_by_whitelist_identity("Impostor", Some(id), false, false));
        assert_eq!(access.op_level_for_identity("Owner", None, false), 0);
        assert!(!access.is_allowed_by_whitelist_identity("Owner", None, false, false));
        assert_eq!(access.op_level_for_identity("Renamed", Some(id), true), 4);
        assert!(access.is_allowed_by_whitelist_identity("Renamed", Some(id), false, true));
        access.add_op(None, "LocalAdmin".into(), 2);
        assert_eq!(access.op_level_for_identity("LocalAdmin", None, false), 2);
    }

    #[test]
    fn corrupt_access_control_denies_login_and_preserves_file_on_save() {
        let dir = std::env::temp_dir().join(format!(
            "radbyte-access-review-{}-{}",
            std::process::id(),
            unique_tmp_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("access_control.json");
        std::fs::write(&path, b"{broken").unwrap();
        let access = AccessControl::load_from_dir(&dir);
        assert!(!access.is_allowed_by_whitelist_identity("Admin", None, true, true));
        assert_eq!(access.op_level_for_identity("Admin", None, true), 0);
        assert!(AccessControl::write_snapshot_atomic(&dir, &access).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_operator_list_denies_login_and_save() {
        let dir = std::env::temp_dir().join(format!(
            "radbyte-access-review-{}-{}",
            std::process::id(),
            unique_tmp_suffix()
        ));
        std::fs::create_dir_all(dir.join("ops.json")).unwrap();
        let access = AccessControl::load_from_dir(&dir);
        assert!(!access.is_allowed_by_whitelist_identity("Player", None, false, false));
        assert!(AccessControl::write_snapshot_atomic(&dir, &access).is_err());
        assert!(dir.join("ops.json").is_dir());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
