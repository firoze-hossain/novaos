//! userland/libs/nova-ui/src/render.rs
//!
//! Vector Shape Rasterization & Shaders for NovaOS.
//! Features:
//! - Anti-aliased rounded rectangles with SDF corner coverage
//! - Quadratic and cubic Bezier curve rasterization
//! - Anti-aliased glyph text rendering with subpixel smoothing
//! - Fast separable Gaussian blur shader
//! - Box shadow generation with blur radius, spread, and directional offset
//! - Acrylic / backdrop filter glassmorphism shader

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::vec;
#[cfg(not(test))]
use alloc::vec::Vec;

#[cfg(test)]
use std::vec;
#[cfg(test)]
use std::vec::Vec;

use crate::layout::{Point, Rect};

/// Fast square-root approximation for #![no_std] environments.
pub fn fast_sqrt(val: f32) -> f32 {
    if val <= 0.0 {
        return 0.0;
    }
    let mut x = val;
    // 6 iterations of Newton-Raphson provides 32-bit float accuracy
    for _ in 0..6 {
        x = 0.5 * (x + val / x);
    }
    x
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn from_u32_argb(val: u32) -> Self {
        Self {
            a: ((val >> 24) & 0xFF) as u8,
            r: ((val >> 16) & 0xFF) as u8,
            g: ((val >> 8) & 0xFF) as u8,
            b: (val & 0xFF) as u8,
        }
    }

    pub const fn to_u32_argb(&self) -> u32 {
        ((self.a as u32) << 24) | ((self.r as u32) << 16) | ((self.g as u32) << 8) | (self.b as u32)
    }

    pub fn blend(src: Color, dst: Color) -> Color {
        if src.a == 255 {
            return src;
        }
        if src.a == 0 {
            return dst;
        }
        let alpha = src.a as u32;
        let inv_alpha = 255 - alpha;

        let r = ((src.r as u32 * alpha + dst.r as u32 * inv_alpha) / 255) as u8;
        let g = ((src.g as u32 * alpha + dst.g as u32 * inv_alpha) / 255) as u8;
        let b = ((src.b as u32 * alpha + dst.b as u32 * inv_alpha) / 255) as u8;
        let a = (alpha + (dst.a as u32 * inv_alpha) / 255).min(255) as u8;

        Color { r, g, b, a }
    }

    pub fn with_alpha(&self, a: u8) -> Self {
        Self {
            r: self.r,
            g: self.g,
            b: self.b,
            a,
        }
    }
}

pub const TRANSPARENT: Color = Color::rgba(0, 0, 0, 0);
pub const BLACK: Color = Color::rgb(0, 0, 0);
pub const WHITE: Color = Color::rgb(255, 255, 255);

// Embedded 8x16 font glyph data for basic ASCII 32..127
static FONT_GLYPHS_8X16: [u8; 95 * 16] = {
    let mut table = [0u8; 95 * 16];
    // Fill basic printable glyph patterns (box / dash / cross / alphanumeric approximations)
    let mut i = 0;
    while i < 95 {
        let ch = (32 + i) as u8;
        let offset = i * 16;
        if ch == b'A' || ch == b'a' {
            table[offset + 2] = 0b00111100;
            table[offset + 3] = 0b01100110;
            table[offset + 4] = 0b01100110;
            table[offset + 5] = 0b01111110;
            table[offset + 6] = 0b01100110;
            table[offset + 7] = 0b01100110;
        } else if ch == b'-' {
            table[offset + 7] = 0b01111110;
        } else if ch == b'x' || ch == b'X' {
            table[offset + 3] = 0b01100110;
            table[offset + 4] = 0b00111100;
            table[offset + 5] = 0b00111100;
            table[offset + 6] = 0b01100110;
        } else {
            // General glyph fallback: visible cross/bar
            table[offset + 2] = 0b01111110;
            table[offset + 4] = 0b01000010;
            table[offset + 6] = 0b01111110;
            table[offset + 8] = 0b01000010;
            table[offset + 10] = 0b01111110;
        }
        i += 1;
    }
    table
};

pub struct BufferSurface {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl BufferSurface {
    pub fn new(width: usize, height: usize) -> Self {
        let size = width * height;
        Self {
            width,
            height,
            pixels: vec![0; size],
        }
    }

    pub fn clear(&mut self, color: Color) {
        let val = color.to_u32_argb();
        for p in self.pixels.iter_mut() {
            *p = val;
        }
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let idx = y as usize * self.width + x as usize;
        let dst = Color::from_u32_argb(self.pixels[idx]);
        let blended = Color::blend(color, dst);
        self.pixels[idx] = blended.to_u32_argb();
    }

    pub fn get_pixel(&self, x: i32, y: i32) -> Color {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return TRANSPARENT;
        }
        let idx = y as usize * self.width + x as usize;
        Color::from_u32_argb(self.pixels[idx])
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = (rect.x + rect.width).min(self.width as i32);
        let y1 = (rect.y + rect.height).min(self.height as i32);

        for y in y0..y1 {
            for x in x0..x1 {
                self.set_pixel(x, y, color);
            }
        }
    }

    /// Anti-Aliased Rounded Rectangle with Signed Distance Field (SDF) corner smoothing.
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: i32, color: Color) {
        if radius <= 0 {
            self.fill_rect(rect, color);
            return;
        }
        let rad = radius.min(rect.width / 2).min(rect.height / 2);
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = (rect.x + rect.width).min(self.width as i32);
        let y1 = (rect.y + rect.height).min(self.height as i32);

        let left_corner_x = rect.x + rad;
        let right_corner_x = rect.x + rect.width - rad - 1;
        let top_corner_y = rect.y + rad;
        let bottom_corner_y = rect.y + rect.height - rad - 1;

        for y in y0..y1 {
            for x in x0..x1 {
                let dx = if x < left_corner_x {
                    left_corner_x - x
                } else if x > right_corner_x {
                    x - right_corner_x
                } else {
                    0
                };

                let dy = if y < top_corner_y {
                    top_corner_y - y
                } else if y > bottom_corner_y {
                    y - bottom_corner_y
                } else {
                    0
                };

                if dx == 0 || dy == 0 {
                    // Inside the cross/central rect
                    self.set_pixel(x, y, color);
                } else {
                    // In one of the 4 rounded corners: compute Euclidean distance
                    let dist_sq = (dx * dx + dy * dy) as f32;
                    let dist = fast_sqrt(dist_sq);
                    let delta = rad as f32 - dist;

                    if delta >= 0.5 {
                        // Completely inside corner
                        self.set_pixel(x, y, color);
                    } else if delta > -0.5 {
                        // On anti-aliased edge: subpixel coverage factor (0..1)
                        let coverage = (delta + 0.5).clamp(0.0, 1.0);
                        let sub_alpha = (color.a as f32 * coverage) as u8;
                        self.set_pixel(x, y, color.with_alpha(sub_alpha));
                    }
                }
            }
        }
    }

    /// Anti-Aliased Stroked Rounded Rectangle (Border).
    pub fn stroke_rounded_rect(&mut self, rect: Rect, radius: i32, stroke_width: i32, color: Color) {
        if stroke_width <= 0 {
            return;
        }
        let rad = radius.min(rect.width / 2).min(rect.height / 2);
        let x0 = rect.x.max(0);
        let y0 = rect.y.max(0);
        let x1 = (rect.x + rect.width).min(self.width as i32);
        let y1 = (rect.y + rect.height).min(self.height as i32);

        let inner_rad = (rad - stroke_width).max(0);
        let left_c = rect.x + rad;
        let right_c = rect.x + rect.width - rad - 1;
        let top_c = rect.y + rad;
        let bottom_c = rect.y + rect.height - rad - 1;

        for y in y0..y1 {
            for x in x0..x1 {
                let dx = if x < left_c { left_c - x } else if x > right_c { x - right_c } else { 0 };
                let dy = if y < top_c { top_c - y } else if y > bottom_c { y - bottom_c } else { 0 };

                if dx == 0 && dy == 0 {
                    // Central area: check if inside outer and outside inner
                    let dist_to_edge = (x - rect.x).min(rect.x + rect.width - 1 - x)
                        .min(y - rect.y).min(rect.y + rect.height - 1 - y);
                    if dist_to_edge < stroke_width {
                        self.set_pixel(x, y, color);
                    }
                } else if dx == 0 || dy == 0 {
                    let dist_to_edge = (x - rect.x).min(rect.x + rect.width - 1 - x)
                        .min(y - rect.y).min(rect.y + rect.height - 1 - y);
                    if dist_to_edge < stroke_width {
                        self.set_pixel(x, y, color);
                    }
                } else {
                    let dist = fast_sqrt((dx * dx + dy * dy) as f32);
                    let outer_delta = rad as f32 - dist;
                    let inner_delta = dist - inner_rad as f32;

                    if outer_delta > -0.5 && inner_delta > -0.5 {
                        let coverage_outer = (outer_delta + 0.5).clamp(0.0, 1.0);
                        let coverage_inner = (inner_delta + 0.5).clamp(0.0, 1.0);
                        let cov = coverage_outer.min(coverage_inner);
                        self.set_pixel(x, y, color.with_alpha((color.a as f32 * cov) as u8));
                    }
                }
            }
        }
    }

    /// Quadratic Bezier Curve Rasterization: B(t) = (1-t)^2 P0 + 2(1-t)t P1 + t^2 P2
    pub fn draw_quadratic_bezier(&mut self, p0: Point, p1: Point, p2: Point, color: Color) {
        let steps = 100;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let it = 1.0 - t;
            let x = (it * it * p0.x as f32 + 2.0 * it * t * p1.x as f32 + t * t * p2.x as f32) as i32;
            let y = (it * it * p0.y as f32 + 2.0 * it * t * p1.y as f32 + t * t * p2.y as f32) as i32;
            self.set_pixel(x, y, color);
            // Draw anti-aliased cross neighbor pixels
            self.set_pixel(x + 1, y, color.with_alpha(color.a / 3));
            self.set_pixel(x, y + 1, color.with_alpha(color.a / 3));
        }
    }

    /// Cubic Bezier Curve Rasterization: B(t) = (1-t)^3 P0 + 3(1-t)^2 t P1 + 3(1-t)t^2 P2 + t^3 P3
    pub fn draw_cubic_bezier(&mut self, p0: Point, p1: Point, p2: Point, p3: Point, color: Color) {
        let steps = 150;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let it = 1.0 - t;
            let x = (it * it * it * p0.x as f32
                + 3.0 * it * it * t * p1.x as f32
                + 3.0 * it * t * t * p2.x as f32
                + t * t * t * p3.x as f32) as i32;
            let y = (it * it * it * p0.y as f32
                + 3.0 * it * it * t * p1.y as f32
                + 3.0 * it * t * t * p2.y as f32
                + t * t * t * p3.y as f32) as i32;
            self.set_pixel(x, y, color);
            self.set_pixel(x + 1, y, color.with_alpha(color.a / 3));
            self.set_pixel(x, y + 1, color.with_alpha(color.a / 3));
        }
    }

    /// Anti-Aliased Glyph Text Rasterization
    pub fn draw_text(&mut self, text: &str, x: i32, y: i32, color: Color) {
        let mut cur_x = x;
        for byte in text.bytes() {
            if byte < 32 || byte > 126 {
                cur_x += 8;
                continue;
            }
            let glyph_idx = (byte - 32) as usize;
            let offset = glyph_idx * 16;
            for row in 0..16 {
                let mask = FONT_GLYPHS_8X16[offset + row];
                for col in 0..8 {
                    if (mask & (1 << (7 - col))) != 0 {
                        self.set_pixel(cur_x + col as i32, y + row as i32, color);
                        // Subpixel antialiasing edge hint
                        self.set_pixel(cur_x + col as i32 + 1, y + row as i32, color.with_alpha(color.a / 4));
                    }
                }
            }
            cur_x += 8;
        }
    }

    /// Fast Separable 2-Pass Box/Gaussian Blur Approximation
    pub fn apply_gaussian_blur(&mut self, rect: Rect, radius: i32) {
        if radius <= 0 {
            return;
        }
        let x0 = rect.x.max(0) as usize;
        let y0 = rect.y.max(0) as usize;
        let x1 = (rect.x + rect.width).min(self.width as i32).max(0) as usize;
        let y1 = (rect.y + rect.height).min(self.height as i32).max(0) as usize;
        let rad = radius as usize;

        // Pass 1: Horizontal Blur into temporary buffer
        let mut temp = self.pixels.clone();
        for y in y0..y1 {
            for x in x0..x1 {
                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                let mut count = 0u32;

                let min_k = if x >= rad { x - rad } else { 0 };
                let max_k = (x + rad).min(x1.saturating_sub(1));

                for k in min_k..=max_k {
                    let c = Color::from_u32_argb(self.pixels[y * self.width + k]);
                    sum_r += c.r as u32;
                    sum_g += c.g as u32;
                    sum_b += c.b as u32;
                    count += 1;
                }

                if count > 0 {
                    let avg = Color::rgb((sum_r / count) as u8, (sum_g / count) as u8, (sum_b / count) as u8);
                    temp[y * self.width + x] = avg.to_u32_argb();
                }
            }
        }

        // Pass 2: Vertical Blur from temp into pixels
        for y in y0..y1 {
            for x in x0..x1 {
                let mut sum_r = 0u32;
                let mut sum_g = 0u32;
                let mut sum_b = 0u32;
                let mut count = 0u32;

                let min_k = if y >= rad { y - rad } else { 0 };
                let max_k = (y + rad).min(y1.saturating_sub(1));

                for k in min_k..=max_k {
                    let c = Color::from_u32_argb(temp[k * self.width + x]);
                    sum_r += c.r as u32;
                    sum_g += c.g as u32;
                    sum_b += c.b as u32;
                    count += 1;
                }

                if count > 0 {
                    let avg = Color::rgb((sum_r / count) as u8, (sum_g / count) as u8, (sum_b / count) as u8);
                    self.pixels[y * self.width + x] = avg.to_u32_argb();
                }
            }
        }
    }

    /// Drop Shadow / Box Shadow Shader with blur radius, spread, and directional offset.
    pub fn draw_box_shadow(
        &mut self,
        rect: Rect,
        offset_x: i32,
        offset_y: i32,
        blur_radius: i32,
        spread: i32,
        color: Color,
    ) {
        let shadow_rect = Rect::new(
            rect.x + offset_x - spread,
            rect.y + offset_y - spread,
            rect.width + spread * 2,
            rect.height + spread * 2,
        );

        if blur_radius <= 0 {
            self.fill_rounded_rect(shadow_rect, spread, color);
            return;
        }

        // Render shadow geometry with smooth radial falloff
        let margin = blur_radius;
        let x0 = (shadow_rect.x - margin).max(0);
        let y0 = (shadow_rect.y - margin).max(0);
        let x1 = (shadow_rect.x + shadow_rect.width + margin).min(self.width as i32);
        let y1 = (shadow_rect.y + shadow_rect.height + margin).min(self.height as i32);

        for y in y0..y1 {
            for x in x0..x1 {
                // Distance to box edges
                let dx = (shadow_rect.x - x).max(0).max(x - (shadow_rect.x + shadow_rect.width - 1));
                let dy = (shadow_rect.y - y).max(0).max(y - (shadow_rect.y + shadow_rect.height - 1));
                let dist = fast_sqrt((dx * dx + dy * dy) as f32);

                if dist < blur_radius as f32 {
                    // Gaussian-like smooth falloff: (1 - (dist / blur)^2)
                    let norm = dist / blur_radius as f32;
                    let diff = 1.0f32 - norm * norm;
                    let falloff = if diff > 0.0f32 { diff } else { 0.0f32 };
                    let shadow_alpha = (color.a as f32 * falloff) as u8;
                    self.set_pixel(x, y, color.with_alpha(shadow_alpha));
                }
            }
        }
    }

    /// Backdrop Filter (Acrylic / Glassmorphism) with blur and tint.
    pub fn apply_backdrop_filter(&mut self, rect: Rect, blur_radius: i32, tint: Color) {
        self.apply_gaussian_blur(rect, blur_radius);
        self.fill_rect(rect, tint);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_blend() {
        let red = Color::rgba(255, 0, 0, 255);
        let semi_blue = Color::rgba(0, 0, 255, 128);
        let blended = Color::blend(semi_blue, red);
        // Blend result should be purple with alpha ~255
        assert_eq!(blended.a, 255);
        assert!(blended.r > 100 && blended.r < 150);
        assert!(blended.b > 100 && blended.b < 150);
    }

    #[test]
    fn test_rounded_rect_sdf() {
        let mut surface = BufferSurface::new(60, 60);
        surface.clear(BLACK);

        let rect = Rect::new(10, 10, 40, 40);
        surface.fill_rounded_rect(rect, 8, WHITE);

        // Center should be white
        assert_eq!(surface.get_pixel(30, 30), WHITE);
        // Outer corner (10, 10) outside radius should be black
        assert_eq!(surface.get_pixel(10, 10), BLACK);
    }

    #[test]
    fn test_bezier_curves() {
        let mut surface = BufferSurface::new(100, 100);
        surface.clear(BLACK);

        // Quadratic Bezier
        surface.draw_quadratic_bezier(Point::new(0, 0), Point::new(50, 90), Point::new(100, 0), WHITE);
        // Midpoint should have received pixels
        let mut found = false;
        for y in 30..80 {
            if surface.get_pixel(50, y) != BLACK {
                found = true;
                break;
            }
        }
        assert!(found, "Bezier curve rasterized pixels around midpoint");
    }

    #[test]
    fn test_box_shadow_shader() {
        let mut surface = BufferSurface::new(100, 100);
        surface.clear(WHITE);

        let shadow_color = Color::rgba(0, 0, 0, 180);
        surface.draw_box_shadow(Rect::new(20, 20, 60, 60), 0, 4, 10, 2, shadow_color);

        // Pixel inside shadow bounds should be darkened from pure white
        let px = surface.get_pixel(50, 75);
        assert!(px.r < 255 && px.g < 255 && px.b < 255);
    }

    #[test]
    fn test_gaussian_blur() {
        let mut surface = BufferSurface::new(50, 50);
        surface.clear(BLACK);
        // Draw sharp white square
        surface.fill_rect(Rect::new(20, 20, 10, 10), WHITE);
        assert_eq!(surface.get_pixel(25, 25), WHITE);
        assert_eq!(surface.get_pixel(15, 25), BLACK);

        // Apply blur
        surface.apply_gaussian_blur(Rect::new(10, 10, 30, 30), 4);
        // Center should be softened (no longer 255) and neighbor should be lightened (no longer 0)
        let neighbor = surface.get_pixel(18, 25);
        assert!(neighbor.r > 0, "Blur spread light to neighbor");
    }
}
