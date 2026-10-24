//! Drawing Helpers, Surface Traits, and Widget Renderers for NovaOS
//!
//! Provides no_std 2D drawing routines operating on clipping-aware surfaces:
//! lines (Bresenham), filled and rounded rectangles, shadows, text, buttons,
//! window titlebars and frames, checkboxes, and progress bars.

use crate::color::{Color, ThemePalette, WHITE};
use crate::geom::Rect;
use crate::spacing::*;
use crate::typography::{get_glyph, measure_text, TextAlign, TypeScale, GLYPH_WIDTH};
use crate::widget::{ButtonVariant, CheckboxState, WidgetState, WidgetThemeSpec, WindowControlKind};

/// Abstract drawing surface for pixel emission
pub trait Surface {
    fn width(&self) -> i32;
    fn height(&self) -> i32;
    fn put_pixel(&mut self, x: i32, y: i32, color: Color);

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.width(), self.height())
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(clipped) = rect.intersection(&self.bounds()) {
            for y in clipped.y..clipped.bottom() {
                for x in clipped.x..clipped.right() {
                    self.put_pixel(x, y, color);
                }
            }
        }
    }
}

/// 32-bit (XRGB/ARGB) pixel buffer surface (used for linear framebuffers and SHM surfaces)
pub struct BufferSurface32<'a> {
    pub buffer: &'a mut [u32],
    pub width: i32,
    pub height: i32,
    pub pitch: i32, // in pixels
}

impl<'a> BufferSurface32<'a> {
    pub fn new(buffer: &'a mut [u32], width: i32, height: i32) -> Self {
        BufferSurface32 {
            buffer,
            width,
            height,
            pitch: width,
        }
    }

    pub fn new_with_pitch(buffer: &'a mut [u32], width: i32, height: i32, pitch: i32) -> Self {
        BufferSurface32 {
            buffer,
            width,
            height,
            pitch,
        }
    }
}

impl<'a> Surface for BufferSurface32<'a> {
    fn width(&self) -> i32 {
        self.width
    }

    fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            let idx = (y * self.pitch + x) as usize;
            if idx < self.buffer.len() {
                if color.a == 255 {
                    self.buffer[idx] = color.to_xrgb();
                } else if color.a > 0 {
                    let cur = Color::from_argb(self.buffer[idx]);
                    self.buffer[idx] = color.blend_over(cur).to_xrgb();
                }
            }
        }
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        if let Some(clipped) = rect.intersection(&self.bounds()) {
            let c32 = color.to_xrgb();
            for y in clipped.y..clipped.bottom() {
                let start = (y * self.pitch + clipped.x) as usize;
                let end = start + clipped.w as usize;
                if end <= self.buffer.len() {
                    if color.a == 255 {
                        for pixel in &mut self.buffer[start..end] {
                            *pixel = c32;
                        }
                    } else if color.a > 0 {
                        for pixel in &mut self.buffer[start..end] {
                            let cur = Color::from_argb(*pixel);
                            *pixel = color.blend_over(cur).to_xrgb();
                        }
                    }
                }
            }
        }
    }
}

/// 8-bit palette buffer surface (used for legacy VGA Mode 13h 320x200 buffers)
pub struct BufferSurface8<'a> {
    pub buffer: &'a mut [u8],
    pub width: i32,
    pub height: i32,
}

impl<'a> BufferSurface8<'a> {
    pub fn new(buffer: &'a mut [u8], width: i32, height: i32) -> Self {
        BufferSurface8 { buffer, width, height }
    }
}

impl<'a> Surface for BufferSurface8<'a> {
    fn width(&self) -> i32 {
        self.width
    }

    fn height(&self) -> i32 {
        self.height
    }

    #[inline]
    fn put_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            let idx = (y * self.width + x) as usize;
            if idx < self.buffer.len() {
                self.buffer[idx] = color.to_vga_index();
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Basic Drawing Helpers
// ---------------------------------------------------------------------------

pub fn draw_line<S: Surface + ?Sized>(s: &mut S, mut x0: i32, mut y0: i32, x1: i32, y1: i32, color: Color) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        s.put_pixel(x0, y0, color);
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

pub fn draw_rect_outline<S: Surface + ?Sized>(s: &mut S, rect: Rect, border_width: i32, color: Color) {
    if border_width <= 0 || rect.w <= 0 || rect.h <= 0 {
        return;
    }
    // Top & Bottom
    s.fill_rect(Rect::new(rect.x, rect.y, rect.w, border_width), color);
    s.fill_rect(Rect::new(rect.x, rect.bottom() - border_width, rect.w, border_width), color);
    // Left & Right
    s.fill_rect(Rect::new(rect.x, rect.y, border_width, rect.h), color);
    s.fill_rect(Rect::new(rect.right() - border_width, rect.y, border_width, rect.h), color);
}

pub fn draw_rounded_rect<S: Surface + ?Sized>(s: &mut S, rect: Rect, radius: i32, color: Color) {
    let r = radius.min(rect.w / 2).min(rect.h / 2);
    if r <= 1 {
        s.fill_rect(rect, color);
        return;
    }
    // Center cross
    s.fill_rect(Rect::new(rect.x + r, rect.y, rect.w - 2 * r, rect.h), color);
    s.fill_rect(Rect::new(rect.x, rect.y + r, r, rect.h - 2 * r), color);
    s.fill_rect(Rect::new(rect.right() - r, rect.y + r, r, rect.h - 2 * r), color);

    // Four rounded corners
    let centers = [
        (rect.x + r, rect.y + r),                     // top-left
        (rect.right() - r - 1, rect.y + r),           // top-right
        (rect.x + r, rect.bottom() - r - 1),          // bottom-left
        (rect.right() - r - 1, rect.bottom() - r - 1) // bottom-right
    ];

    let r2 = r * r;
    for dy in 0..=r {
        for dx in 0..=r {
            if dx * dx + dy * dy <= r2 {
                // Top-left
                s.put_pixel(centers[0].0 - dx, centers[0].1 - dy, color);
                // Top-right
                s.put_pixel(centers[1].0 + dx, centers[1].1 - dy, color);
                // Bottom-left
                s.put_pixel(centers[2].0 - dx, centers[2].1 + dy, color);
                // Bottom-right
                s.put_pixel(centers[3].0 + dx, centers[3].1 + dy, color);
            }
        }
    }
}

pub fn draw_char<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, c: u8, color: Color, scale: TypeScale) {
    let glyph = get_glyph(c);
    let px = scale.pixel_scale();

    for (row, bits) in glyph.iter().enumerate() {
        for col in 0..GLYPH_WIDTH {
            if bits & (0x10 >> col) != 0 {
                let gx = x + col * px;
                let gy = y + row as i32 * px;
                if px == 1 {
                    s.put_pixel(gx, gy, color);
                } else {
                    s.fill_rect(Rect::new(gx, gy, px, px), color);
                }
            }
        }
    }
}

pub fn draw_text<S: Surface + ?Sized>(s: &mut S, mut x: i32, mut y: i32, text: &str, color: Color, scale: TypeScale) {
    let px = scale.pixel_scale();
    let l_spacing = scale.letter_spacing();
    let char_w = GLYPH_WIDTH * px;
    let start_x = x;

    for b in text.bytes() {
        if b == b'\n' {
            x = start_x;
            y += scale.line_height();
        } else {
            draw_char(s, x, y, b, color, scale);
            x += char_w + l_spacing;
        }
    }
}

pub fn draw_text_aligned<S: Surface + ?Sized>(
    s: &mut S,
    bounds: Rect,
    text: &str,
    align: TextAlign,
    color: Color,
    scale: TypeScale,
) {
    let size = measure_text(text, scale);
    let x = match align {
        TextAlign::Left => bounds.x,
        TextAlign::Center => bounds.x + (bounds.w - size.w) / 2,
        TextAlign::Right => bounds.right() - size.w,
    };
    let y = bounds.y + (bounds.h - size.h) / 2;
    draw_text(s, x, y, text, color, scale);
}

// ---------------------------------------------------------------------------
// Standard Widget Renderers
// ---------------------------------------------------------------------------

pub fn draw_button<S: Surface + ?Sized>(
    s: &mut S,
    rect: Rect,
    label: &str,
    variant: ButtonVariant,
    state: WidgetState,
    palette: &ThemePalette,
) {
    let colors = WidgetThemeSpec::button_colors(variant, state, palette);
    draw_rounded_rect(s, rect, colors.radius, colors.bg);

    if colors.border != colors.bg && colors.border != crate::color::TRANSPARENT {
        draw_rect_outline(s, rect, 1, colors.border);
    }

    match variant {
        ButtonVariant::WindowControl(kind) => {
            // Draw standard window control icon
            let center = rect.center();
            match kind {
                WindowControlKind::Close => {
                    // X icon
                    draw_line(s, center.x - 3, center.y - 3, center.x + 3, center.y + 3, colors.text);
                    draw_line(s, center.x + 3, center.y - 3, center.x - 3, center.y + 3, colors.text);
                }
                WindowControlKind::Minimize => {
                    // - icon
                    draw_line(s, center.x - 3, center.y, center.x + 3, center.y, colors.text);
                }
                WindowControlKind::Maximize => {
                    // Square icon
                    draw_rect_outline(s, Rect::new(center.x - 3, center.y - 3, 7, 7), 1, colors.text);
                }
            }
        }
        _ => {
            draw_text_aligned(s, rect, label, TextAlign::Center, colors.text, TypeScale::Body);
        }
    }
}

pub fn draw_window_frame<S: Surface + ?Sized>(
    s: &mut S,
    rect: Rect,
    title: &str,
    is_active: bool,
    palette: &ThemePalette,
) {
    let (tb_bg, tb_text, border_col) = if is_active {
        (palette.titlebar_active, palette.titlebar_text_active, palette.border_focused)
    } else {
        (palette.titlebar_inactive, palette.titlebar_text_inactive, palette.border_normal)
    };

    // Outer window border and client background
    draw_rounded_rect(s, rect, RADIUS_MD, palette.window_bg);
    draw_rect_outline(s, rect, 1, border_col);

    // Titlebar
    let titlebar_rect = Rect::new(rect.x, rect.y, rect.w, TITLEBAR_HEIGHT);
    draw_rounded_rect(s, titlebar_rect, RADIUS_MD, tb_bg);
    // Square off the bottom of titlebar
    s.fill_rect(Rect::new(rect.x, rect.y + TITLEBAR_HEIGHT - RADIUS_MD, rect.w, RADIUS_MD), tb_bg);
    s.fill_rect(Rect::new(rect.x, rect.y + TITLEBAR_HEIGHT, rect.w, 1), palette.border_subtle);

    // Window controls (macOS style left or Windows style right; standard NovaOS has control buttons)
    let btn_size = 14;
    let btn_y = rect.y + (TITLEBAR_HEIGHT - btn_size) / 2;

    // Close button
    draw_button(
        s,
        Rect::new(rect.x + SPACE_SM, btn_y, btn_size, btn_size),
        "",
        ButtonVariant::WindowControl(WindowControlKind::Close),
        WidgetState::Normal,
        palette,
    );
    // Minimize button
    draw_button(
        s,
        Rect::new(rect.x + SPACE_SM + btn_size + SPACE_XS, btn_y, btn_size, btn_size),
        "",
        ButtonVariant::WindowControl(WindowControlKind::Minimize),
        WidgetState::Normal,
        palette,
    );
    // Maximize button
    draw_button(
        s,
        Rect::new(rect.x + SPACE_SM + (btn_size + SPACE_XS) * 2, btn_y, btn_size, btn_size),
        "",
        ButtonVariant::WindowControl(WindowControlKind::Maximize),
        WidgetState::Normal,
        palette,
    );

    // Centered or left-offset title text
    let text_x = rect.x + SPACE_SM + (btn_size + SPACE_XS) * 3 + SPACE_SM;
    let title_bounds = Rect::new(text_x, rect.y, rect.w - text_x - SPACE_SM, TITLEBAR_HEIGHT);
    draw_text_aligned(s, title_bounds, title, TextAlign::Left, tb_text, TypeScale::BodyBold);
}

pub fn draw_checkbox<S: Surface + ?Sized>(
    s: &mut S,
    rect: Rect,
    state: CheckboxState,
    widget_state: WidgetState,
    label: &str,
    palette: &ThemePalette,
) {
    let box_rect = Rect::new(rect.x, rect.y + (rect.h - CHECKBOX_SIZE) / 2, CHECKBOX_SIZE, CHECKBOX_SIZE);
    let (bg, border, check_col) = match state {
        CheckboxState::Checked | CheckboxState::Indeterminate => (
            palette.accent_primary,
            palette.accent_primary,
            WHITE,
        ),
        CheckboxState::Unchecked => match widget_state {
            WidgetState::Hover => (palette.surface_sunken, palette.border_focused, WHITE),
            _ => (palette.surface_sunken, palette.border_normal, WHITE),
        },
    };

    draw_rounded_rect(s, box_rect, RADIUS_XS, bg);
    draw_rect_outline(s, box_rect, 1, border);

    if state == CheckboxState::Checked {
        // Draw checkmark
        let bx = box_rect.x;
        let by = box_rect.y;
        draw_line(s, bx + 3, by + 7, bx + 6, by + 11, check_col);
        draw_line(s, bx + 6, by + 11, bx + 12, by + 4, check_col);
    } else if state == CheckboxState::Indeterminate {
        // Draw dash
        let bx = box_rect.x;
        let by = box_rect.y;
        s.fill_rect(Rect::new(bx + 4, by + 7, 8, 2), check_col);
    }

    // Label
    let label_rect = Rect::new(
        box_rect.right() + SPACE_SM,
        rect.y,
        rect.w - CHECKBOX_SIZE - SPACE_SM,
        rect.h,
    );
    draw_text_aligned(s, label_rect, label, TextAlign::Left, palette.text_primary, TypeScale::Body);
}

pub fn draw_progress_bar<S: Surface + ?Sized>(
    s: &mut S,
    rect: Rect,
    progress: f32, // 0.0 to 1.0
    palette: &ThemePalette,
) {
    let p = if progress < 0.0 { 0.0 } else if progress > 1.0 { 1.0 } else { progress };
    // Track
    draw_rounded_rect(s, rect, RADIUS_SM, palette.surface_sunken);
    draw_rect_outline(s, rect, 1, palette.border_subtle);

    // Fill
    let fill_w = ((rect.w as f32) * p) as i32;
    if fill_w > 0 {
        let fill_rect = Rect::new(rect.x, rect.y, fill_w, rect.h);
        draw_rounded_rect(s, fill_rect, RADIUS_SM, palette.accent_primary);
    }
}
