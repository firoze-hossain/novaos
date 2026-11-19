//! userland/desktop/apps/nova-calendar/src/ui.rs
//!
//! Graphical User Interface for NovaOS Calendar (nova-calendar).
//! Synthesizes Apple Calendar (macOS), Windows 11 Calendar, and GNOME Calendar (Ubuntu).

use nova_ui::prelude::{BufferSurface, Color, Rect};

use crate::model::{
    days_in_month, DAY_NAMES_SHORT, EventCategory, CalendarState, ViewMode,
};

pub const CALENDAR_WIDTH: usize = 800;
pub const CALENDAR_HEIGHT: usize = 600;

// Unified Theme Colors
pub const COLOR_BG: Color = Color::rgb(0x18, 0x1A, 0x24);          // Deep Slate
pub const COLOR_SURFACE: Color = Color::rgb(0x22, 0x25, 0x33);     // Card Surface
pub const COLOR_SURFACE_ALT: Color = Color::rgb(0x2D, 0x32, 0x46); // Hover / Elevated Card
pub const COLOR_BORDER: Color = Color::rgb(0x3E, 0x44, 0x5E);      // Border Subtle
pub const COLOR_TEXT_PRIMARY: Color = Color::rgb(0xF0, 0xF4, 0xFC);// Text Primary White
pub const COLOR_TEXT_MUTED: Color = Color::rgb(0x94, 0xA0, 0xB8);  // Text Secondary Gray
pub const COLOR_ACCENT_CYAN: Color = Color::rgb(0x06, 0xB6, 0xD4); // Electric Cyan
pub const COLOR_ACCENT_BLUE: Color = Color::rgb(0x3B, 0x82, 0xF6); // Royal Blue
pub const COLOR_TODAY_BADGE: Color = Color::rgb(0x00, 0xB4, 0xD8); // Today Circle Highlight
pub const COLOR_SELECTED_RING: Color = Color::rgb(0xF5, 0x9E, 0x0B);// Selected Amber Ring

pub struct CalendarUiRenderer;

impl CalendarUiRenderer {
    pub fn render(surface: &mut BufferSurface, state: &CalendarState) {
        let sw = surface.width as i32;
        let sh = surface.height as i32;

        // 1. Clear background
        surface.fill_rect(Rect::new(0, 0, sw, sh), COLOR_BG);

        let top_h = 44;
        let status_h = 24;
        let content_h = sh - top_h - status_h;

        // 2. Render Top Header Bar (macOS Toolbar + Windows Action Controls)
        Self::render_top_bar(surface, state, top_h);

        // 3. Layout: 3-pane split
        let sidebar_w = 200;
        let right_pane_w = 240;
        let center_w = sw - sidebar_w - right_pane_w;

        let sidebar_rect = Rect::new(0, top_h, sidebar_w, content_h);
        let center_rect = Rect::new(sidebar_w, top_h, center_w, content_h);
        let right_rect = Rect::new(sidebar_w + center_w, top_h, right_pane_w, content_h);

        // Sidebar
        Self::render_sidebar(surface, state, sidebar_rect);

        // Center Content (Month / Week / Day / Agenda)
        match state.view_mode {
            ViewMode::Month => Self::render_month_grid(surface, state, center_rect),
            ViewMode::Week => Self::render_week_view(surface, state, center_rect),
            ViewMode::Day => Self::render_day_view(surface, state, center_rect),
            ViewMode::Agenda => Self::render_agenda_view(surface, state, center_rect),
        }

        // Right Pane: Event Schedule & Inspector
        Self::render_event_inspector(surface, state, right_rect);

        // 4. Status Bar at Bottom
        let status_rect = Rect::new(0, sh - status_h, sw, status_h);
        Self::render_status_bar(surface, state, status_rect);
    }

    fn render_top_bar(surface: &mut BufferSurface, state: &CalendarState, h: i32) {
        let w = surface.width as i32;
        surface.fill_rect(Rect::new(0, 0, w, h), COLOR_SURFACE);
        surface.fill_rect(Rect::new(0, h - 1, w, 1), COLOR_BORDER);

        // Nav Buttons: [<] [Today] [>]
        let btn_prev = Rect::new(14, 10, 26, 24);
        surface.fill_rounded_rect(btn_prev, 4, COLOR_SURFACE_ALT);
        surface.stroke_rounded_rect(btn_prev, 4, 1, COLOR_BORDER);

        let btn_today = Rect::new(46, 10, 52, 24);
        surface.fill_rounded_rect(btn_today, 4, COLOR_SURFACE_ALT);
        surface.stroke_rounded_rect(btn_today, 4, 1, COLOR_BORDER);

        let btn_next = Rect::new(104, 10, 26, 24);
        surface.fill_rounded_rect(btn_next, 4, COLOR_SURFACE_ALT);
        surface.stroke_rounded_rect(btn_next, 4, 1, COLOR_BORDER);

        // macOS-style Segmented View Switcher in Center: [ Month | Week | Day | Agenda ]
        let seg_w = 260;
        let seg_x = (w - seg_w) / 2;
        let seg_rect = Rect::new(seg_x, 9, seg_w, 26);
        surface.fill_rounded_rect(seg_rect, 6, COLOR_SURFACE_ALT);
        surface.stroke_rounded_rect(seg_rect, 6, 1, COLOR_BORDER);

        let item_w = seg_w / 4;
        for (i, mode) in ViewMode::all().iter().enumerate() {
            let item_rect = Rect::new(seg_x + (i as i32) * item_w + 1, 10, item_w - 2, 24);
            if *mode == state.view_mode {
                surface.fill_rounded_rect(item_rect, 5, COLOR_ACCENT_BLUE);
            }
        }

        // Right side: [+ Add Event] action button (Windows 11 style)
        let add_btn = Rect::new(w - 110, 10, 96, 24);
        surface.fill_rounded_rect(add_btn, 4, Color::rgb(0x23, 0x86, 0x36));
        surface.stroke_rounded_rect(add_btn, 4, 1, Color::rgb(0x3F, 0xB9, 0x50));
    }

    fn render_sidebar(surface: &mut BufferSurface, state: &CalendarState, rect: Rect) {
        // Sidebar background & right border
        surface.fill_rect(rect, COLOR_SURFACE);
        surface.fill_rect(Rect::new(rect.x + rect.width - 1, rect.y, 1, rect.height), COLOR_BORDER);

        // 1. Hero Date Card (Windows 11 inspired)
        let hero_rect = Rect::new(rect.x + 12, rect.y + 12, rect.width - 24, 68);
        surface.fill_rounded_rect(hero_rect, 6, COLOR_SURFACE_ALT);
        surface.stroke_rounded_rect(hero_rect, 6, 1, COLOR_BORDER);

        // 2. Mini Monthly Calendar
        let mini_y = hero_rect.y + hero_rect.height + 14;
        let mini_rect = Rect::new(rect.x + 12, mini_y, rect.width - 24, 120);
        surface.fill_rounded_rect(mini_rect, 4, Color::rgba(0, 0, 0, 40));
        surface.stroke_rounded_rect(mini_rect, 4, 1, COLOR_BORDER);

        // 3. Category / Calendar Checkboxes (macOS Calendar list)
        let cat_y = mini_rect.y + mini_rect.height + 16;
        let mut cur_y = cat_y + 18;

        for cat in EventCategory::all().iter() {
            let is_enabled = state.is_category_enabled(*cat);
            let check_rect = Rect::new(rect.x + 16, cur_y + 2, 12, 12);
            let (r, g, b) = cat.color_rgb();
            let cat_col = Color::rgb(r, g, b);

            if is_enabled {
                surface.fill_rounded_rect(check_rect, 3, cat_col);
            } else {
                surface.stroke_rounded_rect(check_rect, 3, 1, COLOR_BORDER);
            }

            cur_y += 24;
        }
    }

    fn render_month_grid(surface: &mut BufferSurface, state: &CalendarState, rect: Rect) {
        surface.fill_rect(rect, COLOR_BG);

        // Day of week headers (Ubuntu GNOME style)
        let header_h = 24;
        let col_w = rect.width / 7;

        surface.fill_rect(Rect::new(rect.x, rect.y, rect.width, header_h), COLOR_SURFACE);
        surface.fill_rect(Rect::new(rect.x, rect.y + header_h - 1, rect.width, 1), COLOR_BORDER);

        // Grid cells (6 rows x 7 cols)
        let grid_y = rect.y + header_h;
        let grid_h = rect.height - header_h;
        let row_h = grid_h / 6;

        let first_dow = state.first_day_of_week() as usize;
        let total_days = state.days_in_current_month() as usize;
        let prev_month_days = if state.view_month == 1 {
            days_in_month(state.view_year - 1, 12) as usize
        } else {
            days_in_month(state.view_year, state.view_month - 1) as usize
        };

        for row in 0..6 {
            for col in 0..7 {
                let cell_x = rect.x + (col as i32) * col_w;
                let cell_y = grid_y + (row as i32) * row_h;
                let cell_rect = Rect::new(cell_x, cell_y, col_w, row_h);

                // Subtle grid border
                surface.stroke_rounded_rect(cell_rect, 0, 1, Color::rgba(255, 255, 255, 12));

                let idx = row * 7 + col;
                if idx < first_dow {
                    let _d = prev_month_days - (first_dow - 1 - idx);
                } else if idx < first_dow + total_days {
                    let d = (idx - first_dow + 1) as u8;
                    let is_today = state.view_year == state.today_year
                        && state.view_month == state.today_month
                        && d == state.today_day;
                    let is_selected = state.view_year == state.selected_year
                        && state.view_month == state.selected_month
                        && d == state.selected_day;

                    if is_selected {
                        surface.fill_rounded_rect(
                            Rect::new(cell_x + 2, cell_y + 2, col_w - 4, row_h - 4),
                            4,
                            Color::rgba(245, 158, 11, 35),
                        );
                        surface.stroke_rounded_rect(
                            Rect::new(cell_x + 2, cell_y + 2, col_w - 4, row_h - 4),
                            4,
                            1,
                            COLOR_SELECTED_RING,
                        );
                    }

                    if is_today {
                        let badge_rect = Rect::new(cell_x + 6, cell_y + 4, 20, 18);
                        surface.fill_rounded_rect(badge_rect, 4, COLOR_TODAY_BADGE);
                    }

                    // Render event pill indicators for this day
                    let events = state.events_for_date(state.view_year, state.view_month, d);
                    let mut dot_x = cell_x + 8;
                    let dot_y = cell_y + row_h - 12;

                    for ev in events.iter().take(3) {
                        let (r, g, b) = ev.category.color_rgb();
                        surface.fill_rounded_rect(Rect::new(dot_x, dot_y, 8, 6), 3, Color::rgb(r, g, b));
                        dot_x += 11;
                    }
                }
            }
        }
    }

    fn render_week_view(surface: &mut BufferSurface, _state: &CalendarState, rect: Rect) {
        surface.fill_rect(rect, COLOR_BG);
        let col_w = rect.width / 7;

        for (i, _name) in DAY_NAMES_SHORT.iter().enumerate() {
            let col_x = rect.x + (i as i32) * col_w;
            surface.fill_rect(Rect::new(col_x, rect.y, col_w, 24), COLOR_SURFACE);
            surface.stroke_rounded_rect(Rect::new(col_x, rect.y, col_w, rect.height), 0, 1, Color::rgba(255, 255, 255, 12));
        }
    }

    fn render_day_view(surface: &mut BufferSurface, state: &CalendarState, rect: Rect) {
        surface.fill_rect(rect, COLOR_BG);

        // Header for selected day
        let hdr_h = 32;
        surface.fill_rect(Rect::new(rect.x, rect.y, rect.width, hdr_h), COLOR_SURFACE);
        surface.fill_rect(Rect::new(rect.x, rect.y + hdr_h - 1, rect.width, 1), COLOR_BORDER);

        // Hourly rows (09:00 to 18:00)
        let row_h = (rect.height - hdr_h) / 10;
        for i in 0..10 {
            let row_y = rect.y + hdr_h + (i as i32) * row_h;
            surface.fill_rect(Rect::new(rect.x, row_y + row_h - 1, rect.width, 1), Color::rgba(255, 255, 255, 10));
        }

        // Render events as timeline cards
        let events = state.events_for_selected_date();
        let mut card_y = rect.y + hdr_h + 10;
        for ev in events {
            if card_y + 40 < rect.y + rect.height {
                let (r, g, b) = ev.category.color_rgb();
                let card_rect = Rect::new(rect.x + 60, card_y, rect.width - 76, 36);
                surface.fill_rounded_rect(card_rect, 4, Color::rgba(r, g, b, 50));
                surface.stroke_rounded_rect(card_rect, 4, 1, Color::rgb(r, g, b));
                card_y += 44;
            }
        }
    }

    fn render_agenda_view(surface: &mut BufferSurface, state: &CalendarState, rect: Rect) {
        surface.fill_rect(rect, COLOR_BG);

        let mut item_y = rect.y + 12;
        let events = state.filtered_events();

        for ev in events {
            if item_y + 54 > rect.y + rect.height {
                break;
            }
            let card_rect = Rect::new(rect.x + 16, item_y, rect.width - 32, 48);
            surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE);
            surface.stroke_rounded_rect(card_rect, 6, 1, COLOR_BORDER);

            let (r, g, b) = ev.category.color_rgb();
            // Category color strip on left edge
            surface.fill_rounded_rect(Rect::new(card_rect.x, card_rect.y, 4, card_rect.height), 2, Color::rgb(r, g, b));

            item_y += 56;
        }
    }

    fn render_event_inspector(surface: &mut BufferSurface, state: &CalendarState, rect: Rect) {
        surface.fill_rect(rect, COLOR_SURFACE);
        surface.fill_rect(Rect::new(rect.x, rect.y, 1, rect.height), COLOR_BORDER);

        // Header: "Today's Schedule & Events"
        let hdr_rect = Rect::new(rect.x, rect.y, rect.width, 36);
        surface.fill_rect(hdr_rect, COLOR_SURFACE_ALT);
        surface.fill_rect(Rect::new(rect.x, hdr_rect.y + hdr_rect.height - 1, rect.width, 1), COLOR_BORDER);

        let events = state.events_for_selected_date();
        let mut cur_y = rect.y + 48;

        for ev in events {
            if cur_y + 64 > rect.y + rect.height - 40 {
                break;
            }
            let card_rect = Rect::new(rect.x + 10, cur_y, rect.width - 20, 58);
            surface.fill_rounded_rect(card_rect, 6, COLOR_SURFACE_ALT);
            surface.stroke_rounded_rect(card_rect, 6, 1, COLOR_BORDER);

            let (r, g, b) = ev.category.color_rgb();
            surface.fill_rounded_rect(Rect::new(card_rect.x + 6, card_rect.y + 6, 4, card_rect.height - 12), 2, Color::rgb(r, g, b));

            cur_y += 66;
        }

        // Quick Add Event Card at Bottom
        let add_card = Rect::new(rect.x + 10, rect.y + rect.height - 36, rect.width - 20, 26);
        surface.fill_rounded_rect(add_card, 4, Color::rgba(255, 255, 255, 16));
        surface.stroke_rounded_rect(add_card, 4, 1, COLOR_BORDER);
    }

    fn render_status_bar(surface: &mut BufferSurface, _state: &CalendarState, rect: Rect) {
        surface.fill_rect(rect, Color::rgb(0x13, 0x15, 0x1D));
        surface.fill_rect(Rect::new(rect.x, rect.y, rect.width, 1), COLOR_BORDER);
    }
}
