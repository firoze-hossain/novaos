//! userland/desktop/nova-shell/src/taskbar.rs
//!
//! Windows 11-Style Centered Dynamic Taskbar & Application Dock for NovaOS.
//!
//! Features:
//! - Customizable alignment: `Centered` (Windows 11 default) or `Left` (classic Windows 10/Linux).
//! - Start Launcher button and Quick Search pill.
//! - Pinned and running application icons with active dot indicators:
//!   - Single active dot for single running window.
//!   - Double dot for multi-window instances.
//!   - Glowing accent pill/dash for the currently focused active window.
//! - Badge notifications (e.g. unread count badge pill).
//! - macOS-style dynamic icon magnification curve and launch bounce animation.
//! - Mouse-hover live window thumbnail previews captured from compositor with close/minimize actions.
//! - Fluent Design acrylic blur background, delicate border, and Level 3 elevation shadows.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

use nova_ui::prelude::*;
use crate::dock::*;

pub const TASKBAR_HEIGHT: i32 = 48;
pub const START_BUTTON_SIZE: i32 = 38;
pub const SEARCH_BUTTON_SIZE: i32 = 38;

// ============================================================================
// Alignment & Actions
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarAlignment {
    Centered,
    Left,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskbarAction {
    OpenStartMenu,
    OpenSearch,
    LaunchApp(String),
    FocusWindow(u64),
    MinimizeWindow(u64),
    CloseWindow(u64),
    ToggleWindow(u64),
    None,
}

// ============================================================================
// Taskbar Struct Definition
// ============================================================================

pub struct Taskbar {
    pub width: i32,
    pub height: i32,
    pub alignment: TaskbarAlignment,
    pub start_bounds: Rect,
    pub search_bounds: Rect,
    pub is_start_hovered: bool,
    pub is_search_hovered: bool,
    pub items: Vec<DockItem>,
    pub hovered_item_idx: Option<usize>,
    pub hover_preview: Option<HoverPreviewCard>,
    pub hover_ticks: u32,
    pub theme_mode: ThemeMode,
    pub tokens: ColorTokens,
    pub elevation: [ElevationTier; 6],
    pub corners: CornerTokens,
    pub magnification_enabled: bool,
}

impl Taskbar {
    pub fn new(width: i32, theme_mode: ThemeMode, accent: AccentPalette) -> Self {
        let tokens = ColorTokens::build(theme_mode, accent);
        let elevation = get_elevation_tiers(theme_mode);
        let corners = CornerTokens::default();

        let mut taskbar = Self {
            width,
            height: TASKBAR_HEIGHT,
            alignment: TaskbarAlignment::Centered,
            start_bounds: Rect::new(0, 0, START_BUTTON_SIZE, START_BUTTON_SIZE),
            search_bounds: Rect::new(0, 0, SEARCH_BUTTON_SIZE, SEARCH_BUTTON_SIZE),
            is_start_hovered: false,
            is_search_hovered: false,
            items: Vec::new(),
            hovered_item_idx: None,
            hover_preview: None,
            hover_ticks: 0,
            theme_mode,
            tokens,
            elevation,
            corners,
            magnification_enabled: true,
        };

        // Populate standard default pinned applications
        taskbar.add_pinned_item("files", "Files", DockIconType::Files, "FILES.ELF");
        taskbar.add_pinned_item("term", "Terminal", DockIconType::Terminal, "TERM.ELF");
        taskbar.add_pinned_item("edit", "Text Editor", DockIconType::Editor, "EDIT.ELF");
        taskbar.add_pinned_item("settings", "Settings", DockIconType::Settings, "SETTINGS.ELF");
        taskbar.add_pinned_item("store", "App Store", DockIconType::Store, "STORE.ELF");

        taskbar.relayout();
        taskbar
    }

    /// Add a pinned application to the dock
    pub fn add_pinned_item(&mut self, app_id: &str, name: &str, icon_type: DockIconType, exec_path: &str) {
        if let Some(item) = self.items.iter_mut().find(|it| it.app_id == app_id) {
            item.is_pinned = true;
            return;
        }
        let item = DockItem::new(app_id, name, icon_type, exec_path, true);
        self.items.push(item);
        self.relayout();
    }

    /// Remove pinning (unpins or removes item if not running)
    pub fn unpin_item(&mut self, app_id: &str) {
        if let Some(pos) = self.items.iter().position(|it| it.app_id == app_id) {
            if self.items[pos].is_running() {
                self.items[pos].is_pinned = false;
            } else {
                self.items.remove(pos);
            }
            self.relayout();
        }
    }

    /// Set badge notification count for an app
    pub fn set_badge_count(&mut self, app_id: &str, count: Option<u32>) {
        if let Some(item) = self.items.iter_mut().find(|it| it.app_id == app_id) {
            item.badge_count = count;
        }
    }

    /// Set Taskbar Alignment (Centered vs Left) and recalculate icon layout
    pub fn set_alignment(&mut self, alignment: TaskbarAlignment) {
        self.alignment = alignment;
        self.relayout();
    }

    /// Recalculate horizontal coordinates for Start, Search, and Dock Items
    pub fn relayout(&mut self) {
        let total_items = self.items.len();
        let gap = DOCK_ITEM_GAP;
        let item_w = DEFAULT_ICON_SIZE;
        let y_pos = (self.height - item_w) / 2;

        let cluster_width = START_BUTTON_SIZE + gap + SEARCH_BUTTON_SIZE + gap
            + (total_items as i32 * item_w) + ((total_items as i32 - 1).max(0) * gap);

        let mut start_x = match self.alignment {
            TaskbarAlignment::Centered => (self.width - cluster_width) / 2,
            TaskbarAlignment::Left => 16,
        };

        // 1. Start Button
        self.start_bounds = Rect::new(start_x, y_pos, START_BUTTON_SIZE, START_BUTTON_SIZE);
        start_x += START_BUTTON_SIZE + gap;

        // 2. Search Button
        self.search_bounds = Rect::new(start_x, y_pos, SEARCH_BUTTON_SIZE, SEARCH_BUTTON_SIZE);
        start_x += SEARCH_BUTTON_SIZE + gap;

        // 3. Dock Items
        for item in self.items.iter_mut() {
            item.base_bounds = Rect::new(start_x, y_pos, item_w, item_w);
            item.rendered_bounds = item.base_bounds;
            start_x += item_w + gap;
        }
    }

    /// Synchronize running window surface thumbnail from nova-compositor
    pub fn sync_window_surface(&mut self, app_id: &str, thumb: WindowThumbnail) {
        // If app is not in taskbar, add dynamically as running unpinned app
        if !self.items.iter().any(|it| it.app_id == app_id) {
            let name = thumb.title.clone();
            let mut new_item = DockItem::new(app_id, &name, DockIconType::Custom(1), app_id, false);
            new_item.update_window(thumb);
            self.items.push(new_item);
            self.relayout();
        } else if let Some(item) = self.items.iter_mut().find(|it| it.app_id == app_id) {
            item.update_window(thumb);
        }

        // If preview card currently matches this app, update its preview thumbnails
        if let Some(card) = &mut self.hover_preview {
            if card.app_id == app_id {
                if let Some(item) = self.items.iter().find(|it| it.app_id == app_id) {
                    card.thumbnails = item.thumbnails.clone();
                    card.relayout_thumbnails();
                }
            }
        }
    }

    /// Notify taskbar that a window has closed
    pub fn remove_window(&mut self, window_id: u64) {
        let mut app_to_prune = None;
        for item in self.items.iter_mut() {
            if item.window_ids.contains(&window_id) {
                item.remove_window(window_id);
                if !item.is_running() && !item.is_pinned {
                    app_to_prune = Some(item.app_id.clone());
                }
                break;
            }
        }

        if let Some(app_id) = app_to_prune {
            self.items.retain(|it| it.app_id != app_id);
            self.relayout();
        }

        if let Some(card) = &mut self.hover_preview {
            card.thumbnails.retain(|t| t.window_id != window_id);
            if card.thumbnails.is_empty() {
                self.hover_preview = None;
            } else {
                card.relayout_thumbnails();
            }
        }
    }

    /// Update active focused window
    pub fn set_active_window(&mut self, window_id: Option<u64>) {
        for item in self.items.iter_mut() {
            if let Some(wid) = window_id {
                if item.window_ids.contains(&wid) {
                    item.active_window_id = Some(wid);
                    for thumb in item.thumbnails.iter_mut() {
                        thumb.is_active = thumb.window_id == wid;
                    }
                } else {
                    item.active_window_id = None;
                    for thumb in item.thumbnails.iter_mut() {
                        thumb.is_active = false;
                    }
                }
            } else {
                item.active_window_id = None;
                for thumb in item.thumbnails.iter_mut() {
                    thumb.is_active = false;
                }
            }
        }
    }

    /// Handle mouse motion: macOS magnification zoom, hover flyout detection, and button states
    pub fn handle_mouse_move(&mut self, pos: Point) -> bool {
        let mut changed = false;

        // 1. Check hover preview card hit testing if open
        if let Some(card) = &mut self.hover_preview {
            if card.bounds.contains(pos) {
                card.hit_test(pos);
                return true;
            }
        }

        // 2. Start and Search buttons hover
        let new_start_hover = self.start_bounds.contains(pos);
        if self.is_start_hovered != new_start_hover {
            self.is_start_hovered = new_start_hover;
            changed = true;
        }

        let new_search_hover = self.search_bounds.contains(pos);
        if self.is_search_hovered != new_search_hover {
            self.is_search_hovered = new_search_hover;
            changed = true;
        }

        // 3. Item Hover & macOS Dock Magnification
        let mut newly_hovered_idx = None;
        let in_taskbar = pos.y >= 0 && pos.y <= self.height;

        for (idx, item) in self.items.iter_mut().enumerate() {
            let center_x = (item.base_bounds.x + item.base_bounds.width / 2) as f32;

            if self.magnification_enabled && in_taskbar {
                let zoom = calculate_icon_magnification(
                    pos.x as f32,
                    center_x,
                    MAX_MAGNIFICATION,
                    MAGNIFICATION_RADIUS,
                );
                item.target_scale = zoom;
            } else {
                item.target_scale = 1.0;
            }

            if item.base_bounds.contains(pos) {
                newly_hovered_idx = Some(idx);
            }
        }

        if self.hovered_item_idx != newly_hovered_idx {
            self.hovered_item_idx = newly_hovered_idx;
            changed = true;

            if let Some(idx) = newly_hovered_idx {
                let item = &self.items[idx];
                if item.is_running() && !item.thumbnails.is_empty() {
                    let anchor = Point::new(
                        item.base_bounds.x + item.base_bounds.width / 2,
                        0, // Top of taskbar
                    );
                    self.hover_preview = Some(HoverPreviewCard::new(
                        &item.app_id,
                        item.thumbnails.clone(),
                        anchor,
                    ));
                } else {
                    self.hover_preview = None;
                }
            } else if !self.is_start_hovered && !self.is_search_hovered {
                if let Some(card) = &self.hover_preview {
                    if !card.bounds.contains(pos) {
                        self.hover_preview = None;
                        changed = true;
                    }
                }
            }
        }

        changed
    }

    /// Handle mouse click interaction
    pub fn handle_mouse_click(&mut self, pos: Point) -> TaskbarAction {
        // 1. Check live preview thumbnail actions (Close, Minimize, Focus)
        if let Some(card) = &mut self.hover_preview {
            let (hit_thumb, hit_close, hit_min) = card.hit_test(pos);
            if let Some(idx) = hit_close {
                let win_id = card.thumbnails[idx].window_id;
                self.remove_window(win_id);
                return TaskbarAction::CloseWindow(win_id);
            } else if let Some(idx) = hit_min {
                let win_id = card.thumbnails[idx].window_id;
                return TaskbarAction::MinimizeWindow(win_id);
            } else if let Some(idx) = hit_thumb {
                let win_id = card.thumbnails[idx].window_id;
                self.set_active_window(Some(win_id));
                self.hover_preview = None;
                return TaskbarAction::FocusWindow(win_id);
            }
        }

        // 2. Start Button click
        if self.start_bounds.contains(pos) {
            self.hover_preview = None;
            return TaskbarAction::OpenStartMenu;
        }

        // 3. Search Button click
        if self.search_bounds.contains(pos) {
            self.hover_preview = None;
            return TaskbarAction::OpenSearch;
        }

        // 4. Dock Items click
        for item in self.items.iter_mut() {
            if item.base_bounds.contains(pos) {
                self.hover_preview = None;
                if !item.is_running() {
                    // Launch app with bounce animation!
                    item.trigger_launch_bounce();
                    return TaskbarAction::LaunchApp(item.exec_path.clone());
                } else if item.window_ids.len() == 1 {
                    // Single window: toggle focus/minimize
                    let win_id = item.window_ids[0];
                    if item.is_focused() {
                        return TaskbarAction::MinimizeWindow(win_id);
                    } else {
                        self.set_active_window(Some(win_id));
                        return TaskbarAction::FocusWindow(win_id);
                    }
                } else {
                    // Multiple windows: open hover preview
                    let anchor = Point::new(
                        item.base_bounds.x + item.base_bounds.width / 2,
                        0,
                    );
                    self.hover_preview = Some(HoverPreviewCard::new(
                        &item.app_id,
                        item.thumbnails.clone(),
                        anchor,
                    ));
                    return TaskbarAction::ToggleWindow(item.window_ids[0]);
                }
            }
        }

        // Click outside closes flyout
        self.hover_preview = None;
        TaskbarAction::None
    }

    /// Advance physics animation loop (bounce + zoom easing)
    pub fn tick_animation(&mut self, dt_factor: f32) -> bool {
        let mut active = false;
        for item in self.items.iter_mut() {
            item.update_animation(dt_factor);
            if item.is_bouncing || (item.current_scale - item.target_scale).abs() > 0.005 {
                active = true;
            }
        }
        active
    }

    /// Render full Taskbar to BufferSurface
    pub fn render(&self, surface: &mut BufferSurface) {
        let bar_rect = Rect::new(0, 0, self.width, self.height);

        // 1. Taskbar Acrylic blurred background
        surface.apply_gaussian_blur(bar_rect, self.tokens.acrylic_blur_radius / 2);
        surface.fill_rect(bar_rect, self.tokens.acrylic_tint);

        // Top subtle delicate border (Windows 11 Mica highlight)
        surface.fill_rect(Rect::new(0, 0, self.width, 1), self.tokens.border_subtle);

        // 2. Render Start Button
        let start_bg = if self.is_start_hovered {
            self.tokens.card_hover
        } else {
            TRANSPARENT
        };
        surface.fill_rounded_rect(self.start_bounds, self.corners.small, start_bg);
        self.render_start_icon(surface, self.start_bounds);

        // 3. Render Search Button
        let search_bg = if self.is_search_hovered {
            self.tokens.card_hover
        } else {
            TRANSPARENT
        };
        surface.fill_rounded_rect(self.search_bounds, self.corners.small, search_bg);
        self.render_search_icon(surface, self.search_bounds);

        // 4. Render Dock Items
        for (idx, item) in self.items.iter().enumerate() {
            let is_hovered = self.hovered_item_idx == Some(idx);
            self.render_dock_item(surface, item, is_hovered);
        }

        // 5. Render Floating Live Window Thumbnail Preview Card if active
        if let Some(card) = &self.hover_preview {
            card.render(surface, &self.tokens, &self.elevation, &self.corners);
        }
    }

    fn render_start_icon(&self, surface: &mut BufferSurface, bounds: Rect) {
        // Windows 11 / NovaOS 4-square grid emblem
        let cx = bounds.x + bounds.width / 2;
        let cy = bounds.y + bounds.height / 2;
        let size = 6;
        let gap = 3;

        let blue = self.tokens.accent_primary;
        // Top-left
        surface.fill_rect(Rect::new(cx - size - gap / 2, cy - size - gap / 2, size, size), blue);
        // Top-right
        surface.fill_rect(Rect::new(cx + gap / 2, cy - size - gap / 2, size, size), blue);
        // Bottom-left
        surface.fill_rect(Rect::new(cx - size - gap / 2, cy + gap / 2, size, size), blue);
        // Bottom-right
        surface.fill_rect(Rect::new(cx + gap / 2, cy + gap / 2, size, size), blue);
    }

    fn render_search_icon(&self, surface: &mut BufferSurface, bounds: Rect) {
        // Magnifying glass icon
        let cx = bounds.x + bounds.width / 2 - 2;
        let cy = bounds.y + bounds.height / 2 - 2;
        let icon_color = self.tokens.text_secondary;

        // Lens outline
        surface.fill_rect(Rect::new(cx - 5, cy - 5, 10, 1), icon_color);
        surface.fill_rect(Rect::new(cx - 5, cy + 4, 10, 1), icon_color);
        surface.fill_rect(Rect::new(cx - 5, cy - 5, 1, 10), icon_color);
        surface.fill_rect(Rect::new(cx + 4, cy - 5, 1, 10), icon_color);
        // Handle diagonal
        for d in 0..5 {
            surface.set_pixel(cx + 4 + d, cy + 4 + d, icon_color);
        }
    }

    fn render_dock_item(&self, surface: &mut BufferSurface, item: &DockItem, is_hovered: bool) {
        let b = item.base_bounds;

        // Apply bounce vertical offset
        let y_offset = item.bounce_offset_y as i32;
        let item_rect = Rect::new(b.x, b.y + y_offset, b.width, b.height);

        // Hover background pill
        if is_hovered {
            surface.fill_rounded_rect(item_rect, self.corners.small, self.tokens.card_hover);
        }

        // Render application icon
        self.render_app_icon(surface, item.icon_type, item_rect);

        // Active running indicator
        if item.is_running() {
            let indicator_y = self.height - 4;
            if item.is_focused() {
                // Focused app: vibrant accent pill (16x3px)
                let ind_w = 16;
                let ind_x = item_rect.x + (item_rect.width - ind_w) / 2;
                surface.fill_rounded_rect(Rect::new(ind_x, indicator_y - 1, ind_w, 3), 1, self.tokens.accent_primary);
            } else if item.window_ids.len() > 1 {
                // Multi-window: double dot (● ●)
                let dot_w = 4;
                let ind_x1 = item_rect.x + (item_rect.width / 2) - 5;
                let ind_x2 = item_rect.x + (item_rect.width / 2) + 2;
                surface.fill_rect(Rect::new(ind_x1, indicator_y, dot_w, 2), self.tokens.text_muted);
                surface.fill_rect(Rect::new(ind_x2, indicator_y, dot_w, 2), self.tokens.text_muted);
            } else {
                // Single window: single dot (●)
                let dot_w = 5;
                let ind_x = item_rect.x + (item_rect.width - dot_w) / 2;
                surface.fill_rect(Rect::new(ind_x, indicator_y, dot_w, 2), self.tokens.text_secondary);
            }
        }

        // Badge notification counter pill
        if let Some(count) = item.badge_count {
            if count > 0 {
                let badge_x = item_rect.x + item_rect.width - 12;
                let badge_y = item_rect.y + 2;
                let badge_color = Color::rgb(232, 17, 35); // Crimson badge
                surface.fill_rounded_rect(Rect::new(badge_x, badge_y, 10, 10), 5, badge_color);
                // Center white pip
                surface.fill_rect(Rect::new(badge_x + 3, badge_y + 3, 4, 4), WHITE);
            }
        }
    }

    fn render_app_icon(&self, surface: &mut BufferSurface, icon_type: DockIconType, bounds: Rect) {
        let cx = bounds.x + bounds.width / 2;
        let cy = bounds.y + bounds.height / 2;

        match icon_type {
            DockIconType::Files => {
                // Folder icon: Manila yellow body with dark amber tab
                let fx = cx - 11;
                let fy = cy - 8;
                // Tab
                surface.fill_rect(Rect::new(fx, fy - 3, 9, 4), Color::rgb(212, 150, 20));
                // Body
                surface.fill_rect(Rect::new(fx, fy, 22, 16), Color::rgb(255, 185, 0));
            }
            DockIconType::Terminal => {
                // Terminal window: dark slate background with '>' chevron and '_' cursor
                let tx = cx - 11;
                let ty = cy - 8;
                surface.fill_rect(Rect::new(tx, ty, 22, 16), Color::rgb(20, 24, 30));
                // Chevron '>'
                for i in 0..4 {
                    surface.set_pixel(tx + 4 + i, ty + 5 + i, Color::rgb(0, 255, 128));
                    surface.set_pixel(tx + 4 + i, ty + 11 - i, Color::rgb(0, 255, 128));
                }
                // Cursor '_'
                surface.fill_rect(Rect::new(tx + 11, ty + 11, 5, 2), WHITE);
            }
            DockIconType::Editor => {
                // Document text icon: Blue card with white text lines
                let ex = cx - 9;
                let ey = cy - 11;
                surface.fill_rect(Rect::new(ex, ey, 18, 22), Color::rgb(0, 120, 215));
                // Lines
                surface.fill_rect(Rect::new(ex + 3, ey + 5, 12, 2), WHITE);
                surface.fill_rect(Rect::new(ex + 3, ey + 9, 10, 2), WHITE);
                surface.fill_rect(Rect::new(ex + 3, ey + 13, 12, 2), WHITE);
                surface.fill_rect(Rect::new(ex + 3, ey + 17, 7, 2), WHITE);
            }
            DockIconType::Settings => {
                // Gear wheel: gray ring with center hub
                surface.fill_rect(Rect::new(cx - 7, cy - 7, 14, 14), Color::rgb(140, 140, 140));
                surface.fill_rect(Rect::new(cx - 3, cy - 3, 6, 6), Color::rgb(40, 40, 40));
                // Teeth
                surface.fill_rect(Rect::new(cx - 2, cy - 9, 4, 3), Color::rgb(180, 180, 180));
                surface.fill_rect(Rect::new(cx - 2, cy + 6, 4, 3), Color::rgb(180, 180, 180));
                surface.fill_rect(Rect::new(cx - 9, cy - 2, 3, 4), Color::rgb(180, 180, 180));
                surface.fill_rect(Rect::new(cx + 6, cy - 2, 3, 4), Color::rgb(180, 180, 180));
            }
            DockIconType::Store => {
                // Shopping bag: purple/violet pouch with handle
                let sx = cx - 9;
                let sy = cy - 7;
                surface.fill_rect(Rect::new(sx, sy, 18, 16), Color::rgb(136, 23, 152));
                // Handle
                surface.fill_rect(Rect::new(sx + 4, sy - 4, 10, 4), Color::rgb(200, 150, 220));
                surface.fill_rect(Rect::new(sx + 6, sy - 3, 6, 3), Color::rgb(40, 40, 40));
            }
            DockIconType::Browser => {
                // Globe / compass: blue sphere
                surface.fill_rounded_rect(Rect::new(cx - 9, cy - 9, 18, 18), 9, Color::rgb(0, 164, 239));
                surface.fill_rect(Rect::new(cx - 9, cy, 18, 1), WHITE);
            }
            _ => {
                // Generic app: rounded colored tile
                surface.fill_rounded_rect(Rect::new(cx - 8, cy - 8, 16, 16), 4, self.tokens.accent_primary);
            }
        }
    }
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_taskbar_initialization_and_alignment() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        assert_eq!(taskbar.height, TASKBAR_HEIGHT);
        assert_eq!(taskbar.alignment, TaskbarAlignment::Centered);
        assert_eq!(taskbar.items.len(), 5);

        // Check centered alignment coordinates
        assert!(taskbar.start_bounds.x > 300);
        assert!(taskbar.start_bounds.x < 500);

        // Switch to Left alignment
        taskbar.set_alignment(TaskbarAlignment::Left);
        assert_eq!(taskbar.alignment, TaskbarAlignment::Left);
        assert_eq!(taskbar.start_bounds.x, 16);
    }

    #[test]
    fn test_pinning_and_unpinning() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        let orig_len = taskbar.items.len();

        taskbar.add_pinned_item("browser", "Browser", DockIconType::Browser, "BROWSER.ELF");
        assert_eq!(taskbar.items.len(), orig_len + 1);

        // Unpin non-running item removes it
        taskbar.unpin_item("browser");
        assert_eq!(taskbar.items.len(), orig_len);
    }

    #[test]
    fn test_running_indicators_and_multi_window() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);

        // Initial state: not running
        assert!(!taskbar.items[0].is_running());

        // Window 1 created
        let thumb1 = WindowThumbnail::create_test_thumbnail(101, "Folder A", 160, 100, Color::rgb(0, 120, 215));
        taskbar.sync_window_surface("files", thumb1);
        assert!(taskbar.items[0].is_running());
        assert_eq!(taskbar.items[0].window_ids.len(), 1);

        // Focus window 1
        taskbar.set_active_window(Some(101));
        assert!(taskbar.items[0].is_focused());

        // Window 2 created (multi-window)
        let thumb2 = WindowThumbnail::create_test_thumbnail(102, "Folder B", 160, 100, Color::rgb(0, 120, 215));
        taskbar.sync_window_surface("files", thumb2);
        assert_eq!(taskbar.items[0].window_ids.len(), 2);

        // Remove window 1
        taskbar.remove_window(101);
        assert_eq!(taskbar.items[0].window_ids.len(), 1);

        // Remove window 2
        taskbar.remove_window(102);
        assert!(!taskbar.items[0].is_running());
    }

    #[test]
    fn test_badge_notifications() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        taskbar.set_badge_count("term", Some(3));
        assert_eq!(taskbar.items[1].badge_count, Some(3));

        taskbar.set_badge_count("term", None);
        assert_eq!(taskbar.items[1].badge_count, None);
    }

    #[test]
    fn test_magnification_physics_and_bounce() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);

        let icon_x = taskbar.items[0].base_bounds.x + 20;
        let pos = Point::new(icon_x, 24);

        // Move over first icon triggers magnification
        taskbar.handle_mouse_move(pos);
        assert!(taskbar.items[0].target_scale > 1.2);

        // Trigger bounce animation
        taskbar.items[0].trigger_launch_bounce();
        assert!(taskbar.items[0].is_bouncing);
        assert!(taskbar.items[0].bounce_velocity < 0.0);

        // Tick animation
        taskbar.tick_animation(1.0);
        assert!(taskbar.items[0].bounce_offset_y < 0.0);
    }

    #[test]
    fn test_live_window_thumbnail_preview_and_close() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);

        let thumb = WindowThumbnail::create_test_thumbnail(42, "My Code.rs", 160, 100, Color::rgb(0, 120, 215));
        taskbar.sync_window_surface("edit", thumb);

        // Move over edit icon opens live window preview
        let edit_pos = Point::new(taskbar.items[2].base_bounds.x + 10, 20);
        taskbar.handle_mouse_move(edit_pos);

        assert!(taskbar.hover_preview.is_some());
        let preview = taskbar.hover_preview.as_ref().unwrap();
        assert_eq!(preview.app_id, "edit");
        assert_eq!(preview.thumbnails.len(), 1);

        // Click close button on thumbnail
        let close_pt = Point::new(
            preview.thumbnails[0].close_button_bounds.x + 4,
            preview.thumbnails[0].close_button_bounds.y + 4,
        );
        let action = taskbar.handle_mouse_click(close_pt);
        assert_eq!(action, TaskbarAction::CloseWindow(42));
    }

    #[test]
    fn test_taskbar_rendering() {
        let mut taskbar = Taskbar::new(1024, ThemeMode::Dark, AccentPalette::FluentBlue);
        let thumb = WindowThumbnail::create_test_thumbnail(99, "Terminal", 160, 100, Color::rgb(0, 255, 128));
        taskbar.sync_window_surface("term", thumb);
        taskbar.set_active_window(Some(99));

        let mut surface = BufferSurface::new(1024, 48);
        surface.clear(Color::rgb(10, 10, 10));

        taskbar.render(&mut surface);

        // Ensure taskbar background was painted
        assert_ne!(surface.get_pixel(100, 24), Color::rgb(10, 10, 10));
    }
}
