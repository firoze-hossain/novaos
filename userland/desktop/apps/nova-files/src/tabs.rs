//! userland/desktop/apps/nova-files/src/tabs.rs
//!
//! macOS/Modern Tabbed Browsing Engine for NovaFiles.
//! Manages multiple independent directory tabs, navigation histories, and view state per tab.

#[cfg(not(test))]
use alloc::{string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{string::{String, ToString}, vec::Vec};


use crate::view::ViewMode;
use nova_ui::prelude::{BufferSurface, Color, Point, Rect, WHITE};

/// An individual browser tab.
#[derive(Clone, Debug)]
pub struct BrowserTab {
    pub id: usize,
    pub title: String,
    pub path: String,
    pub view_mode: ViewMode,
    pub selected_paths: Vec<String>,
    pub history_back: Vec<String>,
    pub history_forward: Vec<String>,
    pub bounds: Rect,
    pub close_bounds: Rect,
}

impl BrowserTab {
    pub fn new(id: usize, path: &str) -> Self {
        let title = if path == "/" {
            "NovaFS".to_string()
        } else if let Some(last) = path.trim_end_matches('/').split('/').last() {
            last.to_string()
        } else {
            "Folder".to_string()
        };

        Self {
            id,
            title,
            path: path.to_string(),
            view_mode: ViewMode::Icons,
            selected_paths: Vec::new(),
            history_back: Vec::new(),
            history_forward: Vec::new(),
            bounds: Rect::new(0, 0, 150, 30),
            close_bounds: Rect::new(0, 0, 14, 14),
        }
    }

    /// Navigates to a new path within this tab, recording back history.
    pub fn navigate(&mut self, new_path: &str) {
        if self.path != new_path {
            self.history_back.push(self.path.clone());
            self.history_forward.clear();
            self.path = new_path.to_string();
            self.title = if new_path == "/" {
                "NovaFS".to_string()
            } else if let Some(last) = new_path.trim_end_matches('/').split('/').last() {
                last.to_string()
            } else {
                "Folder".to_string()
            };
            self.selected_paths.clear();
        }
    }

    /// Navigates back in history.
    pub fn go_back(&mut self) -> bool {
        if let Some(prev) = self.history_back.pop() {
            self.history_forward.push(self.path.clone());
            self.path = prev.clone();
            self.title = if prev == "/" {
                "NovaFS".to_string()
            } else if let Some(last) = prev.trim_end_matches('/').split('/').last() {
                last.to_string()
            } else {
                "Folder".to_string()
            };
            self.selected_paths.clear();
            true
        } else {
            false
        }
    }

    /// Navigates forward in history.
    pub fn go_forward(&mut self) -> bool {
        if let Some(next) = self.history_forward.pop() {
            self.history_back.push(self.path.clone());
            self.path = next.clone();
            self.title = if next == "/" {
                "NovaFS".to_string()
            } else if let Some(last) = next.trim_end_matches('/').split('/').last() {
                last.to_string()
            } else {
                "Folder".to_string()
            };
            self.selected_paths.clear();
            true
        } else {
            false
        }
    }
}

/// Tab bar manager.
pub struct TabBar {
    pub bounds: Rect,
    pub tabs: Vec<BrowserTab>,
    pub active_tab_idx: usize,
    pub next_id: usize,
    pub add_tab_bounds: Rect,
}

impl TabBar {
    pub fn new(bounds: Rect) -> Self {
        let mut tb = Self {
            bounds,
            tabs: Vec::new(),
            active_tab_idx: 0,
            next_id: 1,
            add_tab_bounds: Rect::new(0, 0, 24, 24),
        };
        tb.add_tab("/users/root/Documents");
        tb
    }

    /// Adds a new tab.
    pub fn add_tab(&mut self, path: &str) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(BrowserTab::new(id, path));
        self.active_tab_idx = self.tabs.len() - 1;
        self.rebuild_layout();
        self.active_tab_idx
    }

    /// Closes a tab by index.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 {
            return false; // Keep at least one tab open
        }

        if index < self.tabs.len() {
            self.tabs.remove(index);
            if self.active_tab_idx >= self.tabs.len() {
                self.active_tab_idx = self.tabs.len() - 1;
            }
            self.rebuild_layout();
            true
        } else {
            false
        }
    }

    /// Sets active tab by index.
    pub fn set_active_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab_idx = index;
        }
    }

    /// Returns mutable reference to active tab.
    pub fn active_tab_mut(&mut self) -> Option<&mut BrowserTab> {
        self.tabs.get_mut(self.active_tab_idx)
    }

    /// Returns reference to active tab.
    pub fn active_tab(&self) -> Option<&BrowserTab> {
        self.tabs.get(self.active_tab_idx)
    }

    /// Rebuilds tab geometry layout.
    pub fn rebuild_layout(&mut self) {
        let tab_w = 160;
        let tab_h = self.bounds.height;
        let mut tx = self.bounds.x + 8;

        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            tab.bounds = Rect::new(tx, self.bounds.y, tab_w, tab_h);
            tab.close_bounds = Rect::new(tx + tab_w - 20, self.bounds.y + (tab_h - 14) / 2, 14, 14);
            tx += tab_w + 4;
        }

        self.add_tab_bounds = Rect::new(tx + 4, self.bounds.y + (tab_h - 22) / 2, 22, 22);
    }

    /// Hit-tests tab bar click: returns either clicked tab index or close button of tab.
    pub fn hit_test(&self, p: Point) -> Option<(usize, bool)> {
        for (idx, tab) in self.tabs.iter().enumerate() {
            if tab.close_bounds.contains(p) {
                return Some((idx, true)); // Clicked close
            }
            if tab.bounds.contains(p) {
                return Some((idx, false)); // Clicked tab
            }
        }
        None
    }

    /// Hit-test add tab button.
    pub fn hit_test_add(&self, p: Point) -> bool {
        self.add_tab_bounds.contains(p)
    }

    /// Renders tab bar onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        let bg = if is_dark { Color::rgba(20, 20, 24, 255) } else { Color::rgba(230, 230, 235, 255) };
        let active_tab_bg = if is_dark { Color::rgba(32, 32, 38, 255) } else { Color::rgba(255, 255, 255, 255) };
        let inactive_tab_bg = if is_dark { Color::rgba(26, 26, 30, 255) } else { Color::rgba(240, 240, 245, 255) };
        let border = if is_dark { Color::rgba(45, 45, 52, 255) } else { Color::rgba(215, 215, 222, 255) };
        let text_color = if is_dark { Color::rgba(220, 220, 230, 255) } else { Color::rgba(30, 30, 35, 255) };

        surface.fill_rect(self.bounds, bg);
        surface.fill_rect(Rect::new(self.bounds.x, self.bounds.y + self.bounds.height - 1, self.bounds.width, 1), border);

        // Render Tabs
        for (idx, tab) in self.tabs.iter().enumerate() {
            let is_active = idx == self.active_tab_idx;
            let tab_bg = if is_active { active_tab_bg } else { inactive_tab_bg };

            surface.fill_rect(tab.bounds, tab_bg);
            surface.fill_rect(Rect::new(tab.bounds.x + tab.bounds.width - 1, tab.bounds.y + 4, 1, tab.bounds.height - 8), border);

            // Active blue top accent line
            if is_active {
                surface.fill_rect(Rect::new(tab.bounds.x, tab.bounds.y, tab.bounds.width, 2), Color::rgba(0, 122, 255, 255));
            }

            // Tab folder icon
            surface.fill_rect(Rect::new(tab.bounds.x + 10, tab.bounds.y + 9, 12, 12), Color::rgba(255, 204, 0, 255));

            // Tab title bar
            surface.fill_rect(Rect::new(tab.bounds.x + 28, tab.bounds.y + 13, tab.bounds.width - 56, 4), if is_active { text_color } else { Color::rgba(140, 140, 150, 200) });

            // Close button ('✕')
            surface.fill_rect(tab.close_bounds, Color::rgba(200, 200, 210, 80));
            surface.fill_rect(Rect::new(tab.close_bounds.x + 3, tab.close_bounds.y + 6, 8, 2), if is_dark { WHITE } else { Color::rgba(40, 40, 50, 255) });
        }

        // Add Tab '+' Button
        surface.fill_rect(self.add_tab_bounds, if is_dark { Color::rgba(35, 35, 42, 255) } else { Color::rgba(242, 242, 248, 255) });
        surface.stroke_rounded_rect(self.add_tab_bounds, 0, 1, border);
        // Plus icon
        let ax = self.add_tab_bounds.x + 7;
        let ay = self.add_tab_bounds.y + 7;
        surface.fill_rect(Rect::new(ax, ay + 3, 8, 2), Color::rgba(0, 122, 255, 255));
        surface.fill_rect(Rect::new(ax + 3, ay, 2, 8), Color::rgba(0, 122, 255, 255));
    }
}
