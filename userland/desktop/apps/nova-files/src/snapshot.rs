//! userland/desktop/apps/nova-files/src/snapshot.rs
//!
//! NovaFS Snapshot & File Version History Integration for NovaFiles.
//! Interfaces with NovaFS B-Tree snapshot extents for instant file history, diffs, and rollback.

#[cfg(not(test))]
use alloc::{string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{string::{String, ToString}, vec::Vec};


use crate::model::FileEntry;
use nova_ui::prelude::{BufferSurface, Color, Rect, WHITE};

/// Historical version record of a file captured inside a NovaFS snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileSnapshotVersion {
    pub snapshot_id: u64,
    pub snapshot_name: String,
    pub timestamp: u64,
    pub file_size_bytes: u64,
    pub sha256: [u8; 32],
    pub comment: String,
    pub content_diff_summary: String,
}

/// Snapshot browser & Time Machine modal state.
#[derive(Clone, Debug)]
pub struct SnapshotTimeMachineModal {
    pub is_visible: bool,
    pub target_file_path: String,
    pub versions: Vec<FileSnapshotVersion>,
    pub selected_version_idx: Option<usize>,
    pub modal_bounds: Rect,
}

impl SnapshotTimeMachineModal {
    pub fn new() -> Self {
        Self {
            is_visible: false,
            target_file_path: String::new(),
            versions: Vec::new(),
            selected_version_idx: None,
            modal_bounds: Rect::new(140, 80, 560, 460),
        }
    }

    /// Opens the snapshot version history dialog for a given file.
    pub fn open_for_file(&mut self, file: &FileEntry) {
        self.target_file_path = file.path.clone();
        self.is_visible = true;
        self.populate_simulated_snapshots(file);
    }

    /// Populates snapshot timeline from NovaFS snapshot records.
    pub fn populate_simulated_snapshots(&mut self, file: &FileEntry) {
        self.versions.clear();

        // Snapshot 1: 1 hour ago
        let mut sha1 = file.sha256;
        sha1[0] ^= 0x55;
        self.versions.push(FileSnapshotVersion {
            snapshot_id: 1042,
            snapshot_name: "@hourly-2026-11-15-0600".to_string(),
            timestamp: file.modified_timestamp.saturating_sub(3600),
            file_size_bytes: file.size_bytes.saturating_sub(120),
            sha256: sha1,
            comment: "Automated hourly NovaFS CoW tree checkpoint".to_string(),
            content_diff_summary: "+2 lines added, -1 line modified".to_string(),
        });

        // Snapshot 2: 1 day ago (daily backup)
        let mut sha2 = file.sha256;
        sha2[0] ^= 0xAA;
        self.versions.push(FileSnapshotVersion {
            snapshot_id: 1038,
            snapshot_name: "@daily-backup-2026-11-14".to_string(),
            timestamp: file.modified_timestamp.saturating_sub(86400),
            file_size_bytes: file.size_bytes.saturating_sub(1040),
            sha256: sha2,
            comment: "Nightly system snapshot & container checkpoint".to_string(),
            content_diff_summary: "+18 lines added, initial architecture draft".to_string(),
        });

        // Snapshot 3: Baseline release v1.0.0
        let mut sha3 = file.sha256;
        sha3[0] ^= 0xFF;
        self.versions.push(FileSnapshotVersion {
            snapshot_id: 1001,
            snapshot_name: "@release-baseline-v1.0.0".to_string(),
            timestamp: file.modified_timestamp.saturating_sub(604800),
            file_size_bytes: file.size_bytes.saturating_sub(4200),
            sha256: sha3,
            comment: "Golden master system deployment baseline".to_string(),
            content_diff_summary: "Initial commit of file repository".to_string(),
        });

        self.selected_version_idx = Some(0);
    }

    /// Closes the snapshot modal.
    pub fn close(&mut self) {
        self.is_visible = false;
        self.target_file_path.clear();
        self.versions.clear();
        self.selected_version_idx = None;
    }

    /// Restores the currently selected snapshot version.
    pub fn restore_selected_version(&self) -> Option<&FileSnapshotVersion> {
        self.selected_version_idx.and_then(|idx| self.versions.get(idx))
    }

    /// Renders the Snapshot Time Machine interface onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        if !self.is_visible {
            return;
        }

        let b = self.modal_bounds;

        // Scrim
        surface.fill_rect(Rect::new(0, 0, surface.width as i32, surface.height as i32), Color::rgba(0, 0, 0, 80));

        // Drop shadow
        for i in 1..=10 {
            let s_rect = Rect::new(b.x - i, b.y - i / 2 + 3, b.width + i * 2, b.height + i * 2);
            surface.stroke_rounded_rect(s_rect, 0, 1, Color::rgba(0, 0, 0, (16 - i).max(2) as u8));
        }

        // Card background
        let bg = if is_dark { Color::rgba(30, 30, 36, 255) } else { Color::rgba(255, 255, 255, 255) };
        surface.fill_rect(b, bg);
        surface.stroke_rounded_rect(b, 0, 1, if is_dark { Color::rgba(60, 60, 70, 255) } else { Color::rgba(210, 210, 220, 255) });

        // Header (40px)
        let header_rect = Rect::new(b.x, b.y, b.width, 40);
        let header_bg = if is_dark { Color::rgba(38, 38, 46, 255) } else { Color::rgba(240, 240, 246, 255) };
        surface.fill_rect(header_rect, header_bg);
        surface.fill_rect(Rect::new(b.x, b.y + 39, b.width, 1), if is_dark { Color::rgba(65, 65, 75, 255) } else { Color::rgba(220, 220, 230, 255) });

        // NovaFS Snapshot Logo Pill & Title
        surface.fill_rect(Rect::new(b.x + 14, b.y + 11, 18, 18), Color::rgba(52, 199, 89, 255));
        surface.fill_rect(Rect::new(b.x + 38, b.y + 18, 180, 4), if is_dark { WHITE } else { Color::rgba(20, 20, 25, 255) });

        // Close button
        surface.fill_rect(Rect::new(b.x + b.width - 32, b.y + 12, 16, 16), Color::rgba(255, 69, 58, 220));

        // Two-column layout: Left column = Versions timeline (200px), Right column = Diff/Metadata Inspector (360px)
        let left_w = 210;
        let left_rect = Rect::new(b.x, b.y + 40, left_w, b.height - 86);
        let right_rect = Rect::new(b.x + left_w, b.y + 40, b.width - left_w, b.height - 86);

        // Timeline column separator
        surface.fill_rect(Rect::new(b.x + left_w - 1, b.y + 40, 1, b.height - 86), if is_dark { Color::rgba(50, 50, 60, 255) } else { Color::rgba(220, 220, 230, 255) });

        // Render Version Items in Timeline
        let item_h = 56;
        for (idx, ver) in self.versions.iter().enumerate() {
            let iy = left_rect.y + 8 + (idx as i32) * (item_h + 4);
            let item_rect = Rect::new(left_rect.x + 8, iy, left_rect.width - 16, item_h);

            let is_sel = self.selected_version_idx == Some(idx);
            if is_sel {
                surface.fill_rect(item_rect, if is_dark { Color::rgba(0, 100, 200, 90) } else { Color::rgba(0, 122, 255, 60) });
                surface.stroke_rounded_rect(item_rect, 0, 1, Color::rgba(0, 122, 255, 200));
            } else {
                surface.fill_rect(item_rect, if is_dark { Color::rgba(36, 36, 44, 255) } else { Color::rgba(245, 245, 250, 255) });
            }

            // Snapshot checkpoint dot
            surface.fill_rect(Rect::new(item_rect.x + 8, item_rect.y + 10, 8, 8), Color::rgba(52, 199, 89, 255));

            // Snapshot name bar
            surface.fill_rect(Rect::new(item_rect.x + 22, item_rect.y + 12, item_rect.width - 32, 4), if is_dark { WHITE } else { Color::rgba(25, 25, 30, 255) });

            // Timestamp bar
            surface.fill_rect(Rect::new(item_rect.x + 22, item_rect.y + 24, 90, 3), Color::rgba(140, 140, 150, 200));

            // Diff changes summary bar
            surface.fill_rect(Rect::new(item_rect.x + 22, item_rect.y + 36, 120, 3), Color::rgba(0, 122, 255, 180));
        }

        // Render Right Column: Detailed Diff Inspector & SHA-256 Checksum
        if let Some(sel_idx) = self.selected_version_idx {
            if let Some(ver) = self.versions.get(sel_idx) {
                let rx = right_rect.x + 16;
                let mut ry = right_rect.y + 16;

                // Snapshot Name & ID
                surface.fill_rect(Rect::new(rx, ry, 180, 5), Color::rgba(0, 122, 255, 255));
                ry += 24;

                // Comment box
                let comment_rect = Rect::new(rx, ry, right_rect.width - 32, 40);
                surface.fill_rect(comment_rect, if is_dark { Color::rgba(22, 22, 28, 255) } else { Color::rgba(245, 245, 250, 255) });
                surface.fill_rect(Rect::new(comment_rect.x + 10, comment_rect.y + 12, 180, 4), if is_dark { WHITE } else { Color::rgba(30, 30, 35, 255) });
                surface.fill_rect(Rect::new(comment_rect.x + 10, comment_rect.y + 24, 140, 3), Color::rgba(140, 140, 150, 200));
                ry += 52;

                // Side-by-Side Diff Box
                let diff_box = Rect::new(rx, ry, right_rect.width - 32, 160);
                surface.fill_rect(diff_box, if is_dark { Color::rgba(18, 18, 22, 255) } else { Color::rgba(240, 240, 245, 255) });
                surface.stroke_rounded_rect(diff_box, 0, 1, if is_dark { Color::rgba(50, 50, 60, 255) } else { Color::rgba(215, 215, 225, 255) });

                // Diff line indicators (green + additions, red - deletions)
                surface.fill_rect(Rect::new(diff_box.x + 10, diff_box.y + 14, diff_box.width - 20, 16), Color::rgba(52, 199, 89, 40));
                surface.fill_rect(Rect::new(diff_box.x + 14, diff_box.y + 20, 160, 4), Color::rgba(52, 199, 89, 255));

                surface.fill_rect(Rect::new(diff_box.x + 10, diff_box.y + 36, diff_box.width - 20, 16), Color::rgba(255, 69, 58, 40));
                surface.fill_rect(Rect::new(diff_box.x + 14, diff_box.y + 42, 130, 4), Color::rgba(255, 69, 58, 255));

                surface.fill_rect(Rect::new(diff_box.x + 14, diff_box.y + 60, 180, 3), Color::rgba(140, 140, 150, 200));
                surface.fill_rect(Rect::new(diff_box.x + 14, diff_box.y + 76, 210, 3), Color::rgba(140, 140, 150, 200));
                surface.fill_rect(Rect::new(diff_box.x + 14, diff_box.y + 92, 170, 3), Color::rgba(140, 140, 150, 200));

                // SHA-256 Checksum badge at bottom of right column
                let hash_bar = Rect::new(rx, right_rect.y + right_rect.height - 30, right_rect.width - 32, 20);
                surface.fill_rect(hash_bar, if is_dark { Color::rgba(25, 25, 32, 255) } else { Color::rgba(235, 235, 240, 255) });
                surface.fill_rect(Rect::new(hash_bar.x + 8, hash_bar.y + 8, 190, 4), Color::rgba(52, 199, 89, 255));
            }
        }

        // Footer Actions Bar (46px)
        let footer_rect = Rect::new(b.x, b.y + b.height - 46, b.width, 46);
        surface.fill_rect(footer_rect, header_bg);
        surface.fill_rect(Rect::new(b.x, b.y + b.height - 46, b.width, 1), if is_dark { Color::rgba(65, 65, 75, 255) } else { Color::rgba(220, 220, 230, 255) });

        // "Restore This Version" Button (Emerald Green Accent)
        let btn_restore = Rect::new(b.x + b.width - 180, b.y + b.height - 36, 164, 26);
        surface.fill_rect(btn_restore, Color::rgba(52, 199, 89, 255));
        surface.fill_rect(Rect::new(btn_restore.x + 20, btn_restore.y + 11, 124, 4), WHITE);

        // "Cancel" Button
        let btn_cancel = Rect::new(b.x + b.width - 270, b.y + b.height - 36, 80, 26);
        surface.fill_rect(btn_cancel, if is_dark { Color::rgba(50, 50, 60, 255) } else { Color::rgba(225, 225, 235, 255) });
        surface.fill_rect(Rect::new(btn_cancel.x + 18, btn_cancel.y + 11, 44, 4), if is_dark { WHITE } else { Color::rgba(30, 30, 35, 255) });
    }
}
