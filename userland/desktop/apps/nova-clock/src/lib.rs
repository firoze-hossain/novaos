//! userland/desktop/apps/nova-clock/src/lib.rs
//!
//! # NovaOS Clock Application Library (nova-clock)
//!
//! Sovereign, pure-Rust default Clock application for NovaOS, synthesizing
//! macOS Clock (equirectangular solar terminator world map, vector analog dials)
//! and Ubuntu Clock (Quick Start presets, dynamic countdown progress, centisecond stopwatch).

pub mod model;
pub mod ui;

use model::{ClockHub, ClockTab, DateTime, TimerState};
use ui::{handle_click, BufferSurface32, ChronosView, Rect};

/// Executes the comprehensive autonomous qualification suite for NovaOS Clock.
pub fn run_nova_clock_selftest() -> bool {
    let mut hub = ClockHub::default();

    // 1. Time & Solar Terminator Math: Dhaka local time at UTC 12:00 = 18:00
    hub.sync_rtc(2026, 11, 24, 12, 0, 0, 0);
    let dac = hub.world_mgr.pinned.iter().find(|p| p.city.id == "dac").unwrap();
    let dac_time = hub.world_mgr.city_time(&dac.city);
    if dac_time.hour != 18 || dac_time.minute != 0 {
        eprintln!("[FAIL] Dhaka local time expected 18:00, got {:02}:{:02}", dac_time.hour, dac_time.minute);
        return false;
    }

    // Solar terminator: London in daylight at UTC 12:00
    if !hub.world_mgr.is_coord_in_daylight(51.5, 0.0) {
        eprintln!("[FAIL] London should be in daylight at UTC 12:00");
        return false;
    }

    // 2. Alarms: add alarm, trigger on weekday, snooze and dismiss
    hub.alarm_mgr.alarms.clear();
    hub.alarm_mgr.add_alarm(7, 30, "NovaOS Standup", 0b00111110);
    if hub.alarm_mgr.alarms.len() != 1 {
        eprintln!("[FAIL] Expected 1 alarm scheduled");
        return false;
    }

    let trig_dt = DateTime::new(2026, 11, 24, 7, 30, 0);
    if hub.alarm_mgr.check_tick(&trig_dt).is_none() {
        eprintln!("[FAIL] Alarm should trigger at 07:30 Tuesday");
        return false;
    }

    hub.alarm_mgr.snooze_active(&trig_dt);
    if hub.alarm_mgr.alarms[0].snooze_until.is_none() {
        eprintln!("[FAIL] Snooze should reschedule alarm forward");
        return false;
    }

    hub.alarm_mgr.dismiss_active();
    if hub.alarm_mgr.alarms[0].snooze_until.is_some() {
        eprintln!("[FAIL] Dismiss should clear snooze state");
        return false;
    }

    // 3. Stopwatch: lap recording and telemetry
    hub.stopwatch.reset();
    hub.stopwatch.start();
    hub.stopwatch.advance_millis(15420);
    hub.stopwatch.record_lap();
    hub.stopwatch.advance_millis(12100);
    hub.stopwatch.record_lap();

    if hub.stopwatch.fastest_lap_index() != Some(1) {
        eprintln!("[FAIL] Lap 2 (12.10s) should be fastest lap");
        return false;
    }

    if hub.stopwatch.slowest_lap_index() != Some(0) {
        eprintln!("[FAIL] Lap 1 (15.42s) should be slowest lap");
        return false;
    }

    // 4. Timers: 5-minute countdown preset
    hub.timer.reset();
    hub.timer.set_duration(0, 5, 0);
    hub.timer.start();
    hub.timer.advance_millis(120_000); // 2m elapsed
    if hub.timer.remaining_ms != 180_000 {
        eprintln!("[FAIL] Remaining time expected 180,000 ms, got {}", hub.timer.remaining_ms);
        return false;
    }

    let finished = hub.timer.advance_millis(185_000);
    if !finished || hub.timer.state != TimerState::Finished {
        eprintln!("[FAIL] Timer should fire finished signal when duration expires");
        return false;
    }

    // 5. Software Rasterization & Hit-testing
    let width = 640;
    let height = 480;
    let mut buffer = vec![0u32; (width * height) as usize];
    let bounds = Rect::new(0, 0, width, height);

    for tab in &[ClockTab::WorldClock, ClockTab::Alarms, ClockTab::Stopwatch, ClockTab::Timers] {
        hub.set_tab(*tab);
        let mut surface = BufferSurface32::new(&mut buffer, width, height, width);
        ChronosView::render(&mut surface, bounds, &hub);
    }

    // Test tab click switching
    let tab_alarms_x = bounds.x + (bounds.w - 380) / 2 + 120;
    let handled = handle_click(&mut hub, bounds, tab_alarms_x, 22);
    if !handled || hub.active_tab != ClockTab::Alarms {
        eprintln!("[FAIL] Header click failed to switch tab to Alarms");
        return false;
    }

    true
}
