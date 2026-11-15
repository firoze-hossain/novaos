//! userland/desktop/apps/nova-term/src/tabs.rs
//!
//! Multi-tab session management and tab bar UI for NovaOS Pro Developer Terminal.
//! Manages multiple independent terminal sessions with tab switching, new tab creation,
//! tab closing, and header rendering metrics.

#[cfg(not(test))]
use alloc::{string::String, vec::Vec};
#[cfg(test)]
use std::{string::String, vec::Vec};

use nova_ui::prelude::{Point, Rect};
use crate::panes::{PaneNode, PaneSplit, TerminalPane};
use crate::theme::ColorScheme;

/// Represents an action triggered on the tab bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabBarAction {
    SelectTab(usize),
    CloseTab(usize),
    NewTab,
}

/// A single tab containing its own tree of split panes and active pane tracking.
pub struct TerminalTab {
    pub id: usize,
    pub title: String,
    pub root_pane: PaneNode,
    pub active_pane_id: usize,
    pub bounds: Rect,
}

impl TerminalTab {
    pub fn new(
        id: usize,
        title: &str,
        first_pane_id: usize,
        cols: usize,
        rows: usize,
        theme: &ColorScheme,
    ) -> Self {
        let mut pane = TerminalPane::new(first_pane_id, title, cols, rows, theme);
        pane.is_focused = true;
        Self {
            id,
            title: String::from(title),
            root_pane: PaneNode::Leaf(pane),
            active_pane_id: first_pane_id,
            bounds: Rect::new(0, 0, 0, 0),
        }
    }

    /// Get reference to active terminal pane
    pub fn active_pane(&self) -> Option<&TerminalPane> {
        self.root_pane.find_pane(self.active_pane_id)
    }

    /// Get mutable reference to active terminal pane
    pub fn active_pane_mut(&mut self) -> Option<&mut TerminalPane> {
        let id = self.active_pane_id;
        self.root_pane.find_pane_mut(id)
    }

    /// Focus a specific pane by ID
    pub fn focus_pane(&mut self, pane_id: usize) -> bool {
        if self.root_pane.find_pane(pane_id).is_some() {
            let prev_id = self.active_pane_id;
            if let Some(prev) = self.root_pane.find_pane_mut(prev_id) {
                prev.is_focused = false;
            }
            if let Some(curr) = self.root_pane.find_pane_mut(pane_id) {
                curr.is_focused = true;
                self.active_pane_id = pane_id;
                return true;
            }
        }
        false
    }

    /// Cycle focus to next pane
    pub fn cycle_focus_next(&mut self) {
        let next_id = self.root_pane.next_pane_id(self.active_pane_id);
        self.focus_pane(next_id);
    }

    /// Cycle focus to previous pane
    pub fn cycle_focus_prev(&mut self) {
        let prev_id = self.root_pane.prev_pane_id(self.active_pane_id);
        self.focus_pane(prev_id);
    }

    /// Split active pane horizontally or vertically
    pub fn split_active(
        &mut self,
        new_pane_id: usize,
        split: PaneSplit,
        cols: usize,
        rows: usize,
        theme: &ColorScheme,
    ) -> bool {
        let target_id = self.active_pane_id;
        if self
            .root_pane
            .split_pane(target_id, new_pane_id, split, cols, rows, theme)
        {
            self.focus_pane(new_pane_id);
            true
        } else {
            false
        }
    }

    /// Close active pane; if closed, focus adjacent pane. Returns false if only one pane remains.
    pub fn close_active_pane(&mut self) -> bool {
        let target_id = self.active_pane_id;
        match self.root_pane.close_pane(target_id) {
            Ok(Some(surviving_id)) => {
                self.active_pane_id = surviving_id;
                if let Some(p) = self.root_pane.find_pane_mut(surviving_id) {
                    p.is_focused = true;
                }
                true
            }
            _ => false,
        }
    }

    /// Layout panes in allocated tab bounds
    pub fn layout(
        &mut self,
        bounds: Rect,
        divider_thickness: i32,
        char_width: i32,
        char_height: i32,
        theme: &ColorScheme,
    ) {
        self.bounds = bounds;
        self.root_pane
            .layout(bounds, divider_thickness, char_width, char_height, theme);
    }

    /// Update all panes in this tab
    pub fn update(&mut self, theme: &ColorScheme) -> bool {
        let mut changed = false;
        let mut ids = Vec::new();
        self.root_pane.collect_ids(&mut ids);
        for id in ids {
            if let Some(pane) = self.root_pane.find_pane_mut(id) {
                if pane.update(theme) {
                    changed = true;
                }
            }
        }
        changed
    }
}

/// Information needed to render a tab in the tab bar.
#[derive(Debug, Clone)]
pub struct TabItemMetric {
    pub tab_index: usize,
    pub tab_id: usize,
    pub title: String,
    pub is_active: bool,
    pub rect: Rect,
    pub close_rect: Rect,
}

/// TabBar component managing multiple tabs and tab bar rendering geometry.
pub struct TabBar {
    pub tabs: Vec<TerminalTab>,
    pub active_tab_index: usize,
    pub next_tab_id: usize,
    pub next_pane_id: usize,
    pub height: i32,
    pub bounds: Rect,
}

impl TabBar {
    pub fn new(theme: &ColorScheme) -> Self {
        let mut tab_bar = Self {
            tabs: Vec::new(),
            active_tab_index: 0,
            next_tab_id: 1,
            next_pane_id: 1,
            height: 32,
            bounds: Rect::new(0, 0, 0, 32),
        };

        // Initialize with default primary tab
        tab_bar.add_tab("Terminal 1", 80, 24, theme);
        tab_bar
    }

    /// Add a new tab
    pub fn add_tab(&mut self, title: &str, cols: usize, rows: usize, theme: &ColorScheme) -> usize {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;

        let pane_id = self.next_pane_id;
        self.next_pane_id += 1;

        let tab = TerminalTab::new(tab_id, title, pane_id, cols, rows, theme);
        self.tabs.push(tab);
        self.active_tab_index = self.tabs.len() - 1;
        tab_id
    }

    /// Close tab at specific index. Always preserves at least one tab.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return false;
        }

        self.tabs.remove(index);
        if self.active_tab_index >= self.tabs.len() {
            self.active_tab_index = self.tabs.len() - 1;
        }
        true
    }

    /// Close active tab
    pub fn close_active_tab(&mut self) -> bool {
        let idx = self.active_tab_index;
        self.close_tab(idx)
    }

    /// Get active tab reference
    pub fn active_tab(&self) -> Option<&TerminalTab> {
        self.tabs.get(self.active_tab_index)
    }

    /// Get active tab mutable reference
    pub fn active_tab_mut(&mut self) -> Option<&mut TerminalTab> {
        let idx = self.active_tab_index;
        self.tabs.get_mut(idx)
    }

    /// Select active tab by index
    pub fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab_index = index;
        }
    }

    /// Switch to next tab (cycling)
    pub fn switch_to_next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab_index = (self.active_tab_index + 1) % self.tabs.len();
        }
    }

    /// Switch to previous tab (cycling)
    pub fn switch_to_prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            if self.active_tab_index == 0 {
                self.active_tab_index = self.tabs.len() - 1;
            } else {
                self.active_tab_index -= 1;
            }
        }
    }

    /// Allocate geometry metrics for all tabs given current tab bar bounds
    pub fn compute_tab_metrics(&self) -> (Vec<TabItemMetric>, Rect) {
        let mut metrics = Vec::new();
        if self.tabs.is_empty() {
            return (
                metrics,
                Rect::new(self.bounds.x, self.bounds.y, 28, self.height),
            );
        }

        let max_tab_width = 180;
        let min_tab_width = 90;
        let new_tab_btn_width = 28;

        let available_width = (self.bounds.width - new_tab_btn_width - 10).max(60);
        let count = self.tabs.len() as i32;
        let mut tab_width = (available_width / count).min(max_tab_width);
        if tab_width < min_tab_width {
            tab_width = min_tab_width;
        }

        let mut current_x = self.bounds.x + 4;
        let tab_y = self.bounds.y + 4;
        let tab_h = self.height - 6;

        for (idx, tab) in self.tabs.iter().enumerate() {
            let tab_rect = Rect::new(current_x, tab_y, tab_width, tab_h);
            let close_rect = Rect::new(
                current_x + tab_width - 22,
                tab_y + (tab_h - 16) / 2,
                16,
                16,
            );

            metrics.push(TabItemMetric {
                tab_index: idx,
                tab_id: tab.id,
                title: tab.title.clone(),
                is_active: idx == self.active_tab_index,
                rect: tab_rect,
                close_rect,
            });

            current_x += tab_width + 2;
        }

        let new_tab_rect = Rect::new(current_x + 4, tab_y + (tab_h - 22) / 2, 22, 22);
        (metrics, new_tab_rect)
    }

    /// Handle mouse click inside tab bar
    pub fn handle_click(&mut self, x: i32, y: i32) -> Option<TabBarAction> {
        let pt = Point::new(x, y);
        if !self.bounds.contains(pt) {
            return None;
        }

        let (metrics, new_tab_rect) = self.compute_tab_metrics();

        if new_tab_rect.contains(pt) {
            return Some(TabBarAction::NewTab);
        }

        for item in metrics {
            if item.close_rect.contains(pt) {
                return Some(TabBarAction::CloseTab(item.tab_index));
            }
            if item.rect.contains(pt) {
                return Some(TabBarAction::SelectTab(item.tab_index));
            }
        }

        None
    }
}
