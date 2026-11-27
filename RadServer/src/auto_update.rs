//! Automatic startup updater for `RadServer`, version modules, and `RadAPI` runtimes.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use rad_core::api_runtime::{api_library_filename, api_release_asset_name, api_target_info};
use rad_core::auto_update::{
    cleanup_old_binary, is_auto_update_enabled, is_newer_version,
    replace_current_binary_and_restart,
};
use sha2::{Digest, Sha256};

fn release_repo() -> &'static str {
    option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte")
}

fn fetch_latest_tag(client: &reqwest::blocking::Client, repo: &str) -> Option<String> {
    let rel: serde_json::Value = client
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .send()
        .and_then(|r| r.error_for_status())
        .ok()?
        .json()
        .ok()?;
    rel["tag_name"].as_str().map(str::to_owned)
}

fn download_bytes(
    client: &reqwest::blocking::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.take((max_bytes + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return Err("Invalid download payload size".into());
    }
    Ok(bytes)
}

/// Checks GitHub Releases for newer launcher, version library, and `RadAPI` updates when enabled.
pub fn run_startup_auto_update(cfg_path: &Path, work_dir: &Path, versions_dir: &Path) {
    cleanup_old_binary();
    if !is_auto_update_enabled(cfg_path) {
        return;
    }
    let Ok((target, _, lib_ext)) = api_target_info() else {
        return;
    };
    let exe_ext = if cfg!(windows) { ".exe" } else { "" };
    let repo = release_repo();
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("RadServer/", env!("CARGO_PKG_VERSION")))
        .build()
    else {
        return;
    };

    let Some(latest_tag) = fetch_latest_tag(&client, repo) else {
        return;
    };

    if is_newer_version(&latest_tag, env!("CARGO_PKG_VERSION")) {
        let asset = format!("rad-server-{target}{exe_ext}");
        let url = format!("https://github.com/{repo}/releases/download/{latest_tag}/{asset}");
        if let Ok(bytes) = download_bytes(&client, &url, 64 * 1024 * 1024) {
            tracing::info!(tag = %latest_tag, "auto-updated RadServer; restarting");
            if let Err(e) = replace_current_binary_and_restart(&bytes) {
                tracing::warn!(error = %e, "RadServer self-update failed");
            }
        }
    }

    // Update installed version library (`versions/26.3/`) if it came from an older release tag.
    let ver_dir = versions_dir.join("26.3");
    let manifest_path = ver_dir.join("version.json");
    let tag_marker = ver_dir.join(".release_tag");
    let current_ver_tag = std::fs::read_to_string(&tag_marker).unwrap_or_default();
    if manifest_path.is_file()
        && current_ver_tag.trim() != latest_tag
        && let Ok(raw) = std::fs::read_to_string(&manifest_path)
        && let Ok(mut json) = serde_json::from_str::<serde_json::Value>(&raw)
    {
        let asset = format!("rad-version-26.3-{target}.{lib_ext}");
        let url = format!("https://github.com/{repo}/releases/download/{latest_tag}/{asset}");
        if let Ok(bytes) = download_bytes(&client, &url, 64 * 1024 * 1024) {
            let lib_path = ver_dir.join(&asset);
            let tmp = ver_dir.join(format!("{asset}.tmp.{}", std::process::id()));
            if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &lib_path).is_ok() {
                let sha256 = format!("{:x}", Sha256::digest(&bytes));
                json["library"]["path"] = serde_json::Value::String(asset);
                json["library"]["size_bytes"] = serde_json::Value::from(bytes.len() as u64);
                json["library"]["sha256"] = serde_json::Value::String(sha256);
                if let Ok(updated) = serde_json::to_string_pretty(&json) {
                    let _ = std::fs::write(&manifest_path, updated);
                    let _ = std::fs::write(&tag_marker, &latest_tag);
                    tracing::info!(tag = %latest_tag, "auto-updated rad-version 26.3 library");
                }
            } else {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }

    // Update installed RadAPI runtime (`apis/v1/`) if present.
    let api_dir = work_dir.join("apis").join("v1");
    let api_tag_marker = api_dir.join(".release_tag");
    let current_api_tag = std::fs::read_to_string(&api_tag_marker).unwrap_or_default();
    if api_dir.is_dir()
        && current_api_tag.trim() != latest_tag
        && let (Ok(asset), Ok(local_name)) = (api_release_asset_name(1), api_library_filename(1))
    {
        let url = format!("https://github.com/{repo}/releases/download/{latest_tag}/{asset}");
        if let Ok(bytes) = download_bytes(&client, &url, 32 * 1024 * 1024) {
            let dest = api_dir.join(local_name);
            let tmp = api_dir.join(format!("api.tmp.{}", std::process::id()));
            if std::fs::write(&tmp, &bytes).is_ok() && std::fs::rename(&tmp, &dest).is_ok() {
                let _ = std::fs::write(&api_tag_marker, &latest_tag);
                tracing::info!(tag = %latest_tag, "auto-updated RadAPI v1 runtime");
            } else {
                let _ = std::fs::remove_file(&tmp);
            }
        }
    }
}
