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

/// Default background update check interval in minutes.
pub const DEFAULT_AUTO_UPDATE_INTERVAL_MINUTES: u64 = 30;
/// Minimum allowed background update check interval in minutes.
pub const MIN_AUTO_UPDATE_INTERVAL_MINUTES: u64 = 1;
/// Maximum allowed background update check interval in minutes (7 days).
pub const MAX_AUTO_UPDATE_INTERVAL_MINUTES: u64 = 10_080;

/// Granular auto-update configuration persisted in `rad.properties`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoUpdateSettings {
    /// Master toggle (`auto-update`).
    pub enabled: bool,
    /// Background check interval in minutes (`auto-update-interval-minutes`).
    pub interval_minutes: u64,
    /// Self-update launcher executable (`auto-update-launcher`).
    pub launcher: bool,
    /// Update `RadAPI` runtime (`auto-update-radapi`).
    pub radapi: bool,
    /// Update installed Minecraft version module (`auto-update-versions`).
    pub versions: bool,
    /// Check Radons in `radons/` that declare a GitHub repo (`auto-update-radons`).
    pub radons: bool,
}

impl Default for AutoUpdateSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: DEFAULT_AUTO_UPDATE_INTERVAL_MINUTES,
            launcher: true,
            radapi: true,
            versions: true,
            radons: true,
        }
    }
}

/// Radon manifest update target discovered in `radons/*.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RadonUpdateTarget {
    /// Radon identifier (`id`).
    pub id: String,
    /// Currently installed Radon version (`version`).
    pub version: String,
    /// Normalized GitHub `"owner/repo"`.
    pub repo: String,
}

/// Normalizes `"owner/repo"` or `"https://github.com/owner/repo(.git)"` into `"owner/repo"`.
#[must_use]
pub fn parse_github_repo(raw: &str) -> Option<String> {
    let mut s = raw.trim().trim_end_matches('/');
    if let Some(rest) = s.strip_prefix("https://github.com/") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("http://github.com/") {
        s = rest;
    } else if let Some(rest) = s.strip_prefix("github.com/") {
        s = rest;
    }
    s = s.strip_suffix(".git").unwrap_or(s).trim_matches('/');
    let mut parts = s.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty()
        || repo.is_empty()
        || parts.next().is_some()
        || !owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        || !repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Scans `radons_dir/*.toml` for Radons that declare `github` or `repository`.
#[must_use]
pub fn scan_radon_github_targets(radons_dir: &Path) -> Vec<RadonUpdateTarget> {
    let Ok(entries) = std::fs::read_dir(radons_dir) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let mut id = None;
        let mut version = None;
        let mut github = None;
        let mut repository = None;
        for line in raw.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
                continue;
            }
            let Some((k, v)) = trimmed.split_once('=') else {
                continue;
            };
            let val = v
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches('"')
                .trim_matches('\'')
                .trim()
                .to_owned();
            if val.is_empty() {
                continue;
            }
            match k.trim() {
                "id" => id = Some(val),
                "version" => version = Some(val),
                "github" => github = Some(val),
                "repository" => repository = Some(val),
                _ => {}
            }
        }
        if let (Some(id), Some(version), Some(repo)) = (
            id,
            version,
            github
                .as_deref()
                .or(repository.as_deref())
                .and_then(parse_github_repo),
        ) {
            targets.push(RadonUpdateTarget { id, version, repo });
        }
    }
    targets
}

fn read_property(cfg_path: &Path, key: &str) -> Option<String> {
    let raw = std::fs::read_to_string(cfg_path).ok()?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = trimmed.split_once('=')
            && k.trim().eq_ignore_ascii_case(key)
        {
            return Some(v.trim().to_owned());
        }
    }
    None
}

fn read_bool_property(cfg_path: &Path, key: &str, default: bool) -> bool {
    match read_property(cfg_path, key) {
        Some(v) if is_falsy(&v) => false,
        Some(v) if is_truthy(&v) => true,
        _ => default,
    }
}

fn write_properties(cfg_path: &Path, pairs: &[(&str, String)]) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(cfg_path).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    let mut seen = vec![false; pairs.len()];
    for line in existing.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('#')
            && let Some((k, _)) = trimmed.split_once('=')
        {
            let key_trimmed = k.trim();
            if let Some(idx) = pairs
                .iter()
                .position(|(pk, _)| key_trimmed.eq_ignore_ascii_case(pk))
            {
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
    if let Some(parent) = cfg_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut content = lines.join("\n");
    content.push('\n');
    std::fs::write(cfg_path, content)
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
    read_bool_property(cfg_path, "auto-update", true)
}

/// Returns the background update check interval in minutes (clamped to `1..=10080`).
#[must_use]
pub fn get_auto_update_interval_minutes(cfg_path: &Path) -> u64 {
    if let Ok(val) = std::env::var("RADBYTE_AUTO_UPDATE_INTERVAL_MINUTES")
        && let Ok(mins) = val.trim().parse::<u64>()
    {
        return mins.clamp(
            MIN_AUTO_UPDATE_INTERVAL_MINUTES,
            MAX_AUTO_UPDATE_INTERVAL_MINUTES,
        );
    }
    read_property(cfg_path, "auto-update-interval-minutes")
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(DEFAULT_AUTO_UPDATE_INTERVAL_MINUTES, |m| {
            m.clamp(
                MIN_AUTO_UPDATE_INTERVAL_MINUTES,
                MAX_AUTO_UPDATE_INTERVAL_MINUTES,
            )
        })
}

/// Returns `true` when launcher self-updates are enabled.
#[must_use]
pub fn is_auto_update_launcher_enabled(cfg_path: &Path) -> bool {
    read_bool_property(cfg_path, "auto-update-launcher", true)
}

/// Returns `true` when `RadAPI` runtime updates are enabled.
#[must_use]
pub fn is_auto_update_radapi_enabled(cfg_path: &Path) -> bool {
    read_bool_property(cfg_path, "auto-update-radapi", true)
}

/// Returns `true` when Minecraft version module updates are enabled.
#[must_use]
pub fn is_auto_update_versions_enabled(cfg_path: &Path) -> bool {
    read_bool_property(cfg_path, "auto-update-versions", true)
}

/// Returns `true` when GitHub update checks for Radons in `radons/` are enabled.
#[must_use]
pub fn is_auto_update_radons_enabled(cfg_path: &Path) -> bool {
    read_bool_property(cfg_path, "auto-update-radons", true)
}

/// Loads all granular auto-update settings from `cfg_path` (`rad.properties`).
#[must_use]
pub fn load_auto_update_settings(cfg_path: &Path) -> AutoUpdateSettings {
    AutoUpdateSettings {
        enabled: is_auto_update_enabled(cfg_path),
        interval_minutes: get_auto_update_interval_minutes(cfg_path),
        launcher: is_auto_update_launcher_enabled(cfg_path),
        radapi: is_auto_update_radapi_enabled(cfg_path),
        versions: is_auto_update_versions_enabled(cfg_path),
        radons: is_auto_update_radons_enabled(cfg_path),
    }
}

/// Persists all granular auto-update settings in `cfg_path` (`rad.properties`).
pub fn save_auto_update_settings(
    cfg_path: &Path,
    settings: &AutoUpdateSettings,
) -> std::io::Result<()> {
    let interval = settings.interval_minutes.clamp(
        MIN_AUTO_UPDATE_INTERVAL_MINUTES,
        MAX_AUTO_UPDATE_INTERVAL_MINUTES,
    );
    let b = |v: bool| if v { "true" } else { "false" }.to_owned();
    write_properties(
        cfg_path,
        &[
            ("auto-update", b(settings.enabled)),
            ("auto-update-interval-minutes", interval.to_string()),
            ("auto-update-launcher", b(settings.launcher)),
            ("auto-update-radapi", b(settings.radapi)),
            ("auto-update-versions", b(settings.versions)),
            ("auto-update-radons", b(settings.radons)),
        ],
    )
}

/// Persists `auto-update=true|false` in `cfg_path` (`rad.properties`), preserving existing lines.
pub fn set_auto_update_setting(cfg_path: &Path, enabled: bool) -> std::io::Result<()> {
    let value = if enabled { "true" } else { "false" }.to_owned();
    write_properties(cfg_path, &[("auto-update", value)])
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

/// Writes `new_bytes` to `<current_exe>.new` so it can be applied on graceful restart.
pub fn stage_binary_update(new_bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut new_os = exe.as_os_str().to_owned();
    new_os.push(".new");
    let new_path = std::path::PathBuf::from(new_os);
    std::fs::write(&new_path, new_bytes).map_err(|e| format!("Failed to stage update: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new_path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(new_path)
}

/// Returns `true` when a staged `<current_exe>.new` binary is ready to apply.
#[must_use]
pub fn has_staged_binary_update() -> bool {
    std::env::current_exe().is_ok_and(|exe| {
        let mut new_os = exe.as_os_str().to_owned();
        new_os.push(".new");
        std::path::Path::new(&new_os).is_file()
    })
}

/// Applies a staged `<current_exe>.new` binary, spawns the updated binary, and exits.
pub fn apply_staged_binary_and_restart() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut new_os = exe.as_os_str().to_owned();
    new_os.push(".new");
    let new_path = std::path::PathBuf::from(new_os);
    if !new_path.is_file() {
        return Err("No staged update binary found".into());
    }

    let mut old_os = exe.as_os_str().to_owned();
    old_os.push(".old");
    let old_path = std::path::PathBuf::from(old_os);

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

/// Replaces the current executable on disk with `new_bytes`, spawns the updated binary, and exits.
pub fn replace_current_binary_and_restart(new_bytes: &[u8]) -> Result<(), String> {
    stage_binary_update(new_bytes)?;
    apply_staged_binary_and_restart()
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

    #[test]
    fn granular_auto_update_settings_and_interval_persist() {
        let dir =
            std::env::temp_dir().join(format!("rad-autoupdate-granular-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("rad.properties");

        let custom = AutoUpdateSettings {
            enabled: true,
            interval_minutes: 15,
            launcher: false,
            radapi: true,
            versions: false,
            radons: true,
        };
        save_auto_update_settings(&cfg, &custom).unwrap();
        let loaded = load_auto_update_settings(&cfg);
        assert_eq!(loaded, custom);
        assert_eq!(get_auto_update_interval_minutes(&cfg), 15);
        assert!(!is_auto_update_launcher_enabled(&cfg));
        assert!(is_auto_update_radapi_enabled(&cfg));
        assert!(!is_auto_update_versions_enabled(&cfg));
        assert!(is_auto_update_radons_enabled(&cfg));

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parse_github_repo_and_scan_radons_work() {
        assert_eq!(
            parse_github_repo("RadByte0/SampleRadon").as_deref(),
            Some("RadByte0/SampleRadon")
        );
        assert_eq!(
            parse_github_repo("https://github.com/RadByte0/SampleRadon.git").as_deref(),
            Some("RadByte0/SampleRadon")
        );
        assert_eq!(parse_github_repo("https://gitlab.com/a/b"), None);

        let dir = std::env::temp_dir().join(format!("rad-radon-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("hello.toml"),
            "id = \"hello\"\nversion = \"0.1.0\"\ngithub = \"https://github.com/RadByte0/HelloRadon\"\n",
        )
        .unwrap();
        let targets = scan_radon_github_targets(&dir);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, "hello");
        assert_eq!(targets[0].version, "0.1.0");
        assert_eq!(targets[0].repo, "RadByte0/HelloRadon");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
