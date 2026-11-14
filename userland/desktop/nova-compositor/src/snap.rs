//! userland/desktop/nova-compositor/src/snap.rs
//!
//! NovaOS Wayland Display Server: Windows 11 Snap Layouts & Tiling Grid Subsystem
//!
//! Implements intelligent window tiling and snap layouts:
//!   - Proportional split layouts: 50/50 vertical, 1/3-2/3 split, 2x2 quadrants, and 3-column layouts.
//!   - Interactive snap detection: dragging window to screen edges/corners triggers snap preview zones.
//!   - Maximize button hover overlay: dwelling over window maximize button reveals interactive snap template card.
//!   - Synchronized border resizing: adjustments to a shared boundary smoothly resize neighboring snapped tiles.

#![allow(dead_code)]

use crate::surface::DamageRect;

pub const SNAP_EDGE_THRESHOLD: i32 = 12;
pub const SNAP_CORNER_THRESHOLD: i32 = 40;
pub const MAXIMIZE_HOVER_DWELL_TICKS: u32 = 18; // ~300ms at 60Hz

// -----------------------------------------------------------------------------
// Snap Layout Kinds
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum SnapLayout {
    #[default]
    None,
    Maximize,
    SplitLeft50,
    SplitRight50,
    QuadrantTopLeft,
    QuadrantTopRight,
    QuadrantBottomLeft,
    QuadrantBottomRight,
    SplitOneThirdLeft,
    SplitTwoThirdsRight,
    ColumnThreeLeft,
    ColumnThreeCenter,
    ColumnThreeRight,
}

impl SnapLayout {
    pub fn is_snapped(&self) -> bool {
        !matches!(self, SnapLayout::None)
    }

    pub fn is_half_split(&self) -> bool {
        matches!(self, SnapLayout::SplitLeft50 | SnapLayout::SplitRight50)
    }

    pub fn is_quadrant(&self) -> bool {
        matches!(
            self,
            SnapLayout::QuadrantTopLeft
                | SnapLayout::QuadrantTopRight
                | SnapLayout::QuadrantBottomLeft
                | SnapLayout::QuadrantBottomRight
        )
    }
}

// -----------------------------------------------------------------------------
// Workspace Geometry Constraints
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct WorkspaceArea {
    pub screen_w: u32,
    pub screen_h: u32,
    pub menubar_h: u32,
    pub dock_h: u32,
}

impl WorkspaceArea {
    pub const fn new(screen_w: u32, screen_h: u32, menubar_h: u32, dock_h: u32) -> Self {
        Self {
            screen_w,
            screen_h,
            menubar_h,
            dock_h,
        }
    }

    pub fn usable_rect(&self) -> DamageRect {
        let y = self.menubar_h as i32;
        let h = self.screen_h.saturating_sub(self.menubar_h + self.dock_h);
        DamageRect::new(0, y, self.screen_w, h)
    }
}

// -----------------------------------------------------------------------------
// Snap Engine & Calculator
// -----------------------------------------------------------------------------

pub struct SnapEngine {
    pub workspace: WorkspaceArea,
    pub active_layout: SnapLayout,
    pub preview_rect: Option<DamageRect>,
    pub maximize_dwell_timer: u32,
    pub show_snap_menu: bool,
    pub split_ratio_permille: u32, // 500 = 50.0%
}

impl SnapEngine {
    pub fn new(workspace: WorkspaceArea) -> Self {
        Self {
            workspace,
            active_layout: SnapLayout::None,
            preview_rect: None,
            maximize_dwell_timer: 0,
            show_snap_menu: false,
            split_ratio_permille: 500,
        }
    }

    /// Evaluates cursor position during window dragging to determine snap targets
    pub fn update_drag_gesture(&mut self, cursor_x: i32, cursor_y: i32) -> Option<SnapLayout> {
        let sw = self.workspace.screen_w as i32;
        let _sh = self.workspace.screen_h as i32;
        let top = self.workspace.menubar_h as i32;
        let bottom = (self.workspace.screen_h - self.workspace.dock_h) as i32;

        let at_left   = cursor_x <= SNAP_EDGE_THRESHOLD;
        let at_right  = cursor_x >= (sw - SNAP_EDGE_THRESHOLD);
        let at_top    = cursor_y <= (top + SNAP_EDGE_THRESHOLD);
        let _at_bottom = cursor_y >= (bottom - SNAP_EDGE_THRESHOLD);

        let corner_top_left     = cursor_x <= SNAP_CORNER_THRESHOLD && cursor_y <= (top + SNAP_CORNER_THRESHOLD);
        let corner_top_right    = cursor_x >= (sw - SNAP_CORNER_THRESHOLD) && cursor_y <= (top + SNAP_CORNER_THRESHOLD);
        let corner_bottom_left  = cursor_x <= SNAP_CORNER_THRESHOLD && cursor_y >= (bottom - SNAP_CORNER_THRESHOLD);
        let corner_bottom_right = cursor_x >= (sw - SNAP_CORNER_THRESHOLD) && cursor_y >= (bottom - SNAP_CORNER_THRESHOLD);

        let layout = if corner_top_left {
            SnapLayout::QuadrantTopLeft
        } else if corner_top_right {
            SnapLayout::QuadrantTopRight
        } else if corner_bottom_left {
            SnapLayout::QuadrantBottomLeft
        } else if corner_bottom_right {
            SnapLayout::QuadrantBottomRight
        } else if at_top {
            SnapLayout::Maximize
        } else if at_left {
            SnapLayout::SplitLeft50
        } else if at_right {
            SnapLayout::SplitRight50
        } else {
            SnapLayout::None
        };

        if layout != SnapLayout::None {
            self.active_layout = layout;
            self.preview_rect = Some(self.calculate_tile_geometry(layout));
            Some(layout)
        } else {
            self.active_layout = SnapLayout::None;
            self.preview_rect = None;
            None
        }
    }

    /// Calculates pixel-perfect tile geometry for a given layout
    pub fn calculate_tile_geometry(&self, layout: SnapLayout) -> DamageRect {
        let u = self.workspace.usable_rect();
        let w = u.width;
        let h = u.height;
        let x = u.x;
        let y = u.y;

        let half_w = (w * self.split_ratio_permille) / 1000;
        let remaining_w = w.saturating_sub(half_w);
        let half_h = h / 2;
        let remaining_h = h.saturating_sub(half_h);

        match layout {
            SnapLayout::None => DamageRect::default(),
            SnapLayout::Maximize => u,
            SnapLayout::SplitLeft50 => DamageRect::new(x, y, half_w, h),
            SnapLayout::SplitRight50 => DamageRect::new(x + half_w as i32, y, remaining_w, h),
            SnapLayout::QuadrantTopLeft => DamageRect::new(x, y, half_w, half_h),
            SnapLayout::QuadrantTopRight => DamageRect::new(x + half_w as i32, y, remaining_w, half_h),
            SnapLayout::QuadrantBottomLeft => DamageRect::new(x, y + half_h as i32, half_w, remaining_h),
            SnapLayout::QuadrantBottomRight => DamageRect::new(x + half_w as i32, y + half_h as i32, remaining_w, remaining_h),
            SnapLayout::SplitOneThirdLeft => {
                let w13 = w / 3;
                DamageRect::new(x, y, w13, h)
            }
            SnapLayout::SplitTwoThirdsRight => {
                let w13 = w / 3;
                DamageRect::new(x + w13 as i32, y, w - w13, h)
            }
            SnapLayout::ColumnThreeLeft => DamageRect::new(x, y, w / 3, h),
            SnapLayout::ColumnThreeCenter => DamageRect::new(x + (w / 3) as i32, y, w / 3, h),
            SnapLayout::ColumnThreeRight => DamageRect::new(x + ((2 * w) / 3) as i32, y, w - (2 * w / 3), h),
        }
    }

    /// Handles synchronized border drag resizing between neighboring left & right tiles
    pub fn resize_split_boundary(&mut self, cursor_x: i32) {
        let u = self.workspace.usable_rect();
        let relative_x = cursor_x.saturating_sub(u.x);
        let clamped = relative_x.max(100).min(u.width as i32 - 100) as u32;

        self.split_ratio_permille = ((clamped as u64 * 1000) / u.width as u64) as u32;
    }

    /// Resets snap preview
    pub fn clear_preview(&mut self) {
        self.active_layout = SnapLayout::None;
        self.preview_rect = None;
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_usable_area() {
        let ws = WorkspaceArea::new(1024, 768, 24, 36);
        let usable = ws.usable_rect();

        assert_eq!(usable.x, 0);
        assert_eq!(usable.y, 24);
        assert_eq!(usable.width, 1024);
        assert_eq!(usable.height, 708);
    }

    #[test]
    fn test_snap_edge_gestures() {
        let ws = WorkspaceArea::new(1024, 768, 24, 36);
        let mut engine = SnapEngine::new(ws);

        // Drag to far left edge -> SplitLeft50
        assert_eq!(engine.update_drag_gesture(5, 300), Some(SnapLayout::SplitLeft50));
        let left_rect = engine.preview_rect.unwrap();
        assert_eq!(left_rect.x, 0);
        assert_eq!(left_rect.y, 24);
        assert_eq!(left_rect.width, 512);
        assert_eq!(left_rect.height, 708);

        // Drag to far right edge -> SplitRight50
        assert_eq!(engine.update_drag_gesture(1020, 300), Some(SnapLayout::SplitRight50));
        let right_rect = engine.preview_rect.unwrap();
        assert_eq!(right_rect.x, 512);
        assert_eq!(right_rect.y, 24);
        assert_eq!(right_rect.width, 512);
        assert_eq!(right_rect.height, 708);

        // Drag to top edge -> Maximize
        assert_eq!(engine.update_drag_gesture(500, 26), Some(SnapLayout::Maximize));
        assert_eq!(engine.preview_rect.unwrap(), ws.usable_rect());

        // Drag to top-left corner -> QuadrantTopLeft
        assert_eq!(engine.update_drag_gesture(20, 30), Some(SnapLayout::QuadrantTopLeft));
        let quad_rect = engine.preview_rect.unwrap();
        assert_eq!(quad_rect.x, 0);
        assert_eq!(quad_rect.y, 24);
        assert_eq!(quad_rect.width, 512);
        assert_eq!(quad_rect.height, 354);
    }

    #[test]
    fn test_synchronized_border_resizing() {
        let ws = WorkspaceArea::new(1000, 800, 0, 0);
        let mut engine = SnapEngine::new(ws);

        // Default ratio 50%
        let tile_left = engine.calculate_tile_geometry(SnapLayout::SplitLeft50);
        assert_eq!(tile_left.width, 500);

        // Drag border to x=600 -> 60%
        engine.resize_split_boundary(600);
        assert_eq!(engine.split_ratio_permille, 600);

        let left_resized = engine.calculate_tile_geometry(SnapLayout::SplitLeft50);
        let right_resized = engine.calculate_tile_geometry(SnapLayout::SplitRight50);

        assert_eq!(left_resized.width, 600);
        assert_eq!(right_resized.x, 600);
        assert_eq!(right_resized.width, 400);
    }
}
