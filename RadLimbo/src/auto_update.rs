//! Automatic startup updater for `RadLimbo`.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use rad_core::api_runtime::api_target_info;
use rad_core::auto_update::{
    cleanup_old_binary, get_auto_update_interval_minutes, has_staged_binary_update,
    is_auto_update_enabled, is_auto_update_launcher_enabled, is_auto_update_radons_enabled,
    is_newer_version, replace_current_binary_and_restart, scan_radon_github_targets,
    stage_binary_update,
};

fn release_repo() -> &'static str {
    option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte")
}

/// Checks GitHub Releases for a newer `RadLimbo` launcher when enabled.
pub fn run_startup_auto_update(cfg_path: &Path) {
    cleanup_old_binary();
    if !is_auto_update_enabled(cfg_path) {
        return;
    }
    let Ok((target, _, _)) = api_target_info() else {
        return;
    };
    let exe_ext = if cfg!(windows) { ".exe" } else { "" };
    let repo = release_repo();
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("RadLimbo/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return;
    };
    let Ok(rel) = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.json::<serde_json::Value>())
    else {
        return;
    };
    let Some(latest_tag) = rel["tag_name"].as_str() else {
        return;
    };
    if is_auto_update_launcher_enabled(cfg_path)
        && is_newer_version(latest_tag, env!("CARGO_PKG_VERSION"))
    {
        let asset = format!("rad-limbo-{target}{exe_ext}");
        let url = format!("https://github.com/{repo}/releases/download/{latest_tag}/{asset}");
        if let Ok(resp) = client.get(&url).send().and_then(|r| r.error_for_status()) {
            let mut bytes = Vec::new();
            if resp
                .take(64 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .is_ok()
                && !bytes.is_empty()
                && bytes.len() <= 64 * 1024 * 1024
            {
                tracing::info!(tag = %latest_tag, "auto-updated RadLimbo; restarting");
                if let Err(e) = replace_current_binary_and_restart(&bytes) {
                    tracing::warn!(error = %e, "RadLimbo self-update failed");
                }
            }
        }
    }

    if is_auto_update_radons_enabled(cfg_path) {
        let radons_dir = cfg_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join("radons");
        check_radon_updates(&client, &radons_dir);
    }
}

fn check_radon_updates(client: &reqwest::blocking::Client, radons_dir: &Path) {
    for target in scan_radon_github_targets(radons_dir) {
        let Ok(rel) = client
            .get(format!(
                "https://api.github.com/repos/{}/releases/latest",
                target.repo
            ))
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json::<serde_json::Value>())
        else {
            continue;
        };
        if let Some(latest_tag) = rel["tag_name"].as_str()
            && is_newer_version(latest_tag, &target.version)
        {
            tracing::info!(
                radon = %target.id,
                current = %target.version,
                latest = %latest_tag,
                repo = %target.repo,
                "newer Radon release available on GitHub"
            );
        }
    }
}

/// Downloads and stages a newer `RadLimbo` binary in the background while running.
pub fn check_and_stage_in_background() -> Option<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent(concat!("RadLimbo/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    check_and_stage_with_client(&client)
}

fn check_and_stage_with_client(client: &reqwest::blocking::Client) -> Option<String> {
    let (target, _, _) = api_target_info().ok()?;
    let exe_ext = if cfg!(windows) { ".exe" } else { "" };
    let repo = release_repo();
    let rel: serde_json::Value = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .ok()?
        .json()
        .ok()?;
    let latest_tag = rel["tag_name"].as_str()?.to_owned();
    if !is_newer_version(&latest_tag, env!("CARGO_PKG_VERSION")) {
        return None;
    }
    if !has_staged_binary_update() {
        let asset = format!("rad-limbo-{target}{exe_ext}");
        let url = format!("https://github.com/{repo}/releases/download/{latest_tag}/{asset}");
        let resp = client
            .get(&url)
            .send()
            .and_then(|r| r.error_for_status())
            .ok()?;
        let mut bytes = Vec::new();
        resp.take(64 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.is_empty() || bytes.len() > 64 * 1024 * 1024 {
            return None;
        }
        stage_binary_update(&bytes).ok()?;
    }
    Some(latest_tag)
}

/// Spawns a background watcher that checks for updates at the configured interval while running.
pub fn spawn_background_watcher(cfg_path: std::path::PathBuf) {
    if !is_auto_update_enabled(&cfg_path) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(60));
        let Ok(client) = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("RadLimbo/", env!("CARGO_PKG_VERSION")))
            .build()
        else {
            return;
        };
        let radons_dir = cfg_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join("radons");
        loop {
            if !is_auto_update_enabled(&cfg_path) {
                return;
            }
            if is_auto_update_radons_enabled(&cfg_path) {
                check_radon_updates(&client, &radons_dir);
            }
            if is_auto_update_launcher_enabled(&cfg_path)
                && let Some(tag) = check_and_stage_with_client(&client)
            {
                tracing::info!(
                    tag = %tag,
                    "update downloaded in background; type 'update' to restart and apply"
                );
                return;
            }
            let mins = get_auto_update_interval_minutes(&cfg_path);
            std::thread::sleep(Duration::from_secs(mins.saturating_mul(60)));
        }
    });
}
