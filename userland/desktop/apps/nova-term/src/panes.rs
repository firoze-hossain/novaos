//! userland/desktop/apps/nova-term/src/panes.rs
//!
//! Split horizontal and vertical pane management for NovaOS Pro Developer Terminal.
//! Supports recursive binary split tree, divider layout, focus cycling, and dynamic resizing.

#[cfg(not(test))]
use alloc::{boxed::Box, format, string::String, vec::Vec};
#[cfg(test)]
use std::{boxed::Box, format, string::String, vec::Vec};

use nova_ui::prelude::{Point, Rect};
use crate::ansi::AnsiParser;
use crate::clipboard::TextSelection;
use crate::grid::TerminalGrid;
use crate::pty::PtySession;
use crate::theme::ColorScheme;
use crate::url::UrlScanner;

/// Direction for splitting a terminal pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneSplit {
    /// Split horizontally (pane stacked above and below, divider is horizontal)
    Horizontal,
    /// Split vertically (pane side by side, divider is vertical)
    Vertical,
}

/// Divider geometry between split panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneDivider {
    pub rect: Rect,
    pub split: PaneSplit,
}

/// A single terminal pane containing its own PTY session, screen grid, parser, and selection state.
pub struct TerminalPane {
    pub id: usize,
    pub title: String,
    pub grid: TerminalGrid,
    pub pty: PtySession,
    pub parser: AnsiParser,
    pub urls: UrlScanner,
    pub selection: TextSelection,
    pub bounds: Rect,
    pub is_focused: bool,
}

impl TerminalPane {
    pub fn new(id: usize, title: &str, cols: usize, rows: usize, theme: &ColorScheme) -> Self {
        let mut grid = TerminalGrid::new(cols, rows, theme);
        let mut pty = PtySession::new(
            cols as u16,
            rows as u16,
            (cols * 9) as u16,
            (rows * 18) as u16,
        );
        pty.spawn_shell();

        let mut parser = AnsiParser::new();
        let mut urls = UrlScanner::new();

        // Feed initial shell banner into grid
        let output = pty.read_output();
        if !output.is_empty() {
            parser.parse_bytes(&output, &mut grid, theme);
            urls.scan_grid(&grid);
        }

        Self {
            id,
            title: String::from(title),
            grid,
            pty,
            parser,
            urls,
            selection: TextSelection::new(),
            bounds: Rect::new(0, 0, 0, 0),
            is_focused: false,
        }
    }

    /// Empty stub pane for node swaps
    pub fn new_empty() -> Self {
        let theme = ColorScheme::from_preset(crate::theme::ThemePreset::NovaDark);
        Self {
            id: 0,
            title: String::new(),
            grid: TerminalGrid::new(10, 10, &theme),
            pty: PtySession::new(10, 10, 90, 180),
            parser: AnsiParser::new(),
            urls: UrlScanner::new(),
            selection: TextSelection::new(),
            bounds: Rect::new(0, 0, 0, 0),
            is_focused: false,
        }
    }

    /// Update pane by polling PTY output and feeding it to ANSI parser
    pub fn update(&mut self, theme: &ColorScheme) -> bool {
        let output = self.pty.read_output();
        if !output.is_empty() {
            self.parser.parse_bytes(&output, &mut self.grid, theme);
            self.urls.scan_grid(&self.grid);
            true
        } else {
            false
        }
    }

    /// Send input bytes to the PTY
    pub fn send_input(&mut self, data: &[u8], theme: &ColorScheme) {
        self.pty.write_input(data);
        self.update(theme);
    }

    /// Resize pane geometry and notify PTY / grid
    pub fn resize(&mut self, bounds: Rect, char_width: i32, char_height: i32, theme: &ColorScheme) {
        self.bounds = bounds;
        let cw = if char_width > 0 { char_width } else { 9 };
        let ch = if char_height > 0 { char_height } else { 18 };

        let cols = (bounds.width / cw).max(10) as usize;
        let rows = (bounds.height / ch).max(4) as usize;

        if cols != self.grid.cols || rows != self.grid.rows {
            self.grid.resize(cols, rows, theme);
            self.pty.resize(
                cols as u16,
                rows as u16,
                bounds.width as u16,
                bounds.height as u16,
            );
            self.urls.scan_grid(&self.grid);
        }
    }
}

/// Binary tree node for split panes.
pub enum PaneNode {
    Leaf(TerminalPane),
    Split {
        split: PaneSplit,
        ratio: f32, // 0.0 .. 1.0 (proportion for first child)
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

impl PaneNode {
    /// Calculate layout rectangles recursively for all child panes
    pub fn layout(
        &mut self,
        bounds: Rect,
        divider_thickness: i32,
        char_width: i32,
        char_height: i32,
        theme: &ColorScheme,
    ) {
        match self {
            PaneNode::Leaf(pane) => {
                pane.resize(bounds, char_width, char_height, theme);
            }
            PaneNode::Split {
                split,
                ratio,
                first,
                second,
            } => {
                let div = divider_thickness.max(1);
                let r = ratio.clamp(0.1, 0.9);

                match split {
                    PaneSplit::Vertical => {
                        let total_w = bounds.width - div;
                        let first_w = ((total_w as f32) * r) as i32;
                        let second_w = total_w - first_w;

                        let r1 = Rect::new(bounds.x, bounds.y, first_w.max(1), bounds.height);
                        let r2 = Rect::new(
                            bounds.x + first_w + div,
                            bounds.y,
                            second_w.max(1),
                            bounds.height,
                        );

                        first.layout(r1, divider_thickness, char_width, char_height, theme);
                        second.layout(r2, divider_thickness, char_width, char_height, theme);
                    }
                    PaneSplit::Horizontal => {
                        let total_h = bounds.height - div;
                        let first_h = ((total_h as f32) * r) as i32;
                        let second_h = total_h - first_h;

                        let r1 = Rect::new(bounds.x, bounds.y, bounds.width, first_h.max(1));
                        let r2 = Rect::new(
                            bounds.x,
                            bounds.y + first_h + div,
                            bounds.width,
                            second_h.max(1),
                        );

                        first.layout(r1, divider_thickness, char_width, char_height, theme);
                        second.layout(r2, divider_thickness, char_width, char_height, theme);
                    }
                }
            }
        }
    }

    /// Collect all dividers for rendering
    pub fn collect_dividers(&self, divider_thickness: i32, list: &mut Vec<PaneDivider>) {
        if let PaneNode::Split {
            split,
            first,
            second,
            ..
        } = self
        {
            let div = divider_thickness.max(1);
            let b1 = first.bounds();

            match split {
                PaneSplit::Vertical => {
                    let div_rect = Rect::new(b1.x + b1.width, b1.y, div, b1.height);
                    list.push(PaneDivider {
                        rect: div_rect,
                        split: *split,
                    });
                }
                PaneSplit::Horizontal => {
                    let div_rect = Rect::new(b1.x, b1.y + b1.height, b1.width, div);
                    list.push(PaneDivider {
                        rect: div_rect,
                        split: *split,
                    });
                }
            }

            first.collect_dividers(divider_thickness, list);
            second.collect_dividers(divider_thickness, list);
        }
    }

    /// Get overall bounding box of this node
    pub fn bounds(&self) -> Rect {
        match self {
            PaneNode::Leaf(pane) => pane.bounds,
            PaneNode::Split { first, second, .. } => {
                let b1 = first.bounds();
                let b2 = second.bounds();
                let min_x = b1.x.min(b2.x);
                let min_y = b1.y.min(b2.y);
                let max_x = (b1.x + b1.width).max(b2.x + b2.width);
                let max_y = (b1.y + b1.height).max(b2.y + b2.height);
                Rect::new(min_x, min_y, max_x - min_x, max_y - min_y)
            }
        }
    }

    /// Count leaf panes
    pub fn pane_count(&self) -> usize {
        match self {
            PaneNode::Leaf(_) => 1,
            PaneNode::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    /// Collect all leaf IDs
    pub fn collect_ids(&self, ids: &mut Vec<usize>) {
        match self {
            PaneNode::Leaf(pane) => ids.push(pane.id),
            PaneNode::Split { first, second, .. } => {
                first.collect_ids(ids);
                second.collect_ids(ids);
            }
        }
    }

    /// Find pane by ID
    pub fn find_pane(&self, id: usize) -> Option<&TerminalPane> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == id {
                    Some(pane)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => {
                first.find_pane(id).or_else(|| second.find_pane(id))
            }
        }
    }

    /// Find pane by ID (mutable)
    pub fn find_pane_mut(&mut self, id: usize) -> Option<&mut TerminalPane> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == id {
                    Some(pane)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => {
                if first.find_pane(id).is_some() {
                    first.find_pane_mut(id)
                } else {
                    second.find_pane_mut(id)
                }
            }
        }
    }

    /// Find pane at coordinate
    pub fn find_pane_at_pos(&self, x: i32, y: i32) -> Option<&TerminalPane> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.bounds.contains(Point::new(x, y)) {
                    Some(pane)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => {
                first
                    .find_pane_at_pos(x, y)
                    .or_else(|| second.find_pane_at_pos(x, y))
            }
        }
    }

    /// Find pane at coordinate (mutable)
    pub fn find_pane_at_pos_mut(&mut self, x: i32, y: i32) -> Option<&mut TerminalPane> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.bounds.contains(Point::new(x, y)) {
                    Some(pane)
                } else {
                    None
                }
            }
            PaneNode::Split { first, second, .. } => {
                if first.bounds().contains(Point::new(x, y)) {
                    first.find_pane_at_pos_mut(x, y)
                } else {
                    second.find_pane_at_pos_mut(x, y)
                }
            }
        }
    }

    /// Split a target pane into two panes
    pub fn split_pane(
        &mut self,
        target_id: usize,
        new_id: usize,
        split: PaneSplit,
        cols: usize,
        rows: usize,
        theme: &ColorScheme,
    ) -> bool {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == target_id {
                    let old_title = format!("Pane {}", pane.id);
                    let new_title = format!("Pane {}", new_id);
                    let bounds = pane.bounds;

                    // Create replacement nodes
                    let mut existing_pane =
                        TerminalPane::new(pane.id, &old_title, cols, rows, theme);
                    existing_pane.bounds = bounds;

                    let mut new_pane = TerminalPane::new(new_id, &new_title, cols, rows, theme);
                    new_pane.bounds = bounds;

                    *self = PaneNode::Split {
                        split,
                        ratio: 0.5,
                        first: Box::new(PaneNode::Leaf(existing_pane)),
                        second: Box::new(PaneNode::Leaf(new_pane)),
                    };
                    true
                } else {
                    false
                }
            }
            PaneNode::Split { first, second, .. } => {
                if first.split_pane(target_id, new_id, split, cols, rows, theme) {
                    true
                } else {
                    second.split_pane(target_id, new_id, split, cols, rows, theme)
                }
            }
        }
    }

    /// Close a pane by ID. If found, returns the ID of the surviving adjacent pane.
    pub fn close_pane(&mut self, target_id: usize) -> Result<Option<usize>, ()> {
        match self {
            PaneNode::Leaf(pane) => {
                if pane.id == target_id {
                    // Cannot close root single pane
                    Err(())
                } else {
                    Ok(None)
                }
            }
            PaneNode::Split {
                first,
                second,
                ..
            } => {
                // Check if target is directly a child leaf of this split
                let first_is_target = match **first {
                    PaneNode::Leaf(ref p) => p.id == target_id,
                    _ => false,
                };
                if first_is_target {
                    let surviving = second.first_leaf_id();
                    let s = core::mem::replace(
                        second,
                        Box::new(PaneNode::Leaf(TerminalPane::new_empty())),
                    );
                    *self = *s;
                    return Ok(Some(surviving));
                }

                let second_is_target = match **second {
                    PaneNode::Leaf(ref p) => p.id == target_id,
                    _ => false,
                };
                if second_is_target {
                    let surviving = first.first_leaf_id();
                    let f = core::mem::replace(
                        first,
                        Box::new(PaneNode::Leaf(TerminalPane::new_empty())),
                    );
                    *self = *f;
                    return Ok(Some(surviving));
                }

                // Otherwise recurse down
                if let Ok(Some(id)) = first.close_pane(target_id) {
                    Ok(Some(id))
                } else {
                    second.close_pane(target_id)
                }
            }
        }
    }

    /// Get first leaf ID under this node
    pub fn first_leaf_id(&self) -> usize {
        match self {
            PaneNode::Leaf(pane) => pane.id,
            PaneNode::Split { first, .. } => first.first_leaf_id(),
        }
    }

    /// Traverse leaves to find the next pane ID after current_id (cycling)
    pub fn next_pane_id(&self, current_id: usize) -> usize {
        let mut ids = Vec::new();
        self.collect_ids(&mut ids);
        if ids.is_empty() {
            return current_id;
        }
        if let Some(pos) = ids.iter().position(|&id| id == current_id) {
            ids[(pos + 1) % ids.len()]
        } else {
            ids[0]
        }
    }

    /// Traverse leaves to find the previous pane ID before current_id (cycling)
    pub fn prev_pane_id(&self, current_id: usize) -> usize {
        let mut ids = Vec::new();
        self.collect_ids(&mut ids);
        if ids.is_empty() {
            return current_id;
        }
        if let Some(pos) = ids.iter().position(|&id| id == current_id) {
            if pos == 0 {
                ids[ids.len() - 1]
            } else {
                ids[pos - 1]
            }
        } else {
            ids[0]
        }
    }
}
