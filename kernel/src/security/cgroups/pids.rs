//! kernel/src/security/cgroups/pids.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Cgroups v2 PIDs Controller
//!
//! Provides process count limits & fork-bomb protection:
//!   - pids.max: max number of processes in cgroup ("max" or integer).
//!   - pids.current: current process count in cgroup.
//!   - pids.events: max event counter ("max $N").

#![allow(dead_code)]

use super::memory::{append_kv, copy_slice, format_u64_line, parse_u64, CGROUP_UNLIMITED, EINVAL};

pub const EAGAIN: isize = -11;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PidsControllerState {
    pub max_pids: u64,
    pub current_pids: u64,
    pub events_max: u64,
}

impl PidsControllerState {
    pub const fn empty() -> Self {
        Self {
            max_pids: CGROUP_UNLIMITED,
            current_pids: 0,
            events_max: 0,
        }
    }

    pub fn new() -> Self {
        Self::empty()
    }

    /// Charge a new process fork/clone to this cgroup.
    /// Returns Ok(()) if under limit, or Err(-EAGAIN) if limit reached.
    pub fn charge(&mut self) -> Result<(), isize> {
        if self.max_pids != CGROUP_UNLIMITED && self.current_pids >= self.max_pids {
            self.events_max += 1;
            return Err(EAGAIN);
        }
        self.current_pids += 1;
        Ok(())
    }

    /// Uncharge an exiting process from this cgroup.
    pub fn uncharge(&mut self) {
        self.current_pids = self.current_pids.saturating_sub(1);
    }

    /// Set pids.max ("max" or integer).
    pub fn set_max(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        if trimmed == "max" {
            self.max_pids = CGROUP_UNLIMITED;
            Ok(())
        } else if let Some(val) = parse_u64(trimmed) {
            self.max_pids = val;
            Ok(())
        } else {
            Err(EINVAL)
        }
    }

    /// Format pids.max.
    pub fn format_max(&self, buf: &mut [u8]) -> usize {
        if self.max_pids == CGROUP_UNLIMITED {
            copy_slice(b"max\n", buf)
        } else {
            format_u64_line(self.max_pids, buf)
        }
    }

    /// Format pids.current.
    pub fn format_current(&self, buf: &mut [u8]) -> usize {
        format_u64_line(self.current_pids, buf)
    }

    /// Format pids.events.
    pub fn format_events(&self, buf: &mut [u8]) -> usize {
        append_kv(b"max", self.events_max, buf)
    }
}
