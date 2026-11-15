//! userland/desktop/apps/nova-settings/src/view.rs
//!
//! Windows 11 + macOS Synthesized Settings UI Shell.
//! Coordinates layout geometry, sidebar navigation, real-time search overlays,
//! card grouping, interactive widgets (toggles, sliders, pickers), and BufferSurface rendering.

#[cfg(not(test))]
use alloc::{format, string::String};

#[cfg(test)]
use std::{format, string::String};

use crate::types::{PanelCategory, SearchResult, SettingItem, SettingValue};
use nova_ui::prelude::{BufferSurface, Color, Point, Rect, WHITE};

/// Hit test target result.
#[derive(Clone, Debug, PartialEq)]
pub enum HitTarget {
    Category(PanelCategory),
    SearchBox,
    ClearSearch,
    SearchResult(usize),
    ToggleSwitch(String),
    NextChoice(String),
    SliderDrag { key: String, ratio: f32 },
    ActionButton(String),
}

/// Palette tokens for Settings UI theme.
pub struct SettingsThemeTokens {
    pub bg_window: Color,
    pub bg_sidebar: Color,
    pub bg_card: Color,
    pub border_color: Color,
    pub accent_primary: Color,
    pub accent_hover: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub toggle_off_track: Color,
    pub toggle_knob: Color,
}

impl SettingsThemeTokens {
    pub const fn dark() -> Self {
        Self {
            bg_window: Color::rgb(30, 30, 36),
            bg_sidebar: Color::rgb(24, 24, 28),
            bg_card: Color::rgb(38, 38, 46),
            border_color: Color::rgba(255, 255, 255, 20),
            accent_primary: Color::rgb(0, 120, 212), // Fluent Blue
            accent_hover: Color::rgb(24, 144, 236),
            text_primary: Color::rgb(245, 245, 250),
            text_secondary: Color::rgb(160, 160, 175),
            toggle_off_track: Color::rgb(60, 60, 70),
            toggle_knob: Color::rgb(240, 240, 245),
        }
    }
}

/// Settings UI Layout and Render Engine.
pub struct SettingsView {
    pub width: i32,
    pub height: i32,
    pub sidebar_width: i32,
    pub tokens: SettingsThemeTokens,
}

impl Default for SettingsView {
    fn default() -> Self {
        Self::new(960, 640)
    }
}

impl SettingsView {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            sidebar_width: 260,
            tokens: SettingsThemeTokens::dark(),
        }
    }

    /// Renders the entire settings window: sidebar, search, cards, controls.
    pub fn render(
        &self,
        surface: &mut BufferSurface,
        active_cat: PanelCategory,
        search_query: &str,
        search_results: &[SearchResult],
        items: &[SettingItem],
    ) {
        // 1. Clear background
        surface.clear(self.tokens.bg_window);

        // 2. Render Left Sidebar
        let sidebar_rect = Rect::new(0, 0, self.sidebar_width, self.height);
        surface.fill_rect(sidebar_rect, self.tokens.bg_sidebar);
        surface.fill_rect(
            Rect::new(self.sidebar_width - 1, 0, 1, self.height),
            self.tokens.border_color,
        );

        // 2a. User Profile Pill
        let user_card_rect = Rect::new(16, 16, self.sidebar_width - 32, 44);
        surface.fill_rounded_rect(user_card_rect, 6, self.tokens.bg_card);
        // Avatar circle
        surface.fill_rounded_rect(Rect::new(24, 22, 32, 32), 16, self.tokens.accent_primary);
        surface.draw_text("NA", 32, 30, WHITE);
        // User labels
        surface.draw_text("Administrator", 64, 22, self.tokens.text_primary);
        surface.draw_text("Local Account", 64, 38, self.tokens.text_secondary);

        // 2b. Search Box
        let search_rect = Rect::new(16, 70, self.sidebar_width - 32, 32);
        surface.fill_rounded_rect(search_rect, 6, Color::rgb(42, 42, 50));
        surface.stroke_rounded_rect(search_rect, 6, 1, self.tokens.border_color);
        if search_query.is_empty() {
            surface.draw_text("Search settings...", 26, 78, self.tokens.text_secondary);
        } else {
            surface.draw_text(search_query, 26, 78, self.tokens.text_primary);
            // Clear icon
            surface.draw_text("X", self.sidebar_width - 36, 78, self.tokens.text_secondary);
        }

        // 2c. Category Navigation List
        let cat_start_y = 114;
        let cat_item_height = 40;
        let all_cats = PanelCategory::all();

        for (idx, cat) in all_cats.iter().enumerate() {
            let item_y = cat_start_y + (idx as i32) * (cat_item_height + 4);
            let item_rect = Rect::new(12, item_y, self.sidebar_width - 24, cat_item_height);

            let is_selected = *cat == active_cat;
            if is_selected {
                // Accent pill highlight
                surface.fill_rounded_rect(
                    item_rect,
                    6,
                    Color::rgba(self.tokens.accent_primary.r, self.tokens.accent_primary.g, self.tokens.accent_primary.b, 60),
                );
                // Active indicator stripe on left
                surface.fill_rounded_rect(Rect::new(12, item_y + 8, 3, cat_item_height - 16), 2, self.tokens.accent_primary);
            }

            // Category text
            let text_color = if is_selected { WHITE } else { self.tokens.text_primary };
            surface.draw_text(cat.title(), 26, item_y + 12, text_color);
        }

        // 3. Render Main Content Pane
        let content_x = self.sidebar_width + 24;
        let content_w = self.width - content_x - 24;

        if !search_query.trim().is_empty() {
            // 3a. Search Results Overlay
            let header_text = format!("Search results for: \"{}\"", search_query);
            surface.draw_text(&header_text, content_x, 24, self.tokens.text_primary);
            let count_text = format!("Found {} setting items", search_results.len());
            surface.draw_text(&count_text, content_x, 44, self.tokens.text_secondary);

            let res_start_y = 76;
            for (idx, res) in search_results.iter().take(9).enumerate() {
                let card_y = res_start_y + (idx as i32) * 54;
                let card_rect = Rect::new(content_x, card_y, content_w, 48);
                surface.fill_rounded_rect(card_rect, 6, self.tokens.bg_card);
                surface.stroke_rounded_rect(card_rect, 6, 1, self.tokens.border_color);

                // Title & breadcrumb
                surface.draw_text(&res.title, content_x + 14, card_y + 8, self.tokens.text_primary);
                surface.draw_text(&res.breadcrumb, content_x + 14, card_y + 26, self.tokens.accent_primary);

                // Jump button
                let btn_rect = Rect::new(content_x + content_w - 74, card_y + 12, 60, 24);
                surface.fill_rounded_rect(btn_rect, 4, self.tokens.accent_primary);
                surface.draw_text("Open", content_x + content_w - 60, card_y + 16, WHITE);
            }
        } else {
            // 3b. Normal Category View
            let breadcrumb = format!("Settings > {}", active_cat.title());
            surface.draw_text(&breadcrumb, content_x, 20, self.tokens.accent_primary);
            surface.draw_text(active_cat.title(), content_x, 38, self.tokens.text_primary);
            surface.draw_text(active_cat.summary(), content_x, 58, self.tokens.text_secondary);

            // Group items into cards
            let mut current_y = 86;
            let mut current_subgroup = "";

            for item in items {
                if item.subgroup != current_subgroup {
                    current_subgroup = &item.subgroup;
                    // Subgroup section header
                    surface.draw_text(current_subgroup, content_x, current_y + 6, self.tokens.text_primary);
                    current_y += 28;
                }

                // Setting Row Card
                let row_rect = Rect::new(content_x, current_y, content_w, 44);
                surface.fill_rounded_rect(row_rect, 6, self.tokens.bg_card);
                surface.stroke_rounded_rect(row_rect, 6, 1, self.tokens.border_color);

                // Title & description
                surface.draw_text(&item.title, content_x + 14, current_y + 8, self.tokens.text_primary);
                // Truncate description for display
                let desc_len = item.description.len().min(45);
                surface.draw_text(&item.description[..desc_len], content_x + 14, current_y + 24, self.tokens.text_secondary);

                // Right-aligned control
                self.render_control(surface, item, content_x + content_w, current_y);

                current_y += 50;
                if current_y > self.height - 60 {
                    break;
                }
            }
        }
    }

    /// Renders the appropriate interactive control for a setting row.
    fn render_control(&self, surface: &mut BufferSurface, item: &SettingItem, right_edge_x: i32, row_y: i32) {
        match &item.value {
            SettingValue::Bool(is_on) => {
                let track_w = 44;
                let track_h = 22;
                let track_x = right_edge_x - track_w - 14;
                let track_y = row_y + 11;
                let track_rect = Rect::new(track_x, track_y, track_w, track_h);

                if *is_on {
                    surface.fill_rounded_rect(track_rect, 11, self.tokens.accent_primary);
                    // Knob on right
                    surface.fill_rounded_rect(Rect::new(track_x + 24, track_y + 3, 16, 16), 8, self.tokens.toggle_knob);
                } else {
                    surface.fill_rounded_rect(track_rect, 11, self.tokens.toggle_off_track);
                    surface.stroke_rounded_rect(track_rect, 11, 1, self.tokens.border_color);
                    // Knob on left
                    surface.fill_rounded_rect(Rect::new(track_x + 4, track_y + 3, 16, 16), 8, self.tokens.toggle_knob);
                }
            }
            SettingValue::Choice { selected, options } => {
                let btn_w = 170;
                let btn_h = 26;
                let btn_x = right_edge_x - btn_w - 14;
                let btn_y = row_y + 9;
                let btn_rect = Rect::new(btn_x, btn_y, btn_w, btn_h);

                surface.fill_rounded_rect(btn_rect, 4, Color::rgb(48, 48, 58));
                surface.stroke_rounded_rect(btn_rect, 4, 1, self.tokens.border_color);

                if let Some(opt) = options.get(*selected) {
                    let text_len = opt.len().min(17);
                    surface.draw_text(&opt[..text_len], btn_x + 8, btn_y + 5, self.tokens.text_primary);
                }
                surface.draw_text("v", btn_x + btn_w - 16, btn_y + 5, self.tokens.text_secondary);
            }
            SettingValue::Int(val) => {
                let slider_w = 110;
                let slider_x = right_edge_x - slider_w - 60;
                let slider_y = row_y + 18;

                // Slider track
                surface.fill_rounded_rect(Rect::new(slider_x, slider_y, slider_w, 6), 3, self.tokens.toggle_off_track);
                // Filled progress
                let progress_w = ((val.clamp(&0, &100) * slider_w as i64) / 100) as i32;
                if progress_w > 0 {
                    surface.fill_rounded_rect(Rect::new(slider_x, slider_y, progress_w, 6), 3, self.tokens.accent_primary);
                }
                // Thumb knob
                surface.fill_rounded_rect(Rect::new(slider_x + progress_w - 4, slider_y - 4, 14, 14), 7, WHITE);

                // Value label
                let val_text = format!("{}%", val);
                surface.draw_text(&val_text, right_edge_x - 48, row_y + 14, self.tokens.text_primary);
            }
            SettingValue::Text(text) => {
                let text_w = 150;
                let text_x = right_edge_x - text_w - 14;
                let text_y = row_y + 9;
                let text_rect = Rect::new(text_x, text_y, text_w, 26);

                surface.fill_rounded_rect(text_rect, 4, Color::rgb(45, 45, 54));
                let disp_len = text.len().min(16);
                surface.draw_text(&text[..disp_len], text_x + 8, text_y + 5, self.tokens.text_primary);
            }
            SettingValue::Float(f) => {
                let val_text = format!("{:.1}", f);
                surface.draw_text(&val_text, right_edge_x - 50, row_y + 14, self.tokens.text_primary);
            }
            SettingValue::ColorRgb(r, g, b) => {
                let box_rect = Rect::new(right_edge_x - 38, row_y + 10, 24, 24);
                surface.fill_rounded_rect(box_rect, 12, Color::rgb(*r, *g, *b));
                surface.stroke_rounded_rect(box_rect, 12, 1, WHITE);
            }
        }
    }

    /// Hit-tests a pointer click to identify clicked category, search box, or setting control.
    pub fn hit_test(
        &self,
        pt: Point,
        _active_cat: PanelCategory,
        search_query: &str,
        search_results: &[SearchResult],
        items: &[SettingItem],
    ) -> Option<HitTarget> {
        // 1. Sidebar hit test
        if pt.x < self.sidebar_width {
            // Search box
            let search_rect = Rect::new(16, 70, self.sidebar_width - 32, 32);
            if search_rect.contains(pt) {
                if !search_query.is_empty() && pt.x >= self.sidebar_width - 40 {
                    return Some(HitTarget::ClearSearch);
                }
                return Some(HitTarget::SearchBox);
            }

            // Categories
            let cat_start_y = 114;
            let cat_item_height = 40;
            let all_cats = PanelCategory::all();

            for (idx, cat) in all_cats.iter().enumerate() {
                let item_y = cat_start_y + (idx as i32) * (cat_item_height + 4);
                let item_rect = Rect::new(12, item_y, self.sidebar_width - 24, cat_item_height);
                if item_rect.contains(pt) {
                    return Some(HitTarget::Category(*cat));
                }
            }

            return None;
        }

        // 2. Main content pane hit test
        let content_x = self.sidebar_width + 24;
        let content_w = self.width - content_x - 24;

        if !search_query.trim().is_empty() {
            // Search results hit test
            let res_start_y = 76;
            for (idx, _) in search_results.iter().take(9).enumerate() {
                let card_y = res_start_y + (idx as i32) * 54;
                let card_rect = Rect::new(content_x, card_y, content_w, 48);
                if card_rect.contains(pt) {
                    return Some(HitTarget::SearchResult(idx));
                }
            }
        } else {
            // Category setting rows hit test
            let mut current_y = 86;
            let mut current_subgroup = "";

            for item in items {
                if item.subgroup != current_subgroup {
                    current_subgroup = &item.subgroup;
                    current_y += 28;
                }

                let row_rect = Rect::new(content_x, current_y, content_w, 44);
                if row_rect.contains(pt) {
                    let right_edge_x = content_x + content_w;
                    match &item.value {
                        SettingValue::Bool(_) => {
                            let track_rect = Rect::new(right_edge_x - 58, current_y + 8, 48, 28);
                            if track_rect.contains(pt) {
                                return Some(HitTarget::ToggleSwitch(item.key.clone()));
                            }
                        }
                        SettingValue::Choice { .. } => {
                            let btn_rect = Rect::new(right_edge_x - 184, current_y + 6, 175, 32);
                            if btn_rect.contains(pt) {
                                return Some(HitTarget::NextChoice(item.key.clone()));
                            }
                        }
                        SettingValue::Int(_) => {
                            let slider_x = right_edge_x - 110 - 60;
                            let slider_rect = Rect::new(slider_x - 8, current_y + 10, 126, 24);
                            if slider_rect.contains(pt) {
                                let ratio = ((pt.x - slider_x) as f32 / 110.0).clamp(0.0, 1.0);
                                return Some(HitTarget::SliderDrag {
                                    key: item.key.clone(),
                                    ratio,
                                });
                            }
                        }
                        _ => {}
                    }
                }

                current_y += 50;
                if current_y > self.height - 60 {
                    break;
                }
            }
        }

        None
    }
}
