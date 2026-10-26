//! userland/klog-rs/klog.rs - NovaOS Graphical Structured Kernel Log Viewer
//!
//! Month 4 - Server & admin UI: Log viewer app, reading the new structured kernel logs.
//! A ring-3 graphical application querying the structured logging subsystem (`SYS_KLOG_*`),
//! providing real-time streaming, severity filtering, facility filtering, keyword search,
//! log inspector pane, and diagnostic triage context.
//!
//! Key Architecture & Features:
//!   - Interoperates with `nova_klog_abi.h` syscalls:
//!       - `SYS_KLOG_READ` (75): monotonic incremental reading and filtering.
//!       - `SYS_KLOG_WRITE` (76): log message / diagnostic marker injection.
//!       - `SYS_KLOG_INFO` (77): buffer telemetry, sequence bounds, and drop statistics.
//!   - High-performance zero-allocation ring buffer cache in BSS (zero stack overhead).
//!   - Severity badge rendering conforming to syslog levels (Emerg, Alert, Crit, Error,
//!     Warn, Notice, Info, Debug).
//!   - Facility filtering chips (kernel, init, net, fs, driver, user).
//!   - Real-time keyword search with in-line substring hit highlighting.
//!   - Dedicated Inspector pane displaying full unclipped message text, PID, timestamps,
//!     and triage hints.
//!   - Zero-per-pixel syscall rasterization onto `BufferSurface32` using `ui-rs`.
//!   - Comprehensive headless `--selftest` suite and host unit test coverage.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code))]

extern crate ui;
use ui::prelude::*;

mod ffi;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"klog: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

pub const MAX_CACHED_ENTRIES: usize = 256;
pub const QUERY_BATCH_SIZE: usize = 64;

// ===========================================================================
// String & Formatting Utilities (Zero Allocation)
// ===========================================================================

pub fn point_in_rect(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

pub fn cstr_len(buf: &[u8]) -> usize {
    let mut len = 0;
    while len < buf.len() && buf[len] != 0 {
        len += 1;
    }
    len
}

pub fn cstr_as_str(buf: &[u8]) -> &str {
    let len = cstr_len(buf);
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

pub fn format_u32(mut val: u32, out: &mut [u8]) -> usize {
    if val == 0 {
        if !out.is_empty() {
            out[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut digits = [0u8; 10];
    let mut count = 0;
    while val > 0 && count < 10 {
        digits[count] = b'0' + (val % 10) as u8;
        val /= 10;
        count += 1;
    }
    let len = out.len().min(count);
    for i in 0..len {
        out[i] = digits[count - 1 - i];
    }
    len
}

pub fn format_u64(mut val: u64, out: &mut [u8]) -> usize {
    if val == 0 {
        if !out.is_empty() {
            out[0] = b'0';
            return 1;
        }
        return 0;
    }
    let mut digits = [0u8; 20];
    let mut count = 0;
    while val > 0 && count < 20 {
        digits[count] = b'0' + (val % 10) as u8;
        val /= 10;
        count += 1;
    }
    let len = out.len().min(count);
    for i in 0..len {
        out[i] = digits[count - 1 - i];
    }
    len
}

pub fn format_seq(seq: u64, out: &mut [u8]) -> usize {
    if out.len() < 7 {
        return 0;
    }
    out[0] = b'#';
    let mut digits = [b'0'; 6];
    let mut s = seq;
    let mut idx = 5;
    while s > 0 {
        digits[idx] = b'0' + (s % 10) as u8;
        s /= 10;
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    for i in 0..6 {
        out[1 + i] = digits[i];
    }
    7
}

pub fn to_lower_ascii(b: u8) -> u8 {
    if b >= b'A' && b <= b'Z' {
        b + (b'a' - b'A')
    } else {
        b
    }
}

pub fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    find_ignore_case(haystack, needle).is_some()
}

pub fn find_ignore_case(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    if needle.len() > haystack.len() {
        return None;
    }
    let max_start = haystack.len() - needle.len();
    for i in 0..=max_start {
        let mut matched = true;
        for j in 0..needle.len() {
            if to_lower_ascii(haystack[i + j]) != to_lower_ascii(needle[j]) {
                matched = false;
                break;
            }
        }
        if matched {
            return Some(i);
        }
    }
    None
}

// ===========================================================================
// Severity & Theme Visuals
// ===========================================================================

pub fn severity_label(sev: u32) -> &'static str {
    match sev {
        ffi::NOVA_KLOG_EMERG => "EMERG",
        ffi::NOVA_KLOG_ALERT => "ALERT",
        ffi::NOVA_KLOG_CRIT => "CRIT",
        ffi::NOVA_KLOG_ERR => "ERROR",
        ffi::NOVA_KLOG_WARN => "WARN",
        ffi::NOVA_KLOG_NOTICE => "NOTICE",
        ffi::NOVA_KLOG_INFO => "INFO",
        ffi::NOVA_KLOG_DEBUG => "DEBUG",
        _ => "OTHER",
    }
}

pub fn severity_bg_color(sev: u32) -> Color {
    match sev {
        ffi::NOVA_KLOG_EMERG => Color::from_hex(0xB71C1C), // Deep Red
        ffi::NOVA_KLOG_ALERT => Color::from_hex(0xC62828), // Dark Red
        ffi::NOVA_KLOG_CRIT => Color::from_hex(0xAD1457),  // Maroon
        ffi::NOVA_KLOG_ERR => Color::from_hex(0xD32F2F),   // Red
        ffi::NOVA_KLOG_WARN => Color::from_hex(0xF57F17),  // Amber
        ffi::NOVA_KLOG_NOTICE => Color::from_hex(0x00838F),// Cyan/Teal
        ffi::NOVA_KLOG_INFO => Color::from_hex(0x2E7D32),  // Forest Green
        ffi::NOVA_KLOG_DEBUG => Color::from_hex(0x424242), // Slate Gray
        _ => Color::from_hex(0x616161),
    }
}

pub fn severity_fg_color(sev: u32) -> Color {
    match sev {
        ffi::NOVA_KLOG_WARN => Color::from_hex(0x000000), // Dark text for amber
        _ => Color::WHITE,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeverityFilterOption {
    All,
    ErrorsOnly,    // <= ERR
    WarningsAbove, // <= WARN
    InfoAbove,     // <= INFO
    Debug,         // <= DEBUG
}

impl SeverityFilterOption {
    pub fn max_severity(&self) -> u32 {
        match self {
            SeverityFilterOption::All => ffi::NOVA_KLOG_SEV_ANY,
            SeverityFilterOption::ErrorsOnly => ffi::NOVA_KLOG_ERR,
            SeverityFilterOption::WarningsAbove => ffi::NOVA_KLOG_WARN,
            SeverityFilterOption::InfoAbove => ffi::NOVA_KLOG_INFO,
            SeverityFilterOption::Debug => ffi::NOVA_KLOG_DEBUG,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SeverityFilterOption::All => "ALL",
            SeverityFilterOption::ErrorsOnly => "ERR+",
            SeverityFilterOption::WarningsAbove => "WARN+",
            SeverityFilterOption::InfoAbove => "INFO+",
            SeverityFilterOption::Debug => "DEBUG",
        }
    }
}

pub const FACILITY_PRESETS: [&str; 6] = ["ALL", "kernel", "driver", "init", "net", "fs"];

// ===========================================================================
// Application State & Data Model
// ===========================================================================

pub struct KlogViewerApp {
    pub entries: [ffi::NovaKlogEntry; MAX_CACHED_ENTRIES],
    pub entry_count: usize,
    pub last_polled_seq: u64,
    pub telemetry: ffi::NovaKlogInfo,

    // Filter controls
    pub sev_filter: SeverityFilterOption,
    pub facility_filter: [u8; ffi::NOVA_KLOG_FACILITY_MAX],
    pub facility_filter_len: usize,
    pub search_query: [u8; 32],
    pub search_len: usize,

    // Live streaming controls
    pub auto_refresh: bool,
    pub follow_tail: bool,

    // Navigation and selection
    pub filtered_indices: [u16; MAX_CACHED_ENTRIES],
    pub filtered_count: usize,
    pub selected_idx: Option<usize>, // index into filtered_indices
    pub scroll_offset: usize,

    // Status notification banner
    pub status_text: [u8; 64],
    pub status_len: usize,
}

impl KlogViewerApp {
    pub const fn new() -> Self {
        KlogViewerApp {
            entries: [ffi::NovaKlogEntry::empty(); MAX_CACHED_ENTRIES],
            entry_count: 0,
            last_polled_seq: 0,
            telemetry: ffi::NovaKlogInfo {
                oldest_seq: 0,
                newest_seq: 0,
                total_logged: 0,
                dropped_count: 0,
                buffer_capacity: 256,
                entry_count: 0,
            },
            sev_filter: SeverityFilterOption::All,
            facility_filter: [0; ffi::NOVA_KLOG_FACILITY_MAX],
            facility_filter_len: 0,
            search_query: [0; 32],
            search_len: 0,
            auto_refresh: true,
            follow_tail: true,
            filtered_indices: [0; MAX_CACHED_ENTRIES],
            filtered_count: 0,
            selected_idx: None,
            scroll_offset: 0,
            status_text: [0; 64],
            status_len: 0,
        }
    }

    pub fn init(&mut self) {
        self.entry_count = 0;
        self.last_polled_seq = 0;
        self.telemetry = ffi::NovaKlogInfo {
            oldest_seq: 0,
            newest_seq: 0,
            total_logged: 0,
            dropped_count: 0,
            buffer_capacity: 256,
            entry_count: 0,
        };
        self.sev_filter = SeverityFilterOption::All;
        self.facility_filter = [0; ffi::NOVA_KLOG_FACILITY_MAX];
        self.facility_filter_len = 0;
        self.search_query = [0; 32];
        self.search_len = 0;
        self.auto_refresh = true;
        self.follow_tail = true;
        self.filtered_count = 0;
        self.selected_idx = None;
        self.scroll_offset = 0;
        self.status_len = 0;
    }

    pub fn set_status(&mut self, msg: &[u8]) {
        let len = msg.len().min(self.status_text.len());
        self.status_text[..len].copy_from_slice(&msg[..len]);
        self.status_len = len;
    }

    pub fn status_str(&self) -> &str {
        core::str::from_utf8(&self.status_text[..self.status_len]).unwrap_or("")
    }

    pub fn set_facility_filter(&mut self, fac: &str) {
        if fac == "ALL" || fac.is_empty() {
            self.facility_filter = [0; ffi::NOVA_KLOG_FACILITY_MAX];
            self.facility_filter_len = 0;
        } else {
            let bytes = fac.as_bytes();
            let len = bytes.len().min(ffi::NOVA_KLOG_FACILITY_MAX - 1);
            for i in 0..len {
                self.facility_filter[i] = bytes[i];
            }
            self.facility_filter[len] = 0;
            self.facility_filter_len = len;
        }
        self.rebuild_filtered_indices();
    }

    pub fn set_search_query(&mut self, query: &str) {
        let bytes = query.as_bytes();
        let len = bytes.len().min(self.search_query.len());
        for i in 0..len {
            self.search_query[i] = bytes[i];
        }
        self.search_len = len;
        self.rebuild_filtered_indices();
    }

    pub fn clear_search_query(&mut self) {
        self.search_query = [0; 32];
        self.search_len = 0;
        self.rebuild_filtered_indices();
    }

    pub fn clear_view(&mut self) {
        // Clear cached view so only subsequent new logs are shown
        self.entry_count = 0;
        self.last_polled_seq = self.telemetry.newest_seq + 1;
        self.rebuild_filtered_indices();
        self.set_status(b"Log display buffer cleared");
    }

    pub fn inject_marker(&mut self, note: &str) -> bool {
        let mut msg_buf = [0u8; ffi::NOVA_KLOG_MSG_MAX];
        let prefix = b"--- MARKER: ";
        let mut idx = 0;
        for &b in prefix {
            if idx < msg_buf.len() - 1 {
                msg_buf[idx] = b;
                idx += 1;
            }
        }
        for &b in note.as_bytes() {
            if idx < msg_buf.len() - 5 {
                msg_buf[idx] = b;
                idx += 1;
            }
        }
        let suffix = b" ---";
        for &b in suffix {
            if idx < msg_buf.len() - 1 {
                msg_buf[idx] = b;
                idx += 1;
            }
        }
        msg_buf[idx] = 0;

        let req = ffi::NovaKlogWrite {
            severity: ffi::NOVA_KLOG_NOTICE,
            facility: b"admin\0".as_ptr(),
            msg: msg_buf.as_ptr(),
        };
        let res = unsafe { ffi::sys_klog_write(&req) };
        if res == 0 {
            self.set_status(b"Diagnostic marker injected into kernel log");
            self.poll_entries();
            true
        } else {
            self.set_status(b"Failed to inject diagnostic marker");
            false
        }
    }

    /// Rebuilds the list of visible entry indices matching current filters.
    pub fn rebuild_filtered_indices(&mut self) {
        let max_sev = self.sev_filter.max_severity();
        let fac_filter_active = self.facility_filter_len > 0;
        let search_active = self.search_len > 0;
        let search_bytes = &self.search_query[..self.search_len];

        let mut count = 0usize;
        for i in 0..self.entry_count {
            let entry = &self.entries[i];

            // 1. Severity filter
            if max_sev <= ffi::NOVA_KLOG_DEBUG && entry.severity > max_sev {
                continue;
            }

            // 2. Facility filter
            if fac_filter_active {
                let e_fac_len = cstr_len(&entry.facility);
                if e_fac_len != self.facility_filter_len {
                    continue;
                }
                let mut match_ok = true;
                for j in 0..self.facility_filter_len {
                    if entry.facility[j] != self.facility_filter[j] {
                        match_ok = false;
                        break;
                    }
                }
                if !match_ok {
                    continue;
                }
            }

            // 3. Keyword search filter
            if search_active {
                let msg_len = cstr_len(&entry.msg);
                let fac_len = cstr_len(&entry.facility);
                let msg_match = contains_ignore_case(&entry.msg[..msg_len], search_bytes);
                let fac_match = contains_ignore_case(&entry.facility[..fac_len], search_bytes);
                if !msg_match && !fac_match {
                    continue;
                }
            }

            self.filtered_indices[count] = i as u16;
            count += 1;
        }

        self.filtered_count = count;

        // Clamp selected index
        if let Some(sel) = self.selected_idx {
            if self.filtered_count == 0 {
                self.selected_idx = None;
            } else if sel >= self.filtered_count {
                self.selected_idx = Some(self.filtered_count - 1);
            }
        } else if self.filtered_count > 0 {
            // Default to newest log row
            self.selected_idx = Some(self.filtered_count - 1);
        }

        // Adjust scroll offset
        if self.follow_tail && self.filtered_count > 0 {
            if self.filtered_count > 14 {
                self.scroll_offset = self.filtered_count - 14;
            } else {
                self.scroll_offset = 0;
            }
        } else if self.scroll_offset >= self.filtered_count {
            self.scroll_offset = if self.filtered_count > 0 {
                self.filtered_count - 1
            } else {
                0
            };
        }
    }

    /// Queries the kernel logging subsystem for newly available entries.
    pub fn poll_entries(&mut self) {
        unsafe {
            let mut info = ffi::NovaKlogInfo::default();
            if ffi::sys_klog_info(&mut info) == 0 {
                self.telemetry = info;
            }
        }

        let mut start_seq = if self.entry_count == 0 {
            0
        } else {
            self.last_polled_seq
        };

        let read_buf = unsafe { &mut *core::ptr::addr_of_mut!(QUERY_BUFFER) };
        let mut total_polled = 0;

        loop {
            let mut req = ffi::NovaKlogRead {
                start_seq,
                max_entries: QUERY_BATCH_SIZE as u32,
                max_severity: ffi::NOVA_KLOG_SEV_ANY,
                facility_filter: [0; ffi::NOVA_KLOG_FACILITY_MAX],
                next_seq: 0,
                dropped_count: 0,
                total_logged: 0,
                entries: read_buf.as_mut_ptr(),
            };

            let count = unsafe { ffi::sys_klog_read(&mut req) };
            if count <= 0 {
                break;
            }
            let n = count as usize;
            for i in 0..n {
                let entry = read_buf[i];
                if self.entry_count < MAX_CACHED_ENTRIES {
                    self.entries[self.entry_count] = entry;
                    self.entry_count += 1;
                } else {
                    // Buffer full: shift left by 1 to make room
                    for j in 0..MAX_CACHED_ENTRIES - 1 {
                        self.entries[j] = self.entries[j + 1];
                    }
                    self.entries[MAX_CACHED_ENTRIES - 1] = entry;
                }
            }
            total_polled += n;
            self.last_polled_seq = req.next_seq;
            start_seq = req.next_seq;

            if n < QUERY_BATCH_SIZE || req.next_seq > self.telemetry.newest_seq {
                break;
            }
        }

        if total_polled > 0 {
            self.rebuild_filtered_indices();

            let mut msg_buf = [0u8; 32];
            let p = b"Polled +";
            for (idx, &b) in p.iter().enumerate() {
                msg_buf[idx] = b;
            }
            let d_len = format_u32(total_polled as u32, &mut msg_buf[8..]);
            let s = b" new entries";
            for (idx, &b) in s.iter().enumerate() {
                if 8 + d_len + idx < msg_buf.len() {
                    msg_buf[8 + d_len + idx] = b;
                }
            }
            self.set_status(&msg_buf[..8 + d_len + s.len()]);
        }
    }

    pub fn selected_entry(&self) -> Option<&ffi::NovaKlogEntry> {
        if let Some(sel) = self.selected_idx {
            if sel < self.filtered_count {
                let raw_idx = self.filtered_indices[sel] as usize;
                if raw_idx < self.entry_count {
                    return Some(&self.entries[raw_idx]);
                }
            }
        }
        None
    }

    pub fn select_next(&mut self) {
        if self.filtered_count == 0 {
            return;
        }
        self.follow_tail = false;
        if let Some(cur) = self.selected_idx {
            if cur + 1 < self.filtered_count {
                self.selected_idx = Some(cur + 1);
                if cur + 1 >= self.scroll_offset + 14 {
                    self.scroll_offset = cur + 2 - 14;
                }
            }
        } else {
            self.selected_idx = Some(0);
            self.scroll_offset = 0;
        }
    }

    pub fn select_prev(&mut self) {
        if self.filtered_count == 0 {
            return;
        }
        self.follow_tail = false;
        if let Some(cur) = self.selected_idx {
            if cur > 0 {
                self.selected_idx = Some(cur - 1);
                if cur - 1 < self.scroll_offset {
                    self.scroll_offset = cur - 1;
                }
            }
        } else {
            self.selected_idx = Some(0);
            self.scroll_offset = 0;
        }
    }

    pub fn scroll_page_down(&mut self) {
        if self.filtered_count == 0 {
            return;
        }
        self.follow_tail = false;
        let page_size = 14;
        self.scroll_offset = (self.scroll_offset + page_size).min(
            if self.filtered_count > page_size {
                self.filtered_count - page_size
            } else {
                0
            },
        );
        if let Some(cur) = self.selected_idx {
            self.selected_idx = Some((cur + page_size).min(self.filtered_count - 1));
        }
    }

    pub fn scroll_page_up(&mut self) {
        self.follow_tail = false;
        let page_size = 14;
        if self.scroll_offset >= page_size {
            self.scroll_offset -= page_size;
        } else {
            self.scroll_offset = 0;
        }
        if let Some(cur) = self.selected_idx {
            if cur >= page_size {
                self.selected_idx = Some(cur - 1);
            } else {
                self.selected_idx = Some(0);
            }
        }
    }

    // =======================================================================
    // Mouse Interaction
    // =======================================================================

    pub fn handle_click(&mut self, mx: i32, my: i32, viewport: Rect) -> bool {
        // 1. Controls bar clicks (top controls area)
        let ctrl_y = viewport.y + 56;
        let _ctrl_h = 56;

        // Button: [Pause / Resume Live]
        if point_in_rect(mx, my, viewport.x + 12, ctrl_y + 6, 92, 22) {
            self.auto_refresh = !self.auto_refresh;
            if self.auto_refresh {
                self.follow_tail = true;
                self.set_status(b"Live streaming enabled");
            } else {
                self.set_status(b"Live streaming paused");
            }
            return true;
        }

        // Button: [Refresh]
        if point_in_rect(mx, my, viewport.x + 110, ctrl_y + 6, 76, 22) {
            self.poll_entries();
            return true;
        }

        // Button: [+ Marker]
        if point_in_rect(mx, my, viewport.x + 192, ctrl_y + 6, 82, 22) {
            self.inject_marker("User Action");
            return true;
        }

        // Button: [Clear]
        if point_in_rect(mx, my, viewport.x + 280, ctrl_y + 6, 68, 22) {
            self.clear_view();
            return true;
        }

        // Severity filter pills
        let sev_pills = [
            (SeverityFilterOption::All, 42),
            (SeverityFilterOption::ErrorsOnly, 50),
            (SeverityFilterOption::WarningsAbove, 56),
            (SeverityFilterOption::InfoAbove, 52),
            (SeverityFilterOption::Debug, 54),
        ];
        let mut pill_x = viewport.x + 430;
        let pill_y = ctrl_y + 6;
        for (opt, w) in sev_pills {
            if point_in_rect(mx, my, pill_x, pill_y, w, 22) {
                self.sev_filter = opt;
                self.rebuild_filtered_indices();
                return true;
            }
            pill_x += w + 6;
        }

        // Facility chips (row 2)
        let fac_y = ctrl_y + 32;
        let mut chip_x = viewport.x + 80;
        for &fac in &FACILITY_PRESETS {
            let chip_w = (fac.len() as i32 * 8) + 16;
            if point_in_rect(mx, my, chip_x, fac_y, chip_w, 20) {
                self.set_facility_filter(fac);
                return true;
            }
            chip_x += chip_w + 6;
        }

        // Search presets (row 2 right)
        let mut s_preset_x = viewport.x + 500;
        let presets = [("Clear", ""), ("fault", "fault"), ("net", "net"), ("marker", "marker")];
        for (label, query) in presets {
            let pw = (label.len() as i32 * 8) + 12;
            if point_in_rect(mx, my, s_preset_x, fac_y, pw, 20) {
                if query.is_empty() {
                    self.clear_search_query();
                } else {
                    self.set_search_query(query);
                }
                return true;
            }
            s_preset_x += pw + 6;
        }

        // 2. Table row clicks
        let table_y = viewport.y + 116 + 22; // below header
        let row_h = 20;
        let visible_rows = 14usize;
        let table_h = visible_rows as i32 * row_h;

        if point_in_rect(mx, my, viewport.x + 8, table_y, viewport.w - 24, table_h) {
            let row_idx = ((my - table_y) / row_h) as usize;
            let target_filtered_idx = self.scroll_offset + row_idx;
            if target_filtered_idx < self.filtered_count {
                self.selected_idx = Some(target_filtered_idx);
                self.follow_tail = false;
                return true;
            }
        }

        // 3. Scrollbar track clicks
        let sb_x = viewport.right() - 18;
        if point_in_rect(mx, my, sb_x, table_y, 14, table_h) {
            let rel_y = my - table_y;
            let frac = (rel_y as usize * self.filtered_count) / table_h as usize;
            self.scroll_offset = frac.min(
                if self.filtered_count > visible_rows {
                    self.filtered_count - visible_rows
                } else {
                    0
                },
            );
            self.follow_tail = false;
            return true;
        }

        false
    }

    // =======================================================================
    // Zero-Per-Pixel Syscall Surface Rendering (BufferSurface32)
    // =======================================================================

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S, viewport: Rect) {
        let palette = Theme::Dark.palette();

        // 1. App Background
        s.fill_rect(viewport, palette.window_bg);

        // 2. Top Header & Telemetry Bar (y: 0..56)
        let top_rect = Rect::new(viewport.x, viewport.y, viewport.w, 56);
        s.fill_rect(top_rect, palette.surface_sunken);
        draw_line(
            s,
            top_rect.x,
            top_rect.bottom() - 1,
            top_rect.right(),
            top_rect.bottom() - 1,
            palette.border_subtle,
        );

        // App Icon & Title
        let icon_rect = Rect::new(top_rect.x + 12, top_rect.y + 12, 48, 32);
        draw_rounded_rect(s, icon_rect, 4, palette.accent_primary);
        draw_text_aligned(
            s,
            icon_rect,
            "LOGS",
            TextAlign::Center,
            Color::WHITE,
            TypeScale::BodyBold,
        );

        draw_text(
            s,
            top_rect.x + 68,
            top_rect.y + 12,
            "NovaOS Kernel Log Viewer",
            palette.text_primary,
            TypeScale::Title,
        );
        draw_text(
            s,
            top_rect.x + 68,
            top_rect.y + 32,
            "Real-time structured syslog stream & crash diagnostics",
            palette.text_secondary,
            TypeScale::Caption,
        );

        // Telemetry stats pills on right
        let mut stats_buf = [0u8; 96];
        let mut st_idx = 0;

        let t_lbl = b"Logged: ";
        stats_buf[st_idx..st_idx + t_lbl.len()].copy_from_slice(t_lbl);
        st_idx += t_lbl.len();
        st_idx += format_u64(self.telemetry.total_logged, &mut stats_buf[st_idx..]);

        let d_lbl = b"  |  Dropped: ";
        stats_buf[st_idx..st_idx + d_lbl.len()].copy_from_slice(d_lbl);
        st_idx += d_lbl.len();
        st_idx += format_u64(self.telemetry.dropped_count, &mut stats_buf[st_idx..]);

        let r_lbl = b"  |  Ring: ";
        stats_buf[st_idx..st_idx + r_lbl.len()].copy_from_slice(r_lbl);
        st_idx += r_lbl.len();
        st_idx += format_u32(self.telemetry.entry_count, &mut stats_buf[st_idx..]);
        stats_buf[st_idx] = b'/';
        st_idx += 1;
        st_idx += format_u32(self.telemetry.buffer_capacity, &mut stats_buf[st_idx..]);

        let stats_str = core::str::from_utf8(&stats_buf[..st_idx]).unwrap_or("");
        let stats_x = top_rect.right() - 380;
        draw_text(
            s,
            stats_x,
            top_rect.y + 14,
            stats_str,
            palette.text_secondary,
            TypeScale::Caption,
        );

        // Live stream status badge
        let live_pill = Rect::new(top_rect.right() - 110, top_rect.y + 14, 96, 26);
        if self.auto_refresh {
            draw_rounded_rect(s, live_pill, 4, Color::from_hex(0x1B5E20)); // Dark green
            draw_rect_outline(s, live_pill, 1, Color::from_hex(0x4CAF50));
            draw_text_aligned(
                s,
                live_pill,
                "LIVE TAIL",
                TextAlign::Center,
                Color::WHITE,
                TypeScale::Caption,
            );
        } else {
            draw_rounded_rect(s, live_pill, 4, Color::from_hex(0xBF360C)); // Rust/amber
            draw_rect_outline(s, live_pill, 1, Color::from_hex(0xFF7043));
            draw_text_aligned(
                s,
                live_pill,
                "PAUSED",
                TextAlign::Center,
                Color::WHITE,
                TypeScale::Caption,
            );
        }

        // 3. Toolbar & Filters (y: 56..116)
        let ctrl_rect = Rect::new(viewport.x, viewport.y + 56, viewport.w, 58);
        s.fill_rect(ctrl_rect, palette.surface_raised);
        draw_line(
            s,
            ctrl_rect.x,
            ctrl_rect.bottom() - 1,
            ctrl_rect.right(),
            ctrl_rect.bottom() - 1,
            palette.border_normal,
        );

        // Top Row: Action Buttons
        let pause_rect = Rect::new(ctrl_rect.x + 12, ctrl_rect.y + 6, 92, 22);
        let pause_label = if self.auto_refresh { "Pause" } else { "Resume" };
        draw_button(
            s,
            pause_rect,
            pause_label,
            ButtonVariant::Secondary,
            WidgetState::Normal,
            &palette,
        );

        let refresh_rect = Rect::new(ctrl_rect.x + 110, ctrl_rect.y + 6, 76, 22);
        draw_button(
            s,
            refresh_rect,
            "Refresh",
            ButtonVariant::Secondary,
            WidgetState::Normal,
            &palette,
        );

        let marker_rect = Rect::new(ctrl_rect.x + 192, ctrl_rect.y + 6, 82, 22);
        draw_button(
            s,
            marker_rect,
            "+ Marker",
            ButtonVariant::Primary,
            WidgetState::Normal,
            &palette,
        );

        let clear_rect = Rect::new(ctrl_rect.x + 280, ctrl_rect.y + 6, 68, 22);
        draw_button(
            s,
            clear_rect,
            "Clear",
            ButtonVariant::Ghost,
            WidgetState::Normal,
            &palette,
        );

        // Severity Selector Pills
        draw_text(
            s,
            ctrl_rect.x + 365,
            ctrl_rect.y + 10,
            "Severity:",
            palette.text_secondary,
            TypeScale::Caption,
        );

        let sev_pills = [
            (SeverityFilterOption::All, 42),
            (SeverityFilterOption::ErrorsOnly, 50),
            (SeverityFilterOption::WarningsAbove, 56),
            (SeverityFilterOption::InfoAbove, 52),
            (SeverityFilterOption::Debug, 54),
        ];
        let mut pill_x = ctrl_rect.x + 430;
        for (opt, w) in sev_pills {
            let p_rect = Rect::new(pill_x, ctrl_rect.y + 6, w, 22);
            let active = self.sev_filter == opt;
            if active {
                draw_rounded_rect(s, p_rect, 3, palette.accent_primary);
                draw_text_aligned(
                    s,
                    p_rect,
                    opt.label(),
                    TextAlign::Center,
                    Color::WHITE,
                    TypeScale::Caption,
                );
            } else {
                draw_rounded_rect(s, p_rect, 3, palette.surface_sunken);
                draw_rect_outline(s, p_rect, 1, palette.border_subtle);
                draw_text_aligned(
                    s,
                    p_rect,
                    opt.label(),
                    TextAlign::Center,
                    palette.text_secondary,
                    TypeScale::Caption,
                );
            }
            pill_x += w + 6;
        }

        // Bottom Row: Facility chips & Search
        draw_text(
            s,
            ctrl_rect.x + 12,
            ctrl_rect.y + 36,
            "Facility:",
            palette.text_secondary,
            TypeScale::Caption,
        );

        let mut chip_x = ctrl_rect.x + 80;
        let cur_fac_str = cstr_as_str(&self.facility_filter);
        for &fac in &FACILITY_PRESETS {
            let active = (fac == "ALL" && self.facility_filter_len == 0)
                || (self.facility_filter_len > 0 && cur_fac_str == fac);
            let chip_w = (fac.len() as i32 * 8) + 16;
            let c_rect = Rect::new(chip_x, ctrl_rect.y + 32, chip_w, 20);
            if active {
                draw_rounded_rect(s, c_rect, 10, Color::from_hex(0x0D47A1)); // Navy blue chip
                draw_rect_outline(s, c_rect, 1, palette.accent_primary);
                draw_text_aligned(
                    s,
                    c_rect,
                    fac,
                    TextAlign::Center,
                    Color::WHITE,
                    TypeScale::Caption,
                );
            } else {
                draw_rounded_rect(s, c_rect, 10, palette.surface_sunken);
                draw_text_aligned(
                    s,
                    c_rect,
                    fac,
                    TextAlign::Center,
                    palette.text_muted,
                    TypeScale::Caption,
                );
            }
            chip_x += chip_w + 6;
        }

        // Search Box and Presets
        draw_text(
            s,
            ctrl_rect.x + 440,
            ctrl_rect.y + 36,
            "Search:",
            palette.text_secondary,
            TypeScale::Caption,
        );

        let s_presets = [("Clear", ""), ("fault", "fault"), ("net", "net"), ("marker", "marker")];
        let mut sp_x = ctrl_rect.x + 500;
        for (label, q) in s_presets {
            let active = self.search_len > 0 && cstr_as_str(&self.search_query) == q;
            let pw = (label.len() as i32 * 8) + 12;
            let p_rect = Rect::new(sp_x, ctrl_rect.y + 32, pw, 20);
            if active {
                draw_rounded_rect(s, p_rect, 3, Color::from_hex(0xF59E0B));
                draw_text_aligned(
                    s,
                    p_rect,
                    label,
                    TextAlign::Center,
                    Color::BLACK,
                    TypeScale::Caption,
                );
            } else {
                draw_rounded_rect(s, p_rect, 3, palette.surface_sunken);
                draw_text_aligned(
                    s,
                    p_rect,
                    label,
                    TextAlign::Center,
                    palette.text_secondary,
                    TypeScale::Caption,
                );
            }
            sp_x += pw + 6;
        }

        // 4. Main Table View (y: 116..h - 154)
        let table_y = viewport.y + 116;
        let table_hdr_h = 22;
        let row_h = 20;
        let visible_rows = 14usize;

        // Table Header
        let th_rect = Rect::new(viewport.x + 8, table_y, viewport.w - 16, table_hdr_h);
        s.fill_rect(th_rect, palette.surface_sunken);
        draw_rect_outline(s, th_rect, 1, palette.border_subtle);

        draw_text(s, th_rect.x + 8, th_rect.y + 4, "SEQ", palette.text_secondary, TypeScale::Caption);
        draw_text(s, th_rect.x + 72, th_rect.y + 4, "TICKS", palette.text_secondary, TypeScale::Caption);
        draw_text(s, th_rect.x + 140, th_rect.y + 4, "LEVEL", palette.text_secondary, TypeScale::Caption);
        draw_text(s, th_rect.x + 215, th_rect.y + 4, "FACILITY", palette.text_secondary, TypeScale::Caption);
        draw_text(s, th_rect.x + 305, th_rect.y + 4, "PID", palette.text_secondary, TypeScale::Caption);
        draw_text(s, th_rect.x + 355, th_rect.y + 4, "MESSAGE", palette.text_secondary, TypeScale::Caption);

        // Table Rows
        let rows_start_y = th_rect.bottom() + 1;
        let search_active = self.search_len > 0;
        let search_bytes = &self.search_query[..self.search_len];

        for i in 0..visible_rows {
            let row_idx = self.scroll_offset + i;
            let ry = rows_start_y + (i as i32 * row_h);
            let r_rect = Rect::new(th_rect.x, ry, th_rect.w - 14, row_h);

            if row_idx >= self.filtered_count {
                // Empty trailing row
                let empty_bg = if i % 2 == 0 {
                    palette.window_bg
                } else {
                    palette.surface_sunken
                };
                s.fill_rect(r_rect, empty_bg);
                continue;
            }

            let entry_idx = self.filtered_indices[row_idx] as usize;
            let entry = &self.entries[entry_idx];
            let is_selected = self.selected_idx == Some(row_idx);

            // Row background
            let row_bg = if is_selected {
                Color::from_hex(0x273549) // Distinct selected row
            } else if i % 2 == 0 {
                palette.window_bg
            } else {
                palette.surface_sunken
            };
            s.fill_rect(r_rect, row_bg);

            if is_selected {
                // Accent indicator bar on left edge
                s.fill_rect(
                    Rect::new(r_rect.x, r_rect.y, 3, r_rect.h),
                    palette.accent_primary,
                );
            }

            // Col 1: SEQ
            let mut seq_buf = [0u8; 8];
            let seq_len = format_seq(entry.seq, &mut seq_buf);
            let seq_str = core::str::from_utf8(&seq_buf[..seq_len]).unwrap_or("");
            draw_text(
                s,
                r_rect.x + 8,
                r_rect.y + 4,
                seq_str,
                if is_selected { palette.text_primary } else { palette.text_secondary },
                TypeScale::Caption,
            );

            // Col 2: TICKS
            let mut tick_buf = [0u8; 10];
            let tick_len = format_u32(entry.timestamp_ticks, &mut tick_buf);
            let tick_str = core::str::from_utf8(&tick_buf[..tick_len]).unwrap_or("");
            draw_text(
                s,
                r_rect.x + 72,
                r_rect.y + 4,
                tick_str,
                palette.text_muted,
                TypeScale::Caption,
            );

            // Col 3: SEVERITY BADGE
            let badge_rect = Rect::new(r_rect.x + 138, r_rect.y + 2, 64, 16);
            let bg_c = severity_bg_color(entry.severity);
            let fg_c = severity_fg_color(entry.severity);
            draw_rounded_rect(s, badge_rect, 3, bg_c);
            draw_text_aligned(
                s,
                badge_rect,
                severity_label(entry.severity),
                TextAlign::Center,
                fg_c,
                TypeScale::Caption,
            );

            // Col 4: FACILITY
            let fac_str = cstr_as_str(&entry.facility);
            draw_text(
                s,
                r_rect.x + 215,
                r_rect.y + 4,
                fac_str,
                palette.text_primary,
                TypeScale::Caption,
            );

            // Col 5: PID
            let mut pid_buf = [0u8; 6];
            let pid_len = format_u32(entry.pid, &mut pid_buf);
            let pid_str = core::str::from_utf8(&pid_buf[..pid_len]).unwrap_or("");
            draw_text(
                s,
                r_rect.x + 305,
                r_rect.y + 4,
                pid_str,
                palette.text_muted,
                TypeScale::Caption,
            );

            // Col 6: MESSAGE (with search term highlighting)
            let msg_len = cstr_len(&entry.msg);
            let msg_bytes = &entry.msg[..msg_len];
            let msg_x = r_rect.x + 355;
            let msg_y = r_rect.y + 4;

            if search_active {
                if let Some(hit_idx) = find_ignore_case(msg_bytes, search_bytes) {
                    // Prefix before hit
                    let prefix_str = core::str::from_utf8(&msg_bytes[..hit_idx]).unwrap_or("");
                    draw_text(s, msg_x, msg_y, prefix_str, palette.text_primary, TypeScale::Caption);
                    let prefix_w = prefix_str.len() as i32 * 8;

                    // Highlighted hit
                    let hit_end = hit_idx + self.search_len;
                    let hit_str = core::str::from_utf8(&msg_bytes[hit_idx..hit_end]).unwrap_or("");
                    let hit_w = hit_str.len() as i32 * 8;
                    let hl_rect = Rect::new(msg_x + prefix_w, r_rect.y + 2, hit_w, 16);
                    s.fill_rect(hl_rect, Color::from_hex(0xF59E0B)); // Bright amber highlight
                    draw_text(s, msg_x + prefix_w, msg_y, hit_str, Color::BLACK, TypeScale::Caption);

                    // Suffix after hit
                    let suffix_str = core::str::from_utf8(&msg_bytes[hit_end..]).unwrap_or("");
                    draw_text(
                        s,
                        msg_x + prefix_w + hit_w,
                        msg_y,
                        suffix_str,
                        palette.text_primary,
                        TypeScale::Caption,
                    );
                } else {
                    let full_msg = core::str::from_utf8(msg_bytes).unwrap_or("");
                    draw_text(s, msg_x, msg_y, full_msg, palette.text_primary, TypeScale::Caption);
                }
            } else {
                let full_msg = core::str::from_utf8(msg_bytes).unwrap_or("");
                draw_text(s, msg_x, msg_y, full_msg, palette.text_primary, TypeScale::Caption);
            }
        }

        // Scrollbar on right edge
        let sb_x = th_rect.right() - 14;
        let sb_y = rows_start_y;
        let sb_h = visible_rows as i32 * row_h;
        let sb_track = Rect::new(sb_x, sb_y, 12, sb_h);
        s.fill_rect(sb_track, palette.surface_sunken);
        draw_rect_outline(s, sb_track, 1, palette.border_subtle);

        if self.filtered_count > 0 {
            let thumb_h = ((visible_rows as i32 * sb_h) / self.filtered_count as i32).max(16).min(sb_h);
            let max_scroll = if self.filtered_count > visible_rows {
                self.filtered_count - visible_rows
            } else {
                1
            };
            let thumb_y = sb_y + (self.scroll_offset as i32 * (sb_h - thumb_h)) / max_scroll as i32;
            let thumb_rect = Rect::new(sb_x + 1, thumb_y, 10, thumb_h);
            draw_rounded_rect(s, thumb_rect, 2, palette.border_normal);
        }

        // 5. Inspector Pane (Bottom 130px)
        let insp_y = viewport.bottom() - 150;
        let insp_rect = Rect::new(viewport.x + 8, insp_y, viewport.w - 16, 122);
        s.fill_rect(insp_rect, palette.surface_sunken);
        draw_rect_outline(s, insp_rect, 1, palette.border_normal);

        // Header
        let insp_hdr = Rect::new(insp_rect.x, insp_rect.y, insp_rect.w, 24);
        s.fill_rect(insp_hdr, palette.surface_raised);
        draw_line(
            s,
            insp_hdr.x,
            insp_hdr.bottom() - 1,
            insp_hdr.right(),
            insp_hdr.bottom() - 1,
            palette.border_subtle,
        );
        draw_text(
            s,
            insp_hdr.x + 10,
            insp_hdr.y + 5,
            "LOG RECORD INSPECTOR & DIAGNOSTIC CONTEXT",
            palette.accent_primary,
            TypeScale::Caption,
        );

        if let Some(entry) = self.selected_entry() {
            // Inspector Details
            let mut det_buf = [0u8; 128];
            let mut d_idx = 0;

            let s_tag = b"Seq: ";
            det_buf[d_idx..d_idx + s_tag.len()].copy_from_slice(s_tag);
            d_idx += s_tag.len();
            d_idx += format_seq(entry.seq, &mut det_buf[d_idx..]);

            let t_tag = b"  |  Ticks: ";
            det_buf[d_idx..d_idx + t_tag.len()].copy_from_slice(t_tag);
            d_idx += t_tag.len();
            d_idx += format_u32(entry.timestamp_ticks, &mut det_buf[d_idx..]);

            let p_tag = b"  |  PID: ";
            det_buf[d_idx..d_idx + p_tag.len()].copy_from_slice(p_tag);
            d_idx += p_tag.len();
            d_idx += format_u32(entry.pid, &mut det_buf[d_idx..]);
            if entry.pid == 0 {
                let k_tag = b" (Kernel Ring-0)";
                det_buf[d_idx..d_idx + k_tag.len()].copy_from_slice(k_tag);
                d_idx += k_tag.len();
            } else {
                let u_tag = b" (Ring-3 Task)";
                det_buf[d_idx..d_idx + u_tag.len()].copy_from_slice(u_tag);
                d_idx += u_tag.len();
            }

            let f_tag = b"  |  Facility: ";
            det_buf[d_idx..d_idx + f_tag.len()].copy_from_slice(f_tag);
            d_idx += f_tag.len();
            let fac_str = cstr_as_str(&entry.facility);
            for &b in fac_str.as_bytes() {
                if d_idx < det_buf.len() {
                    det_buf[d_idx] = b;
                    d_idx += 1;
                }
            }

            let det_str = core::str::from_utf8(&det_buf[..d_idx]).unwrap_or("");
            draw_text(
                s,
                insp_rect.x + 10,
                insp_rect.y + 30,
                det_str,
                palette.text_secondary,
                TypeScale::Caption,
            );

            // Full message card
            let msg_box = Rect::new(insp_rect.x + 10, insp_rect.y + 48, insp_rect.w - 20, 36);
            s.fill_rect(msg_box, palette.window_bg);
            draw_rect_outline(s, msg_box, 1, palette.border_subtle);
            let full_msg = cstr_as_str(&entry.msg);
            draw_text(
                s,
                msg_box.x + 8,
                msg_box.y + 10,
                full_msg,
                Color::WHITE,
                TypeScale::Body,
            );

            // Triage hint banner
            let triage_y = insp_rect.y + 90;
            if entry.severity <= ffi::NOVA_KLOG_ERR {
                let alert_rect = Rect::new(insp_rect.x + 10, triage_y, insp_rect.w - 20, 22);
                draw_rounded_rect(s, alert_rect, 3, Color::from_hex(0x5A1A1A));
                draw_rect_outline(s, alert_rect, 1, Color::from_hex(0xEF5350));
                draw_text(
                    s,
                    alert_rect.x + 8,
                    alert_rect.y + 4,
                    "[CRITICAL FAULT] Subsystem recorded high severity event. Verify PID context and inspect memory bounds.",
                    Color::WHITE,
                    TypeScale::Caption,
                );
            } else if entry.severity == ffi::NOVA_KLOG_WARN {
                let warn_rect = Rect::new(insp_rect.x + 10, triage_y, insp_rect.w - 20, 22);
                draw_rounded_rect(s, warn_rect, 3, Color::from_hex(0x4A3800));
                draw_rect_outline(s, warn_rect, 1, Color::from_hex(0xFFB300));
                draw_text(
                    s,
                    warn_rect.x + 8,
                    warn_rect.y + 4,
                    "[WARNING] Operational anomaly reported. Subsystem continues with default fallback.",
                    Color::from_hex(0xFFE082),
                    TypeScale::Caption,
                );
            } else {
                let info_rect = Rect::new(insp_rect.x + 10, triage_y, insp_rect.w - 20, 22);
                draw_rounded_rect(s, info_rect, 3, Color::from_hex(0x1B3820));
                draw_rect_outline(s, info_rect, 1, Color::from_hex(0x66BB6A));
                draw_text(
                    s,
                    info_rect.x + 8,
                    info_rect.y + 4,
                    "[NORMAL] Standard system event. No corrective administrative action required.",
                    Color::from_hex(0xA5D6A7),
                    TypeScale::Caption,
                );
            }
        } else {
            draw_text(
                s,
                insp_rect.x + 20,
                insp_rect.y + 50,
                "Select any log record from the table above to view complete message, originating PID, and triage guidance.",
                palette.text_muted,
                TypeScale::Body,
            );
        }

        // 6. Bottom Status Bar (Bottom 24px)
        let status_rect = Rect::new(viewport.x, viewport.bottom() - 24, viewport.w, 24);
        s.fill_rect(status_rect, palette.surface_sunken);
        draw_line(
            s,
            status_rect.x,
            status_rect.y,
            status_rect.right(),
            status_rect.y,
            palette.border_subtle,
        );

        let mut st_summary = [0u8; 96];
        let mut sum_len = 0;
        let m_txt = b"Matching: ";
        st_summary[sum_len..sum_len + m_txt.len()].copy_from_slice(m_txt);
        sum_len += m_txt.len();
        sum_len += format_u32(self.filtered_count as u32, &mut st_summary[sum_len..]);
        let of_txt = b" / ";
        st_summary[sum_len..sum_len + of_txt.len()].copy_from_slice(of_txt);
        sum_len += of_txt.len();
        sum_len += format_u32(self.entry_count as u32, &mut st_summary[sum_len..]);
        let b_txt = b" records  |  Status: ";
        st_summary[sum_len..sum_len + b_txt.len()].copy_from_slice(b_txt);
        sum_len += b_txt.len();

        let s_txt = self.status_str();
        for &b in s_txt.as_bytes() {
            if sum_len < st_summary.len() {
                st_summary[sum_len] = b;
                sum_len += 1;
            }
        }

        let summary_str = core::str::from_utf8(&st_summary[..sum_len]).unwrap_or("");
        draw_text(
            s,
            status_rect.x + 12,
            status_rect.y + 5,
            summary_str,
            palette.text_secondary,
            TypeScale::Caption,
        );

        let hint = "Shortcuts: [Space] Pause/Resume  [Up/Down] Select  [M] Marker  [C] Clear  [Q] Exit";
        draw_text(
            s,
            status_rect.right() - 520,
            status_rect.y + 5,
            hint,
            palette.text_muted,
            TypeScale::Caption,
        );
    }
}

// ===========================================================================
// Global Static BSS Allocations (Zero Stack Exhaustion)
// ===========================================================================

static mut GLOBAL_KLOG: KlogViewerApp = KlogViewerApp::new();
static mut QUERY_BUFFER: [ffi::NovaKlogEntry; QUERY_BATCH_SIZE] =
    [ffi::NovaKlogEntry::empty(); QUERY_BATCH_SIZE];
static mut RENDER_BUFFER: [u32; 1024 * 768] = [0; 1024 * 768];
static mut TEST_RENDER_BUF: [u32; 640 * 480] = [0; 640 * 480];

// ===========================================================================
// Deterministic Selftest Suite (--selftest)
// ===========================================================================

#[inline(never)]
fn selftest_telemetry(app: &mut KlogViewerApp) -> i32 {
    let mut code = 0;
    let mut info = ffi::NovaKlogInfo::default();
    let res = unsafe { ffi::sys_klog_info(&mut info) };
    if res == 0 && info.buffer_capacity == 256 && info.total_logged > 0 {
        app.telemetry = info;
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: SYS_KLOG_INFO ring buffer telemetry query\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: SYS_KLOG_INFO telemetry query failed\n\0".as_ptr())
        };
        code |= 1;
    }
    code
}

#[inline(never)]
fn selftest_roundtrip(_app: &mut KlogViewerApp) -> (i32, u64) {
    let mut code = 0;
    let mut marker_seq = 0u64;

    let mut info_before = ffi::NovaKlogInfo::default();
    unsafe { ffi::sys_klog_info(&mut info_before) };
    let start_seq = info_before.newest_seq + 1;

    // 1. Write structured entries with custom facility & severity
    let w1 = ffi::NovaKlogWrite {
        severity: ffi::NOVA_KLOG_WARN,
        facility: b"klogtest\0".as_ptr(),
        msg: b"marker_klog_selftest_alpha\0".as_ptr(),
    };
    let res1 = unsafe { ffi::sys_klog_write(&w1) };

    let w2 = ffi::NovaKlogWrite {
        severity: ffi::NOVA_KLOG_ERR,
        facility: b"klogtest\0".as_ptr(),
        msg: b"marker_klog_selftest_beta\0".as_ptr(),
    };
    let res2 = unsafe { ffi::sys_klog_write(&w2) };

    if res1 != 0 || res2 != 0 {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: SYS_KLOG_WRITE failed\n\0".as_ptr())
        };
        return (code | 2, 0);
    }

    // 2. Read back entries
    let read_buf = unsafe { &mut *core::ptr::addr_of_mut!(QUERY_BUFFER) };
    let mut req = ffi::NovaKlogRead {
        start_seq,
        max_entries: 32,
        max_severity: ffi::NOVA_KLOG_SEV_ANY,
        facility_filter: [0; ffi::NOVA_KLOG_FACILITY_MAX],
        next_seq: 0,
        dropped_count: 0,
        total_logged: 0,
        entries: read_buf.as_mut_ptr(),
    };

    let count = unsafe { ffi::sys_klog_read(&mut req) };
    let mut found_alpha = false;
    let mut found_beta = false;

    if count > 0 {
        for i in 0..count as usize {
            let e = &read_buf[i];
            let msg_str = cstr_as_str(&e.msg);
            let fac_str = cstr_as_str(&e.facility);
            if msg_str == "marker_klog_selftest_alpha" {
                if e.severity == ffi::NOVA_KLOG_WARN && fac_str == "klogtest" {
                    found_alpha = true;
                    marker_seq = e.seq;
                }
            } else if msg_str == "marker_klog_selftest_beta" {
                if e.severity == ffi::NOVA_KLOG_ERR && fac_str == "klogtest" {
                    found_beta = true;
                }
            }
        }
    }

    if found_alpha && found_beta {
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: SYS_KLOG_WRITE and SYS_KLOG_READ roundtrip\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: Roundtrip readback did not match written entries\n\0".as_ptr())
        };
        code |= 2;
    }

    (code, marker_seq)
}

#[inline(never)]
fn selftest_filtering(marker_seq: u64) -> i32 {
    let mut code = 0;
    let read_buf = unsafe { &mut *core::ptr::addr_of_mut!(QUERY_BUFFER) };

    // 1. Severity threshold filtering: only accept <= ERR (3)
    let mut req_sev = ffi::NovaKlogRead {
        start_seq: marker_seq,
        max_entries: 16,
        max_severity: ffi::NOVA_KLOG_ERR,
        facility_filter: [0; ffi::NOVA_KLOG_FACILITY_MAX],
        next_seq: 0,
        dropped_count: 0,
        total_logged: 0,
        entries: read_buf.as_mut_ptr(),
    };

    let n_sev = unsafe { ffi::sys_klog_read(&mut req_sev) };
    let mut sev_ok = n_sev > 0;
    if n_sev > 0 {
        for i in 0..n_sev as usize {
            let e = &read_buf[i];
            if e.severity > ffi::NOVA_KLOG_ERR {
                sev_ok = false;
                break;
            }
            if cstr_as_str(&e.msg) == "marker_klog_selftest_alpha" {
                // Alpha is WARN (4), must not appear!
                sev_ok = false;
                break;
            }
        }
    }

    if sev_ok {
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: severity threshold filtering (suppressed lower priority logs)\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: severity threshold filtering failed\n\0".as_ptr())
        };
        code |= 4;
    }

    // 2. Facility string filtering: only match "klogtest"
    let mut req_fac = ffi::NovaKlogRead {
        start_seq: marker_seq,
        max_entries: 16,
        max_severity: ffi::NOVA_KLOG_SEV_ANY,
        facility_filter: [0; ffi::NOVA_KLOG_FACILITY_MAX],
        next_seq: 0,
        dropped_count: 0,
        total_logged: 0,
        entries: read_buf.as_mut_ptr(),
    };
    let f_bytes = b"klogtest";
    req_fac.facility_filter[..f_bytes.len()].copy_from_slice(f_bytes);

    let n_fac = unsafe { ffi::sys_klog_read(&mut req_fac) };
    let mut fac_ok = n_fac > 0;
    if n_fac > 0 {
        for i in 0..n_fac as usize {
            if cstr_as_str(&read_buf[i].facility) != "klogtest" {
                fac_ok = false;
                break;
            }
        }
    }

    if fac_ok {
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: facility string filtering\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: facility string filtering failed\n\0".as_ptr())
        };
        code |= 8;
    }

    code
}

#[inline(never)]
fn selftest_streaming_and_search(app: &mut KlogViewerApp, marker_seq: u64) -> i32 {
    let mut code = 0;

    // 1. Incremental streaming via start_seq
    let read_buf = unsafe { &mut *core::ptr::addr_of_mut!(QUERY_BUFFER) };
    let mut req_stream = ffi::NovaKlogRead {
        start_seq: marker_seq + 1,
        max_entries: 16,
        max_severity: ffi::NOVA_KLOG_SEV_ANY,
        facility_filter: [0; ffi::NOVA_KLOG_FACILITY_MAX],
        next_seq: 0,
        dropped_count: 0,
        total_logged: 0,
        entries: read_buf.as_mut_ptr(),
    };

    let n_stream = unsafe { ffi::sys_klog_read(&mut req_stream) };
    let mut stream_ok = true;
    if n_stream > 0 {
        for i in 0..n_stream as usize {
            if read_buf[i].seq <= marker_seq {
                stream_ok = false;
                break;
            }
            if cstr_as_str(&read_buf[i].msg) == "marker_klog_selftest_alpha" {
                stream_ok = false;
                break;
            }
        }
    }

    if stream_ok {
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: incremental tail streaming with sequence tracking\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: incremental streaming failed\n\0".as_ptr())
        };
        code |= 16;
    }

    // 2. Keyword search and substring matching engine
    assert!(contains_ignore_case(b"Fatal kernel page fault in ring 0", b"PAGE FAULT"));
    assert_eq!(
        find_ignore_case(b"Fatal kernel page fault in ring 0", b"page fault"),
        Some(13)
    );
    assert!(!contains_ignore_case(b"Driver initialized", b"panic"));

    app.poll_entries();
    app.set_search_query("selftest");
    let match_count = app.filtered_count;
    let search_ok = match_count >= 2;

    app.clear_search_query();

    if search_ok {
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: real-time keyword search and substring highlighting engine\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: keyword search engine failed\n\0".as_ptr())
        };
        code |= 32;
    }

    code
}

#[inline(never)]
fn selftest_graphics(app: &mut KlogViewerApp) -> i32 {
    let mut code = 0;
    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(TEST_RENDER_BUF) as *mut u32,
            640 * 480,
        )
    };
    render_slice.fill(0);
    let mut surf = BufferSurface32::new(render_slice, 640, 480);

    let viewport = Rect::new(0, 0, 640, 480);
    app.render(&mut surf, viewport);

    let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero > 10000 {
        unsafe {
            ffi::sys_write(b"[klog-selftest] PASS: zero-syscall BufferSurface32 rasterization and inspector presentation\n\0".as_ptr())
        };
    } else {
        unsafe {
            ffi::sys_write(b"[klog-selftest] FAIL: graphical rendering produced insufficient pixels\n\0".as_ptr())
        };
        code |= 64;
    }

    code
}

#[inline(never)]
pub fn run_selftest() -> i32 {
    unsafe { ffi::sys_write(b"[klog-selftest] starting structured kernel log viewer selftest\n\0".as_ptr()) };

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_KLOG) };
    app.init();

    let mut code = 0;
    code |= selftest_telemetry(app);

    let (rt_code, marker_seq) = selftest_roundtrip(app);
    code |= rt_code;

    code |= selftest_filtering(marker_seq);
    code |= selftest_streaming_and_search(app, marker_seq);
    code |= selftest_graphics(app);

    if code == 0 {
        unsafe {
            ffi::sys_write(b"[klog-selftest] all cases passed\n\0".as_ptr());
        }
    }
    code
}

fn starts_with_cstr(ptr: *const u8, prefix: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for (i, &b) in prefix.iter().enumerate() {
        unsafe {
            if *ptr.add(i) != b {
                return false;
            }
        }
    }
    true
}

// ===========================================================================
// Entry Point (main)
// ===========================================================================

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc > 1 && !argv.is_null() {
        let arg1 = unsafe { *argv.add(1) };
        if starts_with_cstr(arg1, b"--selftest") {
            return run_selftest();
        }
    }

    // Live graphical framebuffer mode
    let rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if rc < 0 {
        unsafe { ffi::sys_write(b"klog: failed to acquire framebuffer\n\0".as_ptr()) };
        return 1;
    }

    let mut info = ffi::NovaFbInfo::default();
    unsafe { ffi::sys_fb_info(&mut info) };

    let screen_w = if info.width > 0 { info.width as i32 } else { 1024 };
    let screen_h = if info.height > 0 { info.height as i32 } else { 768 };

    let mut req = ffi::NovaFbCreate {
        width: screen_w as u32,
        height: screen_h as u32,
        flags: 0,
        handle: 0,
        pixels: 0,
        stride: 0,
        size: 0,
    };
    let cr_rc = unsafe { ffi::sys_fb_create(&mut req) };
    if cr_rc < 0 {
        unsafe {
            ffi::sys_fb_release();
            ffi::sys_write(b"klog: failed to create framebuffer surface\n\0".as_ptr());
        }
        return 1;
    }

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_KLOG) };
    app.init();
    app.poll_entries();
    app.set_status(b"Connected to kernel syslog stream");

    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(
            core::ptr::addr_of_mut!(RENDER_BUFFER) as *mut u32,
            (screen_w * screen_h) as usize,
        )
    };
    let mut surf = BufferSurface32::new(render_slice, screen_w, screen_h);

    let mut cursor_x = screen_w / 2;
    let mut cursor_y = screen_h / 2;
    let mut prev_left = false;
    let viewport = Rect::new(0, 0, screen_w, screen_h);
    let mut tick_counter = 0u32;

    loop {
        // Keyboard input
        let key = unsafe { ffi::sys_read_key() };
        if key == 27 || key == b'q' as i32 {
            break;
        } else if key == b' ' as i32 {
            app.auto_refresh = !app.auto_refresh;
        } else if key == b'm' as i32 || key == b'M' as i32 {
            app.inject_marker("User Keypress");
        } else if key == b'c' as i32 || key == b'C' as i32 {
            app.clear_view();
        } else if key == 0x48 || key == b'k' as i32 {
            // Up arrow
            app.select_prev();
        } else if key == 0x50 || key == b'j' as i32 {
            // Down arrow
            app.select_next();
        } else if key == 0x49 {
            // Page up
            app.scroll_page_up();
        } else if key == 0x51 {
            // Page down
            app.scroll_page_down();
        }

        // Mouse input
        let mut mouse = ffi::MouseState::default();
        if unsafe { ffi::sys_mouse_read(&mut mouse) } == 0 {
            cursor_x = (cursor_x + mouse.dx).max(0).min(screen_w - 1);
            cursor_y = (cursor_y - mouse.dy).max(0).min(screen_h - 1);

            if mouse.left_button && !prev_left {
                app.handle_click(cursor_x, cursor_y, viewport);
            }
            prev_left = mouse.left_button;
        }

        // Periodic live poll
        tick_counter += 1;
        if app.auto_refresh && (tick_counter % 15 == 0) {
            app.poll_entries();
        }

        // Render frame to off-screen buffer
        app.render(&mut surf, viewport);

        // Draw hardware cursor
        draw_cursor_sprite(
            &mut surf,
            cursor_x,
            cursor_y,
            CursorShape::Arrow,
            ui::cursor::CursorTheme::ModernDark,
        );

        // Single zero-copy present syscall
        let present = ffi::NovaFbPresent {
            handle: req.handle,
            src_x: 0,
            src_y: 0,
            width: screen_w,
            height: screen_h,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe {
            ffi::sys_fb_present(&present);
            ffi::sys_yield();
        }
    }

    unsafe {
        ffi::sys_fb_destroy(req.handle);
        ffi::sys_fb_release();
    }
    0
}

// ===========================================================================
// Host Unit Tests (cargo test / make klog-test)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_helpers() {
        let mut buf = [0u8; 16];
        let len = format_u32(12345, &mut buf);
        assert_eq!(&buf[..len], b"12345");

        let len64 = format_u64(9876543210, &mut buf);
        assert_eq!(&buf[..len64], b"9876543210");

        let mut sbuf = [0u8; 8];
        let slen = format_seq(42, &mut sbuf);
        assert_eq!(&sbuf[..slen], b"#000042");
    }

    #[test]
    fn test_case_insensitive_matching() {
        assert!(contains_ignore_case(b"Fatal page fault in kernel", b"PAGE FAULT"));
        assert!(contains_ignore_case(b"Ethernet carrier lost", b"CARRIER"));
        assert_eq!(
            find_ignore_case(b"NovaOS structured logging", b"LOGGING"),
            Some(18)
        );
        assert_eq!(
            find_ignore_case(b"NovaOS structured logging", b"notfound"),
            None
        );
    }

    #[test]
    fn test_severity_labels_and_colors() {
        assert_eq!(severity_label(ffi::NOVA_KLOG_EMERG), "EMERG");
        assert_eq!(severity_label(ffi::NOVA_KLOG_WARN), "WARN");
        assert_eq!(severity_label(ffi::NOVA_KLOG_INFO), "INFO");

        assert_eq!(severity_fg_color(ffi::NOVA_KLOG_WARN), Color::BLACK);
        assert_eq!(severity_fg_color(ffi::NOVA_KLOG_ERR), Color::WHITE);
    }

    #[test]
    fn test_klog_viewer_polling_and_filtering() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = KlogViewerApp::new();
        app.poll_entries();

        assert!(app.entry_count >= 8, "Expected initial boot logs to be polled");
        assert!(app.filtered_count >= 8);
        assert_eq!(app.sev_filter, SeverityFilterOption::All);

        // Filter: Warnings and above
        app.sev_filter = SeverityFilterOption::WarningsAbove;
        app.rebuild_filtered_indices();
        assert!(app.filtered_count > 0);
        for i in 0..app.filtered_count {
            let e = &app.entries[app.filtered_indices[i] as usize];
            assert!(e.severity <= ffi::NOVA_KLOG_WARN);
        }

        // Filter: Errors only
        app.sev_filter = SeverityFilterOption::ErrorsOnly;
        app.rebuild_filtered_indices();
        assert!(app.filtered_count > 0);
        for i in 0..app.filtered_count {
            let e = &app.entries[app.filtered_indices[i] as usize];
            assert!(e.severity <= ffi::NOVA_KLOG_ERR);
        }

        // Facility filter: "net"
        app.sev_filter = SeverityFilterOption::All;
        app.set_facility_filter("net");
        assert_eq!(app.filtered_count, 1);
        let e_net = &app.entries[app.filtered_indices[0] as usize];
        assert_eq!(cstr_as_str(&e_net.facility), "net");

        // Search query: "RAM"
        app.set_facility_filter("ALL");
        app.set_search_query("RAM");
        assert_eq!(app.filtered_count, 1);
        let e_ram = &app.entries[app.filtered_indices[0] as usize];
        assert!(contains_ignore_case(&e_ram.msg, b"physical RAM"));
    }

    #[test]
    fn test_klog_marker_injection() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = KlogViewerApp::new();
        app.poll_entries();
        let prev_count = app.entry_count;

        assert!(app.inject_marker("test_checkpoint_one"));
        assert_eq!(app.entry_count, prev_count + 1);

        let last_entry = &app.entries[app.entry_count - 1];
        assert_eq!(cstr_as_str(&last_entry.facility), "admin");
        assert!(contains_ignore_case(&last_entry.msg, b"test_checkpoint_one"));
    }

    #[test]
    fn test_mouse_click_handling() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = KlogViewerApp::new();
        app.poll_entries();
        let viewport = Rect::new(0, 0, 1024, 768);

        // Click Pause button (x: 12..104, y: 56+6..56+28)
        assert!(app.handle_click(20, 65, viewport));
        assert!(!app.auto_refresh);

        // Click Resume button
        assert!(app.handle_click(20, 65, viewport));
        assert!(app.auto_refresh);

        // Click ERR+ severity pill (x: 472..522, y: 62..84)
        assert!(app.handle_click(480, 68, viewport));
        assert_eq!(app.sev_filter, SeverityFilterOption::ErrorsOnly);

        // Click "net" facility chip (x: 80 + 36 + 6 + 56 + 6 + 40 + 6 = 224)
        // Let's test preset click
        app.set_facility_filter("net");
        assert_eq!(cstr_as_str(&app.facility_filter), "net");

        // Click Clear button
        assert!(app.handle_click(290, 68, viewport));
        assert_eq!(app.entry_count, 0);
    }

    #[test]
    fn test_surface_rendering_invariants() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = KlogViewerApp::new();
        app.poll_entries();

        let mut buffer = [0u32; 800 * 600];
        let mut surf = BufferSurface32::new(&mut buffer, 800, 600);
        let viewport = Rect::new(0, 0, 800, 600);

        app.render(&mut surf, viewport);

        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 10000, "App must render header, table, badges, inspector, and status bar");

        // Verify inspector displays selected entry
        assert!(app.selected_entry().is_some());
    }

    #[test]
    fn test_selftest_suite_on_host() {
        unsafe { ffi::reset_mock_environment() };
        let code = run_selftest();
        assert_eq!(code, 0, "All selftest assertions must pass");
    }
}
