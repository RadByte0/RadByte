//! Shared `RadAPI` runtime library discovery and on-demand installation helpers.

use std::path::{Path, PathBuf};

/// Returns `(rust_target, library_prefix, shared_ext)` for the current OS and architecture.
pub fn api_target_info() -> Result<(&'static str, &'static str, &'static str), String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Ok(("x86_64-pc-windows-msvc", "", "dll")),
        ("windows", "aarch64") => Ok(("aarch64-pc-windows-msvc", "", "dll")),
        ("linux", "x86_64") if cfg!(target_env = "musl") => {
            Ok(("x86_64-unknown-linux-musl", "lib", "so"))
        }
        ("linux", "aarch64") if cfg!(target_env = "musl") => {
            Ok(("aarch64-unknown-linux-musl", "lib", "so"))
        }
        ("linux", "x86_64") => Ok(("x86_64-unknown-linux-gnu", "lib", "so")),
        ("linux", "aarch64") => Ok(("aarch64-unknown-linux-gnu", "lib", "so")),
        ("linux", "riscv64") => Ok(("riscv64gc-unknown-linux-musl", "lib", "so")),
        ("linux", "powerpc64") => Ok(("powerpc64le-unknown-linux-musl", "lib", "so")),
        ("linux", "x86") => Ok(("i686-unknown-linux-musl", "lib", "so")),
        ("macos", "x86_64") => Ok(("x86_64-apple-darwin", "lib", "dylib")),
        ("macos", "aarch64") => Ok(("aarch64-apple-darwin", "lib", "dylib")),
        (os, arch) => Err(format!(
            "Unsupported platform for RadAPI runtime: {os}/{arch}"
        )),
    }
}

/// Returns the local shared library filename for `api_version` (e.g., `rad_api_v1.dll`).
pub fn api_library_filename(api_version: u32) -> Result<String, String> {
    let (_, prefix, ext) = api_target_info()?;
    Ok(format!("{prefix}rad_api_v{api_version}.{ext}"))
}

/// Returns the GitHub Release asset filename for `api_version` on the current platform.
pub fn api_release_asset_name(api_version: u32) -> Result<String, String> {
    let (target, _, ext) = api_target_info()?;
    Ok(format!("rad-api-v{api_version}-{target}.{ext}"))
}

/// Resolves `base_dir/apis/v{api_version}/<library_filename>`.
pub fn resolve_api_library_path(base_dir: &Path, api_version: u32) -> Result<PathBuf, String> {
    Ok(base_dir
        .join("apis")
        .join(format!("v{api_version}"))
        .join(api_library_filename(api_version)?))
}

/// Returns `true` when `radons_dir` contains at least one `.wasm` or `.toml` radon file.
#[must_use]
pub fn has_radons_in_dir(radons_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(radons_dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("wasm") || ext.eq_ignore_ascii_case("toml")
                })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_radons_in_dir_detects_empty_vs_populated() {
        let dir = std::env::temp_dir().join(format!("rad-api-rt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!has_radons_in_dir(&dir));
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        assert!(!has_radons_in_dir(&dir));
        std::fs::write(dir.join("plugin.wasm"), b"\0asm").unwrap();
        assert!(has_radons_in_dir(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
