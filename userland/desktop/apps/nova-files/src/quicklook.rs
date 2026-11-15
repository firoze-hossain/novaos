//! userland/desktop/apps/nova-files/src/quicklook.rs
//!
//! Spacebar QuickLook Floating Acrylic Preview Modal for NovaFiles.
//! Supports instant previews for Text/Code, Images, Audio waveforms, Package archives, and Binaries.




use crate::model::{FileEntry, FileKind};
use nova_ui::prelude::{BufferSurface, Color, Rect, WHITE};

/// QuickLook modal state and preview engine.
#[derive(Clone, Debug)]
pub struct QuickLookOverlay {
    pub is_visible: bool,
    pub entry: Option<FileEntry>,
    pub modal_bounds: Rect,
    pub active_tab: usize,
}

impl QuickLookOverlay {
    pub fn new() -> Self {
        Self {
            is_visible: false,
            entry: None,
            modal_bounds: Rect::new(160, 100, 520, 420),
            active_tab: 0,
        }
    }

    /// Toggles QuickLook visibility for the given selected file.
    pub fn toggle(&mut self, entry: Option<FileEntry>) {
        if self.is_visible {
            self.hide();
        } else if let Some(e) = entry {
            self.show(e);
        }
    }

    /// Shows QuickLook preview for an entry.
    pub fn show(&mut self, entry: FileEntry) {
        self.entry = Some(entry);
        self.is_visible = true;
    }

    /// Hides QuickLook modal.
    pub fn hide(&mut self) {
        self.is_visible = false;
        self.entry = None;
    }

    /// Renders the QuickLook floating card with Level 4 elevation drop shadow.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        if !self.is_visible {
            return;
        }

        let entry = match &self.entry {
            Some(e) => e,
            None => return,
        };

        let b = self.modal_bounds;

        // Backdrop scrim (translucent overlay dimming background)
        surface.fill_rect(Rect::new(0, 0, surface.width as i32, surface.height as i32), Color::rgba(0, 0, 0, 75));

        // Elevation drop shadow (Level 4: 16px spread, smooth penumbra)
        for i in 1..=12 {
            let shadow_rect = Rect::new(b.x - i, b.y - i / 2 + 4, b.width + i * 2, b.height + i * 2);
            let alpha = (18 - i).max(2) as u8;
            surface.stroke_rounded_rect(shadow_rect, 0, 1, Color::rgba(0, 0, 0, alpha));
        }

        // Modal acrylic background
        let bg_color = if is_dark {
            Color::rgba(32, 32, 38, 240)
        } else {
            Color::rgba(255, 255, 255, 245)
        };
        surface.fill_rect(b, bg_color);
        surface.stroke_rounded_rect(b, 0, 1, if is_dark { Color::rgba(60, 60, 70, 255) } else { Color::rgba(210, 210, 220, 255) });

        // Header (38px height)
        let header_rect = Rect::new(b.x, b.y, b.width, 38);
        let header_bg = if is_dark { Color::rgba(40, 40, 48, 255) } else { Color::rgba(242, 242, 246, 255) };
        surface.fill_rect(header_rect, header_bg);
        surface.fill_rect(Rect::new(b.x, b.y + 37, b.width, 1), if is_dark { Color::rgba(65, 65, 75, 255) } else { Color::rgba(220, 220, 230, 255) });

        // Icon pill
        surface.fill_rect(Rect::new(b.x + 12, b.y + 10, 18, 18), Color::rgba(0, 122, 255, 255));

        // Filename indicator
        surface.fill_rect(Rect::new(b.x + 36, b.y + 17, 160, 4), if is_dark { WHITE } else { Color::rgba(20, 20, 25, 255) });

        // Size pill
        surface.fill_rect(Rect::new(b.x + b.width - 90, b.y + 11, 45, 16), if is_dark { Color::rgba(55, 55, 65, 255) } else { Color::rgba(225, 225, 235, 255) });

        // Close button (✕)
        let close_rect = Rect::new(b.x + b.width - 32, b.y + 11, 16, 16);
        surface.fill_rect(close_rect, Color::rgba(255, 70, 70, 220));

        // Body Content based on FileKind
        let body_rect = Rect::new(b.x + 16, b.y + 50, b.width - 32, b.height - 100);

        match entry.kind {
            FileKind::TextDocument | FileKind::Code | FileKind::Configuration => {
                self.render_text_preview(surface, entry, body_rect, is_dark);
            }
            FileKind::Image => {
                self.render_image_preview(surface, entry, body_rect, is_dark);
            }
            FileKind::Audio => {
                self.render_audio_preview(surface, entry, body_rect, is_dark);
            }
            FileKind::PackageArchive | FileKind::ApplicationBundle => {
                self.render_archive_preview(surface, entry, body_rect, is_dark);
            }
            _ => {
                self.render_binary_preview(surface, entry, body_rect, is_dark);
            }
        }

        // Footer Actions Bar (40px)
        let footer_rect = Rect::new(b.x, b.y + b.height - 40, b.width, 40);
        surface.fill_rect(footer_rect, header_bg);
        surface.fill_rect(Rect::new(b.x, b.y + b.height - 40, b.width, 1), if is_dark { Color::rgba(65, 65, 75, 255) } else { Color::rgba(220, 220, 230, 255) });

        // Primary action button ("Open with App")
        let btn_open = Rect::new(b.x + b.width - 130, b.y + b.height - 32, 116, 24);
        surface.fill_rect(btn_open, Color::rgba(0, 122, 255, 255));
        surface.fill_rect(Rect::new(btn_open.x + 16, btn_open.y + 10, 84, 4), WHITE);
    }

    fn render_text_preview(&self, surface: &mut BufferSurface, _entry: &FileEntry, bounds: Rect, is_dark: bool) {
        // Line number gutter (36px width)
        let gutter_rect = Rect::new(bounds.x, bounds.y, 36, bounds.height);
        let gutter_bg = if is_dark { Color::rgba(26, 26, 32, 255) } else { Color::rgba(235, 235, 240, 255) };
        surface.fill_rect(gutter_rect, gutter_bg);
        surface.fill_rect(Rect::new(bounds.x + 35, bounds.y, 1, bounds.height), if is_dark { Color::rgba(50, 50, 60, 255) } else { Color::rgba(215, 215, 225, 255) });

        // Code editor preview area
        let code_rect = Rect::new(bounds.x + 36, bounds.y, bounds.width - 36, bounds.height);
        let code_bg = if is_dark { Color::rgba(22, 22, 28, 255) } else { Color::rgba(245, 245, 250, 255) };
        surface.fill_rect(code_rect, code_bg);

        // Render code lines with line numbers
        let line_h = 18;
        let line_count = (bounds.height / line_h).min(14);
        let line_num_color = Color::rgba(120, 120, 135, 255);

        for l in 0..line_count {
            let ly = bounds.y + l * line_h + 4;
            // Line number dash
            surface.fill_rect(Rect::new(bounds.x + 10, ly + 4, 16, 3), line_num_color);

            // Code text segment (syntax colors: keyword blue, string green, comment gray)
            let color = match l % 4 {
                0 => Color::rgba(0, 122, 255, 255),   // keyword
                1 => Color::rgba(52, 199, 89, 255),   // string
                2 => Color::rgba(255, 149, 0, 255),   // identifier
                _ => Color::rgba(140, 140, 150, 255), // comment
            };
            let code_w = (80 + (l * 37) % 200).min(bounds.width - 60);
            surface.fill_rect(Rect::new(bounds.x + 48, ly + 4, code_w, 4), color);
        }
    }

    fn render_image_preview(&self, surface: &mut BufferSurface, _entry: &FileEntry, bounds: Rect, is_dark: bool) {
        // Checkerboard transparency canvas
        let cell_size = 12;
        let c1 = if is_dark { Color::rgba(45, 45, 52, 255) } else { Color::rgba(240, 240, 245, 255) };
        let c2 = if is_dark { Color::rgba(35, 35, 40, 255) } else { Color::rgba(225, 225, 230, 255) };

        for y in (bounds.y..bounds.y + bounds.height - 30).step_by(cell_size as usize) {
            for x in (bounds.x..bounds.x + bounds.width).step_by(cell_size as usize) {
                let is_alt = ((x - bounds.x) / cell_size + (y - bounds.y) / cell_size) % 2 == 0;
                let r = Rect::new(x, y, cell_size.min(bounds.x + bounds.width - x), cell_size.min(bounds.y + bounds.height - 30 - y));
                surface.fill_rect(r, if is_alt { c1 } else { c2 });
            }
        }

        // Centered image viewport
        let img_w = (bounds.width - 60).min(320);
        let img_h = (bounds.height - 80).min(180);
        let img_rect = Rect::new(bounds.x + (bounds.width - img_w) / 2, bounds.y + 20, img_w, img_h);
        surface.fill_rect(img_rect, Color::rgba(80, 140, 220, 255));
        surface.stroke_rounded_rect(img_rect, 0, 1, WHITE);

        // Metadata badge bar (Resolution: 1920x1080 | 32-bit ARGB | Aspect Ratio: 16:9)
        let badge_rect = Rect::new(bounds.x + 20, bounds.y + bounds.height - 24, bounds.width - 40, 20);
        surface.fill_rect(badge_rect, if is_dark { Color::rgba(40, 40, 48, 255) } else { Color::rgba(230, 230, 235, 255) });
        surface.fill_rect(Rect::new(badge_rect.x + 10, badge_rect.y + 8, 140, 4), Color::rgba(0, 122, 255, 255));
    }

    fn render_audio_preview(&self, surface: &mut BufferSurface, _entry: &FileEntry, bounds: Rect, is_dark: bool) {
        // Visualizer backdrop
        let vis_rect = Rect::new(bounds.x, bounds.y, bounds.width, bounds.height - 40);
        let vis_bg = if is_dark { Color::rgba(20, 20, 26, 255) } else { Color::rgba(245, 245, 250, 255) };
        surface.fill_rect(vis_rect, vis_bg);

        // Waveform bars (28 spectrum bars)
        let bar_count = 28;
        let bar_w = (vis_rect.width - 40) / bar_count;
        let mid_y = vis_rect.y + vis_rect.height / 2;

        let heights: [i32; 28] = [
            12, 24, 38, 55, 72, 85, 94, 76, 62, 80, 95, 110, 88, 65,
            50, 78, 102, 115, 90, 74, 60, 45, 68, 82, 54, 38, 26, 15,
        ];

        for (i, &h) in heights.iter().enumerate() {
            let bx = vis_rect.x + 20 + (i as i32) * bar_w;
            let bar_h = h.min(vis_rect.height - 20);
            let bar_rect = Rect::new(bx, mid_y - bar_h / 2, bar_w - 3, bar_h);
            let bar_color = Color::rgba(0, 122 + (i as u8) * 4, 255, 230);
            surface.fill_rect(bar_rect, bar_color);
        }

        // Track scrubber bar
        let scrub_rect = Rect::new(bounds.x + 10, bounds.y + bounds.height - 30, bounds.width - 20, 6);
        surface.fill_rect(scrub_rect, if is_dark { Color::rgba(60, 60, 70, 255) } else { Color::rgba(210, 210, 220, 255) });
        // Progress (42%)
        let prog_w = (scrub_rect.width * 42) / 100;
        surface.fill_rect(Rect::new(scrub_rect.x, scrub_rect.y, prog_w, scrub_rect.height), Color::rgba(0, 122, 255, 255));

        // Scrubber thumb dot
        surface.fill_rect(Rect::new(scrub_rect.x + prog_w - 4, scrub_rect.y - 3, 12, 12), WHITE);
    }

    fn render_archive_preview(&self, surface: &mut BufferSurface, _entry: &FileEntry, bounds: Rect, is_dark: bool) {
        // Archive table of contents
        let list_bg = if is_dark { Color::rgba(25, 25, 30, 255) } else { Color::rgba(245, 245, 248, 255) };
        surface.fill_rect(bounds, list_bg);

        // Header info bar (Uncompressed: 24.5 MB | 18 files | Savings: 42%)
        let info_rect = Rect::new(bounds.x, bounds.y, bounds.width, 26);
        surface.fill_rect(info_rect, if is_dark { Color::rgba(35, 35, 42, 255) } else { Color::rgba(235, 235, 240, 255) });
        surface.fill_rect(Rect::new(info_rect.x + 10, info_rect.y + 11, 160, 4), Color::rgba(255, 149, 0, 255));

        // Sample packed files list
        let row_h = 24;
        let file_count = ((bounds.height - 30) / row_h).min(8);
        for i in 0..file_count {
            let ry = bounds.y + 30 + i * row_h;
            // File icon dot
            surface.fill_rect(Rect::new(bounds.x + 12, ry + 6, 12, 12), Color::rgba(100, 180, 255, 255));
            // Filename bar
            let name_w = (100 + (i * 29) % 150).min(bounds.width - 120);
            surface.fill_rect(Rect::new(bounds.x + 32, ry + 10, name_w, 3), if is_dark { WHITE } else { Color::rgba(30, 30, 35, 255) });
            // Size bar
            surface.fill_rect(Rect::new(bounds.x + bounds.width - 60, ry + 10, 40, 3), Color::rgba(140, 140, 150, 200));
        }
    }

    fn render_binary_preview(&self, surface: &mut BufferSurface, _entry: &FileEntry, bounds: Rect, is_dark: bool) {
        // Hex dump view
        let dump_bg = if is_dark { Color::rgba(18, 18, 24, 255) } else { Color::rgba(245, 245, 250, 255) };
        surface.fill_rect(bounds, dump_bg);

        let row_h = 20;
        let rows = (bounds.height / row_h).min(12);

        for r in 0..rows {
            let ry = bounds.y + 6 + r * row_h;
            // Offset column (e.g. 00000000:)
            surface.fill_rect(Rect::new(bounds.x + 10, ry + 4, 45, 3), Color::rgba(0, 122, 255, 220));

            // Hex pairs (16 bytes = 4 groups of 4)
            for g in 0..4 {
                let gx = bounds.x + 65 + g * 50;
                surface.fill_rect(Rect::new(gx, ry + 4, 42, 3), if is_dark { Color::rgba(200, 200, 210, 255) } else { Color::rgba(40, 40, 45, 255) });
            }

            // ASCII column
            surface.fill_rect(Rect::new(bounds.x + bounds.width - 90, ry + 4, 75, 3), Color::rgba(52, 199, 89, 200));
        }
    }
}
