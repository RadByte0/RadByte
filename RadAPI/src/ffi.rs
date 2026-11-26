//! C-ABI runtime exports for dynamically loaded `rad-api` shared libraries.

use std::ffi::c_char;

/// Current C-ABI version exposed by `rad_api` shared libraries.
pub const RAD_API_ABI_VERSION: u32 = 1;

static PACKAGE_VERSION_CSTR: &[u8] = concat!(env!("CARGO_PKG_VERSION"), "\0").as_bytes();

/// Returns the C-ABI version of this `rad-api` shared library.
#[unsafe(no_mangle)]
pub extern "C" fn rad_api_abi_version() -> u32 {
    RAD_API_ABI_VERSION
}

/// Writes supported `RadAPI` major version numbers into `out_ptr` (up to `cap`) and returns the total count.
///
/// # Safety
/// If `out_ptr` is non-null, it must be valid for writing `cap` `u32` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rad_api_supported_versions(out_ptr: *mut u32, cap: usize) -> usize {
    let mut versions = [0u32; 2];
    let mut len = 0usize;
    #[cfg(feature = "v1")]
    {
        versions[len] = 1;
        len += 1;
    }
    #[cfg(feature = "v2")]
    {
        versions[len] = 2;
        len += 1;
    }
    if !out_ptr.is_null() && cap > 0 {
        let count = len.min(cap);
        unsafe {
            std::ptr::copy_nonoverlapping(versions.as_ptr(), out_ptr, count);
        }
    }
    len
}

/// Returns a null-terminated UTF-8 string containing the `rad-api` crate version.
#[unsafe(no_mangle)]
pub extern "C" fn rad_api_package_version() -> *const c_char {
    PACKAGE_VERSION_CSTR.as_ptr().cast()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_exports_report_abi_and_supported_versions() {
        assert_eq!(rad_api_abi_version(), RAD_API_ABI_VERSION);
        let mut buf = [0u32; 4];
        let count = unsafe { rad_api_supported_versions(buf.as_mut_ptr(), buf.len()) };
        assert!(count >= 1);
        assert_eq!(buf[0], 1);
        let ver = unsafe { std::ffi::CStr::from_ptr(rad_api_package_version()) }
            .to_str()
            .unwrap();
        assert!(!ver.is_empty());
    }
}
