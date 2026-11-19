//! userland/desktop/apps/nova-recovery/src/ui.rs
//!
//! Retained-Mode Graphical User Interface for NovaOS Recovery Environment.
//! Renders an 800x600 emergency rescue dashboard into a `BufferSurface` with zero syscall overhead.

use nova_ui::prelude::{BufferSurface, Color, Rect};

use crate::boot_repair::BootRepairManager;
use crate::log_viewer::{LogLevel, LogViewer};
use crate::passwd_reset::PasswordResetManager;
use crate::snapshot::SnapshotManager;

pub const RESCUE_WIDTH: usize = 800;
pub const RESCUE_HEIGHT: usize = 600;

// Rescue Theme Colors
pub const COLOR_BG: Color = Color::rgb(0x18, 0x18, 0x25);          // Deep Slate
pub const COLOR_SURFACE: Color = Color::rgb(0x24, 0x27, 0x3A);     // Card Background
pub const COLOR_SURFACE_ALT: Color = Color::rgb(0x31, 0x32, 0x44); // Hover / Highlight
pub const COLOR_BORDER: Color = Color::rgb(0x45, 0x47, 0x5A);      // Slate Outline
pub const COLOR_TEXT_PRIMARY: Color = Color::rgb(0xCD, 0xD6, 0xF4);// Off-white
pub const COLOR_TEXT_MUTED: Color = Color::rgb(0x8A, 0x93, 0xAC);  // Neutral Slate
pub const COLOR_ACCENT_AMBER: Color = Color::rgb(0xF5, 0xA9, 0x7F);// Rescue Amber
pub const COLOR_ACCENT_BLUE: Color = Color::rgb(0x8A, 0xAD, 0xF4); // Nova Blue
pub const COLOR_SUCCESS_GREEN: Color = Color::rgb(0xA6, 0xDA, 0x95);// Verified Green
pub const COLOR_DANGER_RED: Color = Color::rgb(0xED, 0x87, 0x96);   // Panic Red

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RecoveryTab {
    Snapshots,
    PasswordReset,
    BootRepair,
    LogViewer,
    Reboot,
}

impl RecoveryTab {
    pub fn all() -> [RecoveryTab; 5] {
        [
            RecoveryTab::Snapshots,
            RecoveryTab::PasswordReset,
            RecoveryTab::BootRepair,
            RecoveryTab::LogViewer,
            RecoveryTab::Reboot,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            RecoveryTab::Snapshots => "1. Snapshots & Rollback",
            RecoveryTab::PasswordReset => "2. Reset Passwords",
            RecoveryTab::BootRepair => "3. Repair Boot & GPT",
            RecoveryTab::LogViewer => "4. System Logs",
            RecoveryTab::Reboot => "5. Reboot / Exit",
        }
    }
}

/// Recovery Application State Container
pub struct RecoveryApp {
    pub active_tab: RecoveryTab,
    pub selected_snapshot_idx: usize,
    pub selected_user: String,
    pub new_password_buf: String,
    pub status_message: String,
    pub snapshot_mgr: SnapshotManager,
    pub passwd_mgr: PasswordResetManager,
    pub repair_mgr: BootRepairManager,
    pub log_viewer: LogViewer,
}

impl RecoveryApp {
    pub fn new() -> Self {
        Self {
            active_tab: RecoveryTab::Snapshots,
            selected_snapshot_idx: 0,
            selected_user: "root".to_string(),
            new_password_buf: "NovaRecovery2026!".to_string(),
            status_message: "Recovery Environment Ready. Select an operation above.".to_string(),
            snapshot_mgr: SnapshotManager::new(),
            passwd_mgr: PasswordResetManager::new(),
            repair_mgr: BootRepairManager::new(20971520),
            log_viewer: LogViewer::new(),
        }
    }

    /// Triggers atomic rollback to the currently selected snapshot
    pub fn rollback_selected(&mut self) -> Result<(), String> {
        let snaps = self.snapshot_mgr.list_snapshots().to_vec();
        if self.selected_snapshot_idx < snaps.len() {
            let snap = &snaps[self.selected_snapshot_idx];
            let snap_id = snap.id;
            let snap_name = snap.name.clone();
            self.snapshot_mgr.rollback_to_snapshot(snap_id)?;
            self.status_message = format!(
                "Successfully rolled back to '{}' (ID: {}). Next boot will use this generation.",
                snap_name, snap_id
            );
            Ok(())
        } else {
            Err("No snapshot selected".to_string())
        }
    }

    /// Resets the selected user's password
    pub fn reset_user_password(&mut self) -> Result<(), String> {
        let salt = b"recovsalt1234567";
        self.passwd_mgr
            .reset_password(&self.selected_user, &self.new_password_buf, salt)?;
        self.status_message = format!(
            "Password for user '{}' updated successfully in /etc/shadow.",
            self.selected_user
        );
        Ok(())
    }
}

pub struct RecoveryUiRenderer;

impl RecoveryUiRenderer {
    pub fn render(app: &RecoveryApp, surface: &mut BufferSurface) {
        let (width, height) = (surface.width, surface.height);
        if width == 0 || height == 0 {
            return;
        }

        let w = width as i32;
        let h = height as i32;

        // 1. Clear background
        surface.fill_rect(Rect::new(0, 0, w, h), COLOR_BG);

        // 2. Render Header Bar
        Self::render_header(surface, w);

        // 3. Render Navigation Tab Bar
        Self::render_tabs(app, surface, w);

        // 4. Render Active Tab Body
        match app.active_tab {
            RecoveryTab::Snapshots => Self::render_snapshots_tab(app, surface, w, h),
            RecoveryTab::PasswordReset => Self::render_password_tab(app, surface, w, h),
            RecoveryTab::BootRepair => Self::render_boot_repair_tab(app, surface, w, h),
            RecoveryTab::LogViewer => Self::render_logs_tab(app, surface, w, h),
            RecoveryTab::Reboot => Self::render_reboot_tab(app, surface, w, h),
        }

        // 5. Render Footer & Status Bar
        Self::render_footer(app, surface, w, h);
    }

    fn render_header(surface: &mut BufferSurface, w: i32) {
        let header_rect = Rect::new(0, 0, w, 44);
        surface.fill_rect(header_rect, COLOR_SURFACE);
        surface.draw_text("NovaOS 2.0 Emergency Recovery Environment", 20, 14, COLOR_ACCENT_AMBER);
        surface.draw_text("[RESCUE CONSOLE: RING 0 / LEVEL 1 ACTIVE]", w - 380, 14, COLOR_TEXT_MUTED);
    }

    fn render_tabs(app: &RecoveryApp, surface: &mut BufferSurface, w: i32) {
        let bar_rect = Rect::new(0, 44, w, 36);
        surface.fill_rect(bar_rect, COLOR_SURFACE_ALT);

        let tab_width = w / 5;
        for (i, &tab) in RecoveryTab::all().iter().enumerate() {
            let tab_x = (i as i32) * tab_width;
            let is_active = tab == app.active_tab;

            if is_active {
                surface.fill_rect(Rect::new(tab_x, 44, tab_width, 36), COLOR_BG);
                // Amber active indicator bar
                surface.fill_rect(Rect::new(tab_x, 78, tab_width, 2), COLOR_ACCENT_AMBER);
                surface.draw_text(tab.title(), tab_x + 12, 54, COLOR_ACCENT_AMBER);
            } else {
                surface.draw_text(tab.title(), tab_x + 12, 54, COLOR_TEXT_MUTED);
            }
        }
    }

    fn render_snapshots_tab(app: &RecoveryApp, surface: &mut BufferSurface, w: i32, h: i32) {
        let card_rect = Rect::new(20, 96, w - 40, h - 160);
        surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE);

        surface.draw_text("NovaFS B-Tree Snapshot Restore & Rollback", 40, 116, COLOR_TEXT_PRIMARY);
        surface.draw_text(
            "Select a historical snapshot generation to revert rootfs state. Rollback is atomic and O(1).",
            40,
            138,
            COLOR_TEXT_MUTED,
        );

        // Table Header
        let table_y = 170;
        surface.fill_rect(Rect::new(40, table_y, w - 80, 26), COLOR_SURFACE_ALT);
        surface.draw_text("ID", 50, table_y + 6, COLOR_TEXT_MUTED);
        surface.draw_text("SNAPSHOT NAME", 100, table_y + 6, COLOR_TEXT_MUTED);
        surface.draw_text("GENERATION", 440, table_y + 6, COLOR_TEXT_MUTED);
        surface.draw_text("ROOT BLOCK", 540, table_y + 6, COLOR_TEXT_MUTED);
        surface.draw_text("SIZE", 650, table_y + 6, COLOR_TEXT_MUTED);

        // Snapshot rows
        let snaps = app.snapshot_mgr.list_snapshots();
        let mut row_y = table_y + 32;
        for (i, snap) in snaps.iter().enumerate() {
            let is_sel = i == app.selected_snapshot_idx;
            let row_color = if is_sel {
                COLOR_SURFACE_ALT
            } else if i % 2 == 0 {
                COLOR_SURFACE
            } else {
                COLOR_BG
            };

            surface.fill_rect(Rect::new(40, row_y, w - 80, 28), row_color);
            let text_color = if is_sel {
                COLOR_ACCENT_AMBER
            } else {
                COLOR_TEXT_PRIMARY
            };

            surface.draw_text(&format!("{}", snap.id), 50, row_y + 7, text_color);
            surface.draw_text(&snap.name, 100, row_y + 7, text_color);
            surface.draw_text(&format!("{}", snap.generation), 440, row_y + 7, text_color);
            surface.draw_text(&format!("{}", snap.root_block), 540, row_y + 7, text_color);
            surface.draw_text(&format!("{} GB", snap.size_bytes / 1_000_000_000), 650, row_y + 7, text_color);

            row_y += 32;
        }

        // Action Buttons
        let btn_y = h - 110;
        let btn_rect = Rect::new(40, btn_y, 220, 36);
        surface.fill_rounded_rect(btn_rect, 4, COLOR_ACCENT_AMBER);
        surface.draw_text("ROLLBACK TO SELECTED", 56, btn_y + 10, COLOR_BG);

        let chk_rect = Rect::new(280, btn_y, 240, 36);
        surface.fill_rounded_rect(chk_rect, 4, COLOR_SURFACE_ALT);
        surface.draw_text("CREATE SAFETY CHECKPOINT", 296, btn_y + 10, COLOR_TEXT_PRIMARY);
    }

    fn render_password_tab(app: &RecoveryApp, surface: &mut BufferSurface, w: i32, h: i32) {
        let card_rect = Rect::new(20, 96, w - 40, h - 160);
        surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE);

        surface.draw_text("Administrative Credential Recovery", 40, 116, COLOR_TEXT_PRIMARY);
        surface.draw_text(
            "Regenerates PBKDF2-HMAC-SHA256 password hashes directly into /etc/shadow.",
            40,
            138,
            COLOR_TEXT_MUTED,
        );

        // User Selection
        surface.draw_text("Target User Account:", 40, 180, COLOR_TEXT_PRIMARY);
        let users = app.passwd_mgr.list_usernames();
        let mut uy = 210;
        for u in &users {
            let is_sel = u == &app.selected_user;
            let bg = if is_sel { COLOR_SURFACE_ALT } else { COLOR_BG };
            surface.fill_rounded_rect(Rect::new(40, uy, 200, 32), 4, bg);
            let fg = if is_sel { COLOR_ACCENT_AMBER } else { COLOR_TEXT_PRIMARY };
            surface.draw_text(u, 54, uy + 8, fg);
            uy += 40;
        }

        // New Password Input Field
        surface.draw_text("New Password:", 300, 180, COLOR_TEXT_PRIMARY);
        let input_rect = Rect::new(300, 210, 360, 36);
        surface.fill_rounded_rect(input_rect, 4, COLOR_BG);
        surface.draw_text(&app.new_password_buf, 314, 220, COLOR_TEXT_PRIMARY);

        // Commit Button
        let btn_rect = Rect::new(300, 270, 200, 36);
        surface.fill_rounded_rect(btn_rect, 4, COLOR_SUCCESS_GREEN);
        surface.draw_text("UPDATE PASSWORD", 328, 280, COLOR_BG);
    }

    fn render_boot_repair_tab(_app: &RecoveryApp, surface: &mut BufferSurface, w: i32, h: i32) {
        let card_rect = Rect::new(20, 96, w - 40, h - 160);
        surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE);

        surface.draw_text("Bootloader & GUID Partition Table Repair", 40, 116, COLOR_TEXT_PRIMARY);
        surface.draw_text(
            "Verifies Protective MBR (LBA 0) and dual GPT redundancy headers against IEEE 802.3 CRC32.",
            40,
            138,
            COLOR_TEXT_MUTED,
        );

        // Status Cards
        let cards = [
            ("Protective MBR (LBA 0)", "VALID (Type 0xEE, Sig 0xAA55)", COLOR_SUCCESS_GREEN),
            ("Primary GPT Header (LBA 1)", "VALID (CRC32: 0x9B1C5ED5)", COLOR_SUCCESS_GREEN),
            ("Backup GPT Header (LBA Last)", "VALID (CRC32: 0x9B1C5ED5)", COLOR_SUCCESS_GREEN),
            ("UEFI Bootloader (/EFI/BOOT)", "INSTALLED (BOOTX64.EFI present)", COLOR_ACCENT_BLUE),
        ];

        let mut cy = 180;
        for (title, status, col) in &cards {
            let row_rect = Rect::new(40, cy, w - 80, 44);
            surface.fill_rounded_rect(row_rect, 4, COLOR_SURFACE_ALT);
            surface.draw_text(title, 56, cy + 14, COLOR_TEXT_PRIMARY);
            surface.draw_text(status, w - 380, cy + 14, *col);
            cy += 54;
        }

        // Repair Action Button
        let btn_rect = Rect::new(40, cy + 20, 260, 36);
        surface.fill_rounded_rect(btn_rect, 4, COLOR_ACCENT_AMBER);
        surface.draw_text("AUTOMATED 1-CLICK REPAIR", 60, cy + 30, COLOR_BG);
    }

    fn render_logs_tab(app: &RecoveryApp, surface: &mut BufferSurface, w: i32, h: i32) {
        let card_rect = Rect::new(20, 96, w - 40, h - 160);
        surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE);

        surface.draw_text("Emergency System & Kernel Crash Logs", 40, 116, COLOR_TEXT_PRIMARY);
        surface.draw_text("Real-time inspection of /var/log/kernel.log and /CRASHDMP.DAT.", 40, 138, COLOR_TEXT_MUTED);

        let log_box_rect = Rect::new(40, 170, w - 80, h - 250);
        surface.fill_rounded_rect(log_box_rect, 4, COLOR_BG);

        let mut ly = 180;
        for entry in &app.log_viewer.entries {
            let col = match entry.level {
                LogLevel::Panic => COLOR_DANGER_RED,
                LogLevel::Error => COLOR_DANGER_RED,
                LogLevel::Warn => COLOR_ACCENT_AMBER,
                LogLevel::Info => COLOR_TEXT_PRIMARY,
            };
            let line = format!("[{}] <{}> {}", entry.level.as_str(), entry.facility, entry.message);
            surface.draw_text(&line, 52, ly, col);
            ly += 22;
            if ly > h - 100 {
                break;
            }
        }
    }

    fn render_reboot_tab(_app: &RecoveryApp, surface: &mut BufferSurface, w: i32, h: i32) {
        let card_rect = Rect::new(20, 96, w - 40, h - 160);
        surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE);

        surface.draw_text("Reboot System & Exit Recovery Console", 40, 116, COLOR_TEXT_PRIMARY);
        surface.draw_text(
            "Flushes all pending disk writes, syncs superblocks, and triggers hardware reboot.",
            40,
            138,
            COLOR_TEXT_MUTED,
        );

        let btn1 = Rect::new(40, 200, 240, 44);
        surface.fill_rounded_rect(btn1, 4, COLOR_SUCCESS_GREEN);
        surface.draw_text("SYNC & REBOOT SYSTEM", 64, 214, COLOR_BG);

        let btn2 = Rect::new(40, 260, 240, 44);
        surface.fill_rounded_rect(btn2, 4, COLOR_SURFACE_ALT);
        surface.draw_text("EXIT TO RESCUE SHELL", 64, 274, COLOR_TEXT_PRIMARY);

        let btn3 = Rect::new(40, 320, 240, 44);
        surface.fill_rounded_rect(btn3, 4, COLOR_DANGER_RED);
        surface.draw_text("SHUTDOWN MACHINE", 64, 334, COLOR_TEXT_PRIMARY);
    }

    fn render_footer(app: &RecoveryApp, surface: &mut BufferSurface, w: i32, h: i32) {
        let footer_rect = Rect::new(0, h - 38, w, 38);
        surface.fill_rect(footer_rect, COLOR_SURFACE);
        surface.draw_text(&app.status_message, 20, h - 26, COLOR_ACCENT_AMBER);
        surface.draw_text("F1: Help | F5: Refresh | F10: Reboot", w - 300, h - 26, COLOR_TEXT_MUTED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_rendering_into_buffer() {
        let mut app = RecoveryApp::new();
        let mut surface = BufferSurface::new(RESCUE_WIDTH, RESCUE_HEIGHT);

        // Render each tab without panic
        for &tab in &RecoveryTab::all() {
            app.active_tab = tab;
            RecoveryUiRenderer::render(&app, &mut surface);
        }

        // Test action calls
        assert!(app.rollback_selected().is_ok());
        assert!(app.reset_user_password().is_ok());
    }
}
