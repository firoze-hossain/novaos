//! userland/libs/nova-ui/src/widget.rs
//!
//! Declarative Widgets & Reactive State Bindings for NovaOS.
//! Implements Button, Slider, TextInput, List, Table, Tabs, and Modal.

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

use crate::layout::{Point, Rect};
use crate::render::{Color, WHITE};
use crate::scene::{PaintCommand, SceneNode};

// Standard NovaOS Palette Constants
pub const PRIMARY_ACCENT: Color = Color::rgb(0, 120, 215);      // Fluent Blue
pub const PRIMARY_HOVER: Color = Color::rgb(24, 134, 230);
pub const PRIMARY_PRESSED: Color = Color::rgb(0, 102, 184);
pub const SURFACE_BG: Color = Color::rgb(32, 32, 32);           // Dark Theme Surface
pub const CARD_BG: Color = Color::rgb(45, 45, 45);              // Card Background
pub const BORDER_COLOR: Color = Color::rgba(255, 255, 255, 40);
pub const TEXT_PRIMARY: Color = Color::rgb(240, 240, 240);
pub const TEXT_MUTED: Color = Color::rgb(160, 160, 160);

// ============================================================================
// 1. Button Widget
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Normal,
    Hovered,
    Pressed,
    Disabled,
}

#[derive(Clone, Debug)]
pub struct ButtonWidget {
    pub label: String,
    pub state: ButtonState,
    pub corner_radius: i32,
    pub bg_color: Color,
    pub text_color: Color,
}

impl ButtonWidget {
    pub fn new(label: &str) -> Self {
        Self {
            label: String::from(label),
            state: ButtonState::Normal,
            corner_radius: 6,
            bg_color: PRIMARY_ACCENT,
            text_color: WHITE,
        }
    }

    pub fn render_to_node(&self, bounds: Rect, node: &mut SceneNode) {
        node.bounds = bounds;
        node.commands.clear();

        let current_bg = match self.state {
            ButtonState::Normal => self.bg_color,
            ButtonState::Hovered => PRIMARY_HOVER,
            ButtonState::Pressed => PRIMARY_PRESSED,
            ButtonState::Disabled => Color::rgb(70, 70, 70),
        };

        // 1. Drop shadow (if not disabled or pressed)
        if self.state == ButtonState::Normal || self.state == ButtonState::Hovered {
            node.commands.push(PaintCommand::BoxShadow(
                bounds, 0, 2, 6, 0, Color::rgba(0, 0, 0, 80),
            ));
        }

        // 2. Rounded background
        node.commands.push(PaintCommand::FillRoundedRect(
            bounds, self.corner_radius, current_bg,
        ));

        // 3. Subtle highlight border
        node.commands.push(PaintCommand::StrokeRoundedRect(
            bounds, self.corner_radius, 1, BORDER_COLOR,
        ));

        // 4. Centered label text (approximate 8px per char, 16px high)
        let text_w = self.label.len() as i32 * 8;
        let text_x = bounds.x + (bounds.width - text_w) / 2;
        let text_y = bounds.y + (bounds.height - 16) / 2;

        node.commands.push(PaintCommand::Text(
            self.label.clone(),
            Point::new(text_x, text_y),
            if self.state == ButtonState::Disabled { TEXT_MUTED } else { self.text_color },
        ));
    }
}

// ============================================================================
// 2. Slider Widget
// ============================================================================

#[derive(Clone, Debug)]
pub struct SliderWidget {
    pub min: f32,
    pub max: f32,
    pub value: f32,
    pub step: Option<f32>,
    pub is_dragging: bool,
    pub track_height: i32,
    pub thumb_radius: i32,
}

impl SliderWidget {
    pub fn new(min: f32, max: f32, initial: f32) -> Self {
        Self {
            min,
            max,
            value: initial.clamp(min, max),
            step: None,
            is_dragging: false,
            track_height: 4,
            thumb_radius: 8,
        }
    }

    pub fn set_from_pos(&mut self, pos_x: i32, bounds: Rect) {
        let left = bounds.x + self.thumb_radius;
        let right = bounds.x + bounds.width - self.thumb_radius;
        if right <= left {
            return;
        }
        let ratio = ((pos_x - left) as f32 / (right - left) as f32).clamp(0.0, 1.0);
        let mut raw_val = self.min + ratio * (self.max - self.min);
        if let Some(s) = self.step {
            let steps = ((raw_val / s) + 0.5f32) as i32;
            raw_val = steps as f32 * s;
        }
        self.value = raw_val.clamp(self.min, self.max);
    }

    pub fn render_to_node(&self, bounds: Rect, node: &mut SceneNode) {
        node.bounds = bounds;
        node.commands.clear();

        let track_y = bounds.y + (bounds.height - self.track_height) / 2;
        let track_w = bounds.width - self.thumb_radius * 2;
        let track_rect = Rect::new(bounds.x + self.thumb_radius, track_y, track_w, self.track_height);

        // Track background
        node.commands.push(PaintCommand::FillRoundedRect(
            track_rect, self.track_height / 2, Color::rgb(60, 60, 60),
        ));

        // Active track portion
        let norm = if self.max > self.min {
            (self.value - self.min) / (self.max - self.min)
        } else {
            0.0
        };
        let active_w = (track_w as f32 * norm) as i32;
        let active_rect = Rect::new(track_rect.x, track_y, active_w, self.track_height);
        node.commands.push(PaintCommand::FillRoundedRect(
            active_rect, self.track_height / 2, PRIMARY_ACCENT,
        ));

        // Thumb position
        let thumb_x = track_rect.x + active_w - self.thumb_radius;
        let thumb_y = bounds.y + (bounds.height - self.thumb_radius * 2) / 2;
        let thumb_rect = Rect::new(thumb_x, thumb_y, self.thumb_radius * 2, self.thumb_radius * 2);

        // Thumb shadow and circle
        node.commands.push(PaintCommand::BoxShadow(
            thumb_rect, 0, 1, 4, 0, Color::rgba(0, 0, 0, 100),
        ));
        node.commands.push(PaintCommand::FillRoundedRect(
            thumb_rect, self.thumb_radius, WHITE,
        ));
    }
}

// ============================================================================
// 3. TextInput Widget
// ============================================================================

#[derive(Clone, Debug)]
pub struct TextInputWidget {
    pub text: String,
    pub placeholder: String,
    pub cursor_pos: usize,
    pub is_focused: bool,
}

impl TextInputWidget {
    pub fn new(placeholder: &str) -> Self {
        Self {
            text: String::new(),
            placeholder: String::from(placeholder),
            cursor_pos: 0,
            is_focused: false,
        }
    }

    pub fn insert_char(&mut self, ch: char) {
        if self.cursor_pos <= self.text.len() {
            self.text.insert(self.cursor_pos, ch);
            self.cursor_pos += 1;
        }
    }

    pub fn delete_backward(&mut self) {
        if self.cursor_pos > 0 && self.cursor_pos <= self.text.len() {
            self.cursor_pos -= 1;
            self.text.remove(self.cursor_pos);
        }
    }

    pub fn render_to_node(&self, bounds: Rect, node: &mut SceneNode) {
        node.bounds = bounds;
        node.commands.clear();

        // 1. Container background
        node.commands.push(PaintCommand::FillRoundedRect(
            bounds, 4, Color::rgb(40, 40, 40),
        ));

        // 2. Focused accent border vs normal subtle border
        let border = if self.is_focused { PRIMARY_ACCENT } else { BORDER_COLOR };
        let border_w = if self.is_focused { 2 } else { 1 };
        node.commands.push(PaintCommand::StrokeRoundedRect(
            bounds, 4, border_w, border,
        ));

        // 3. Render Text or Placeholder
        let pad_x = 8;
        let text_y = bounds.y + (bounds.height - 16) / 2;

        if self.text.is_empty() {
            node.commands.push(PaintCommand::Text(
                self.placeholder.clone(),
                Point::new(bounds.x + pad_x, text_y),
                TEXT_MUTED,
            ));
        } else {
            node.commands.push(PaintCommand::Text(
                self.text.clone(),
                Point::new(bounds.x + pad_x, text_y),
                TEXT_PRIMARY,
            ));
        }

        // 4. Cursor line if focused
        if self.is_focused {
            let cur_x = bounds.x + pad_x + (self.cursor_pos as i32 * 8);
            let cur_rect = Rect::new(cur_x, text_y, 2, 16);
            node.commands.push(PaintCommand::FillRect(cur_rect, PRIMARY_ACCENT));
        }
    }
}

// ============================================================================
// 4. List Widget
// ============================================================================

#[derive(Clone, Debug)]
pub struct ListWidget {
    pub items: Vec<String>,
    pub selected_index: Option<usize>,
    pub item_height: i32,
    pub scroll_offset: i32,
}

impl ListWidget {
    pub fn new(items: Vec<String>) -> Self {
        Self {
            items,
            selected_index: None,
            item_height: 32,
            scroll_offset: 0,
        }
    }

    pub fn select_at_pos(&mut self, rel_y: i32) {
        let idx = (rel_y + self.scroll_offset) / self.item_height;
        if idx >= 0 && (idx as usize) < self.items.len() {
            self.selected_index = Some(idx as usize);
        }
    }

    pub fn render_to_node(&self, bounds: Rect, node: &mut SceneNode) {
        node.bounds = bounds;
        node.commands.clear();
        node.clip = Some(bounds);

        // Background card
        node.commands.push(PaintCommand::FillRoundedRect(bounds, 6, CARD_BG));

        let mut y = bounds.y - self.scroll_offset;
        for (i, item) in self.items.iter().enumerate() {
            if y + self.item_height >= bounds.y && y < bounds.y + bounds.height {
                let item_rect = Rect::new(bounds.x, y, bounds.width, self.item_height);

                if Some(i) == self.selected_index {
                    // Highlight selected item
                    node.commands.push(PaintCommand::FillRoundedRect(
                        item_rect, 4, PRIMARY_ACCENT,
                    ));
                    node.commands.push(PaintCommand::Text(
                        item.clone(),
                        Point::new(bounds.x + 12, y + (self.item_height - 16) / 2),
                        WHITE,
                    ));
                } else {
                    node.commands.push(PaintCommand::Text(
                        item.clone(),
                        Point::new(bounds.x + 12, y + (self.item_height - 16) / 2),
                        TEXT_PRIMARY,
                    ));
                }
            }
            y += self.item_height;
        }
    }
}

// ============================================================================
// 5. Table Widget
// ============================================================================

#[derive(Clone, Debug)]
pub struct TableWidget {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub col_widths: Vec<i32>,
    pub selected_row: Option<usize>,
    pub row_height: i32,
}

impl TableWidget {
    pub fn new(headers: Vec<String>, rows: Vec<Vec<String>>, col_widths: Vec<i32>) -> Self {
        Self {
            headers,
            rows,
            col_widths,
            selected_row: None,
            row_height: 28,
        }
    }

    pub fn render_to_node(&self, bounds: Rect, node: &mut SceneNode) {
        node.bounds = bounds;
        node.commands.clear();

        // 1. Table Container
        node.commands.push(PaintCommand::FillRoundedRect(bounds, 6, CARD_BG));
        node.commands.push(PaintCommand::StrokeRoundedRect(bounds, 6, 1, BORDER_COLOR));

        // 2. Header Bar
        let header_rect = Rect::new(bounds.x, bounds.y, bounds.width, self.row_height);
        node.commands.push(PaintCommand::FillRoundedRect(header_rect, 4, Color::rgb(55, 55, 55)));

        let mut x_off = bounds.x + 8;
        for (col_idx, header) in self.headers.iter().enumerate() {
            let w = *self.col_widths.get(col_idx).unwrap_or(&80);
            node.commands.push(PaintCommand::Text(
                header.clone(),
                Point::new(x_off, bounds.y + 6),
                WHITE,
            ));
            x_off += w;
        }

        // 3. Rows (Zebra striped)
        let mut cur_y = bounds.y + self.row_height;
        for (r_idx, row) in self.rows.iter().enumerate() {
            if cur_y + self.row_height > bounds.y + bounds.height {
                break;
            }
            let row_rect = Rect::new(bounds.x, cur_y, bounds.width, self.row_height);

            if Some(r_idx) == self.selected_row {
                node.commands.push(PaintCommand::FillRect(row_rect, PRIMARY_ACCENT));
            } else if r_idx % 2 == 1 {
                node.commands.push(PaintCommand::FillRect(row_rect, Color::rgb(38, 38, 38)));
            }

            let mut col_x = bounds.x + 8;
            for (c_idx, cell) in row.iter().enumerate() {
                let w = *self.col_widths.get(c_idx).unwrap_or(&80);
                node.commands.push(PaintCommand::Text(
                    cell.clone(),
                    Point::new(col_x, cur_y + 6),
                    if Some(r_idx) == self.selected_row { WHITE } else { TEXT_PRIMARY },
                ));
                col_x += w;
            }
            cur_y += self.row_height;
        }
    }
}

// ============================================================================
// 6. Tabs Widget
// ============================================================================

#[derive(Clone, Debug)]
pub struct TabsWidget {
    pub tabs: Vec<String>,
    pub active_tab: usize,
    pub tab_height: i32,
}

impl TabsWidget {
    pub fn new(tabs: Vec<String>) -> Self {
        Self {
            tabs,
            active_tab: 0,
            tab_height: 36,
        }
    }

    pub fn select_tab_at_x(&mut self, rel_x: i32, total_width: i32) {
        if self.tabs.is_empty() || total_width <= 0 {
            return;
        }
        let tab_w = total_width / self.tabs.len() as i32;
        if tab_w > 0 {
            let idx = (rel_x / tab_w) as usize;
            if idx < self.tabs.len() {
                self.active_tab = idx;
            }
        }
    }

    pub fn render_to_node(&self, bounds: Rect, node: &mut SceneNode) {
        node.bounds = bounds;
        node.commands.clear();

        let tab_count = self.tabs.len() as i32;
        if tab_count == 0 {
            return;
        }
        let tab_w = bounds.width / tab_count;

        for (i, tab) in self.tabs.iter().enumerate() {
            let tx = bounds.x + (i as i32) * tab_w;
            let tab_rect = Rect::new(tx, bounds.y, tab_w, self.tab_height);

            let is_active = i == self.active_tab;
            let color = if is_active { WHITE } else { TEXT_MUTED };

            let text_x = tx + (tab_w - (tab.len() as i32 * 8)) / 2;
            let text_y = bounds.y + (self.tab_height - 16) / 2;

            node.commands.push(PaintCommand::Text(tab.clone(), Point::new(text_x, text_y), color));

            if is_active {
                // Active indicator line
                let indicator = Rect::new(tx + 4, bounds.y + self.tab_height - 3, tab_w - 8, 3);
                node.commands.push(PaintCommand::FillRoundedRect(indicator, 2, PRIMARY_ACCENT));
            }
        }
    }
}

// ============================================================================
// 7. Modal Widget
// ============================================================================

#[derive(Clone, Debug)]
pub struct ModalWidget {
    pub title: String,
    pub is_open: bool,
    pub card_width: i32,
    pub card_height: i32,
}

impl ModalWidget {
    pub fn new(title: &str, width: i32, height: i32) -> Self {
        Self {
            title: String::from(title),
            is_open: false,
            card_width: width,
            card_height: height,
        }
    }

    pub fn open(&mut self) {
        self.is_open = true;
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }

    pub fn render_to_node(&self, screen_bounds: Rect, node: &mut SceneNode) {
        node.bounds = screen_bounds;
        node.commands.clear();

        if !self.is_open {
            return;
        }

        // 1. Scrim backdrop with acrylic blur & dimming
        node.commands.push(PaintCommand::BackdropFilter(
            screen_bounds,
            8,
            Color::rgba(0, 0, 0, 140),
        ));

        // 2. Centered Modal Card
        let card_x = screen_bounds.x + (screen_bounds.width - self.card_width) / 2;
        let card_y = screen_bounds.y + (screen_bounds.height - self.card_height) / 2;
        let card_rect = Rect::new(card_x, card_y, self.card_width, self.card_height);

        // Card Drop Shadow
        node.commands.push(PaintCommand::BoxShadow(
            card_rect, 0, 10, 24, 0, Color::rgba(0, 0, 0, 160),
        ));

        // Card Body
        node.commands.push(PaintCommand::FillRoundedRect(card_rect, 10, CARD_BG));
        node.commands.push(PaintCommand::StrokeRoundedRect(card_rect, 10, 1, BORDER_COLOR));

        // Title Bar
        node.commands.push(PaintCommand::Text(
            self.title.clone(),
            Point::new(card_x + 16, card_y + 16),
            WHITE,
        ));

        // Close Button "X"
        let close_x = card_x + self.card_width - 28;
        node.commands.push(PaintCommand::Text(
            String::from("X"),
            Point::new(close_x, card_y + 16),
            TEXT_MUTED,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_button_widget_states() {
        let mut btn = ButtonWidget::new("Save File");
        let mut node = SceneNode::new(crate::scene::NodeId(1), "btn");
        let bounds = Rect::new(10, 10, 120, 40);

        btn.render_to_node(bounds, &mut node);
        assert!(!node.commands.is_empty());
        assert_eq!(btn.state, ButtonState::Normal);

        btn.state = ButtonState::Pressed;
        btn.render_to_node(bounds, &mut node);
        assert_eq!(btn.state, ButtonState::Pressed);
    }

    #[test]
    fn test_slider_widget_computation() {
        let mut slider = SliderWidget::new(0.0, 100.0, 50.0);
        let bounds = Rect::new(0, 0, 200, 30);
        let mut node = SceneNode::new(crate::scene::NodeId(2), "slider");

        slider.render_to_node(bounds, &mut node);
        assert_eq!(slider.value, 50.0);

        // Drag to right edge
        slider.set_from_pos(190, bounds);
        assert!(slider.value > 90.0);

        // Drag to left edge
        slider.set_from_pos(5, bounds);
        assert_eq!(slider.value, 0.0);
    }

    #[test]
    fn test_text_input_editing() {
        let mut input = TextInputWidget::new("Enter name...");
        input.insert_char('N');
        input.insert_char('o');
        input.insert_char('v');
        input.insert_char('a');
        assert_eq!(input.text, "Nova");
        assert_eq!(input.cursor_pos, 4);

        input.delete_backward();
        assert_eq!(input.text, "Nov");
        assert_eq!(input.cursor_pos, 3);
    }

    #[test]
    fn test_tabs_and_modal() {
        let mut tabs = TabsWidget::new(vec![String::from("General"), String::from("Network")]);
        assert_eq!(tabs.active_tab, 0);
        tabs.select_tab_at_x(150, 200);
        assert_eq!(tabs.active_tab, 1);

        let mut modal = ModalWidget::new("Settings", 300, 200);
        assert!(!modal.is_open);
        modal.open();
        assert!(modal.is_open);
        modal.close();
        assert!(!modal.is_open);
    }
}
