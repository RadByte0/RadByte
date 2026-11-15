//! Lightweight Minecraft JSON [`TextComponent`] builder.
//!
//! Used for server MOTDs, disconnect reasons, chat messages, and titles across
//! `RadProxy`, `RadLimbo`, and `RadServer`.

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// A Minecraft JSON text component.
///
/// # Example
/// ```
/// use rad_core::text::TextComponent;
///
/// let motd = TextComponent::text("RadByte Proxy ")
///     .color("aqua")
///     .bold()
///     .extra(TextComponent::text("v0.1").color("gray").not_bold());
///
/// let json = motd.to_json().unwrap();
/// assert!(json.contains("RadByte Proxy"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TextComponent {
    /// Plain text content of this component.
    pub text: String,

    /// Named color (e.g., `"red"`, `"aqua"`) or hex color (`"#FF5555"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// Whether the text is bold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bold: Option<bool>,

    /// Whether the text is italic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub italic: Option<bool>,

    /// Whether the text is underlined.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub underlined: Option<bool>,

    /// Whether the text has strikethrough.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strikethrough: Option<bool>,

    /// Whether the text is obfuscated (`&k`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub obfuscated: Option<bool>,

    /// Child components appended after `text`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<TextComponent>,
}

impl TextComponent {
    /// Creates a new plain text component.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Default::default()
        }
    }

    /// Sets the component color (named or `#RRGGBB`).
    #[must_use]
    pub fn color(mut self, color: impl Into<String>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Enables bold formatting.
    #[must_use]
    pub const fn bold(mut self) -> Self {
        self.bold = Some(true);
        self
    }

    /// Explicitly disables bold formatting (useful for child components).
    #[must_use]
    pub const fn not_bold(mut self) -> Self {
        self.bold = Some(false);
        self
    }

    /// Enables italic formatting.
    #[must_use]
    pub const fn italic(mut self) -> Self {
        self.italic = Some(true);
        self
    }

    /// Enables underline formatting.
    #[must_use]
    pub const fn underlined(mut self) -> Self {
        self.underlined = Some(true);
        self
    }

    /// Appends a child [`TextComponent`].
    #[must_use]
    pub fn extra(mut self, child: TextComponent) -> Self {
        self.extra.push(child);
        self
    }

    /// Serializes this component into a compact JSON string.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

impl From<&str> for TextComponent {
    fn from(value: &str) -> Self {
        Self::text(value)
    }
}

impl From<String> for TextComponent {
    fn from(value: String) -> Self {
        Self::text(value)
    }
}
