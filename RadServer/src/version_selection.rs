//! User-selected Minecraft version in `versions/version.toml`.

use std::io::{ErrorKind, Write};
use std::path::Path;

use serde::Deserialize;

const DEFAULT_CONFIG: &str = "# Minecraft version supported by this RadServer build.\n# Use a quoted version number; only installed and supported versions can run.\nversion = \"26.3\"\n";
const MAX_CONFIG_BYTES: u64 = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionSelection {
    version: String,
}

/// Reads the selected version, creating a default config for existing installations.
pub fn load(versions_dir: &Path) -> Result<String, String> {
    let path = versions_dir.join("version.toml");
    // Do not manufacture an empty versions directory when the package is missing.
    if versions_dir.is_dir() {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => file
                .write_all(DEFAULT_CONFIG.as_bytes())
                .map_err(|e| format!("cannot write {}: {e}", path.display()))?,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        }
    }
    let symlink_meta = std::fs::symlink_metadata(&path)
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
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let config: VersionSelection = toml::from_str(raw.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("invalid {}: {e}; use version = \"26.3\"", path.display()))?;
    let parts: Vec<_> = config.version.split('.').collect();
    if !(2..=3).contains(&parts.len())
        || parts
            .iter()
            .any(|p| p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(format!(
            "invalid version '{}' in {}: use a numeric version like \"26.3\"",
            config.version,
            path.display()
        ));
    }
    Ok(config.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_default_and_rejects_invalid_version_configs() {
        let dir = std::env::temp_dir().join(format!(
            "rad-version-select-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(load(&dir).unwrap(), "26.3");
        let path = dir.join("version.toml");
        std::fs::write(&path, "version = \"26.4\"\n").unwrap();
        assert_eq!(load(&dir).unwrap(), "26.4");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "version = \"26.4\"\n"
        );
        for raw in [
            "version = \"../26.3\"",
            "version = \"26.3/../../etc\"",
            "version = 26.3",
            "version = \"26.3\"\nextra = true",
            "version = \"\"",
        ] {
            std::fs::write(&path, raw).unwrap();
            assert!(load(&dir).is_err(), "accepted {raw}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
