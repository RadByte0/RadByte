//! Automatic startup updater for `RadLimbo`.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use rad_core::api_runtime::api_target_info;
use rad_core::auto_update::{
    cleanup_old_binary, is_auto_update_enabled, is_newer_version,
    replace_current_binary_and_restart,
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
    if is_newer_version(latest_tag, env!("CARGO_PKG_VERSION")) {
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
}
