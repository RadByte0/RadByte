//! **v2** RadAPI module (`API_VERSION = 2`) with extended command, server-switch,
//! and plugin-messaging hooks.
//!
//! Enable with `features = ["v2"]` in `Cargo.toml`.

/// `RadAPI` v2 version constant.
pub const API_VERSION: u32 = 2;

#[cfg(feature = "proxy")]
mod v2_types {
    use std::net::SocketAddr;

    /// Decision returned by a v2 command or plugin-message hook.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum EventResult {
        /// Pass the event through normally.
        Allow,
        /// Cancel/consume the event so it is not forwarded.
        Cancel,
    }

    /// Extended v2 trait for `RadProxy` radons.
    pub trait ProxyRadonV2: Send + Sync {
        /// Unique radon name.
        fn name(&self) -> &str;

        /// Radon version string.
        fn version(&self) -> &str;

        /// Called when a player executes a chat command (`/command ...`).
        fn on_command(&self, _username: &str, _command: &str) -> EventResult {
            EventResult::Allow
        }

        /// Called right before a player switches to a new backend server.
        fn on_server_switch(
            &self,
            _username: &str,
            _from_server: Option<&str>,
            _to_server: &str,
            _to_addr: SocketAddr,
        ) -> EventResult {
            EventResult::Allow
        }

        /// Called when a plugin message (`CustomPayload`) is intercepted on a channel.
        fn on_plugin_message(&self, _username: &str, _channel: &str, _data: &[u8]) -> EventResult {
            EventResult::Allow
        }
    }
}

#[cfg(feature = "proxy")]
pub use v2_types::*;
