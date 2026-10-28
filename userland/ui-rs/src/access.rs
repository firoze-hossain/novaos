//! # NovaOS Global Accessibility and Font Scaling System
//!
//! Provides global accessibility configuration (`AccessibilityConfig`),
//! scalable font multiplier settings (`FontScale`), WCAG AAA high-contrast
//! theme helpers, and binary persistence in `ACCESS.CFG` (magic "NVAC", 64 bytes).

use crate::color::{Theme, ThemePalette};
use crate::cursor::CursorTheme;
use crate::font::Font;

/// Global font scaling factor supporting 100%, 125%, 150%, and 200%.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FontScale {
    Normal = 100,      // 100% (1.0x baseline)
    Medium = 125,      // 125% (1.25x large)
    Large = 150,       // 150% (1.50x larger)
    ExtraLarge = 200,  // 200% (2.00x extra large / high accessibility)
}

impl FontScale {
    /// Percentage integer representation (100, 125, 150, 200).
    pub const fn percent(&self) -> u16 {
        *self as u16
    }

    /// Converts a percentage number to the nearest `FontScale` level.
    pub const fn from_percent(pct: u16) -> Self {
        if pct >= 180 {
            FontScale::ExtraLarge
        } else if pct >= 140 {
            FontScale::Large
        } else if pct >= 115 {
            FontScale::Medium
        } else {
            FontScale::Normal
        }
    }

    /// Cycles to the next available font scaling tier.
    pub const fn cycle_next(&self) -> Self {
        match self {
            FontScale::Normal => FontScale::Medium,
            FontScale::Medium => FontScale::Large,
            FontScale::Large => FontScale::ExtraLarge,
            FontScale::ExtraLarge => FontScale::Normal,
        }
    }

    /// Short percentage label for compact UI pills and buttons.
    pub const fn label(&self) -> &'static str {
        match self {
            FontScale::Normal => "100%",
            FontScale::Medium => "125%",
            FontScale::Large => "150%",
            FontScale::ExtraLarge => "200%",
        }
    }

    /// Human-readable descriptive name.
    pub const fn display_name(&self) -> &'static str {
        match self {
            FontScale::Normal => "Normal (100%)",
            FontScale::Medium => "Medium (125%)",
            FontScale::Large => "Large (150%)",
            FontScale::ExtraLarge => "Extra Large (200%)",
        }
    }

    /// Scales a pixel measurement proportionally according to this scale factor.
    pub const fn scale_px(&self, px: i32) -> i32 {
        (px * (*self as i32) + 50) / 100
    }
}

pub const ACCESS_CFG_MAGIC: [u8; 4] = *b"NVAC";
pub const ACCESS_CFG_VERSION: u8 = 1;
pub const ACCESS_CFG_SIZE: usize = 64;

/// Global accessibility and appearance configuration structure.
/// Stored in `/ACCESS.CFG` on the FAT32 root partition (magic "NVAC", 64 bytes).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AccessibilityConfig {
    pub theme: Theme,
    pub font_scale: FontScale,
    pub high_contrast_cursor: bool,
    pub cursor_theme: CursorTheme,
    pub show_focus_rects: bool,
    pub large_titlebars: bool,
}

impl Default for AccessibilityConfig {
    fn default() -> Self {
        AccessibilityConfig {
            theme: Theme::Dark,
            font_scale: FontScale::Normal,
            high_contrast_cursor: false,
            cursor_theme: CursorTheme::ModernDark,
            show_focus_rects: false,
            large_titlebars: false,
        }
    }
}

impl AccessibilityConfig {
    /// Standard baseline default configuration.
    pub const fn new() -> Self {
        AccessibilityConfig {
            theme: Theme::Dark,
            font_scale: FontScale::Normal,
            high_contrast_cursor: false,
            cursor_theme: CursorTheme::ModernDark,
            show_focus_rects: false,
            large_titlebars: false,
        }
    }

    /// High-contrast accessible preset with maximized contrast and prominent cursor.
    pub const fn high_contrast() -> Self {
        AccessibilityConfig {
            theme: Theme::HighContrast,
            font_scale: FontScale::Large,
            high_contrast_cursor: true,
            cursor_theme: CursorTheme::HighContrast,
            show_focus_rects: true,
            large_titlebars: true,
        }
    }

    /// Returns the active semantic theme palette.
    pub const fn effective_palette(&self) -> ThemePalette {
        self.theme.palette()
    }

    /// Returns the effective cursor theme, prioritizing high-contrast if enabled.
    pub const fn effective_cursor_theme(&self) -> CursorTheme {
        if self.theme.is_high_contrast() || self.high_contrast_cursor {
            CursorTheme::HighContrast
        } else {
            self.cursor_theme
        }
    }

    /// Scales a font according to this accessibility configuration.
    pub fn scale_font(&self, base_font: &Font) -> Font {
        base_font.scaled(self.font_scale)
    }

    /// Computes titlebar height adapted for font scaling and accessibility flags.
    pub fn titlebar_height(&self, base_h: i32) -> i32 {
        if self.large_titlebars || self.font_scale != FontScale::Normal {
            let scaled = self.font_scale.scale_px(base_h);
            if scaled < base_h + 4 && self.font_scale != FontScale::Normal {
                base_h + 4
            } else {
                scaled
            }
        } else {
            base_h
        }
    }

    /// Computes menubar height adapted for font scaling.
    pub fn menubar_height(&self, base_h: i32) -> i32 {
        if base_h <= 0 {
            0
        } else if self.font_scale != FontScale::Normal {
            self.font_scale.scale_px(base_h).max(base_h)
        } else {
            base_h
        }
    }

    /// Computes taskbar/dock height adapted for font scaling.
    pub fn taskbar_height(&self, base_h: i32) -> i32 {
        if base_h <= 0 {
            0
        } else if self.font_scale != FontScale::Normal {
            self.font_scale.scale_px(base_h).max(base_h)
        } else {
            base_h
        }
    }

    /// Serializes configuration into a deterministic 64-byte binary block with CRC checksum.
    pub fn serialize(&self, out: &mut [u8; ACCESS_CFG_SIZE]) {
        for b in out.iter_mut() {
            *b = 0;
        }

        // 0..4: Magic "NVAC"
        out[0..4].copy_from_slice(&ACCESS_CFG_MAGIC);
        // 4: Version (1)
        out[4] = ACCESS_CFG_VERSION;
        // 5: Theme (0: Dark, 1: Light, 2: HighContrast)
        out[5] = match self.theme {
            Theme::Dark => 0,
            Theme::Light => 1,
            Theme::HighContrast => 2,
        };
        // 6: Font Scale (100, 125, 150, 200)
        out[6] = self.font_scale as u8;
        // 7: High Contrast Cursor
        out[7] = if self.high_contrast_cursor { 1 } else { 0 };
        // 8: Cursor Theme (0: Classic, 1: ModernDark, 2: ModernLight, 3: Amber, 4: HighContrast)
        out[8] = match self.cursor_theme {
            CursorTheme::Classic => 0,
            CursorTheme::ModernDark => 1,
            CursorTheme::ModernLight => 2,
            CursorTheme::Amber => 3,
            CursorTheme::HighContrast => 4,
        };
        // 9: Show Focus Rectangles
        out[9] = if self.show_focus_rects { 1 } else { 0 };
        // 10: Large Titlebars
        out[10] = if self.large_titlebars { 1 } else { 0 };

        // Calculate 16-bit checksum over bytes 0..62
        let mut sum: u16 = 0;
        for i in 0..62 {
            sum = sum.wrapping_add(out[i] as u16);
        }
        out[62] = (sum & 0xFF) as u8;
        out[63] = ((sum >> 8) & 0xFF) as u8;
    }

    /// Deserializes and validates a 64-byte binary block from `/ACCESS.CFG`.
    pub fn deserialize(buf: &[u8]) -> Option<Self> {
        if buf.len() < ACCESS_CFG_SIZE {
            return None;
        }
        if buf[0..4] != ACCESS_CFG_MAGIC {
            return None;
        }
        if buf[4] != ACCESS_CFG_VERSION {
            return None;
        }

        // Verify checksum
        let mut sum: u16 = 0;
        for i in 0..62 {
            sum = sum.wrapping_add(buf[i] as u16);
        }
        let stored_sum = (buf[62] as u16) | ((buf[63] as u16) << 8);
        if sum != stored_sum {
            return None;
        }

        let theme = match buf[5] {
            0 => Theme::Dark,
            1 => Theme::Light,
            2 => Theme::HighContrast,
            _ => Theme::Dark,
        };

        let font_scale = FontScale::from_percent(buf[6] as u16);
        let high_contrast_cursor = buf[7] != 0;

        let cursor_theme = match buf[8] {
            0 => CursorTheme::Classic,
            1 => CursorTheme::ModernDark,
            2 => CursorTheme::ModernLight,
            3 => CursorTheme::Amber,
            4 => CursorTheme::HighContrast,
            _ => CursorTheme::ModernDark,
        };

        let show_focus_rects = buf[9] != 0;
        let large_titlebars = buf[10] != 0;

        Some(AccessibilityConfig {
            theme,
            font_scale,
            high_contrast_cursor,
            cursor_theme,
            show_focus_rects,
            large_titlebars,
        })
    }
}
