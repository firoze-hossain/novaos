//! userland/libs/nova-ui/src/theme/fluent_macos.rs
//!
//! Fluent Acrylic & macOS Material Design System for NovaOS.
//! Synthesizes Windows 11 Fluent Design (acrylic/mica materials, glassmorphism, accent colors,
//! multi-tier smooth elevation shadows) and macOS Sequoia (rounded squircle corners 8-20px,
//! delicate borders, vibrant color palettes, crisp dark/light themes).

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;

use crate::layout::Rect;
use crate::render::{Color, BufferSurface, WHITE};
use crate::scene::{NodeId, SceneGraph};

// ============================================================================
// 1. Theme Mode & Accent Palette
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccentPalette {
    FluentBlue,    // Windows 11 Fluent Accent #0078D4
    SequoiaBlue,   // macOS Sequoia Accent #007AFF
    Purple,        // macOS System Purple #AF52DE
    Pink,          // macOS System Pink #FF2D55
    Red,           // macOS System Red #FF3B30
    Orange,        // macOS System Orange #FF9500
    Yellow,        // macOS System Yellow #FFCC00
    Green,         // macOS System Green #34C759
    Graphite,      // macOS System Graphite #8E8E93
}

impl AccentPalette {
    pub fn to_colors(&self, mode: ThemeMode) -> (Color, Color, Color, Color) {
        // (Primary, Hover, Pressed, Subtle/Tint)
        match self {
            AccentPalette::FluentBlue => match mode {
                ThemeMode::Dark => (
                    Color::rgb(0, 120, 212),
                    Color::rgb(24, 134, 230),
                    Color::rgb(0, 102, 180),
                    Color::rgba(0, 120, 212, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(0, 103, 184),
                    Color::rgb(0, 120, 212),
                    Color::rgb(0, 85, 155),
                    Color::rgba(0, 103, 184, 25),
                ),
            },
            AccentPalette::SequoiaBlue => match mode {
                ThemeMode::Dark => (
                    Color::rgb(10, 132, 255),
                    Color::rgb(40, 150, 255),
                    Color::rgb(0, 110, 230),
                    Color::rgba(10, 132, 255, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(0, 122, 255),
                    Color::rgb(30, 140, 255),
                    Color::rgb(0, 100, 220),
                    Color::rgba(0, 122, 255, 25),
                ),
            },
            AccentPalette::Purple => match mode {
                ThemeMode::Dark => (
                    Color::rgb(191, 90, 242),
                    Color::rgb(205, 115, 245),
                    Color::rgb(170, 70, 220),
                    Color::rgba(191, 90, 242, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(175, 82, 222),
                    Color::rgb(190, 100, 235),
                    Color::rgb(150, 60, 200),
                    Color::rgba(175, 82, 222, 25),
                ),
            },
            AccentPalette::Pink => match mode {
                ThemeMode::Dark => (
                    Color::rgb(255, 55, 95),
                    Color::rgb(255, 85, 120),
                    Color::rgb(230, 35, 75),
                    Color::rgba(255, 55, 95, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(255, 45, 85),
                    Color::rgb(255, 75, 110),
                    Color::rgb(220, 25, 65),
                    Color::rgba(255, 45, 85, 25),
                ),
            },
            AccentPalette::Red => match mode {
                ThemeMode::Dark => (
                    Color::rgb(255, 69, 58),
                    Color::rgb(255, 95, 85),
                    Color::rgb(225, 45, 35),
                    Color::rgba(255, 69, 58, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(255, 59, 48),
                    Color::rgb(255, 85, 75),
                    Color::rgb(215, 35, 25),
                    Color::rgba(255, 59, 48, 25),
                ),
            },
            AccentPalette::Orange => match mode {
                ThemeMode::Dark => (
                    Color::rgb(255, 159, 10),
                    Color::rgb(255, 180, 45),
                    Color::rgb(230, 135, 0),
                    Color::rgba(255, 159, 10, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(255, 149, 0),
                    Color::rgb(255, 170, 30),
                    Color::rgb(220, 125, 0),
                    Color::rgba(255, 149, 0, 25),
                ),
            },
            AccentPalette::Yellow => match mode {
                ThemeMode::Dark => (
                    Color::rgb(255, 214, 10),
                    Color::rgb(255, 225, 50),
                    Color::rgb(230, 190, 0),
                    Color::rgba(255, 214, 10, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(255, 204, 0),
                    Color::rgb(255, 215, 35),
                    Color::rgb(220, 175, 0),
                    Color::rgba(255, 204, 0, 25),
                ),
            },
            AccentPalette::Green => match mode {
                ThemeMode::Dark => (
                    Color::rgb(48, 209, 88),
                    Color::rgb(75, 225, 110),
                    Color::rgb(30, 180, 65),
                    Color::rgba(48, 209, 88, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(52, 199, 89),
                    Color::rgb(70, 215, 105),
                    Color::rgb(35, 175, 70),
                    Color::rgba(52, 199, 89, 25),
                ),
            },
            AccentPalette::Graphite => match mode {
                ThemeMode::Dark => (
                    Color::rgb(142, 142, 147),
                    Color::rgb(165, 165, 170),
                    Color::rgb(120, 120, 125),
                    Color::rgba(142, 142, 147, 35),
                ),
                ThemeMode::Light => (
                    Color::rgb(142, 142, 147),
                    Color::rgb(160, 160, 165),
                    Color::rgb(120, 120, 125),
                    Color::rgba(142, 142, 147, 25),
                ),
            },
        }
    }
}

// ============================================================================
// 2. Corner Radii Tokens (macOS Squircle / Rounded Profiles)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CornerTokens {
    pub none: i32,      // 0px (sharp edges)
    pub small: i32,     // 4px (badges, tags, chips)
    pub medium: i32,    // 8px (standard buttons, text inputs)
    pub card: i32,      // 10px (content cards, list items)
    pub large: i32,     // 12px (tooltips, flyout popovers)
    pub window: i32,    // 16px (top-level application windows)
    pub modal: i32,     // 20px (system sheets, alert modals)
    pub pill: i32,      // 9999px (capsule buttons, search pills)
}

impl Default for CornerTokens {
    fn default() -> Self {
        Self {
            none: 0,
            small: 4,
            medium: 8,
            card: 10,
            large: 12,
            window: 16,
            modal: 20,
            pill: 9999,
        }
    }
}

// ============================================================================
// 3. Multi-Tier Elevation Shadows (Windows 11 Fluent)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElevationShadow {
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: i32,
    pub spread: i32,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ElevationTier {
    pub ambient: ElevationShadow,
    pub directional: ElevationShadow,
}

impl ElevationTier {
    pub const fn new(
        amb_y: i32, amb_blur: i32, amb_alpha: u8,
        dir_y: i32, dir_blur: i32, dir_alpha: u8,
    ) -> Self {
        Self {
            ambient: ElevationShadow {
                offset_x: 0,
                offset_y: amb_y,
                blur: amb_blur,
                spread: 0,
                color: Color::rgba(0, 0, 0, amb_alpha),
            },
            directional: ElevationShadow {
                offset_x: 0,
                offset_y: dir_y,
                blur: dir_blur,
                spread: 0,
                color: Color::rgba(0, 0, 0, dir_alpha),
            },
        }
    }
}

pub fn get_elevation_tiers(mode: ThemeMode) -> [ElevationTier; 6] {
    let factor: f32 = match mode {
        ThemeMode::Dark => 1.4,   // Darker shadows in dark mode for contrast
        ThemeMode::Light => 1.0,  // Subtle soft shadows in light mode
    };

    [
        // Level 0: Resting Flat (0px)
        ElevationTier::new(0, 0, 0, 0, 0, 0),
        // Level 1: Resting Card / Subsurface (2px)
        ElevationTier::new(1, 2, (18.0 * factor) as u8, 2, 4, (32.0 * factor) as u8),
        // Level 2: Hovered Card / Active Button (4px)
        ElevationTier::new(2, 4, (22.0 * factor) as u8, 4, 8, (45.0 * factor) as u8),
        // Level 3: Popover / Flyout / Dropdown Menu (8px)
        ElevationTier::new(3, 8, (28.0 * factor) as u8, 8, 16, (60.0 * factor) as u8),
        // Level 4: Floating Tool Window / Dialog (12px)
        ElevationTier::new(4, 12, (35.0 * factor) as u8, 12, 24, (75.0 * factor) as u8),
        // Level 5: Modal / Alert / System Notification (20px)
        ElevationTier::new(6, 20, (45.0 * factor) as u8, 20, 36, (90.0 * factor) as u8),
    ]
}

// ============================================================================
// 4. Color Tokens (Fluent + macOS Unified Palette)
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorTokens {
    // Canvas & Backdrops
    pub app_background: Color,
    pub canvas_surface: Color,
    pub card_background: Color,
    pub card_hover: Color,
    pub card_pressed: Color,
    pub popup_background: Color,

    // Acrylic & Mica Materials
    pub acrylic_tint: Color,
    pub acrylic_blur_radius: i32,
    pub mica_tint: Color,
    pub mica_blur_radius: i32,
    pub glass_border: Color,

    // Subtle & Delicate Borders
    pub border_subtle: Color,
    pub border_strong: Color,
    pub divider: Color,

    // Typography
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_on_accent: Color,

    // Interactive Accent
    pub accent_primary: Color,
    pub accent_hover: Color,
    pub accent_pressed: Color,
    pub accent_subtle: Color,
    pub focus_ring: Color,

    // Semantic Status
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
}

impl ColorTokens {
    pub fn build(mode: ThemeMode, accent: AccentPalette) -> Self {
        let (acc_p, acc_h, acc_press, acc_sub) = accent.to_colors(mode);

        match mode {
            ThemeMode::Dark => Self {
                // Windows 11 Fluent Dark / macOS Sequoia Dark
                app_background: Color::rgb(26, 26, 26),
                canvas_surface: Color::rgb(32, 32, 32),
                card_background: Color::rgb(44, 44, 46),       // macOS System Background Dark
                card_hover: Color::rgb(54, 54, 56),
                card_pressed: Color::rgb(38, 38, 40),
                popup_background: Color::rgb(48, 48, 50),

                // Acrylic / Mica Glassmorphic Tokens
                acrylic_tint: Color::rgba(30, 30, 30, 160),    // 63% opacity tinted scrim
                acrylic_blur_radius: 20,
                mica_tint: Color::rgba(32, 32, 32, 220),       // 86% subtle desktop tint
                mica_blur_radius: 36,
                glass_border: Color::rgba(255, 255, 255, 30),  // 1px delicate specular highlight

                // Delicate Borders
                border_subtle: Color::rgba(255, 255, 255, 22),
                border_strong: Color::rgba(255, 255, 255, 48),
                divider: Color::rgba(255, 255, 255, 18),

                // Typography
                text_primary: Color::rgb(245, 245, 247),       // Crisp Apple off-white
                text_secondary: Color::rgb(186, 186, 192),
                text_muted: Color::rgb(142, 142, 147),
                text_on_accent: WHITE,

                // Accent
                accent_primary: acc_p,
                accent_hover: acc_h,
                accent_pressed: acc_press,
                accent_subtle: acc_sub,
                focus_ring: Color::rgba(acc_p.r, acc_p.g, acc_p.b, 160),

                // Semantic
                success: Color::rgb(48, 209, 88),
                warning: Color::rgb(255, 214, 10),
                danger: Color::rgb(255, 69, 58),
                info: Color::rgb(10, 132, 255),
            },

            ThemeMode::Light => Self {
                // Windows 11 Fluent Light / macOS Sequoia Light
                app_background: Color::rgb(243, 243, 243),
                canvas_surface: Color::rgb(255, 255, 255),
                card_background: Color::rgb(255, 255, 255),
                card_hover: Color::rgb(246, 246, 248),
                card_pressed: Color::rgb(238, 238, 240),
                popup_background: Color::rgb(255, 255, 255),

                // Acrylic / Mica Glassmorphic Tokens
                acrylic_tint: Color::rgba(255, 255, 255, 185), // Translucent frosted white
                acrylic_blur_radius: 20,
                mica_tint: Color::rgba(243, 243, 243, 230),
                mica_blur_radius: 36,
                glass_border: Color::rgba(0, 0, 0, 16),

                // Delicate Borders
                border_subtle: Color::rgba(0, 0, 0, 14),
                border_strong: Color::rgba(0, 0, 0, 36),
                divider: Color::rgba(0, 0, 0, 10),

                // Typography
                text_primary: Color::rgb(28, 28, 30),          // macOS SF Black
                text_secondary: Color::rgb(72, 72, 74),
                text_muted: Color::rgb(142, 142, 147),
                text_on_accent: WHITE,

                // Accent
                accent_primary: acc_p,
                accent_hover: acc_h,
                accent_pressed: acc_press,
                accent_subtle: acc_sub,
                focus_ring: Color::rgba(acc_p.r, acc_p.g, acc_p.b, 140),

                // Semantic
                success: Color::rgb(52, 199, 89),
                warning: Color::rgb(255, 149, 0),
                danger: Color::rgb(255, 59, 48),
                info: Color::rgb(0, 122, 255),
            },
        }
    }
}

// ============================================================================
// 5. Accessibility & Contrast Calculation (WCAG 2.1)
// ============================================================================

/// Computes standard sRGB relative luminance: L = 0.2126*R + 0.7152*G + 0.0722*B
pub fn relative_luminance(c: Color) -> f32 {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Calculates WCAG contrast ratio between two colors: (L1 + 0.05) / (L2 + 0.05)
pub fn contrast_ratio(c1: Color, c2: Color) -> f32 {
    let l1 = relative_luminance(c1);
    let l2 = relative_luminance(c2);
    let lighter = l1.max(l2);
    let darker = l1.min(l2);
    (lighter + 0.05) / (darker + 0.05)
}

/// Checks if text on background passes WCAG 2.1 AA requirement (minimum 4.5:1 ratio).
pub fn meets_wcag_aa(text: Color, background: Color) -> bool {
    contrast_ratio(text, background) >= 4.5
}

// ============================================================================
// 6. Material Shaders (Acrylic & Mica Composition)
// ============================================================================

/// Renders a full Fluent/macOS frosted acrylic material panel into a buffer surface.
pub fn render_acrylic_panel(
    surface: &mut BufferSurface,
    rect: Rect,
    radius: i32,
    tokens: &ColorTokens,
) {
    // 1. Two-pass Gaussian blur over background
    surface.apply_gaussian_blur(rect, tokens.acrylic_blur_radius);

    // 2. Translucent tinted acrylic scrim
    surface.fill_rounded_rect(rect, radius, tokens.acrylic_tint);

    // 3. Delicate 1px glass border highlight
    surface.stroke_rounded_rect(rect, radius, 1, tokens.glass_border);
}

/// Renders an elevated card with ambient & directional drop shadows, squircle corners, and border.
pub fn render_elevated_card(
    surface: &mut BufferSurface,
    rect: Rect,
    radius: i32,
    elevation_level: usize,
    tiers: &[ElevationTier; 6],
    tokens: &ColorTokens,
) {
    let tier = &tiers[elevation_level.min(5)];

    // 1. Ambient Shadow (soft omnidirectional glow)
    if tier.ambient.blur > 0 {
        surface.draw_box_shadow(
            rect,
            tier.ambient.offset_x,
            tier.ambient.offset_y,
            tier.ambient.blur,
            tier.ambient.spread,
            tier.ambient.color,
        );
    }

    // 2. Directional Key Light Shadow (vertical depth offset)
    if tier.directional.blur > 0 {
        surface.draw_box_shadow(
            rect,
            tier.directional.offset_x,
            tier.directional.offset_y,
            tier.directional.blur,
            tier.directional.spread,
            tier.directional.color,
        );
    }

    // 3. Rounded Card Surface
    surface.fill_rounded_rect(rect, radius, tokens.card_background);

    // 4. Delicate Border
    surface.stroke_rounded_rect(rect, radius, 1, tokens.border_subtle);
}

// ============================================================================
// 7. Theme Manager (Live Dark/Light & Accent Toggles)
// ============================================================================

pub struct ThemeManager {
    pub mode: ThemeMode,
    pub accent: AccentPalette,
    pub tokens: ColorTokens,
    pub elevation: [ElevationTier; 6],
    pub corners: CornerTokens,
    subscribers: Vec<NodeId>,
}

impl ThemeManager {
    pub fn new(mode: ThemeMode, accent: AccentPalette) -> Self {
        Self {
            mode,
            accent,
            tokens: ColorTokens::build(mode, accent),
            elevation: get_elevation_tiers(mode),
            corners: CornerTokens::default(),
            subscribers: Vec::new(),
        }
    }

    pub fn subscribe(&mut self, node: NodeId) {
        if !self.subscribers.contains(&node) {
            self.subscribers.push(node);
        }
    }

    pub fn unsubscribe(&mut self, node: NodeId) {
        self.subscribers.retain(|&n| n != node);
    }

    /// Toggles between Dark and Light mode, updating tokens and notifying subscribers.
    pub fn toggle_theme(&mut self, scene: &mut SceneGraph) {
        let new_mode = match self.mode {
            ThemeMode::Dark => ThemeMode::Light,
            ThemeMode::Light => ThemeMode::Dark,
        };
        self.set_mode(new_mode, scene);
    }

    /// Sets the theme mode explicitly.
    pub fn set_mode(&mut self, mode: ThemeMode, scene: &mut SceneGraph) {
        self.mode = mode;
        self.tokens = ColorTokens::build(self.mode, self.accent);
        self.elevation = get_elevation_tiers(self.mode);
        self.notify_subscribers(scene);
    }

    /// Changes the active accent color across all components.
    pub fn set_accent(&mut self, accent: AccentPalette, scene: &mut SceneGraph) {
        self.accent = accent;
        self.tokens = ColorTokens::build(self.mode, self.accent);
        self.notify_subscribers(scene);
    }

    fn notify_subscribers(&self, scene: &mut SceneGraph) {
        for &sub in &self.subscribers {
            scene.mark_dirty(sub);
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theme_tokens_generation() {
        let dark_tokens = ColorTokens::build(ThemeMode::Dark, AccentPalette::FluentBlue);
        assert_eq!(dark_tokens.app_background, Color::rgb(26, 26, 26));
        assert_eq!(dark_tokens.text_on_accent, WHITE);

        let light_tokens = ColorTokens::build(ThemeMode::Light, AccentPalette::SequoiaBlue);
        assert_eq!(light_tokens.app_background, Color::rgb(243, 243, 243));
        assert_eq!(light_tokens.accent_primary, Color::rgb(0, 122, 255));
    }

    #[test]
    fn test_wcag_aa_contrast() {
        let dark = ColorTokens::build(ThemeMode::Dark, AccentPalette::FluentBlue);
        let light = ColorTokens::build(ThemeMode::Light, AccentPalette::FluentBlue);

        // Dark theme: primary text on card background
        let dark_ratio = contrast_ratio(dark.text_primary, dark.card_background);
        assert!(dark_ratio >= 4.5, "Dark theme contrast ratio {} satisfies WCAG AA (>= 4.5)", dark_ratio);

        // Light theme: primary text on card background
        let light_ratio = contrast_ratio(light.text_primary, light.card_background);
        assert!(light_ratio >= 4.5, "Light theme contrast ratio {} satisfies WCAG AA (>= 4.5)", light_ratio);

        // Text on accent
        let text_on_accent_ratio = contrast_ratio(dark.text_on_accent, dark.accent_primary);
        assert!(text_on_accent_ratio >= 2.0, "Accent contrast ratio {} satisfies legible standard", text_on_accent_ratio);
    }

    #[test]
    fn test_elevation_tiers_shadows() {
        let dark_elev = get_elevation_tiers(ThemeMode::Dark);
        assert_eq!(dark_elev.len(), 6);
        assert_eq!(dark_elev[0].ambient.blur, 0); // Level 0 flat
        assert!(dark_elev[1].directional.blur > 0); // Level 1 card
        assert!(dark_elev[5].directional.blur > dark_elev[2].directional.blur); // Level 5 modal > Level 2 button
    }

    #[test]
    fn test_corner_radii_tokens() {
        let corners = CornerTokens::default();
        assert_eq!(corners.small, 4);
        assert_eq!(corners.medium, 8);
        assert_eq!(corners.card, 10);
        assert_eq!(corners.large, 12);
        assert_eq!(corners.window, 16);
        assert_eq!(corners.modal, 20);
        assert_eq!(corners.pill, 9999);
    }

    #[test]
    fn test_live_theme_toggle() {
        let mut sg = SceneGraph::new();
        let card_node = sg.create_node("card");
        sg.add_child(sg.root, card_node);

        let mut theme = ThemeManager::new(ThemeMode::Dark, AccentPalette::FluentBlue);
        theme.subscribe(card_node);
        assert_eq!(theme.mode, ThemeMode::Dark);

        // Toggle to light mode
        theme.toggle_theme(&mut sg);
        assert_eq!(theme.mode, ThemeMode::Light);
        assert_eq!(theme.tokens.app_background, Color::rgb(243, 243, 243));
        assert!(sg.get_node(card_node).unwrap().dirty_paint, "Subscribed node received dirty notification on toggle");

        // Change accent to Sequoia Purple
        theme.set_accent(AccentPalette::Purple, &mut sg);
        assert_eq!(theme.accent, AccentPalette::Purple);
    }

    #[test]
    fn test_acrylic_and_elevated_card_rendering() {
        let mut surface = BufferSurface::new(120, 120);
        surface.clear(Color::rgb(20, 20, 20));

        let tokens = ColorTokens::build(ThemeMode::Dark, AccentPalette::FluentBlue);
        let tiers = get_elevation_tiers(ThemeMode::Dark);

        // Render acrylic panel
        render_acrylic_panel(&mut surface, Rect::new(10, 10, 100, 40), 8, &tokens);
        // Render elevated card at level 2
        render_elevated_card(&mut surface, Rect::new(10, 60, 100, 50), 10, 2, &tiers, &tokens);

        // Surface pixels should be populated
        assert_ne!(surface.get_pixel(30, 25), Color::rgb(20, 20, 20));
        assert_ne!(surface.get_pixel(30, 80), Color::rgb(20, 20, 20));
    }
}
