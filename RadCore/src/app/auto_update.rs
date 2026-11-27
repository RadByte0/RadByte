//! Cross-platform auto-update helpers for RadByte launchers, version modules, and `RadAPI` runtimes.

use std::ffi::OsStr;
use std::path::Path;

/// Parses a tag like `"v0.3.5"` or `"0.3.5"` into `(major, minor, patch)`.
#[must_use]
pub fn parse_semver(tag: &str) -> Option<(u64, u64, u64)> {
    let clean = tag.trim().trim_start_matches(['v', 'V']);
    let core = clean.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    Some((major, minor, patch))
}

/// Returns `true` when `latest_tag` is strictly newer than `current_version`.
#[must_use]
pub fn is_newer_version(latest_tag: &str, current_version: &str) -> bool {
    match (parse_semver(latest_tag), parse_semver(current_version)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

fn is_truthy(val: &str) -> bool {
    matches!(
        val.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn is_falsy(val: &str) -> bool {
    matches!(
        val.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// Returns `true` when auto-updating launcher, versions, and `RadAPI` is enabled.
#[must_use]
pub fn is_auto_update_enabled(cfg_path: &Path) -> bool {
    for arg in std::env::args_os().skip(1) {
        if arg == OsStr::new("--no-auto-update") {
            return false;
        }
        if arg == OsStr::new("--auto-update") {
            return true;
        }
    }
    if std::env::var("RADBYTE_NO_AUTO_UPDATE").is_ok_and(|v| is_truthy(&v)) {
        return false;
    }
    if let Ok(val) = std::env::var("RADBYTE_AUTO_UPDATE") {
        if is_falsy(&val) {
            return false;
        }
        if is_truthy(&val) {
            return true;
        }
    }
    if let Ok(raw) = std::fs::read_to_string(cfg_path) {
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = trimmed.split_once('=')
                && k.trim().eq_ignore_ascii_case("auto-update")
            {
                if is_falsy(v) {
                    return false;
                }
                if is_truthy(v) {
                    return true;
                }
            }
        }
    }
    true
}

/// Persists `auto-update=true|false` in `cfg_path` (`rad.properties`), preserving existing lines.
pub fn set_auto_update_setting(cfg_path: &Path, enabled: bool) -> std::io::Result<()> {
    let value = if enabled { "true" } else { "false" };
    let existing = std::fs::read_to_string(cfg_path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut updated = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#')
            && let Some((k, _)) = trimmed.split_once('=')
            && k.trim().eq_ignore_ascii_case("auto-update")
        {
            lines.push(format!("auto-update={value}"));
            updated = true;
            continue;
        }
        lines.push(line.to_owned());
    }
    if !updated {
        lines.push(format!("auto-update={value}"));
    }
    if let Some(parent) = cfg_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut content = lines.join("\n");
    content.push('\n');
    std::fs::write(cfg_path, content)
}

/// Removes any leftover `.old` executable left behind by a previous Windows self-update.
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        let mut old = exe.as_os_str().to_owned();
        old.push(".old");
        let old_path = std::path::PathBuf::from(old);
        if old_path.is_file() {
            let _ = std::fs::remove_file(old_path);
        }
    }
}

/// Replaces the current executable on disk with `new_bytes`, spawns the updated binary, and exits.
pub fn replace_current_binary_and_restart(new_bytes: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut new_os = exe.as_os_str().to_owned();
    new_os.push(".new");
    let new_path = std::path::PathBuf::from(new_os);

    let mut old_os = exe.as_os_str().to_owned();
    old_os.push(".old");
    let old_path = std::path::PathBuf::from(old_os);

    std::fs::write(&new_path, new_bytes).map_err(|e| format!("Failed to write update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755));
    }

    if old_path.exists() {
        let _ = std::fs::remove_file(&old_path);
    }
    #[cfg(windows)]
    {
        std::fs::rename(&exe, &old_path)
            .map_err(|e| format!("Failed to stage old executable: {e}"))?;
    }
    if let Err(e) = std::fs::rename(&new_path, &exe) {
        #[cfg(windows)]
        let _ = std::fs::rename(&old_path, &exe);
        let _ = std::fs::remove_file(&new_path);
        return Err(format!("Failed to install updated executable: {e}"));
    }

    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let child = std::process::Command::new(&exe)
        .args(args)
        .spawn()
        .map_err(|e| format!("Updated binary saved, but failed to relaunch: {e}"))?;
    drop(child);
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_works() {
        assert!(is_newer_version("v0.3.6", "0.3.5"));
        assert!(!is_newer_version("v0.3.5", "0.3.5"));
        assert!(!is_newer_version("v0.3.4", "0.3.5"));
        assert!(is_newer_version("v1.0.0", "0.9.9"));
    }

    #[test]
    fn auto_update_setting_persists_in_properties() {
        let dir = std::env::temp_dir().join(format!("rad-autoupdate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("rad.properties");
        assert!(is_auto_update_enabled(&cfg));
        set_auto_update_setting(&cfg, false).unwrap();
        assert!(!is_auto_update_enabled(&cfg));
        set_auto_update_setting(&cfg, true).unwrap();
        assert!(is_auto_update_enabled(&cfg));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
