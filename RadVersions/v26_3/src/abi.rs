//! C ABI exports for the loadable Minecraft Java 26.3 (`protocol 777`) version library.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use rad_core::version_package::{
    LoadedVersionPackage, RAD_VERSION_ABI_VERSION, RAD_VERSION_ERR_BUFFER_TOO_SMALL,
    RAD_VERSION_ERR_INTERNAL, RAD_VERSION_ERR_INVALID_ARG, RAD_VERSION_ERR_NOT_FOUND,
    RAD_VERSION_OK, RadServerStartOptions, RadVersionInfo,
};
use rad_version::MC_26_3;

static ACTIVE_PKG: OnceLock<Arc<LoadedVersionPackage>> = OnceLock::new();

pub(crate) fn set_active_package(pkg: Arc<LoadedVersionPackage>) {
    let _ = ACTIVE_PKG.set(pkg);
}

fn active_pkg() -> Option<&'static LoadedVersionPackage> {
    ACTIVE_PKG.get().map(Arc::as_ref)
}

fn write_fixed_cstr(dst: &mut [u8], s: &str) {
    dst.fill(0);
    let bytes = s.as_bytes();
    let n = bytes.len().min(dst.len().saturating_sub(1));
    dst[..n].copy_from_slice(&bytes[..n]);
}

unsafe fn read_utf8_slice<'a>(ptr: *const u8, len: usize) -> Option<&'a str> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(slice).ok()
}

unsafe fn write_out_str(s: &str, out_ptr: *mut u8, out_cap: usize, out_len: *mut usize) -> i32 {
    let bytes = s.as_bytes();
    if !out_len.is_null() {
        unsafe { *out_len = bytes.len() };
    }
    if out_ptr.is_null() || out_cap < bytes.len() {
        return RAD_VERSION_ERR_BUFFER_TOO_SMALL;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out_ptr, bytes.len());
    }
    RAD_VERSION_OK
}

/// Returns the C ABI version implemented by this Minecraft version library.
#[unsafe(no_mangle)]
pub extern "C" fn rad_version_abi_version() -> u32 {
    RAD_VERSION_ABI_VERSION
}

/// Returns the Minecraft network protocol version (`777`).
#[unsafe(no_mangle)]
pub extern "C" fn rad_version_protocol_version() -> i32 {
    MC_26_3.protocol_version
}

/// Writes version, protocol, and target metadata into `out`.
///
/// # Safety
/// `out` must be a valid, non-null, properly aligned pointer to [`RadVersionInfo`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_info(out: *mut RadVersionInfo) -> i32 {
    if out.is_null() {
        return RAD_VERSION_ERR_INVALID_ARG;
    }
    let mut info = RadVersionInfo {
        struct_size: std::mem::size_of::<RadVersionInfo>() as u32,
        abi_version: MC_26_3.abi_version,
        protocol_version: MC_26_3.protocol_version,
        world_data_version: MC_26_3.world_data_version,
        game_version: [0; 32],
        target_os: [0; 32],
        target_arch: [0; 32],
    };
    write_fixed_cstr(&mut info.game_version, MC_26_3.game_version);
    write_fixed_cstr(&mut info.target_os, std::env::consts::OS);
    write_fixed_cstr(&mut info.target_arch, std::env::consts::ARCH);
    unsafe { *out = info };
    RAD_VERSION_OK
}

/// Runs the Minecraft 26.3 (`protocol 777`) server from this version library.
///
/// # Safety
/// `opts` must be a valid, non-null pointer to [`RadServerStartOptions`] whose byte slices
/// remain valid for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_run_server(opts: *const RadServerStartOptions) -> i32 {
    if opts.is_null() {
        return RAD_VERSION_ERR_INVALID_ARG;
    }
    let opts = unsafe { &*opts };
    if opts.struct_size as usize != std::mem::size_of::<RadServerStartOptions>()
        || opts.abi_version != RAD_VERSION_ABI_VERSION
    {
        return RAD_VERSION_ERR_INVALID_ARG;
    }
    let Some(config_path) =
        (unsafe { read_utf8_slice(opts.config_path_ptr, opts.config_path_len) })
    else {
        return RAD_VERSION_ERR_INVALID_ARG;
    };
    let Some(versions_dir) =
        (unsafe { read_utf8_slice(opts.versions_dir_ptr, opts.versions_dir_len) })
    else {
        return RAD_VERSION_ERR_INVALID_ARG;
    };

    let result = std::panic::catch_unwind(|| {
        crate::run_server(&PathBuf::from(config_path), &PathBuf::from(versions_dir))
    });
    match result {
        Ok(code) => code,
        Err(_) => RAD_VERSION_ERR_INTERNAL,
    }
}

/// Resolves a Minecraft 26.3 item name (e.g. `"minecraft:stone"` or `"stone"`) to its numeric ID.
/// Returns `RAD_VERSION_ERR_NOT_FOUND` if unknown or `RAD_VERSION_ERR_INVALID_ARG` on invalid input.
///
/// # Safety
/// `name_ptr` must be valid for `name_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_item_id(name_ptr: *const u8, name_len: usize) -> i32 {
    let Some(name) = (unsafe { read_utf8_slice(name_ptr, name_len) }) else {
        return RAD_VERSION_ERR_INVALID_ARG;
    };
    active_pkg()
        .and_then(|pkg| pkg.item_id_by_name(name))
        .unwrap_or(RAD_VERSION_ERR_NOT_FOUND)
}

/// Resolves a numeric item ID to its namespaced name (e.g. `"minecraft:stone"`).
///
/// # Safety
/// `out_ptr` (if non-null) must be valid for `out_cap` bytes; `out_len` (if non-null) must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_item_name(
    id: i32,
    out_ptr: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    let Some(name) = active_pkg().and_then(|pkg| pkg.item_name_by_id(id)) else {
        return RAD_VERSION_ERR_NOT_FOUND;
    };
    unsafe { write_out_str(name, out_ptr, out_cap, out_len) }
}

/// Resolves a Minecraft 26.3 block name to its default block state ID.
///
/// # Safety
/// `name_ptr` must be valid for `name_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_block_default_state(
    name_ptr: *const u8,
    name_len: usize,
) -> i32 {
    let Some(name) = (unsafe { read_utf8_slice(name_ptr, name_len) }) else {
        return RAD_VERSION_ERR_INVALID_ARG;
    };
    active_pkg()
        .and_then(|pkg| pkg.block_by_name(name))
        .map(|b| i32::from(b.default_state))
        .unwrap_or(RAD_VERSION_ERR_NOT_FOUND)
}

/// Resolves a block state ID to its namespaced block name.
///
/// # Safety
/// `out_ptr` (if non-null) must be valid for `out_cap` bytes; `out_len` (if non-null) must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_block_name_by_state(
    state_id: u16,
    out_ptr: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    let Some(name) = active_pkg().and_then(|pkg| pkg.block_name_by_state(state_id)) else {
        return RAD_VERSION_ERR_NOT_FOUND;
    };
    unsafe { write_out_str(name, out_ptr, out_cap, out_len) }
}

/// Resolves a Minecraft 26.3 entity type name to its numeric ID.
///
/// # Safety
/// `name_ptr` must be valid for `name_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_entity_id(name_ptr: *const u8, name_len: usize) -> i32 {
    let Some(name) = (unsafe { read_utf8_slice(name_ptr, name_len) }) else {
        return RAD_VERSION_ERR_INVALID_ARG;
    };
    active_pkg()
        .and_then(|pkg| pkg.entity_id_by_name(name))
        .unwrap_or(RAD_VERSION_ERR_NOT_FOUND)
}

/// Resolves a Minecraft 26.3 menu/screen type name to its numeric ID.
///
/// # Safety
/// `name_ptr` must be valid for `name_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_menu_id(name_ptr: *const u8, name_len: usize) -> i32 {
    let Some(name) = (unsafe { read_utf8_slice(name_ptr, name_len) }) else {
        return RAD_VERSION_ERR_INVALID_ARG;
    };
    active_pkg()
        .and_then(|pkg| pkg.menu_id_by_name(name))
        .unwrap_or(RAD_VERSION_ERR_NOT_FOUND)
}

/// Formats a Minecraft 26.3 (`protocol 777`) Server List Ping JSON payload.
///
/// # Safety
/// `motd_ptr` must be valid for `motd_len` bytes; `out_ptr`/`out_len` must be valid if non-null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_version_format_status_json(
    motd_ptr: *const u8,
    motd_len: usize,
    online: i32,
    max_players: i32,
    out_ptr: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    let motd = if motd_ptr.is_null() || motd_len == 0 {
        ""
    } else {
        let Some(s) = (unsafe { read_utf8_slice(motd_ptr, motd_len) }) else {
            return RAD_VERSION_ERR_INVALID_ARG;
        };
        s
    };
    let json = serde_json::json!({
        "version": { "name": MC_26_3.game_version, "protocol": MC_26_3.protocol_version },
        "players": { "max": max_players, "online": online, "sample": [] },
        "description": { "text": motd },
        "enforcesSecureChat": false
    })
    .to_string();
    unsafe { write_out_str(&json, out_ptr, out_cap, out_len) }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;

    #[test]
    fn rejects_null_pointers_and_invalid_start_options() {
        unsafe {
            assert_eq!(
                rad_version_info(std::ptr::null_mut()),
                RAD_VERSION_ERR_INVALID_ARG
            );
            assert_eq!(
                rad_version_run_server(std::ptr::null()),
                RAD_VERSION_ERR_INVALID_ARG
            );
            assert_eq!(
                rad_version_item_id(std::ptr::null(), 0),
                RAD_VERSION_ERR_INVALID_ARG
            );
            let opts = RadServerStartOptions {
                struct_size: 0,
                abi_version: RAD_VERSION_ABI_VERSION,
                config_path_ptr: std::ptr::null(),
                config_path_len: 0,
                versions_dir_ptr: std::ptr::null(),
                versions_dir_len: 0,
            };
            assert_eq!(rad_version_run_server(&opts), RAD_VERSION_ERR_INVALID_ARG);
        }
    }

    #[test]
    fn status_size_probe_and_small_buffer_preserve_output_boundaries() {
        let motd = "Héllo \"player\"\n";
        let mut required = 0;
        unsafe {
            assert_eq!(
                rad_version_format_status_json(
                    motd.as_ptr(),
                    motd.len(),
                    1,
                    20,
                    std::ptr::null_mut(),
                    0,
                    &mut required
                ),
                RAD_VERSION_ERR_BUFFER_TOO_SMALL
            );
            let mut small = [0xAB; 8];
            assert_eq!(
                rad_version_format_status_json(
                    motd.as_ptr(),
                    motd.len(),
                    1,
                    20,
                    small.as_mut_ptr(),
                    small.len(),
                    &mut required
                ),
                RAD_VERSION_ERR_BUFFER_TOO_SMALL
            );
            assert_eq!(small, [0xAB; 8]);
            let mut output = vec![0xAB; required + 1];
            let capacity = required;
            assert_eq!(
                rad_version_format_status_json(
                    motd.as_ptr(),
                    motd.len(),
                    1,
                    20,
                    output.as_mut_ptr(),
                    capacity,
                    &mut required
                ),
                RAD_VERSION_OK
            );
            assert_eq!(output[required], 0xAB);
            let json: serde_json::Value = serde_json::from_slice(&output[..required]).unwrap();
            assert_eq!(json["description"]["text"], motd);
        }
    }

    #[test]
    fn invalid_utf8_is_rejected_without_writing_output() {
        let input = [0xFF];
        let mut output = [0xAB; 8];
        let mut written = 99;
        unsafe {
            assert_eq!(
                rad_version_format_status_json(
                    input.as_ptr(),
                    input.len(),
                    0,
                    20,
                    output.as_mut_ptr(),
                    output.len(),
                    &mut written
                ),
                RAD_VERSION_ERR_INVALID_ARG
            );
        }
        assert_eq!(output, [0xAB; 8]);
        assert_eq!(written, 99);
    }
}
