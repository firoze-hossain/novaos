//! userland/desktop/apps/nova-files/src/breadcrumbs.rs
//!
//! Interactive Segmented Path Breadcrumbs, History Navigation, and Search Bar for NovaFiles.

#[cfg(not(test))]
use alloc::{string::{String, ToString}, vec::Vec};

#[cfg(test)]
use std::{string::{String, ToString}, vec::Vec};


use nova_ui::prelude::{BufferSurface, Color, Point, Rect};

/// A single clickable breadcrumb segment.
#[derive(Clone, Debug)]
pub struct BreadcrumbCrumb {
    pub label: String,
    pub path: String,
    pub bounds: Rect,
}

/// Navigation Bar managing breadcrumbs, history, and instant folder search.
pub struct BreadcrumbBar {
    pub bounds: Rect,
    pub current_path: String,
    pub crumbs: Vec<BreadcrumbCrumb>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub search_query: String,
    pub is_search_active: bool,
    pub is_editing_path: bool,
}

impl BreadcrumbBar {
    pub fn new(bounds: Rect) -> Self {
        let mut bar = Self {
            bounds,
            current_path: "/users/root/Documents".to_string(),
            crumbs: Vec::new(),
            can_go_back: false,
            can_go_forward: false,
            search_query: String::new(),
            is_search_active: false,
            is_editing_path: false,
        };
        bar.set_path("/users/root/Documents");
        bar
    }

    /// Sets the current path and reconstructs clickable crumbs.
    pub fn set_path(&mut self, path: &str) {
        self.current_path = path.to_string();
        self.rebuild_crumbs();
    }

    /// Rebuilds breadcrumb segments from current path.
    pub fn rebuild_crumbs(&mut self) {
        self.crumbs.clear();

        let clean = self.current_path.trim_matches('/');
        let segments: Vec<&str> = if clean.is_empty() {
            Vec::new()
        } else {
            clean.split('/').collect()
        };

        let start_x = self.bounds.x + 100; // After back/forward/up buttons
        let mut curr_x = start_x;
        let crumb_h = 24;
        let crumb_y = self.bounds.y + (self.bounds.height - crumb_h) / 2;

        // Root crumb ("NovaFS")
        let root_w = 64;
        self.crumbs.push(BreadcrumbCrumb {
            label: "NovaFS".to_string(),
            path: "/".to_string(),
            bounds: Rect::new(curr_x, crumb_y, root_w, crumb_h),
        });
        curr_x += root_w + 16; // 16px for chevron '>'

        let mut acc_path = String::new();
        for seg in segments {
            acc_path.push('/');
            acc_path.push_str(seg);

            let seg_w = (seg.len() as i32 * 8 + 16).max(40);
            self.crumbs.push(BreadcrumbCrumb {
                label: seg.to_string(),
                path: acc_path.clone(),
                bounds: Rect::new(curr_x, crumb_y, seg_w, crumb_h),
            });
            curr_x += seg_w + 16;
        }
    }

    /// Hit-tests a click within the breadcrumb bar.
    pub fn hit_test_crumb(&self, p: Point) -> Option<&BreadcrumbCrumb> {
        self.crumbs.iter().find(|c| c.bounds.contains(p))
    }

    /// Renders the breadcrumb navigation bar onto BufferSurface.
    pub fn render(&self, surface: &mut BufferSurface, is_dark: bool) {
        let bg = if is_dark { Color::rgba(32, 32, 38, 255) } else { Color::rgba(245, 245, 248, 255) };
        let border = if is_dark { Color::rgba(55, 55, 62, 255) } else { Color::rgba(220, 220, 228, 255) };
        let text_color = if is_dark { Color::rgba(220, 220, 230, 255) } else { Color::rgba(30, 30, 35, 255) };

        surface.fill_rect(self.bounds, bg);
        surface.fill_rect(Rect::new(self.bounds.x, self.bounds.y + self.bounds.height - 1, self.bounds.width, 1), border);

        // Navigation History Buttons: Back (◀), Forward (▶), Up (▲), Refresh (⟳)
        let btn_y = self.bounds.y + (self.bounds.height - 24) / 2;
        let mut bx = self.bounds.x + 8;

        // Back button
        let back_color = if self.can_go_back { text_color } else { Color::rgba(120, 120, 130, 100) };
        surface.fill_rect(Rect::new(bx + 4, btn_y + 10, 12, 4), back_color);
        bx += 24;

        // Forward button
        let fwd_color = if self.can_go_forward { text_color } else { Color::rgba(120, 120, 130, 100) };
        surface.fill_rect(Rect::new(bx + 4, btn_y + 10, 12, 4), fwd_color);
        bx += 24;

        // Up button
        surface.fill_rect(Rect::new(bx + 6, btn_y + 6, 8, 12), text_color);
        bx += 24;

        // Separator line
        surface.fill_rect(Rect::new(bx + 4, btn_y + 2, 1, 20), border);

        // Path Breadcrumbs Container Card
        let path_box_w = self.bounds.width - 340;
        let path_box = Rect::new(self.bounds.x + 96, btn_y, path_box_w, 24);
        let path_box_bg = if is_dark { Color::rgba(24, 24, 28, 255) } else { Color::rgba(255, 255, 255, 255) };
        surface.fill_rect(path_box, path_box_bg);
        surface.stroke_rounded_rect(path_box, 0, 1, border);

        // Render Crumbs inside path box
        for crumb in &self.crumbs {
            if crumb.bounds.x + crumb.bounds.width > path_box.x + path_box.width {
                break;
            }

            // Crumb text bar
            surface.fill_rect(Rect::new(crumb.bounds.x + 4, crumb.bounds.y + 10, crumb.bounds.width - 8, 4), text_color);

            // Chevron separator '>'
            let chev_x = crumb.bounds.x + crumb.bounds.width + 4;
            if chev_x + 6 < path_box.x + path_box.width {
                surface.fill_rect(Rect::new(chev_x, crumb.bounds.y + 10, 6, 4), Color::rgba(140, 140, 150, 200));
            }
        }

        // Instant Search Input Box on the right (200px width)
        let search_box = Rect::new(self.bounds.x + self.bounds.width - 220, btn_y, 208, 24);
        surface.fill_rect(search_box, path_box_bg);
        surface.stroke_rounded_rect(search_box, 0, 1, border);

        // Search magnifying glass icon pill
        surface.fill_rect(Rect::new(search_box.x + 8, search_box.y + 7, 10, 10), Color::rgba(0, 122, 255, 255));

        // Search text bar or placeholder
        if self.search_query.is_empty() {
            surface.fill_rect(Rect::new(search_box.x + 24, search_box.y + 10, 70, 4), Color::rgba(140, 140, 150, 140));
        } else {
            surface.fill_rect(Rect::new(search_box.x + 24, search_box.y + 10, 120, 4), Color::rgba(0, 122, 255, 255));
            // Clear '✕' button
            surface.fill_rect(Rect::new(search_box.x + search_box.width - 18, search_box.y + 7, 10, 10), Color::rgba(255, 69, 58, 200));
        }
    }
}
