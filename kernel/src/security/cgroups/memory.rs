//! kernel/src/security/cgroups/memory.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Cgroups v2 Memory Controller
//!
//! Provides granular memory governance:
//!   - memory.max: hard limit in bytes; exceeding triggers synchronous OOM killer.
//!   - memory.high: soft throttle watermark in bytes; exceeding penalizes allocations.
//!   - memory.current: current usage accounting across all processes in cgroup.
//!   - memory.stat: breakdown (anon, file, kernel_stack, pagetables, sock, shmem).
//!   - memory.oom.group: when 1, OOM kills all processes in the cgroup atomically.
//!   - memory.events: event counters (low, high, max, oom, oom_kill, oom_group_kill).

#![allow(dead_code)]

pub const CGROUP_UNLIMITED: u64 = u64::MAX;

pub const EINVAL: isize = -22;
pub const ENOMEM: isize = -12;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct MemoryControllerState {
    pub max_bytes: u64,
    pub high_bytes: u64,
    pub current_bytes: u64,
    pub anon_bytes: u64,
    pub file_bytes: u64,
    pub kernel_stack_bytes: u64,
    pub pagetables_bytes: u64,
    pub sock_bytes: u64,
    pub shmem_bytes: u64,
    pub oom_group: bool,
    // Event counters
    pub events_low: u64,
    pub events_high: u64,
    pub events_max: u64,
    pub events_oom: u64,
    pub events_oom_kill: u64,
    pub events_oom_group_kill: u64,
}

impl MemoryControllerState {
    pub const fn empty() -> Self {
        Self {
            max_bytes: CGROUP_UNLIMITED,
            high_bytes: CGROUP_UNLIMITED,
            current_bytes: 0,
            anon_bytes: 0,
            file_bytes: 0,
            kernel_stack_bytes: 0,
            pagetables_bytes: 0,
            sock_bytes: 0,
            shmem_bytes: 0,
            oom_group: false,
            events_low: 0,
            events_high: 0,
            events_max: 0,
            events_oom: 0,
            events_oom_kill: 0,
            events_oom_group_kill: 0,
        }
    }

    pub fn new() -> Self {
        Self::empty()
    }

    /// Charge memory usage to the cgroup.
    /// Returns Ok(throttled: bool) if within limit, or Err(ENOMEM) if exceeding memory.max.
    pub fn charge(&mut self, bytes: u64, is_anon: bool) -> Result<bool, isize> {
        let new_total = match self.current_bytes.checked_add(bytes) {
            Some(t) => t,
            None => return Err(ENOMEM),
        };

        if self.max_bytes != CGROUP_UNLIMITED && new_total > self.max_bytes {
            self.events_max += 1;
            self.events_oom += 1;
            return Err(ENOMEM);
        }

        self.current_bytes = new_total;
        if is_anon {
            self.anon_bytes = self.anon_bytes.saturating_add(bytes);
        } else {
            self.file_bytes = self.file_bytes.saturating_add(bytes);
        }

        let is_throttled = self.high_bytes != CGROUP_UNLIMITED && self.current_bytes > self.high_bytes;
        if is_throttled {
            self.events_high += 1;
        }

        Ok(is_throttled)
    }

    /// Uncharge memory usage from the cgroup.
    pub fn uncharge(&mut self, bytes: u64, is_anon: bool) {
        self.current_bytes = self.current_bytes.saturating_sub(bytes);
        if is_anon {
            self.anon_bytes = self.anon_bytes.saturating_sub(bytes);
        } else {
            self.file_bytes = self.file_bytes.saturating_sub(bytes);
        }
    }

    /// Set memory.max from string input ("max" or integer bytes).
    pub fn set_max(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        if trimmed == "max" {
            self.max_bytes = CGROUP_UNLIMITED;
            Ok(())
        } else if let Some(val) = parse_u64(trimmed) {
            self.max_bytes = val;
            Ok(())
        } else {
            Err(EINVAL)
        }
    }

    /// Set memory.high from string input ("max" or integer bytes).
    pub fn set_high(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        if trimmed == "max" {
            self.high_bytes = CGROUP_UNLIMITED;
            Ok(())
        } else if let Some(val) = parse_u64(trimmed) {
            self.high_bytes = val;
            Ok(())
        } else {
            Err(EINVAL)
        }
    }

    /// Set memory.oom.group (0 or 1).
    pub fn set_oom_group(&mut self, s: &str) -> Result<(), isize> {
        let trimmed = s.trim();
        if trimmed == "1" {
            self.oom_group = true;
            Ok(())
        } else if trimmed == "0" {
            self.oom_group = false;
            Ok(())
        } else {
            Err(EINVAL)
        }
    }

    /// Format memory.current into buffer.
    pub fn format_current(&self, buf: &mut [u8]) -> usize {
        format_u64_line(self.current_bytes, buf)
    }

    /// Format memory.max into buffer.
    pub fn format_max(&self, buf: &mut [u8]) -> usize {
        if self.max_bytes == CGROUP_UNLIMITED {
            copy_slice(b"max\n", buf)
        } else {
            format_u64_line(self.max_bytes, buf)
        }
    }

    /// Format memory.high into buffer.
    pub fn format_high(&self, buf: &mut [u8]) -> usize {
        if self.high_bytes == CGROUP_UNLIMITED {
            copy_slice(b"max\n", buf)
        } else {
            format_u64_line(self.high_bytes, buf)
        }
    }

    /// Format memory.oom.group into buffer.
    pub fn format_oom_group(&self, buf: &mut [u8]) -> usize {
        if self.oom_group {
            copy_slice(b"1\n", buf)
        } else {
            copy_slice(b"0\n", buf)
        }
    }

    /// Format memory.stat into buffer.
    pub fn format_stat(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        offset += append_kv(b"anon", self.anon_bytes, &mut buf[offset..]);
        offset += append_kv(b"file", self.file_bytes, &mut buf[offset..]);
        offset += append_kv(b"kernel_stack", self.kernel_stack_bytes, &mut buf[offset..]);
        offset += append_kv(b"pagetables", self.pagetables_bytes, &mut buf[offset..]);
        offset += append_kv(b"sock", self.sock_bytes, &mut buf[offset..]);
        offset += append_kv(b"shmem", self.shmem_bytes, &mut buf[offset..]);
        offset += append_kv(b"file_mapped", self.file_bytes, &mut buf[offset..]);
        offset += append_kv(b"file_dirty", 0, &mut buf[offset..]);
        offset += append_kv(b"file_writeback", 0, &mut buf[offset..]);
        offset
    }

    /// Format memory.events into buffer.
    pub fn format_events(&self, buf: &mut [u8]) -> usize {
        let mut offset = 0;
        offset += append_kv(b"low", self.events_low, &mut buf[offset..]);
        offset += append_kv(b"high", self.events_high, &mut buf[offset..]);
        offset += append_kv(b"max", self.events_max, &mut buf[offset..]);
        offset += append_kv(b"oom", self.events_oom, &mut buf[offset..]);
        offset += append_kv(b"oom_kill", self.events_oom_kill, &mut buf[offset..]);
        offset += append_kv(b"oom_group_kill", self.events_oom_group_kill, &mut buf[offset..]);
        offset
    }
}

// -----------------------------------------------------------------------------
// Format & Parsing Helpers
// -----------------------------------------------------------------------------

pub fn parse_u64(s: &str) -> Option<u64> {
    if s.is_empty() {
        return None;
    }
    let mut val: u64 = 0;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u64)?;
    }
    Some(val)
}

pub fn format_u64(mut val: u64, buf: &mut [u8]) -> usize {
    if val == 0 {
        if !buf.is_empty() {
            buf[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut temp = [0u8; 20];
    let mut idx = 0;
    while val > 0 && idx < 20 {
        temp[idx] = b'0' + (val % 10) as u8;
        val /= 10;
        idx += 1;
    }
    let copy_len = idx.min(buf.len());
    for i in 0..copy_len {
        buf[i] = temp[idx - 1 - i];
    }
    copy_len
}

pub fn format_u64_line(val: u64, buf: &mut [u8]) -> usize {
    let mut len = format_u64(val, buf);
    if len < buf.len() {
        buf[len] = b'\n';
        len += 1;
    }
    len
}

pub fn copy_slice(src: &[u8], dst: &mut [u8]) -> usize {
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
    len
}

pub fn append_kv(key: &[u8], val: u64, buf: &mut [u8]) -> usize {
    let mut offset = copy_slice(key, buf);
    if offset < buf.len() {
        buf[offset] = b' ';
        offset += 1;
    }
    offset += format_u64(val, &mut buf[offset..]);
    if offset < buf.len() {
        buf[offset] = b'\n';
        offset += 1;
    }
    offset
}
