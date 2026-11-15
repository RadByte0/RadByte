//! Minecraft protocol version registry and helpers.

use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{MINECRAFT_VERSION, PROTOCOL_VERSION};

/// Describes a Minecraft target version separating network protocol, content tables, and world format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameVersion {
    /// Human-readable Minecraft version (e.g. `"26.3"`).
    pub game_version: String,
    /// Network protocol ID (e.g. `777`).
    pub protocol_version: i32,
    /// RadByte version-package content schema version.
    pub content_version: u32,
    /// World Anvil/save data version.
    pub world_data_version: i32,
}

impl GameVersion {
    /// Returns the descriptor for Minecraft Java 26.3 (protocol `777`).
    #[must_use]
    pub fn v26_3() -> Self {
        Self {
            game_version: MINECRAFT_VERSION.to_owned(),
            protocol_version: PROTOCOL_VERSION,
            content_version: 1,
            world_data_version: 4790,
        }
    }
}

/// Represents a Minecraft Java Edition network protocol version.
///
/// Designed for forward compatibility: unknown or newer versions are preserved
/// as [`ProtocolVersion::Other`] so proxies and version adapters can inspect or
/// translate them cleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolVersion {
    /// Minecraft Java Edition 26.3 (`777`).
    V26_3,
    /// Any other protocol version number.
    Other(i32),
}

impl ProtocolVersion {
    /// Returns the numeric wire protocol version ID.
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::V26_3 => PROTOCOL_VERSION,
            Self::Other(id) => id,
        }
    }

    /// Returns a human-readable version name when known.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::V26_3 => MINECRAFT_VERSION,
            Self::Other(_) => "unknown",
        }
    }

    /// Returns `true` if this version matches the native target version (`26.3`).
    #[must_use]
    pub const fn is_supported_native(self) -> bool {
        matches!(self, Self::V26_3)
    }
}

impl From<i32> for ProtocolVersion {
    #[inline]
    fn from(value: i32) -> Self {
        match value {
            PROTOCOL_VERSION => Self::V26_3,
            other => Self::Other(other),
        }
    }
}

impl From<ProtocolVersion> for i32 {
    #[inline]
    fn from(value: ProtocolVersion) -> Self {
        value.id()
    }
}

impl fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::V26_3 => write!(f, "{} ({})", MINECRAFT_VERSION, PROTOCOL_VERSION),
            Self::Other(id) => write!(f, "unknown ({id})"),
        }
    }
}
