//! userland/desktop/nova-search/src/lib.rs
//!
//! macOS Spotlight-Style Universal Instant Search & Calculator for NovaOS.
//!
//! Features:
//! - Global centered floating search modal with acrylic blur and Level 4 elevation shadow.
//! - Super+Space / Ctrl+Space shortcut activation.
//! - Sub-millisecond fuzzy search indexer across Apps, Documents, Settings, and Shell commands.
//! - Real-time inline math calculation and natural unit conversions.
//! - Full keyboard arrow navigation and Enter action execution.

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
extern crate alloc;
#[cfg(not(test))]
use alloc::{string::String, vec::Vec};

#[cfg(test)]
use std::{string::String, vec::Vec};

pub mod indexer;
pub use indexer::*;

use nova_ui::prelude::*;

pub const SPOTLIGHT_WIDTH: i32 = 640;
pub const SEARCH_BAR_HEIGHT: i32 = 54;
pub const RESULT_ITEM_HEIGHT: i32 = 48;

// ============================================================================
// Spotlight Overlay Component
// ============================================================================

pub struct SpotlightOverlay {
    pub screen_w: i32,
    pub screen_h: i32,
    pub modal_width: i32,
    pub is_visible: bool,
    pub query: String,
    pub results: Vec<SearchItem>,
    pub selected_index: usize,
    pub indexer: SearchIndex,
    pub theme_mode: ThemeMode,
    pub tokens: ColorTokens,
    pub elevation: [ElevationTier; 6],
    pub corners: CornerTokens,
}

impl SpotlightOverlay {
    pub fn new(screen_w: i32, screen_h: i32, mode: ThemeMode, accent: AccentPalette) -> Self {
        let tokens = ColorTokens::build(mode, accent);
        let elevation = get_elevation_tiers(mode);
        let corners = CornerTokens::default();
        let indexer = SearchIndex::new();

        Self {
            screen_w,
            screen_h,
            modal_width: SPOTLIGHT_WIDTH,
            is_visible: false,
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            indexer,
            theme_mode: mode,
            tokens,
            elevation,
            corners,
        }
    }

    pub fn toggle_visibility(&mut self) {
        if self.is_visible {
            self.hide();
        } else {
            self.show();
        }
    }

    pub fn show(&mut self) {
        self.is_visible = true;
        self.query.clear();
        self.results = self.indexer.query("", 6);
        self.selected_index = 0;
    }

    pub fn hide(&mut self) {
        self.is_visible = false;
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
    }

    /// Handles typing character input
    pub fn handle_char_input(&mut self, c: char) {
        if !self.is_visible {
            return;
        }
        self.query.push(c);
        self.update_results();
    }

    /// Handles backspace key
    pub fn handle_backspace(&mut self) {
        if !self.is_visible {
            return;
        }
        self.query.pop();
        self.update_results();
    }

    /// Updates search query string directly
    pub fn set_query(&mut self, new_query: &str) {
        self.query = String::from(new_query);
        self.update_results();
    }

    fn update_results(&mut self) {
        self.results = self.indexer.query(&self.query, 6);
        self.selected_index = 0;
    }

    /// Arrow navigation: move selection up
    pub fn handle_arrow_up(&mut self) {
        if !self.results.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.results.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    /// Arrow navigation: move selection down
    pub fn handle_arrow_down(&mut self) {
        if !self.results.is_empty() {
            if self.selected_index + 1 >= self.results.len() {
                self.selected_index = 0;
            } else {
                self.selected_index += 1;
            }
        }
    }

    /// Enter key: activates currently selected item
    pub fn handle_enter(&mut self) -> Option<SearchItem> {
        if !self.is_visible || self.results.is_empty() {
            return None;
        }
        let selected = self.results.get(self.selected_index).cloned();
        self.hide();
        selected
    }

    /// Computes the modal card bounding box
    pub fn modal_bounds(&self) -> Rect {
        let card_w = self.modal_width;
        let results_h = if self.results.is_empty() {
            0
        } else {
            self.results.len() as i32 * RESULT_ITEM_HEIGHT + 8
        };
        let card_h = SEARCH_BAR_HEIGHT + results_h;
        let card_x = (self.screen_w - card_w) / 2;
        let card_y = (self.screen_h / 6).max(60);

        Rect::new(card_x, card_y, card_w, card_h)
    }

    /// Render the Spotlight search modal onto a BufferSurface
    pub fn render(&self, surface: &mut BufferSurface) {
        if !self.is_visible {
            return;
        }

        // 1. Scrim: Dark translucent backdrop overlay (30% opacity)
        let screen_rect = Rect::new(0, 0, self.screen_w, self.screen_h);
        surface.fill_rect(screen_rect, Color::rgba(0, 0, 0, 75));

        // 2. Centered Floating Card with Level 4 Elevation Shadow
        let card_rect = self.modal_bounds();
        render_elevated_card(
            surface,
            card_rect,
            self.corners.large,
            4,
            &self.elevation,
            &self.tokens,
        );

        // Card acrylic background
        surface.apply_gaussian_blur(card_rect, self.tokens.acrylic_blur_radius / 2);
        surface.fill_rounded_rect(card_rect, self.corners.large, self.tokens.popup_background);

        // Delicate outer border
        surface.fill_rect(Rect::new(card_rect.x, card_rect.y, card_rect.width, 1), self.tokens.border_subtle);
        surface.fill_rect(Rect::new(card_rect.x, card_rect.y + card_rect.height - 1, card_rect.width, 1), self.tokens.border_subtle);
        surface.fill_rect(Rect::new(card_rect.x, card_rect.y, 1, card_rect.height), self.tokens.border_subtle);
        surface.fill_rect(Rect::new(card_rect.x + card_rect.width - 1, card_rect.y, 1, card_rect.height), self.tokens.border_subtle);

        // 3. Search Input Header
        let icon_x = card_rect.x + 18;
        let icon_y = card_rect.y + 18;
        // Search glyph (magnifying glass)
        surface.fill_rect(Rect::new(icon_x, icon_y, 10, 1), self.tokens.accent_primary);
        surface.fill_rect(Rect::new(icon_x, icon_y + 9, 10, 1), self.tokens.accent_primary);
        surface.fill_rect(Rect::new(icon_x, icon_y, 1, 10), self.tokens.accent_primary);
        surface.fill_rect(Rect::new(icon_x + 9, icon_y, 1, 10), self.tokens.accent_primary);
        for d in 0..5 {
            surface.set_pixel(icon_x + 9 + d, icon_y + 9 + d, self.tokens.accent_primary);
        }

        // Search text / placeholder
        let text_x = card_rect.x + 44;
        let text_y = card_rect.y + 18;
        if self.query.is_empty() {
            surface.draw_text("Spotlight Search or calculate (e.g. 42 * 1024, files, settings)...", text_x, text_y, self.tokens.text_muted);
        } else {
            surface.draw_text(&self.query, text_x, text_y, self.tokens.text_primary);
            // Blinking cursor pip
            let cursor_x = text_x + (self.query.len() as i32 * 8) + 2;
            surface.fill_rect(Rect::new(cursor_x, text_y - 2, 2, 18), self.tokens.accent_primary);
        }

        // 4. Results List
        if !self.results.is_empty() {
            // Divider between search bar and results
            let div_y = card_rect.y + SEARCH_BAR_HEIGHT;
            surface.fill_rect(Rect::new(card_rect.x + 12, div_y, card_rect.width - 24, 1), self.tokens.divider);

            let mut cur_y = div_y + 4;
            for (idx, item) in self.results.iter().enumerate() {
                let is_selected = idx == self.selected_index;
                let row_rect = Rect::new(card_rect.x + 8, cur_y, card_rect.width - 16, RESULT_ITEM_HEIGHT);

                // Row selection highlight pill
                if is_selected {
                    surface.fill_rounded_rect(row_rect, self.corners.small, self.tokens.accent_subtle);
                }

                // Category Icon
                surface.draw_text(&item.icon_glyph, row_rect.x + 12, cur_y + 14, self.tokens.text_primary);

                // Title
                let title_color = if is_selected { self.tokens.accent_primary } else { self.tokens.text_primary };
                surface.draw_text(&item.title, row_rect.x + 36, cur_y + 8, title_color);

                // Subtitle
                surface.draw_text(&item.subtitle, row_rect.x + 36, cur_y + 26, self.tokens.text_muted);

                // Right Category Badge Pill
                let badge = item.category.badge_label();
                let badge_w = (badge.len() as i32 * 7) + 12;
                let badge_x = row_rect.x + row_rect.width - badge_w - 12;
                let badge_rect = Rect::new(badge_x, cur_y + 14, badge_w, 20);

                let badge_bg = match item.category {
                    SearchCategory::Calculation => self.tokens.accent_primary,
                    SearchCategory::Application => self.tokens.card_hover,
                    _ => self.tokens.card_background,
                };
                let badge_text_color = match item.category {
                    SearchCategory::Calculation => self.tokens.text_on_accent,
                    _ => self.tokens.text_secondary,
                };

                surface.fill_rounded_rect(badge_rect, 6, badge_bg);
                surface.draw_text(badge, badge_rect.x + 6, badge_rect.y + 4, badge_text_color);

                cur_y += RESULT_ITEM_HEIGHT;
            }
        }
    }
}

// ============================================================================
// Self-Test Runner
// ============================================================================

/// Verifies Spotlight search indexing, math evaluation, keyboard flow, and rendering
pub fn run_nova_search_selftest() -> bool {
    let mut spotlight = SpotlightOverlay::new(
        1024,
        768,
        ThemeMode::Dark,
        AccentPalette::FluentBlue,
    );

    // 1. Overlay toggle & initial results
    spotlight.show();
    if !spotlight.is_visible {
        return false;
    }

    // 2. Fuzzy Search for "Files"
    spotlight.set_query("files");
    if spotlight.results.is_empty() || spotlight.results[0].id != "app_files" {
        return false;
    }

    // 3. Inline Math Calculation: "42 * 1024"
    spotlight.set_query("42 * 1024");
    if spotlight.results.is_empty() || spotlight.results[0].category != SearchCategory::Calculation {
        return false;
    }
    if !spotlight.results[0].title.contains("43008") {
        return false;
    }

    // 4. Unit Conversion: "1024 KB to MB"
    spotlight.set_query("1024 KB to MB");
    if spotlight.results.is_empty() || spotlight.results[0].category != SearchCategory::Calculation {
        return false;
    }
    if !spotlight.results[0].title.contains("1.00 MB") {
        return false;
    }

    // 5. Arrow navigation & Enter execution
    spotlight.set_query("term");
    if spotlight.results.is_empty() {
        return false;
    }
    spotlight.handle_arrow_down();
    let action = spotlight.handle_enter();
    if action.is_none() || spotlight.is_visible {
        return false; // enter must return selected item and hide modal
    }

    // 6. Surface Rendering
    spotlight.show();
    spotlight.set_query("settings");
    let mut surface = BufferSurface::new(1024, 768);
    spotlight.render(&mut surface);

    true
}

// ============================================================================
// Unit Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spotlight_lifecycle() {
        let mut spot = SpotlightOverlay::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        assert!(!spot.is_visible);

        spot.show();
        assert!(spot.is_visible);

        spot.hide();
        assert!(!spot.is_visible);
        assert!(spot.query.is_empty());
    }

    #[test]
    fn test_fuzzy_search_applications() {
        let mut spot = SpotlightOverlay::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        spot.show();

        // Exact match
        spot.set_query("Terminal");
        assert!(!spot.results.is_empty());
        assert_eq!(spot.results[0].id, "app_term");

        // Fuzzy subsequence "txted" -> "Text Editor"
        spot.set_query("txted");
        assert!(!spot.results.is_empty());
        assert_eq!(spot.results[0].id, "app_edit");
    }

    #[test]
    fn test_inline_math_expressions() {
        let mut spot = SpotlightOverlay::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        spot.show();

        // Arithmetic
        spot.set_query("128 + 256");
        assert_eq!(spot.results[0].category, SearchCategory::Calculation);
        assert!(spot.results[0].title.contains("384"));

        // Precedence & Parentheses
        spot.set_query("(10 + 5) * 4");
        assert_eq!(spot.results[0].category, SearchCategory::Calculation);
        assert!(spot.results[0].title.contains("60"));

        // Function call: sqrt(144)
        spot.set_query("sqrt(144)");
        assert_eq!(spot.results[0].category, SearchCategory::Calculation);
        assert!(spot.results[0].title.contains("12"));
    }

    #[test]
    fn test_unit_conversions() {
        let mut spot = SpotlightOverlay::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        spot.show();

        spot.set_query("16 GB to MB");
        assert_eq!(spot.results[0].category, SearchCategory::Calculation);
        assert!(spot.results[0].title.contains("16384 MB"));

        spot.set_query("100 C to F");
        assert_eq!(spot.results[0].category, SearchCategory::Calculation);
        assert!(spot.results[0].title.contains("212.0 °F"));
    }

    #[test]
    fn test_keyboard_navigation_and_enter() {
        let mut spot = SpotlightOverlay::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        spot.show();
        spot.set_query("set");

        assert!(spot.results.len() >= 2);
        assert_eq!(spot.selected_index, 0);

        spot.handle_arrow_down();
        assert_eq!(spot.selected_index, 1);

        spot.handle_arrow_up();
        assert_eq!(spot.selected_index, 0);

        let selected = spot.handle_enter().unwrap();
        assert!(!spot.is_visible);
        assert_eq!(selected.category, SearchCategory::Application);
        assert_eq!(selected.action_target, "SETTINGS.ELF");
    }

    #[test]
    fn test_spotlight_rendering() {
        let mut spot = SpotlightOverlay::new(1024, 768, ThemeMode::Dark, AccentPalette::FluentBlue);
        spot.show();
        spot.set_query("ping");

        let mut surface = BufferSurface::new(1024, 768);
        surface.clear(Color::rgb(20, 20, 20));

        spot.render(&mut surface);

        // Center card pixels should be painted
        let card = spot.modal_bounds();
        let center_x = card.x + card.width / 2;
        let center_y = card.y + 20;
        assert_ne!(surface.get_pixel(center_x, center_y), Color::rgb(20, 20, 20));
    }
}
