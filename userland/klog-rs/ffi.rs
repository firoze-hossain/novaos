//! userland/klog-rs/ffi.rs - FFI bindings and mock environment for NovaOS structured kernel logging.
//!
//! Provides the binary ABI interfaces matching `nova_klog_abi.h` for:
//!   - `SYS_KLOG_READ` (75)
//!   - `SYS_KLOG_WRITE` (76)
//!   - `SYS_KLOG_INFO` (77)
//! along with Framebuffer graphics and PS/2 mouse subsystem syscalls.
//!
//! In host test mode (`#[cfg(test)]`), an in-memory mock ring buffer simulates
//! the exact kernel behavior for headless verification.

#![allow(dead_code, static_mut_refs)]

// ===========================================================================
// NovaOS Structured Kernel Logging ABI (nova_klog_abi.h)
// ===========================================================================

pub const NOVA_SYS_KLOG_READ: u32 = 75;
pub const NOVA_SYS_KLOG_WRITE: u32 = 76;
pub const NOVA_SYS_KLOG_INFO: u32 = 77;

pub const NOVA_KLOG_EMERG: u32 = 0;
pub const NOVA_KLOG_ALERT: u32 = 1;
pub const NOVA_KLOG_CRIT: u32 = 2;
pub const NOVA_KLOG_ERR: u32 = 3;
pub const NOVA_KLOG_WARN: u32 = 4;
pub const NOVA_KLOG_NOTICE: u32 = 5;
pub const NOVA_KLOG_INFO: u32 = 6;
pub const NOVA_KLOG_DEBUG: u32 = 7;
pub const NOVA_KLOG_SEV_ANY: u32 = 0xFFFFFFFF;

pub const NOVA_KLOG_FACILITY_MAX: usize = 16;
pub const NOVA_KLOG_MSG_MAX: usize = 128;
pub const KLOG_BUFFER_CAPACITY: usize = 256;

/// A single structured log record (164 bytes in C ABI).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaKlogEntry {
    pub seq: u64,
    pub timestamp_ticks: u32,
    pub pid: u32,
    pub severity: u32,
    pub facility: [u8; NOVA_KLOG_FACILITY_MAX],
    pub msg: [u8; NOVA_KLOG_MSG_MAX],
}

impl NovaKlogEntry {
    pub const fn empty() -> Self {
        NovaKlogEntry {
            seq: 0,
            timestamp_ticks: 0,
            pid: 0,
            severity: NOVA_KLOG_INFO,
            facility: [0; NOVA_KLOG_FACILITY_MAX],
            msg: [0; NOVA_KLOG_MSG_MAX],
        }
    }
}

impl Default for NovaKlogEntry {
    fn default() -> Self {
        Self::empty()
    }
}

/// Query / Read request (60 bytes in 32-bit ABI).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaKlogRead {
    pub start_seq: u64,
    pub max_entries: u32,
    pub max_severity: u32,
    pub facility_filter: [u8; NOVA_KLOG_FACILITY_MAX],
    pub next_seq: u64,
    pub dropped_count: u64,
    pub total_logged: u64,
    pub entries: *mut NovaKlogEntry,
}

impl NovaKlogRead {
    pub const fn new(entries_ptr: *mut NovaKlogEntry, max_entries: u32) -> Self {
        NovaKlogRead {
            start_seq: 0,
            max_entries,
            max_severity: NOVA_KLOG_SEV_ANY,
            facility_filter: [0; NOVA_KLOG_FACILITY_MAX],
            next_seq: 0,
            dropped_count: 0,
            total_logged: 0,
            entries: entries_ptr,
        }
    }
}

impl Default for NovaKlogRead {
    fn default() -> Self {
        Self::new(core::ptr::null_mut(), 0)
    }
}

/// Write / Append request (12 bytes in 32-bit ABI).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NovaKlogWrite {
    pub severity: u32,
    pub facility: *const u8,
    pub msg: *const u8,
}

impl NovaKlogWrite {
    pub const fn new(severity: u32, facility: *const u8, msg: *const u8) -> Self {
        NovaKlogWrite {
            severity,
            facility,
            msg,
        }
    }
}

/// Subsystem statistics (40 bytes in 32-bit ABI).
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaKlogInfo {
    pub oldest_seq: u64,
    pub newest_seq: u64,
    pub total_logged: u64,
    pub dropped_count: u64,
    pub buffer_capacity: u32,
    pub entry_count: u32,
}

// ===========================================================================
// Graphics & Input ABI
// ===========================================================================

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbInfo {
    pub struct_size: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub backend: u32,
    pub backend_mask: u32,
    pub caps: u32,
    pub max_surface_dim: u32,
    pub max_surfaces: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbCreate {
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub handle: u32,
    pub pixels: u32,
    pub stride: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbPresent {
    pub handle: u32,
    pub src_x: i32,
    pub src_y: i32,
    pub width: i32,
    pub height: i32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub flags: u32,
}

pub const NOVA_FB_BACKEND_ANY: u32 = 0;

// ===========================================================================
// Bare-Metal Syscalls (not(test))
// ===========================================================================

#[cfg(not(test))]
extern "C" {
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_yield();
    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;

    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    pub fn sys_klog_read(req: *mut NovaKlogRead) -> i32;
    pub fn sys_klog_write(req: *const NovaKlogWrite) -> i32;
    pub fn sys_klog_info(out: *mut NovaKlogInfo) -> i32;
}

// ===========================================================================
// Host Unit Testing Mocks (cfg(test))
// ===========================================================================

#[cfg(test)]
pub static mut MOCK_KLOG_ENTRIES: [NovaKlogEntry; KLOG_BUFFER_CAPACITY] =
    [NovaKlogEntry::empty(); KLOG_BUFFER_CAPACITY];
#[cfg(test)]
pub static mut MOCK_KLOG_HEAD: usize = 0;
#[cfg(test)]
pub static mut MOCK_KLOG_TAIL: usize = 0;
#[cfg(test)]
pub static mut MOCK_KLOG_COUNT: usize = 0;
#[cfg(test)]
pub static mut MOCK_KLOG_NEXT_SEQ: u64 = 1;
#[cfg(test)]
pub static mut MOCK_KLOG_DROPPED: u64 = 0;
#[cfg(test)]
pub static mut MOCK_KLOG_TOTAL_LOGGED: u64 = 0;

#[cfg(test)]
pub unsafe fn reset_mock_environment() {
    MOCK_KLOG_ENTRIES = [NovaKlogEntry::empty(); KLOG_BUFFER_CAPACITY];
    MOCK_KLOG_HEAD = 0;
    MOCK_KLOG_TAIL = 0;
    MOCK_KLOG_COUNT = 0;
    MOCK_KLOG_NEXT_SEQ = 1;
    MOCK_KLOG_DROPPED = 0;
    MOCK_KLOG_TOTAL_LOGGED = 0;

    // Seed default boot messages for realistic initial state
    mock_append(NOVA_KLOG_NOTICE, b"kernel", b"NovaOS kernel 1.0.0 initializing on i686", 0, 10);
    mock_append(NOVA_KLOG_INFO, b"kernel", b"Memory manager detected 65536 KB physical RAM", 0, 25);
    mock_append(NOVA_KLOG_INFO, b"driver", b"PCI bus scan complete: 4 devices discovered", 0, 40);
    mock_append(NOVA_KLOG_INFO, b"driver", b"RTL8139 Fast Ethernet NIC detected at I/O 0xC000", 0, 65);
    mock_append(NOVA_KLOG_INFO, b"init", b"novainit service manager starting ring-3 daemons", 1, 100);
    mock_append(NOVA_KLOG_WARN, b"net", b"DHCP lease timed out, falling back to static IP 10.0.2.15", 4, 150);
    mock_append(NOVA_KLOG_INFO, b"fs", b"VFS mounted ext2 root partition at / (flags: rw)", 1, 190);
    mock_append(NOVA_KLOG_ERR, b"klogtest", b"simulated disk sector read retry count: 1", 5, 230);
}

#[cfg(test)]
unsafe fn mock_append(severity: u32, facility: &[u8], msg: &[u8], pid: u32, ticks: u32) -> u64 {
    let seq = MOCK_KLOG_NEXT_SEQ;
    MOCK_KLOG_NEXT_SEQ = MOCK_KLOG_NEXT_SEQ.wrapping_add(1);
    MOCK_KLOG_TOTAL_LOGGED = MOCK_KLOG_TOTAL_LOGGED.wrapping_add(1);

    let mut entry = NovaKlogEntry::empty();
    entry.seq = seq;
    entry.timestamp_ticks = ticks;
    entry.pid = pid;
    entry.severity = severity;

    let flen = facility.len().min(NOVA_KLOG_FACILITY_MAX - 1);
    for i in 0..flen {
        entry.facility[i] = facility[i];
    }
    entry.facility[flen] = 0;

    let mlen = msg.len().min(NOVA_KLOG_MSG_MAX - 1);
    for i in 0..mlen {
        entry.msg[i] = msg[i];
    }
    entry.msg[mlen] = 0;

    if MOCK_KLOG_COUNT == KLOG_BUFFER_CAPACITY {
        MOCK_KLOG_ENTRIES[MOCK_KLOG_HEAD] = entry;
        MOCK_KLOG_HEAD = (MOCK_KLOG_HEAD + 1) % KLOG_BUFFER_CAPACITY;
        MOCK_KLOG_TAIL = (MOCK_KLOG_TAIL + 1) % KLOG_BUFFER_CAPACITY;
        MOCK_KLOG_DROPPED = MOCK_KLOG_DROPPED.wrapping_add(1);
    } else {
        MOCK_KLOG_ENTRIES[MOCK_KLOG_HEAD] = entry;
        MOCK_KLOG_HEAD = (MOCK_KLOG_HEAD + 1) % KLOG_BUFFER_CAPACITY;
        MOCK_KLOG_COUNT += 1;
    }

    seq
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_yield() {}

#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if !out.is_null() {
        (*out).width = 1024;
        (*out).height = 768;
        (*out).format = 1;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_create(req: *mut NovaFbCreate) -> i32 {
    if !req.is_null() {
        (*req).handle = 1;
        (*req).stride = (*req).width * 4;
        (*req).size = (*req).stride * (*req).height;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_destroy(_handle: u32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_present(_req: *const NovaFbPresent) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_klog_info(out: *mut NovaKlogInfo) -> i32 {
    if out.is_null() {
        return -14; // -EFAULT
    }
    let oldest = if MOCK_KLOG_COUNT > 0 {
        MOCK_KLOG_ENTRIES[MOCK_KLOG_TAIL].seq
    } else {
        0
    };
    let newest = if MOCK_KLOG_COUNT > 0 {
        let last_idx = (MOCK_KLOG_HEAD + KLOG_BUFFER_CAPACITY - 1) % KLOG_BUFFER_CAPACITY;
        MOCK_KLOG_ENTRIES[last_idx].seq
    } else {
        0
    };
    *out = NovaKlogInfo {
        oldest_seq: oldest,
        newest_seq: newest,
        total_logged: MOCK_KLOG_TOTAL_LOGGED,
        dropped_count: MOCK_KLOG_DROPPED,
        buffer_capacity: KLOG_BUFFER_CAPACITY as u32,
        entry_count: MOCK_KLOG_COUNT as u32,
    };
    0
}

#[cfg(test)]
pub unsafe fn sys_klog_write(req: *const NovaKlogWrite) -> i32 {
    if req.is_null() {
        return -14; // -EFAULT
    }
    let r = &*req;
    if r.msg.is_null() {
        return -14;
    }

    let mut msg_len = 0;
    while msg_len < NOVA_KLOG_MSG_MAX && *r.msg.add(msg_len) != 0 {
        msg_len += 1;
    }
    let msg_bytes = core::slice::from_raw_parts(r.msg, msg_len);

    let fac_bytes: &[u8] = if !r.facility.is_null() {
        let mut flen = 0;
        while flen < NOVA_KLOG_FACILITY_MAX && *r.facility.add(flen) != 0 {
            flen += 1;
        }
        core::slice::from_raw_parts(r.facility, flen)
    } else {
        b"user"
    };

    mock_append(r.severity, fac_bytes, msg_bytes, 100, 300);
    0
}

#[cfg(test)]
pub unsafe fn sys_klog_read(req: *mut NovaKlogRead) -> i32 {
    if req.is_null() {
        return -14; // -EFAULT
    }
    let r = &mut *req;
    if r.entries.is_null() && r.max_entries > 0 {
        return -14;
    }

    if r.max_entries == 0 || MOCK_KLOG_COUNT == 0 {
        r.next_seq = MOCK_KLOG_NEXT_SEQ;
        r.dropped_count = MOCK_KLOG_DROPPED;
        r.total_logged = MOCK_KLOG_TOTAL_LOGGED;
        return 0;
    }

    let limit = (r.max_entries as usize).min(KLOG_BUFFER_CAPACITY);
    let mut written = 0usize;
    let mut highest_seq = 0u64;

    let mut filter_len = 0;
    while filter_len < NOVA_KLOG_FACILITY_MAX && r.facility_filter[filter_len] != 0 {
        filter_len += 1;
    }

    for i in 0..MOCK_KLOG_COUNT {
        let idx = (MOCK_KLOG_TAIL + i) % KLOG_BUFFER_CAPACITY;
        let entry = &MOCK_KLOG_ENTRIES[idx];

        if r.start_seq > 0 && entry.seq < r.start_seq {
            continue;
        }

        if r.max_severity <= NOVA_KLOG_DEBUG && entry.severity > r.max_severity {
            continue;
        }

        if filter_len > 0 {
            let mut match_ok = true;
            for j in 0..filter_len {
                if entry.facility[j] != r.facility_filter[j] {
                    match_ok = false;
                    break;
                }
            }
            if match_ok && filter_len < NOVA_KLOG_FACILITY_MAX && entry.facility[filter_len] != 0 {
                match_ok = false;
            }
            if !match_ok {
                continue;
            }
        }

        *r.entries.add(written) = *entry;
        highest_seq = entry.seq;
        written += 1;

        if written >= limit {
            break;
        }
    }

    r.next_seq = if highest_seq > 0 {
        highest_seq.wrapping_add(1)
    } else if r.start_seq > 0 {
        r.start_seq
    } else {
        MOCK_KLOG_NEXT_SEQ
    };
    r.dropped_count = MOCK_KLOG_DROPPED;
    r.total_logged = MOCK_KLOG_TOTAL_LOGGED;

    written as i32
}
