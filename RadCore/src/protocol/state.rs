//! Minecraft protocol connection states.

use core::fmt;

use crate::error::{Error, Result};

/// Represents the current protocol phase of a Minecraft client-server connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConnectionState {
    /// Initial state before the client sends a `Handshake` packet.
    Handshaking,
    /// Server list ping state (`StatusRequest` / `PingRequest`).
    Status,
    /// Authentication and encryption negotiation state.
    Login,
    /// Registry synchronization and client configuration state (1.20.2+).
    Configuration,
    /// Active gameplay state.
    Play,
}

impl ConnectionState {
    /// Returns a human-readable static name for the connection state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Handshaking => "Handshaking",
            Self::Status => "Status",
            Self::Login => "Login",
            Self::Configuration => "Configuration",
            Self::Play => "Play",
        }
    }

    /// Parses the `next_state` field from a `Handshake` packet.
    ///
    /// Valid values in the Minecraft protocol are:
    /// - `1` -> [`ConnectionState::Status`]
    /// - `2` -> [`ConnectionState::Login`]
    /// - `3` -> [`ConnectionState::Login`] (Transfer intent in 1.20.5+, enters Login first)
    pub fn from_handshake_next_state(value: i32) -> Result<Self> {
        match value {
            1 => Ok(Self::Status),
            2 | 3 => Ok(Self::Login),
            other => Err(Error::InvalidState(other)),
        }
    }
}

impl fmt::Display for ConnectionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
