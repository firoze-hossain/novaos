//! userland/desktop/apps/nova-files/src/sidebar.rs
//!
//! Navigation Tree Pane, Quick Access Favorites, Volumes, and Tags Filter for NovaFiles.

#[cfg(not(test))]
use alloc::{string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{string::{String, ToString}, vec::Vec};


use crate::tags::FileTag;
use nova_ui::prelude::{BufferSurface, Color, Point, Rect};

/// Item in the navigation sidebar.
#[derive(Clone, Debug)]
pub struct SidebarItem {
    pub id: String,
    pub title: &'static str,
    pub path: String,
    pub icon_color: Color,
    pub bounds: Rect,
    pub is_selected: bool,
    pub is_drop_target: bool,
    pub tag_filter: Option<FileTag>,
}

/// Navigation sidebar component.
pub struct NavigationSidebar {
    pub bounds: Rect,
    pub items: Vec<SidebarItem>,
    pub selected_path: String,
    pub selected_tag: FileTag,
}

impl NavigationSidebar {
    pub fn new(bounds: Rect) -> Self {
        let mut s = Self {
            bounds,
            items: Vec::new(),
            selected_path: "/users/root/Documents".to_string(),
            selected_tag: FileTag::None,
        };
        s.rebuild_items();
        s
    }

    /// Rebuilds sidebar items (Favorites, Storage, Tags).
    pub fn rebuild_items(&mut self) {
        self.items.clear();

        let row_h = 26;
        let mut curr_y = self.bounds.y + 12;
        let w = self.bounds.width - 16;
        let x = self.bounds.x + 8;

        // --- Favorites Section ---
        let favorites = [
            ("Home", "/users/root", Color::rgba(0, 122, 255, 255)),
            ("Documents", "/users/root/Documents", Color::rgba(0, 122, 255, 255)),
            ("Downloads", "/users/root/Downloads", Color::rgba(52, 199, 89, 255)),
            ("Pictures", "/users/root/Pictures", Color::rgba(175, 82, 222, 255)),
            ("Music", "/users/root/Music", Color::rgba(255, 149, 0, 255)),
            ("System", "/sys", Color::rgba(142, 142, 147, 255)),
        ];

        curr_y += 18; // Section header gap
        for &(title, path, color) in favorites.iter() {
            let is_sel = self.selected_path == path && self.selected_tag == FileTag::None;
            self.items.push(SidebarItem {
                id: title.to_string(),
                title,
                path: path.to_string(),
                icon_color: color,
                bounds: Rect::new(x, curr_y, w, row_h),
                is_selected: is_sel,
                is_drop_target: false,
                tag_filter: None,
            });
            curr_y += row_h + 2;
        }

        // --- Storage Section ---
        curr_y += 14;
        let volumes = [
            ("NovaFS Root", "/", Color::rgba(0, 122, 255, 255)),
            ("FAT32 System", "/mnt/fat32", Color::rgba(255, 204, 0, 255)),
            ("ext2 Storage", "/mnt/ext2", Color::rgba(52, 199, 89, 255)),
            ("Snapshots", "/snapshots", Color::rgba(52, 199, 89, 255)),
        ];

        curr_y += 18;
        for &(title, path, color) in volumes.iter() {
            let is_sel = self.selected_path == path && self.selected_tag == FileTag::None;
            self.items.push(SidebarItem {
                id: title.to_string(),
                title,
                path: path.to_string(),
                icon_color: color,
                bounds: Rect::new(x, curr_y, w, row_h),
                is_selected: is_sel,
                is_drop_target: false,
                tag_filter: None,
            });
            curr_y += row_h + 2;
        }

        // --- Tags Section ---
        curr_y += 14;
        curr_y += 18;
        for tag in FileTag::ALL_TAGS {
            let is_sel = self.selected_tag == tag;
            self.items.push(SidebarItem {
                id: tag.name().to_string(),
                title: tag.name(),
                path: "/tags".to_string(),
                icon_color: tag.color(),
                bounds: Rect::new(x, curr_y, w, row_h),
                is_selected: is_sel,
                is_drop_target: false,
                tag_filter: Some(tag),
            });
            curr_y += row_h + 2;
        }
    }

    /// Hit-tests a click in the sidebar.
    pub fn hit_test(&self, p: Point) -> Option<&SidebarItem> {
        self.items.iter().find(|item| item.bounds.contains(p))
    }

    /// Sets the active selected path.
    pub fn set_selected_path(&mut self, path: &str) {
        self.selected_path = path.to_string();
        self.selected_tag = FileTag::None;
        for item in self.items.iter_mut() {
            item.is_selected = item.path == path && item.tag_filter.is_none();
        }
    }

    /// Sets the active tag filter.
    pub fn set_selected_tag(&mut self, tag: FileTag) {
        self.selected_tag = tag;
        for item in self.items.iter_mut() {
            item.is_selected = item.tag_filter == Some(tag);
        }
    }

    /// Highlights drop target during drag-and-drop.
    pub fn update_drop_target(&mut self, p: Point) -> Option<String> {
        let mut target_path = None;
        for item in self.items.iter_mut() {
            if item.bounds.contains(p) && item.tag_filter.is_none() {
                item.is_drop_target = true;
                target_path = Some(item.path.clone());
            } else {
                item.is_drop_target = false;
            }
        }
        target_path
    }

    /// Clears drop target highlights.
    pub fn clear_drop_targets(&mut self) {
        for item in self.items.iter_mut() {
            item.is_drop_target = false;
        }
    }

    /// Renders sidebar onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        let bg = if is_dark { Color::rgba(24, 24, 28, 255) } else { Color::rgba(242, 242, 245, 255) };
        let border = if is_dark { Color::rgba(45, 45, 52, 255) } else { Color::rgba(220, 220, 226, 255) };
        let text_color = if is_dark { Color::rgba(220, 220, 230, 255) } else { Color::rgba(30, 30, 35, 255) };
        let header_color = Color::rgba(140, 140, 150, 200);

        surface.fill_rect(self.bounds, bg);
        // Vertical right separator line
        surface.fill_rect(Rect::new(self.bounds.x + self.bounds.width - 1, self.bounds.y, 1, self.bounds.height), border);

        // Render Section Headers
        let mut hy = self.bounds.y + 12;
        // Favorites header
        surface.fill_rect(Rect::new(self.bounds.x + 14, hy + 2, 60, 3), header_color);
        hy += 18 + 6 * 28 + 14;

        // Drives header
        surface.fill_rect(Rect::new(self.bounds.x + 14, hy + 2, 70, 3), header_color);
        hy += 18 + 4 * 28 + 14;

        // Tags header
        surface.fill_rect(Rect::new(self.bounds.x + 14, hy + 2, 40, 3), header_color);

        // Render items
        for item in &self.items {
            if item.is_drop_target {
                surface.fill_rect(item.bounds, Color::rgba(52, 199, 89, 70));
                surface.stroke_rounded_rect(item.bounds, 0, 1, Color::rgba(52, 199, 89, 255));
            } else if item.is_selected {
                surface.fill_rect(item.bounds, if is_dark { Color::rgba(0, 100, 200, 80) } else { Color::rgba(0, 122, 255, 50) });
                surface.fill_rect(Rect::new(item.bounds.x, item.bounds.y + 3, 3, item.bounds.height - 6), Color::rgba(0, 122, 255, 255));
            }

            // Icon dot or badge
            if item.tag_filter.is_some() {
                surface.fill_rect(Rect::new(item.bounds.x + 10, item.bounds.y + 9, 8, 8), item.icon_color);
            } else {
                surface.fill_rect(Rect::new(item.bounds.x + 8, item.bounds.y + 6, 14, 14), item.icon_color);
            }

            // Title label bar
            surface.fill_rect(Rect::new(item.bounds.x + 28, item.bounds.y + 11, item.bounds.width - 40, 3), text_color);
        }
    }
}
