//! userland/desktop/apps/nova-files/src/properties.rs
//!
//! Detailed Properties / Inspector Pane for NovaFiles.
//! Displays POSIX permissions, SHA-256 integrity checksum, size on disk, timestamps, and tag assignments.




use crate::model::FileEntry;
use crate::tags::FileTag;
use nova_ui::prelude::{BufferSurface, Color, Rect, WHITE};

/// Detailed properties inspector pane state.
pub struct PropertiesInspectorPane {
    pub is_visible: bool,
    pub bounds: Rect,
    pub entry: Option<FileEntry>,
}

impl PropertiesInspectorPane {
    pub fn new(bounds: Rect) -> Self {
        Self {
            is_visible: false,
            bounds,
            entry: None,
        }
    }

    pub fn set_entry(&mut self, entry: Option<FileEntry>) {
        self.entry = entry;
    }

    pub fn toggle(&mut self) {
        self.is_visible = !self.is_visible;
    }

    /// Renders properties inspector onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        if !self.is_visible {
            return;
        }

        let bg = if is_dark { Color::rgba(26, 26, 32, 255) } else { Color::rgba(245, 245, 248, 255) };
        let border = if is_dark { Color::rgba(50, 50, 60, 255) } else { Color::rgba(220, 220, 228, 255) };
        let text_color = if is_dark { Color::rgba(225, 225, 235, 255) } else { Color::rgba(25, 25, 30, 255) };
        let label_color = Color::rgba(140, 140, 150, 200);

        // Panel background & left border
        surface.fill_rect(self.bounds, bg);
        surface.fill_rect(Rect::new(self.bounds.x, self.bounds.y, 1, self.bounds.height), border);

        let entry = match &self.entry {
            Some(e) => e,
            None => {
                // Empty state message
                surface.fill_rect(Rect::new(self.bounds.x + 20, self.bounds.y + 40, self.bounds.width - 40, 4), label_color);
                return;
            }
        };

        let px = self.bounds.x + 14;
        let mut py = self.bounds.y + 16;
        let w = self.bounds.width - 28;

        // Big Icon (48x48)
        let icon_rect = Rect::new(px + (w - 48) / 2, py, 48, 48);
        surface.fill_rect(icon_rect, Color::rgba(0, 122, 255, 255));
        py += 56;

        // File Name
        surface.fill_rect(Rect::new(px + 10, py, w - 20, 5), text_color);
        py += 18;

        // Kind
        surface.fill_rect(Rect::new(px + 20, py, w - 40, 3), label_color);
        py += 24;

        // Separator
        surface.fill_rect(Rect::new(px, py, w, 1), border);
        py += 14;

        // Size & Size on Disk
        surface.fill_rect(Rect::new(px, py, 45, 3), label_color);
        surface.fill_rect(Rect::new(px + 55, py, w - 60, 4), text_color);
        py += 18;

        // Location / Path
        surface.fill_rect(Rect::new(px, py, 50, 3), label_color);
        surface.fill_rect(Rect::new(px + 60, py, w - 65, 3), text_color);
        py += 22;

        // Timestamps (Created, Modified)
        surface.fill_rect(Rect::new(px, py, 60, 3), label_color);
        surface.fill_rect(Rect::new(px + 70, py, w - 75, 3), text_color);
        py += 18;

        surface.fill_rect(Rect::new(px, py, 55, 3), label_color);
        surface.fill_rect(Rect::new(px + 65, py, w - 70, 3), text_color);
        py += 24;

        // Separator
        surface.fill_rect(Rect::new(px, py, w, 1), border);
        py += 14;

        // POSIX Permissions (Octal e.g. 0755 & -rwxr-xr-x)
        surface.fill_rect(Rect::new(px, py, 75, 3), label_color);
        let perm_box = Rect::new(px + w - 70, py - 4, 65, 18);
        surface.fill_rect(perm_box, if is_dark { Color::rgba(38, 38, 46, 255) } else { Color::rgba(235, 235, 240, 255) });
        surface.fill_rect(Rect::new(perm_box.x + 8, perm_box.y + 7, perm_box.width - 16, 4), Color::rgba(0, 122, 255, 255));
        py += 28;

        // NovaFS SHA-256 Block Integrity
        surface.fill_rect(Rect::new(px, py, 110, 3), label_color);
        py += 8;
        // Shield badge
        let shield_rect = Rect::new(px, py, w, 24);
        surface.fill_rect(shield_rect, Color::rgba(52, 199, 89, 35));
        surface.stroke_rounded_rect(shield_rect, 0, 1, Color::rgba(52, 199, 89, 180));
        surface.fill_rect(Rect::new(shield_rect.x + 8, shield_rect.y + 6, 12, 12), Color::rgba(52, 199, 89, 255));
        surface.fill_rect(Rect::new(shield_rect.x + 26, shield_rect.y + 10, w - 34, 3), Color::rgba(52, 199, 89, 255));
        py += 36;

        // Colored Tags Selector (7 circles)
        surface.fill_rect(Rect::new(px, py, 35, 3), label_color);
        py += 10;
        let tag_circle_size = 16;
        for (idx, tag) in FileTag::ALL_TAGS.iter().enumerate() {
            let cx = px + (idx as i32) * (tag_circle_size + 8);
            let circle = Rect::new(cx, py, tag_circle_size, tag_circle_size);
            surface.fill_rect(circle, tag.color());
            if entry.tag == *tag {
                surface.stroke_rounded_rect(Rect::new(circle.x - 2, circle.y - 2, circle.width + 4, circle.height + 4), 0, 1, WHITE);
            }
        }
    }
}
