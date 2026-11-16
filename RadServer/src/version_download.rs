//! Download a missing native module from the exact release that built the host.

use std::io::{Read, Write};
use std::path::{Component, Path};
use std::time::Duration;

use rad_core::version_package::{LoadedVersionPackage, RAD_VERSION_ABI_VERSION};
use sha2::{Digest, Sha256};

const MAX_LIBRARY_SIZE: u64 = 256 * 1024 * 1024;

pub fn install_missing(package: &LoadedVersionPackage) -> Result<(), String> {
    let Some(lib) = package.manifest().library.as_ref() else {
        return Ok(());
    };
    let mut components = Path::new(&lib.path).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return Ok(()); // Existing verifier handles other paths and rejects unsafe ones.
    }
    let destination = package.package_dir().join(&lib.path);
    if destination.symlink_metadata().is_ok() {
        return Ok(()); // Existing files are checked by the normal package verifier.
    }
    if package.manifest().game_version != "26.3" {
        return Err("automatic installation is not available for this game version".into());
    }
    if lib.abi_version != RAD_VERSION_ABI_VERSION
        || lib.target_os != std::env::consts::OS
        || lib.target_arch != std::env::consts::ARCH
        || lib.size_bytes == 0
        || lib.size_bytes > MAX_LIBRARY_SIZE
    {
        return Err("library metadata does not match this platform or has an invalid size".into());
    }
    let tag = option_env!("RADBYTE_RELEASE_TAG")
        .filter(|s| {
            s.starts_with('v')
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
        .ok_or("automatic library installation is available only in tagged release builds")?;
    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        _ => return Err("unsupported platform for automatic library installation".into()),
    };
    let asset = match std::env::consts::OS {
        "windows" => format!("rad_version_26_3-{target}.dll"),
        "linux" => format!("librad_version_26_3-{target}.so"),
        "macos" => format!("librad_version_26_3-{target}.dylib"),
        _ => unreachable!(),
    };
    let repository = option_env!("RADBYTE_RELEASE_REPOSITORY").unwrap_or("RadByte0/RadByte");
    if !repository
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
    {
        return Err("invalid release repository".into());
    }
    let url = format!("https://github.com/{repository}/releases/download/{tag}/{asset}");
    tracing::info!(%url, "downloading missing version library");
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("library download failed: {e}"))?;
    let mut bytes = Vec::new();
    response
        .take(lib.size_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    if bytes.len() as u64 != lib.size_bytes || hash != lib.sha256 {
        return Err(
            "downloaded library does not match the package size and SHA-256; no file installed"
                .into(),
        );
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|e| format!("cannot create {}: {e}", destination.display()))?;
    if let Err(error) = file.write_all(&bytes) {
        drop(file);
        let _ = std::fs::remove_file(&destination);
        return Err(format!("cannot write {}: {error}", destination.display()));
    }
    Ok(())
}
