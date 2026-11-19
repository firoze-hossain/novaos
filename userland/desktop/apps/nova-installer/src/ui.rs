//! userland/desktop/apps/nova-installer/src/ui.rs
//!
//! Retained-Mode Graphical User Interface Renderer for NovaOS Installer.
//! Synthesizes Windows 11 Fluent mica styling and macOS elegance.
//! Renders pixel-perfect buffers into `BufferSurface` with zero syscall overhead.

use nova_ui::prelude::{BufferSurface, Color, Rect};

use crate::wizard::{InstallerWizardApp, WizardStep};

pub const WINDOW_WIDTH: usize = 1024;
pub const WINDOW_HEIGHT: usize = 700;

// Color Tokens (RGB)
pub const COLOR_BG_DARK: Color = Color::rgb(0x18, 0x18, 0x25);       // Deep Charcoal Mica
pub const COLOR_CARD_DARK: Color = Color::rgb(0x24, 0x27, 0x3A);     // Elevated Surface
pub const COLOR_CARD_LIGHT: Color = Color::rgb(0x31, 0x32, 0x44);    // Card Hover
pub const COLOR_BORDER: Color = Color::rgb(0x45, 0x47, 0x5A);        // Subtle Border
pub const COLOR_TEXT_PRIMARY: Color = Color::rgb(0xCD, 0xD6, 0xF4);  // White/Light
pub const COLOR_TEXT_MUTED: Color = Color::rgb(0xA6, 0xAD, 0xC8);    // Cool Gray
pub const COLOR_ACCENT_BLUE: Color = Color::rgb(0x00, 0x78, 0xD4);   // Nova Blue
pub const COLOR_SUCCESS_GREEN: Color = Color::rgb(0x10, 0x7C, 0x41); // Success
pub const COLOR_DANGER_RED: Color = Color::rgb(0xD8, 0x3B, 0x01);    // Warning/Danger

fn draw_circle(surface: &mut BufferSurface, cx: i32, cy: i32, radius: i32, color: Color) {
    let r2 = radius * radius;
    for y in -radius..=radius {
        for x in -radius..=radius {
            if x * x + y * y <= r2 {
                surface.set_pixel(cx + x, cy + y, color);
            }
        }
    }
}

pub struct InstallerWindowRenderer;

impl InstallerWindowRenderer {
    /// Renders the complete installer application frame into the provided surface
    pub fn render(app: &InstallerWizardApp, surface: &mut BufferSurface) {
        let (width, height) = (surface.width, surface.height);
        if width == 0 || height == 0 {
            return;
        }

        let w_i32 = width as i32;
        let h_i32 = height as i32;

        // 1. Fill base dark mica background
        surface.fill_rect(Rect::new(0, 0, w_i32, h_i32), COLOR_BG_DARK);

        // 2. Render Window Title Bar (height: 38px)
        Self::render_title_bar(surface, w_i32);

        // 3. Render Top Stepper Header (height: 48px)
        Self::render_stepper_header(app, surface, w_i32);

        // 4. Render Main Content Body Card (x: 24, y: 94, w: width - 48, h: height - 170)
        let body_rect = Rect::new(24, 94, w_i32 - 48, h_i32 - 170);
        surface.fill_rounded_rect(body_rect, 10, COLOR_CARD_DARK);

        // Render Step-Specific UI inside Card
        match app.current_step {
            WizardStep::Welcome => Self::render_welcome_step(app, surface, &body_rect),
            WizardStep::PreFlightChecks => Self::render_preflight_step(app, surface, &body_rect),
            WizardStep::Partitioning => Self::render_partition_step(app, surface, &body_rect),
            WizardStep::Timezone => Self::render_timezone_step(app, surface, &body_rect),
            WizardStep::UserAccount => Self::render_account_step(app, surface, &body_rect),
            WizardStep::DesktopPreview => Self::render_preview_step(app, surface, &body_rect),
            WizardStep::SummaryConfirm => Self::render_summary_step(app, surface, &body_rect),
            WizardStep::Installing => Self::render_installing_step(app, surface, &body_rect),
            WizardStep::Complete => Self::render_complete_step(app, surface, &body_rect),
        }

        // 5. Render Bottom Action Bar (y: height - 64, h: 64)
        Self::render_action_bar(app, surface, w_i32, h_i32);

        // 6. Render Error Banner (if active)
        if let Some(ref err) = app.error_banner {
            Self::render_error_toast(surface, w_i32, err);
        }
    }

    fn render_title_bar(surface: &mut BufferSurface, width: i32) {
        let bar_rect = Rect::new(0, 0, width, 38);
        surface.fill_rect(bar_rect, Color::rgb(0x11, 0x11, 0x1B));

        // Window Controls (macOS style left dots: close red, minimize yellow, zoom green)
        draw_circle(surface, 18, 19, 6, Color::rgb(0xFF, 0x5F, 0x56));
        draw_circle(surface, 36, 19, 6, Color::rgb(0xFF, 0xBD, 0x2E));
        draw_circle(surface, 54, 19, 6, Color::rgb(0x27, 0xC9, 0x3F));

        // Subtitle divider line
        surface.fill_rect(Rect::new(0, 37, width, 1), COLOR_BORDER);
    }

    fn render_stepper_header(app: &InstallerWizardApp, surface: &mut BufferSurface, width: i32) {
        let total = WizardStep::total_steps() as i32;
        let step_w = (width - 48) / total;
        let y = 48;

        for i in 0..total {
            let x = 24 + (i * step_w);
            let is_past = i < app.current_step as i32;
            let is_current = i == app.current_step as i32;

            let color = if is_current {
                COLOR_ACCENT_BLUE
            } else if is_past {
                COLOR_SUCCESS_GREEN
            } else {
                COLOR_CARD_LIGHT
            };

            // Pill indicator
            surface.fill_rounded_rect(Rect::new(x + 4, y, step_w - 8, 6), 3, color);
        }
    }

    fn render_welcome_step(_app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        // Hero Icon
        draw_circle(surface, card.x + 80, card.y + 100, 40, COLOR_ACCENT_BLUE);

        // Architecture Badges
        let badge_y = card.y + 220;
        for i in 0..4i32 {
            let bx = card.x + 40 + (i * 220);
            surface.fill_rounded_rect(Rect::new(bx, badge_y, 200, 36), 6, COLOR_CARD_LIGHT);
        }
    }

    fn render_preflight_step(app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        let checks = [
            ("64-Bit x86_64 Long Mode Architecture", app.preflight.cpu_cores >= 1),
            ("Sufficient Physical Memory (>= 2048 MB RAM)", app.preflight.ram_mb >= 2048),
            ("Modern UEFI 2.9+ Boot Firmware Handover", app.preflight.is_uefi_64bit),
            ("High-Performance NVMe / VirtIO Storage Available", app.preflight.has_storage),
        ];

        let start_y = card.y + 40;
        for (i, (_desc, ok)) in checks.iter().enumerate() {
            let cy = start_y + (i as i32 * 65);
            let pill_color = if *ok { COLOR_SUCCESS_GREEN } else { COLOR_DANGER_RED };

            surface.fill_rounded_rect(Rect::new(card.x + 40, cy, card.width - 80, 50), 8, COLOR_CARD_LIGHT);
            draw_circle(surface, card.x + 70, cy + 25, 12, pill_color);
        }
    }

    fn render_partition_step(app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        // 1. Storage Bar Gauge (Multi-colored proportional visual gauge)
        let gauge_rect = Rect::new(card.x + 40, card.y + 40, card.width - 80, 32);
        surface.fill_rounded_rect(gauge_rect, 6, COLOR_CARD_LIGHT);

        let segments = app.partition_wizard.compute_visual_segments();
        for seg in &segments {
            let seg_x = gauge_rect.x + ((seg.start_pct / 100.0) * gauge_rect.width as f32) as i32;
            let seg_w = (((seg.width_pct / 100.0) * gauge_rect.width as f32) as i32).max(4);
            let seg_color = Color::rgb(
                ((seg.color_rgb >> 16) & 0xFF) as u8,
                ((seg.color_rgb >> 8) & 0xFF) as u8,
                (seg.color_rgb & 0xFF) as u8,
            );
            surface.fill_rect(Rect::new(seg_x, gauge_rect.y, seg_w, gauge_rect.height), seg_color);
        }

        // 2. Partition List Cards
        let list_y = card.y + 90;
        for (i, p) in app.partition_wizard.partitions.iter().enumerate().take(5) {
            let py = list_y + (i as i32 * 50);
            surface.fill_rounded_rect(Rect::new(card.x + 40, py, card.width - 80, 42), 6, COLOR_CARD_LIGHT);

            // Mount point badge
            let badge_color = match p.part_type {
                crate::partition_wizard::PartitionType::EfiSystem => COLOR_ACCENT_BLUE,
                crate::partition_wizard::PartitionType::RootFs => COLOR_SUCCESS_GREEN,
                _ => COLOR_TEXT_MUTED,
            };
            surface.fill_rounded_rect(Rect::new(card.x + 55, py + 8, 80, 26), 4, badge_color);
        }
    }

    fn render_timezone_step(app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        // Selected Timezone Banner Card
        let selected = app.timezone_db.selected();
        let banner_rect = Rect::new(card.x + 40, card.y + 40, card.width - 80, 80);
        surface.fill_rounded_rect(banner_rect, 8, COLOR_CARD_LIGHT);

        // Clock Preview Pill
        surface.fill_rounded_rect(Rect::new(banner_rect.x + banner_rect.width - 150, banner_rect.y + 20, 130, 40), 20, COLOR_ACCENT_BLUE);

        // Timezone list items (first 4)
        for (i, tz) in app.timezone_db.entries.iter().take(4).enumerate() {
            let ty = card.y + 140 + (i as i32 * 45);
            let is_sel = tz.id == selected.id;
            let bg = if is_sel { COLOR_CARD_LIGHT } else { Color::rgb(0x1E, 0x1E, 0x2E) };
            surface.fill_rounded_rect(Rect::new(card.x + 40, ty, card.width - 80, 38), 6, bg);
        }
    }

    fn render_account_step(app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        let start_y = card.y + 30;
        for i in 0..5i32 {
            let fy = start_y + (i * 55);
            surface.fill_rounded_rect(Rect::new(card.x + 40, fy, card.width - 80, 40), 6, COLOR_CARD_LIGHT);
        }

        // Password strength meter bar
        let strength = crate::account::UserAccountConfig::evaluate_password_strength(&app.user_password_input);
        let s_color = Color::rgb(
            ((strength.color_rgb() >> 16) & 0xFF) as u8,
            ((strength.color_rgb() >> 8) & 0xFF) as u8,
            (strength.color_rgb() & 0xFF) as u8,
        );
        let meter_w = match strength {
            crate::account::PasswordStrength::VeryWeak => 60,
            crate::account::PasswordStrength::Weak => 120,
            crate::account::PasswordStrength::Fair => 200,
            crate::account::PasswordStrength::Good => 300,
            crate::account::PasswordStrength::Strong => 420,
        };
        surface.fill_rounded_rect(Rect::new(card.x + 40, card.y + 310, meter_w, 6), 3, s_color);
    }

    fn render_preview_step(_app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        // Accent Color Dots
        for (i, acc) in crate::preview::AVAILABLE_ACCENTS.iter().enumerate() {
            let dot_x = card.x + 50 + (i as i32 * 60);
            let dot_y = card.y + 50;
            let col = Color::rgb(
                ((acc.rgb >> 16) & 0xFF) as u8,
                ((acc.rgb >> 8) & 0xFF) as u8,
                (acc.rgb & 0xFF) as u8,
            );
            draw_circle(surface, dot_x, dot_y, 16, col);
        }

        // Miniature Desktop Preview Card (480x260)
        let prev_w = (card.width - 80).min(560);
        let prev_rect = Rect::new(card.x + 40, card.y + 90, prev_w, 260);
        surface.fill_rounded_rect(prev_rect, 10, Color::rgb(0x0F, 0x0F, 0x17));
    }

    fn render_summary_step(app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        let blueprint_card = Rect::new(card.x + 40, card.y + 40, card.width - 80, 220);
        surface.fill_rounded_rect(blueprint_card, 8, COLOR_CARD_LIGHT);

        // Confirmation Authorization Checkbox
        let check_y = card.y + 280;
        let box_color = if app.confirm_format_authorized { COLOR_SUCCESS_GREEN } else { COLOR_TEXT_MUTED };
        surface.fill_rounded_rect(Rect::new(card.x + 40, check_y, 22, 22), 4, box_color);
    }

    fn render_installing_step(app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        // Progress Bar
        let pct = app.engine.current_stage.progress_pct();
        let bar_bg = Rect::new(card.x + 40, card.y + 40, card.width - 80, 18);
        surface.fill_rounded_rect(bar_bg, 9, COLOR_CARD_LIGHT);

        let fill_w = ((pct / 100.0) * bar_bg.width as f32) as i32;
        surface.fill_rounded_rect(Rect::new(bar_bg.x, bar_bg.y, fill_w, bar_bg.height), 9, COLOR_ACCENT_BLUE);

        // Animated Slide Showcase Card
        let slide_card = Rect::new(card.x + 40, card.y + 80, card.width - 80, 260);
        surface.fill_rounded_rect(slide_card, 10, COLOR_CARD_LIGHT);
    }

    fn render_complete_step(_app: &InstallerWizardApp, surface: &mut BufferSurface, card: &Rect) {
        // Giant Success Green Checkmark Circle
        let center_x = card.x + card.width / 2;
        let center_y = card.y + 120;
        draw_circle(surface, center_x, center_y, 50, COLOR_SUCCESS_GREEN);

        // Reboot CTA button
        let btn_rect = Rect::new(center_x - 120, card.y + 240, 240, 48);
        surface.fill_rounded_rect(btn_rect, 8, COLOR_ACCENT_BLUE);
    }

    fn render_action_bar(app: &InstallerWizardApp, surface: &mut BufferSurface, width: i32, height: i32) {
        let bar_y = height - 64;
        surface.fill_rect(Rect::new(0, bar_y, width, 64), Color::rgb(0x11, 0x11, 0x1B));

        // Back Button
        if app.current_step != WizardStep::Welcome
            && app.current_step != WizardStep::Installing
            && app.current_step != WizardStep::Complete
        {
            surface.fill_rounded_rect(Rect::new(24, bar_y + 14, 100, 36), 6, COLOR_CARD_LIGHT);
        }

        // Next / Install / Reboot Button
        let can_next = app.can_advance() || app.current_step == WizardStep::Complete;
        let next_color = if can_next { COLOR_ACCENT_BLUE } else { COLOR_CARD_LIGHT };
        let next_btn_w = if app.current_step == WizardStep::SummaryConfirm {
            140
        } else {
            110
        };

        surface.fill_rounded_rect(
            Rect::new(width - 24 - next_btn_w, bar_y + 14, next_btn_w, 36),
            6,
            next_color,
        );
    }

    fn render_error_toast(surface: &mut BufferSurface, width: i32, _err: &str) {
        let toast = Rect::new(24, 42, width - 48, 38);
        surface.fill_rounded_rect(toast, 6, COLOR_DANGER_RED);
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installer_window_render_all_steps() {
        let mut app = InstallerWizardApp::new();
        let mut surface = BufferSurface::new(1024, 700);

        // Step 0: Welcome
        InstallerWindowRenderer::render(&app, &mut surface);
        assert_ne!(surface.get_pixel(100, 100), COLOR_BG_DARK);

        // Step 1: Pre-flight
        app.next_step();
        InstallerWindowRenderer::render(&app, &mut surface);

        // Step 2: Partitioning
        app.next_step();
        InstallerWindowRenderer::render(&app, &mut surface);

        // Step 3: Timezone
        app.next_step();
        InstallerWindowRenderer::render(&app, &mut surface);

        // Step 4: User account
        app.next_step();
        app.user_password_input = "Pass123!".to_string();
        app.user_password_confirm = "Pass123!".to_string();
        InstallerWindowRenderer::render(&app, &mut surface);

        // Step 5: Desktop Preview
        app.next_step();
        InstallerWindowRenderer::render(&app, &mut surface);

        // Step 6: Summary
        app.next_step();
        app.confirm_format_authorized = true;
        InstallerWindowRenderer::render(&app, &mut surface);

        // Step 7/8: Installation & Complete
        app.next_step();
        InstallerWindowRenderer::render(&app, &mut surface);
        assert_eq!(app.current_step, WizardStep::Complete);
    }
}
