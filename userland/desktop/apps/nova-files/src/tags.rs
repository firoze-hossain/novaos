//! userland/desktop/apps/nova-files/src/tags.rs
//!
//! macOS-Style Colored Tags System for NovaFiles.
//! Supports 7 standard macOS tag colors with RGB tokens and filtering.

use nova_ui::prelude::Color;

/// Standard macOS-style tag colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileTag {
    None,
    Red,
    Orange,
    Yellow,
    Green,
    Blue,
    Purple,
    Gray,
}

impl FileTag {
    /// Returns the human-readable display name of the tag.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Gray => "Gray",
        }
    }

    /// Returns the primary RGBA color for this tag.
    pub const fn color(&self) -> Color {
        match self {
            Self::None => Color::rgba(0, 0, 0, 0),
            Self::Red => Color::rgba(255, 59, 48, 255),       // #FF3B30
            Self::Orange => Color::rgba(255, 149, 0, 255),    // #FF9500
            Self::Yellow => Color::rgba(255, 204, 0, 255),    // #FFCC00
            Self::Green => Color::rgba(52, 199, 89, 255),     // #34C759
            Self::Blue => Color::rgba(0, 122, 255, 255),      // #007AFF
            Self::Purple => Color::rgba(175, 82, 222, 255),   // #AF52DE
            Self::Gray => Color::rgba(142, 142, 147, 255),    // #8E8E93
        }
    }

    /// Returns a translucent background color for tag badges/chips.
    pub const fn badge_bg(&self) -> Color {
        match self {
            Self::None => Color::rgba(0, 0, 0, 0),
            Self::Red => Color::rgba(255, 59, 48, 48),
            Self::Orange => Color::rgba(255, 149, 0, 48),
            Self::Yellow => Color::rgba(255, 204, 0, 48),
            Self::Green => Color::rgba(52, 199, 89, 48),
            Self::Blue => Color::rgba(0, 122, 255, 48),
            Self::Purple => Color::rgba(175, 82, 222, 48),
            Self::Gray => Color::rgba(142, 142, 147, 48),
        }
    }

    /// Array of all available selectable tags.
    pub const ALL_TAGS: [FileTag; 7] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
        Self::Gray,
    ];

    /// Parse tag name from string.
    pub fn from_str_name(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "red" => Self::Red,
            "orange" => Self::Orange,
            "yellow" => Self::Yellow,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "purple" => Self::Purple,
            "gray" | "grey" => Self::Gray,
            _ => Self::None,
        }
    }
}
