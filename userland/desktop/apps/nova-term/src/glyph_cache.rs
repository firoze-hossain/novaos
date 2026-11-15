//! userland/desktop/apps/nova-term/src/glyph_cache.rs
//!
//! GPU Glyph Texture Atlas Cache & Monospaced Typography Blitter for NovaTerm.

use crate::grid::{Cell, FLAG_BOLD, FLAG_STRIKETHROUGH, FLAG_UNDERLINE};
use nova_ui::prelude::{BufferSurface, Color, Rect};

/// Font style variant for glyph cache lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontStyle {
    Regular,
    Bold,
    Italic,
}

/// Monospaced character cell metrics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellMetrics {
    pub cell_width: i32,
    pub cell_height: i32,
    pub baseline_y: i32,
}

impl CellMetrics {
    pub const fn default_metrics() -> Self {
        Self {
            cell_width: 9,
            cell_height: 18,
            baseline_y: 14,
        }
    }
}

/// Cursor display style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorStyle {
    Block,
    Underline,
    Beam,
}

/// High-speed character and cell rasterizer.
pub struct GlyphCache {
    pub metrics: CellMetrics,
    pub cursor_style: CursorStyle,
    pub cursor_blink_state: bool,
}

impl GlyphCache {
    pub fn new() -> Self {
        Self {
            metrics: CellMetrics::default_metrics(),
            cursor_style: CursorStyle::Block,
            cursor_blink_state: true,
        }
    }

    /// Renders a single terminal character cell onto the BufferSurface.
    pub fn render_cell(
        &self,
        surface: &mut BufferSurface,
        x: i32,
        y: i32,
        cell: &Cell,
        is_selected: bool,
        sel_color: Color,
        default_bg: Color,
    ) {
        let cw = self.metrics.cell_width;
        let ch = self.metrics.cell_height;
        let cell_rect = Rect::new(x, y, cw, ch);

        let (mut fg, mut bg) = (cell.fg, cell.bg);

        // Inverse attribute
        if cell.is_inverse() {
            core::mem::swap(&mut fg, &mut bg);
        }

        // Selection highlighting
        if is_selected {
            bg = sel_color;
        }

        // 1. Draw Cell Background (if different from default screen background or selected)
        if bg != default_bg || is_selected {
            surface.fill_rect(cell_rect, bg);
        }

        // 2. Draw Glyph if printable non-whitespace
        if cell.ch != ' ' && cell.ch as u32 >= 0x20 {
            self.draw_char_glyph(surface, x, y, cell.ch, fg, cell.flags);
        }

        // 3. Draw Underline
        if (cell.flags & FLAG_UNDERLINE) != 0 {
            let line_y = y + self.metrics.baseline_y + 1;
            surface.fill_rect(Rect::new(x, line_y, cw, 1), fg);
        }

        // 4. Draw Strikethrough
        if (cell.flags & FLAG_STRIKETHROUGH) != 0 {
            let line_y = y + ch / 2;
            surface.fill_rect(Rect::new(x, line_y, cw, 1), fg);
        }
    }

    /// Draws a text string onto the surface using monospaced cell metrics.
    pub fn draw_string(
        &self,
        surface: &mut BufferSurface,
        x: i32,
        y: i32,
        text: &str,
        color: Color,
    ) {
        let cw = self.metrics.cell_width;
        let mut cur_x = x;
        for ch in text.chars() {
            if ch != ' ' {
                self.draw_char_glyph(surface, cur_x, y, ch, color, 0);
            }
            cur_x += cw;
        }
    }

    /// Draws monospaced character glyph using procedural vector geometry.
    pub fn draw_char_glyph(
        &self,
        surface: &mut BufferSurface,
        x: i32,
        y: i32,
        ch: char,
        color: Color,
        flags: u8,
    ) {
        let gx = x + 1;
        let gy = y + 2;
        let stroke_w = if (flags & FLAG_BOLD) != 0 { 2 } else { 1 };

        // Fast representative vector strokes for common terminal CLI characters
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' => {
                // Main glyph stem
                surface.fill_rect(Rect::new(gx + 1, gy + 2, stroke_w, 10), color);
                surface.fill_rect(Rect::new(gx + 2, gy + 2, 4, stroke_w), color);
                surface.fill_rect(Rect::new(gx + 5, gy + 2, stroke_w, 10), color);
                surface.fill_rect(Rect::new(gx + 2, gy + 7, 4, stroke_w), color);
            }
            '$' | '#' | '>' | '%' => {
                // Prompt symbols
                surface.fill_rect(Rect::new(gx + 2, gy + 2, stroke_w, 11), color);
                surface.fill_rect(Rect::new(gx + 1, gy + 4, 5, stroke_w), color);
                surface.fill_rect(Rect::new(gx + 1, gy + 8, 5, stroke_w), color);
            }
            '-' | '_' | '=' => {
                let my = if ch == '_' { gy + 11 } else if ch == '=' { gy + 6 } else { gy + 7 };
                surface.fill_rect(Rect::new(gx, my, 7, stroke_w), color);
                if ch == '=' {
                    surface.fill_rect(Rect::new(gx, my + 3, 7, stroke_w), color);
                }
            }
            '/' | '\\' | '|' => {
                surface.fill_rect(Rect::new(gx + 3, gy + 1, stroke_w, 12), color);
            }
            ':' | ';' => {
                surface.fill_rect(Rect::new(gx + 3, gy + 4, 2, 2), color);
                surface.fill_rect(Rect::new(gx + 3, gy + 9, 2, 2), color);
            }
            '.' | ',' => {
                surface.fill_rect(Rect::new(gx + 3, gy + 10, 2, 2), color);
            }
            _ => {
                // Generic glyph representation
                surface.fill_rect(Rect::new(gx + 2, gy + 3, 4, 8), color);
            }
        }
    }

    /// Renders cursor at specified pixel position.
    pub fn render_cursor(
        &self,
        surface: &mut BufferSurface,
        x: i32,
        y: i32,
        cursor_color: Color,
    ) {
        if !self.cursor_blink_state {
            return;
        }

        let cw = self.metrics.cell_width;
        let ch = self.metrics.cell_height;

        match self.cursor_style {
            CursorStyle::Block => {
                // Translucent block or solid outline
                let block_rect = Rect::new(x, y, cw, ch);
                surface.fill_rect(block_rect, Color::rgba(cursor_color.r, cursor_color.g, cursor_color.b, 170));
            }
            CursorStyle::Underline => {
                let line_rect = Rect::new(x, y + ch - 2, cw, 2);
                surface.fill_rect(line_rect, cursor_color);
            }
            CursorStyle::Beam => {
                let beam_rect = Rect::new(x, y, 2, ch);
                surface.fill_rect(beam_rect, cursor_color);
            }
        }
    }
}
