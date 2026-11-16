//! kernel/src/security/cgroups/io.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Cgroups v2 I/O Controller
//!
//! Provides granular per-device block I/O throttling and proportional weights:
//!   - io.max: per-device rate limits ($MAJOR:$MINOR rbps=$B wbps=$B riops=$N wiops=$N).
//!   - io.weight: proportional I/O weight (1 to 10000, default 100).
//!   - io.stat: cumulative I/O accounting (rbytes, wbytes, rios, wios).

#![allow(dead_code)]

use super::memory::{copy_slice, format_u64, format_u64_line, parse_u64, CGROUP_UNLIMITED, EINVAL};

pub const MAX_IO_LIMITS: usize = 4;
pub const DEFAULT_IO_WEIGHT: u32 = 100;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IoDeviceLimit {
    pub major: u32,
    pub minor: u32,
    pub rbps: u64,
    pub wbps: u64,
    pub riops: u64,
    pub wiops: u64,
    pub active: bool,
}

impl IoDeviceLimit {
    pub const fn empty() -> Self {
        Self {
            major: 0,
            minor: 0,
            rbps: CGROUP_UNLIMITED,
            wbps: CGROUP_UNLIMITED,
            riops: CGROUP_UNLIMITED,
            wiops: CGROUP_UNLIMITED,
            active: false,
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct IoControllerState {
    pub limits: [IoDeviceLimit; MAX_IO_LIMITS],
    pub weight: u32,
    pub window_start_tick: u64,
    pub window_started: bool,
    pub cur_rbytes: u64,
    pub cur_wbytes: u64,
    pub cur_rios: u64,
    pub cur_wios: u64,
    pub total_rbytes: u64,
    pub total_wbytes: u64,
    pub total_rios: u64,
    pub total_wios: u64,
}

impl IoControllerState {
    pub const fn empty() -> Self {
        Self {
            limits: [IoDeviceLimit::empty(); MAX_IO_LIMITS],
            weight: DEFAULT_IO_WEIGHT,
            window_start_tick: 0,
            window_started: false,
            cur_rbytes: 0,
            cur_wbytes: 0,
            cur_rios: 0,
            cur_wios: 0,
            total_rbytes: 0,
            total_wbytes: 0,
            total_rios: 0,
            total_wios: 0,
        }
    }

    pub fn new() -> Self {
        Self::empty()
    }

    /// Check rate limit and charge block I/O.
    /// Returns Ok(()) if allowed, or Err(-16 / -EBUSY) if throttled.
    pub fn check_and_charge(
        &mut self,
        current_tick: u64,
        major: u32,
        minor: u32,
        is_write: bool,
        bytes: u64,
    ) -> Result<(), isize> {
        if !self.window_started {
            self.window_start_tick = current_tick;
            self.window_started = true;
        }

        // 1 second window = 100 ticks at 100Hz
        let elapsed = current_tick.saturating_sub(self.window_start_tick);
        if elapsed >= 100 {
            self.cur_rbytes = 0;
            self.cur_wbytes = 0;
            self.cur_rios = 0;
            self.cur_wios = 0;
            self.window_start_tick = current_tick;
        }

        // Find limit for this device
        let mut dev_limit = None;
        for lim in self.limits.iter() {
            if lim.active && lim.major == major && lim.minor == minor {
                dev_limit = Some(lim);
                break;
            }
        }

        if let Some(lim) = dev_limit {
            if is_write {
                if lim.wbps != CGROUP_UNLIMITED && self.cur_wbytes.saturating_add(bytes) > lim.wbps {
                    return Err(-16); // EBUSY / Throttled
                }
                if lim.wiops != CGROUP_UNLIMITED && self.cur_wios.saturating_add(1) > lim.wiops {
                    return Err(-16);
                }
            } else {
                if lim.rbps != CGROUP_UNLIMITED && self.cur_rbytes.saturating_add(bytes) > lim.rbps {
                    return Err(-16);
                }
                if lim.riops != CGROUP_UNLIMITED && self.cur_rios.saturating_add(1) > lim.riops {
                    return Err(-16);
                }
            }
        }

        // Account
        if is_write {
            self.cur_wbytes = self.cur_wbytes.saturating_add(bytes);
            self.cur_wios = self.cur_wios.saturating_add(1);
            self.total_wbytes = self.total_wbytes.saturating_add(bytes);
            self.total_wios = self.total_wios.saturating_add(1);
        } else {
            self.cur_rbytes = self.cur_rbytes.saturating_add(bytes);
            self.cur_rios = self.cur_rios.saturating_add(1);
            self.total_rbytes = self.total_rbytes.saturating_add(bytes);
            self.total_rios = self.total_rios.saturating_add(1);
        }

        Ok(())
    }

    /// Set io.max from string (e.g. "8:0 rbps=10485760 wbps=10485760 riops=1000 wiops=1000" or "8:0 max").
    pub fn set_max(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        let mut tokens = trimmed.split_ascii_whitespace();

        let dev_str = match tokens.next() {
            Some(d) => d,
            None => return Err(EINVAL),
        };

        let mut colon_parts = dev_str.split(':');
        let major = match colon_parts.next().and_then(parse_u64) {
            Some(m) => m as u32,
            None => return Err(EINVAL),
        };
        let minor = match colon_parts.next().and_then(parse_u64) {
            Some(m) => m as u32,
            None => return Err(EINVAL),
        };

        // Find or allocate limit slot
        let mut slot_idx = None;
        for (i, lim) in self.limits.iter().enumerate() {
            if lim.active && lim.major == major && lim.minor == minor {
                slot_idx = Some(i);
                break;
            }
        }
        if slot_idx.is_none() {
            for (i, lim) in self.limits.iter().enumerate() {
                if !lim.active {
                    slot_idx = Some(i);
                    break;
                }
            }
        }

        let idx = match slot_idx {
            Some(i) => i,
            None => return Err(-12), // ENOMEM
        };

        let lim = &mut self.limits[idx];
        lim.major = major;
        lim.minor = minor;
        lim.active = true;

        for tok in tokens {
            if tok == "max" {
                lim.rbps = CGROUP_UNLIMITED;
                lim.wbps = CGROUP_UNLIMITED;
                lim.riops = CGROUP_UNLIMITED;
                lim.wiops = CGROUP_UNLIMITED;
                continue;
            }
            if let Some(val_str) = tok.strip_prefix("rbps=") {
                lim.rbps = if val_str == "max" { CGROUP_UNLIMITED } else { parse_u64(val_str).ok_or(EINVAL)? };
            } else if let Some(val_str) = tok.strip_prefix("wbps=") {
                lim.wbps = if val_str == "max" { CGROUP_UNLIMITED } else { parse_u64(val_str).ok_or(EINVAL)? };
            } else if let Some(val_str) = tok.strip_prefix("riops=") {
                lim.riops = if val_str == "max" { CGROUP_UNLIMITED } else { parse_u64(val_str).ok_or(EINVAL)? };
            } else if let Some(val_str) = tok.strip_prefix("wiops=") {
                lim.wiops = if val_str == "max" { CGROUP_UNLIMITED } else { parse_u64(val_str).ok_or(EINVAL)? };
            }
        }

        Ok(())
    }

    /// Set io.weight (1 to 10000).
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

    /// Format io.max.
    pub fn format_max(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        for lim in self.limits.iter() {
            if !lim.active {
                continue;
            }
            offset += format_u64(lim.major as u64, &mut buf[offset..]);
            if offset < buf.len() { buf[offset] = b':'; offset += 1; }
            offset += format_u64(lim.minor as u64, &mut buf[offset..]);

            if lim.rbps != CGROUP_UNLIMITED {
                offset += copy_slice(b" rbps=", &mut buf[offset..]);
                offset += format_u64(lim.rbps, &mut buf[offset..]);
            }
            if lim.wbps != CGROUP_UNLIMITED {
                offset += copy_slice(b" wbps=", &mut buf[offset..]);
                offset += format_u64(lim.wbps, &mut buf[offset..]);
            }
            if lim.riops != CGROUP_UNLIMITED {
                offset += copy_slice(b" riops=", &mut buf[offset..]);
                offset += format_u64(lim.riops, &mut buf[offset..]);
            }
            if lim.wiops != CGROUP_UNLIMITED {
                offset += copy_slice(b" wiops=", &mut buf[offset..]);
                offset += format_u64(lim.wiops, &mut buf[offset..]);
            }
            if offset < buf.len() { buf[offset] = b'\n'; offset += 1; }
        }
        offset
    }

    /// Format io.weight.
    pub fn format_weight(&self, buf: &mut [u8]) -> usize {
        format_u64_line(self.weight as u64, buf)
    }

    /// Format io.stat.
    pub fn format_stat(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        // Default entry for 8:0 if active
        offset += copy_slice(b"8:0 rbytes=", &mut buf[offset..]);
        offset += format_u64(self.total_rbytes, &mut buf[offset..]);
        offset += copy_slice(b" wbytes=", &mut buf[offset..]);
        offset += format_u64(self.total_wbytes, &mut buf[offset..]);
        offset += copy_slice(b" rios=", &mut buf[offset..]);
        offset += format_u64(self.total_rios, &mut buf[offset..]);
        offset += copy_slice(b" wios=", &mut buf[offset..]);
        offset += format_u64(self.total_wios, &mut buf[offset..]);
        if offset < buf.len() { buf[offset] = b'\n'; offset += 1; }
        offset
    }
}
