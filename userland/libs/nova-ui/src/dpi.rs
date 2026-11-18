//! userland/libs/nova-ui/src/dpi.rs
//!
//! NovaOS High-Resolution Display Scaling & Vector Icon Rasterizer
//!
//! Provides dynamic display scaling (100%, 125%, 150%, 200%) across the retained-mode UI toolkit:
//!   - `DpiScale`: Scale factor enumeration (1.0x, 1.25x, 1.5x, 2.0x) with fixed-point permille math.
//!   - Bi-directional coordinate mapping: logical points (`Pt`) <-> physical hardware device pixels (`Px`).
//!   - Layout geometry scaling (`Point`, `Rect`, `Size`, `EdgeInsets`, `FlexStyle`).
//!   - Scalable Vector Icon Engine: razor-sharp native pixel rasterization for UI icons with anti-aliasing.
//!   - Display metrics & EDID integration for monitor density detection.

#![allow(dead_code)]

use crate::layout::{Dimension, EdgeInsets, FlexStyle, Point, Rect, Size};
use crate::render::{Color, BufferSurface};

pub const BASELINE_DPI: f32 = 96.0;

// =============================================================================
// DPI Scale Factor
// =============================================================================

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum DpiScale {
    #[default]
    Scale100, // 1.00x (100% - standard 96 DPI)
    Scale125, // 1.25x (125% - 120 DPI)
    Scale150, // 1.50x (150% - 144 DPI)
    Scale200, // 2.00x (200% - 192 DPI Retina/4K)
}

impl DpiScale {
    /// Floating-point multiplier
    pub fn factor(&self) -> f32 {
        match self {
            DpiScale::Scale100 => 1.0,
            DpiScale::Scale125 => 1.25,
            DpiScale::Scale150 => 1.5,
            DpiScale::Scale200 => 2.0,
        }
    }

    /// Fixed-point permille (1000 = 1.00x) for deterministic integer arithmetic
    pub const fn permille(&self) -> u32 {
        match self {
            DpiScale::Scale100 => 1000,
            DpiScale::Scale125 => 1250,
            DpiScale::Scale150 => 1500,
            DpiScale::Scale200 => 2000,
        }
    }

    /// Integer scale multiplier (1 for 100..150%, 2 for 200%)
    pub const fn integer_scale(&self) -> u32 {
        match self {
            DpiScale::Scale200 => 2,
            _ => 1,
        }
    }

    /// UI display percentage integer (100, 125, 150, 200)
    pub const fn percent(&self) -> u32 {
        match self {
            DpiScale::Scale100 => 100,
            DpiScale::Scale125 => 125,
            DpiScale::Scale150 => 150,
            DpiScale::Scale200 => 200,
        }
    }

    /// Human-readable label
    pub const fn label(&self) -> &'static str {
        match self {
            DpiScale::Scale100 => "100%",
            DpiScale::Scale125 => "125%",
            DpiScale::Scale150 => "150%",
            DpiScale::Scale200 => "200% (Retina)",
        }
    }

    /// Auto-detects optimal scale factor from physical DPI
    pub fn from_dpi(dpi: f32) -> Self {
        if dpi >= 180.0 {
            DpiScale::Scale200
        } else if dpi >= 135.0 {
            DpiScale::Scale150
        } else if dpi >= 110.0 {
            DpiScale::Scale125
        } else {
            DpiScale::Scale100
        }
    }

    /// Selects scale from float factor
    pub fn from_factor(factor: f32) -> Self {
        if factor >= 1.75 {
            DpiScale::Scale200
        } else if factor >= 1.375 {
            DpiScale::Scale150
        } else if factor >= 1.125 {
            DpiScale::Scale125
        } else {
            DpiScale::Scale100
        }
    }

    /// Selects scale from percent value (100, 125, 150, 200)
    pub fn from_percent(pct: u32) -> Self {
        if pct >= 175 {
            DpiScale::Scale200
        } else if pct >= 135 {
            DpiScale::Scale150
        } else if pct >= 110 {
            DpiScale::Scale125
        } else {
            DpiScale::Scale100
        }
    }

    // -------------------------------------------------------------------------
    // Coordinate Mapping & Scaling
    // -------------------------------------------------------------------------

    /// Converts logical points to physical device pixels
    pub fn to_physical(&self, logical: i32) -> i32 {
        ((logical as i64 * self.permille() as i64) / 1000) as i32
    }

    /// Converts physical device pixels to logical points
    pub fn to_logical(&self, physical: i32) -> i32 {
        ((physical as i64 * 1000) / self.permille() as i64) as i32
    }

    /// Scales a 2D Point from logical space to physical pixel space
    pub fn scale_pt(&self, pt: Point) -> Point {
        Point::new(self.to_physical(pt.x), self.to_physical(pt.y))
    }

    /// Unscales a 2D Point from physical pixel space to logical space
    pub fn unscale_pt(&self, pt: Point) -> Point {
        Point::new(self.to_logical(pt.x), self.to_logical(pt.y))
    }

    /// Scales a 2D Size from logical space to physical pixel space
    pub fn scale_size(&self, size: Size) -> Size {
        Size::new(
            self.to_physical(size.width),
            self.to_physical(size.height),
        )
    }

    /// Unscales a 2D Size from physical pixel space to logical space
    pub fn unscale_size(&self, size: Size) -> Size {
        Size::new(
            self.to_logical(size.width),
            self.to_logical(size.height),
        )
    }

    /// Scales a Rectangle with conservative expansion (floor for origin, ceil for extents)
    pub fn scale_rect(&self, r: Rect) -> Rect {
        let x0 = ((r.x as i64 * self.permille() as i64) / 1000) as i32;
        let y0 = ((r.y as i64 * self.permille() as i64) / 1000) as i32;
        let x1 = (((r.x + r.width) as i64 * self.permille() as i64 + 999) / 1000) as i32;
        let y1 = (((r.y + r.height) as i64 * self.permille() as i64 + 999) / 1000) as i32;

        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }

    /// Unscales a Rectangle from physical pixel space to logical space
    pub fn unscale_rect(&self, r: Rect) -> Rect {
        let x0 = ((r.x as i64 * 1000) / self.permille() as i64) as i32;
        let y0 = ((r.y as i64 * 1000) / self.permille() as i64) as i32;
        let x1 = (((r.x + r.width) as i64 * 1000) / self.permille() as i64) as i32;
        let y1 = (((r.y + r.height) as i64 * 1000) / self.permille() as i64) as i32;

        Rect::new(x0, y0, (x1 - x0).max(0), (y1 - y0).max(0))
    }

    /// Scales EdgeInsets (padding/margin)
    pub fn scale_insets(&self, insets: EdgeInsets) -> EdgeInsets {
        EdgeInsets {
            top: self.to_physical(insets.top),
            right: self.to_physical(insets.right),
            bottom: self.to_physical(insets.bottom),
            left: self.to_physical(insets.left),
        }
    }

    /// Scales a Flexbox Dimension
    pub fn scale_dimension(&self, dim: Dimension) -> Dimension {
        match dim {
            Dimension::Points(p) => Dimension::Points(self.to_physical(p)),
            other => other,
        }
    }

    /// Scales an entire FlexStyle (gap, padding, margin, width, height)
    pub fn scale_flex_style(&self, style: &FlexStyle) -> FlexStyle {
        let mut scaled = style.clone();
        scaled.gap = self.to_physical(style.gap);
        scaled.padding = self.scale_insets(style.padding);
        scaled.margin = self.scale_insets(style.margin);
        scaled.width = self.scale_dimension(style.width);
        scaled.height = self.scale_dimension(style.height);
        scaled.min_width = style.min_width.map(|v| self.to_physical(v));
        scaled.max_width = style.max_width.map(|v| self.to_physical(v));
        scaled.min_height = style.min_height.map(|v| self.to_physical(v));
        scaled.max_height = style.max_height.map(|v| self.to_physical(v));
        scaled.flex_basis = style.flex_basis.map(|v| self.to_physical(v));
        scaled
    }

    /// Scales font point size to physical pixels
    pub fn scale_font_px(&self, font_size_pt: f32) -> f32 {
        (font_size_pt * BASELINE_DPI / 72.0) * self.factor()
    }
}

// =============================================================================
// Display Metrics & EDID Integration
// =============================================================================

#[derive(Debug, Clone, Copy)]
pub struct DisplayMetrics {
    pub width_px: u32,
    pub height_px: u32,
    pub width_mm: u32,
    pub height_mm: u32,
    pub dpi_x: f32,
    pub dpi_y: f32,
    pub dpi: f32,
    pub diagonal_inches: f32,
    pub recommended_scale: DpiScale,
}

impl DisplayMetrics {
    pub fn new(width_px: u32, height_px: u32, width_mm: u32, height_mm: u32) -> Self {
        let (dpi_x, dpi_y, dpi, diag) = if width_mm > 0 && height_mm > 0 && width_px > 0 && height_px > 0 {
            let dx = (width_px as f32 / width_mm as f32) * 25.4;
            let dy = (height_px as f32 / height_mm as f32) * 25.4;
            let d_mean = (dx + dy) / 2.0;

            let w_in = width_mm as f32 / 25.4;
            let h_in = height_mm as f32 / 25.4;
            let diag_in = crate::render::fast_sqrt(w_in * w_in + h_in * h_in);
            (dx, dy, d_mean, diag_in)
        } else {
            (BASELINE_DPI, BASELINE_DPI, BASELINE_DPI, 24.0)
        };

        let recommended_scale = DpiScale::from_dpi(dpi);

        Self {
            width_px,
            height_px,
            width_mm,
            height_mm,
            dpi_x,
            dpi_y,
            dpi,
            diagonal_inches: diag,
            recommended_scale,
        }
    }
}

// =============================================================================
// Scalable Vector Icon Engine
// =============================================================================

/// Declarative vector icons rendered with pixel-perfect anti-aliased sharpness
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum VectorIcon {
    Close,
    Minimize,
    Maximize,
    Restore,
    ChevronDown,
    ChevronUp,
    ChevronLeft,
    ChevronRight,
    Checkmark,
    Search,
    Settings,
    Folder,
    File,
    Monitor,
    Terminal,
    Wifi,
    Battery,
}

/// Helper to draw a thick anti-aliased line segment onto a `BufferSurface`
fn draw_midpoint_circle(surface: &mut BufferSurface, cx: i32, cy: i32, radius: i32, color: Color) {
    let mut x = radius;
    let mut y = 0;
    let mut err = 0;

    while x >= y {
        surface.set_pixel(cx + x, cy + y, color);
        surface.set_pixel(cx + y, cy + x, color);
        surface.set_pixel(cx - y, cy + x, color);
        surface.set_pixel(cx - x, cy + y, color);
        surface.set_pixel(cx - x, cy - y, color);
        surface.set_pixel(cx - y, cy - x, color);
        surface.set_pixel(cx + y, cy - x, color);
        surface.set_pixel(cx + x, cy - y, color);

        if err <= 0 {
            y += 1;
            err += 2 * y + 1;
        }
        if err > 0 {
            x -= 1;
            err -= 2 * x + 1;
        }
    }
}

fn draw_upper_arc(surface: &mut BufferSurface, cx: i32, cy: i32, radius: i32, color: Color) {
    let mut x = radius;
    let mut y = 0;
    let mut err = 0;

    while x >= y {
        surface.set_pixel(cx - x, cy - y, color);
        surface.set_pixel(cx - y, cy - x, color);
        surface.set_pixel(cx + y, cy - x, color);
        surface.set_pixel(cx + x, cy - y, color);

        if err <= 0 {
            y += 1;
            err += 2 * y + 1;
        }
        if err > 0 {
            x -= 1;
            err -= 2 * x + 1;
        }
    }
}

fn draw_aa_line(surface: &mut BufferSurface, x0: i32, y0: i32, x1: i32, y1: i32, stroke: i32, color: Color) {
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx - dy;

    let half_stroke = stroke / 2;
    let mut cur_x = x0;
    let mut cur_y = y0;

    loop {
        // Draw brush footprint
        for oy in -half_stroke..=half_stroke {
            for ox in -half_stroke..=half_stroke {
                if ox * ox + oy * oy <= half_stroke * half_stroke + 1 {
                    surface.set_pixel(cur_x + ox, cur_y + oy, color);
                }
            }
        }

        if cur_x == x1 && cur_y == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 > -dy {
            err -= dy;
            cur_x += sx;
        }
        if e2 < dx {
            err += dx;
            cur_y += sy;
        }
    }
}

/// Renders a native scalable vector icon at target bounds and DPI scale factor
pub fn render_vector_icon(
    surface: &mut BufferSurface,
    icon: VectorIcon,
    bounds: Rect,
    scale: DpiScale,
    color: Color,
) {
    let stroke = (scale.factor() * 1.5).max(1.0) as i32;
    let w = bounds.width;
    let h = bounds.height;
    if w <= 0 || h <= 0 {
        return;
    }

    let cx = bounds.x + w / 2;
    let cy = bounds.y + h / 2;

    match icon {
        VectorIcon::Close => {
            // Anti-aliased diagonal 'X'
            let pad = (w.min(h) * 22 / 100).max(2);
            let x0 = bounds.x + pad;
            let y0 = bounds.y + pad;
            let x1 = bounds.x + w - pad;
            let y1 = bounds.y + h - pad;

            draw_aa_line(surface, x0, y0, x1, y1, stroke, color);
            draw_aa_line(surface, x0, y1, x1, y0, stroke, color);
        }

        VectorIcon::Minimize => {
            // Horizontal line '-'
            let pad = (w * 20 / 100).max(2);
            let line_y = cy + (h * 15 / 100);
            draw_aa_line(surface, bounds.x + pad, line_y, bounds.x + w - pad, line_y, stroke, color);
        }

        VectorIcon::Maximize => {
            // Rectangular outline box
            let pad = (w.min(h) * 20 / 100).max(2);
            let rect = Rect::new(bounds.x + pad, bounds.y + pad, w - 2 * pad, h - 2 * pad);
            let x0 = rect.x;
            let y0 = rect.y;
            let x1 = rect.x + rect.width;
            let y1 = rect.y + rect.height;

            draw_aa_line(surface, x0, y0, x1, y0, stroke, color);
            draw_aa_line(surface, x1, y0, x1, y1, stroke, color);
            draw_aa_line(surface, x1, y1, x0, y1, stroke, color);
            draw_aa_line(surface, x0, y1, x0, y0, stroke, color);
        }

        VectorIcon::Restore => {
            // Dual overlapping window frames
            let pad = (w.min(h) * 18 / 100).max(2);
            let offset = (w * 15 / 100).max(2);
            let fw = w - 2 * pad - offset;
            let fh = h - 2 * pad - offset;

            // Back frame
            let bx = bounds.x + pad + offset;
            let by = bounds.y + pad;
            draw_aa_line(surface, bx, by, bx + fw, by, stroke, color);
            draw_aa_line(surface, bx + fw, by, bx + fw, by + fh, stroke, color);

            // Front frame
            let fx = bounds.x + pad;
            let fy = bounds.y + pad + offset;
            draw_aa_line(surface, fx, fy, fx + fw, fy, stroke, color);
            draw_aa_line(surface, fx + fw, fy, fx + fw, fy + fh, stroke, color);
            draw_aa_line(surface, fx + fw, fy + fh, fx, fy + fh, stroke, color);
            draw_aa_line(surface, fx, fy + fh, fx, fy, stroke, color);
        }

        VectorIcon::ChevronDown => {
            let arm = (w * 25 / 100).max(3);
            let y_tip = cy + arm / 2;
            let y_base = cy - arm / 2;
            draw_aa_line(surface, cx - arm, y_base, cx, y_tip, stroke, color);
            draw_aa_line(surface, cx, y_tip, cx + arm, y_base, stroke, color);
        }

        VectorIcon::ChevronUp => {
            let arm = (w * 25 / 100).max(3);
            let y_tip = cy - arm / 2;
            let y_base = cy + arm / 2;
            draw_aa_line(surface, cx - arm, y_base, cx, y_tip, stroke, color);
            draw_aa_line(surface, cx, y_tip, cx + arm, y_base, stroke, color);
        }

        VectorIcon::ChevronLeft => {
            let arm = (h * 25 / 100).max(3);
            let x_tip = cx - arm / 2;
            let x_base = cx + arm / 2;
            draw_aa_line(surface, x_base, cy - arm, x_tip, cy, stroke, color);
            draw_aa_line(surface, x_tip, cy, x_base, cy + arm, stroke, color);
        }

        VectorIcon::ChevronRight => {
            let arm = (h * 25 / 100).max(3);
            let x_tip = cx + arm / 2;
            let x_base = cx - arm / 2;
            draw_aa_line(surface, x_base, cy - arm, x_tip, cy, stroke, color);
            draw_aa_line(surface, x_tip, cy, x_base, cy + arm, stroke, color);
        }

        VectorIcon::Checkmark => {
            // Dynamic checkmark
            let p0 = Point::new(cx - (w * 25 / 100), cy);
            let p1 = Point::new(cx - (w * 5 / 100), cy + (h * 22 / 100));
            let p2 = Point::new(cx + (w * 28 / 100), cy - (h * 20 / 100));

            let chk_stroke = (scale.factor() * 1.8).max(1.0) as i32;
            draw_aa_line(surface, p0.x, p0.y, p1.x, p1.y, chk_stroke, color);
            draw_aa_line(surface, p1.x, p1.y, p2.x, p2.y, chk_stroke, color);
        }

        VectorIcon::Search => {
            // Circle + diagonal handle
            let radius = (w.min(h) * 22 / 100).max(3);
            let center_x = cx - radius / 3;
            let center_y = cy - radius / 3;

            // Draw circle outline
            draw_midpoint_circle(surface, center_x, center_y, radius, color);

            // Handle
            let h_x0 = center_x + (radius * 70 / 100);
            let h_y0 = center_y + (radius * 70 / 100);
            let h_x1 = h_x0 + radius;
            let h_y1 = h_y0 + radius;
            draw_aa_line(surface, h_x0, h_y0, h_x1, h_y1, stroke + 1, color);
        }

        VectorIcon::Settings => {
            // Gear / Cog: central circle + inner hole + 8 teeth (4 crossed axes)
            let radius = (w.min(h) * 20 / 100).max(4);
            let inner_r = (radius * 45 / 100).max(2);
            let r_teeth = radius + (radius * 35 / 100).max(2);

            draw_midpoint_circle(surface, cx, cy, radius, color);
            draw_midpoint_circle(surface, cx, cy, inner_r, color);

            // Vertical tooth line
            draw_aa_line(surface, cx, cy - r_teeth, cx, cy + r_teeth, stroke, color);
            // Horizontal tooth line
            draw_aa_line(surface, cx - r_teeth, cy, cx + r_teeth, cy, stroke, color);

            // 45-degree diagonal teeth
            let d = (r_teeth * 707) / 1000;
            draw_aa_line(surface, cx - d, cy - d, cx + d, cy + d, stroke, color);
            draw_aa_line(surface, cx - d, cy + d, cx + d, cy - d, stroke, color);
        }

        VectorIcon::Folder => {
            // Folder outline with tab
            let pad = (w * 18 / 100).max(2);
            let x0 = bounds.x + pad;
            let y0 = bounds.y + pad + (h * 10 / 100);
            let x1 = bounds.x + w - pad;
            let y1 = bounds.y + h - pad;

            // Tab
            let tab_w = (x1 - x0) * 40 / 100;
            let tab_h = (h * 12 / 100).max(2);
            draw_aa_line(surface, x0, y0, x0 + tab_w, y0, stroke, color);
            draw_aa_line(surface, x0 + tab_w, y0, x0 + tab_w + tab_h, y0 + tab_h, stroke, color);

            // Body
            draw_aa_line(surface, x0, y0 + tab_h, x1, y0 + tab_h, stroke, color);
            draw_aa_line(surface, x1, y0 + tab_h, x1, y1, stroke, color);
            draw_aa_line(surface, x1, y1, x0, y1, stroke, color);
            draw_aa_line(surface, x0, y1, x0, y0, stroke, color);
        }

        VectorIcon::File => {
            // Document with folded top-right corner
            let pad_x = (w * 22 / 100).max(2);
            let pad_y = (h * 16 / 100).max(2);
            let x0 = bounds.x + pad_x;
            let y0 = bounds.y + pad_y;
            let x1 = bounds.x + w - pad_x;
            let y1 = bounds.y + h - pad_y;

            let fold = (x1 - x0) * 35 / 100;
            // Top edge to fold
            draw_aa_line(surface, x0, y0, x1 - fold, y0, stroke, color);
            // Diagonal fold
            draw_aa_line(surface, x1 - fold, y0, x1, y0 + fold, stroke, color);
            // Right edge
            draw_aa_line(surface, x1, y0 + fold, x1, y1, stroke, color);
            // Bottom edge
            draw_aa_line(surface, x1, y1, x0, y1, stroke, color);
            // Left edge
            draw_aa_line(surface, x0, y1, x0, y0, stroke, color);
        }

        VectorIcon::Monitor => {
            // Monitor frame + stand + base
            let pad_x = (w * 16 / 100).max(2);
            let pad_y = (h * 18 / 100).max(2);
            let screen_h = (h - 2 * pad_y) * 72 / 100;

            let x0 = bounds.x + pad_x;
            let y0 = bounds.y + pad_y;
            let x1 = bounds.x + w - pad_x;
            let y1 = y0 + screen_h;

            // Screen frame
            draw_aa_line(surface, x0, y0, x1, y0, stroke, color);
            draw_aa_line(surface, x1, y0, x1, y1, stroke, color);
            draw_aa_line(surface, x1, y1, x0, y1, stroke, color);
            draw_aa_line(surface, x0, y1, x0, y0, stroke, color);

            // Stand
            let stand_y1 = bounds.y + h - pad_y;
            draw_aa_line(surface, cx, y1, cx, stand_y1, stroke, color);

            // Base
            let base_w = (w * 28 / 100).max(4);
            draw_aa_line(surface, cx - base_w, stand_y1, cx + base_w, stand_y1, stroke, color);
        }

        VectorIcon::Terminal => {
            // Window frame + prompt '>_'
            let pad = (w.min(h) * 16 / 100).max(2);
            let x0 = bounds.x + pad;
            let y0 = bounds.y + pad;
            let x1 = bounds.x + w - pad;
            let y1 = bounds.y + h - pad;

            draw_aa_line(surface, x0, y0, x1, y0, stroke, color);
            draw_aa_line(surface, x1, y0, x1, y1, stroke, color);
            draw_aa_line(surface, x1, y1, x0, y1, stroke, color);
            draw_aa_line(surface, x0, y1, x0, y0, stroke, color);

            // Prompt arrow '>'
            let a_x0 = x0 + (w * 15 / 100);
            let a_y0 = cy - (h * 12 / 100);
            let a_xm = a_x0 + (w * 12 / 100);
            let a_y1 = cy + (h * 12 / 100);
            draw_aa_line(surface, a_x0, a_y0, a_xm, cy, stroke, color);
            draw_aa_line(surface, a_xm, cy, a_x0, a_y1, stroke, color);

            // Underscore '_'
            let u_x0 = a_xm + (w * 8 / 100);
            let u_x1 = u_x0 + (w * 15 / 100);
            draw_aa_line(surface, u_x0, a_y1, u_x1, a_y1, stroke, color);
        }

        VectorIcon::Wifi => {
            // Concentric radio arcs
            let base_y = bounds.y + h - (h * 20 / 100);
            for r_step in [1, 2, 3] {
                let r = (w * r_step * 12 / 100).max(3);
                draw_upper_arc(surface, cx, base_y, r, color);
            }
            // Dot at base center
            surface.set_pixel(cx, base_y - 2, color);
        }

        VectorIcon::Battery => {
            // Battery body + terminal nipple
            let pad_x = (w * 16 / 100).max(2);
            let pad_y = (h * 25 / 100).max(2);
            let nipple_w = (w * 6 / 100).max(2);

            let x0 = bounds.x + pad_x;
            let y0 = bounds.y + pad_y;
            let x1 = bounds.x + w - pad_x - nipple_w;
            let y1 = bounds.y + h - pad_y;

            // Housing
            draw_aa_line(surface, x0, y0, x1, y0, stroke, color);
            draw_aa_line(surface, x1, y0, x1, y1, stroke, color);
            draw_aa_line(surface, x1, y1, x0, y1, stroke, color);
            draw_aa_line(surface, x0, y1, x0, y0, stroke, color);

            // Nipple
            let n_h = (y1 - y0) * 40 / 100;
            let ny0 = cy - n_h / 2;
            let ny1 = cy + n_h / 2;
            draw_aa_line(surface, x1, ny0, x1 + nipple_w, ny0, stroke, color);
            draw_aa_line(surface, x1 + nipple_w, ny0, x1 + nipple_w, ny1, stroke, color);
            draw_aa_line(surface, x1 + nipple_w, ny1, x1, ny1, stroke, color);

            // Fill partial charge indicator
            let fill_w = (x1 - x0 - 4) * 65 / 100;
            surface.fill_rect(Rect::new(x0 + 2, y0 + 2, fill_w, y1 - y0 - 4), color);
        }
    }
}

// =============================================================================
// Unit Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{BLACK, WHITE};

    #[test]
    fn test_scale_conversions_and_factors() {
        assert_eq!(DpiScale::Scale100.factor(), 1.0);
        assert_eq!(DpiScale::Scale125.factor(), 1.25);
        assert_eq!(DpiScale::Scale150.factor(), 1.5);
        assert_eq!(DpiScale::Scale200.factor(), 2.0);

        assert_eq!(DpiScale::Scale100.percent(), 100);
        assert_eq!(DpiScale::Scale125.percent(), 125);
        assert_eq!(DpiScale::Scale150.percent(), 150);
        assert_eq!(DpiScale::Scale200.percent(), 200);

        assert_eq!(DpiScale::from_dpi(92.0), DpiScale::Scale100);
        assert_eq!(DpiScale::from_dpi(120.0), DpiScale::Scale125);
        assert_eq!(DpiScale::from_dpi(163.0), DpiScale::Scale150);
        assert_eq!(DpiScale::from_dpi(331.0), DpiScale::Scale200);
    }

    #[test]
    fn test_point_and_rect_scaling() {
        let s200 = DpiScale::Scale200;
        let pt = Point::new(40, 60);
        let scaled_pt = s200.scale_pt(pt);
        assert_eq!(scaled_pt.x, 80);
        assert_eq!(scaled_pt.y, 120);
        assert_eq!(s200.unscale_pt(scaled_pt), pt);

        let rect = Rect::new(10, 20, 100, 50);
        let scaled_rect = s200.scale_rect(rect);
        assert_eq!(scaled_rect.x, 20);
        assert_eq!(scaled_rect.y, 40);
        assert_eq!(scaled_rect.width, 200);
        assert_eq!(scaled_rect.height, 100);
        assert_eq!(s200.unscale_rect(scaled_rect), rect);
    }

    #[test]
    fn test_fractional_125_and_150_scaling() {
        let s125 = DpiScale::Scale125;
        assert_eq!(s125.to_physical(100), 125);
        assert_eq!(s125.to_logical(125), 100);

        let s150 = DpiScale::Scale150;
        assert_eq!(s150.to_physical(100), 150);
        assert_eq!(s150.to_logical(150), 100);
    }

    #[test]
    fn test_vector_icons_rasterization() {
        let mut surface = BufferSurface::new(64, 64);
        surface.clear(BLACK);

        let test_icons = [
            VectorIcon::Close,
            VectorIcon::Minimize,
            VectorIcon::Maximize,
            VectorIcon::Checkmark,
            VectorIcon::Search,
            VectorIcon::Settings,
            VectorIcon::Monitor,
            VectorIcon::Terminal,
        ];

        for icon in test_icons {
            surface.clear(BLACK);
            render_vector_icon(
                &mut surface,
                icon,
                Rect::new(8, 8, 48, 48),
                DpiScale::Scale200,
                WHITE,
            );

            // Assert that some pixels were drawn on surface
            let mut drawn = 0;
            for y in 0..64 {
                for x in 0..64 {
                    if surface.get_pixel(x, y) != BLACK {
                        drawn += 1;
                    }
                }
            }
            assert!(drawn > 10, "Icon {:?} must produce rendered pixels", icon);
        }
    }

    #[test]
    fn test_display_metrics_calculation() {
        // 27" 4K: 3840x2160, 597mm x 336mm
        let m = DisplayMetrics::new(3840, 2160, 597, 336);
        assert!(m.dpi > 160.0 && m.dpi < 165.0);
        assert_eq!(m.recommended_scale, DpiScale::Scale150);
        assert!(m.diagonal_inches > 26.5 && m.diagonal_inches < 27.5);
    }
}
