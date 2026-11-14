//! userland/desktop/nova-compositor/src/snap.rs
//!
//! NovaOS Wayland Display Server: Windows 11 Snap Layouts & Tiling Grid Subsystem
//!
//! Implements intelligent window tiling and snap layouts:
//!   - Proportional split layouts: 50/50 vertical, 1/3-2/3 split, 2x2 quadrants, and 3-column layouts.
//!   - Interactive snap detection: dragging window to screen edges/corners triggers snap preview zones.
//!   - Maximize button hover overlay: dwelling over window maximize button reveals interactive snap template card.
//!   - Synchronized border resizing: adjustments to a shared boundary smoothly resize neighboring snapped tiles.
//!   - Pre-snap geometry restoration: un-snapping a window restores its original floating coordinates.
//!   - Translucent ghost preview overlay and acrylic card rendering onto compositor surfaces.

#![allow(dead_code)]

use crate::surface::DamageRect;

pub const SNAP_EDGE_THRESHOLD: i32 = 12;
pub const SNAP_CORNER_THRESHOLD: i32 = 40;
pub const MAXIMIZE_HOVER_DWELL_TICKS: u32 = 18; // ~300ms at 60Hz
pub const MAX_SNAPPED_WINDOWS: usize = 16;

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

    pub fn is_three_column(&self) -> bool {
        matches!(
            self,
            SnapLayout::ColumnThreeLeft
                | SnapLayout::ColumnThreeCenter
                | SnapLayout::ColumnThreeRight
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
// Interactive Snap Layout Overlay Card
// -----------------------------------------------------------------------------

/// Interactive Windows 11 snap layout template flyout card
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SnapOverlayCard {
    pub bounds: DamageRect,
    pub target_window_id: Option<u64>,
    pub hovered_tile: Option<SnapLayout>,

    // Template 1: 50 / 50 Split
    pub box_split50: DamageRect,
    pub tile_split50_l: DamageRect,
    pub tile_split50_r: DamageRect,

    // Template 2: 1/3 - 2/3 Split
    pub box_split13: DamageRect,
    pub tile_split13_l: DamageRect,
    pub tile_split13_r: DamageRect,

    // Template 3: 2x2 Quadrants
    pub box_quad: DamageRect,
    pub tile_quad_tl: DamageRect,
    pub tile_quad_tr: DamageRect,
    pub tile_quad_bl: DamageRect,
    pub tile_quad_br: DamageRect,

    // Template 4: 3 Columns
    pub box_col3: DamageRect,
    pub tile_col3_l: DamageRect,
    pub tile_col3_c: DamageRect,
    pub tile_col3_r: DamageRect,
}

impl SnapOverlayCard {
    pub fn new(anchor_x: i32, anchor_y: i32, target_window_id: Option<u64>, ws: &WorkspaceArea) -> Self {
        let card_w = 260u32;
        let card_h = 144u32;
        let pad = 8i32;
        let gap = 8i32;

        let usable = ws.usable_rect();

        // Position card centered below the maximize button anchor
        let mut x = anchor_x - (card_w as i32 / 2);
        let mut y = anchor_y + 8; // small gap below button

        // Clamp to screen bounds
        if x < usable.x {
            x = usable.x + 8;
        } else if (x + card_w as i32) > (usable.x + usable.width as i32) {
            x = (usable.x + usable.width as i32) - card_w as i32 - 8;
        }
        if (y + card_h as i32) > (usable.y + usable.height as i32) {
            // Flip above button if overflowing bottom
            y = anchor_y - card_h as i32 - 8;
        }

        let bounds = DamageRect::new(x, y, card_w, card_h);

        // 2x2 Grid of Templates inside the card
        let template_w = (card_w as i32 - (pad * 2 + gap)) / 2;
        let template_h = (card_h as i32 - (pad * 2 + gap)) / 2;

        let t1_x = x + pad;
        let t1_y = y + pad;
        let t2_x = t1_x + template_w + gap;
        let t2_y = t1_y;
        let t3_x = t1_x;
        let t3_y = t1_y + template_h + gap;
        let t4_x = t2_x;
        let t4_y = t3_y;

        let box_split50 = DamageRect::new(t1_x, t1_y, template_w as u32, template_h as u32);
        let box_split13 = DamageRect::new(t2_x, t2_y, template_w as u32, template_h as u32);
        let box_quad = DamageRect::new(t3_x, t3_y, template_w as u32, template_h as u32);
        let box_col3 = DamageRect::new(t4_x, t4_y, template_w as u32, template_h as u32);

        // Sub-tiles for Template 1 (50/50)
        let t1_half = template_w / 2;
        let tile_split50_l = DamageRect::new(t1_x, t1_y, t1_half as u32, template_h as u32);
        let tile_split50_r = DamageRect::new(t1_x + t1_half, t1_y, (template_w - t1_half) as u32, template_h as u32);

        // Sub-tiles for Template 2 (1/3 - 2/3)
        let t2_third = template_w / 3;
        let tile_split13_l = DamageRect::new(t2_x, t2_y, t2_third as u32, template_h as u32);
        let tile_split13_r = DamageRect::new(t2_x + t2_third, t2_y, (template_w - t2_third) as u32, template_h as u32);

        // Sub-tiles for Template 3 (2x2 Quadrants)
        let t3_half_w = template_w / 2;
        let t3_half_h = template_h / 2;
        let tile_quad_tl = DamageRect::new(t3_x, t3_y, t3_half_w as u32, t3_half_h as u32);
        let tile_quad_tr = DamageRect::new(t3_x + t3_half_w, t3_y, (template_w - t3_half_w) as u32, t3_half_h as u32);
        let tile_quad_bl = DamageRect::new(t3_x, t3_y + t3_half_h, t3_half_w as u32, (template_h - t3_half_h) as u32);
        let tile_quad_br = DamageRect::new(t3_x + t3_half_w, t3_y + t3_half_h, (template_w - t3_half_w) as u32, (template_h - t3_half_h) as u32);

        // Sub-tiles for Template 4 (3 Columns)
        let t4_third = template_w / 3;
        let tile_col3_l = DamageRect::new(t4_x, t4_y, t4_third as u32, template_h as u32);
        let tile_col3_c = DamageRect::new(t4_x + t4_third, t4_y, t4_third as u32, template_h as u32);
        let tile_col3_r = DamageRect::new(t4_x + (2 * t4_third), t4_y, (template_w - 2 * t4_third) as u32, template_h as u32);

        Self {
            bounds,
            target_window_id,
            hovered_tile: None,
            box_split50,
            tile_split50_l,
            tile_split50_r,
            box_split13,
            tile_split13_l,
            tile_split13_r,
            box_quad,
            tile_quad_tl,
            tile_quad_tr,
            tile_quad_bl,
            tile_quad_br,
            box_col3,
            tile_col3_l,
            tile_col3_c,
            tile_col3_r,
        }
    }

    /// Evaluates cursor position to hit-test template tiles
    pub fn hit_test(&mut self, px: i32, py: i32) -> Option<SnapLayout> {
        if !self.bounds.contains_point(px, py) {
            self.hovered_tile = None;
            return None;
        }

        let hit = if self.tile_split50_l.contains_point(px, py) {
            Some(SnapLayout::SplitLeft50)
        } else if self.tile_split50_r.contains_point(px, py) {
            Some(SnapLayout::SplitRight50)
        } else if self.tile_split13_l.contains_point(px, py) {
            Some(SnapLayout::SplitOneThirdLeft)
        } else if self.tile_split13_r.contains_point(px, py) {
            Some(SnapLayout::SplitTwoThirdsRight)
        } else if self.tile_quad_tl.contains_point(px, py) {
            Some(SnapLayout::QuadrantTopLeft)
        } else if self.tile_quad_tr.contains_point(px, py) {
            Some(SnapLayout::QuadrantTopRight)
        } else if self.tile_quad_bl.contains_point(px, py) {
            Some(SnapLayout::QuadrantBottomLeft)
        } else if self.tile_quad_br.contains_point(px, py) {
            Some(SnapLayout::QuadrantBottomRight)
        } else if self.tile_col3_l.contains_point(px, py) {
            Some(SnapLayout::ColumnThreeLeft)
        } else if self.tile_col3_c.contains_point(px, py) {
            Some(SnapLayout::ColumnThreeCenter)
        } else if self.tile_col3_r.contains_point(px, py) {
            Some(SnapLayout::ColumnThreeRight)
        } else {
            None
        };

        self.hovered_tile = hit;
        hit
    }

    /// Checks if mouse is inside card bounds with a margin for hysteresis
    pub fn contains_point_with_margin(&self, px: i32, py: i32, margin: i32) -> bool {
        let x1 = self.bounds.x - margin;
        let y1 = self.bounds.y - margin;
        let x2 = self.bounds.x + self.bounds.width as i32 + margin;
        let y2 = self.bounds.y + self.bounds.height as i32 + margin;

        px >= x1 && px < x2 && py >= y1 && py < y2
    }
}

// -----------------------------------------------------------------------------
// Snapped Window & Tiling Grid
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SnappedWindow {
    pub window_id: u64,
    pub layout: SnapLayout,
    pub pre_snap_rect: DamageRect,
    pub current_rect: DamageRect,
}

impl SnappedWindow {
    pub const fn new(window_id: u64, layout: SnapLayout, pre_snap_rect: DamageRect, current_rect: DamageRect) -> Self {
        Self {
            window_id,
            layout,
            pre_snap_rect,
            current_rect,
        }
    }
}

/// Fixed-capacity array of resized neighbor windows
#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub struct ResizedNeighbors {
    pub items: [(u64, DamageRect); 8],
    pub count: usize,
}

impl ResizedNeighbors {
    pub fn push(&mut self, window_id: u64, rect: DamageRect) {
        if self.count < 8 {
            self.items[self.count] = (window_id, rect);
            self.count += 1;
        }
    }
}

/// Tiling grid manager for tracking and dynamically resizing snapped windows
#[derive(Debug, Copy, Clone)]
pub struct TilingGrid {
    pub windows: [Option<SnappedWindow>; MAX_SNAPPED_WINDOWS],
    pub count: usize,
}

impl TilingGrid {
    pub const fn new() -> Self {
        Self {
            windows: [None; MAX_SNAPPED_WINDOWS],
            count: 0,
        }
    }

    /// Snaps a window into the tiling grid
    pub fn snap_window(
        &mut self,
        window_id: u64,
        layout: SnapLayout,
        pre_snap_geom: DamageRect,
        new_rect: DamageRect,
    ) -> DamageRect {
        // Check if window already in grid
        for slot in self.windows.iter_mut() {
            if let Some(w) = slot {
                if w.window_id == window_id {
                    w.layout = layout;
                    w.current_rect = new_rect;
                    return new_rect;
                }
            }
        }

        // Add new snapped window
        for slot in self.windows.iter_mut() {
            if slot.is_none() {
                *slot = Some(SnappedWindow::new(window_id, layout, pre_snap_geom, new_rect));
                self.count += 1;
                return new_rect;
            }
        }

        new_rect
    }

    /// Unsnaps a window from the grid, returning its original floating geometry
    pub fn unsnap_window(&mut self, window_id: u64) -> Option<DamageRect> {
        for slot in self.windows.iter_mut() {
            if let Some(w) = slot {
                if w.window_id == window_id {
                    let pre_snap = w.pre_snap_rect;
                    *slot = None;
                    self.count = self.count.saturating_sub(1);
                    return Some(pre_snap);
                }
            }
        }
        None
    }

    pub fn get_layout(&self, window_id: u64) -> Option<SnapLayout> {
        for slot in self.windows.iter() {
            if let Some(w) = slot {
                if w.window_id == window_id {
                    return Some(w.layout);
                }
            }
        }
        None
    }

    pub fn get_geometry(&self, window_id: u64) -> Option<DamageRect> {
        for slot in self.windows.iter() {
            if let Some(w) = slot {
                if w.window_id == window_id {
                    return Some(w.current_rect);
                }
            }
        }
        None
    }

    /// Resizes neighboring snapped windows when a shared border is dragged
    pub fn resize_neighbors(
        &mut self,
        cursor_x: i32,
        engine: &mut SnapEngine,
    ) -> ResizedNeighbors {
        engine.resize_split_boundary(cursor_x);
        let mut resized = ResizedNeighbors::default();

        for slot in self.windows.iter_mut() {
            if let Some(w) = slot {
                if w.layout.is_snapped() {
                    let updated_geom = engine.calculate_tile_geometry(w.layout);
                    w.current_rect = updated_geom;
                    resized.push(w.window_id, updated_geom);
                }
            }
        }

        resized
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
    pub snap_card: Option<SnapOverlayCard>,
    pub tiling_grid: TilingGrid,
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
            snap_card: None,
            tiling_grid: TilingGrid::new(),
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

    /// Updates maximize button hover dwelling to summon the Windows 11 snap layout card
    pub fn update_maximize_hover(
        &mut self,
        cursor_x: i32,
        cursor_y: i32,
        max_btn_rect: DamageRect,
        dt_ticks: u32,
        target_window_id: Option<u64>,
    ) -> bool {
        let is_over_button = max_btn_rect.contains_point(cursor_x, cursor_y);

        if is_over_button {
            self.maximize_dwell_timer = self.maximize_dwell_timer.saturating_add(dt_ticks);

            if self.maximize_dwell_timer >= MAXIMIZE_HOVER_DWELL_TICKS && !self.show_snap_menu {
                // Summon interactive snap card
                let anchor_x = max_btn_rect.x + (max_btn_rect.width as i32 / 2);
                let anchor_y = max_btn_rect.y + max_btn_rect.height as i32;

                self.snap_card = Some(SnapOverlayCard::new(
                    anchor_x,
                    anchor_y,
                    target_window_id,
                    &self.workspace,
                ));
                self.show_snap_menu = true;
            }
            return self.show_snap_menu;
        }

        // If snap menu is active, inspect if cursor is within card
        if self.show_snap_menu {
            if let Some(card) = &mut self.snap_card {
                if card.contains_point_with_margin(cursor_x, cursor_y, 14) {
                    if let Some(tile_layout) = card.hit_test(cursor_x, cursor_y) {
                        self.active_layout = tile_layout;
                        self.preview_rect = Some(self.calculate_tile_geometry(tile_layout));
                    } else {
                        self.preview_rect = None;
                    }
                    return true;
                }
            }

            // Drifted away: dismiss menu
            self.dismiss_snap_menu();
        } else {
            self.maximize_dwell_timer = 0;
        }

        false
    }

    /// Handles mouse click on the snap overlay card
    pub fn handle_overlay_click(&mut self, cursor_x: i32, cursor_y: i32) -> Option<SnapLayout> {
        if !self.show_snap_menu {
            return None;
        }

        if let Some(card) = &mut self.snap_card {
            if let Some(layout) = card.hit_test(cursor_x, cursor_y) {
                let target_win = card.target_window_id;
                self.dismiss_snap_menu();

                // If target window was specified, record in tiling grid
                if let Some(win_id) = target_win {
                    let pre_rect = self.calculate_tile_geometry(SnapLayout::None);
                    self.snap_window(win_id, layout, pre_rect);
                }

                return Some(layout);
            }
        }

        self.dismiss_snap_menu();
        None
    }

    /// Dismisses the snap overlay card and resets dwelling timer
    pub fn dismiss_snap_menu(&mut self) {
        self.show_snap_menu = false;
        self.snap_card = None;
        self.maximize_dwell_timer = 0;
        self.preview_rect = None;
    }

    /// Snaps a window into the tiling grid
    pub fn snap_window(&mut self, window_id: u64, layout: SnapLayout, current_geom: DamageRect) -> DamageRect {
        let new_rect = self.calculate_tile_geometry(layout);
        self.tiling_grid.snap_window(window_id, layout, current_geom, new_rect)
    }

    /// Resizes neighboring snapped windows when a shared border is dragged
    pub fn resize_tiling_grid_boundary(&mut self, cursor_x: i32) -> ResizedNeighbors {
        self.resize_split_boundary(cursor_x);
        let ws = self.workspace;
        let ratio = self.split_ratio_permille;
        let mut resized = ResizedNeighbors::default();

        for slot in self.tiling_grid.windows.iter_mut() {
            if let Some(w) = slot {
                if w.layout.is_snapped() {
                    let updated_geom = Self::compute_geometry(ws, ratio, w.layout);
                    w.current_rect = updated_geom;
                    resized.push(w.window_id, updated_geom);
                }
            }
        }

        resized
    }

    /// Unsnaps a window from the grid, restoring its pre-snap floating geometry
    pub fn unsnap_window(&mut self, window_id: u64) -> Option<DamageRect> {
        self.tiling_grid.unsnap_window(window_id)
    }

    /// Calculates pixel-perfect tile geometry for a given layout
    pub fn calculate_tile_geometry(&self, layout: SnapLayout) -> DamageRect {
        Self::compute_geometry(self.workspace, self.split_ratio_permille, layout)
    }

    /// Static calculation of tile geometry given workspace, ratio, and layout
    pub fn compute_geometry(workspace: WorkspaceArea, split_permille: u32, layout: SnapLayout) -> DamageRect {
        let u = workspace.usable_rect();
        let w = u.width;
        let h = u.height;
        let x = u.x;
        let y = u.y;

        let half_w = (w * split_permille) / 1000;
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

    /// Renders translucent ghost snap preview overlay onto display framebuffer
    pub fn render_ghost_preview(&self, buffer: &mut [u32], stride: u32, accent_argb: u32) {
        if let Some(rect) = self.preview_rect {
            if rect.is_empty() {
                return;
            }

            let x1 = rect.x.max(0) as u32;
            let y1 = rect.y.max(0) as u32;
            let x2 = (rect.x + rect.width as i32).max(0) as u32;
            let y2 = (rect.y + rect.height as i32).max(0) as u32;

            // Semi-transparent ghost blend: 25% accent tint
            let accent_r = (accent_argb >> 16) & 0xFF;
            let accent_g = (accent_argb >> 8) & 0xFF;
            let accent_b = accent_argb & 0xFF;

            for cy in y1..y2 {
                let row_offset = (cy * stride) as usize;
                for cx in x1..x2 {
                    let idx = row_offset + cx as usize;
                    if idx >= buffer.len() {
                        continue;
                    }

                    // Border highlight (outer 2px)
                    let is_border = cx < (x1 + 2) || cx >= (x2 - 2) || cy < (y1 + 2) || cy >= (y2 - 2);

                    if is_border {
                        buffer[idx] = accent_argb;
                    } else {
                        // Blend 25% tint
                        let orig = buffer[idx];
                        let orig_r = (orig >> 16) & 0xFF;
                        let orig_g = (orig >> 8) & 0xFF;
                        let orig_b = orig & 0xFF;

                        let r = (orig_r * 3 + accent_r) / 4;
                        let g = (orig_g * 3 + accent_g) / 4;
                        let b = (orig_b * 3 + accent_b) / 4;

                        buffer[idx] = (0xFF << 24) | (r << 16) | (g << 8) | b;
                    }
                }
            }
        }
    }

    /// Renders interactive Windows 11 snap overlay card with 4 layout templates
    pub fn render_snap_menu(&self, buffer: &mut [u32], stride: u32, accent_argb: u32) {
        if !self.show_snap_menu {
            return;
        }

        if let Some(card) = &self.snap_card {
            let cx1 = card.bounds.x.max(0) as u32;
            let cy1 = card.bounds.y.max(0) as u32;
            let cx2 = (card.bounds.x + card.bounds.width as i32).max(0) as u32;
            let cy2 = (card.bounds.y + card.bounds.height as i32).max(0) as u32;

            // Card background: Acrylic dark slate (0xFF202024) with border (0xFF3E3E42)
            let card_bg = 0xFF202024;
            let card_border = 0xFF3E3E42;
            let tile_idle_bg = 0xFF2B2B30;
            let tile_border = 0xFF45454C;

            for y in cy1..cy2 {
                let row_offset = (y * stride) as usize;
                for x in cx1..cx2 {
                    let idx = row_offset + x as usize;
                    if idx >= buffer.len() {
                        continue;
                    }

                    let is_card_border = x == cx1 || x == (cx2 - 1) || y == cy1 || y == (cy2 - 1);
                    buffer[idx] = if is_card_border { card_border } else { card_bg };
                }
            }

            // Helper lambda for rendering a template tile
            let mut draw_tile = |rect: DamageRect, layout: SnapLayout| {
                let is_hovered = card.hovered_tile == Some(layout);
                let bg_color = if is_hovered { accent_argb } else { tile_idle_bg };

                let rx1 = rect.x.max(0) as u32;
                let ry1 = rect.y.max(0) as u32;
                let rx2 = (rect.x + rect.width as i32).max(0) as u32;
                let ry2 = (rect.y + rect.height as i32).max(0) as u32;

                for y in ry1..ry2 {
                    let row_offset = (y * stride) as usize;
                    for x in rx1..rx2 {
                        let idx = row_offset + x as usize;
                        if idx >= buffer.len() {
                            continue;
                        }
                        let is_tile_border = x == rx1 || x == (rx2 - 1) || y == ry1 || y == (ry2 - 1);
                        buffer[idx] = if is_tile_border { tile_border } else { bg_color };
                    }
                }
            };

            // Template 1: 50 / 50
            draw_tile(card.tile_split50_l, SnapLayout::SplitLeft50);
            draw_tile(card.tile_split50_r, SnapLayout::SplitRight50);

            // Template 2: 1/3 - 2/3
            draw_tile(card.tile_split13_l, SnapLayout::SplitOneThirdLeft);
            draw_tile(card.tile_split13_r, SnapLayout::SplitTwoThirdsRight);

            // Template 3: 2x2 Quadrants
            draw_tile(card.tile_quad_tl, SnapLayout::QuadrantTopLeft);
            draw_tile(card.tile_quad_tr, SnapLayout::QuadrantTopRight);
            draw_tile(card.tile_quad_bl, SnapLayout::QuadrantBottomLeft);
            draw_tile(card.tile_quad_br, SnapLayout::QuadrantBottomRight);

            // Template 4: 3 Columns
            draw_tile(card.tile_col3_l, SnapLayout::ColumnThreeLeft);
            draw_tile(card.tile_col3_c, SnapLayout::ColumnThreeCenter);
            draw_tile(card.tile_col3_r, SnapLayout::ColumnThreeRight);
        }
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
    fn test_maximize_hover_dwelling_and_snap_menu() {
        let ws = WorkspaceArea::new(1024, 768, 28, 48);
        let mut engine = SnapEngine::new(ws);
        let max_btn = DamageRect::new(800, 30, 24, 24);

        // Hover for 10 ticks (under threshold 18) -> menu not shown
        assert!(!engine.update_maximize_hover(810, 40, max_btn, 10, Some(42)));
        assert!(!engine.show_snap_menu);

        // Hover 10 more ticks (total 20 >= 18) -> snap menu opens!
        assert!(engine.update_maximize_hover(810, 40, max_btn, 10, Some(42)));
        assert!(engine.show_snap_menu);
        assert!(engine.snap_card.is_some());

        let card = engine.snap_card.unwrap();
        assert_eq!(card.target_window_id, Some(42));
        assert!(card.bounds.width >= 240);
        assert!(card.bounds.height >= 120);
    }

    #[test]
    fn test_snap_card_interactive_hit_testing_and_selection() {
        let ws = WorkspaceArea::new(1024, 768, 28, 48);
        let mut engine = SnapEngine::new(ws);
        let max_btn = DamageRect::new(500, 30, 24, 24);

        // Open menu
        engine.update_maximize_hover(510, 40, max_btn, 25, Some(101));
        assert!(engine.show_snap_menu);

        let card = engine.snap_card.unwrap();

        // Hit-test Template 1 Left (50/50 Split Left)
        let t1_l_center_x = card.tile_split50_l.x + card.tile_split50_l.width as i32 / 2;
        let t1_l_center_y = card.tile_split50_l.y + card.tile_split50_l.height as i32 / 2;

        engine.update_maximize_hover(t1_l_center_x, t1_l_center_y, max_btn, 1, Some(101));
        assert_eq!(engine.active_layout, SnapLayout::SplitLeft50);
        assert!(engine.preview_rect.is_some());

        // Click to select
        let selected = engine.handle_overlay_click(t1_l_center_x, t1_l_center_y);
        assert_eq!(selected, Some(SnapLayout::SplitLeft50));
        assert!(!engine.show_snap_menu); // menu closed upon selection

        // Window 101 must now be in the tiling grid!
        assert_eq!(engine.tiling_grid.get_layout(101), Some(SnapLayout::SplitLeft50));
    }

    #[test]
    fn test_all_four_snap_templates_hit_testing() {
        let ws = WorkspaceArea::new(1024, 768, 0, 0);
        let mut card = SnapOverlayCard::new(500, 50, Some(1), &ws);

        // 1. Template 1 (50/50)
        assert_eq!(
            card.hit_test(card.tile_split50_l.x + 2, card.tile_split50_l.y + 2),
            Some(SnapLayout::SplitLeft50)
        );
        assert_eq!(
            card.hit_test(card.tile_split50_r.x + 2, card.tile_split50_r.y + 2),
            Some(SnapLayout::SplitRight50)
        );

        // 2. Template 2 (1/3 - 2/3)
        assert_eq!(
            card.hit_test(card.tile_split13_l.x + 2, card.tile_split13_l.y + 2),
            Some(SnapLayout::SplitOneThirdLeft)
        );
        assert_eq!(
            card.hit_test(card.tile_split13_r.x + 2, card.tile_split13_r.y + 2),
            Some(SnapLayout::SplitTwoThirdsRight)
        );

        // 3. Template 3 (2x2 Quadrants)
        assert_eq!(
            card.hit_test(card.tile_quad_tl.x + 2, card.tile_quad_tl.y + 2),
            Some(SnapLayout::QuadrantTopLeft)
        );
        assert_eq!(
            card.hit_test(card.tile_quad_br.x + 2, card.tile_quad_br.y + 2),
            Some(SnapLayout::QuadrantBottomRight)
        );

        // 4. Template 4 (3 Columns)
        assert_eq!(
            card.hit_test(card.tile_col3_l.x + 2, card.tile_col3_l.y + 2),
            Some(SnapLayout::ColumnThreeLeft)
        );
        assert_eq!(
            card.hit_test(card.tile_col3_c.x + 2, card.tile_col3_c.y + 2),
            Some(SnapLayout::ColumnThreeCenter)
        );
        assert_eq!(
            card.hit_test(card.tile_col3_r.x + 2, card.tile_col3_r.y + 2),
            Some(SnapLayout::ColumnThreeRight)
        );
    }

    #[test]
    fn test_tiling_grid_and_unsnap_restoration() {
        let ws = WorkspaceArea::new(1024, 768, 28, 48);
        let mut engine = SnapEngine::new(ws);

        let initial_floating = DamageRect::new(100, 100, 600, 400);

        // Snap window 55 to QuadrantTopLeft
        let snapped_rect = engine.snap_window(55, SnapLayout::QuadrantTopLeft, initial_floating);
        assert_eq!(snapped_rect.width, 512);
        assert_eq!(snapped_rect.height, 346); // (768 - 28 - 48) / 2 = 692 / 2 = 346

        assert_eq!(engine.tiling_grid.get_layout(55), Some(SnapLayout::QuadrantTopLeft));

        // Unsnap window 55 -> pre-snap geometry restored!
        let restored = engine.unsnap_window(55);
        assert_eq!(restored, Some(initial_floating));
        assert_eq!(engine.tiling_grid.get_layout(55), None);
    }

    #[test]
    fn test_synchronized_border_resizing_in_grid() {
        let ws = WorkspaceArea::new(1000, 800, 0, 0);
        let mut engine = SnapEngine::new(ws);

        // Snap Window 1 to Left50, Window 2 to Right50
        engine.snap_window(1, SnapLayout::SplitLeft50, DamageRect::new(50, 50, 400, 300));
        engine.snap_window(2, SnapLayout::SplitRight50, DamageRect::new(450, 50, 400, 300));

        assert_eq!(engine.tiling_grid.get_geometry(1).unwrap().width, 500);
        assert_eq!(engine.tiling_grid.get_geometry(2).unwrap().width, 500);

        // Drag shared split boundary to x=700 (70% split)
        let resized = engine.resize_tiling_grid_boundary(700);
        assert_eq!(resized.count, 2);

        // Window 1 resized to 700px, Window 2 moved to x=700 with width=300px!
        assert_eq!(engine.tiling_grid.get_geometry(1).unwrap().width, 700);
        assert_eq!(engine.tiling_grid.get_geometry(2).unwrap().x, 700);
        assert_eq!(engine.tiling_grid.get_geometry(2).unwrap().width, 300);
    }

    #[test]
    fn test_ghost_preview_and_menu_rendering() {
        let ws = WorkspaceArea::new(800, 600, 0, 0);
        let mut engine = SnapEngine::new(ws);

        let mut fb = [0u32; 800 * 600];

        // 1. Ghost preview rendering
        engine.preview_rect = Some(DamageRect::new(50, 50, 300, 200));
        engine.render_ghost_preview(&mut fb, 800, 0xFF0078D4);
        assert_ne!(fb[50 * 800 + 50], 0);

        // 2. Snap menu rendering
        let max_btn = DamageRect::new(200, 20, 20, 20);
        engine.update_maximize_hover(210, 25, max_btn, 30, Some(10));
        assert!(engine.show_snap_menu);

        engine.render_snap_menu(&mut fb, 800, 0xFF0078D4);
        let card = engine.snap_card.unwrap();
        let center_idx = (card.bounds.y + 10) as usize * 800 + (card.bounds.x + 10) as usize;
        assert_ne!(fb[center_idx], 0);
    }
}
