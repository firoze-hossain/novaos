//! userland/desktop/apps/nova-installer/src/preview.rs
//!
//! Interactive Desktop Preview & OOBE Personalization for NovaOS Graphical Installer.
//! Synthesizes Windows 11 Fluent (Mica & Snap Layouts) + macOS (Menu Bar & Centered Dock).
//! Allows the user to select theme mode, accent color, and scaling factor with a real-time preview.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeMode {
    DarkMica,
    LightCupertino,
    AutoSolar,
}

impl ThemeMode {
    pub fn label(&self) -> &'static str {
        match self {
            ThemeMode::DarkMica => "Dark Acrylic / Mica (Windows 11 Fluent)",
            ThemeMode::LightCupertino => "Light Minimal (macOS Cupertino)",
            ThemeMode::AutoSolar => "Dynamic Daylight (Automatic)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccentColor {
    pub name: &'static str,
    pub rgb: u32,
}

pub const ACCENT_NOVA_BLUE: AccentColor = AccentColor {
    name: "Nova Blue",
    rgb: 0x0078D4,
};
pub const ACCENT_OBSIDIAN_PURPLE: AccentColor = AccentColor {
    name: "Obsidian Purple",
    rgb: 0x7B1FA2,
};
pub const ACCENT_EMERALD_GREEN: AccentColor = AccentColor {
    name: "Emerald Green",
    rgb: 0x00897B,
};
pub const ACCENT_SUNSET_CORAL: AccentColor = AccentColor {
    name: "Sunset Coral",
    rgb: 0xFF6B6B,
};
pub const ACCENT_CYBERPUNK_NEON: AccentColor = AccentColor {
    name: "Cyberpunk Cyan",
    rgb: 0x00E5FF,
};

pub const AVAILABLE_ACCENTS: [AccentColor; 5] = [
    ACCENT_NOVA_BLUE,
    ACCENT_OBSIDIAN_PURPLE,
    ACCENT_EMERALD_GREEN,
    ACCENT_SUNSET_CORAL,
    ACCENT_CYBERPUNK_NEON,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockPosition {
    BottomCentered,
    Left,
    Right,
}

#[derive(Debug, Clone)]
pub struct DesktopThemeConfig {
    pub theme_mode: ThemeMode,
    pub accent: AccentColor,
    pub dock_position: DockPosition,
    pub scale_factor_pct: u32, // 100, 125, 150, 200
    pub enable_snap_layouts: bool,
    pub enable_acrylic_blur: bool,
}

impl DesktopThemeConfig {
    pub fn new() -> Self {
        Self {
            theme_mode: ThemeMode::DarkMica,
            accent: ACCENT_NOVA_BLUE,
            dock_position: DockPosition::BottomCentered,
            scale_factor_pct: 100,
            enable_snap_layouts: true,
            enable_acrylic_blur: true,
        }
    }

    /// Generates JSON configuration for desktop personalization
    pub fn generate_desktop_config(&self) -> String {
        let theme_str = match self.theme_mode {
            ThemeMode::DarkMica => "dark_mica",
            ThemeMode::LightCupertino => "light_cupertino",
            ThemeMode::AutoSolar => "auto_solar",
        };
        let dock_str = match self.dock_position {
            DockPosition::BottomCentered => "bottom_center",
            DockPosition::Left => "left",
            DockPosition::Right => "right",
        };
        let accent_hex = format!("#{:06X}", self.accent.rgb);

        format!(
            r#"{{
  "version": 2,
  "theme": "{theme}",
  "accent_color": "{accent}",
  "accent_name": "{accent_name}",
  "dock": {{
    "position": "{dock}",
    "magnification": true,
    "autohide": false
  }},
  "display": {{
    "scale_percent": {scale},
    "snap_layouts": {snap},
    "acrylic_blur": {blur}
  }}
}}
"#,
            theme = theme_str,
            accent = accent_hex,
            accent_name = self.accent.name,
            dock = dock_str,
            scale = self.scale_factor_pct,
            snap = self.enable_snap_layouts,
            blur = self.enable_acrylic_blur,
        )
    }
}

/// Simulated Desktop Preview Canvas model (for rasterization into thumbnail or live preview)
pub struct DesktopPreviewCanvas {
    pub width: u32,
    pub height: u32,
}

impl DesktopPreviewCanvas {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// Renders the preview canvas pixels into a 32-bit ARGB/XRGB buffer
    pub fn render_preview(&self, config: &DesktopThemeConfig, buf: &mut [u32]) {
        if buf.len() < (self.width * self.height) as usize {
            return;
        }

        let is_dark = config.theme_mode != ThemeMode::LightCupertino;
        let bg_top = if is_dark { 0x1E1E2E } else { 0xE6ECF5 };
        let bg_bottom = if is_dark { 0x0F0F17 } else { 0xC8D6E5 };
        let menubar_bg = if is_dark { 0x181825 } else { 0xF5F7FA };
        let _text_color = if is_dark { 0xCDD6F4 } else { 0x1E1E2E };
        let dock_bg = if is_dark { 0x313244 } else { 0xFFFFFF };
        let accent = config.accent.rgb;

        // 1. Render gradient wallpaper
        for y in 0..self.height {
            let t = y as f32 / self.height as f32;
            let r = (((bg_top >> 16) & 0xFF) as f32 * (1.0 - t) + ((bg_bottom >> 16) & 0xFF) as f32 * t) as u32;
            let g = (((bg_top >> 8) & 0xFF) as f32 * (1.0 - t) + ((bg_bottom >> 8) & 0xFF) as f32 * t) as u32;
            let b = ((bg_top & 0xFF) as f32 * (1.0 - t) + (bg_bottom & 0xFF) as f32 * t) as u32;
            let row_color = 0xFF000000 | (r << 16) | (g << 8) | b;

            for x in 0..self.width {
                buf[(y * self.width + x) as usize] = row_color;
            }
        }

        // 2. Render Top macOS Menu Bar (height: ~24px scaled)
        let menubar_height = (24.0 * (config.scale_factor_pct as f32 / 100.0)).min(32.0) as u32;
        for y in 0..menubar_height.min(self.height) {
            for x in 0..self.width {
                buf[(y * self.width + x) as usize] = 0xFF000000 | menubar_bg;
            }
        }

        // Accent indicator bar under menu bar
        let bar_y = menubar_height.min(self.height - 1);
        for x in 0..self.width {
            buf[(bar_y * self.width + x) as usize] = 0xFF000000 | (accent & 0x7F7F7F);
        }

        // 3. Render Windows 11 Snap Layout preview card (top-right area)
        if config.enable_snap_layouts && self.width > 200 && self.height > 150 {
            let snap_w = 60u32;
            let snap_h = 40u32;
            let snap_x = self.width.saturating_sub(snap_w + 20);
            let snap_y = menubar_height + 15;

            for y in snap_y..(snap_y + snap_h).min(self.height) {
                for x in snap_x..(snap_x + snap_w).min(self.width) {
                    buf[(y * self.width + x) as usize] = 0xFF000000 | (dock_bg & 0xEEEEEE);
                }
            }
            // Left split highlight in snap card
            let half_w = snap_w / 2;
            for y in (snap_y + 2)..(snap_y + snap_h - 2).min(self.height) {
                for x in (snap_x + 2)..(snap_x + half_w - 1).min(self.width) {
                    buf[(y * self.width + x) as usize] = 0xFF000000 | accent;
                }
            }
        }

        // 4. Render Centered Fluent Dock at bottom
        let dock_w = (self.width * 55 / 100).clamp(140, 480);
        let dock_h = 36u32;
        let dock_x = (self.width.saturating_sub(dock_w)) / 2;
        let dock_y = self.height.saturating_sub(dock_h + 10);

        for y in dock_y..(dock_y + dock_h).min(self.height) {
            for x in dock_x..(dock_x + dock_w).min(self.width) {
                buf[(y * self.width + x) as usize] = 0xFF000000 | dock_bg;
            }
        }

        // Dock icons (6 simulated colorful app icons)
        let icon_count = 6;
        let icon_size = 20u32;
        let icon_gap = 12u32;
        let total_icons_w = icon_count * icon_size + (icon_count - 1) * icon_gap;
        let icons_start_x = dock_x + (dock_w.saturating_sub(total_icons_w)) / 2;
        let icons_y = dock_y + (dock_h.saturating_sub(icon_size)) / 2;

        let icon_colors = [
            0x0078D4, // Finder/Files (Blue)
            0x107C41, // Terminal (Green)
            0x8764B8, // Settings (Purple)
            0xF7630C, // Store (Orange)
            0x00B7C3, // Code / Edit (Teal)
            0xE81123, // System Monitor (Red)
        ];

        for (i, &color) in icon_colors.iter().enumerate() {
            let ix = icons_start_x + (i as u32) * (icon_size + icon_gap);
            for y in icons_y..(icons_y + icon_size).min(self.height) {
                for x in ix..(ix + icon_size).min(self.width) {
                    buf[(y * self.width + x) as usize] = 0xFF000000 | color;
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_desktop_theme_config_generation() {
        let mut cfg = DesktopThemeConfig::new();
        cfg.theme_mode = ThemeMode::DarkMica;
        cfg.accent = ACCENT_NOVA_BLUE;
        cfg.scale_factor_pct = 125;

        let json = cfg.generate_desktop_config();
        assert!(json.contains("\"theme\": \"dark_mica\""));
        assert!(json.contains("\"accent_color\": \"#0078D4\""));
        assert!(json.contains("\"scale_percent\": 125"));
    }

    #[test]
    fn test_preview_canvas_rendering() {
        let canvas = DesktopPreviewCanvas::new(200, 120);
        let mut buf = vec![0u32; (200 * 120) as usize];
        let cfg = DesktopThemeConfig::new();

        canvas.render_preview(&cfg, &mut buf);
        assert_ne!(buf[0], 0); // Menu bar rendered
        assert_ne!(buf[100 * 200 + 100], 0); // Center rendered
    }
}
