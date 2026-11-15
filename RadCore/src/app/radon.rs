//! Shared **Radon** manifest and target definitions for the RadByte ecosystem.
//!
//! All RadByte extensions (for `RadProxy`, `RadLimbo`, and `RadServer`) are unified as
//! **Radons** stored in `radons/` with a common manifest format.

use serde::{Deserialize, Serialize};

/// Target RadByte component(s) that a radon runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RadonTarget {
    /// Runs on `RadProxy`.
    Proxy,
    /// Runs on `RadLimbo`.
    Limbo,
    /// Runs on `RadServer`.
    Server,
    /// Runs on all RadByte components.
    All,
}

/// Shared metadata manifest for a radon in `radons/` (e.g. `my-radon.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadonManifest {
    /// Unique radon identifier.
    pub id: String,
    /// Radon semantic version string.
    pub version: String,
    /// Target RadByte Radon API version (currently `1`).
    #[serde(default = "default_api_version")]
    pub api_version: u32,
    /// Target components (`["proxy"]`, `["limbo"]`, `["server"]`, or `["all"]`).
    #[serde(default = "default_targets")]
    pub target: Vec<RadonTarget>,
    /// Optional human-readable description.
    #[serde(default)]
    pub description: String,
    /// Optional hostname match for declarative routing on `RadProxy`.
    #[serde(default)]
    pub route_host: Option<String>,
    /// Target backend socket address when `route_host` matches.
    #[serde(default)]
    pub route_to: Option<String>,
}

impl RadonManifest {
    /// Returns `true` if this radon targets `target` (or [`RadonTarget::All`]).
    #[must_use]
    pub fn matches_target(&self, target: RadonTarget) -> bool {
        self.target
            .iter()
            .any(|&t| t == target || t == RadonTarget::All)
    }
}

const fn default_api_version() -> u32 {
    1
}

fn default_targets() -> Vec<RadonTarget> {
    vec![RadonTarget::All]
}
