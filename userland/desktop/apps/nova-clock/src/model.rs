//! userland/desktop/apps/nova-clock/src/model.rs
//!
//! Domain models and mathematical timekeeping engines for NovaOS Clock.

pub use chronos_core::time::{day_of_week, days_in_month, is_leap_year, DateTime};
pub use chronos_core::world::{City, PinnedCity, WorldClockManager, ALL_CITIES};
pub use chronos_core::alarm::{Alarm, AlarmManager};
pub use chronos_core::stopwatch::{format_ms, Lap, Stopwatch, StopwatchState};
pub use chronos_core::timer::{Timer, TimerState, QUICK_PRESETS};
pub use chronos_core::{ClockHub, ClockTab};
