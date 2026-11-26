//! # RadAPI — Versioned Extension API for RadByte
//!
//! `rad-api` provides a stable, versioned interface for building **Radons**
//! (plugins/extensions) for `RadProxy` and the broader RadByte ecosystem.
//!
//! ## API Versioning & Selective Loading
//! - Enable only the API version you need in `Cargo.toml`:
//!   - `features = ["v1"]` (or `default-features = false, features = ["v1"]` for WASM)
//!   - `features = ["v2"]` (reserved for future v2 API)
//! - [`v1`] — Current stable API (`API_VERSION = 1`), covering [`v1::ProxyRadon`],
//!   [`v1::RadonContext`], [`v1::RadonAction`], manifest re-exports, and the
//!   cross-platform [`v1::wasm`] guest SDK.
//! - `v2` — Extended API module with command, server switch, and plugin
//!   message hooks (enabled via the `v2` Cargo feature).
//! - `latest` aliases the newest enabled stable API module.

#![warn(missing_docs)]

#[cfg(not(target_arch = "wasm32"))]
pub mod ffi;

#[cfg(feature = "v1")]
pub mod v1;

#[cfg(feature = "v2")]
pub mod v2;

/// Alias to the latest enabled `RadAPI` version.
#[cfg(feature = "v1")]
pub use v1 as latest;

/// Alias to `v2` when compiled with only the `v2` feature.
#[cfg(all(feature = "v2", not(feature = "v1")))]
pub use v2 as latest;

#[cfg(test)]
mod tests {
    #[cfg(feature = "v1")]
    #[test]
    fn v1_wasm_constants_and_stubs_work() {
        assert_eq!(crate::v1::API_VERSION, 1);
        assert_eq!(crate::v1::wasm::ACTION_CONTINUE, 0);
        assert_eq!(crate::v1::wasm::ACTION_DENY, 1);
        assert_eq!(crate::v1::wasm::ACTION_ALLOW, 0);
        assert_eq!(crate::v1::wasm::ACTION_CANCEL, 1);
        assert!(crate::v1::wasm::host().is_empty());
        assert!(crate::v1::wasm::username().is_empty());
        assert!(crate::v1::wasm::ip().is_empty());
        assert!(crate::v1::wasm::config().is_empty());
        crate::v1::wasm::set_deny_reason("test");
        crate::v1::wasm::send_chat("hi");
        crate::v1::wasm::broadcast_chat("all");
        crate::v1::wasm::teleport_player(0.5, 64.0, 0.5);
        crate::v1::wasm::set_block(0, 64, 0, 1);
    }

    #[cfg(feature = "v2")]
    #[test]
    fn v2_version_constant() {
        assert_eq!(crate::v2::API_VERSION, 2);
    }
}
