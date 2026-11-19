//! userland/desktop/apps/nova-calendar/src/lib.rs
//!
//! # NovaOS Calendar Application Library (nova-calendar)
//!
//! Unified Calendar & Event Scheduling System for NovaOS Desktop Experience.
//! Synthesizes Apple Calendar (macOS), Windows 11 Calendar, and GNOME Calendar (Ubuntu).

pub mod model;
pub mod ui;

use model::{CalendarEvent, CalendarState, EventCategory, ViewMode};
use ui::{CalendarUiRenderer, CALENDAR_HEIGHT, CALENDAR_WIDTH};
use nova_ui::prelude::BufferSurface;

/// Executes the comprehensive autonomous qualification suite for NovaOS Calendar.
pub fn run_nova_calendar_selftest() -> bool {
    let mut state = CalendarState::new(2026, 11, 19);

    // 1. Initial State & Events
    if state.events.is_empty() {
        eprintln!("[FAIL] Expected pre-loaded enterprise events.");
        return false;
    }
    let initial_count = state.events.len();

    // 2. Month Navigation
    state.next_month();
    if state.view_month != 12 || state.view_year != 2026 {
        eprintln!("[FAIL] next_month failed to advance to December 2026.");
        return false;
    }

    state.prev_month();
    if state.view_month != 11 || state.view_year != 2026 {
        eprintln!("[FAIL] prev_month failed to return to November 2026.");
        return false;
    }

    // 3. Event Addition & Deletion
    let new_ev_id = state.add_event(CalendarEvent::new(
        0,
        "NovaOS Security Workshop",
        2026,
        11,
        25,
        "10:00 AM",
        "12:00 PM",
        EventCategory::Work,
        "Online",
        "Deep-dive into capability-based access control.",
    ));
    if state.events.len() != initial_count + 1 {
        eprintln!("[FAIL] Event was not added.");
        return false;
    }

    if !state.delete_event(new_ev_id) {
        eprintln!("[FAIL] delete_event failed to delete event.");
        return false;
    }
    if state.events.len() != initial_count {
        eprintln!("[FAIL] Event count after deletion mismatch.");
        return false;
    }

    // 4. View Mode Switching
    state.view_mode = ViewMode::Week;
    if state.view_mode != ViewMode::Week {
        return false;
    }
    state.view_mode = ViewMode::Day;
    if state.view_mode != ViewMode::Day {
        return false;
    }
    state.view_mode = ViewMode::Agenda;
    if state.view_mode != ViewMode::Agenda {
        return false;
    }
    state.view_mode = ViewMode::Month;

    // 5. Category Filtering
    state.toggle_category(EventCategory::Work);
    if state.is_category_enabled(EventCategory::Work) {
        eprintln!("[FAIL] Category should be disabled after toggle.");
        return false;
    }
    state.toggle_category(EventCategory::Work);
    if !state.is_category_enabled(EventCategory::Work) {
        eprintln!("[FAIL] Category should be re-enabled after toggle.");
        return false;
    }

    // 6. UI Rasterization Test
    let mut surface = BufferSurface::new(CALENDAR_WIDTH, CALENDAR_HEIGHT);
    CalendarUiRenderer::render(&mut surface, &state);

    let mut non_zero = 0;
    for &px in &surface.pixels {
        if px != 0 {
            non_zero += 1;
        }
    }
    if non_zero < 10000 {
        eprintln!("[FAIL] Rasterized surface has insufficient drawn pixels: {}", non_zero);
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calendar_selftest() {
        assert!(run_nova_calendar_selftest());
    }

    #[test]
    fn test_calendar_date_math() {
        assert!(model::is_leap_year(2024));
        assert!(!model::is_leap_year(2026));
        assert_eq!(model::days_in_month(2026, 2), 28);
        assert_eq!(model::days_in_month(2024, 2), 29);
        assert_eq!(model::day_of_week(2026, 11, 19), 4); // Thursday
    }

    #[test]
    fn test_calendar_events_retrieval() {
        let state = CalendarState::new(2026, 11, 19);
        let today_events = state.events_for_selected_date();
        assert!(!today_events.is_empty(), "Should have events for launch day");
        assert!(today_events.iter().any(|e| e.title.contains("NovaOS 2.0 GA")));
    }

    #[test]
    fn test_calendar_ui_views_rasterization() {
        let mut state = CalendarState::new(2026, 11, 19);
        let mut surface = BufferSurface::new(CALENDAR_WIDTH, CALENDAR_HEIGHT);

        for mode in ViewMode::all() {
            state.view_mode = mode;
            CalendarUiRenderer::render(&mut surface, &state);
            assert!(surface.pixels.iter().any(|&p| p != 0));
        }
    }
}
