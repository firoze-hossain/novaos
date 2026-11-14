//! userland/desktop/nova-shell/src/dock.rs
//!
//! Windows 11 & macOS Hybrid Application Dock.
//!
//! Core Responsibilities:
//! - Pinned and running application icon models.
//! - Running indicators (subtle active dot / multi-window dots / focused accent bar).
//! - Badge notifications (e.g. unread counts, system alerts).
//! - macOS-style dynamic magnification curve (smooth Gaussian zoom on mouse hover).
//! - Launch animations: bounce & zoom physics state machine.
//! - Live window thumbnail preview captures from compositor surfaces with close/minimize actions.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

use nova_ui::prelude::*;

pub const DEFAULT_ICON_SIZE: i32 = 40;
pub const DOCK_PADDING: i32 = 6;
pub const DOCK_ITEM_GAP: i32 = 8;
pub const MAX_MAGNIFICATION: f32 = 1.35;
pub const MAGNIFICATION_RADIUS: f32 = 90.0;

// ============================================================================
// Dock Item & Thumbnail Data Models
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DockIconType {
    Start,
    Search,
    Files,
    Terminal,
    Editor,
    Settings,
    Store,
    Browser,
    Custom(u32),
}

/// Miniature surface thumbnail captured from compositor for live window previews
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowThumbnail {
    pub window_id: u64,
    pub title: String,
    pub width: u32,
    pub height: u32,
    /// XRGB8888 downscaled pixel buffer (e.g. 160x100)
    pub pixels: Vec<u32>,
    pub is_active: bool,
    pub is_minimized: bool,
    pub close_button_bounds: Rect,
    pub minimize_button_bounds: Rect,
    pub card_bounds: Rect,
}

impl WindowThumbnail {
    pub fn new(window_id: u64, title: &str, width: u32, height: u32, pixels: Vec<u32>) -> Self {
        Self {
            window_id,
            title: String::from(title),
            width,
            height,
            pixels,
            is_active: false,
            is_minimized: false,
            close_button_bounds: Rect::new(0, 0, 16, 16),
            minimize_button_bounds: Rect::new(0, 0, 16, 16),
            card_bounds: Rect::new(0, 0, width as i32 + 16, height as i32 + 36),
        }
    }

    /// Generate a simulated miniature surface thumbnail for testing & previews
    pub fn create_test_thumbnail(window_id: u64, title: &str, width: u32, height: u32, accent: Color) -> Self {
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                // Window chrome: top titlebar
                if y < 14 {
                    pixels.push(accent.to_u32_argb());
                } else if x == 0 || x == width - 1 || y == height - 1 {
                    pixels.push(0xFF404040); // Dark border
                } else {
                    // Dark content background
                    pixels.push(0xFF282828);
                }
            }
        }
        Self::new(window_id, title, width, height, pixels)
    }
}

/// Dock item representing an application launcher, running app, or system action
#[derive(Clone, Debug)]
pub struct DockItem {
    pub app_id: String,
    pub name: String,
    pub icon_type: DockIconType,
    pub exec_path: String,
    pub is_pinned: bool,
    pub window_ids: Vec<u64>,
    pub active_window_id: Option<u64>,
    pub badge_count: Option<u32>,
    pub base_bounds: Rect,
    pub rendered_bounds: Rect,
    pub current_scale: f32,
    pub target_scale: f32,
    pub bounce_offset_y: f32,
    pub bounce_velocity: f32,
    pub is_bouncing: bool,
    pub thumbnails: Vec<WindowThumbnail>,
}

impl DockItem {
    pub fn new(app_id: &str, name: &str, icon_type: DockIconType, exec_path: &str, is_pinned: bool) -> Self {
        Self {
            app_id: String::from(app_id),
            name: String::from(name),
            icon_type,
            exec_path: String::from(exec_path),
            is_pinned,
            window_ids: Vec::new(),
            active_window_id: None,
            badge_count: None,
            base_bounds: Rect::new(0, 0, DEFAULT_ICON_SIZE, DEFAULT_ICON_SIZE),
            rendered_bounds: Rect::new(0, 0, DEFAULT_ICON_SIZE, DEFAULT_ICON_SIZE),
            current_scale: 1.0,
            target_scale: 1.0,
            bounce_offset_y: 0.0,
            bounce_velocity: 0.0,
            is_bouncing: false,
            thumbnails: Vec::new(),
        }
    }

    pub fn is_running(&self) -> bool {
        !self.window_ids.is_empty()
    }

    pub fn is_focused(&self) -> bool {
        self.active_window_id.is_some()
    }

    /// Trigger launch bounce animation (macOS-style jumping icon)
    pub fn trigger_launch_bounce(&mut self) {
        self.is_bouncing = true;
        self.bounce_offset_y = 0.0;
        self.bounce_velocity = -12.0; // initial upward impulse
    }

    /// Update physics animation tick
    pub fn update_animation(&mut self, dt_factor: f32) {
        // 1. Scale smoothing towards target magnification
        let scale_diff = self.target_scale - self.current_scale;
        self.current_scale += scale_diff * 0.35 * dt_factor;
        if (self.current_scale - self.target_scale).abs() < 0.01 {
            self.current_scale = self.target_scale;
        }

        // 2. Bounce physics: Gravity and elastic dampening
        if self.is_bouncing {
            const GRAVITY: f32 = 1.2;
            const RESTITUTION: f32 = -0.62;

            self.bounce_velocity += GRAVITY * dt_factor;
            self.bounce_offset_y += self.bounce_velocity * dt_factor;

            // Hit ground (offset 0)
            if self.bounce_offset_y >= 0.0 {
                self.bounce_offset_y = 0.0;
                self.bounce_velocity *= RESTITUTION;
                if self.bounce_velocity.abs() < 1.0 {
                    self.bounce_velocity = 0.0;
                    self.is_bouncing = false;
                }
            }
        }
    }

    /// Add or update running window with its miniature surface thumbnail
    pub fn update_window(&mut self, thumb: WindowThumbnail) {
        if !self.window_ids.contains(&thumb.window_id) {
            self.window_ids.push(thumb.window_id);
        }
        if thumb.is_active {
            self.active_window_id = Some(thumb.window_id);
        }
        if let Some(pos) = self.thumbnails.iter().position(|t| t.window_id == thumb.window_id) {
            self.thumbnails[pos] = thumb;
        } else {
            self.thumbnails.push(thumb);
        }
    }

    /// Remove a closed window
    pub fn remove_window(&mut self, window_id: u64) {
        self.window_ids.retain(|&id| id != window_id);
        self.thumbnails.retain(|t| t.window_id != window_id);
        if self.active_window_id == Some(window_id) {
            self.active_window_id = self.window_ids.first().copied();
        }
    }
}

// ============================================================================
// Magnification Mathematics (macOS Smooth Zoom)
// ============================================================================

/// Computes the zoom factor for an icon center given the current cursor X position.
/// Uses a smooth cosine/Gaussian curve over `radius`.
pub fn calculate_icon_magnification(mouse_x: f32, icon_center_x: f32, max_scale: f32, radius: f32) -> f32 {
    let dist = (mouse_x - icon_center_x).abs();
    if dist >= radius {
        1.0
    } else {
        let normalized = dist / radius;
        let factor = (1.0 - normalized * normalized).max(0.0);
        1.0 + (max_scale - 1.0) * (factor * factor)
    }
}

// ============================================================================
// Live Window Thumbnail Hover Card State
// ============================================================================

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HoverPreviewCard {
    pub app_id: String,
    pub bounds: Rect,
    pub thumbnails: Vec<WindowThumbnail>,
    pub hovered_thumb_idx: Option<usize>,
    pub hovered_close_idx: Option<usize>,
    pub hovered_minimize_idx: Option<usize>,
}

impl HoverPreviewCard {
    pub fn new(app_id: &str, thumbnails: Vec<WindowThumbnail>, anchor_top_center: Point) -> Self {
        let thumb_count = thumbnails.len();
        if thumb_count == 0 {
            return Self {
                app_id: String::from(app_id),
                bounds: Rect::new(0, 0, 0, 0),
                thumbnails,
                hovered_thumb_idx: None,
                hovered_close_idx: None,
                hovered_minimize_idx: None,
            };
        }

        let thumb_w = 160;
        let thumb_h = 100;
        let card_pad = 10;
        let item_gap = 12;

        let total_w = card_pad * 2 + (thumb_count as i32 * thumb_w) + ((thumb_count as i32 - 1).max(0) * item_gap);
        let total_h = card_pad * 2 + thumb_h + 30; // 30px for title & action buttons

        let card_x = anchor_top_center.x - total_w / 2;
        let card_y = anchor_top_center.y - total_h - 10; // 10px floating gap above taskbar

        let bounds = Rect::new(card_x, card_y, total_w, total_h);

        let mut card = Self {
            app_id: String::from(app_id),
            bounds,
            thumbnails,
            hovered_thumb_idx: None,
            hovered_close_idx: None,
            hovered_minimize_idx: None,
        };
        card.relayout_thumbnails();
        card
    }

    pub fn relayout_thumbnails(&mut self) {
        let card_pad = 10;
        let item_gap = 12;
        let thumb_w = 160;
        let thumb_h = 100;

        let mut cur_x = self.bounds.x + card_pad;
        let cur_y = self.bounds.y + card_pad;

        for thumb in self.thumbnails.iter_mut() {
            thumb.card_bounds = Rect::new(cur_x, cur_y, thumb_w, thumb_h + 28);
            // Close button in top-right of thumbnail
            thumb.close_button_bounds = Rect::new(cur_x + thumb_w - 20, cur_y + 4, 16, 16);
            // Minimize button next to close button
            thumb.minimize_button_bounds = Rect::new(cur_x + thumb_w - 40, cur_y + 4, 16, 16);
            cur_x += thumb_w + item_gap;
        }
    }

    /// Hit-test mouse position against thumbnail previews and action buttons
    pub fn hit_test(&mut self, pos: Point) -> (Option<usize>, Option<usize>, Option<usize>) {
        if !self.bounds.contains(pos) {
            self.hovered_thumb_idx = None;
            self.hovered_close_idx = None;
            self.hovered_minimize_idx = None;
            return (None, None, None);
        }

        let mut hit_thumb = None;
        let mut hit_close = None;
        let mut hit_min = None;

        for (idx, thumb) in self.thumbnails.iter().enumerate() {
            if thumb.card_bounds.contains(pos) {
                hit_thumb = Some(idx);
                if thumb.close_button_bounds.contains(pos) {
                    hit_close = Some(idx);
                } else if thumb.minimize_button_bounds.contains(pos) {
                    hit_min = Some(idx);
                }
                break;
            }
        }

        self.hovered_thumb_idx = hit_thumb;
        self.hovered_close_idx = hit_close;
        self.hovered_minimize_idx = hit_min;

        (hit_thumb, hit_close, hit_min)
    }

    /// Render live window thumbnail preview flyout with acrylic blur, downscaled surface, and buttons
    pub fn render(
        &self,
        surface: &mut BufferSurface,
        tokens: &ColorTokens,
        elevation: &[ElevationTier; 6],
        corners: &CornerTokens,
    ) {
        if self.thumbnails.is_empty() || self.bounds.width <= 0 {
            return;
        }

        // 1. Background card with Level 3 Elevation Shadow
        render_elevated_card(
            surface,
            self.bounds,
            corners.card,
            3,
            elevation,
            tokens,
        );

        // 2. Render each window thumbnail
        for (idx, thumb) in self.thumbnails.iter().enumerate() {
            let is_thumb_hovered = self.hovered_thumb_idx == Some(idx);
            let bg_color = if thumb.is_active {
                tokens.accent_primary
            } else if is_thumb_hovered {
                tokens.card_hover
            } else {
                tokens.card_background
            };

            // Thumbnail card outline & title header
            surface.fill_rect(
                Rect::new(thumb.card_bounds.x, thumb.card_bounds.y, thumb.card_bounds.width, 24),
                bg_color,
            );

            // Thumbnail surface blit area
            let img_x = thumb.card_bounds.x + 2;
            let img_y = thumb.card_bounds.y + 26;
            let img_w = thumb.card_bounds.width - 4;
            let img_h = thumb.card_bounds.height - 28;

            // Blit thumbnail surface pixels (downscaled from compositor)
            if !thumb.pixels.is_empty() && thumb.width > 0 && thumb.height > 0 {
                let step_x = thumb.width as f32 / img_w as f32;
                let step_y = thumb.height as f32 / img_h as f32;

                for row in 0..img_h {
                    let src_y = (row as f32 * step_y) as u32;
                    if src_y >= thumb.height {
                        continue;
                    }
                    for col in 0..img_w {
                        let src_x = (col as f32 * step_x) as u32;
                        if src_x >= thumb.width {
                            continue;
                        }
                        let p = thumb.pixels[(src_y * thumb.width + src_x) as usize];
                        surface.set_pixel(img_x + col, img_y + row, Color::from_u32_argb(p));
                    }
                }
            } else {
                // Empty thumbnail placeholder
                surface.fill_rect(Rect::new(img_x, img_y, img_w, img_h), Color::rgb(35, 35, 35));
            }

            // Title indicator: small colored dot
            surface.fill_rect(Rect::new(thumb.card_bounds.x + 6, thumb.card_bounds.y + 8, 8, 8), Color::rgb(220, 220, 220));

            // Minimize button (—)
            let is_min_hover = self.hovered_minimize_idx == Some(idx);
            let min_bg = if is_min_hover { Color::rgb(70, 70, 70) } else { TRANSPARENT };
            surface.fill_rect(thumb.minimize_button_bounds, min_bg);
            surface.fill_rect(Rect::new(thumb.minimize_button_bounds.x + 4, thumb.minimize_button_bounds.y + 7, 8, 2), Color::rgb(200, 200, 200));

            // Close button (✕)
            let is_close_hover = self.hovered_close_idx == Some(idx);
            let close_bg = if is_close_hover { Color::rgb(232, 17, 35) } else { TRANSPARENT };
            surface.fill_rect(thumb.close_button_bounds, close_bg);
            for d in 0..6 {
                surface.set_pixel(thumb.close_button_bounds.x + 5 + d, thumb.close_button_bounds.y + 5 + d, WHITE);
                surface.set_pixel(thumb.close_button_bounds.x + 10 - d, thumb.close_button_bounds.y + 5 + d, WHITE);
            }
        }
    }
}
