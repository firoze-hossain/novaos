//! # NovaOS Unified Design System & Widget Framework
//!
//! Provides the foundational visual language, constants, color scales,
//! spacing grid, typography metrics, and drawing primitives across all NovaOS
//! applications, the window manager, and system control panels.

#![no_std]

pub mod color;
pub mod cursor;
pub mod dock;
pub mod draw;
pub mod font;
pub mod font_data;
pub mod geom;
pub mod menu;
pub mod spacing;
pub mod typography;
pub mod widget;

pub mod prelude {
    pub use crate::color::{Color, Theme, ThemePalette, BLACK, TRANSPARENT, WHITE};
    pub use crate::cursor::{
        draw_cursor_sprite, CursorColors, CursorShape, CursorSprite, CursorTheme,
    };
    pub use crate::dock::{
        app_for_id, draw_dock_app_icon, draw_dock_surface, measure_dock_item, DockApp, DockEvent,
        DockItem, APP_ID_EDITOR, APP_ID_FILES, APP_ID_SETTINGS, APP_ID_SYSTEM, APP_ID_TERMINAL,
        DEFAULT_PINNED_APPS, MAX_PINNED_APPS,
    };
    pub use crate::draw::{
        draw_button, draw_char, draw_checkbox, draw_line, draw_progress_bar, draw_rect_outline,
        draw_rounded_rect, draw_text, draw_text_aligned, draw_window_frame, BufferSurface32,
        BufferSurface8, Surface,
    };
    pub use crate::font::{
        draw_char_font, draw_text_font, draw_text_font_aligned, font_for_scale, AntialiasMode,
        Font, FontSize, FontStyle, GlyphMetrics,
    };
    pub use crate::geom::{Insets, Point, Rect, Size};
    pub use crate::menu::{
        draw_dropdown_surface, draw_menubar_surface, measure_dropdown_rect, measure_menubar_item,
        menubar_for_label, Menu, MenuBar, MenuClickEvent, MenuItem, ACTION_FILE_CLOSE,
        ACTION_FILE_NEW, ACTION_FILE_OPEN, ACTION_FILE_QUIT, ACTION_FILE_SAVE, EDITOR_MENUBAR,
        FILES_MENUBAR, SYSTEM_MENUBAR, TERMINAL_MENUBAR,
    };
    pub use crate::spacing::*;
    pub use crate::typography::{measure_text, TextAlign, TypeScale};
    pub use crate::widget::{
        ButtonVariant, CheckboxState, WidgetColorSet, WidgetState, WidgetThemeSpec,
        WindowControlKind,
    };
}

use prelude::*;

/// Comprehensive design system self-test verifying geometric invariants,
/// color math, typography bounds, and widget styling contracts.
pub fn run_design_system_selftest() -> i32 {
    let mut failures = 0;

    // 1. Color and Alpha Blending Invariants
    let c1 = Color::rgb(100, 150, 200);
    if c1.to_u32() != 0xFF6496C8 {
        failures |= 1;
    }
    let semi = Color::rgba(255, 0, 0, 128);
    let bg = Color::rgb(0, 0, 255);
    let blended = semi.blend_over(bg);
    if blended.r < 120 || blended.b < 120 {
        failures |= 2;
    }
    // VGA 16-color index round trip
    let black = BLACK;
    let white = WHITE;
    if black.to_vga_index() != 0 || white.to_vga_index() != 15 {
        failures |= 4;
    }

    // 2. Geometry & Rect Invariants
    let r1 = Rect::new(10, 20, 100, 50);
    if !r1.contains(15, 25) || r1.contains(5, 5) || r1.contains(110, 70) {
        failures |= 8;
    }
    let r2 = Rect::new(50, 40, 80, 60);
    if let Some(inter) = r1.intersection(&r2) {
        if inter.x != 50 || inter.y != 40 || inter.w != 60 || inter.h != 30 {
            failures |= 16;
        }
    } else {
        failures |= 16;
    }
    let padded = r1.pad(5);
    if padded.x != 15 || padded.y != 25 || padded.w != 90 || padded.h != 40 {
        failures |= 32;
    }

    // 3. Spacing Grid Invariants
    if SPACE_XS != 4 || SPACE_SM != 8 || SPACE_MD != 12 || SPACE_LG != 16 {
        failures |= 64;
    }

    // 4. Typography Scale & Measurement
    let caption_size = measure_text("NovaOS", TypeScale::Caption);
    let title_size = measure_text("NovaOS", TypeScale::Title);
    if title_size.w <= caption_size.w || title_size.h <= caption_size.h {
        failures |= 128;
    }

    // 5. Widget Color Spec Resolution
    let theme = Theme::Dark;
    let palette = theme.palette();
    let btn_normal = WidgetThemeSpec::button_colors(ButtonVariant::Primary, WidgetState::Normal, &palette);
    let btn_hover = WidgetThemeSpec::button_colors(ButtonVariant::Primary, WidgetState::Hover, &palette);
    if btn_normal.bg == btn_hover.bg {
        failures |= 256;
    }

    // 6. Surface & Clipping Invariant
    let mut pixels = [0u32; 64]; // 8x8 buffer
    {
        let mut surface = BufferSurface32::new(&mut pixels, 8, 8);
        surface.fill_rect(Rect::new(-5, -5, 10, 10), WHITE);
        // Pixels (0,0) to (4,4) must be white (0x00FFFFFF or 0xFFFFFFFF)
        if surface.buffer[0] != 0x00FFFFFF {
            failures |= 512;
        }
    }

    // 7. Multi-size Font & Antialiasing Invariants (Month 2 Compositor)
    let f_micro = Font::new(FontSize::Micro, FontStyle::Sans, AntialiasMode::None);
    let f_small = Font::small();
    let f_reg = Font::regular();
    let f_large = Font::large();
    let s_micro = f_micro.measure_str("NovaOS");
    let s_small = f_small.measure_str("NovaOS");
    let s_reg = f_reg.measure_str("NovaOS");
    let s_large = f_large.measure_str("NovaOS");
    if s_large.w <= s_reg.w || s_reg.w <= s_micro.w || s_large.h <= s_reg.h || s_reg.h <= s_small.h || s_small.h <= s_micro.h {
        failures |= 1024;
    }

    // Verify antialiased rendering produces intermediate fractional alpha coverage
    let mut font_pixels = [0u32; 16 * 16];
    {
        let mut surface = BufferSurface32::new(&mut font_pixels, 16, 16);
        draw_char_font(&mut surface, 0, 0, b'A', Color::WHITE, &f_reg);
        let has_intermediate = font_pixels.iter().any(|&p| p != 0 && p != 0x00FFFFFF);
        if !has_intermediate {
            failures |= 2048;
        }
    }

    // 8. Cursor Themes and Shapes Invariants (Month 2 Compositor)
    let c_classic = CursorTheme::Classic.colors();
    let c_dark = CursorTheme::ModernDark.colors();
    let c_light = CursorTheme::ModernLight.colors();
    let c_amber = CursorTheme::Amber.colors();
    if c_classic.outline == c_dark.outline || c_classic.fill == c_dark.fill || c_light.accent == c_amber.accent {
        failures |= 4096;
    }

    let arrow = CursorShape::Arrow.sprite();
    let pointer = CursorShape::Pointer.sprite();
    let move_sp = CursorShape::Move.sprite();
    if arrow.width != 16 || arrow.height != 16 || arrow.hotspot_x != 0 || arrow.hotspot_y != 0 {
        failures |= 8192;
    }
    if pointer.hotspot_x != 4 || pointer.hotspot_y != 0 || move_sp.hotspot_x != 7 || move_sp.hotspot_y != 7 {
        failures |= 8192;
    }

    let mut cursor_pixels = [0u32; 16 * 16];
    {
        let mut surface = BufferSurface32::new(&mut cursor_pixels, 16, 16);
        draw_cursor_sprite(&mut surface, 0, 0, CursorShape::Arrow, CursorTheme::Classic);
        let has_outline = cursor_pixels[0] == 0x00000000;
        let has_fill = cursor_pixels[2 * 16 + 1] == 0x00FFFFFF;
        let has_transparent = cursor_pixels[15] == 0;
        if !has_outline || !has_fill || !has_transparent {
            failures |= 16384;
        }
    }

    // 9. Desktop Menu Bar Invariants (Month 3 Core Desktop)
    let term_bar = TERMINAL_MENUBAR;
    if term_bar.menus.is_empty() || term_bar.menus[0].items.is_empty() {
        failures |= 32768;
    }
    let quit_item = term_bar.menus[0].items.iter().find(|it| it.action_id == ACTION_FILE_QUIT);
    if quit_item.is_none() {
        failures |= 32768;
    }

    // 10. Dock & Taskbar Invariants (Month 3 Core Desktop)
    let pinned = DEFAULT_PINNED_APPS;
    if pinned.len() < 5 || !pinned[0].is_pinned || pinned[0].app_id != APP_ID_TERMINAL {
        failures |= 65536;
    }
    if app_for_id(APP_ID_SETTINGS).is_none() {
        failures |= 65536;
    }

    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selftest_passes() {
        assert_eq!(run_design_system_selftest(), 0);
    }

    #[test]
    fn test_color_conversions() {
        let c = Color::from_hex(0x007ACC);
        assert_eq!(c.r, 0x00);
        assert_eq!(c.g, 0x7A);
        assert_eq!(c.b, 0xCC);
        assert_eq!(c.a, 255);
        assert_eq!(c.to_xrgb(), 0x007ACC);

        let c_dark = c.darken(20);
        assert!(c_dark.g < c.g);
        assert!(c_dark.b < c.b);

        let c_light = c.lighten(20);
        assert!(c_light.r >= c.r);
        assert!(c_light.g > c.g);
        assert!(c_light.b >= c.b);

        let translucent = Color::rgba(255, 0, 0, 128);
        let blended = translucent.blend_over(Color::rgb(0, 0, 255));
        assert!(blended.r > 100);
        assert!(blended.b > 100);
    }

    #[test]
    fn test_vga_palette() {
        assert_eq!(BLACK.to_vga_index(), 0);
        assert_eq!(WHITE.to_vga_index(), 15);
        let red = Color::rgb(255, 0, 0);
        assert_eq!(red.to_vga_index(), 4);
        let blue = Color::rgb(0, 0, 255);
        assert_eq!(blue.to_vga_index(), 1);
    }

    #[test]
    fn test_geom_and_insets() {
        let r = Rect::new(10, 10, 100, 80);
        assert!(r.contains(10, 10));
        assert!(r.contains(109, 89));
        assert!(!r.contains(110, 90));
        assert!(!r.contains(9, 10));

        let center = r.center();
        assert_eq!(center.x, 60);
        assert_eq!(center.y, 50);

        let child = Rect::new(0, 0, 40, 20);
        let centered = child.center_in(&r);
        assert_eq!(centered.x, 40);
        assert_eq!(centered.y, 40);

        let insets = Insets::symmetric(4, 8);
        let inset_r = r.inset(insets);
        assert_eq!(inset_r.x, 18);
        assert_eq!(inset_r.y, 14);
        assert_eq!(inset_r.w, 84);
        assert_eq!(inset_r.h, 72);
    }

    #[test]
    fn test_typography_metrics() {
        let s = measure_text("NovaOS UI", TypeScale::Body);
        assert!(s.w > 0);
        assert_eq!(s.h, 12);

        let multiline = measure_text("Line 1\nLine 2", TypeScale::Body);
        assert_eq!(multiline.h, 24);

        let title = measure_text("NovaOS UI", TypeScale::Title);
        assert_eq!(title.h, 18);
        assert!(title.w > s.w);

        // Check glyphs exist
        for b in 32..=126 {
            let g = typography::get_glyph(b);
            assert_eq!(g.len(), 7);
        }
    }

    #[test]
    fn test_widget_specs_and_themes() {
        for theme in [Theme::Dark, Theme::Light, Theme::HighContrast] {
            let pal = theme.palette();
            assert_ne!(pal.desktop_bg, pal.text_primary);
            assert_ne!(pal.titlebar_active, pal.titlebar_text_active);

            for variant in [
                ButtonVariant::Primary,
                ButtonVariant::Secondary,
                ButtonVariant::Danger,
                ButtonVariant::Ghost,
            ] {
                let norm = WidgetThemeSpec::button_colors(variant, WidgetState::Normal, &pal);
                let hov = WidgetThemeSpec::button_colors(variant, WidgetState::Hover, &pal);
                let press = WidgetThemeSpec::button_colors(variant, WidgetState::Pressed, &pal);
                let dis = WidgetThemeSpec::button_colors(variant, WidgetState::Disabled, &pal);

                assert_ne!(norm.bg, hov.bg);
                assert_ne!(hov.bg, press.bg);
                assert_eq!(dis.text, pal.text_muted);
            }
        }
    }

    #[test]
    fn test_drawing_and_clipping() {
        let mut buf = [0u32; 100 * 100];
        let mut surface = BufferSurface32::new(&mut buf, 100, 100);

        // Draw clipped lines and rects
        draw_line(&mut surface, -20, -20, 150, 150, Color::WHITE);
        draw_rect_outline(&mut surface, Rect::new(-10, -10, 50, 50), 2, Color::rgb(255, 0, 0));
        draw_rounded_rect(&mut surface, Rect::new(20, 20, 60, 40), 6, Color::rgb(0, 255, 0));
        draw_text(&mut surface, 25, 25, "OK", Color::BLACK, TypeScale::Body);

        let pal = Theme::Dark.palette();
        draw_button(&mut surface, Rect::new(30, 70, 40, 20), "Go", ButtonVariant::Primary, WidgetState::Normal, &pal);
        draw_checkbox(&mut surface, Rect::new(10, 10, 80, 20), CheckboxState::Checked, WidgetState::Normal, "Check", &pal);
        draw_progress_bar(&mut surface, Rect::new(10, 95, 80, 4), 0.75, &pal);

        // Verify that buffer received pixels
        let mut non_zero = 0;
        for &p in surface.buffer.iter() {
            if p != 0 {
                non_zero += 1;
            }
        }
        assert!(non_zero > 100);
    }

    #[test]
    fn test_font_sizes_and_metrics() {
        let small = Font::small();
        let regular = Font::regular();
        let large = Font::large();

        assert_eq!(small.line_height(), 10);
        assert_eq!(regular.line_height(), 16);
        assert_eq!(large.line_height(), 24);

        // Proportional spacing differences
        let w_i = regular.measure_char(b'i');
        let w_m = regular.measure_char(b'm');
        let w_w = regular.measure_char(b'w');
        let w_dot = regular.measure_char(b'.');
        assert!(w_i < w_m);
        assert!(w_dot < w_w);

        // Monospace fixed pitch
        let mono = Font::mono();
        assert_eq!(mono.measure_char(b'i'), mono.measure_char(b'W'));
        assert_eq!(mono.measure_char(b'.'), mono.measure_char(b'M'));

        // Multiline string measurement
        let text = "Hello\nWorld";
        let sz = regular.measure_str(text);
        assert_eq!(sz.h, regular.line_height() * 2);
        assert!(sz.w > 0);
    }

    #[test]
    fn test_font_antialiased_rendering() {
        let mut buf = [0u32; 32 * 32];
        let mut surface = BufferSurface32::new(&mut buf, 32, 32);

        let font_aa = Font::large(); // 12x24 antialiased
        draw_char_font(&mut surface, 4, 4, b'A', Color::WHITE, &font_aa);

        let mut has_solid = false;
        let mut has_intermediate = false;
        for &p in surface.buffer.iter() {
            if p == 0x00FFFFFF {
                has_solid = true;
            } else if p != 0 {
                has_intermediate = true;
            }
        }

        assert!(has_solid, "Glyph core should have solid white pixels");
        assert!(has_intermediate, "Antialiased edges should produce blended fractional pixels");
    }

    #[test]
    fn test_vector_stroke_rasterizer() {
        let font_vec = Font::vector(28);
        assert_eq!(font_vec.line_height(), (28 * 12) / 10);

        let mut buf = [0u32; 32 * 32];
        let mut surface = BufferSurface32::new(&mut buf, 32, 32);

        // Draw scalable vector glyph '0'
        draw_char_font(&mut surface, 2, 2, b'0', Color::WHITE, &font_vec);

        let non_zero_count = surface.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_count > 20, "Vector glyph should rasterize pixels into surface");

        // Intermediate coverage present from analytical distance falloff
        let aa_pixels = surface.buffer.iter().filter(|&&p| p != 0 && p != 0x00FFFFFF).count();
        assert!(aa_pixels > 0, "Vector rasterizer should produce antialiased edges");
    }

    #[test]
    fn test_cursor_sprite_rendering() {
        let mut buf = [0u32; 16 * 16];
        let mut surface = BufferSurface32::new(&mut buf, 16, 16);
        draw_cursor_sprite(&mut surface, 0, 0, CursorShape::Arrow, CursorTheme::Classic);

        // Hotspot pixel (0, 0) should be outline (black in Classic theme)
        assert_eq!(surface.buffer[0], 0x00000000, "Hotspot pixel should be black outline");

        // Pixel (1, 2) in Arrow sprite is '.' (fill = white)
        assert_eq!(surface.buffer[2 * 16 + 1], 0x00FFFFFF, "Interior should be white fill");

        // Transparent pixel (e.g. at (15, 0)) should remain untouched (0)
        assert_eq!(surface.buffer[15], 0, "Outside pixels should be transparent");
    }

    #[test]
    fn test_cursor_shapes_and_themes() {
        let shapes = [
            CursorShape::Arrow,
            CursorShape::Pointer,
            CursorShape::IBeam,
            CursorShape::Move,
            CursorShape::ResizeNS,
            CursorShape::ResizeEW,
            CursorShape::ResizeNWSE,
            CursorShape::ResizeNESW,
            CursorShape::Wait,
            CursorShape::Crosshair,
        ];

        for shape in &shapes {
            let sp = shape.sprite();
            assert!(sp.width > 0 && sp.height > 0);
            assert!(sp.hotspot_x >= 0 && sp.hotspot_x < sp.width);
            assert!(sp.hotspot_y >= 0 && sp.hotspot_y < sp.height);
            assert_eq!(sp.rows.len(), sp.height as usize);
        }

        let c_classic = CursorTheme::Classic.colors();
        let c_dark = CursorTheme::ModernDark.colors();
        let c_light = CursorTheme::ModernLight.colors();
        let c_amber = CursorTheme::Amber.colors();

        assert_ne!(c_classic.outline, c_dark.outline);
        assert_ne!(c_light.accent, c_amber.accent);
    }

    #[test]
    fn test_menubar_structures() {
        let bar = menubar_for_label(1);
        assert_eq!(bar.app_name, "Terminal");
        assert_eq!(bar.menus.len(), 4);
        assert_eq!(bar.menus[0].title, "File");

        // Verify Quit item exists and has ACTION_FILE_QUIT
        let quit_item = bar.menus[0].items.iter().find(|i| i.action_id == ACTION_FILE_QUIT);
        assert!(quit_item.is_some());
        assert_eq!(quit_item.unwrap().shortcut, "Ctrl+Q");
    }

    #[test]
    fn test_menubar_surface_rendering() {
        let bar = TERMINAL_MENUBAR;
        let mut buf = [0u32; 800 * 24];
        let mut surface = BufferSurface32::new(&mut buf, 800, 24);
        let palette = Theme::Dark.palette();

        draw_menubar_surface(
            &mut surface,
            Rect::new(0, 0, 800, 24),
            &bar,
            Some(0), // "File" active
            None,
            "18:18",
            &palette,
        );

        // Verify surface has non-zero pixels
        let non_zero = surface.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 100);

        // Verify bottom divider line is rendered
        let divider_p = surface.buffer[23 * 800 + 400];
        assert_eq!(divider_p, palette.border_subtle.to_xrgb());
    }

    #[test]
    fn test_dropdown_surface_rendering() {
        let menu = &TERMINAL_MENUBAR.menus[0]; // File
        let font = Font::regular();
        let bounds = measure_dropdown_rect(10, 24, menu, 800, &font);

        assert!(bounds.w >= 140);
        assert!(bounds.h > 60);

        let mut buf = [0u32; 250 * 150];
        let mut surface = BufferSurface32::new(&mut buf, 250, 150);
        let palette = Theme::Dark.palette();

        draw_dropdown_surface(&mut surface, bounds, menu, Some(0), &palette);

        // Verify dropdown background and highlighted item pixels exist
        let non_zero = surface.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 500);
    }

    #[test]
    fn test_dock_structures_and_defaults() {
        let apps = DEFAULT_PINNED_APPS;
        assert_eq!(apps.len(), 5);
        assert_eq!(apps[0].name, "Terminal");
        assert_eq!(apps[0].exec_path, "SHELL.ELF");
        assert!(apps[0].is_pinned);

        assert_eq!(apps[4].name, "Settings");
        assert_eq!(apps[4].exec_path, "SETTINGS.ELF");

        let app = app_for_id(APP_ID_FILES).expect("files app must exist");
        assert_eq!(app.name, "Files");
    }

    #[test]
    fn test_dock_item_states_and_measurement() {
        let app = DEFAULT_PINNED_APPS[0];
        let item_running = DockItem::new(app, true, true, false, Some(0));
        assert!(item_running.is_running);
        assert!(item_running.is_active);
        assert!(!item_running.is_minimized);
        assert_eq!(item_running.window_id, Some(0));

        let item_minimized = DockItem::new(app, true, false, true, Some(0));
        assert!(item_minimized.is_minimized);

        let item_unlaunched = DockItem::new(app, false, false, false, None);
        assert!(!item_unlaunched.is_running);

        let taskbar = Rect::new(0, 732, 1024, 36);
        let rect = measure_dock_item(0, 5, taskbar, 48, 120);
        assert!(rect.is_some());
        let r = rect.unwrap();
        assert_eq!(r.x, 48);
        assert_eq!(r.y, 734);
        assert!(r.w >= 48);
        assert_eq!(r.h, 32);
    }

    #[test]
    fn test_dock_surface_rendering() {
        let items = [
            DockItem::new(DEFAULT_PINNED_APPS[0], true, true, false, Some(0)),
            DockItem::new(DEFAULT_PINNED_APPS[1], true, false, false, Some(1)),
            DockItem::new(DEFAULT_PINNED_APPS[2], true, false, true, Some(2)),
            DockItem::new(DEFAULT_PINNED_APPS[3], false, false, false, None),
        ];

        let mut buf = [0u32; 1024 * 36];
        let mut surface = BufferSurface32::new(&mut buf, 1024, 36);
        let palette = Theme::Dark.palette();

        draw_dock_surface(
            &mut surface,
            Rect::new(0, 0, 1024, 36),
            &items,
            Some(1),
            "12:45 PM",
            &palette,
        );

        // Verify taskbar rendered non-zero pixels
        let non_zero = surface.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 1000);

        // Verify top border line
        assert_eq!(surface.buffer[500], palette.border_subtle.to_xrgb());

        // Verify active running indicator bar is rendered at bottom of item 0
        let item0_rect = measure_dock_item(0, 4, Rect::new(0, 0, 1024, 36), 48, 120).unwrap();
        let bar_pixel = surface.buffer[(item0_rect.bottom() - 1) as usize * 1024 + (item0_rect.x + 15) as usize];
        assert_eq!(bar_pixel, palette.accent_primary.to_xrgb());
    }
}
