//! kernel/src/security/cgroups/cpu.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Cgroups v2 CPU Bandwidth Controller
//!
//! Provides CFS (Completely Fair Scheduler) bandwidth limiting & proportional weighting:
//!   - cpu.max: quota and period in microseconds ("$QUOTA $PERIOD" or "max $PERIOD").
//!   - cpu.weight: proportional share weight (1 to 10000, default 100).
//!   - cpu.stat: accounting metrics (usage_usec, user_usec, system_usec, nr_periods,
//!               nr_throttled, throttled_usec).
//!   - Enforces throttling on every scheduler quantum/tick.

#![allow(dead_code)]

use super::memory::{append_kv, copy_slice, format_u64, format_u64_line, parse_u64, CGROUP_UNLIMITED, EINVAL};

pub const DEFAULT_CPU_PERIOD_USEC: u64 = 100_000; // 100ms
pub const DEFAULT_CPU_WEIGHT: u32 = 100;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CpuControllerState {
    pub quota_usec: u64,
    pub period_usec: u64,
    pub weight: u32,
    pub usage_usec: u64,
    pub user_usec: u64,
    pub system_usec: u64,
    pub period_usage_usec: u64,
    pub period_start_tick: u64,
    pub period_started: bool,
    pub is_throttled: bool,
    pub nr_periods: u64,
    pub nr_throttled: u64,
    pub throttled_usec: u64,
}

impl CpuControllerState {
    pub const fn empty() -> Self {
        Self {
            quota_usec: CGROUP_UNLIMITED,
            period_usec: DEFAULT_CPU_PERIOD_USEC,
            weight: DEFAULT_CPU_WEIGHT,
            usage_usec: 0,
            user_usec: 0,
            system_usec: 0,
            period_usage_usec: 0,
            period_start_tick: 0,
            period_started: false,
            is_throttled: false,
            nr_periods: 0,
            nr_throttled: 0,
            throttled_usec: 0,
        }
    }

    pub fn new() -> Self {
        Self::empty()
    }

    /// Called on scheduler tick / quantum execution for a task belonging to this cgroup.
    /// `delta_usec` is the CPU execution time consumed in this step.
    /// Returns `true` if the process is allowed to run, or `false` if throttled.
    pub fn on_tick(&mut self, current_tick: u64, delta_usec: u64, is_user: bool) -> bool {
        // Compute elapsed time in ticks (assuming 1 tick = 10ms = 10,000 usec for 100Hz PIT)
        if !self.period_started {
            self.period_start_tick = current_tick;
            self.period_started = true;
        }

        let elapsed_ticks = current_tick.saturating_sub(self.period_start_tick);
        let elapsed_usec = elapsed_ticks.saturating_mul(10_000);

        if elapsed_usec >= self.period_usec {
            // New period starts: reset period usage and un-throttle
            self.period_usage_usec = 0;
            self.period_start_tick = current_tick;
            self.nr_periods += 1;
            if self.is_throttled {
                self.is_throttled = false;
            }
        }

        // Account CPU runtime
        self.usage_usec = self.usage_usec.saturating_add(delta_usec);
        self.period_usage_usec = self.period_usage_usec.saturating_add(delta_usec);
        if is_user {
            self.user_usec = self.user_usec.saturating_add(delta_usec);
        } else {
            self.system_usec = self.system_usec.saturating_add(delta_usec);
        }

        // Enforce quota
        if self.quota_usec != CGROUP_UNLIMITED && self.period_usage_usec >= self.quota_usec {
            if !self.is_throttled {
                self.is_throttled = true;
                self.nr_throttled += 1;
            }
            self.throttled_usec = self.throttled_usec.saturating_add(delta_usec);
            return false;
        }

        !self.is_throttled
    }

    /// Set cpu.max from string input ("$QUOTA $PERIOD" or "max $PERIOD" or "max").
    pub fn set_max(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        let mut parts = trimmed.split_ascii_whitespace();

        let quota_str = match parts.next() {
            Some(q) => q,
            None => return Err(EINVAL),
        };

        if quota_str == "max" {
            self.quota_usec = CGROUP_UNLIMITED;
        } else if let Some(q) = parse_u64(quota_str) {
            self.quota_usec = q;
        } else {
            return Err(EINVAL);
        }

        if let Some(period_str) = parts.next() {
            if let Some(p) = parse_u64(period_str) {
                if p == 0 {
                    return Err(EINVAL);
                }
                self.period_usec = p;
            } else {
                return Err(EINVAL);
            }
        }

        Ok(())
    }

    /// Set cpu.weight (1 to 10000).
    pub fn set_weight(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        if let Some(w) = parse_u64(trimmed) {
            if (1..=10000).contains(&w) {
                self.weight = w as u32;
                return Ok(());
            }
        }
        Err(EINVAL)
    }

    /// Format cpu.max ("$QUOTA $PERIOD\n").
    pub fn format_max(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        if self.quota_usec == CGROUP_UNLIMITED {
            offset += copy_slice(b"max", &mut buf[offset..]);
        } else {
            offset += format_u64(self.quota_usec, &mut buf[offset..]);
        }
        if offset < buf.len() {
            buf[offset] = b' ';
            offset += 1;
        }
        offset += format_u64(self.period_usec, &mut buf[offset..]);
        if offset < buf.len() {
            buf[offset] = b'\n';
            offset += 1;
        }
        offset
    }

    /// Format cpu.weight ("$WEIGHT\n").
    pub fn format_weight(&self, buf: &mut [u8]) -> usize {
        format_u64_line(self.weight as u64, buf)
    }

    /// Format cpu.stat.
    pub fn format_stat(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        offset += append_kv(b"usage_usec", self.usage_usec, &mut buf[offset..]);
        offset += append_kv(b"user_usec", self.user_usec, &mut buf[offset..]);
        offset += append_kv(b"system_usec", self.system_usec, &mut buf[offset..]);
        offset += append_kv(b"nr_periods", self.nr_periods, &mut buf[offset..]);
        offset += append_kv(b"nr_throttled", self.nr_throttled, &mut buf[offset..]);
        offset += append_kv(b"throttled_usec", self.throttled_usec, &mut buf[offset..]);
        offset
    }
}
