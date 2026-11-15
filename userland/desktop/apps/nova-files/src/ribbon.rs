//! userland/desktop/apps/nova-files/src/ribbon.rs
//!
//! Windows File Explorer Ribbon Toolbar for NovaFiles.
//! Provides categorized panels: Home (Clipboard, Organize, Select), View (Layouts, Panes, Sort), and Versions.

#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec::Vec;


use crate::view::ViewMode;
use nova_ui::prelude::{BufferSurface, Color, Point, Rect};

/// Ribbon tab category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    View,
    Versions,
}

/// Action item on the ribbon.
#[derive(Clone, Debug)]
pub struct RibbonButton {
    pub id: &'static str,
    pub label: &'static str,
    pub bounds: Rect,
    pub is_active: bool,
}

/// Ribbon toolbar component.
pub struct RibbonToolbar {
    pub bounds: Rect,
    pub active_tab: RibbonTab,
    pub is_collapsed: bool,
    pub buttons: Vec<RibbonButton>,
}

impl RibbonToolbar {
    pub fn new(bounds: Rect) -> Self {
        let mut r = Self {
            bounds,
            active_tab: RibbonTab::Home,
            is_collapsed: false,
            buttons: Vec::new(),
        };
        r.rebuild_buttons();
        r
    }

    /// Rebuilds button layout based on active ribbon tab.
    pub fn rebuild_buttons(&mut self) {
        self.buttons.clear();
        if self.is_collapsed {
            return;
        }

        let panel_y = self.bounds.y + 26;
        let btn_h = 32;
        let mut bx = self.bounds.x + 12;

        match self.active_tab {
            RibbonTab::Home => {
                // Clipboard Group
                self.buttons.push(RibbonButton { id: "copy", label: "Copy", bounds: Rect::new(bx, panel_y, 48, btn_h), is_active: false });
                bx += 52;
                self.buttons.push(RibbonButton { id: "cut", label: "Cut", bounds: Rect::new(bx, panel_y, 44, btn_h), is_active: false });
                bx += 48;
                self.buttons.push(RibbonButton { id: "paste", label: "Paste", bounds: Rect::new(bx, panel_y, 52, btn_h), is_active: false });
                bx += 64; // Group separator

                // Organize Group
                self.buttons.push(RibbonButton { id: "new_folder", label: "New Folder", bounds: Rect::new(bx, panel_y, 82, btn_h), is_active: false });
                bx += 86;
                self.buttons.push(RibbonButton { id: "rename", label: "Rename", bounds: Rect::new(bx, panel_y, 60, btn_h), is_active: false });
                bx += 64;
                self.buttons.push(RibbonButton { id: "delete", label: "Delete", bounds: Rect::new(bx, panel_y, 56, btn_h), is_active: false });
                bx += 60;
                self.buttons.push(RibbonButton { id: "duplicate", label: "Duplicate", bounds: Rect::new(bx, panel_y, 68, btn_h), is_active: false });
                bx += 80;

                // Selection Group
                self.buttons.push(RibbonButton { id: "select_all", label: "Select All", bounds: Rect::new(bx, panel_y, 74, btn_h), is_active: false });
            }
            RibbonTab::View => {
                // Layout Group: Icons, List, Columns, Details
                self.buttons.push(RibbonButton { id: "view_icons", label: "Icons", bounds: Rect::new(bx, panel_y, 56, btn_h), is_active: false });
                bx += 60;
                self.buttons.push(RibbonButton { id: "view_list", label: "List", bounds: Rect::new(bx, panel_y, 52, btn_h), is_active: false });
                bx += 56;
                self.buttons.push(RibbonButton { id: "view_columns", label: "Columns", bounds: Rect::new(bx, panel_y, 68, btn_h), is_active: false });
                bx += 72;
                self.buttons.push(RibbonButton { id: "view_details", label: "Details", bounds: Rect::new(bx, panel_y, 64, btn_h), is_active: false });
                bx += 80;

                // Panes Group
                self.buttons.push(RibbonButton { id: "toggle_preview", label: "Preview Pane", bounds: Rect::new(bx, panel_y, 90, btn_h), is_active: false });
                bx += 94;
                self.buttons.push(RibbonButton { id: "toggle_hidden", label: "Hidden Files", bounds: Rect::new(bx, panel_y, 86, btn_h), is_active: false });
            }
            RibbonTab::Versions => {
                self.buttons.push(RibbonButton { id: "open_snapshots", label: "Previous Versions", bounds: Rect::new(bx, panel_y, 124, btn_h), is_active: false });
                bx += 130;
                self.buttons.push(RibbonButton { id: "create_snapshot", label: "Create Snapshot", bounds: Rect::new(bx, panel_y, 114, btn_h), is_active: false });
            }
        }
    }

    /// Sets the active tab.
    pub fn set_tab(&mut self, tab: RibbonTab) {
        self.active_tab = tab;
        self.rebuild_buttons();
    }

    /// Synchronizes active state for view switcher buttons.
    pub fn update_view_mode(&mut self, mode: ViewMode) {
        for btn in self.buttons.iter_mut() {
            btn.is_active = match btn.id {
                "view_icons" => mode == ViewMode::Icons,
                "view_list" => mode == ViewMode::List,
                "view_columns" => mode == ViewMode::Columns,
                "view_details" => mode == ViewMode::Details,
                _ => false,
            };
        }
    }

    /// Hit-tests a click to find clicked ribbon button.
    pub fn hit_test_button(&self, p: Point) -> Option<&RibbonButton> {
        self.buttons.iter().find(|btn| btn.bounds.contains(p))
    }

    /// Renders ribbon toolbar onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        let bg = if is_dark { Color::rgba(28, 28, 34, 255) } else { Color::rgba(245, 245, 248, 255) };
        let tab_bg = if is_dark { Color::rgba(36, 36, 42, 255) } else { Color::rgba(255, 255, 255, 255) };
        let border = if is_dark { Color::rgba(55, 55, 62, 255) } else { Color::rgba(220, 220, 228, 255) };
        let text_color = if is_dark { Color::rgba(220, 220, 230, 255) } else { Color::rgba(30, 30, 35, 255) };

        surface.fill_rect(self.bounds, bg);
        surface.fill_rect(Rect::new(self.bounds.x, self.bounds.y + self.bounds.height - 1, self.bounds.width, 1), border);

        // Ribbon Category Tabs Bar (26px height)
        let tab_w = 70;
        let tabs = [
            (RibbonTab::Home, "Home"),
            (RibbonTab::View, "View"),
            (RibbonTab::Versions, "Versions"),
        ];

        for (idx, &(tab_type, _label)) in tabs.iter().enumerate() {
            let tx = self.bounds.x + 12 + (idx as i32) * tab_w;
            let tab_rect = Rect::new(tx, self.bounds.y, tab_w, 26);

            let is_sel = self.active_tab == tab_type;
            if is_sel {
                surface.fill_rect(tab_rect, tab_bg);
                surface.fill_rect(Rect::new(tab_rect.x, tab_rect.y + 24, tab_w, 2), Color::rgba(0, 122, 255, 255));
                surface.fill_rect(Rect::new(tab_rect.x + 16, tab_rect.y + 11, tab_w - 32, 4), Color::rgba(0, 122, 255, 255));
            } else {
                surface.fill_rect(Rect::new(tab_rect.x + 16, tab_rect.y + 11, tab_w - 32, 4), Color::rgba(140, 140, 150, 200));
            }
        }

        // Render Ribbon Buttons in the panel area below tabs
        if !self.is_collapsed {
            for btn in &self.buttons {
                if btn.is_active {
                    surface.fill_rect(btn.bounds, if is_dark { Color::rgba(0, 100, 200, 80) } else { Color::rgba(0, 122, 255, 50) });
                    surface.stroke_rounded_rect(btn.bounds, 0, 1, Color::rgba(0, 122, 255, 180));
                } else {
                    surface.fill_rect(btn.bounds, if is_dark { Color::rgba(34, 34, 40, 255) } else { Color::rgba(255, 255, 255, 255) });
                    surface.stroke_rounded_rect(btn.bounds, 0, 1, border);
                }

                // Button icon & label bars
                surface.fill_rect(Rect::new(btn.bounds.x + 8, btn.bounds.y + 8, 14, 14), if btn.is_active { Color::rgba(0, 122, 255, 255) } else { Color::rgba(120, 120, 130, 255) });
                surface.fill_rect(Rect::new(btn.bounds.x + 26, btn.bounds.y + 14, btn.bounds.width - 34, 3), text_color);
            }
        }
    }
}
