//! Color Palette and Theme System for NovaOS
//!
//! Provides 32-bit XRGB/ARGB color primitives, alpha blending, standard color
//! constants, 16-color VGA palette backwards-compatibility, and semantic theme
//! palettes (Dark, Light, HighContrast).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const WHITE: Color = Color::rgb(255, 255, 255);
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b, a: 255 }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { r, g, b, a }
    }

    pub const fn from_hex(hex: u32) -> Self {
        Color {
            r: ((hex >> 16) & 0xFF) as u8,
            g: ((hex >> 8) & 0xFF) as u8,
            b: (hex & 0xFF) as u8,
            a: 255,
        }
    }

    pub const fn from_argb(argb: u32) -> Self {
        Color {
            r: ((argb >> 16) & 0xFF) as u8,
            g: ((argb >> 8) & 0xFF) as u8,
            b: (argb & 0xFF) as u8,
            a: ((argb >> 24) & 0xFF) as u8,
        }
    }

    pub const fn to_u32(&self) -> u32 {
        ((self.a as u32) << 24)
            | ((self.r as u32) << 16)
            | ((self.g as u32) << 8)
            | (self.b as u32)
    }

    pub const fn to_xrgb(&self) -> u32 {
        ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    pub const fn from_vga_index(idx: u8) -> Self {
        match idx & 0x0F {
            0 => Color::rgb(0, 0, 0),        // 0: Black
            1 => Color::rgb(0, 0, 170),      // 1: Blue
            2 => Color::rgb(0, 170, 0),      // 2: Green
            3 => Color::rgb(0, 170, 170),    // 3: Cyan
            4 => Color::rgb(170, 0, 0),      // 4: Red
            5 => Color::rgb(170, 0, 170),    // 5: Magenta
            6 => Color::rgb(170, 85, 0),     // 6: Brown
            7 => Color::rgb(170, 170, 170),  // 7: Light Gray
            8 => Color::rgb(85, 85, 85),     // 8: Dark Gray
            9 => Color::rgb(85, 85, 255),    // 9: Bright Blue
            10 => Color::rgb(85, 255, 85),   // 10: Bright Green
            11 => Color::rgb(85, 255, 255),  // 11: Bright Cyan
            12 => Color::rgb(255, 85, 85),   // 12: Bright Red
            13 => Color::rgb(255, 85, 255),  // 13: Bright Magenta
            14 => Color::rgb(255, 255, 85),  // 14: Yellow
            _ => Color::rgb(255, 255, 255),  // 15: Bright White
        }
    }

    pub fn to_vga_index(&self) -> u8 {
        let mut best_idx = 0u8;
        let mut best_dist = u32::MAX;
        let mut i = 0u8;
        while i < 16 {
            let v = Color::from_vga_index(i);
            let dr = self.r as i32 - v.r as i32;
            let dg = self.g as i32 - v.g as i32;
            let db = self.b as i32 - v.b as i32;
            let dist = (dr * dr + dg * dg + db * db) as u32;
            if dist < best_dist {
                best_dist = dist;
                best_idx = i;
            }
            i += 1;
        }
        best_idx
    }

    pub const fn blend_over(&self, bg: Color) -> Color {
        if self.a == 255 {
            return *self;
        }
        if self.a == 0 {
            return bg;
        }
        let alpha = self.a as u32;
        let inv_alpha = 255 - alpha;
        let r = ((self.r as u32 * alpha + bg.r as u32 * inv_alpha) / 255) as u8;
        let g = ((self.g as u32 * alpha + bg.g as u32 * inv_alpha) / 255) as u8;
        let b = ((self.b as u32 * alpha + bg.b as u32 * inv_alpha) / 255) as u8;
        Color { r, g, b, a: 255 }
    }

    pub const fn lighten(&self, amount: u8) -> Color {
        let r = if (self.r as u16 + amount as u16) > 255 { 255 } else { self.r + amount };
        let g = if (self.g as u16 + amount as u16) > 255 { 255 } else { self.g + amount };
        let b = if (self.b as u16 + amount as u16) > 255 { 255 } else { self.b + amount };
        Color { r, g, b, a: self.a }
    }

    pub const fn darken(&self, amount: u8) -> Color {
        let r = if self.r < amount { 0 } else { self.r - amount };
        let g = if self.g < amount { 0 } else { self.g - amount };
        let b = if self.b < amount { 0 } else { self.b - amount };
        Color { r, g, b, a: self.a }
    }
}

// Universal constants
pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
pub const BLACK: Color = Color::rgb(0, 0, 0);
pub const WHITE: Color = Color::rgb(255, 255, 255);

// Neutral Slate scale
pub const SLATE_50: Color  = Color::from_hex(0xF8FAFC);
pub const SLATE_100: Color = Color::from_hex(0xF1F5F9);
pub const SLATE_200: Color = Color::from_hex(0xE2E8F0);
pub const SLATE_300: Color = Color::from_hex(0xCBD5E1);
pub const SLATE_400: Color = Color::from_hex(0x94A3B8);
pub const SLATE_500: Color = Color::from_hex(0x64748B);
pub const SLATE_600: Color = Color::from_hex(0x475569);
pub const SLATE_700: Color = Color::from_hex(0x334155);
pub const SLATE_800: Color = Color::from_hex(0x1E293B);
pub const SLATE_900: Color = Color::from_hex(0x0F172A);
pub const SLATE_950: Color = Color::from_hex(0x080D1A);

// NovaOS Brand Accents
pub const NOVA_AZURE: Color       = Color::from_hex(0x007ACC);
pub const NOVA_AZURE_HOVER: Color = Color::from_hex(0x1B88D8);
pub const NOVA_AZURE_PRESS: Color = Color::from_hex(0x005E99);
pub const NOVA_CYAN: Color        = Color::from_hex(0x00B4D8);

// Status Colors
pub const STATUS_SUCCESS: Color       = Color::from_hex(0x22C55E); // Emerald Green
pub const STATUS_SUCCESS_HOVER: Color = Color::from_hex(0x16A34A);
pub const STATUS_WARNING: Color       = Color::from_hex(0xEAB308); // Amber Gold
pub const STATUS_WARNING_HOVER: Color = Color::from_hex(0xCA8A04);
pub const STATUS_DANGER: Color        = Color::from_hex(0xEF4444); // Crimson Red
pub const STATUS_DANGER_HOVER: Color  = Color::from_hex(0xDC2626);
pub const STATUS_INFO: Color          = Color::from_hex(0x38BDF8); // Sky Blue

/// Semantic theme palette giving every UI element an explicit role
#[derive(Clone, Copy, Debug)]
pub struct ThemePalette {
    pub desktop_bg: Color,
    pub window_bg: Color,
    pub surface_raised: Color,
    pub surface_sunken: Color,
    pub titlebar_active: Color,
    pub titlebar_inactive: Color,
    pub titlebar_text_active: Color,
    pub titlebar_text_inactive: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_inverse: Color,
    pub accent_primary: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub border_focused: Color,
    pub border_normal: Color,
    pub border_subtle: Color,
    pub taskbar_bg: Color,
    pub taskbar_item_active: Color,
    pub taskbar_item_inactive: Color,
    pub status_success: Color,
    pub status_warning: Color,
    pub status_danger: Color,
    pub status_info: Color,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Theme {
    Dark,
    Light,
    HighContrast,
}

impl Theme {
    pub const fn palette(&self) -> ThemePalette {
        match self {
            Theme::Dark => ThemePalette {
                desktop_bg: SLATE_950,
                window_bg: SLATE_900,
                surface_raised: SLATE_800,
                surface_sunken: Color::from_hex(0x0A0F1D),
                titlebar_active: SLATE_800,
                titlebar_inactive: SLATE_900,
                titlebar_text_active: SLATE_100,
                titlebar_text_inactive: SLATE_500,
                text_primary: SLATE_100,
                text_secondary: SLATE_400,
                text_muted: SLATE_600,
                text_inverse: SLATE_950,
                accent_primary: NOVA_AZURE,
                accent_hover: NOVA_AZURE_HOVER,
                accent_pressed: NOVA_AZURE_PRESS,
                border_focused: NOVA_AZURE,
                border_normal: SLATE_700,
                border_subtle: SLATE_800,
                taskbar_bg: Color::from_hex(0x0A0F1A),
                taskbar_item_active: SLATE_800,
                taskbar_item_inactive: Color::from_hex(0x0A0F1A),
                status_success: STATUS_SUCCESS,
                status_warning: STATUS_WARNING,
                status_danger: STATUS_DANGER,
                status_info: STATUS_INFO,
            },
            Theme::Light => ThemePalette {
                desktop_bg: Color::from_hex(0xD8E2DC),
                window_bg: SLATE_50,
                surface_raised: WHITE,
                surface_sunken: SLATE_100,
                titlebar_active: SLATE_200,
                titlebar_inactive: SLATE_100,
                titlebar_text_active: SLATE_900,
                titlebar_text_inactive: SLATE_500,
                text_primary: SLATE_900,
                text_secondary: SLATE_600,
                text_muted: SLATE_400,
                text_inverse: WHITE,
                accent_primary: NOVA_AZURE,
                accent_hover: NOVA_AZURE_HOVER,
                accent_pressed: NOVA_AZURE_PRESS,
                border_focused: NOVA_AZURE,
                border_normal: SLATE_300,
                border_subtle: SLATE_200,
                taskbar_bg: SLATE_200,
                taskbar_item_active: WHITE,
                taskbar_item_inactive: SLATE_200,
                status_success: STATUS_SUCCESS,
                status_warning: STATUS_WARNING,
                status_danger: STATUS_DANGER,
                status_info: STATUS_INFO,
            },
            Theme::HighContrast => ThemePalette {
                desktop_bg: BLACK,
                window_bg: BLACK,
                surface_raised: Color::from_hex(0x222222),
                surface_sunken: Color::from_hex(0x111111),
                titlebar_active: WHITE,
                titlebar_inactive: BLACK,
                titlebar_text_active: BLACK,
                titlebar_text_inactive: WHITE,
                text_primary: WHITE,
                text_secondary: WHITE,
                text_muted: WHITE,
                text_inverse: BLACK,
                accent_primary: Color::from_hex(0xFFFF00), // Pure yellow
                accent_hover: Color::from_hex(0xFFFF88),
                accent_pressed: Color::from_hex(0xCCCC00),
                border_focused: Color::from_hex(0xFFFF00),
                border_normal: WHITE,
                border_subtle: WHITE,
                taskbar_bg: BLACK,
                taskbar_item_active: WHITE,
                taskbar_item_inactive: BLACK,
                status_success: Color::from_hex(0x00FF00),
                status_warning: Color::from_hex(0xFFFF00),
                status_danger: Color::from_hex(0xFF0000),
                status_info: Color::from_hex(0x00FFFF),
            },
        }
    }
}
