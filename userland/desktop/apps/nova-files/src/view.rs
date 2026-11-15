//! userland/desktop/apps/nova-files/src/view.rs
//!
//! View Mode Engines for NovaFiles: Icons, List, Columns (macOS Miller Columns), and Details.
//! Coordinates layout, geometry calculations, selection hit-testing, and BufferSurface rendering.

#[cfg(not(test))]
use alloc::{string::{String, ToString}, vec, vec::Vec};

#[cfg(test)]
use std::{string::{String, ToString}, vec, vec::Vec};


use crate::model::{FileEntry, FileKind, SortField, SortOrder};
use crate::tags::FileTag;
use nova_ui::prelude::{BufferSurface, Color, Point, Rect, WHITE};

/// The 4 standard view modes supported by NovaFiles.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewMode {
    Icons,
    List,
    Columns,
    Details,
}

impl ViewMode {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Icons => "Icons",
            Self::List => "List",
            Self::Columns => "Columns",
            Self::Details => "Details",
        }
    }

    pub const fn icon(&self) -> &'static str {
        match self {
            Self::Icons => "⊞",
            Self::List => "☰",
            Self::Columns => "☷",
            Self::Details => "≣",
        }
    }
}

/// A rendered item card in the view area.
#[derive(Clone, Debug)]
pub struct ViewItemSlot {
    pub entry: FileEntry,
    pub bounds: Rect,
    pub is_selected: bool,
    pub is_hovered: bool,
}

/// Column definition for Details View.
#[derive(Clone, Debug)]
pub struct DetailColumn {
    pub field: SortField,
    pub title: &'static str,
    pub width: i32,
    pub min_width: i32,
}

/// State for macOS Miller Columns view.
#[derive(Clone, Debug)]
pub struct MillerColumn {
    pub dir_path: String,
    pub items: Vec<FileEntry>,
    pub selected_index: Option<usize>,
    pub scroll_y: i32,
}

/// Comprehensive view engine managing layout and rendering.
pub struct ViewEngine {
    pub mode: ViewMode,
    pub bounds: Rect,
    pub scroll_y: i32,
    pub scroll_x: i32,
    pub item_slots: Vec<ViewItemSlot>,
    pub selected_paths: Vec<String>,
    pub hovered_path: Option<String>,
    pub sort_by: SortField,
    pub sort_order: SortOrder,
    pub icon_size: i32, // 48 or 64 px
    pub detail_columns: Vec<DetailColumn>,
    pub miller_columns: Vec<MillerColumn>,
}

impl ViewEngine {
    pub fn new(bounds: Rect) -> Self {
        let cols = vec![
            DetailColumn { field: SortField::Name, title: "Name", width: 280, min_width: 120 },
            DetailColumn { field: SortField::DateModified, title: "Date Modified", width: 160, min_width: 100 },
            DetailColumn { field: SortField::Type, title: "Type", width: 140, min_width: 80 },
            DetailColumn { field: SortField::Size, title: "Size", width: 100, min_width: 60 },
            DetailColumn { field: SortField::Tag, title: "Tags", width: 80, min_width: 50 },
        ];

        Self {
            mode: ViewMode::Icons,
            bounds,
            scroll_y: 0,
            scroll_x: 0,
            item_slots: Vec::new(),
            selected_paths: Vec::new(),
            hovered_path: None,
            sort_by: SortField::Name,
            sort_order: SortOrder::Ascending,
            icon_size: 64,
            detail_columns: cols,
            miller_columns: Vec::new(),
        }
    }

    /// Updates layout of items based on active view mode and available bounds.
    pub fn update_layout(&mut self, items: &[FileEntry]) {
        self.item_slots.clear();

        match self.mode {
            ViewMode::Icons => self.layout_icons_view(items),
            ViewMode::List => self.layout_list_view(items),
            ViewMode::Columns => self.layout_columns_view(items),
            ViewMode::Details => self.layout_details_view(items),
        }
    }

    /// Layout for 2D Grid / Icons View.
    fn layout_icons_view(&mut self, items: &[FileEntry]) {
        let slot_w = 96;
        let slot_h = 96;
        let padding = 16;
        let start_x = self.bounds.x + padding;
        let start_y = self.bounds.y + padding - self.scroll_y;
        let avail_w = self.bounds.width - padding * 2;
        let cols = (avail_w / slot_w).max(1);

        for (idx, entry) in items.iter().enumerate() {
            let col = (idx as i32) % cols;
            let row = (idx as i32) / cols;

            let x = start_x + col * slot_w;
            let y = start_y + row * slot_h;

            let is_sel = self.selected_paths.contains(&entry.path);
            let is_hov = self.hovered_path.as_ref() == Some(&entry.path);

            self.item_slots.push(ViewItemSlot {
                entry: entry.clone(),
                bounds: Rect::new(x, y, slot_w - 8, slot_h - 8),
                is_selected: is_sel,
                is_hovered: is_hov,
            });
        }
    }

    /// Layout for compact horizontal List View.
    fn layout_list_view(&mut self, items: &[FileEntry]) {
        let row_h = 28;
        let padding = 8;
        let mut curr_y = self.bounds.y + padding - self.scroll_y;
        let w = self.bounds.width - padding * 2;

        for entry in items.iter() {
            let is_sel = self.selected_paths.contains(&entry.path);
            let is_hov = self.hovered_path.as_ref() == Some(&entry.path);

            self.item_slots.push(ViewItemSlot {
                entry: entry.clone(),
                bounds: Rect::new(self.bounds.x + padding, curr_y, w, row_h),
                is_selected: is_sel,
                is_hovered: is_hov,
            });

            curr_y += row_h + 2;
        }
    }

    /// Layout for macOS Miller Columns View.
    fn layout_columns_view(&mut self, items: &[FileEntry]) {
        let col_w = 220;
        let row_h = 26;
        let start_x = self.bounds.x - self.scroll_x;

        // Populate primary column if empty
        if self.miller_columns.is_empty() {
            self.miller_columns.push(MillerColumn {
                dir_path: "/".to_string(),
                items: items.to_vec(),
                selected_index: None,
                scroll_y: 0,
            });
        } else {
            self.miller_columns[0].items = items.to_vec();
        }

        // Layout items across all active Miller columns
        for (col_idx, m_col) in self.miller_columns.iter().enumerate() {
            let cx = start_x + (col_idx as i32) * col_w;
            let mut cy = self.bounds.y + 4 - m_col.scroll_y;

            for (row_idx, entry) in m_col.items.iter().enumerate() {
                let is_sel = m_col.selected_index == Some(row_idx);
                let is_hov = self.hovered_path.as_ref() == Some(&entry.path);

                self.item_slots.push(ViewItemSlot {
                    entry: entry.clone(),
                    bounds: Rect::new(cx + 4, cy, col_w - 8, row_h),
                    is_selected: is_sel,
                    is_hovered: is_hov,
                });

                cy += row_h + 2;
            }
        }
    }

    /// Layout for Windows Details View with Sortable Columns.
    fn layout_details_view(&mut self, items: &[FileEntry]) {
        let header_h = 28;
        let row_h = 26;
        let start_y = self.bounds.y + header_h - self.scroll_y;
        let total_w = self.bounds.width;

        let mut curr_y = start_y;
        for entry in items.iter() {
            let is_sel = self.selected_paths.contains(&entry.path);
            let is_hov = self.hovered_path.as_ref() == Some(&entry.path);

            self.item_slots.push(ViewItemSlot {
                entry: entry.clone(),
                bounds: Rect::new(self.bounds.x, curr_y, total_w, row_h),
                is_selected: is_sel,
                is_hovered: is_hov,
            });

            curr_y += row_h + 1;
        }
    }

    /// Hit-tests a point in the view to locate clicked item slot.
    pub fn hit_test(&self, p: Point) -> Option<&ViewItemSlot> {
        self.item_slots.iter().find(|slot| slot.bounds.contains(p))
    }

    /// Toggles item selection on click with modifier support.
    pub fn select_item(&mut self, path: &str, is_multi: bool, is_range: bool) {
        if !is_multi && !is_range {
            self.selected_paths.clear();
            self.selected_paths.push(path.to_string());
        } else if is_multi {
            if let Some(pos) = self.selected_paths.iter().position(|p| p == path) {
                self.selected_paths.remove(pos);
            } else {
                self.selected_paths.push(path.to_string());
            }
        } else if is_range {
            if !self.selected_paths.contains(&path.to_string()) {
                self.selected_paths.push(path.to_string());
            }
        }

        // Synchronize slot selected flags
        for slot in self.item_slots.iter_mut() {
            slot.is_selected = self.selected_paths.contains(&slot.entry.path);
        }
    }

    /// Selects all visible items.
    pub fn select_all(&mut self) {
        self.selected_paths.clear();
        for slot in self.item_slots.iter() {
            self.selected_paths.push(slot.entry.path.clone());
        }
        for slot in self.item_slots.iter_mut() {
            slot.is_selected = true;
        }
    }

    /// Clears active selection.
    pub fn clear_selection(&mut self) {
        self.selected_paths.clear();
        for slot in self.item_slots.iter_mut() {
            slot.is_selected = false;
        }
    }

    /// Sets hovered path.
    pub fn set_hover(&mut self, p: Point) {
        if let Some(slot) = self.hit_test(p) {
            self.hovered_path = Some(slot.entry.path.clone());
        } else {
            self.hovered_path = None;
        }
        for slot in self.item_slots.iter_mut() {
            slot.is_hovered = self.hovered_path.as_ref() == Some(&slot.entry.path);
        }
    }

    /// Renders view items onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        let bg_color = if is_dark {
            Color::rgba(26, 26, 30, 255)
        } else {
            Color::rgba(250, 250, 252, 255)
        };
        surface.fill_rect(self.bounds, bg_color);

        match self.mode {
            ViewMode::Icons => self.render_icons(surface, is_dark),
            ViewMode::List => self.render_list(surface, is_dark),
            ViewMode::Columns => self.render_columns(surface, is_dark),
            ViewMode::Details => self.render_details(surface, is_dark),
        }
    }

    fn render_icons(&self, surface: &mut BufferSurface, is_dark: bool) {
        let text_color = if is_dark { Color::rgba(235, 235, 240, 255) } else { Color::rgba(20, 20, 24, 255) };
        let sel_bg = if is_dark { Color::rgba(0, 100, 200, 90) } else { Color::rgba(0, 120, 255, 60) };
        let sel_border = Color::rgba(0, 122, 255, 200);
        let hov_bg = if is_dark { Color::rgba(255, 255, 255, 20) } else { Color::rgba(0, 0, 0, 15) };

        for slot in &self.item_slots {
            if slot.bounds.y + slot.bounds.height < self.bounds.y || slot.bounds.y > self.bounds.y + self.bounds.height {
                continue; // Clipped
            }

            if slot.is_selected {
                surface.fill_rect(slot.bounds, sel_bg);
                surface.stroke_rounded_rect(slot.bounds, 0, 1, sel_border);
            } else if slot.is_hovered {
                surface.fill_rect(slot.bounds, hov_bg);
            }

            // Draw icon placeholder & tag dot
            let icon_rect = Rect::new(slot.bounds.x + (slot.bounds.width - 40) / 2, slot.bounds.y + 10, 40, 40);
            let icon_color = match slot.entry.kind {
                FileKind::Directory => Color::rgba(255, 200, 70, 255),
                FileKind::TextDocument => Color::rgba(100, 180, 255, 255),
                FileKind::Code => Color::rgba(70, 210, 120, 255),
                FileKind::Image => Color::rgba(230, 120, 230, 255),
                FileKind::Audio => Color::rgba(255, 120, 100, 255),
                FileKind::PackageArchive => Color::rgba(240, 150, 50, 255),
                FileKind::Executable => Color::rgba(180, 180, 190, 255),
                _ => Color::rgba(140, 140, 150, 255),
            };
            surface.fill_rect(icon_rect, icon_color);

            // Colored tag dot on bottom right of icon
            if slot.entry.tag != FileTag::None {
                let tag_dot = Rect::new(icon_rect.x + icon_rect.width - 6, icon_rect.y + icon_rect.height - 6, 8, 8);
                surface.fill_rect(tag_dot, slot.entry.tag.color());
                surface.stroke_rounded_rect(tag_dot, 0, 1, WHITE);
            }

            // Text label bar
            let label_rect = Rect::new(slot.bounds.x + 4, slot.bounds.y + 56, slot.bounds.width - 8, 16);
            let bar_color = if slot.is_selected { Color::rgba(0, 122, 255, 180) } else { text_color };
            surface.fill_rect(Rect::new(label_rect.x + 4, label_rect.y + 6, label_rect.width - 8, 3), bar_color);
        }
    }

    fn render_list(&self, surface: &mut BufferSurface, is_dark: bool) {
        let text_color = if is_dark { Color::rgba(220, 220, 230, 255) } else { Color::rgba(30, 30, 35, 255) };
        let sel_bg = if is_dark { Color::rgba(0, 100, 200, 80) } else { Color::rgba(0, 120, 255, 50) };
        let hov_bg = if is_dark { Color::rgba(255, 255, 255, 15) } else { Color::rgba(0, 0, 0, 10) };

        for slot in &self.item_slots {
            if slot.bounds.y + slot.bounds.height < self.bounds.y || slot.bounds.y > self.bounds.y + self.bounds.height {
                continue;
            }

            if slot.is_selected {
                surface.fill_rect(slot.bounds, sel_bg);
                surface.stroke_rounded_rect(slot.bounds, 0, 1, Color::rgba(0, 122, 255, 160));
            } else if slot.is_hovered {
                surface.fill_rect(slot.bounds, hov_bg);
            }

            // 16x16 icon
            let icon_rect = Rect::new(slot.bounds.x + 8, slot.bounds.y + 5, 16, 16);
            let ic_color = if slot.entry.kind == FileKind::Directory { Color::rgba(255, 200, 70, 255) } else { Color::rgba(100, 180, 255, 255) };
            surface.fill_rect(icon_rect, ic_color);

            // Tag indicator dot
            if slot.entry.tag != FileTag::None {
                let tag_rect = Rect::new(slot.bounds.x + 28, slot.bounds.y + 10, 6, 6);
                surface.fill_rect(tag_rect, slot.entry.tag.color());
            }

            // Name bar & size bar
            let name_bar = Rect::new(slot.bounds.x + 40, slot.bounds.y + 11, (slot.bounds.width / 2).min(200), 4);
            surface.fill_rect(name_bar, text_color);

            let size_bar = Rect::new(slot.bounds.x + slot.bounds.width - 70, slot.bounds.y + 11, 50, 3);
            surface.fill_rect(size_bar, Color::rgba(140, 140, 150, 200));
        }
    }

    fn render_columns(&self, surface: &mut BufferSurface, is_dark: bool) {
        let border_color = if is_dark { Color::rgba(50, 50, 55, 255) } else { Color::rgba(220, 220, 225, 255) };
        let sel_bg = if is_dark { Color::rgba(0, 100, 200, 90) } else { Color::rgba(0, 120, 255, 60) };
        let col_w = 220;

        // Render column separators
        for c_idx in 0..self.miller_columns.len().max(3) {
            let cx = self.bounds.x - self.scroll_x + (c_idx as i32) * col_w;
            surface.fill_rect(Rect::new(cx + col_w - 1, self.bounds.y, 1, self.bounds.height), border_color);
        }

        // Render items inside columns
        for slot in &self.item_slots {
            if slot.is_selected {
                surface.fill_rect(slot.bounds, sel_bg);
            } else if slot.is_hovered {
                surface.fill_rect(slot.bounds, Color::rgba(255, 255, 255, 15));
            }

            // Icon
            let ic_color = if slot.entry.kind == FileKind::Directory { Color::rgba(255, 200, 70, 255) } else { Color::rgba(100, 180, 255, 255) };
            surface.fill_rect(Rect::new(slot.bounds.x + 6, slot.bounds.y + 6, 14, 14), ic_color);

            // Name bar
            surface.fill_rect(Rect::new(slot.bounds.x + 26, slot.bounds.y + 11, slot.bounds.width - 50, 3), if is_dark { WHITE } else { Color::rgba(30, 30, 35, 255) });

            // Folder chevron if directory
            if slot.entry.kind == FileKind::Directory {
                surface.fill_rect(Rect::new(slot.bounds.x + slot.bounds.width - 14, slot.bounds.y + 10, 5, 5), Color::rgba(150, 150, 160, 255));
            }
        }
    }

    fn render_details(&self, surface: &mut BufferSurface, is_dark: bool) {
        let header_h = 28;
        let header_bg = if is_dark { Color::rgba(36, 36, 42, 255) } else { Color::rgba(240, 240, 244, 255) };
        let border_color = if is_dark { Color::rgba(55, 55, 62, 255) } else { Color::rgba(220, 220, 228, 255) };
        let zebra_bg = if is_dark { Color::rgba(30, 30, 35, 255) } else { Color::rgba(245, 245, 248, 255) };
        let text_color = if is_dark { Color::rgba(220, 220, 230, 255) } else { Color::rgba(30, 30, 35, 255) };
        let sel_bg = if is_dark { Color::rgba(0, 100, 200, 85) } else { Color::rgba(0, 120, 255, 55) };

        // Table Header
        let header_rect = Rect::new(self.bounds.x, self.bounds.y, self.bounds.width, header_h);
        surface.fill_rect(header_rect, header_bg);
        surface.fill_rect(Rect::new(self.bounds.x, self.bounds.y + header_h - 1, self.bounds.width, 1), border_color);

        let mut col_x = self.bounds.x + 8;
        for col in &self.detail_columns {
            // Header column separator
            surface.fill_rect(Rect::new(col_x + col.width - 4, self.bounds.y + 4, 1, header_h - 8), border_color);

            // Title indicator bar
            let title_w = (col.width - 30).min(100);
            surface.fill_rect(Rect::new(col_x + 4, self.bounds.y + 11, title_w, 4), text_color);

            // Sort indicator arrow if active
            if self.sort_by == col.field {
                let arrow_color = Color::rgba(0, 122, 255, 255);
                surface.fill_rect(Rect::new(col_x + title_w + 10, self.bounds.y + 10, 6, 6), arrow_color);
            }

            col_x += col.width;
        }

        // Table Rows
        for (idx, slot) in self.item_slots.iter().enumerate() {
            if slot.bounds.y + slot.bounds.height < self.bounds.y + header_h || slot.bounds.y > self.bounds.y + self.bounds.height {
                continue;
            }

            if slot.is_selected {
                surface.fill_rect(slot.bounds, sel_bg);
                surface.stroke_rounded_rect(slot.bounds, 0, 1, Color::rgba(0, 122, 255, 140));
            } else if slot.is_hovered {
                surface.fill_rect(slot.bounds, Color::rgba(255, 255, 255, 15));
            } else if idx % 2 == 1 {
                surface.fill_rect(slot.bounds, zebra_bg);
            }

            let mut cell_x = self.bounds.x + 8;

            // Column 1: Name with icon and tag dot
            let ic_color = if slot.entry.kind == FileKind::Directory { Color::rgba(255, 200, 70, 255) } else { Color::rgba(100, 180, 255, 255) };
            surface.fill_rect(Rect::new(cell_x + 4, slot.bounds.y + 5, 16, 16), ic_color);

            if slot.entry.tag != FileTag::None {
                surface.fill_rect(Rect::new(cell_x + 24, slot.bounds.y + 10, 6, 6), slot.entry.tag.color());
            }

            // Name bar
            surface.fill_rect(Rect::new(cell_x + 36, slot.bounds.y + 11, 140, 3), text_color);
            cell_x += self.detail_columns[0].width;

            // Column 2: Date Modified
            surface.fill_rect(Rect::new(cell_x + 4, slot.bounds.y + 11, 80, 3), Color::rgba(140, 140, 150, 200));
            cell_x += self.detail_columns[1].width;

            // Column 3: Type
            surface.fill_rect(Rect::new(cell_x + 4, slot.bounds.y + 11, 60, 3), Color::rgba(140, 140, 150, 200));
            cell_x += self.detail_columns[2].width;

            // Column 4: Size
            surface.fill_rect(Rect::new(cell_x + 4, slot.bounds.y + 11, 40, 3), Color::rgba(140, 140, 150, 200));
            cell_x += self.detail_columns[3].width;

            // Column 5: Tag
            if slot.entry.tag != FileTag::None {
                surface.fill_rect(Rect::new(cell_x + 10, slot.bounds.y + 8, 10, 10), slot.entry.tag.color());
            }
        }
    }
}
