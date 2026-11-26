//! On-demand installation and verification of versioned `RadAPI` runtime libraries for `RadProxy`.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use libloading::{Library, Symbol};
use rad_core::api_runtime::{api_release_asset_name, has_radons_in_dir, resolve_api_library_path};

type FnApiAbiVersion = unsafe extern "C" fn() -> u32;
type FnApiSupportedVersions = unsafe extern "C" fn(*mut u32, usize) -> usize;

fn release_repo() -> Result<&'static str, String> {
    let repo = option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte");
    if !repo
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"/.-_".contains(&b))
    {
        return Err("Invalid release repository".into());
    }
    Ok(repo)
}

fn download_api_library(dest: &Path, api_version: u32) -> Result<(), String> {
    let repo = release_repo()?;
    let asset_name = api_release_asset_name(api_version)?;
    let url = match option_env!("RADBYTE_RELEASE_TAG").filter(|s| !s.is_empty()) {
        Some(tag) => format!("https://github.com/{repo}/releases/download/{tag}/{asset_name}"),
        None => format!("https://github.com/{repo}/releases/latest/download/{asset_name}"),
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("RadProxy/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            format!("Cannot download RadAPI v{api_version} runtime ({asset_name}): {e}")
        })?;
    let mut bytes = Vec::new();
    resp.take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.is_empty() || bytes.len() > 32 * 1024 * 1024 {
        return Err(format!(
            "Invalid RadAPI v{api_version} library payload size"
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = dest.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

fn verify_and_load_api(path: &Path, api_version: u32) -> Result<Library, String> {
    let lib = unsafe { Library::new(path) }
        .map_err(|e| format!("Failed to load {}: {e}", path.display()))?;
    let abi_fn: Symbol<'_, FnApiAbiVersion> = unsafe { lib.get(b"rad_api_abi_version\0") }
        .map_err(|e| format!("Missing rad_api_abi_version in {}: {e}", path.display()))?;
    let abi = unsafe { abi_fn() };
    if abi != 1 {
        return Err(format!(
            "Unsupported RadAPI ABI version {abi} in {}",
            path.display()
        ));
    }
    let versions_fn: Symbol<'_, FnApiSupportedVersions> =
        unsafe { lib.get(b"rad_api_supported_versions\0") }.map_err(|e| {
            format!(
                "Missing rad_api_supported_versions in {}: {e}",
                path.display()
            )
        })?;
    let mut buf = [0u32; 8];
    let count = unsafe { versions_fn(buf.as_mut_ptr(), buf.len()) };
    if !buf[..count.min(buf.len())].contains(&api_version) {
        return Err(format!(
            "RadAPI library {} does not support API v{api_version}",
            path.display()
        ));
    }
    Ok(lib)
}

/// Installs and verifies `RadAPI` v`api_version` only when `radons_dir` contains radons.
pub fn ensure_api_installed_if_needed(
    base_dir: &Path,
    radons_dir: &Path,
    api_version: u32,
) -> Result<Option<Library>, String> {
    if !has_radons_in_dir(radons_dir) {
        return Ok(None);
    }
    let path = resolve_api_library_path(base_dir, api_version)?;
    if !path.is_file() {
        tracing::info!(
            api_version,
            path = %path.display(),
            "radons detected; installing RadAPI runtime on demand"
        );
        download_api_library(&path, api_version)?;
    }
    verify_and_load_api(&path, api_version).map(Some)
}
