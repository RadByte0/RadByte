//! RadServer — host launcher that verifies and dynamically loads the Minecraft version DLL.

#![warn(missing_docs)]

mod api_install;
mod version_download;
mod version_install;
mod version_selection;

use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use mimalloc::MiMalloc;
use rad_core::cli::{exit_with_pause, print_startup_banner, resolve_config_path};
use rad_core::console_ui::{ConsoleUi, make_log_writer};
use rad_core::version_package::{
    LoadedVersionPackage, RAD_VERSION_ABI_VERSION, RAD_VERSION_OK, RadServerStartOptions,
    RadVersionInfo, VersionManifest,
};
use tracing::{error, info};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

const SERVER_BANNER: &str = r"
  ____           _ ____
 |  _ \ __ _  __| / ___|  ___ _ ____   _____ _ __
 | |_) / _` |/ _` \___ \ / _ \ '__\ \ / / _ \ '__|
 |  _ < (_| | (_| |___) |  __/ |   \ V /  __/ |
 |_| \_\__,_|\__,_|____/ \___|_|    \_/ \___|_|
";

type FnAbiVersion = unsafe extern "C" fn() -> u32;
type FnVersionInfo = unsafe extern "C" fn(*mut RadVersionInfo) -> i32;
type FnRunServer = unsafe extern "C" fn(*const RadServerStartOptions) -> i32;

fn cstr_bytes_to_str(buf: &[u8]) -> &str {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    std::str::from_utf8(&buf[..end]).unwrap_or("<invalid-utf8>")
}

fn main() {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_writer(make_log_writer)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    print_startup_banner("RadServer", env!("CARGO_PKG_VERSION"), SERVER_BANNER);

    let cfg_path = resolve_config_path("rad.properties");
    let work_dir = cfg_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let versions_dir = resolve_versions_dir(&cfg_path);

    if let Err(e) = version_install::ensure_installed_headless(work_dir, &versions_dir) {
        error!(error = %e, "version setup required before starting RadServer");
        exit_with_pause(1);
    }

    let _console_ui = ConsoleUi::start("RadServer", env!("CARGO_PKG_VERSION"), "help, op, stop");

    let selected_version = match version_selection::load(&versions_dir) {
        Ok(version) => version,
        Err(e) => {
            error!(error = %e, "failed to select Minecraft version");
            exit_with_pause(1);
        }
    };
    let version_package =
        match LoadedVersionPackage::load_from_versions_dir(&versions_dir, &selected_version) {
            Ok(pkg) => pkg,
            Err(e) => {
                error!(
                    error = %e,
                    versions_dir = %versions_dir.display(),
                    "failed to load required Minecraft version package before opening port"
                );
                exit_with_pause(1);
            }
        };

    if let Err(e) = version_download::install_missing(&version_package) {
        error!(error = %e, "failed to install missing version library");
        exit_with_pause(1);
    }

    let lib_path = match version_package.verify_and_resolve_library_path() {
        Ok(p) => p,
        Err(e) => {
            error!(
                error = %e,
                versions_dir = %versions_dir.display(),
                "failed to verify native Minecraft version library before opening port"
            );
            exit_with_pause(1);
        }
    };

    let library = match unsafe { Library::new(&lib_path) } {
        Ok(lib) => lib,
        Err(e) => {
            error!(
                error = %e,
                library = %lib_path.display(),
                "failed to load native Minecraft version library"
            );
            exit_with_pause(1);
        }
    };

    let _api_library = match api_install::ensure_api_installed_if_needed(work_dir, 1) {
        Ok(Some(lib)) => {
            info!(api_version = 1, "loaded on-demand RadAPI runtime");
            Some(lib)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, api_version = 1, "RadAPI runtime unavailable; continuing with built-in API");
            None
        }
    };

    let exit_code = match launch_version_library(
        &library,
        &lib_path,
        &cfg_path,
        &versions_dir,
        version_package.manifest(),
    ) {
        Ok(code) => code,
        Err(msg) => {
            error!(library = %lib_path.display(), "{msg}");
            1
        }
    };

    if exit_code != 0 {
        exit_with_pause(exit_code);
    }
}

fn verify_loaded_library_info(
    info: &RadVersionInfo,
    manifest: &VersionManifest,
) -> Result<(), String> {
    if info.struct_size as usize != std::mem::size_of::<RadVersionInfo>() {
        return Err(format!(
            "version library RadVersionInfo struct_size mismatch: got {}, expected {}",
            info.struct_size,
            std::mem::size_of::<RadVersionInfo>()
        ));
    }
    if info.abi_version != RAD_VERSION_ABI_VERSION {
        return Err(format!(
            "version library info ABI mismatch: got {}, expected {RAD_VERSION_ABI_VERSION}",
            info.abi_version
        ));
    }
    let game_version = cstr_bytes_to_str(&info.game_version);
    if game_version != manifest.game_version
        || info.protocol_version != manifest.protocol_version
        || info.world_data_version != manifest.world_data_version
    {
        return Err(format!(
            "version library mismatch: got {game_version} (protocol {}, data_version {}), manifest expects {} (protocol {}, data_version {})",
            info.protocol_version,
            info.world_data_version,
            manifest.game_version,
            manifest.protocol_version,
            manifest.world_data_version
        ));
    }
    let target_os = cstr_bytes_to_str(&info.target_os);
    let target_arch = cstr_bytes_to_str(&info.target_arch);
    if target_os != std::env::consts::OS || target_arch != std::env::consts::ARCH {
        return Err(format!(
            "version library platform mismatch: got {target_os}/{target_arch}, host is {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }
    Ok(())
}

fn launch_version_library(
    library: &Library,
    lib_path: &Path,
    cfg_path: &Path,
    versions_dir: &Path,
    manifest: &VersionManifest,
) -> Result<i32, String> {
    unsafe {
        let abi_fn: Symbol<'_, FnAbiVersion> = library
            .get(b"rad_version_abi_version\0")
            .map_err(|e| format!("missing symbol rad_version_abi_version: {e}"))?;
        let actual_abi = abi_fn();
        if actual_abi != RAD_VERSION_ABI_VERSION {
            return Err(format!(
                "version library ABI mismatch: got {actual_abi}, expected {RAD_VERSION_ABI_VERSION}"
            ));
        }

        let info_fn: Symbol<'_, FnVersionInfo> = library
            .get(b"rad_version_info\0")
            .map_err(|e| format!("missing symbol rad_version_info: {e}"))?;
        let mut info = RadVersionInfo {
            struct_size: 0,
            abi_version: 0,
            protocol_version: 0,
            world_data_version: 0,
            game_version: [0; 32],
            target_os: [0; 32],
            target_arch: [0; 32],
        };
        let info_rc = info_fn(&mut info);
        if info_rc != RAD_VERSION_OK {
            return Err(format!("rad_version_info failed with code {info_rc}"));
        }
        verify_loaded_library_info(&info, manifest)?;
        let game_version = cstr_bytes_to_str(&info.game_version);

        info!(
            library = %lib_path.display(),
            game_version,
            protocol = info.protocol_version,
            world_data_version = info.world_data_version,
            abi_version = info.abi_version,
            "loaded native Minecraft version library"
        );

        let run_fn: Symbol<'_, FnRunServer> = library
            .get(b"rad_version_run_server\0")
            .map_err(|e| format!("missing symbol rad_version_run_server: {e}"))?;

        let cfg_str = cfg_path.to_string_lossy();
        let ver_str = versions_dir.to_string_lossy();
        let opts = RadServerStartOptions {
            struct_size: std::mem::size_of::<RadServerStartOptions>() as u32,
            abi_version: RAD_VERSION_ABI_VERSION,
            config_path_ptr: cfg_str.as_ptr(),
            config_path_len: cfg_str.len(),
            versions_dir_ptr: ver_str.as_ptr(),
            versions_dir_len: ver_str.len(),
        };
        Ok(run_fn(&opts))
    }
}

fn resolve_versions_dir(cfg_path: &Path) -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--versions-dir"
            && let Some(val) = args.next()
        {
            return PathBuf::from(val);
        }
        if let Some(val) = arg.strip_prefix("--versions-dir=") {
            return PathBuf::from(val);
        }
    }
    if let Some(parent) = cfg_path.parent()
        && !parent.as_os_str().is_empty()
    {
        let candidate = parent.join("versions");
        if candidate.exists() {
            return candidate;
        }
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(exe_dir) = exe.parent()
    {
        let candidate = exe_dir.join("versions");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("versions")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_cstr(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        let bytes = s.as_bytes();
        let len = bytes.len().min(31);
        out[..len].copy_from_slice(&bytes[..len]);
        out
    }

    fn sample_manifest() -> VersionManifest {
        VersionManifest {
            schema_version: 1,
            game_version: "26.3".to_owned(),
            protocol_version: 777,
            package_format_version: 1,
            world_data_version: 4790,
            source: "test".to_owned(),
            generated_utc: "2026-11-15T00:00:00Z".to_owned(),
            notes: None,
            library: None,
            files: Vec::new(),
        }
    }

    fn sample_info() -> RadVersionInfo {
        RadVersionInfo {
            struct_size: std::mem::size_of::<RadVersionInfo>() as u32,
            abi_version: RAD_VERSION_ABI_VERSION,
            protocol_version: 777,
            world_data_version: 4790,
            game_version: fixed_cstr("26.3"),
            target_os: fixed_cstr(std::env::consts::OS),
            target_arch: fixed_cstr(std::env::consts::ARCH),
        }
    }

    #[test]
    fn verify_loaded_library_info_accepts_match_and_rejects_mismatches() {
        let manifest = sample_manifest();
        let valid = sample_info();
        assert!(verify_loaded_library_info(&valid, &manifest).is_ok());

        let mut bad_abi = valid;
        bad_abi.abi_version = RAD_VERSION_ABI_VERSION + 1;
        assert!(verify_loaded_library_info(&bad_abi, &manifest).is_err());

        let mut bad_ver = valid;
        bad_ver.game_version = fixed_cstr("26.4");
        assert!(verify_loaded_library_info(&bad_ver, &manifest).is_err());

        let mut bad_proto = valid;
        bad_proto.protocol_version = 778;
        assert!(verify_loaded_library_info(&bad_proto, &manifest).is_err());

        let mut bad_data = valid;
        bad_data.world_data_version = 4791;
        assert!(verify_loaded_library_info(&bad_data, &manifest).is_err());

        let mut bad_os = valid;
        bad_os.target_os = fixed_cstr("plan9");
        assert!(verify_loaded_library_info(&bad_os, &manifest).is_err());
    }
}
