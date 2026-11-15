//! userland/desktop/apps/nova-store/src/view.rs
//!
//! Windows 11 / Ubuntu Software Center + macOS App Store Synthesized UI Shell.
//! Coordinates catalog grids, hero carousel banners, screenshot previews,
//! sandbox permission inspector cards, and zero-allocation BufferSurface rendering.

#[cfg(not(test))]
use alloc::{
    format,
    string::String,
};

#[cfg(test)]
use std::{
    format,
    string::String,
};

use crate::package::{AppCategory, AppPackage, InstallStatus};
use nova_ui::prelude::{BufferSurface, Color, Point, Rect, WHITE};

/// User click target returned by hit testing.
#[derive(Clone, Debug, PartialEq)]
pub enum StoreHitTarget {
    Category(AppCategory),
    SearchBox,
    ClearSearch,
    SelectApp(String),
    BackToCatalog,
    InstallPackage(String),
    UpdatePackage(String),
    RemovePackage(String),
}

/// Palette tokens for Store UI.
pub struct StoreThemeTokens {
    pub bg_window: Color,
    pub bg_sidebar: Color,
    pub bg_card: Color,
    pub bg_card_hover: Color,
    pub border_color: Color,
    pub accent_primary: Color,
    pub accent_green: Color,
    pub accent_purple: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub verified_badge: Color,
}

impl StoreThemeTokens {
    pub const fn dark() -> Self {
        Self {
            bg_window: Color::rgb(26, 26, 32),
            bg_sidebar: Color::rgb(20, 20, 24),
            bg_card: Color::rgb(34, 34, 42),
            bg_card_hover: Color::rgb(42, 42, 54),
            border_color: Color::rgba(255, 255, 255, 22),
            accent_primary: Color::rgb(0, 120, 212), // Fluent Blue
            accent_green: Color::rgb(48, 209, 88),   // Update Green
            accent_purple: Color::rgb(191, 90, 242),
            text_primary: Color::rgb(245, 245, 250),
            text_secondary: Color::rgb(160, 160, 175),
            verified_badge: Color::rgb(52, 199, 89), // Ed25519 Verified
        }
    }
}

/// Visual presentation engine for Nova Store.
pub struct StoreView {
    pub width: i32,
    pub height: i32,
    pub sidebar_width: i32,
    pub tokens: StoreThemeTokens,
}

impl Default for StoreView {
    fn default() -> Self {
        Self::new(1024, 680)
    }
}

impl StoreView {
    pub fn new(width: i32, height: i32) -> Self {
        Self {
            width,
            height,
            sidebar_width: 240,
            tokens: StoreThemeTokens::dark(),
        }
    }

    /// Renders the complete Store UI.
    pub fn render(
        &self,
        surface: &mut BufferSurface,
        active_cat: AppCategory,
        search_query: &str,
        packages: &[&AppPackage],
        selected_app: Option<&AppPackage>,
        installed_count: usize,
        updates_count: usize,
    ) {
        // 1. Clear background
        surface.clear(self.tokens.bg_window);

        // 2. Render Sidebar
        self.render_sidebar(surface, active_cat, search_query, installed_count, updates_count);

        // 3. Render Main Content Area
        let content_x = self.sidebar_width + 24;
        let content_w = self.width - content_x - 24;

        if let Some(app) = selected_app {
            // Render App Details Inspector View
            self.render_app_details(surface, app, content_x, content_w);
        } else {
            // Render App Catalog / Search Grid View
            self.render_catalog_grid(surface, active_cat, search_query, packages, content_x, content_w);
        }
    }

    /// Renders the left navigation sidebar.
    fn render_sidebar(
        &self,
        surface: &mut BufferSurface,
        active_cat: AppCategory,
        search_query: &str,
        installed_count: usize,
        updates_count: usize,
    ) {
        let sidebar_rect = Rect::new(0, 0, self.sidebar_width, self.height);
        surface.fill_rect(sidebar_rect, self.tokens.bg_sidebar);
        surface.fill_rect(
            Rect::new(self.sidebar_width - 1, 0, 1, self.height),
            self.tokens.border_color,
        );

        // Branding Header
        surface.fill_rounded_rect(Rect::new(16, 16, 36, 36), 8, self.tokens.accent_primary);
        surface.draw_text("NS", 26, 26, WHITE);
        surface.draw_text("Nova Store", 60, 20, self.tokens.text_primary);
        surface.draw_text("Official Repository", 60, 36, self.tokens.text_secondary);

        // Search Input Box
        let search_rect = Rect::new(14, 68, self.sidebar_width - 28, 32);
        surface.fill_rounded_rect(search_rect, 6, Color::rgb(36, 36, 44));
        surface.stroke_rounded_rect(search_rect, 6, 1, self.tokens.border_color);
        if search_query.is_empty() {
            surface.draw_text("Search apps...", 24, 76, self.tokens.text_secondary);
        } else {
            surface.draw_text(search_query, 24, 76, self.tokens.text_primary);
            surface.draw_text("X", self.sidebar_width - 32, 76, self.tokens.text_secondary);
        }

        // Category Items
        let start_y = 112;
        let item_h = 38;
        let all_cats = AppCategory::all();

        for (idx, cat) in all_cats.iter().enumerate() {
            let item_y = start_y + (idx as i32) * (item_h + 4);
            let item_rect = Rect::new(12, item_y, self.sidebar_width - 24, item_h);

            let is_selected = *cat == active_cat;
            if is_selected {
                surface.fill_rounded_rect(
                    item_rect,
                    6,
                    Color::rgba(self.tokens.accent_primary.r, self.tokens.accent_primary.g, self.tokens.accent_primary.b, 60),
                );
                surface.fill_rounded_rect(Rect::new(12, item_y + 8, 3, item_h - 16), 2, self.tokens.accent_primary);
            }

            let text_color = if is_selected { WHITE } else { self.tokens.text_primary };
            surface.draw_text(cat.icon(), 22, item_y + 11, self.tokens.accent_primary);
            surface.draw_text(cat.title(), 54, item_y + 11, text_color);

            // Badges for Installed / Updates
            if *cat == AppCategory::Installed && installed_count > 0 {
                let badge_text = format!("{}", installed_count);
                surface.draw_text(&badge_text, self.sidebar_width - 36, item_y + 11, self.tokens.text_secondary);
            } else if *cat == AppCategory::Updates && updates_count > 0 {
                let badge_rect = Rect::new(self.sidebar_width - 40, item_y + 9, 20, 20);
                surface.fill_rounded_rect(badge_rect, 10, self.tokens.accent_green);
                let badge_text = format!("{}", updates_count);
                surface.draw_text(&badge_text, self.sidebar_width - 34, item_y + 11, WHITE);
            }
        }
    }

    /// Renders the catalog grid with hero banner and package cards.
    fn render_catalog_grid(
        &self,
        surface: &mut BufferSurface,
        active_cat: AppCategory,
        search_query: &str,
        packages: &[&AppPackage],
        content_x: i32,
        content_w: i32,
    ) {
        let mut cur_y = 16;

        // Header Title / Breadcrumb
        if !search_query.trim().is_empty() {
            let title = format!("Search results for: \"{}\"", search_query);
            surface.draw_text(&title, content_x, cur_y, self.tokens.text_primary);
            cur_y += 20;
            let count_str = format!("Found {} applications", packages.len());
            surface.draw_text(&count_str, content_x, cur_y, self.tokens.text_secondary);
            cur_y += 24;
        } else {
            let breadcrumb = format!("Nova Store > {}", active_cat.title());
            surface.draw_text(&breadcrumb, content_x, cur_y, self.tokens.accent_primary);
            cur_y += 20;
            surface.draw_text(active_cat.title(), content_x, cur_y, self.tokens.text_primary);
            cur_y += 24;

            // Hero Spotlight Banner for Featured category
            if active_cat == AppCategory::Featured && !packages.is_empty() {
                let hero_rect = Rect::new(content_x, cur_y, content_w, 100);
                surface.fill_rounded_rect(hero_rect, 10, Color::rgb(28, 44, 68));
                surface.stroke_rounded_rect(hero_rect, 10, 1, self.tokens.accent_primary);

                surface.draw_text("FEATURED SPOTLIGHT", content_x + 16, cur_y + 14, self.tokens.accent_primary);
                surface.draw_text("Nova Code Studio v2.4.0", content_x + 16, cur_y + 34, WHITE);
                surface.draw_text("Fast, distraction-free code editor & IDE with native Rust and C tooling", content_x + 16, cur_y + 54, self.tokens.text_secondary);

                // Verified seal badge
                surface.fill_rounded_rect(Rect::new(content_x + content_w - 180, cur_y + 14, 160, 24), 4, Color::rgba(48, 209, 88, 30));
                surface.draw_text("[+] Ed25519 Verified", content_x + content_w - 170, cur_y + 18, self.tokens.verified_badge);

                cur_y += 114;
            }
        }

        // 2-Column App Cards Grid
        let card_w = (content_w - 16) / 2;
        let card_h = 86;

        for (idx, app) in packages.iter().enumerate() {
            let col = idx % 2;
            let row = idx / 2;

            let card_x = content_x + (col as i32) * (card_w + 16);
            let card_y = cur_y + (row as i32) * (card_h + 12);

            if card_y > self.height - 90 {
                break;
            }

            let card_rect = Rect::new(card_x, card_y, card_w, card_h);
            surface.fill_rounded_rect(card_rect, 8, self.tokens.bg_card);
            surface.stroke_rounded_rect(card_rect, 8, 1, self.tokens.border_color);

            // Icon square
            let icon_rect = Rect::new(card_x + 12, card_y + 14, 48, 48);
            surface.fill_rounded_rect(icon_rect, 8, Color::rgb(48, 54, 68));
            surface.draw_text(app.icon_symbol, card_x + 18, card_y + 30, self.tokens.accent_primary);

            // Titles
            surface.draw_text(&app.name, card_x + 68, card_y + 12, self.tokens.text_primary);
            let summary_len = app.summary.len().min(30);
            surface.draw_text(&app.summary[..summary_len], card_x + 68, card_y + 30, self.tokens.text_secondary);

            // Rating & Size
            let rating_str = format!("* {:.1}  {}", app.rating, app.formatted_size());
            surface.draw_text(&rating_str, card_x + 68, card_y + 48, Color::rgb(255, 180, 0));

            // Action Button on Right
            self.render_action_button(surface, app, card_x + card_w - 82, card_y + 28);
        }
    }

    /// Renders action button (GET / OPEN / UPDATE / PROGRESS) for a package card.
    fn render_action_button(&self, surface: &mut BufferSurface, app: &AppPackage, btn_x: i32, btn_y: i32) {
        let btn_rect = Rect::new(btn_x, btn_y, 70, 28);

        match &app.status {
            InstallStatus::NotInstalled => {
                surface.fill_rounded_rect(btn_rect, 14, self.tokens.accent_primary);
                surface.draw_text("GET", btn_x + 22, btn_y + 6, WHITE);
            }
            InstallStatus::Installed { .. } => {
                surface.fill_rounded_rect(btn_rect, 14, Color::rgb(48, 48, 58));
                surface.stroke_rounded_rect(btn_rect, 14, 1, self.tokens.border_color);
                surface.draw_text("OPEN", btn_x + 18, btn_y + 6, self.tokens.text_primary);
            }
            InstallStatus::Updatable { .. } => {
                surface.fill_rounded_rect(btn_rect, 14, self.tokens.accent_green);
                surface.draw_text("UPDATE", btn_x + 10, btn_y + 6, WHITE);
            }
            InstallStatus::Installing { progress_pct } => {
                surface.fill_rounded_rect(btn_rect, 14, Color::rgb(36, 36, 44));
                let prog_w = ((*progress_pct as i32) * 70) / 100;
                surface.fill_rounded_rect(Rect::new(btn_x, btn_y, prog_w, 28), 14, self.tokens.accent_primary);
                let prog_text = format!("{}%", progress_pct);
                surface.draw_text(&prog_text, btn_x + 20, btn_y + 6, WHITE);
            }
            InstallStatus::Removing => {
                surface.fill_rounded_rect(btn_rect, 14, Color::rgb(180, 40, 40));
                surface.draw_text("DEL...", btn_x + 14, btn_y + 6, WHITE);
            }
        }
    }

    /// Renders the App Details Inspector View.
    fn render_app_details(
        &self,
        surface: &mut BufferSurface,
        app: &AppPackage,
        content_x: i32,
        content_w: i32,
    ) {
        let mut cur_y = 16;

        // Back button
        let back_rect = Rect::new(content_x, cur_y, 80, 24);
        surface.fill_rounded_rect(back_rect, 4, Color::rgb(40, 40, 50));
        surface.draw_text("< Back", content_x + 12, cur_y + 4, self.tokens.text_primary);
        cur_y += 34;

        // App Hero Header Card
        let hero_rect = Rect::new(content_x, cur_y, content_w, 86);
        surface.fill_rounded_rect(hero_rect, 8, self.tokens.bg_card);
        surface.stroke_rounded_rect(hero_rect, 8, 1, self.tokens.border_color);

        // Big Icon
        let icon_rect = Rect::new(content_x + 16, cur_y + 12, 60, 60);
        surface.fill_rounded_rect(icon_rect, 10, self.tokens.accent_primary);
        surface.draw_text(app.icon_symbol, content_x + 24, cur_y + 34, WHITE);

        // Titles
        surface.draw_text(&app.name, content_x + 88, cur_y + 14, WHITE);
        surface.draw_text(&app.developer, content_x + 88, cur_y + 34, self.tokens.text_secondary);

        // Verified Shield Badge
        surface.fill_rounded_rect(Rect::new(content_x + 88, cur_y + 52, 140, 20), 4, Color::rgba(48, 209, 88, 25));
        surface.draw_text("[+] Verified Ed25519", content_x + 94, cur_y + 54, self.tokens.verified_badge);

        // Primary Action Button (Right aligned)
        let act_btn_x = content_x + content_w - 110;
        let act_btn_y = cur_y + 26;
        let act_btn_rect = Rect::new(act_btn_x, act_btn_y, 96, 34);

        if app.is_installed() {
            surface.fill_rounded_rect(act_btn_rect, 6, Color::rgb(180, 40, 40));
            surface.draw_text("REMOVE", act_btn_x + 22, act_btn_y + 9, WHITE);
        } else {
            surface.fill_rounded_rect(act_btn_rect, 6, self.tokens.accent_primary);
            surface.draw_text("INSTALL", act_btn_x + 20, act_btn_y + 9, WHITE);
        }

        cur_y += 100;

        // Screenshots Carousel Preview Row
        if !app.screenshots.is_empty() {
            surface.draw_text("Screenshots & Previews", content_x, cur_y, self.tokens.text_primary);
            cur_y += 20;

            let ss_w = (content_w - 20) / 2;
            for (i, ss) in app.screenshots.iter().take(2).enumerate() {
                let ss_x = content_x + (i as i32) * (ss_w + 16);
                let ss_rect = Rect::new(ss_x, cur_y, ss_w, 90);
                surface.fill_rounded_rect(ss_rect, 6, Color::rgb(30, 32, 40));
                surface.stroke_rounded_rect(ss_rect, 6, 1, self.tokens.border_color);

                // Screenshot accent preview bar
                surface.fill_rounded_rect(
                    Rect::new(ss_x + 10, cur_y + 10, 24, 24),
                    4,
                    Color::rgb(ss.accent_rgb.0, ss.accent_rgb.1, ss.accent_rgb.2),
                );
                surface.draw_text(&ss.title, ss_x + 44, cur_y + 14, self.tokens.text_primary);
                surface.draw_text(&ss.caption, ss_x + 12, cur_y + 44, self.tokens.text_secondary);
            }
            cur_y += 104;
        }

        // Two-Column Layout: Left = Description, Right = Sandbox Permissions & Info
        let left_w = (content_w * 60) / 100;
        let right_w = content_w - left_w - 16;
        let right_x = content_x + left_w + 16;

        // Left Column: Description & Changelog
        surface.draw_text("Description", content_x, cur_y, self.tokens.text_primary);
        cur_y += 20;
        let desc_len = app.description.len().min(80);
        surface.draw_text(&app.description[..desc_len], content_x, cur_y, self.tokens.text_secondary);
        cur_y += 30;

        surface.draw_text("What's New in this Version", content_x, cur_y, self.tokens.text_primary);
        cur_y += 20;
        let change_len = app.changelog.len().min(80);
        surface.draw_text(&app.changelog[..change_len], content_x, cur_y, self.tokens.text_secondary);

        // Right Column: Sandbox Permissions Card
        let perm_rect = Rect::new(right_x, cur_y - 70, right_w, 140);
        surface.fill_rounded_rect(perm_rect, 6, self.tokens.bg_card);
        surface.stroke_rounded_rect(perm_rect, 6, 1, self.tokens.border_color);

        surface.draw_text("Sandbox Permissions", right_x + 12, cur_y - 58, self.tokens.text_primary);
        let perms = app.permissions.list_declared_permissions();
        for (i, p) in perms.iter().take(4).enumerate() {
            let p_len = p.len().min(30);
            surface.draw_text(&p[..p_len], right_x + 12, cur_y - 36 + (i as i32) * 18, self.tokens.text_secondary);
        }
    }

    /// Hit-tests pointer coordinates to identify clicked category, search box, card, or action button.
    pub fn hit_test(
        &self,
        pt: Point,
        search_query: &str,
        packages: &[&AppPackage],
        selected_app: Option<&AppPackage>,
    ) -> Option<StoreHitTarget> {
        // 1. Sidebar hit test
        if pt.x < self.sidebar_width {
            let search_rect = Rect::new(14, 68, self.sidebar_width - 28, 32);
            if search_rect.contains(pt) {
                if !search_query.is_empty() && pt.x >= self.sidebar_width - 36 {
                    return Some(StoreHitTarget::ClearSearch);
                }
                return Some(StoreHitTarget::SearchBox);
            }

            let start_y = 112;
            let item_h = 38;
            for (idx, cat) in AppCategory::all().iter().enumerate() {
                let item_y = start_y + (idx as i32) * (item_h + 4);
                let item_rect = Rect::new(12, item_y, self.sidebar_width - 24, item_h);
                if item_rect.contains(pt) {
                    return Some(StoreHitTarget::Category(*cat));
                }
            }

            return None;
        }

        // 2. Main Content hit test
        let content_x = self.sidebar_width + 24;
        let content_w = self.width - content_x - 24;

        if let Some(app) = selected_app {
            // Back button
            let back_rect = Rect::new(content_x, 16, 80, 24);
            if back_rect.contains(pt) {
                return Some(StoreHitTarget::BackToCatalog);
            }

            // Hero action button
            let hero_y = 50;
            let act_btn_x = content_x + content_w - 110;
            let act_btn_y = hero_y + 26;
            let act_btn_rect = Rect::new(act_btn_x, act_btn_y, 96, 34);

            if act_btn_rect.contains(pt) {
                if app.is_installed() {
                    return Some(StoreHitTarget::RemovePackage(app.id.clone()));
                } else {
                    return Some(StoreHitTarget::InstallPackage(app.id.clone()));
                }
            }
        } else {
            // App Cards Grid hit test
            let mut cur_y = 16 + 20 + 24;
            if search_query.trim().is_empty() && !packages.is_empty() {
                // Featured spotlight hero offset
                cur_y += 114;
            }

            let card_w = (content_w - 16) / 2;
            let card_h = 86;

            for (idx, app) in packages.iter().enumerate() {
                let col = idx % 2;
                let row = idx / 2;

                let card_x = content_x + (col as i32) * (card_w + 16);
                let card_y = cur_y + (row as i32) * (card_h + 12);

                if card_y > self.height - 90 {
                    break;
                }

                let card_rect = Rect::new(card_x, card_y, card_w, card_h);
                if card_rect.contains(pt) {
                    // Check if clicked the action button on right
                    let btn_x = card_x + card_w - 82;
                    let btn_y = card_y + 28;
                    let btn_rect = Rect::new(btn_x, btn_y, 70, 28);

                    if btn_rect.contains(pt) {
                        if app.is_updatable() {
                            return Some(StoreHitTarget::UpdatePackage(app.id.clone()));
                        } else if app.is_installed() {
                            // Already installed -> select/open
                            return Some(StoreHitTarget::SelectApp(app.id.clone()));
                        } else {
                            return Some(StoreHitTarget::InstallPackage(app.id.clone()));
                        }
                    }

                    // Otherwise clicked card -> view details
                    return Some(StoreHitTarget::SelectApp(app.id.clone()));
                }
            }
        }

        None
    }
}
