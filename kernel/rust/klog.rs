//! kernel/rust/klog.rs - Phase 89: Structured, queryable kernel logging subsystem.
//!
//! WHAT THIS IS
//!
//! Replaces the plain serial-port log stream with real, timestamped, queryable,
//! per-service log entries - the difference between "why did this crash" being
//! answerable or not. A ring-buffer log store in the kernel with query syscalls,
//! feeding both existing kernel_log() call sites and novainit's own service output.
//!
//! DESIGN & GUARANTEES
//!
//! 1. Memory-bounded & zero-allocation:
//!    A fixed ring buffer of 256 entries (`KLOG_CAPACITY`). When full, the oldest
//!    unconsumed entries are overwritten and `dropped_count` is incremented so gap
//!    detection is visible to readers. No heap allocation is ever required.
//!
//! 2. Concurrency:
//!    Guarded by `SpinLock`, preserving CPU interrupt flags so it is safe to call
//!    from both interrupt contexts (e.g. panics/faults) and ordinary syscall threads
//!    across multiple CPUs.
//!
//! 3. Incremental streaming:
//!    Every log entry is assigned a strictly monotonic 64-bit sequence number (`seq`).
//!    Queries specify `start_seq` and receive `next_seq` to stream entries incrementally
//!    (follow/tail -f semantics).
//!
//! 4. Filtering:
//!    Queries can filter by:
//!    - Minimum priority / maximum numerical severity (e.g., WARN and above).
//!    - Originating facility/service name (exact or prefix match).

#![cfg_attr(not(test), no_std)]

#[cfg(not(test))]
use crate::spinlock::SpinLock;

#[cfg(test)]
struct SpinLock<T>(std::sync::Mutex<T>);

#[cfg(test)]
impl<T> SpinLock<T> {
    const fn new(v: T) -> Self {
        SpinLock(std::sync::Mutex::new(v))
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, T> {
        self.0.lock().unwrap()
    }
}

pub const KLOG_CAPACITY: usize = 256;
pub const KLOG_FACILITY_MAX: usize = 16;
pub const KLOG_MSG_MAX: usize = 128;

pub const KLOG_SEV_EMERG: u32 = 0;
pub const KLOG_SEV_ALERT: u32 = 1;
pub const KLOG_SEV_CRIT: u32 = 2;
pub const KLOG_SEV_ERR: u32 = 3;
pub const KLOG_SEV_WARN: u32 = 4;
pub const KLOG_SEV_NOTICE: u32 = 5;
pub const KLOG_SEV_INFO: u32 = 6;
pub const KLOG_SEV_DEBUG: u32 = 7;
pub const KLOG_SEV_ANY: u32 = 0xFFFFFFFF;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KlogEntry {
    pub seq: u64,
    pub timestamp_ticks: u32,
    pub pid: u32,
    pub severity: u32,
    pub facility: [u8; KLOG_FACILITY_MAX],
    pub msg: [u8; KLOG_MSG_MAX],
}

impl KlogEntry {
    pub const fn empty() -> Self {
        KlogEntry {
            seq: 0,
            timestamp_ticks: 0,
            pid: 0,
            severity: KLOG_SEV_INFO,
            facility: [0; KLOG_FACILITY_MAX],
            msg: [0; KLOG_MSG_MAX],
        }
    }
}

pub struct KlogRing {
    entries: [KlogEntry; KLOG_CAPACITY],
    head: usize,        // Next write index
    tail: usize,        // Oldest valid index
    count: usize,       // Current active entries (0..=KLOG_CAPACITY)
    next_seq: u64,      // Monotonic sequence counter (starts at 1)
    dropped_count: u64, // Overwritten entries count
    total_logged: u64,  // Total entries ever logged
}

impl KlogRing {
    pub const fn new() -> Self {
        KlogRing {
            entries: [KlogEntry::empty(); KLOG_CAPACITY],
            head: 0,
            tail: 0,
            count: 0,
            next_seq: 1,
            dropped_count: 0,
            total_logged: 0,
        }
    }

    /// Appends a new structured entry to the ring buffer.
    pub fn append(
        &mut self,
        severity: u32,
        facility: &[u8],
        msg: &[u8],
        pid: u32,
        timestamp_ticks: u32,
    ) -> u64 {
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1);
        self.total_logged = self.total_logged.wrapping_add(1);

        let mut entry = KlogEntry {
            seq,
            timestamp_ticks,
            pid,
            severity,
            facility: [0; KLOG_FACILITY_MAX],
            msg: [0; KLOG_MSG_MAX],
        };

        // Copy facility (up to KLOG_FACILITY_MAX - 1 bytes to guarantee NUL-termination)
        let fac_len = if facility.len() < KLOG_FACILITY_MAX - 1 {
            facility.len()
        } else {
            KLOG_FACILITY_MAX - 1
        };
        for i in 0..fac_len {
            entry.facility[i] = facility[i];
        }

        // Copy msg (up to KLOG_MSG_MAX - 1 bytes to guarantee NUL-termination)
        let msg_len = if msg.len() < KLOG_MSG_MAX - 1 {
            msg.len()
        } else {
            KLOG_MSG_MAX - 1
        };
        for i in 0..msg_len {
            entry.msg[i] = msg[i];
        }

        if self.count == KLOG_CAPACITY {
            // Buffer full: advance tail and record drop
            self.entries[self.head] = entry;
            self.head = (self.head + 1) % KLOG_CAPACITY;
            self.tail = (self.tail + 1) % KLOG_CAPACITY;
            self.dropped_count = self.dropped_count.wrapping_add(1);
        } else {
            // Buffer has room
            self.entries[self.head] = entry;
            self.head = (self.head + 1) % KLOG_CAPACITY;
            self.count += 1;
        }

        seq
    }

    /// Reads matching entries from the ring buffer.
    ///
    /// - `start_seq`: return entries with `seq >= start_seq`. If 0, starts from oldest.
    /// - `max_severity`: if <= KLOG_SEV_DEBUG, filter `entry.severity <= max_severity`.
    /// - `facility_filter`: if non-empty, matches `entry.facility`.
    /// - `out`: slice of KlogEntry to write into.
    ///
    /// Returns: `(entries_written, next_seq, dropped_count, total_logged)`
    pub fn read(
        &self,
        start_seq: u64,
        max_severity: u32,
        facility_filter: &[u8],
        out: &mut [KlogEntry],
    ) -> (usize, u64, u64, u64) {
        if out.is_empty() || self.count == 0 {
            return (0, self.next_seq, self.dropped_count, self.total_logged);
        }

        let mut written = 0usize;
        let mut highest_seq = 0u64;

        for i in 0..self.count {
            let idx = (self.tail + i) % KLOG_CAPACITY;
            let entry = &self.entries[idx];

            if start_seq > 0 && entry.seq < start_seq {
                continue;
            }

            // Severity filter: lower number = higher severity
            if max_severity <= KLOG_SEV_DEBUG && entry.severity > max_severity {
                continue;
            }

            // Facility filter
            if !facility_filter.is_empty() {
                let mut matched = true;
                let filter_len = facility_filter.len();
                for j in 0..filter_len {
                    if j >= KLOG_FACILITY_MAX || entry.facility[j] != facility_filter[j] {
                        matched = false;
                        break;
                    }
                }
                // Verify exact NUL match or length boundary
                if matched && filter_len < KLOG_FACILITY_MAX && entry.facility[filter_len] != 0 {
                    matched = false;
                }
                if !matched {
                    continue;
                }
            }

            // Entry matches! Copy to output
            out[written] = *entry;
            highest_seq = entry.seq;
            written += 1;

            if written == out.len() {
                break;
            }
        }

        let next_seq = if highest_seq > 0 {
            highest_seq.wrapping_add(1)
        } else if start_seq > 0 {
            start_seq
        } else {
            self.next_seq
        };

        (written, next_seq, self.dropped_count, self.total_logged)
    }

    /// Returns subsystem statistics.
    /// `(oldest_seq, newest_seq, total_logged, dropped_count, capacity, count)`
    pub fn info(&self) -> (u64, u64, u64, u64, u32, u32) {
        let oldest = if self.count > 0 {
            self.entries[self.tail].seq
        } else {
            0
        };
        let newest = if self.count > 0 {
            let last_idx = (self.head + KLOG_CAPACITY - 1) % KLOG_CAPACITY;
            self.entries[last_idx].seq
        } else {
            0
        };
        (
            oldest,
            newest,
            self.total_logged,
            self.dropped_count,
            KLOG_CAPACITY as u32,
            self.count as u32,
        )
    }
}

// Global kernel log ring protected by SpinLock
static KLOG: SpinLock<KlogRing> = SpinLock::new(KlogRing::new());

// ------------------------------------------------------------------
// C FFI Boundary
// ------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn rust_klog_append(
    severity: u32,
    facility_ptr: *const u8,
    facility_len: u32,
    msg_ptr: *const u8,
    msg_len: u32,
    pid: u32,
    ticks: u32,
) -> u64 {
    if facility_ptr.is_null() || msg_ptr.is_null() {
        return 0;
    }
    let facility = unsafe { core::slice::from_raw_parts(facility_ptr, facility_len as usize) };
    let msg = unsafe { core::slice::from_raw_parts(msg_ptr, msg_len as usize) };

    let mut ring = KLOG.lock();
    ring.append(severity, facility, msg, pid, ticks)
}

/// Helper called by kernel_log to parse line prefix and log to structured klog
#[no_mangle]
pub extern "C" fn rust_klog_log_kernel_line(
    line_ptr: *const u8,
    line_len: u32,
    ticks: u32,
    pid: u32,
) -> u64 {
    if line_ptr.is_null() || line_len == 0 {
        return 0;
    }
    let mut raw = unsafe { core::slice::from_raw_parts(line_ptr, line_len as usize) };

    // Strip trailing newlines or carriage returns
    while !raw.is_empty() && (raw[raw.len() - 1] == b'\n' || raw[raw.len() - 1] == b'\r') {
        raw = &raw[..raw.len() - 1];
    }
    if raw.is_empty() {
        return 0;
    }

    // Determine severity from prefix tags
    let (severity, facility, content) = if raw.starts_with(b"[PANIC]") || raw.starts_with(b"[FAULT]") {
        (KLOG_SEV_ERR, &b"kernel"[..], raw)
    } else if raw.starts_with(b"[WARN]") {
        (KLOG_SEV_WARN, &b"kernel"[..], raw)
    } else if raw.starts_with(b"[ OK ]") {
        (KLOG_SEV_INFO, &b"kernel"[..], raw)
    } else if raw.starts_with(b"[ .. ]") {
        (KLOG_SEV_NOTICE, &b"kernel"[..], raw)
    } else if raw.starts_with(b"[SYSCALL]") {
        (KLOG_SEV_DEBUG, &b"syscall"[..], raw)
    } else if raw.starts_with(b"[FW]") {
        (KLOG_SEV_NOTICE, &b"firewall"[..], raw)
    } else if raw.starts_with(b"[sandbox]") {
        (KLOG_SEV_INFO, &b"sandbox"[..], raw)
    } else {
        (KLOG_SEV_INFO, &b"kernel"[..], raw)
    };

    let mut ring = KLOG.lock();
    ring.append(severity, facility, content, pid, ticks)
}

#[no_mangle]
pub extern "C" fn rust_klog_read(
    start_seq: u64,
    max_entries: u32,
    max_severity: u32,
    filter_ptr: *const u8,
    filter_len: u32,
    out_entries: *mut KlogEntry,
    out_stats: *mut u64, // [next_seq, dropped_count, total_logged, count_written]
) -> i32 {
    if out_entries.is_null() || out_stats.is_null() {
        return -14; // -EFAULT
    }
    let filter = if !filter_ptr.is_null() && filter_len > 0 {
        unsafe { core::slice::from_raw_parts(filter_ptr, filter_len as usize) }
    } else {
        &[]
    };
    let out_slice = unsafe { core::slice::from_raw_parts_mut(out_entries, max_entries as usize) };

    let ring = KLOG.lock();
    let (written, next_seq, dropped, total) = ring.read(start_seq, max_severity, filter, out_slice);

    unsafe {
        *out_stats.add(0) = next_seq;
        *out_stats.add(1) = dropped;
        *out_stats.add(2) = total;
        *out_stats.add(3) = written as u64;
    }

    written as i32
}

#[no_mangle]
pub extern "C" fn rust_klog_info(out_info: *mut u64) -> i32 {
    if out_info.is_null() {
        return -14; // -EFAULT
    }
    let ring = KLOG.lock();
    let (oldest, newest, total, dropped, cap, count) = ring.info();

    unsafe {
        *out_info.add(0) = oldest;
        *out_info.add(1) = newest;
        *out_info.add(2) = total;
        *out_info.add(3) = dropped;
        *out_info.add(4) = (cap as u64) | ((count as u64) << 32);
    }
    0
}

/// Self-test executed at kernel boot.
/// Returns 0 on success, or non-zero bitmask indicating failure.
#[no_mangle]
pub extern "C" fn rust_klog_selftest() -> u32 {
    let mut err = 0u32;
    let mut test_ring = KlogRing::new();

    // 1. Append and exact field round-trip
    let seq1 = test_ring.append(KLOG_SEV_INFO, b"init", b"boot message 1", 1, 100);
    if seq1 != 1 {
        err |= 1 << 0;
    }

    let mut buf = [KlogEntry::empty(); 10];
    let (n, next_seq, dropped, total) = test_ring.read(0, KLOG_SEV_ANY, b"", &mut buf);
    if n != 1 || next_seq != 2 || dropped != 0 || total != 1 {
        err |= 1 << 1;
    } else {
        if buf[0].seq != 1
            || buf[0].severity != KLOG_SEV_INFO
            || buf[0].pid != 1
            || buf[0].timestamp_ticks != 100
            || &buf[0].facility[..4] != b"init"
            || &buf[0].msg[..14] != b"boot message 1"
        {
            err |= 1 << 2;
        }
    }

    // 2. Incremental streaming via start_seq
    let _seq2 = test_ring.append(KLOG_SEV_WARN, b"vfs", b"disk mount warning", 0, 150);
    let _seq3 = test_ring.append(KLOG_SEV_ERR, b"net", b"packet drop error", 2, 200);

    let (n2, next_seq2, _, _) = test_ring.read(2, KLOG_SEV_ANY, b"", &mut buf);
    if n2 != 2 || next_seq2 != 4 || buf[0].seq != 2 || buf[1].seq != 3 {
        err |= 1 << 3;
    }

    // 3. Severity filtering (e.g. only WARN and above: ERR=3, WARN=4, INFO=6)
    // Asking for max_severity = KLOG_SEV_WARN should return seq 2 (WARN) and seq 3 (ERR), but not seq 1 (INFO)
    let (n3, _, _, _) = test_ring.read(0, KLOG_SEV_WARN, b"", &mut buf);
    if n3 != 2 || buf[0].seq != 2 || buf[1].seq != 3 {
        err |= 1 << 4;
    }

    // 4. Facility filtering (e.g. only "vfs")
    let (n4, _, _, _) = test_ring.read(0, KLOG_SEV_ANY, b"vfs", &mut buf);
    if n4 != 1 || buf[0].seq != 2 || &buf[0].facility[..3] != b"vfs" {
        err |= 1 << 5;
    }

    // 5. Ring buffer wraparound and dropped_count tracking
    let mut wrap_ring = KlogRing::new();
    for i in 0..300 {
        wrap_ring.append(KLOG_SEV_DEBUG, b"stress", b"data", 0, i as u32);
    }
    let (oldest, newest, total_w, dropped_w, cap, count) = wrap_ring.info();
    if total_w != 300 || dropped_w != 44 || count != 256 || cap != 256 {
        err |= 1 << 6;
    }
    if oldest != 45 || newest != 300 {
        err |= 1 << 7;
    }

    err
}

// ------------------------------------------------------------------
// Host Unit Tests
// ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_klog_basic_lifecycle() {
        let mut ring = KlogRing::new();
        assert_eq!(ring.count, 0);

        let seq = ring.append(KLOG_SEV_INFO, b"test", b"hello world", 42, 10);
        assert_eq!(seq, 1);
        assert_eq!(ring.count, 1);
        assert_eq!(ring.total_logged, 1);

        let mut out = [KlogEntry::empty(); 4];
        let (n, next, dropped, total) = ring.read(0, KLOG_SEV_ANY, b"", &mut out);
        assert_eq!(n, 1);
        assert_eq!(next, 2);
        assert_eq!(dropped, 0);
        assert_eq!(total, 1);
        assert_eq!(out[0].pid, 42);
        assert_eq!(&out[0].facility[..4], b"test");
        assert_eq!(&out[0].msg[..11], b"hello world");
    }

    #[test]
    fn test_klog_filters() {
        let mut ring = KlogRing::new();
        ring.append(KLOG_SEV_INFO, b"auth", b"login success", 10, 100);
        ring.append(KLOG_SEV_WARN, b"disk", b"space low", 1, 101);
        ring.append(KLOG_SEV_ERR, b"net", b"timeout", 2, 102);

        let mut out = [KlogEntry::empty(); 10];

        // Filter by facility
        let (n, _, _, _) = ring.read(0, KLOG_SEV_ANY, b"disk", &mut out);
        assert_eq!(n, 1);
        assert_eq!(&out[0].facility[..4], b"disk");

        // Filter by severity: only ERR (<= 3)
        let (n, _, _, _) = ring.read(0, KLOG_SEV_ERR, b"", &mut out);
        assert_eq!(n, 1);
        assert_eq!(out[0].severity, KLOG_SEV_ERR);

        // Filter by severity: WARN and above (<= 4) -> includes ERR and WARN
        let (n, _, _, _) = ring.read(0, KLOG_SEV_WARN, b"", &mut out);
        assert_eq!(n, 2);
    }

    #[test]
    fn test_klog_overflow_and_wrap() {
        let mut ring = KlogRing::new();
        for i in 0..500 {
            ring.append(KLOG_SEV_INFO, b"loop", b"msg", 0, i);
        }
        assert_eq!(ring.count, KLOG_CAPACITY);
        assert_eq!(ring.total_logged, 500);
        assert_eq!(ring.dropped_count, 500 - KLOG_CAPACITY as u64);

        let (oldest, newest, total, dropped, cap, count) = ring.info();
        assert_eq!(oldest, 500 - KLOG_CAPACITY as u64 + 1);
        assert_eq!(newest, 500);
        assert_eq!(total, 500);
        assert_eq!(dropped, 244);
        assert_eq!(cap, 256);
        assert_eq!(count, 256);
    }

    #[test]
    fn test_selftest_passes() {
        assert_eq!(rust_klog_selftest(), 0);
    }
}
