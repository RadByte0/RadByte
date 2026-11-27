//! Automatic startup updater for `RadProxy` and `RadAPI` runtimes.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use rad_core::api_runtime::{api_library_filename, api_release_asset_name, api_target_info};
use rad_core::auto_update::{
    cleanup_old_binary, is_auto_update_enabled, is_newer_version,
    replace_current_binary_and_restart,
};

fn release_repo() -> &'static str {
    option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte")
}

/// Checks GitHub Releases for newer `RadProxy` and `RadAPI` updates when enabled.
pub fn run_startup_auto_update(cfg_path: &Path, work_dir: &Path) {
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
        .user_agent(concat!("RadProxy/", env!("CARGO_PKG_VERSION")))
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

    if is_newer_version(latest_tag, env!("CARGO_PKG_VERSION")) {
        let asset = format!("rad-proxy-{target}{exe_ext}");
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
                tracing::info!(tag = %latest_tag, "auto-updated RadProxy; restarting");
                if let Err(e) = replace_current_binary_and_restart(&bytes) {
                    tracing::warn!(error = %e, "RadProxy self-update failed");
                }
            }
        }
    }

    let api_dir = work_dir.join("apis").join("v1");
    let api_tag_marker = api_dir.join(".release_tag");
    let current_api_tag = std::fs::read_to_string(&api_tag_marker).unwrap_or_default();
    if api_dir.is_dir()
        && current_api_tag.trim() != latest_tag
        && let (Ok(asset), Ok(local_name)) = (api_release_asset_name(1), api_library_filename(1))
    {
        let url = format!("https://github.com/{repo}/releases/download/{latest_tag}/{asset}");
        if let Ok(resp) = client.get(&url).send().and_then(|r| r.error_for_status()) {
            let mut bytes = Vec::new();
            if resp
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .is_ok()
                && !bytes.is_empty()
                && bytes.len() <= 32 * 1024 * 1024
            {
                let dest = api_dir.join(local_name);
                let tmp = api_dir.join(format!("api.tmp.{}", std::process::id()));
                if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &dest).is_ok() {
                    let _ = std::fs::write(&api_tag_marker, latest_tag);
                    tracing::info!(tag = %latest_tag, "auto-updated RadAPI v1 runtime");
                } else {
                    let _ = std::fs::remove_file(&tmp);
                }
            }
        }
    }
}
