//! Advanced Typography, Multi-Size Font Rendering, and Antialiasing Engine for NovaOS
//!
//! Provides a complete font system replacing single-size bitmap fonts:
//!   - Multiple font sizes: Micro (5x7 legacy), Small (8x8 compact), Regular (8x16 body),
//!     Large (12x24 headers/titlebars), and dynamic Vector(H) (scalable stroke rasterizer).
//!   - Natural proportional kerning & spacing (Sans) and fixed-pitch grid (Mono).
//!   - Coverage-based subpixel and area-sampled antialiasing (8-bit alpha blending).
//!   - Scalable analytical vector stroke rasterizer for dynamic font sizes with zero heap allocations.
//!   - Word wrapping, alignment, string dimension measurement, and Surface compositing.

use crate::color::Color;
use crate::draw::Surface;
use crate::font_data::*;
use crate::geom::{Rect, Size};
use crate::typography::{get_glyph, TextAlign, TypeScale};

pub const MAX_GLYPH_DIM: usize = 32;
pub const MAX_GLYPH_BYTES: usize = MAX_GLYPH_DIM * MAX_GLYPH_DIM; // 1024 bytes

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontSize {
    Micro,      // 5x7 legacy bitmap font
    Small,      // 8x8 compact font (cell: 8x8)
    Regular,    // 8x16 standard system body font (cell: 8x16)
    Large,      // 12x24 title/heading font (cell: 12x24)
    Vector(u8), // Scalable vector stroke rasterizer (target height 8..48px)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FontStyle {
    Sans, // Natural proportional letter spacing
    Mono, // Fixed-pitch monospaced cells (code, terminal, hex view)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AntialiasMode {
    None,     // 1-bit crisp binary edges
    Coverage, // 8-bit alpha coverage antialiasing (smooth edges)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct GlyphMetrics {
    pub width: i32,     // Visual width of glyph bounding box
    pub height: i32,    // Visual height of glyph
    pub advance: i32,   // Horizontal cursor advance
    pub bearing_x: i32, // Left side bearing offset
    pub bearing_y: i32, // Top side bearing offset relative to baseline
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Font {
    pub size: FontSize,
    pub style: FontStyle,
    pub antialias: AntialiasMode,
}

impl Font {
    pub const fn new(size: FontSize, style: FontStyle, antialias: AntialiasMode) -> Self {
        Font { size, style, antialias }
    }

    /// Compact 8x8 font for small UI widgets, badges, and 320x200 fallback.
    pub const fn small() -> Self {
        Font {
            size: FontSize::Small,
            style: FontStyle::Sans,
            antialias: AntialiasMode::Coverage,
        }
    }

    /// Standard 8x16 system font for body text, buttons, menus, inputs.
    pub const fn regular() -> Self {
        Font {
            size: FontSize::Regular,
            style: FontStyle::Sans,
            antialias: AntialiasMode::Coverage,
        }
    }

    /// Large 12x24 font for window titlebars, dialog headers, section headings.
    pub const fn large() -> Self {
        Font {
            size: FontSize::Large,
            style: FontStyle::Sans,
            antialias: AntialiasMode::Coverage,
        }
    }

    /// Monospaced 8x16 font for terminal, code, and hex viewer.
    pub const fn mono() -> Self {
        Font {
            size: FontSize::Regular,
            style: FontStyle::Mono,
            antialias: AntialiasMode::None,
        }
    }

    /// Prominent window titlebar / header font.
    pub const fn title() -> Self {
        Font {
            size: FontSize::Large,
            style: FontStyle::Sans,
            antialias: AntialiasMode::Coverage,
        }
    }

    /// Dynamic scalable vector font with analytical antialiasing.
    pub const fn vector(height: u8) -> Self {
        Font {
            size: FontSize::Vector(height),
            style: FontStyle::Sans,
            antialias: AntialiasMode::Coverage,
        }
    }

    pub const fn with_antialias(mut self, antialias: AntialiasMode) -> Self {
        self.antialias = antialias;
        self
    }

    pub const fn with_style(mut self, style: FontStyle) -> Self {
        self.style = style;
        self
    }

    /// Recommended line height (in pixels) for this font configuration.
    pub fn line_height(&self) -> i32 {
        match self.size {
            FontSize::Micro => 9,
            FontSize::Small => 10,
            FontSize::Regular => 16,
            FontSize::Large => 24,
            FontSize::Vector(h) => (h as i32 * 12) / 10,
        }
    }

    /// Ascent from baseline to top of capital letters.
    pub fn ascent(&self) -> i32 {
        match self.size {
            FontSize::Micro => 7,
            FontSize::Small => 7,
            FontSize::Regular => 12,
            FontSize::Large => 18,
            FontSize::Vector(h) => (h as i32 * 3) / 4,
        }
    }

    /// Returns metrics for an ASCII character.
    pub fn glyph_metrics(&self, c: u8) -> GlyphMetrics {
        if c < 32 || c > 126 {
            return GlyphMetrics {
                width: 0,
                height: self.line_height(),
                advance: self.measure_char(b' '),
                bearing_x: 0,
                bearing_y: 0,
            };
        }

        let idx = (c - 32) as usize;
        match self.size {
            FontSize::Micro => {
                let adv = if self.style == FontStyle::Mono { 6 } else { 5 };
                GlyphMetrics {
                    width: 5,
                    height: 7,
                    advance: adv,
                    bearing_x: 0,
                    bearing_y: 0,
                }
            }
            FontSize::Small => {
                if self.style == FontStyle::Mono {
                    GlyphMetrics {
                        width: 8,
                        height: 8,
                        advance: 8,
                        bearing_x: 0,
                        bearing_y: 0,
                    }
                } else {
                    let (min_c, raw_w, adv) = PROPORTIONAL_METRICS_8X8[idx];
                    GlyphMetrics {
                        width: raw_w as i32,
                        height: 8,
                        advance: adv as i32,
                        bearing_x: min_c as i32,
                        bearing_y: 0,
                    }
                }
            }
            FontSize::Regular => {
                if self.style == FontStyle::Mono {
                    GlyphMetrics {
                        width: 8,
                        height: 16,
                        advance: 8,
                        bearing_x: 0,
                        bearing_y: 0,
                    }
                } else {
                    let (min_c, raw_w, adv) = PROPORTIONAL_METRICS_8X16[idx];
                    GlyphMetrics {
                        width: raw_w as i32,
                        height: 16,
                        advance: adv as i32,
                        bearing_x: min_c as i32,
                        bearing_y: 0,
                    }
                }
            }
            FontSize::Large => {
                if self.style == FontStyle::Mono {
                    GlyphMetrics {
                        width: 12,
                        height: 24,
                        advance: 12,
                        bearing_x: 0,
                        bearing_y: 0,
                    }
                } else {
                    let (min_c, raw_w, adv) = PROPORTIONAL_METRICS_8X16[idx];
                    let scaled_w = (raw_w as i32 * 3 + 1) / 2;
                    let scaled_adv = (adv as i32 * 3 + 1) / 2;
                    let scaled_bx = (min_c as i32 * 3) / 2;
                    GlyphMetrics {
                        width: scaled_w,
                        height: 24,
                        advance: scaled_adv,
                        bearing_x: scaled_bx,
                        bearing_y: 0,
                    }
                }
            }
            FontSize::Vector(h) => {
                let target_h = h as i32;
                let target_w = (target_h * 3) / 4;
                let adv = if self.style == FontStyle::Mono {
                    target_w
                } else {
                    let (_, _, base_adv) = PROPORTIONAL_METRICS_8X16[idx];
                    ((base_adv as i32 * target_h) / 16).max(3)
                };
                GlyphMetrics {
                    width: target_w,
                    height: target_h,
                    advance: adv,
                    bearing_x: 0,
                    bearing_y: 0,
                }
            }
        }
    }

    /// Horizontal advance width for one character.
    pub fn measure_char(&self, c: u8) -> i32 {
        self.glyph_metrics(c).advance
    }

    /// Measures the dimensions of a multi-line string.
    pub fn measure_str(&self, text: &str) -> Size {
        let mut max_w = 0;
        let mut cur_w = 0;
        let mut lines = 1;

        for b in text.bytes() {
            if b == b'\n' {
                if cur_w > max_w {
                    max_w = cur_w;
                }
                cur_w = 0;
                lines += 1;
            } else {
                cur_w += self.measure_char(b);
            }
        }
        if cur_w > max_w {
            max_w = cur_w;
        }

        Size::new(max_w, lines * self.line_height())
    }

    /// Renders a glyph into a caller-supplied fixed buffer, returning (width, height, metrics).
    /// Each element in `out_buf` receives an 8-bit coverage value [0..=255].
    pub fn render_glyph(&self, c: u8, out_buf: &mut [u8; MAX_GLYPH_BYTES]) -> (i32, i32, GlyphMetrics) {
        // Zero the buffer
        for b in out_buf.iter_mut() {
            *b = 0;
        }

        let metrics = self.glyph_metrics(c);
        if c < 32 || c > 126 {
            return (0, 0, metrics);
        }
        let idx = (c - 32) as usize;

        match self.size {
            FontSize::Micro => {
                let glyph = get_glyph(c);
                for (r, &row) in glyph.iter().enumerate() {
                    for col in 0..5 {
                        if (row & (0x10 >> col)) != 0 {
                            out_buf[r * MAX_GLYPH_DIM + col] = 255;
                        }
                    }
                }
                (5, 7, metrics)
            }
            FontSize::Small => {
                let glyph = &FONT_8X8[idx];
                let is_aa = self.antialias == AntialiasMode::Coverage;

                for (r, &row) in glyph.iter().enumerate() {
                    for col in 0..8 {
                        if (row & (0x80 >> col)) != 0 {
                            let cov = if is_aa {
                                // Light corner softening for 8x8
                                let mut neighbors = 0;
                                if r > 0 && (glyph[r - 1] & (0x80 >> col)) != 0 { neighbors += 1; }
                                if r < 7 && (glyph[r + 1] & (0x80 >> col)) != 0 { neighbors += 1; }
                                if col > 0 && (row & (0x80 >> (col - 1))) != 0 { neighbors += 1; }
                                if col < 7 && (row & (0x80 >> (col + 1))) != 0 { neighbors += 1; }
                                if neighbors <= 1 { 210 } else { 255 }
                            } else {
                                255
                            };
                            out_buf[r * MAX_GLYPH_DIM + col] = cov;
                        } else if is_aa {
                            // Diagonal transition smoothing
                            let d1 = col > 0 && r > 0 && (glyph[r - 1] & (0x80 >> (col - 1))) != 0
                                  && col < 7 && r < 7 && (glyph[r + 1] & (0x80 >> (col + 1))) != 0;
                            let d2 = col < 7 && r > 0 && (glyph[r - 1] & (0x80 >> (col + 1))) != 0
                                  && col > 0 && r < 7 && (glyph[r + 1] & (0x80 >> (col - 1))) != 0;
                            if d1 || d2 {
                                out_buf[r * MAX_GLYPH_DIM + col] = 75;
                            }
                        }
                    }
                }
                (8, 8, metrics)
            }
            FontSize::Regular => {
                let glyph = &FONT_8X16[idx];
                let is_aa = self.antialias == AntialiasMode::Coverage;

                for (r, &row) in glyph.iter().enumerate() {
                    for col in 0..8 {
                        if (row & (0x80 >> col)) != 0 {
                            let cov = if is_aa {
                                let mut n = 0;
                                if r > 0 && (glyph[r - 1] & (0x80 >> col)) != 0 { n += 1; }
                                if r < 15 && (glyph[r + 1] & (0x80 >> col)) != 0 { n += 1; }
                                if col > 0 && (row & (0x80 >> (col - 1))) != 0 { n += 1; }
                                if col < 7 && (row & (0x80 >> (col + 1))) != 0 { n += 1; }
                                if n <= 1 { 200 } else { 255 }
                            } else {
                                255
                            };
                            out_buf[r * MAX_GLYPH_DIM + col] = cov;
                        } else if is_aa {
                            let d1 = col > 0 && r > 0 && (glyph[r - 1] & (0x80 >> (col - 1))) != 0
                                  && col < 7 && r < 15 && (glyph[r + 1] & (0x80 >> (col + 1))) != 0;
                            let d2 = col < 7 && r > 0 && (glyph[r - 1] & (0x80 >> (col + 1))) != 0
                                  && col > 0 && r < 15 && (glyph[r + 1] & (0x80 >> (col - 1))) != 0;
                            if d1 || d2 {
                                out_buf[r * MAX_GLYPH_DIM + col] = 85;
                            }
                        }
                    }
                }
                (8, 16, metrics)
            }
            FontSize::Large => {
                // 12x24 antialiased font via 3x3 supersampled area filtering from 8x16 master
                let glyph = &FONT_8X16[idx];
                let out_w = 12;
                let out_h = 24;

                for y in 0..out_h {
                    for x in 0..out_w {
                        if self.antialias == AntialiasMode::Coverage {
                            let mut lit_count = 0u32;
                            for sy in 0..3 {
                                for sx in 0..3 {
                                    let src_x = (x * 3 + sx) * 8 / (out_w * 3);
                                    let src_y = (y * 3 + sy) * 16 / (out_h * 3);
                                    if src_x < 8 && src_y < 16 && (glyph[src_y] & (0x80 >> src_x)) != 0 {
                                        lit_count += 1;
                                    }
                                }
                            }
                            let cov = ((lit_count * 255) / 9) as u8;
                            out_buf[y * MAX_GLYPH_DIM + x] = cov;
                        } else {
                            let src_x = (x * 8) / out_w;
                            let src_y = (y * 16) / out_h;
                            if src_x < 8 && src_y < 16 && (glyph[src_y] & (0x80 >> src_x)) != 0 {
                                out_buf[y * MAX_GLYPH_DIM + x] = 255;
                            }
                        }
                    }
                }
                (out_w as i32, out_h as i32, metrics)
            }
            FontSize::Vector(h) => {
                let target_h = (h as usize).min(MAX_GLYPH_DIM);
                let target_w = ((target_h * 3) / 4).max(4);

                // Try vector stroke rasterizer
                if let Some(strokes) = get_vector_strokes(c) {
                    rasterize_vector_strokes(strokes, target_w as i32, target_h as i32, out_buf);
                } else {
                    // Fall back to supersampled area filtering from 8x16 master
                    let glyph = &FONT_8X16[idx];
                    for y in 0..target_h {
                        for x in 0..target_w {
                            let mut lit = 0u32;
                            for sy in 0..3 {
                                for sx in 0..3 {
                                    let src_x = (x * 3 + sx) * 8 / (target_w * 3);
                                    let src_y = (y * 3 + sy) * 16 / (target_h * 3);
                                    if src_x < 8 && src_y < 16 && (glyph[src_y] & (0x80 >> src_x)) != 0 {
                                        lit += 1;
                                    }
                                }
                            }
                            out_buf[y * MAX_GLYPH_DIM + x] = ((lit * 255) / 9) as u8;
                        }
                    }
                }
                (target_w as i32, target_h as i32, metrics)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing Primitives Supporting Antialiased Typography
// ---------------------------------------------------------------------------

/// Draws a single character using the specified font with subpixel/coverage antialiasing.
pub fn draw_char_font<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, c: u8, color: Color, font: &Font) {
    let mut buf = [0u8; MAX_GLYPH_BYTES];
    let (gw, gh, metrics) = font.render_glyph(c, &mut buf);
    if gw <= 0 || gh <= 0 {
        return;
    }

    // In proportional mode, adjust x by left bearing if applicable
    let start_x = if font.style == FontStyle::Sans {
        x - metrics.bearing_x
    } else {
        x
    };

    for row in 0..gh {
        for col in 0..gw {
            let cov = buf[(row as usize) * MAX_GLYPH_DIM + col as usize];
            if cov > 0 {
                let px = start_x + col;
                let py = y + row;
                if font.antialias == AntialiasMode::Coverage && cov < 255 {
                    let blended_a = ((color.a as u32 * cov as u32 + 127) / 255) as u8;
                    s.put_pixel(px, py, Color::rgba(color.r, color.g, color.b, blended_a));
                } else {
                    s.put_pixel(px, py, color);
                }
            }
        }
    }
}

/// Draws multi-line text using the specified font.
pub fn draw_text_font<S: Surface + ?Sized>(
    s: &mut S,
    mut x: i32,
    mut y: i32,
    text: &str,
    color: Color,
    font: &Font,
) {
    let start_x = x;
    let line_h = font.line_height();

    for b in text.bytes() {
        if b == b'\n' {
            x = start_x;
            y += line_h;
        } else {
            let adv = font.measure_char(b);
            draw_char_font(s, x, y, b, color, font);
            x += adv;
        }
    }
}

/// Draws text aligned within a bounding rectangle.
pub fn draw_text_font_aligned<S: Surface + ?Sized>(
    s: &mut S,
    bounds: Rect,
    text: &str,
    align: TextAlign,
    color: Color,
    font: &Font,
) {
    let size = font.measure_str(text);
    let x = match align {
        TextAlign::Left => bounds.x,
        TextAlign::Center => bounds.x + (bounds.w - size.w) / 2,
        TextAlign::Right => bounds.right() - size.w,
    };
    let y = bounds.y + (bounds.h - size.h) / 2;
    draw_text_font(s, x, y, text, color, font);
}

/// Automatically selects the modern font matching a TypeScale token.
pub fn font_for_scale(scale: TypeScale) -> Font {
    match scale {
        TypeScale::Caption => Font::small(),
        TypeScale::Body => Font::regular(),
        TypeScale::BodyBold => Font::regular(),
        TypeScale::Subtitle => Font::large(),
        TypeScale::Title => Font::large(),
        TypeScale::Heading => Font::large(),
        TypeScale::Display => Font::vector(32),
    }
}

// ---------------------------------------------------------------------------
// Scalable Vector Stroke Rasterizer (Analytical Distance Antialiasing)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
pub struct StrokeSegment {
    pub x1: u8, // 0..100 normalized coordinates
    pub y1: u8,
    pub x2: u8,
    pub y2: u8,
}

impl StrokeSegment {
    pub const fn new(x1: u8, y1: u8, x2: u8, y2: u8) -> Self {
        StrokeSegment { x1, y1, x2, y2 }
    }
}

/// Analytical distance-to-segment calculation using integer arithmetic.
fn dist_sq_to_segment(px: i32, py: i32, x1: i32, y1: i32, x2: i32, y2: i32) -> i32 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len_sq = dx * dx + dy * dy;
    if len_sq == 0 {
        let ex = px - x1;
        let ey = py - y1;
        return ex * ex + ey * ey;
    }

    let dot = (px - x1) * dx + (py - y1) * dy;
    let t = if dot <= 0 {
        0
    } else if dot >= len_sq {
        1024
    } else {
        (dot * 1024) / len_sq
    };

    let proj_x = x1 + (t * dx) / 1024;
    let proj_y = y1 + (t * dy) / 1024;
    let ex = px - proj_x;
    let ey = py - proj_y;
    ex * ex + ey * ey
}

/// Integer square root via binary digit-by-digit calculation.
fn isqrt(val: u32) -> u32 {
    let mut x = val;
    let mut c = 0u32;
    let mut d = 1u32 << 30;
    while d > val {
        d >>= 2;
    }
    while d != 0 {
        if x >= c + d {
            x -= c + d;
            c = (c >> 1) + d;
        } else {
            c >>= 1;
        }
        d >>= 2;
    }
    c
}

fn rasterize_vector_strokes(
    strokes: &[StrokeSegment],
    target_w: i32,
    target_h: i32,
    out: &mut [u8; MAX_GLYPH_BYTES],
) {
    if strokes.is_empty() || target_w <= 0 || target_h <= 0 {
        return;
    }

    let stroke_width_fixed = (1800 * target_h) / 100; // Fixed-point 1024-based stroke width (~1.8px)
    let half_width = stroke_width_fixed / 2;
    let falloff = 512; // 0.5px transition boundary

    for y in 0..target_h {
        let ny = (y * 100) / target_h;
        for x in 0..target_w {
            let nx = (x * 100) / target_w;

            let mut min_dist_fixed = i32::MAX;
            for seg in strokes {
                let d2 = dist_sq_to_segment(nx, ny, seg.x1 as i32, seg.y1 as i32, seg.x2 as i32, seg.y2 as i32);
                let d_fixed = ((isqrt(d2 as u32) as i32) * target_h * 1024) / 100;
                if d_fixed < min_dist_fixed {
                    min_dist_fixed = d_fixed;
                }
            }

            // Analytical coverage calculation
            let cov = if min_dist_fixed <= half_width {
                255
            } else if min_dist_fixed >= half_width + falloff {
                0
            } else {
                let delta = half_width + falloff - min_dist_fixed;
                ((delta * 255) / falloff).clamp(0, 255) as u8
            };

            out[(y as usize) * MAX_GLYPH_DIM + (x as usize)] = cov;
        }
    }
}

static STROKES_A: [StrokeSegment; 3] = [
    StrokeSegment::new(15, 90, 50, 10),
    StrokeSegment::new(50, 10, 85, 90),
    StrokeSegment::new(28, 58, 72, 58),
];

static STROKES_B: [StrokeSegment; 8] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 10, 65, 10),
    StrokeSegment::new(65, 10, 75, 25),
    StrokeSegment::new(75, 25, 65, 50),
    StrokeSegment::new(20, 50, 65, 50),
    StrokeSegment::new(65, 50, 78, 68),
    StrokeSegment::new(78, 68, 65, 90),
    StrokeSegment::new(20, 90, 65, 90),
];

static STROKES_C: [StrokeSegment; 4] = [
    StrokeSegment::new(80, 20, 35, 20),
    StrokeSegment::new(35, 20, 20, 50),
    StrokeSegment::new(20, 50, 35, 85),
    StrokeSegment::new(35, 85, 80, 85),
];

static STROKES_D: [StrokeSegment; 5] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 10, 55, 10),
    StrokeSegment::new(55, 10, 80, 50),
    StrokeSegment::new(80, 50, 55, 90),
    StrokeSegment::new(20, 90, 55, 90),
];

static STROKES_E: [StrokeSegment; 4] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 10, 80, 10),
    StrokeSegment::new(20, 50, 65, 50),
    StrokeSegment::new(20, 90, 80, 90),
];

static STROKES_F: [StrokeSegment; 3] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 10, 80, 10),
    StrokeSegment::new(20, 50, 65, 50),
];

static STROKES_H: [StrokeSegment; 3] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(80, 10, 80, 90),
    StrokeSegment::new(20, 50, 80, 50),
];

static STROKES_I: [StrokeSegment; 3] = [
    StrokeSegment::new(50, 10, 50, 90),
    StrokeSegment::new(25, 10, 75, 10),
    StrokeSegment::new(25, 90, 75, 90),
];

static STROKES_L: [StrokeSegment; 2] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 90, 75, 90),
];

static STROKES_N: [StrokeSegment; 3] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 10, 80, 90),
    StrokeSegment::new(80, 10, 80, 90),
];

static STROKES_O: [StrokeSegment; 6] = [
    StrokeSegment::new(35, 10, 65, 10),
    StrokeSegment::new(65, 10, 80, 50),
    StrokeSegment::new(80, 50, 65, 90),
    StrokeSegment::new(65, 90, 35, 90),
    StrokeSegment::new(35, 90, 20, 50),
    StrokeSegment::new(20, 50, 35, 10),
];

static STROKES_P: [StrokeSegment; 5] = [
    StrokeSegment::new(20, 10, 20, 90),
    StrokeSegment::new(20, 10, 65, 10),
    StrokeSegment::new(65, 10, 78, 30),
    StrokeSegment::new(78, 30, 65, 55),
    StrokeSegment::new(20, 55, 65, 55),
];

static STROKES_T: [StrokeSegment; 2] = [
    StrokeSegment::new(50, 10, 50, 90),
    StrokeSegment::new(15, 10, 85, 10),
];

static STROKES_V: [StrokeSegment; 2] = [
    StrokeSegment::new(15, 10, 50, 90),
    StrokeSegment::new(50, 90, 85, 10),
];

static STROKES_X: [StrokeSegment; 2] = [
    StrokeSegment::new(20, 10, 80, 90),
    StrokeSegment::new(80, 10, 20, 90),
];

static STROKES_0: [StrokeSegment; 7] = [
    StrokeSegment::new(35, 10, 65, 10),
    StrokeSegment::new(65, 10, 80, 50),
    StrokeSegment::new(80, 50, 65, 90),
    StrokeSegment::new(65, 90, 35, 90),
    StrokeSegment::new(35, 90, 20, 50),
    StrokeSegment::new(20, 50, 35, 10),
    StrokeSegment::new(70, 25, 30, 75),
];

static STROKES_1: [StrokeSegment; 3] = [
    StrokeSegment::new(30, 30, 50, 10),
    StrokeSegment::new(50, 10, 50, 90),
    StrokeSegment::new(25, 90, 75, 90),
];

static STROKES_2: [StrokeSegment; 4] = [
    StrokeSegment::new(20, 25, 45, 10),
    StrokeSegment::new(45, 10, 75, 25),
    StrokeSegment::new(75, 25, 20, 90),
    StrokeSegment::new(20, 90, 80, 90),
];

static STROKES_3: [StrokeSegment; 5] = [
    StrokeSegment::new(20, 15, 75, 15),
    StrokeSegment::new(75, 15, 45, 45),
    StrokeSegment::new(45, 45, 75, 65),
    StrokeSegment::new(75, 65, 55, 90),
    StrokeSegment::new(55, 90, 20, 85),
];

static STROKES_4: [StrokeSegment; 3] = [
    StrokeSegment::new(65, 10, 20, 60),
    StrokeSegment::new(20, 60, 80, 60),
    StrokeSegment::new(65, 10, 65, 90),
];

static STROKES_5: [StrokeSegment; 6] = [
    StrokeSegment::new(75, 15, 25, 15),
    StrokeSegment::new(25, 15, 20, 48),
    StrokeSegment::new(20, 48, 65, 48),
    StrokeSegment::new(65, 48, 75, 70),
    StrokeSegment::new(75, 70, 55, 90),
    StrokeSegment::new(55, 90, 20, 85),
];

static STROKES_PLUS: [StrokeSegment; 2] = [
    StrokeSegment::new(50, 25, 50, 75),
    StrokeSegment::new(25, 50, 75, 50),
];

static STROKES_MINUS: [StrokeSegment; 1] = [
    StrokeSegment::new(20, 50, 80, 50),
];

/// Vector stroke definitions for common alphanumeric glyphs on a 100x100 grid.
fn get_vector_strokes(c: u8) -> Option<&'static [StrokeSegment]> {
    match c {
        b'A' => Some(&STROKES_A),
        b'B' => Some(&STROKES_B),
        b'C' => Some(&STROKES_C),
        b'D' => Some(&STROKES_D),
        b'E' => Some(&STROKES_E),
        b'F' => Some(&STROKES_F),
        b'H' => Some(&STROKES_H),
        b'I' => Some(&STROKES_I),
        b'L' => Some(&STROKES_L),
        b'N' => Some(&STROKES_N),
        b'O' => Some(&STROKES_O),
        b'P' => Some(&STROKES_P),
        b'T' => Some(&STROKES_T),
        b'V' => Some(&STROKES_V),
        b'X' => Some(&STROKES_X),
        b'0' => Some(&STROKES_0),
        b'1' => Some(&STROKES_1),
        b'2' => Some(&STROKES_2),
        b'3' => Some(&STROKES_3),
        b'4' => Some(&STROKES_4),
        b'5' => Some(&STROKES_5),
        b'+' => Some(&STROKES_PLUS),
        b'-' => Some(&STROKES_MINUS),
        _ => None,
    }
}
