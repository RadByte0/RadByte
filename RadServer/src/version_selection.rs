//! User-selected Minecraft version in `rad.properties` (with legacy `versions/version.toml` fallback).

use std::path::Path;

use serde::Deserialize;

const DEFAULT_VERSION: &str = "26.3";
const MAX_CONFIG_BYTES: u64 = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyVersionSelection {
    version: String,
}

fn validate_version(version: &str, source: &Path) -> Result<String, String> {
    let parts: Vec<_> = version.split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "invalid version '{version}' in {}: use a numeric version like \"26.3\"",
            source.display()
        ));
    }
    Ok(version.to_owned())
}

fn read_checked_file(path: &Path) -> Result<String, String> {
    let symlink_meta = std::fs::symlink_metadata(path)
        .map_err(|e| format!("cannot access {}: {e}", path.display()))?;
    if symlink_meta.file_type().is_symlink() {
        return Err(format!("{} must not be a symlink", path.display()));
    }
    if !symlink_meta.is_file() || symlink_meta.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "{} must be a regular file no larger than {MAX_CONFIG_BYTES} bytes",
            path.display()
        ));
    }
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn parse_rad_properties_version(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=') {
            let key = k.trim();
            if key.eq_ignore_ascii_case("version") || key.eq_ignore_ascii_case("minecraft-version")
            {
                return Some(v.trim().trim_matches('"').trim_matches('\'').to_owned());
            }
        }
    }
    None
}

/// Persists `version=<ver>` in `rad_cfg_path` (`rad.properties`), preserving existing lines.
pub fn write_version(rad_cfg_path: &Path, version: &str) -> Result<(), String> {
    upsert_rad_properties(rad_cfg_path, &[("version", version.to_owned())])
}

/// Persists detailed version metadata (`version`, `version-protocol`, `version-data-version`,
/// `version-source`, `version-tag`) in `rad_cfg_path` (`rad.properties`).
pub fn write_version_details(
    rad_cfg_path: &Path,
    manifest: &rad_core::version_package::VersionManifest,
    versions_dir: &Path,
) -> Result<(), String> {
    let tag_marker = versions_dir
        .join(&manifest.game_version)
        .join(".release_tag");
    let tag = std::fs::read_to_string(tag_marker)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| concat!("v", env!("CARGO_PKG_VERSION")).to_owned());
    upsert_rad_properties(
        rad_cfg_path,
        &[
            ("version", manifest.game_version.clone()),
            ("version-protocol", manifest.protocol_version.to_string()),
            (
                "version-data-version",
                manifest.world_data_version.to_string(),
            ),
            ("version-source", manifest.source.clone()),
            ("version-tag", tag),
        ],
    )
}

fn upsert_rad_properties(rad_cfg_path: &Path, pairs: &[(&str, String)]) -> Result<(), String> {
    let existing = std::fs::read_to_string(rad_cfg_path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut seen = vec![false; pairs.len()];
    for line in existing.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#')
            && let Some((k, _)) = trimmed.split_once('=')
        {
            let key = k.trim();
            if let Some(idx) = pairs.iter().position(|(pk, _)| {
                key.eq_ignore_ascii_case(pk)
                    || (*pk == "version" && key.eq_ignore_ascii_case("minecraft-version"))
            }) {
                lines.push(format!("{}={}", pairs[idx].0, pairs[idx].1));
                seen[idx] = true;
                continue;
            }
        }
        lines.push(line.to_owned());
    }
    for (idx, (k, v)) in pairs.iter().enumerate() {
        if !seen[idx] {
            lines.push(format!("{k}={v}"));
        }
    }
    if let Some(parent) = rad_cfg_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create dir: {e}"))?;
    }
    let mut content = lines.join("\n");
    content.push('\n');
    std::fs::write(rad_cfg_path, content)
        .map_err(|e| format!("cannot write {}: {e}", rad_cfg_path.display()))
}

/// Reads the selected version from `rad.properties` (or legacy `versions/version.toml`),
/// writing `version=26.3` into `rad.properties` for new/existing installations.
pub fn load(rad_cfg_path: &Path, versions_dir: &Path) -> Result<String, String> {
    if rad_cfg_path.exists() {
        let raw = read_checked_file(rad_cfg_path)?;
        if let Some(ver) = parse_rad_properties_version(&raw) {
            return validate_version(&ver, rad_cfg_path);
        }
    }

    let legacy_toml = versions_dir.join("version.toml");
    if legacy_toml.exists() {
        let raw = read_checked_file(&legacy_toml)?;
        let config: LegacyVersionSelection = toml::from_str(raw.trim_start_matches('\u{feff}'))
            .map_err(|e| {
                format!(
                    "invalid {}: {e}; use version = \"26.3\"",
                    legacy_toml.display()
                )
            })?;
        return validate_version(&config.version, &legacy_toml);
    }

    if !versions_dir.is_dir() {
        return Err(format!(
            "cannot access {} (missing versions directory)",
            versions_dir.display()
        ));
    }

    write_version(rad_cfg_path, DEFAULT_VERSION)?;
    Ok(DEFAULT_VERSION.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_default_in_rad_properties_and_rejects_invalid_versions() {
        let dir = std::env::temp_dir().join(format!(
            "rad-version-select-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let versions_dir = dir.join("versions");
        std::fs::create_dir_all(&versions_dir).unwrap();
        let rad_cfg = dir.join("rad.properties");

        assert_eq!(load(&rad_cfg, &versions_dir).unwrap(), "26.3");
        assert!(
            std::fs::read_to_string(&rad_cfg)
                .unwrap()
                .contains("version=26.3")
        );

        std::fs::write(&rad_cfg, "auto-update=true\nversion=26.4\n").unwrap();
        assert_eq!(load(&rad_cfg, &versions_dir).unwrap(), "26.4");

        for raw in [
            "version=../26.3\n",
            "version=26.3/../../etc\n",
            "version=\n",
        ] {
            std::fs::write(&rad_cfg, raw).unwrap();
            assert!(load(&rad_cfg, &versions_dir).is_err(), "accepted {raw}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
