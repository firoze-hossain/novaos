//! userland/wm-rs/wm.rs - NovaOS Framebuffer Window Manager (Month 2 Compositor)
//!
//! Ports the ring-3 interactive window manager off legacy Mode 13h VGA text/VGA
//! mode onto the unified 32-bit linear framebuffer architecture (SYS_FB_*).
//!
//! Features:
//!   - Real 32-bit XRGB framebuffer rendering using zero-allocation Surface trait
//!     (ui::draw::BufferSurface32) and the NovaOS Design System (ui-rs).
//!   - Exclusive display ownership handshake via SYS_FB_ACQUIRE / SYS_FB_RELEASE.
//!   - Damage-aware frame presentation via SYS_FB_PRESENT with single-syscall-per-frame blits.
//!   - Full window management preservation: multiple overlapping windows, titlebar dragging,
//!     click-to-focus (body and titlebar), taskbar focus switching, and left/right edge snapping
//!     with restoration of pre-snap floating geometry.
//!   - Scalable to any native display resolution (e.g. 1024x768 VESA/VBE and virtio-gpu) while
//!     preserving backwards-compatible fallback geometry.

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
        ffi::sys_fb_release(); // Best-effort: restore text console if panicking
        let msg = b"wm: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

pub const DEFAULT_SCREEN_W: i32 = 320;
pub const DEFAULT_SCREEN_H: i32 = 200;
pub const DEFAULT_TITLEBAR_H: i32 = 10;
pub const DEFAULT_TASKBAR_H: i32 = 12;
pub const DEFAULT_DESKTOP_H: i32 = DEFAULT_SCREEN_H - DEFAULT_TASKBAR_H;

pub const FB_SCREEN_W: i32 = 1024;
pub const FB_SCREEN_H: i32 = 768;
pub const FB_TITLEBAR_H: i32 = TITLEBAR_HEIGHT; // 28px
pub const FB_TASKBAR_H: i32 = TASKBAR_HEIGHT;   // 36px
pub const FB_MENUBAR_H: i32 = MENUBAR_HEIGHT;   // 24px
pub const FB_DESKTOP_H: i32 = FB_SCREEN_H - FB_TASKBAR_H; // 732px

pub const MAX_WINDOWS: usize = 4;
pub const MAX_DOCK_ITEMS: usize = 8;
pub const SNAP_MARGIN: i32 = 4;

/// Target docking geometry for snapped windows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SnapState {
    Normal,
    Left,
    Right,
    Maximize,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl SnapState {
    #[inline]
    pub fn is_snapped(&self) -> bool {
        *self != SnapState::Normal
    }

    #[inline]
    pub fn is_half(&self) -> bool {
        matches!(self, SnapState::Left | SnapState::Right)
    }

    #[inline]
    pub fn is_quadrant(&self) -> bool {
        matches!(
            self,
            SnapState::TopLeft
                | SnapState::TopRight
                | SnapState::BottomLeft
                | SnapState::BottomRight
        )
    }

    #[inline]
    pub fn is_maximized(&self) -> bool {
        *self == SnapState::Maximize
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SnapState::Normal => "Floating",
            SnapState::Left => "Left Half",
            SnapState::Right => "Right Half",
            SnapState::Maximize => "Maximized",
            SnapState::TopLeft => "Top Left",
            SnapState::TopRight => "Top Right",
            SnapState::BottomLeft => "Bottom Left",
            SnapState::BottomRight => "Bottom Right",
        }
    }
}

/// Usable desktop workspace area taking menubars and taskbars into account.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl WorkArea {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        WorkArea { x, y, w, h }
    }

    pub fn to_rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.w, self.h)
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// Physical or virtual monitor boundary descriptor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MonitorGeometry {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub is_primary: bool,
}

/// Multi-window productivity tiling modes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileMode {
    SideBySide,
    Grid2x2,
    Cascade,
}

/// Computes the exact window bounding rectangle for a snap target within a work area,
/// ensuring smoother edges, zero overlap, and gapless quadrant/half tiling.
pub fn calculate_snap_rect(snap: SnapState, work: WorkArea) -> (i32, i32, i32, i32) {
    match snap {
        SnapState::Normal => (work.x, work.y, work.w, work.h),
        SnapState::Maximize => (work.x, work.y, work.w, work.h),
        SnapState::Left => {
            let half_w = work.w / 2;
            (work.x, work.y, half_w, work.h)
        }
        SnapState::Right => {
            let half_w = work.w / 2;
            (work.x + half_w, work.y, work.w - half_w, work.h)
        }
        SnapState::TopLeft => {
            let half_w = work.w / 2;
            let half_h = work.h / 2;
            (work.x, work.y, half_w, half_h)
        }
        SnapState::TopRight => {
            let half_w = work.w / 2;
            let half_h = work.h / 2;
            (work.x + half_w, work.y, work.w - half_w, half_h)
        }
        SnapState::BottomLeft => {
            let half_w = work.w / 2;
            let half_h = work.h / 2;
            (work.x, work.y + half_h, half_w, work.h - half_h)
        }
        SnapState::BottomRight => {
            let half_w = work.w / 2;
            let half_h = work.h / 2;
            (work.x + half_w, work.y + half_h, work.w - half_w, work.h - half_h)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Window {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub body_color: Color,
    pub titlebar_color: Color,
    pub label: u8,
    pub snap: SnapState,
    pub pre_snap: (i32, i32, i32, i32),
    pub is_open: bool,
    pub is_minimized: bool,
}

impl Window {
    pub const fn new(x: i32, y: i32, w: i32, h: i32, body: Color, title: Color, label: u8) -> Self {
        Window {
            x, y, w, h,
            body_color: body,
            titlebar_color: title,
            label,
            snap: SnapState::Normal,
            pre_snap: (x, y, w, h),
            is_open: true,
            is_minimized: false,
        }
    }

    pub fn from_vga(x: i32, y: i32, w: i32, h: i32, body_vga: u8, title_vga: u8, label: u8) -> Self {
        Window::new(
            x, y, w, h,
            Color::from_vga_index(body_vga),
            Color::from_vga_index(title_vga),
            label,
        )
    }
}

fn point_in_rect(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DesktopIconSpec {
    pub label: &'static str,
    pub sublabel: &'static str,
    pub app_id: u8,
    pub icon_type: u8,
}

pub const DESKTOP_SHORTCUTS: [DesktopIconSpec; 12] = [
    DesktopIconSpec { label: "This PC",     sublabel: "NovaOS 2.0",    app_id: 1, icon_type: 0 },
    DesktopIconSpec { label: "Files",       sublabel: "/root",          app_id: 2, icon_type: 2 },
    DesktopIconSpec { label: "Terminal",    sublabel: "NovaTerm",       app_id: 1, icon_type: 1 },
    DesktopIconSpec { label: "Calendar",    sublabel: "Events & Tasks", app_id: 6, icon_type: 6 },
    DesktopIconSpec { label: "Calculator",  sublabel: "Math Studio",    app_id: 7, icon_type: 7 },
    DesktopIconSpec { label: "Aero Web",    sublabel: "Browser",        app_id: 8, icon_type: 8 },
    DesktopIconSpec { label: "Spectra",     sublabel: "Media Studio",   app_id: 9, icon_type: 9 },
    DesktopIconSpec { label: "Vellum",      sublabel: "PDF Studio",     app_id: 10, icon_type: 10 },
    DesktopIconSpec { label: "Office",      sublabel: "Zenith Suite",   app_id: 12, icon_type: 12 },
    DesktopIconSpec { label: "App Store",   sublabel: "Packages",       app_id: 4, icon_type: 4 },
    DesktopIconSpec { label: "Settings",    sublabel: "Control Panel",  app_id: 5, icon_type: 5 },
    DesktopIconSpec { label: "README.txt",  sublabel: "Document",       app_id: 3, icon_type: 3 },
];

// --- International Timezone & Wall-Clock Calendar Subsystem ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimezoneSpec {
    pub code: &'static str,
    pub name: &'static str,
    pub city_country: &'static str,
    pub offset_minutes: i32,
}

pub const TIMEZONES: [TimezoneSpec; 22] = [
    TimezoneSpec { code: "BST", name: "Asia/Dhaka", city_country: "Dhaka, Bangladesh", offset_minutes: 360 }, // UTC+6:00 (Default)
    TimezoneSpec { code: "IST", name: "Asia/Kolkata", city_country: "New Delhi, India", offset_minutes: 330 }, // UTC+5:30
    TimezoneSpec { code: "GST", name: "Asia/Dubai", city_country: "Dubai, UAE", offset_minutes: 240 }, // UTC+4:00
    TimezoneSpec { code: "PKT", name: "Asia/Karachi", city_country: "Karachi, Pakistan", offset_minutes: 300 }, // UTC+5:00
    TimezoneSpec { code: "SGT", name: "Asia/Singapore", city_country: "Singapore", offset_minutes: 480 }, // UTC+8:00
    TimezoneSpec { code: "JST", name: "Asia/Tokyo", city_country: "Tokyo, Japan", offset_minutes: 540 }, // UTC+9:00
    TimezoneSpec { code: "AEST", name: "Australia/Sydney", city_country: "Sydney, Australia", offset_minutes: 600 }, // UTC+10:00
    TimezoneSpec { code: "NZST", name: "Pacific/Auckland", city_country: "Auckland, New Zealand", offset_minutes: 720 }, // UTC+12:00
    TimezoneSpec { code: "UTC", name: "UTC", city_country: "Universal Coordinated Time", offset_minutes: 0 }, // UTC+0:00
    TimezoneSpec { code: "GMT", name: "Europe/London", city_country: "London, UK", offset_minutes: 0 }, // UTC+0:00
    TimezoneSpec { code: "CET", name: "Europe/Paris", city_country: "Paris, France", offset_minutes: 60 }, // UTC+1:00
    TimezoneSpec { code: "CET", name: "Europe/Berlin", city_country: "Berlin, Germany", offset_minutes: 60 }, // UTC+1:00
    TimezoneSpec { code: "MSK", name: "Europe/Moscow", city_country: "Moscow, Russia", offset_minutes: 180 }, // UTC+3:00
    TimezoneSpec { code: "AST", name: "Asia/Riyadh", city_country: "Riyadh, Saudi Arabia", offset_minutes: 180 }, // UTC+3:00
    TimezoneSpec { code: "EET", name: "Africa/Cairo", city_country: "Cairo, Egypt", offset_minutes: 120 }, // UTC+2:00
    TimezoneSpec { code: "SAST", name: "Africa/Johannesburg", city_country: "Johannesburg, S. Africa", offset_minutes: 120 }, // UTC+2:00
    TimezoneSpec { code: "BRT", name: "America/Sao_Paulo", city_country: "Sao Paulo, Brazil", offset_minutes: -180 }, // UTC-3:00
    TimezoneSpec { code: "EST", name: "America/New_York", city_country: "New York, USA", offset_minutes: -300 }, // UTC-5:00
    TimezoneSpec { code: "CST", name: "America/Chicago", city_country: "Chicago, USA", offset_minutes: -360 }, // UTC-6:00
    TimezoneSpec { code: "MST", name: "America/Denver", city_country: "Denver, USA", offset_minutes: -420 }, // UTC-7:00
    TimezoneSpec { code: "PST", name: "America/Los_Angeles", city_country: "Los Angeles, USA", offset_minutes: -480 }, // UTC-8:00
    TimezoneSpec { code: "HST", name: "Pacific/Honolulu", city_country: "Honolulu, Hawaii", offset_minutes: -600 }, // UTC-10:00
];

pub fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

pub fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 => 31,
        2 => if is_leap_year(year) { 29 } else { 28 },
        3 => 31,
        4 => 30,
        5 => 31,
        6 => 30,
        7 => 31,
        8 => 31,
        9 => 30,
        10 => 31,
        11 => 30,
        12 => 31,
        _ => 30,
    }
}

pub fn day_of_week(year: u16, month: u8, day: u8) -> u8 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year as i32;
    if month < 3 {
        y -= 1;
    }
    let m = (month.saturating_sub(1) as usize).min(11);
    let dow = (y + y / 4 - y / 100 + y / 400 + t[m] + day as i32) % 7;
    if dow < 0 {
        ((dow + 7) % 7) as u8
    } else {
        dow as u8
    }
}

pub const MONTH_NAMES_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"
];

pub const MONTH_NAMES_FULL: [&str; 12] = [
    "January", "February", "March", "April", "May", "June",
    "July", "August", "September", "October", "November", "December"
];

pub const DAY_NAMES_SHORT: [&str; 7] = [
    "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"
];

pub const DAY_NAMES_FULL: [&str; 7] = [
    "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub day_of_week: u8,
}

pub fn calculate_datetime(base: ffi::RtcTime, offset_minutes: i32, offset_days: i32) -> DateTime {
    let base_year = if base.year >= 2020 && base.year <= 2099 { base.year } else { 2026 };
    let base_month = if base.month >= 1 && base.month <= 12 { base.month } else { 11 };
    let base_day = if base.day >= 1 && base.day <= 31 { base.day } else { 19 };
    let base_hour = if base.hour <= 23 { base.hour } else { 14 };
    let base_min = if base.minute <= 59 { base.minute } else { 45 };
    let base_sec = if base.second <= 59 { base.second } else { 0 };

    let mut total_mins = (base_hour as i32) * 60 + (base_min as i32) + offset_minutes;
    let mut day_delta = offset_days;

    while total_mins < 0 {
        total_mins += 1440;
        day_delta -= 1;
    }
    while total_mins >= 1440 {
        total_mins -= 1440;
        day_delta += 1;
    }

    let final_hour = (total_mins / 60) as u8;
    let final_min = (total_mins % 60) as u8;

    let mut cur_y = base_year;
    let mut cur_m = base_month;
    let mut cur_d = (base_day as i32) + day_delta;

    while cur_d > days_in_month(cur_y, cur_m) as i32 {
        cur_d -= days_in_month(cur_y, cur_m) as i32;
        cur_m += 1;
        if cur_m > 12 {
            cur_m = 1;
            cur_y = cur_y.saturating_add(1);
        }
    }
    while cur_d < 1 {
        cur_m = if cur_m == 1 { 12 } else { cur_m - 1 };
        if cur_m == 12 {
            cur_y = cur_y.saturating_sub(1);
        }
        cur_d += days_in_month(cur_y, cur_m) as i32;
    }

    let final_day = (cur_d as u8).max(1);
    let dow = day_of_week(cur_y, cur_m, final_day);

    DateTime {
        year: cur_y,
        month: cur_m,
        day: final_day,
        hour: final_hour,
        minute: final_min,
        second: base_sec,
        day_of_week: dow,
    }
}

pub fn format_u32(val: u32, buf: &mut [u8], offset: &mut usize) {
    if val == 0 {
        if *offset < buf.len() {
            buf[*offset] = b'0';
            *offset += 1;
        }
        return;
    }
    let mut temp = [b'0'; 10];
    let mut v = val;
    let mut digits = 0;
    while v > 0 && digits < 10 {
        temp[9 - digits] = b'0' + (v % 10) as u8;
        v /= 10;
        digits += 1;
    }
    let start = 10 - digits;
    for i in start..10 {
        if *offset < buf.len() {
            buf[*offset] = temp[i];
            *offset += 1;
        }
    }
}

pub fn format_u32_padded(val: u32, width: usize, buf: &mut [u8], offset: &mut usize) {
    let mut temp = [b'0'; 10];
    let mut v = val;
    let mut digits = 0;
    if v == 0 {
        digits = 1;
    } else {
        while v > 0 && digits < 10 {
            temp[9 - digits] = b'0' + (v % 10) as u8;
            v /= 10;
            digits += 1;
        }
    }
    let pad = if width > digits { width - digits } else { 0 };
    for _ in 0..pad {
        if *offset < buf.len() {
            buf[*offset] = b'0';
            *offset += 1;
        }
    }
    let start = 10 - digits;
    for i in start..10 {
        if *offset < buf.len() {
            buf[*offset] = temp[i];
            *offset += 1;
        }
    }
}

pub fn append_str(s: &str, buf: &mut [u8], offset: &mut usize) {
    let b = s.as_bytes();
    for &ch in b {
        if *offset < buf.len() {
            buf[*offset] = ch;
            *offset += 1;
        }
    }
}

pub fn fast_sqrt64(val: f64) -> f64 {
    if val <= 0.0 {
        return 0.0;
    }
    let mut x = val;
    let mut i = 0;
    while i < 10 {
        x = 0.5 * (x + val / x);
        i += 1;
    }
    x
}

pub fn format_f64_into(val: f64, buf: &mut [u8], offset: &mut usize) {
    if val.is_nan() {
        append_str("NaN", buf, offset);
        return;
    }
    if val < 0.0 {
        append_str("-", buf, offset);
    }
    let abs_val = if val < 0.0 { -val } else { val };
    let int_part = abs_val as u64;
    format_u32(int_part as u32, buf, offset);
    let frac = abs_val - (int_part as f64);
    if frac > 1e-5 && *offset < buf.len() {
        append_str(".", buf, offset);
        let frac_int = (frac * 10000.0 + 0.5) as u32;
        let mut digits = [0u8; 4];
        let mut d_off = 0;
        format_u32_padded(frac_int, 4, &mut digits, &mut d_off);
        while d_off > 1 && digits[d_off - 1] == b'0' {
            d_off -= 1;
        }
        for i in 0..d_off {
            if *offset < buf.len() {
                buf[*offset] = digits[i];
                *offset += 1;
            }
        }
    }
}

pub fn parse_f64_from(slice: &[u8]) -> f64 {
    let mut val = 0.0;
    let mut is_neg = false;
    let mut frac_div = 0.0;
    for &b in slice {
        if b == b'-' {
            is_neg = true;
        } else if b == b'.' {
            frac_div = 10.0;
        } else if b >= b'0' && b <= b'9' {
            let digit = (b - b'0') as f64;
            if frac_div > 0.0 {
                val += digit / frac_div;
                frac_div *= 10.0;
            } else {
                val = val * 10.0 + digit;
            }
        }
    }
    if is_neg { -val } else { val }
}

pub fn parse_i64_base(slice: &[u8], base: u32) -> i64 {
    let mut val: i64 = 0;
    for &b in slice {
        let d = match b {
            b'0'..=b'9' => (b - b'0') as i64,
            b'a'..=b'f' => (b - b'a' + 10) as i64,
            b'A'..=b'F' => (b - b'A' + 10) as i64,
            _ => continue,
        };
        if d < base as i64 {
            val = val.wrapping_mul(base as i64).wrapping_add(d);
        }
    }
    val
}

pub fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() {
        return true;
    }
    if h.len() < n.len() {
        return false;
    }
    for i in 0..=h.len() - n.len() {
        let mut matches = true;
        for j in 0..n.len() {
            if h[i + j].to_ascii_lowercase() != n[j].to_ascii_lowercase() {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }
    false
}

pub fn format_clock<'a>(
    dt: &DateTime,
    is_24h: bool,
    show_seconds: bool,
    buf: &'a mut [u8],
) -> &'a str {
    let mut off = 0;
    if is_24h {
        format_u32_padded(dt.hour as u32, 2, buf, &mut off);
        append_str(":", buf, &mut off);
        format_u32_padded(dt.minute as u32, 2, buf, &mut off);
        if show_seconds {
            append_str(":", buf, &mut off);
            format_u32_padded(dt.second as u32, 2, buf, &mut off);
        }
    } else {
        let (h12, ampm) = if dt.hour == 0 {
            (12, "AM")
        } else if dt.hour < 12 {
            (dt.hour, "AM")
        } else if dt.hour == 12 {
            (12, "PM")
        } else {
            (dt.hour - 12, "PM")
        };
        format_u32(h12 as u32, buf, &mut off);
        append_str(":", buf, &mut off);
        format_u32_padded(dt.minute as u32, 2, buf, &mut off);
        if show_seconds {
            append_str(":", buf, &mut off);
            format_u32_padded(dt.second as u32, 2, buf, &mut off);
        }
        append_str(" ", buf, &mut off);
        append_str(ampm, buf, &mut off);
    }
    core::str::from_utf8(&buf[..off]).unwrap_or("12:00 PM")
}

pub fn format_full_date<'a>(dt: &DateTime, buf: &'a mut [u8]) -> &'a str {
    let mut off = 0;
    let dow_name = DAY_NAMES_FULL[dt.day_of_week as usize % 7];
    let mon_name = MONTH_NAMES_FULL[dt.month.saturating_sub(1) as usize % 12];
    append_str(dow_name, buf, &mut off);
    append_str(", ", buf, &mut off);
    format_u32(dt.day as u32, buf, &mut off);
    append_str(" ", buf, &mut off);
    append_str(mon_name, buf, &mut off);
    append_str(" ", buf, &mut off);
    format_u32(dt.year as u32, buf, &mut off);
    core::str::from_utf8(&buf[..off]).unwrap_or("")
}

pub fn format_short_date<'a>(dt: &DateTime, buf: &'a mut [u8]) -> &'a str {
    let mut off = 0;
    let dow_name = DAY_NAMES_SHORT[dt.day_of_week as usize % 7];
    let mon_name = MONTH_NAMES_SHORT[dt.month.saturating_sub(1) as usize % 12];
    append_str(dow_name, buf, &mut off);
    append_str(" ", buf, &mut off);
    append_str(mon_name, buf, &mut off);
    append_str(" ", buf, &mut off);
    format_u32(dt.day as u32, buf, &mut off);
    core::str::from_utf8(&buf[..off]).unwrap_or("")
}

pub fn format_tz_summary<'a>(tz: &TimezoneSpec, buf: &'a mut [u8]) -> &'a str {
    let mut off = 0;
    append_str(tz.city_country, buf, &mut off);
    append_str(" (UTC", buf, &mut off);
    let sign = if tz.offset_minutes >= 0 { "+" } else { "-" };
    append_str(sign, buf, &mut off);
    let off_h = tz.offset_minutes.abs() / 60;
    let off_m = tz.offset_minutes.abs() % 60;
    format_u32(off_h as u32, buf, &mut off);
    if off_m != 0 {
        append_str(":", buf, &mut off);
        format_u32_padded(off_m as u32, 2, buf, &mut off);
    }
    append_str(")", buf, &mut off);
    core::str::from_utf8(&buf[..off]).unwrap_or("")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeConfig {
    pub timezone_idx: usize,
    pub is_24h: bool,
    pub show_seconds: bool,
    pub manual_offset_minutes: i32,
    pub manual_offset_days: i32,
}

impl Default for TimeConfig {
    fn default() -> Self {
        TimeConfig {
            timezone_idx: 0, // Asia/Dhaka (UTC+6) default
            is_24h: false,   // 12H AM/PM default
            show_seconds: false,
            manual_offset_minutes: 0,
            manual_offset_days: 0,
        }
    }
}

// --- NovaOS Desktop Calendar & Schedule Application Subsystem ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmViewMode {
    Month,
    Week,
    Day,
    Agenda,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmCategory {
    Work,
    Personal,
    NovaOS,
    Holidays,
    Reminders,
}

impl WmCategory {
    pub fn name(&self) -> &'static str {
        match self {
            WmCategory::Work => "Work",
            WmCategory::Personal => "Personal",
            WmCategory::NovaOS => "NovaOS",
            WmCategory::Holidays => "Holidays",
            WmCategory::Reminders => "Remind",
        }
    }

    pub fn color_rgb(&self) -> (u8, u8, u8) {
        match self {
            WmCategory::Work => (239, 68, 68),      // Coral Red
            WmCategory::Personal => (16, 185, 129),  // Emerald Green
            WmCategory::NovaOS => (139, 92, 246),    // Royal Purple
            WmCategory::Holidays => (245, 158, 11),  // Amber Gold
            WmCategory::Reminders => (14, 165, 233), // Sky Blue
        }
    }

    pub fn color(&self) -> Color {
        let (r, g, b) = self.color_rgb();
        Color::rgb(r, g, b)
    }

    pub fn bit_mask(&self) -> u8 {
        match self {
            WmCategory::Work => 1,
            WmCategory::Personal => 2,
            WmCategory::NovaOS => 4,
            WmCategory::Holidays => 8,
            WmCategory::Reminders => 16,
        }
    }
}

pub const WM_MAX_CALENDAR_EVENTS: usize = 16;

#[derive(Clone, Copy, Debug)]
pub struct WmCalendarEvent {
    pub id: u16,
    pub title: &'static str,
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub time_start: &'static str,
    pub time_end: &'static str,
    pub category: WmCategory,
    pub location: &'static str,
    pub note: &'static str,
}

pub struct WmCalendarApp {
    pub view_year: u16,
    pub view_month: u8,
    pub selected_day: u8,
    pub today_year: u16,
    pub today_month: u8,
    pub today_day: u8,
    pub view_mode: WmViewMode,
    pub filter_mask: u8,
    pub events: [WmCalendarEvent; WM_MAX_CALENDAR_EVENTS],
    pub event_count: usize,
    pub next_event_id: u16,
    pub status_msg: &'static str,
}

impl WmCalendarApp {
    pub fn new() -> Self {
        WmCalendarApp {
            view_year: 2026,
            view_month: 11,
            selected_day: 19,
            today_year: 2026,
            today_month: 11,
            today_day: 19,
            view_mode: WmViewMode::Month,
            filter_mask: 0x1F, // all 5 categories enabled
            events: [
                WmCalendarEvent {
                    id: 1,
                    title: "NovaOS 2.0 GA Release Launch",
                    year: 2026,
                    month: 11,
                    day: 19,
                    time_start: "02:00 PM",
                    time_end: "04:00 PM",
                    category: WmCategory::NovaOS,
                    location: "Main Auditorium",
                    note: "Official GA release signing and production launch.",
                },
                WmCalendarEvent {
                    id: 2,
                    title: "Daily Architecture Standup",
                    year: 2026,
                    month: 11,
                    day: 19,
                    time_start: "09:30 AM",
                    time_end: "10:00 AM",
                    category: WmCategory::Work,
                    location: "Matrix Channel",
                    note: "SMP kernel, Long Mode memory paging, ring-3 transition.",
                },
                WmCalendarEvent {
                    id: 3,
                    title: "Team Celebration Dinner",
                    year: 2026,
                    month: 11,
                    day: 19,
                    time_start: "07:30 PM",
                    time_end: "09:30 PM",
                    category: WmCategory::Personal,
                    location: "The Sapphire Bistro",
                    note: "Celebrating Month 6 hardening and release readiness.",
                },
                WmCalendarEvent {
                    id: 4,
                    title: "Backup Cloud Snapshot Sync",
                    year: 2026,
                    month: 11,
                    day: 19,
                    time_start: "11:00 PM",
                    time_end: "11:30 PM",
                    category: WmCategory::Reminders,
                    location: "Automated Host",
                    note: "Verify Btrfs subvolume replication across cluster.",
                },
                WmCalendarEvent {
                    id: 5,
                    title: "Security Architecture Review",
                    year: 2026,
                    month: 11,
                    day: 25,
                    time_start: "10:00 AM",
                    time_end: "12:00 PM",
                    category: WmCategory::Work,
                    location: "Conf Room B",
                    note: "Capability-based syscall authorization verification.",
                },
                WmCalendarEvent {
                    id: 6,
                    title: "Thanksgiving Holiday",
                    year: 2026,
                    month: 11,
                    day: 26,
                    time_start: "All Day",
                    time_end: "",
                    category: WmCategory::Holidays,
                    location: "Worldwide",
                    note: "Annual holiday observance.",
                },
                WmCalendarEvent {
                    id: 7,
                    title: "Sprint 7 Planning",
                    year: 2026,
                    month: 12,
                    day: 1,
                    time_start: "09:00 AM",
                    time_end: "10:30 AM",
                    category: WmCategory::Work,
                    location: "Virtual Room 1",
                    note: "Roadmap planning for post-v2.0 enterprise telemetry.",
                },
                WmCalendarEvent {
                    id: 8,
                    title: "Christmas Day Holiday",
                    year: 2026,
                    month: 12,
                    day: 25,
                    time_start: "All Day",
                    time_end: "",
                    category: WmCategory::Holidays,
                    location: "Worldwide",
                    note: "Statutory winter holiday.",
                },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
                WmCalendarEvent { id: 0, title: "", year: 0, month: 0, day: 0, time_start: "", time_end: "", category: WmCategory::Work, location: "", note: "" },
            ],
            event_count: 8,
            next_event_id: 9,
            status_msg: "Calendar Ready (Synced with RTC)",
        }
    }

    pub fn next_month(&mut self) {
        if self.view_month == 12 {
            self.view_month = 1;
            self.view_year = self.view_year.saturating_add(1);
        } else {
            self.view_month += 1;
        }
        let max_d = days_in_month(self.view_year, self.view_month);
        self.selected_day = self.selected_day.min(max_d).max(1);
    }

    pub fn prev_month(&mut self) {
        if self.view_month == 1 {
            self.view_month = 12;
            self.view_year = self.view_year.saturating_sub(1);
        } else {
            self.view_month -= 1;
        }
        let max_d = days_in_month(self.view_year, self.view_month);
        self.selected_day = self.selected_day.min(max_d).max(1);
    }

    pub fn go_today(&mut self) {
        self.view_year = self.today_year;
        self.view_month = self.today_month;
        self.selected_day = self.today_day;
    }

    pub fn toggle_category(&mut self, cat: WmCategory) {
        self.filter_mask ^= cat.bit_mask();
    }

    pub fn is_category_enabled(&self, cat: WmCategory) -> bool {
        (self.filter_mask & cat.bit_mask()) != 0
    }

    pub fn add_sample_event(&mut self) {
        if self.event_count < WM_MAX_CALENDAR_EVENTS {
            let id = self.next_event_id;
            self.next_event_id += 1;
            self.events[self.event_count] = WmCalendarEvent {
                id,
                title: "User Scheduled Session",
                year: self.view_year,
                month: self.view_month,
                day: self.selected_day,
                time_start: "03:30 PM",
                time_end: "04:30 PM",
                category: WmCategory::Work,
                location: "NovaOS Workspace",
                note: "Interactive event created via Calendar UI.",
            };
            self.event_count += 1;
            self.status_msg = "New event added to calendar!";
        } else {
            self.status_msg = "Calendar schedule full.";
        }
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let sb_w = if client.w >= 300 { 110 } else { 0 };

        // 1. Sidebar clicks
        if sb_w > 0 && px >= client.x && px < client.x + sb_w {
            // Hero Date card click -> jump today
            let hero_card = Rect::new(client.x + 6, client.y + 6, sb_w - 12, 58);
            if point_in_rect(px, py, hero_card.x, hero_card.y, hero_card.w, hero_card.h) {
                self.go_today();
                return;
            }

            // Categories checkboxes: starts at y + 80
            let cats = [
                WmCategory::Work,
                WmCategory::Personal,
                WmCategory::NovaOS,
                WmCategory::Holidays,
                WmCategory::Reminders,
            ];
            let mut cat_y = client.y + 80;
            for cat in cats.iter() {
                let cat_rect = Rect::new(client.x + 6, cat_y, sb_w - 12, 18);
                if point_in_rect(px, py, cat_rect.x, cat_rect.y, cat_rect.w, cat_rect.h) {
                    self.toggle_category(*cat);
                    return;
                }
                cat_y += 20;
            }

            // + Add Event button at bottom
            let add_btn = Rect::new(client.x + 8, client.bottom() - 28, sb_w - 16, 22);
            if point_in_rect(px, py, add_btn.x, add_btn.y, add_btn.w, add_btn.h) {
                self.add_sample_event();
                return;
            }
            return;
        }

        // 2. Main content area
        let cx = client.x + sb_w;
        let cw = client.w - sb_w;
        let hdr_h = 28;

        // Top Toolbar clicks
        if py >= client.y && py < client.y + hdr_h {
            // [ < ] Prev Month
            let btn_prev = Rect::new(cx + 8, client.y + 4, 20, 20);
            if point_in_rect(px, py, btn_prev.x, btn_prev.y, btn_prev.w, btn_prev.h) {
                self.prev_month();
                return;
            }

            // [ > ] Next Month
            let btn_next = Rect::new(cx + 128, client.y + 4, 20, 20);
            if point_in_rect(px, py, btn_next.x, btn_next.y, btn_next.w, btn_next.h) {
                self.next_month();
                return;
            }

            // [ Today ]
            let btn_today = Rect::new(cx + 154, client.y + 4, 46, 20);
            if point_in_rect(px, py, btn_today.x, btn_today.y, btn_today.w, btn_today.h) {
                self.go_today();
                return;
            }

            // View Switcher: [ Month | Week | Day | Agenda ]
            let v_modes = [WmViewMode::Month, WmViewMode::Week, WmViewMode::Day, WmViewMode::Agenda];
            let seg_w = 40;
            let total_seg_w = seg_w * 4;
            let seg_start_x = cx + cw - total_seg_w - 8;
            for (i, mode) in v_modes.iter().enumerate() {
                let seg_btn = Rect::new(seg_start_x + (i as i32) * seg_w, client.y + 4, seg_w, 20);
                if point_in_rect(px, py, seg_btn.x, seg_btn.y, seg_btn.w, seg_btn.h) {
                    self.view_mode = *mode;
                    return;
                }
            }
            return;
        }

        // 3. Grid clicks in Month view
        if self.view_mode == WmViewMode::Month {
            let grid_y0 = client.y + hdr_h + 18;
            let col_w = (cw - 12) / 7;
            let row_h = (client.h - hdr_h - 18 - 36).max(14) / 6;

            if px >= cx + 6 && px < cx + 6 + col_w * 7 && py >= grid_y0 && py < grid_y0 + row_h * 6 {
                let col = (px - (cx + 6)) / col_w;
                let row = (py - grid_y0) / row_h;
                let idx = (row * 7 + col) as usize;

                let first_dow = day_of_week(self.view_year, self.view_month, 1) as usize;
                let total_days = days_in_month(self.view_year, self.view_month) as usize;

                if idx >= first_dow && idx < first_dow + total_days {
                    let clicked_day = (idx - first_dow + 1) as u8;
                    self.selected_day = clicked_day;
                }
            }
        }
    }
}

// --- NovaOS Desktop Unified Calculator Subsystem (macOS + Windows 11 + Ubuntu Synthesized) ---

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WmCalcMode {
    Standard = 0,
    Scientific = 1,
    Programmer = 2,
    History = 3,
}

impl WmCalcMode {
    pub fn name(&self) -> &'static str {
        match self {
            WmCalcMode::Standard => "Standard",
            WmCalcMode::Scientific => "Scientific",
            WmCalcMode::Programmer => "Programmer",
            WmCalcMode::History => "History",
        }
    }
}

pub struct WmCalcApp {
    pub display: [u8; 32],
    pub display_len: usize,
    pub expr_line: [u8; 48],
    pub expr_len: usize,
    pub accumulator: f64,
    pub pending_op: u8,
    pub clear_on_next: bool,
    pub mode: WmCalcMode,
    pub prog_base: u8, // 0: Hex, 1: Dec, 2: Oct, 3: Bin
    pub memory: f64,
    pub has_memory: bool,
    pub is_error: bool,
    pub history_expr: [[u8; 32]; 4],
    pub history_expr_len: [usize; 4],
    pub history_res: [[u8; 16]; 4],
    pub history_res_len: [usize; 4],
    pub history_count: usize,
}

impl WmCalcApp {
    pub fn new() -> Self {
        let mut app = WmCalcApp {
            display: [0u8; 32],
            display_len: 1,
            expr_line: [0u8; 48],
            expr_len: 0,
            accumulator: 0.0,
            pending_op: 0,
            clear_on_next: false,
            mode: WmCalcMode::Standard,
            prog_base: 1, // Dec
            memory: 0.0,
            has_memory: false,
            is_error: false,
            history_expr: [[0u8; 32]; 4],
            history_expr_len: [0; 4],
            history_res: [[0u8; 16]; 4],
            history_res_len: [0; 4],
            history_count: 0,
        };
        app.display[0] = b'0';
        app.record_history("125 * 8", "= 1000");
        app.record_history("sqrt(256) + 4", "= 20");
        app
    }

    pub fn display_str(&self) -> &str {
        core::str::from_utf8(&self.display[..self.display_len]).unwrap_or("0")
    }

    pub fn expr_str(&self) -> &str {
        core::str::from_utf8(&self.expr_line[..self.expr_len]).unwrap_or("")
    }

    pub fn set_display_str(&mut self, s: &str) {
        let b = s.as_bytes();
        let copy_len = b.len().min(31);
        self.display[..copy_len].copy_from_slice(&b[..copy_len]);
        self.display_len = copy_len;
    }

    pub fn current_val(&self) -> f64 {
        if self.is_error {
            0.0
        } else if self.mode == WmCalcMode::Programmer {
            self.current_int() as f64
        } else {
            parse_f64_from(&self.display[..self.display_len])
        }
    }

    pub fn current_int(&self) -> i64 {
        if self.is_error {
            return 0;
        }
        let base = match self.prog_base {
            0 => 16,
            1 => 10,
            2 => 8,
            3 => 2,
            _ => 10,
        };
        parse_i64_base(&self.display[..self.display_len], base)
    }

    pub fn set_current_int(&mut self, val: i64) {
        self.display_len = 0;
        let uval = val as u64;
        match self.prog_base {
            0 => {
                let mut buf = [0u8; 32];
                let mut off = 0;
                let mut tmp = uval;
                if tmp == 0 {
                    append_str("0", &mut buf, &mut off);
                } else {
                    let mut digits = [0u8; 16];
                    let mut d_cnt = 0;
                    while tmp > 0 && d_cnt < 16 {
                        let rem = (tmp % 16) as u8;
                        digits[d_cnt] = if rem < 10 { b'0' + rem } else { b'A' + (rem - 10) };
                        tmp /= 16;
                        d_cnt += 1;
                    }
                    while d_cnt > 0 {
                        d_cnt -= 1;
                        buf[off] = digits[d_cnt];
                        off += 1;
                    }
                }
                let copy_len = off.min(31);
                self.display[..copy_len].copy_from_slice(&buf[..copy_len]);
                self.display_len = copy_len;
            }
            1 => {
                let mut buf = [0u8; 32];
                let mut off = 0;
                format_u32(val as u32, &mut buf, &mut off);
                let copy_len = off.min(31);
                self.display[..copy_len].copy_from_slice(&buf[..copy_len]);
                self.display_len = copy_len;
            }
            2 => {
                let mut buf = [0u8; 32];
                let mut off = 0;
                let mut tmp = uval;
                if tmp == 0 {
                    append_str("0", &mut buf, &mut off);
                } else {
                    let mut digits = [0u8; 22];
                    let mut d_cnt = 0;
                    while tmp > 0 && d_cnt < 22 {
                        digits[d_cnt] = b'0' + (tmp % 8) as u8;
                        tmp /= 8;
                        d_cnt += 1;
                    }
                    while d_cnt > 0 {
                        d_cnt -= 1;
                        buf[off] = digits[d_cnt];
                        off += 1;
                    }
                }
                let copy_len = off.min(31);
                self.display[..copy_len].copy_from_slice(&buf[..copy_len]);
                self.display_len = copy_len;
            }
            _ => {
                let mut buf = [0u8; 32];
                let mut off = 0;
                let mut tmp = uval;
                if tmp == 0 {
                    append_str("0", &mut buf, &mut off);
                } else {
                    let mut digits = [0u8; 32];
                    let mut d_cnt = 0;
                    while tmp > 0 && d_cnt < 32 {
                        digits[d_cnt] = b'0' + (tmp % 2) as u8;
                        tmp /= 2;
                        d_cnt += 1;
                    }
                    while d_cnt > 0 {
                        d_cnt -= 1;
                        buf[off] = digits[d_cnt];
                        off += 1;
                    }
                }
                let copy_len = off.min(31);
                self.display[..copy_len].copy_from_slice(&buf[..copy_len]);
                self.display_len = copy_len;
            }
        }
        self.clear_on_next = true;
    }

    pub fn input_digit(&mut self, digit: u8) {
        if self.is_error {
            self.clear();
        }

        if self.mode == WmCalcMode::Programmer {
            let valid = match self.prog_base {
                0 => (digit >= b'0' && digit <= b'9') || (digit >= b'A' && digit <= b'F') || (digit >= b'a' && digit <= b'f'),
                1 => digit >= b'0' && digit <= b'9',
                2 => digit >= b'0' && digit <= b'7',
                3 => digit == b'0' || digit == b'1',
                _ => true,
            };
            if !valid {
                return;
            }
        }

        let ch = if digit >= b'a' && digit <= b'f' { digit - 32 } else { digit };

        if self.clear_on_next || (self.display_len == 1 && self.display[0] == b'0') {
            self.display[0] = ch;
            self.display_len = 1;
            self.clear_on_next = false;
        } else if self.display_len < 24 {
            self.display[self.display_len] = ch;
            self.display_len += 1;
        }
    }

    pub fn input_dot(&mut self) {
        if self.mode == WmCalcMode::Programmer {
            return;
        }
        if self.is_error || self.clear_on_next {
            self.set_display_str("0.");
            self.clear_on_next = false;
            self.is_error = false;
        } else if !self.display[..self.display_len].contains(&b'.') && self.display_len < 24 {
            self.display[self.display_len] = b'.';
            self.display_len += 1;
        }
    }

    pub fn backspace(&mut self) {
        if self.is_error || self.clear_on_next {
            self.set_display_str("0");
            self.clear_on_next = false;
            self.is_error = false;
            return;
        }
        if self.display_len > 1 {
            self.display_len -= 1;
            if self.display_len == 1 && self.display[0] == b'-' {
                self.set_display_str("0");
            }
        } else {
            self.set_display_str("0");
        }
    }

    pub fn toggle_sign(&mut self) {
        if self.is_error || (self.display_len == 1 && self.display[0] == b'0') {
            return;
        }
        if self.mode == WmCalcMode::Programmer {
            let iv = self.current_int();
            self.set_current_int(-iv);
        } else {
            if self.display[0] == b'-' {
                for i in 0..self.display_len - 1 {
                    self.display[i] = self.display[i + 1];
                }
                self.display_len -= 1;
            } else if self.display_len < 30 {
                for i in (0..self.display_len).rev() {
                    self.display[i + 1] = self.display[i];
                }
                self.display[0] = b'-';
                self.display_len += 1;
            }
        }
    }

    pub fn clear(&mut self) {
        self.set_display_str("0");
        self.expr_len = 0;
        self.accumulator = 0.0;
        self.pending_op = 0;
        self.clear_on_next = false;
        self.is_error = false;
    }

    pub fn clear_entry(&mut self) {
        self.set_display_str("0");
        self.clear_on_next = false;
        self.is_error = false;
    }

    pub fn apply_op(&mut self, op: u8) {
        if self.is_error {
            return;
        }

        let cur = self.current_val();

        if self.pending_op != 0 && !self.clear_on_next {
            self.compute_binary(self.accumulator, cur, self.pending_op);
        } else {
            self.accumulator = cur;
        }

        self.pending_op = op;
        self.expr_len = 0;
        format_f64_into(self.accumulator, &mut self.expr_line, &mut self.expr_len);
        let op_sym = match op {
            b'*' => " * ",
            b'/' => " / ",
            b'+' => " + ",
            b'-' => " - ",
            b'%' => " % ",
            b'^' => " ^ ",
            b'&' => " & ",
            b'|' => " | ",
            b'X' => " ^ ",
            _ => " ",
        };
        append_str(op_sym, &mut self.expr_line, &mut self.expr_len);
        self.clear_on_next = true;
    }

    fn compute_binary(&mut self, a: f64, b: f64, op: u8) {
        let res = match op {
            b'+' => Ok(a + b),
            b'-' => Ok(a - b),
            b'*' => Ok(a * b),
            b'/' => {
                if b.abs() < 1e-12 {
                    Err("Divide by 0")
                } else {
                    Ok(a / b)
                }
            }
            b'%' => {
                if b.abs() < 1e-12 {
                    Err("Divide by 0")
                } else {
                    let ia = a as i64;
                    let ib = b as i64;
                    if ib == 0 { Err("Divide by 0") } else { Ok((ia % ib) as f64) }
                }
            }
            b'^' => {
                let mut r = 1.0;
                let exp = b as i32;
                if exp >= 0 {
                    for _ in 0..exp {
                        r *= a;
                    }
                    Ok(r)
                } else {
                    for _ in 0..(-exp) {
                        r *= a;
                    }
                    if r.abs() < 1e-12 { Err("Divide by 0") } else { Ok(1.0 / r) }
                }
            }
            b'&' => Ok(((a as i64) & (b as i64)) as f64),
            b'|' => Ok(((a as i64) | (b as i64)) as f64),
            b'X' => Ok(((a as i64) ^ (b as i64)) as f64),
            b'<' => {
                let sh = (b as i64).clamp(0, 63);
                Ok(((a as i64) << sh) as f64)
            }
            b'>' => {
                let sh = (b as i64).clamp(0, 63);
                Ok(((a as i64) >> sh) as f64)
            }
            _ => Ok(b),
        };

        match res {
            Ok(v) => {
                self.accumulator = v;
                if self.mode == WmCalcMode::Programmer {
                    self.set_current_int(v as i64);
                } else {
                    let mut b = [0u8; 32];
                    let mut off = 0;
                    format_f64_into(v, &mut b, &mut off);
                    let copy_len = off.min(31);
                    self.display[..copy_len].copy_from_slice(&b[..copy_len]);
                    self.display_len = copy_len;
                }
            }
            Err(e) => {
                self.set_display_str(e);
                self.is_error = true;
                self.pending_op = 0;
            }
        }
    }

    pub fn equals(&mut self) {
        if self.is_error || self.pending_op == 0 {
            return;
        }

        let cur = self.current_val();
        let op = self.pending_op;

        let mut expr_buf = [0u8; 32];
        let mut e_off = 0;
        format_f64_into(self.accumulator, &mut expr_buf, &mut e_off);
        let op_sym = match op {
            b'*' => " * ",
            b'/' => " / ",
            b'+' => " + ",
            b'-' => " - ",
            _ => " ? ",
        };
        append_str(op_sym, &mut expr_buf, &mut e_off);
        format_f64_into(cur, &mut expr_buf, &mut e_off);
        let expr_str = core::str::from_utf8(&expr_buf[..e_off]).unwrap_or("");

        self.compute_binary(self.accumulator, cur, op);

        if !self.is_error {
            let mut res_buf = [0u8; 16];
            let mut r_off = 0;
            append_str("= ", &mut res_buf, &mut r_off);
            append_str(self.display_str(), &mut res_buf, &mut r_off);
            let res_str = core::str::from_utf8(&res_buf[..r_off]).unwrap_or("");
            self.record_history(expr_str, res_str);

            self.expr_len = 0;
            append_str(expr_str, &mut self.expr_line, &mut self.expr_len);
            append_str(" =", &mut self.expr_line, &mut self.expr_len);
            self.pending_op = 0;
            self.clear_on_next = true;
        }
    }

    pub fn apply_unary(&mut self, op_name: &str) {
        if self.is_error {
            return;
        }

        let val = self.current_val();
        let mut expr_desc = [0u8; 32];
        let mut e_off = 0;

        let res = match op_name {
            "sqrt" => {
                append_str("sqrt(", &mut expr_desc, &mut e_off);
                format_f64_into(val, &mut expr_desc, &mut e_off);
                append_str(")", &mut expr_desc, &mut e_off);
                if val < 0.0 {
                    Err("Negative root")
                } else {
                    Ok(fast_sqrt64(val))
                }
            }
            "x^2" => {
                append_str("sqr(", &mut expr_desc, &mut e_off);
                format_f64_into(val, &mut expr_desc, &mut e_off);
                append_str(")", &mut expr_desc, &mut e_off);
                Ok(val * val)
            }
            "1/x" => {
                append_str("1/(", &mut expr_desc, &mut e_off);
                format_f64_into(val, &mut expr_desc, &mut e_off);
                append_str(")", &mut expr_desc, &mut e_off);
                if val.abs() < 1e-12 {
                    Err("Divide by 0")
                } else {
                    Ok(1.0 / val)
                }
            }
            "%" => {
                append_str(self.display_str(), &mut expr_desc, &mut e_off);
                append_str("%", &mut expr_desc, &mut e_off);
                Ok(val / 100.0)
            }
            "fact" => {
                append_str("fact(", &mut expr_desc, &mut e_off);
                format_f64_into(val, &mut expr_desc, &mut e_off);
                append_str(")", &mut expr_desc, &mut e_off);
                if val < 0.0 || val > 12.0 {
                    Err("Invalid fact")
                } else {
                    let n = val as u64;
                    let mut f = 1u64;
                    for i in 2..=n {
                        f *= i;
                    }
                    Ok(f as f64)
                }
            }
            "abs" => {
                append_str("abs(", &mut expr_desc, &mut e_off);
                format_f64_into(val, &mut expr_desc, &mut e_off);
                append_str(")", &mut expr_desc, &mut e_off);
                Ok(if val < 0.0 { -val } else { val })
            }
            "not" => {
                append_str("NOT(", &mut expr_desc, &mut e_off);
                format_f64_into(val, &mut expr_desc, &mut e_off);
                append_str(")", &mut expr_desc, &mut e_off);
                let iv = val as i64;
                Ok((!iv) as f64)
            }
            _ => Ok(val),
        };

        match res {
            Ok(v) => {
                if self.mode == WmCalcMode::Programmer {
                    self.set_current_int(v as i64);
                } else {
                    let mut b = [0u8; 32];
                    let mut off = 0;
                    format_f64_into(v, &mut b, &mut off);
                    let copy_len = off.min(31);
                    self.display[..copy_len].copy_from_slice(&b[..copy_len]);
                    self.display_len = copy_len;
                }

                self.expr_len = 0;
                let copy_e = e_off.min(47);
                self.expr_line[..copy_e].copy_from_slice(&expr_desc[..copy_e]);
                self.expr_len = copy_e;

                let e_str = core::str::from_utf8(&expr_desc[..copy_e]).unwrap_or("");
                let mut r_buf = [0u8; 16];
                let mut r_off = 0;
                append_str("= ", &mut r_buf, &mut r_off);
                let mut d_tmp = [0u8; 32];
                let dlen = self.display_len;
                d_tmp[..dlen].copy_from_slice(&self.display[..dlen]);
                let d_str = core::str::from_utf8(&d_tmp[..dlen]).unwrap_or("");
                append_str(d_str, &mut r_buf, &mut r_off);
                let r_str = core::str::from_utf8(&r_buf[..r_off]).unwrap_or("");
                self.record_history(e_str, r_str);
                self.clear_on_next = true;
            }
            Err(e) => {
                self.set_display_str(e);
                self.is_error = true;
            }
        }
    }

    pub fn memory_store(&mut self) {
        if !self.is_error {
            self.memory = self.current_val();
            self.has_memory = true;
            self.clear_on_next = true;
        }
    }

    pub fn memory_recall(&mut self) {
        if self.has_memory {
            let mut b = [0u8; 32];
            let mut off = 0;
            format_f64_into(self.memory, &mut b, &mut off);
            let copy_len = off.min(31);
            self.display[..copy_len].copy_from_slice(&b[..copy_len]);
            self.display_len = copy_len;
            self.clear_on_next = true;
        }
    }

    pub fn memory_add(&mut self) {
        if !self.is_error {
            self.memory += self.current_val();
            self.has_memory = true;
            self.clear_on_next = true;
        }
    }

    pub fn memory_sub(&mut self) {
        if !self.is_error {
            self.memory -= self.current_val();
            self.has_memory = true;
            self.clear_on_next = true;
        }
    }

    pub fn memory_clear(&mut self) {
        self.memory = 0.0;
        self.has_memory = false;
    }

    pub fn set_mode(&mut self, mode: WmCalcMode) {
        self.mode = mode;
    }

    pub fn record_history(&mut self, expr: &str, res: &str) {
        if self.history_count >= 4 {
            for i in 0..3 {
                self.history_expr[i] = self.history_expr[i + 1];
                self.history_expr_len[i] = self.history_expr_len[i + 1];
                self.history_res[i] = self.history_res[i + 1];
                self.history_res_len[i] = self.history_res_len[i + 1];
            }
            self.history_count = 3;
        }

        let idx = self.history_count;
        let eb = expr.as_bytes();
        let elen = eb.len().min(31);
        self.history_expr[idx][..elen].copy_from_slice(&eb[..elen]);
        self.history_expr_len[idx] = elen;

        let rb = res.as_bytes();
        let rlen = rb.len().min(15);
        self.history_res[idx][..rlen].copy_from_slice(&rb[..rlen]);
        self.history_res_len[idx] = rlen;

        self.history_count += 1;
    }

    pub fn clear_history(&mut self) {
        self.history_count = 0;
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let mode_bar_h = 26;
        if py >= client.y && py < client.y + mode_bar_h {
            let modes = [WmCalcMode::Standard, WmCalcMode::Scientific, WmCalcMode::Programmer, WmCalcMode::History];
            let pad = 4;
            let tab_w = (client.w - pad * 2) / 4;
            for (i, m) in modes.iter().enumerate() {
                let tx = client.x + pad + i as i32 * tab_w;
                let tr = Rect::new(tx, client.y + 2, tab_w - 4, mode_bar_h - 4);
                if point_in_rect(px, py, tr.x, tr.y, tr.w, tr.h) {
                    self.set_mode(*m);
                    return;
                }
            }
            return;
        }

        let screen_h = if self.mode == WmCalcMode::Programmer { 64 } else { 48 };
        let screen_y = client.y + mode_bar_h + 4;
        let mut key_start_y = screen_y + screen_h + 4;

        if self.mode == WmCalcMode::History {
            let clr_btn = Rect::new(client.x + client.w - 100, key_start_y + 4, 90, 20);
            if point_in_rect(px, py, clr_btn.x, clr_btn.y, clr_btn.w, clr_btn.h) {
                self.clear_history();
            }
            return;
        }

        if self.mode == WmCalcMode::Standard || self.mode == WmCalcMode::Scientific {
            let mem_h = 20;
            if py >= key_start_y && py < key_start_y + mem_h {
                let btns = ["MC", "MR", "M+", "M-", "MS"];
                let btn_w = (client.w - 16) / 5;
                for (i, &b) in btns.iter().enumerate() {
                    let bx = client.x + 8 + i as i32 * btn_w;
                    let br = Rect::new(bx, key_start_y, btn_w - 4, mem_h);
                    if point_in_rect(px, py, br.x, br.y, br.w, br.h) {
                        match b {
                            "MC" => self.memory_clear(),
                            "MR" => self.memory_recall(),
                            "M+" => self.memory_add(),
                            "M-" => self.memory_sub(),
                            "MS" => self.memory_store(),
                            _ => {}
                        }
                        return;
                    }
                }
            }
            key_start_y += mem_h + 4;
        }

        let pad = 4;
        let avail_h = client.bottom() - key_start_y - 6;
        if avail_h <= 0 {
            return;
        }

        match self.mode {
            WmCalcMode::Standard => {
                let grid = [
                    ["%", "CE", "C", "del"],
                    ["1/x", "x^2", "sqrt", "/"],
                    ["7", "8", "9", "*"],
                    ["4", "5", "6", "-"],
                    ["1", "2", "3", "+"],
                    ["+/-", "0", ".", "="],
                ];
                let rows = 6;
                let cols = 4;
                let btn_w = (client.w - 16 - pad * (cols - 1)) / cols;
                let btn_h = (avail_h - pad * (rows - 1)) / rows;

                for r in 0..rows {
                    for c in 0..cols {
                        let bx = client.x + 8 + c * (btn_w + pad);
                        let by = key_start_y + r * (btn_h + pad);
                        let br = Rect::new(bx, by, btn_w, btn_h);
                        if point_in_rect(px, py, br.x, br.y, br.w, br.h) {
                            let label = grid[r as usize][c as usize];
                            match label {
                                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => self.input_digit(label.as_bytes()[0]),
                                "." => self.input_dot(),
                                "+/-" => self.toggle_sign(),
                                "del" => self.backspace(),
                                "C" => self.clear(),
                                "CE" => self.clear_entry(),
                                "+" => self.apply_op(b'+'),
                                "-" => self.apply_op(b'-'),
                                "*" => self.apply_op(b'*'),
                                "/" => self.apply_op(b'/'),
                                "%" => self.apply_unary("%"),
                                "1/x" => self.apply_unary("1/x"),
                                "x^2" => self.apply_unary("x^2"),
                                "sqrt" => self.apply_unary("sqrt"),
                                "=" => self.equals(),
                                _ => {}
                            }
                            return;
                        }
                    }
                }
            }
            WmCalcMode::Scientific => {
                let grid = [
                    ["sin", "cos", "tan", "CE", "C"],
                    ["pi", "e", "^", "sqrt", "/"],
                    ["ln", "log", "x^2", "1/x", "*"],
                    ["n!", "7", "8", "9", "-"],
                    ["|x|", "4", "5", "6", "+"],
                    ["+/-", "1", "2", "3", "="],
                ];
                let rows = 6;
                let cols = 5;
                let btn_w = (client.w - 16 - pad * (cols - 1)) / cols;
                let btn_h = (avail_h - pad * (rows - 1)) / rows;

                for r in 0..rows {
                    for c in 0..cols {
                        let bx = client.x + 8 + c * (btn_w + pad);
                        let by = key_start_y + r * (btn_h + pad);
                        let br = Rect::new(bx, by, btn_w, btn_h);
                        if point_in_rect(px, py, br.x, br.y, br.w, br.h) {
                            let label = grid[r as usize][c as usize];
                            match label {
                                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => self.input_digit(label.as_bytes()[0]),
                                "+/-" => self.toggle_sign(),
                                "C" => self.clear(),
                                "CE" => self.clear_entry(),
                                "+" => self.apply_op(b'+'),
                                "-" => self.apply_op(b'-'),
                                "*" => self.apply_op(b'*'),
                                "/" => self.apply_op(b'/'),
                                "^" => self.apply_op(b'^'),
                                "sqrt" => self.apply_unary("sqrt"),
                                "x^2" => self.apply_unary("x^2"),
                                "1/x" => self.apply_unary("1/x"),
                                "n!" => self.apply_unary("fact"),
                                "|x|" => self.apply_unary("abs"),
                                "pi" => {
                                    self.set_display_str("3.14159");
                                    self.clear_on_next = true;
                                }
                                "e" => {
                                    self.set_display_str("2.71828");
                                    self.clear_on_next = true;
                                }
                                "=" => self.equals(),
                                _ => {}
                            }
                            return;
                        }
                    }
                }
            }
            WmCalcMode::Programmer => {
                let grid = [
                    ["AND", "OR", "XOR", "NOT"],
                    ["<<", ">>", "CE", "C"],
                    ["A", "B", "7", "8"],
                    ["C", "D", "4", "5"],
                    ["E", "F", "1", "2"],
                    ["HEX", "DEC", "0", "="],
                ];
                let rows = 6;
                let cols = 4;
                let btn_w = (client.w - 16 - pad * (cols - 1)) / cols;
                let btn_h = (avail_h - pad * (rows - 1)) / rows;

                for r in 0..rows {
                    for c in 0..cols {
                        let bx = client.x + 8 + c * (btn_w + pad);
                        let by = key_start_y + r * (btn_h + pad);
                        let br = Rect::new(bx, by, btn_w, btn_h);
                        if point_in_rect(px, py, br.x, br.y, br.w, br.h) {
                            if r == 1 && c == 3 {
                                self.clear();
                                return;
                            }
                            let label = grid[r as usize][c as usize];
                            match label {
                                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                                | "A" | "B" | "C" | "D" | "E" | "F" => self.input_digit(label.as_bytes()[0]),
                                "CE" => self.clear_entry(),
                                "AND" => self.apply_op(b'&'),
                                "OR" => self.apply_op(b'|'),
                                "XOR" => self.apply_op(b'X'),
                                "NOT" => self.apply_unary("not"),
                                "<<" => self.apply_op(b'<'),
                                ">>" => self.apply_op(b'>'),
                                "HEX" => {
                                    let v = self.current_int();
                                    self.prog_base = 0;
                                    self.set_current_int(v);
                                }
                                "DEC" => {
                                    let v = self.current_int();
                                    self.prog_base = 1;
                                    self.set_current_int(v);
                                }
                                "=" => self.equals(),
                                _ => {}
                            }
                            return;
                        }
                    }
                }
            }
            WmCalcMode::History => {}
        }
    }
}

pub const TERM_MAX_LINES: usize = 28;
pub const TERM_LINE_LEN: usize = 96;
pub const TERM_INPUT_LEN: usize = 128;

#[derive(Clone, Copy)]
pub struct TermLine {
    pub text: [u8; TERM_LINE_LEN],
    pub len: usize,
    pub color: Color,
}

impl TermLine {
    pub const fn empty() -> Self {
        TermLine {
            text: [0; TERM_LINE_LEN],
            len: 0,
            color: Color::WHITE,
        }
    }

    pub fn from_str(s: &str, color: Color) -> Self {
        let mut line = TermLine::empty();
        line.color = color;
        let bytes = s.as_bytes();
        let copy_len = bytes.len().min(TERM_LINE_LEN);
        let mut i = 0;
        while i < copy_len {
            line.text[i] = bytes[i];
            i += 1;
        }
        line.len = copy_len;
        line
    }
}

// --- Aero Browser Desktop App Subsystem ---

#[derive(Clone, Debug, PartialEq)]
pub struct WmBrowserApp {
    pub active_tab: usize,
    pub tab_count: usize,
    pub url_buf: [u8; 64],
    pub url_len: usize,
    pub is_secure: bool,
    pub shields_up: bool,
    pub trackers_blocked: u32,
    pub bandwidth_saved_kb: u32,
    pub shields_flyout_open: bool,
    pub reader_mode: bool,
    pub devtools_open: bool,
    pub devtools_tab: u8, // 0: Elements, 1: Console, 2: Network, 3: Shields
    pub scroll_y: i32,
}

impl WmBrowserApp {
    pub fn new() -> Self {
        let mut app = WmBrowserApp {
            active_tab: 0,
            tab_count: 3,
            url_buf: [0u8; 64],
            url_len: 0,
            is_secure: true,
            shields_up: true,
            trackers_blocked: 14,
            bandwidth_saved_kb: 42,
            shields_flyout_open: false,
            reader_mode: false,
            devtools_open: false,
            devtools_tab: 0,
            scroll_y: 0,
        };
        app.set_url("about:newtab");
        app
    }

    pub fn set_url(&mut self, url: &str) {
        self.url_len = 0;
        for b in url.bytes() {
            if self.url_len < self.url_buf.len() {
                self.url_buf[self.url_len] = b;
                self.url_len += 1;
            }
        }
        self.is_secure = url.starts_with("https://") || url.starts_with("about:");
    }

    pub fn url_str(&self) -> &str {
        core::str::from_utf8(&self.url_buf[..self.url_len]).unwrap_or("about:newtab")
    }

    pub fn switch_tab(&mut self, idx: usize) {
        if idx < self.tab_count {
            self.active_tab = idx;
            match idx {
                0 => self.set_url("about:newtab"),
                1 => self.set_url("about:shields"),
                2 => self.set_url("https://novaos.org"),
                _ => {}
            }
            self.scroll_y = 0;
            self.reader_mode = false;
        }
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let rel_x = px - client.x;
        let rel_y = py - client.y;

        // 0. Shields flyout dismiss or toggle button click
        if self.shields_flyout_open {
            let flyout_w = 230;
            let flyout_h = 160;
            let flyout_x = client.w - flyout_w - 10;
            let flyout_y = 62;
            if rel_x >= flyout_x && rel_x < flyout_x + flyout_w && rel_y >= flyout_y && rel_y < flyout_y + flyout_h {
                // Click toggle UP/DOWN button
                if rel_x >= flyout_x + flyout_w - 85 && rel_y >= flyout_y + 8 && rel_y <= flyout_y + 28 {
                    self.shields_up = !self.shields_up;
                }
                return;
            } else {
                self.shields_flyout_open = false;
            }
        }

        // 1. Tab strip (y: 0..28)
        if rel_y >= 0 && rel_y < 28 {
            let tab_w = 110;
            for i in 0..self.tab_count {
                let tx = 8 + (i as i32) * (tab_w + 4);
                if rel_x >= tx && rel_x < tx + tab_w {
                    self.switch_tab(i);
                    return;
                }
            }
        }

        // 2. Navigation bar (y: 28..62)
        if rel_y >= 28 && rel_y < 62 {
            // Back (<-)
            if rel_x >= 8 && rel_x < 26 {
                if self.active_tab > 0 {
                    self.switch_tab(self.active_tab - 1);
                }
                return;
            }
            // Forward (->)
            if rel_x >= 28 && rel_x < 46 {
                if self.active_tab + 1 < self.tab_count {
                    self.switch_tab(self.active_tab + 1);
                }
                return;
            }
            // Reload (R)
            if rel_x >= 48 && rel_x < 66 {
                self.scroll_y = 0;
                return;
            }
            // Home (H)
            if rel_x >= 68 && rel_x < 86 {
                self.switch_tab(0);
                return;
            }

            // Shields button (14)
            let shields_x = client.w - 110;
            if rel_x >= shields_x && rel_x < shields_x + 36 {
                self.shields_flyout_open = !self.shields_flyout_open;
                return;
            }
            // Reader mode button (Rd)
            let reader_x = client.w - 70;
            if rel_x >= reader_x && rel_x < reader_x + 30 {
                self.reader_mode = !self.reader_mode;
                return;
            }
            // DevTools button (DT)
            let devtools_x = client.w - 36;
            if rel_x >= devtools_x && rel_x < devtools_x + 30 {
                self.devtools_open = !self.devtools_open;
                return;
            }
        }

        // 3. Bookmarks bar (y: 62..84)
        if rel_y >= 62 && rel_y < 84 {
            let bm_w = 95;
            if rel_x >= 10 && rel_x < 10 + bm_w {
                self.switch_tab(2); // NovaOS Portal
                return;
            }
            if rel_x >= 10 + bm_w && rel_x < 10 + bm_w * 2 {
                self.switch_tab(1); // Shields
                return;
            }
            if rel_x >= 10 + bm_w * 2 && rel_x < 10 + bm_w * 3 {
                self.switch_tab(0); // New Tab
                return;
            }
        }

        // 4. DevTools tabs if open
        if self.devtools_open && rel_y >= client.h - 90 {
            let dt_y = client.h - 90;
            if rel_y < dt_y + 24 {
                let mut tx = 10;
                for tab_idx in 0..4 {
                    if rel_x >= tx && rel_x < tx + 55 {
                        self.devtools_tab = tab_idx;
                        return;
                    }
                    tx += 60;
                }
            }
        }
    }
}

// --- Nova Player Desktop App Subsystem ---

#[derive(Clone, Debug, PartialEq)]
pub struct WmPlayerApp {
    pub is_playing: bool,
    pub active_track: usize,
    pub volume_pct: u32,
    pub is_muted: bool,
    pub show_visualizer: bool,
    pub show_playlist: bool,
    pub show_about: bool,
    pub is_looping: bool,
    pub is_shuffling: bool,
    pub active_menu: Option<usize>,
    pub speed_step: u8,
    pub aspect_idx: u8,
    pub position_secs: u32,
    pub duration_secs: u32,
}

impl WmPlayerApp {
    pub fn new() -> Self {
        WmPlayerApp {
            is_playing: true,
            active_track: 0,
            volume_pct: 100,
            is_muted: false,
            show_visualizer: false,
            show_playlist: false,
            show_about: false,
            is_looping: false,
            is_shuffling: false,
            active_menu: None,
            speed_step: 1, // 1.0x
            aspect_idx: 0, // Auto
            position_secs: 45,
            duration_secs: 180,
        }
    }

    pub fn toggle_play(&mut self) {
        self.is_playing = !self.is_playing;
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
        self.position_secs = 0;
    }

    pub fn next_track(&mut self) {
        self.active_track = (self.active_track + 1) % 4;
        self.position_secs = 0;
    }

    pub fn prev_track(&mut self) {
        if self.position_secs > 5 {
            self.position_secs = 0;
        } else if self.active_track > 0 {
            self.active_track -= 1;
            self.position_secs = 0;
        }
    }

    pub fn toggle_viz(&mut self) {
        self.show_visualizer = !self.show_visualizer;
    }

    pub fn toggle_playlist(&mut self) {
        self.show_playlist = !self.show_playlist;
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let rel_x = px - client.x;
        let rel_y = py - client.y;

        // 1. About Modal Dialog close
        if self.show_about {
            self.show_about = false;
            return;
        }

        // 2. Dropdown Menu Clicks (if an active menu is open)
        if let Some(menu_idx) = self.active_menu {
            // Dropdown area: x in 6..client.w - 6, y in 46..210
            if rel_y >= 46 && rel_y <= 210 && rel_x >= 6 && rel_x <= client.w - 6 {
                let item_idx = ((rel_y - 48) / 20).max(0) as usize;
                match menu_idx {
                    0 => { // Media
                        match item_idx {
                            0 | 1 | 2 => self.next_track(),
                            _ => {}
                        }
                    }
                    1 => { // Playback
                        match item_idx {
                            0 => self.toggle_play(),
                            1 => self.stop(),
                            2 => self.prev_track(),
                            3 => self.next_track(),
                            4 => self.is_looping = !self.is_looping,
                            5 => self.is_shuffling = !self.is_shuffling,
                            _ => {}
                        }
                    }
                    2 => { // Audio
                        match item_idx {
                            0 => self.is_muted = !self.is_muted,
                            1 => self.volume_pct = (self.volume_pct + 5).min(125),
                            2 => self.volume_pct = self.volume_pct.saturating_sub(5),
                            3 => self.toggle_viz(),
                            _ => {}
                        }
                    }
                    3 => { // Video
                        match item_idx {
                            0 => self.aspect_idx = (self.aspect_idx + 1) % 4,
                            1 => self.active_track = (self.active_track + 1) % 4,
                            _ => {}
                        }
                    }
                    4 => { // Subtitle
                        // Subtitle toggle
                    }
                    5 => { // Tools
                        match item_idx {
                            0 => self.toggle_viz(),
                            _ => {}
                        }
                    }
                    6 => { // View
                        match item_idx {
                            0 => self.toggle_playlist(),
                            1 => self.toggle_viz(),
                            _ => {}
                        }
                    }
                    7 => { // Help
                        match item_idx {
                            0 => self.show_about = true,
                            _ => {}
                        }
                    }
                    _ => {}
                }
                self.active_menu = None;
                return;
            } else if rel_y > 46 {
                // Clicking outside closes the active dropdown menu
                self.active_menu = None;
            }
        }

        // 3. Menubar Strip Clicks (rel_y in 26..46)
        if rel_y >= 26 && rel_y <= 46 {
            let menu_slots = [
                (6, 48),    // Media
                (50, 106),  // Playback
                (108, 146), // Audio
                (148, 186), // Video
                (188, 238), // Subtitle
                (240, 278), // Tools
                (280, 314), // View
                (316, 348), // Help
            ];
            for (idx, &(mx1, mx2)) in menu_slots.iter().enumerate() {
                if rel_x >= mx1 && rel_x <= mx2 {
                    if self.active_menu == Some(idx) {
                        self.active_menu = None;
                    } else {
                        self.active_menu = Some(idx);
                    }
                    return;
                }
            }
        }

        // 4. Bottom Toolbar (VLC Layout)
        if rel_y >= client.h - 58 && rel_y <= client.h - 6 {
            // Volume Mute toggle (right aligned)
            if rel_x >= client.w - 110 && rel_x <= client.w - 80 {
                self.is_muted = !self.is_muted;
                return;
            }
            // Volume slider (right aligned)
            if rel_x >= client.w - 75 && rel_x <= client.w - 15 {
                let frac = ((rel_x - (client.w - 75)) as f32 / 60.0).clamp(0.0, 1.0);
                self.volume_pct = (frac * 125.0) as u32;
                self.is_muted = false;
                return;
            }
            // Play/Pause button
            if rel_x >= 14 && rel_x <= 46 {
                self.toggle_play();
                return;
            }
            // Stop button
            if rel_x >= 48 && rel_x <= 74 {
                self.stop();
                return;
            }
            // Prev button
            if rel_x >= 76 && rel_x <= 102 {
                self.prev_track();
                return;
            }
            // Next button
            if rel_x >= 104 && rel_x <= 130 {
                self.next_track();
                return;
            }
            // Fullscreen button
            if rel_x >= 132 && rel_x <= 158 {
                return;
            }
            // EQ / Effects button
            if rel_x >= 160 && rel_x <= 188 {
                self.toggle_viz();
                return;
            }
            // Playlist button
            if rel_x >= 190 && rel_x <= 218 {
                self.toggle_playlist();
                return;
            }
            // Loop button (wide mode)
            if rel_x >= 220 && rel_x <= 250 {
                self.is_looping = !self.is_looping;
                return;
            }
            // Shuffle button (wide mode)
            if rel_x >= 252 && rel_x <= 282 {
                self.is_shuffling = !self.is_shuffling;
                return;
            }
        }

        // 5. Top right quick actions (PL, VIZ)
        if rel_y >= 6 && rel_y <= 24 {
            if rel_x >= client.w - 40 && rel_x <= client.w - 8 {
                self.toggle_playlist();
                return;
            }
            if rel_x >= client.w - 75 && rel_x <= client.w - 42 {
                self.toggle_viz();
                return;
            }
        }

        // Click on video canvas toggles play
        self.toggle_play();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WmVellumApp {
    pub page: usize,
    pub total_pages: usize,
    pub mode: u8,             // 0: Read, 1: Annotate, 2: Edit, 3: Form
    pub active_tool: u8,       // 0: Select, 1: Text, 2: Highlight, 3: Underline, 4: Strike, 5: Pen, 6: Rect, 7: TextBox, 8: Note, 9: Redact
    pub active_color: u8,      // 0: Yellow, 1: Green, 2: Cyan, 3: Coral, 4: Purple
    pub zoom_pct: u32,
    pub sidebar_open: bool,
    pub sidebar_tab: u8,       // 0: Pages, 1: Outline, 2: Annots
    pub search_open: bool,
    pub match_idx: usize,
    pub total_matches: usize,
    pub night_mode: bool,
    pub show_about: bool,
    pub show_inspector: bool,
    pub selected_span: Option<usize>,
}

impl WmVellumApp {
    pub fn new() -> Self {
        WmVellumApp {
            page: 0,
            total_pages: 3,
            mode: 0,
            active_tool: 0,
            active_color: 0,
            zoom_pct: 100,
            sidebar_open: true,
            sidebar_tab: 0,
            search_open: false,
            match_idx: 1,
            total_matches: 3,
            night_mode: false,
            show_about: false,
            show_inspector: false,
            selected_span: None,
        }
    }

    pub fn set_mode(&mut self, m: u8) {
        self.mode = m;
        if m == 1 {
            self.active_tool = 2; // Default to Highlight tool in Annotate mode
        }
    }

    pub fn set_tool(&mut self, t: u8) {
        self.active_tool = t;
    }

    pub fn set_color(&mut self, c: u8) {
        self.active_color = c % 5;
    }

    pub fn next_page(&mut self) {
        if self.page + 1 < self.total_pages {
            self.page += 1;
            self.selected_span = None;
        }
    }

    pub fn prev_page(&mut self) {
        if self.page > 0 {
            self.page -= 1;
            self.selected_span = None;
        }
    }

    pub fn zoom_in(&mut self) {
        self.zoom_pct = (self.zoom_pct + 15).min(300);
    }

    pub fn zoom_out(&mut self) {
        self.zoom_pct = self.zoom_pct.saturating_sub(15).max(30);
    }

    pub fn toggle_search(&mut self) {
        self.search_open = !self.search_open;
    }

    pub fn toggle_night(&mut self) {
        self.night_mode = !self.night_mode;
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_open = !self.sidebar_open;
    }

    pub fn toggle_inspector(&mut self) {
        self.show_inspector = !self.show_inspector;
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let rel_x = px - client.x;
        let rel_y = py - client.y;

        // 1. If About modal is open, click anywhere dismisses it
        if self.show_about {
            self.show_about = false;
            return;
        }

        // 2. Top Ribbon Clicks (rel_y in 0..32)
        if rel_y >= 0 && rel_y <= 32 {
            // Mode Tabs: Read (32..68), Annotate (70..118), Edit (120..154), Form (156..190)
            if rel_x >= 32 && rel_x <= 68 {
                self.set_mode(0);
                return;
            }
            if rel_x >= 70 && rel_x <= 118 {
                self.set_mode(1);
                return;
            }
            if rel_x >= 120 && rel_x <= 154 {
                self.set_mode(2);
                return;
            }
            if rel_x >= 156 && rel_x <= 190 {
                self.set_mode(3);
                return;
            }

            // Right view controls:
            // ? About: client.w - 28 .. client.w - 4
            if rel_x >= client.w - 28 && rel_x <= client.w - 4 {
                self.show_about = true;
                return;
            }
            // Night: client.w - 68 .. client.w - 30
            if rel_x >= client.w - 68 && rel_x <= client.w - 30 {
                self.toggle_night();
                return;
            }
            // Find (Search): client.w - 106 .. client.w - 70
            if rel_x >= client.w - 106 && rel_x <= client.w - 70 {
                self.toggle_search();
                return;
            }
            // Sidebar: client.w - 152 .. client.w - 108
            if rel_x >= client.w - 152 && rel_x <= client.w - 108 {
                self.toggle_sidebar();
                return;
            }
            // Inspect (if wide client): client.w - 200 .. client.w - 154
            if client.w >= 500 && rel_x >= client.w - 200 && rel_x <= client.w - 154 {
                self.toggle_inspector();
                return;
            }
            return;
        }

        // 3. Search Bar Clicks (if search open, overlay at top right takes priority)
        if self.search_open && rel_x >= client.w - 240 && rel_x <= client.w - 10 && rel_y >= 34 && rel_y <= 62 {
            // Next hit: '>' button at client.w - 55..client.w - 35
            if rel_x >= client.w - 55 && rel_x <= client.w - 35 {
                self.match_idx = if self.match_idx >= self.total_matches { 1 } else { self.match_idx + 1 };
                return;
            }
            // Prev hit: '<' button at client.w - 75..client.w - 58
            if rel_x >= client.w - 75 && rel_x <= client.w - 58 {
                self.match_idx = if self.match_idx <= 1 { self.total_matches } else { self.match_idx - 1 };
                return;
            }
            // Close button: 'X' at client.w - 32..client.w - 10
            if rel_x >= client.w - 32 && rel_x <= client.w - 10 {
                self.search_open = false;
                return;
            }
            return;
        }

        // 4. Markup Bar Clicks (rel_y in 32..58) when in Annotate mode
        if self.mode == 1 && rel_y >= 32 && rel_y <= 58 {
            if rel_x >= 12 && rel_x <= 48 { self.set_tool(0); return; }
            if rel_x >= 52 && rel_x <= 82 { self.set_tool(1); return; }
            if rel_x >= 86 && rel_x <= 145 { self.set_tool(2); return; }
            if rel_x >= 149 && rel_x <= 205 { self.set_tool(3); return; }
            if rel_x >= 209 && rel_x <= 252 { self.set_tool(4); return; }
            if rel_x >= 256 && rel_x <= 286 { self.set_tool(5); return; }
            if rel_x >= 290 && rel_x <= 322 { self.set_tool(6); return; }
            if rel_x >= 326 && rel_x <= 372 { self.set_tool(7); return; }
            if rel_x >= 376 && rel_x <= 410 { self.set_tool(8); return; }
            if rel_x >= 414 && rel_x <= 458 { self.set_tool(9); return; }

            // Color swatches (right aligned in markup bar: 5 swatches)
            let swatch_start = (client.w - 120).max(470);
            for i in 0..5 {
                let sx = swatch_start + (i as i32) * 20;
                if rel_x >= sx && rel_x <= sx + 16 {
                    self.set_color(i as u8);
                    return;
                }
            }
            return;
        }

        // 5. Bottom Status Bar Clicks (rel_y >= client.h - 26)
        if rel_y >= client.h - 26 {
            // Prev Page button '<': rel_x in 8..24
            if rel_x >= 8 && rel_x <= 24 {
                self.prev_page();
                return;
            }
            // Next Page button '>': rel_x in 92..108
            if rel_x >= 92 && rel_x <= 108 {
                self.next_page();
                return;
            }
            // Zoom Out '-': rel_x in client.w - 90..client.w - 72
            if rel_x >= client.w - 90 && rel_x <= client.w - 72 {
                self.zoom_out();
                return;
            }
            // Zoom In '+': rel_x in client.w - 40..client.w - 22
            if rel_x >= client.w - 40 && rel_x <= client.w - 22 {
                self.zoom_in();
                return;
            }
            return;
        }

        // 6. Left Sidebar Thumbnail Clicks (if sidebar open, rel_x in 0..120)
        if self.sidebar_open && rel_x <= 120 {
            // Tabs: Pages (8..45), Outline (48..85), Annots (88..118)
            if rel_y >= 34 && rel_y <= 52 {
                if rel_x <= 46 { self.sidebar_tab = 0; }
                else if rel_x <= 86 { self.sidebar_tab = 1; }
                else { self.sidebar_tab = 2; }
                return;
            }

            // Thumbnail cards:
            if rel_y >= 58 && rel_y <= 140 { self.page = 0; return; }
            if rel_y >= 148 && rel_y <= 230 { self.page = 1; return; }
            if rel_y >= 238 && rel_y <= 320 { self.page = 2; return; }
            return;
        }

        // 7. Paper Canvas Click: in Edit mode, click selects text span
        if self.mode == 2 {
            self.selected_span = Some(if self.selected_span.is_some() { 0 } else { 1 });
        }
    }
}

// --- Vortex Studio Desktop App Subsystem ---

#[derive(Clone, Debug, PartialEq)]
pub struct WmVortexApp {
    pub filter: u8, // 0: All, 1: Torrents, 2: Streams, 3: Active, 4: Completed
    pub selected_task: usize, // 0: NovaOS Torrent, 1: YouTube Stream, 2: Linux Kernel Torrent, 3: Vimeo Stream
    pub inspector_tab: u8, // 0: Visualizer (Piece Grid or Chunk Bars), 1: Peers & Connections, 2: Info
    pub show_about: bool,
    pub download_rate_mb: u32,
    pub upload_rate_mb: u32,
    pub active_tasks: u8,
    pub completed_tasks: u8,
    pub torrent_pieces_done: u16,
    pub torrent_pieces_total: u16,
    pub stream_chunks_done: u8,
    pub stream_chunks_total: u8,
}

impl WmVortexApp {
    pub fn new() -> Self {
        WmVortexApp {
            filter: 0,
            selected_task: 0,
            inspector_tab: 0,
            show_about: false,
            download_rate_mb: 31,
            upload_rate_mb: 3,
            active_tasks: 2,
            completed_tasks: 2,
            torrent_pieces_done: 962,
            torrent_pieces_total: 1300,
            stream_chunks_done: 5,
            stream_chunks_total: 8,
        }
    }

    pub fn set_filter(&mut self, f: u8) {
        self.filter = f % 5;
    }

    pub fn set_selected_task(&mut self, idx: usize) {
        self.selected_task = idx % 4;
    }

    pub fn set_inspector_tab(&mut self, tab: u8) {
        self.inspector_tab = tab % 3;
    }

    pub fn toggle_about(&mut self) {
        self.show_about = !self.show_about;
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let rel_x = px - client.x;
        let rel_y = py - client.y;

        // 1. If About dialog is open, click anywhere dismisses it
        if self.show_about {
            self.show_about = false;
            return;
        }

        // 2. Toolbar clicks (rel_y: 0..40)
        if rel_y >= 0 && rel_y <= 40 {
            // ? About button: client.w - 36 .. client.w - 8
            if rel_x >= client.w - 36 && rel_x <= client.w - 8 {
                self.show_about = true;
                return;
            }
            // + Torrent: 70..140
            if rel_x >= 70 && rel_x <= 140 {
                self.torrent_pieces_done = 962;
                self.selected_task = 0;
                return;
            }
            // + Add URL / Stream: 145..200
            if rel_x >= 145 && rel_x <= 200 {
                self.selected_task = 1;
                return;
            }
            // Resume: 205..260
            if rel_x >= 205 && rel_x <= 260 {
                self.download_rate_mb = 31;
                return;
            }
            // Pause: 265..315
            if rel_x >= 265 && rel_x <= 315 {
                self.download_rate_mb = 0;
                return;
            }
            // Delete: 320..370
            if rel_x >= 320 && rel_x <= 370 {
                self.selected_task = 0;
                return;
            }
            return;
        }

        // 3. Sidebar Category Clicks (rel_x in 0..110, rel_y in 50..180)
        if rel_x >= 0 && rel_x <= 110 && rel_y >= 50 && rel_y <= 180 {
            let cat_idx = ((rel_y - 50) / 24).max(0) as u8;
            self.filter = cat_idx.min(4);
            return;
        }

        // 4. Main Queue Table Clicks (rel_x > 110, rel_y in 50..170)
        if rel_x > 110 && rel_y >= 50 && rel_y <= 170 {
            let row_idx = ((rel_y - 50) / 26).max(0) as usize;
            self.selected_task = row_idx.min(3);
            return;
        }

        // 5. Bottom Inspector Tab Clicks (rel_y in 170..200)
        if rel_y >= 170 && rel_y <= 200 {
            let tab_idx = ((rel_x - 110) / 80).max(0) as u8;
            self.inspector_tab = tab_idx.min(2);
            return;
        }
    }
}

// --- Zenith Office Productivity Suite Desktop App Subsystem ---

#[derive(Clone, Debug, PartialEq)]
pub struct WmZenithApp {
    pub active_module: u8, // 0: Writer, 1: Calc, 2: Present, 3: Base
    pub ribbon_tab: u8,    // 0: Home, 1: Insert, 2: Layout, 3: Formulas/Data, 4: View
    pub font_family_idx: u8, // 0: Calibri, 1: Times New Roman, 2: Arial, 3: Segoe UI, 4: Cambria, 5: Consolas, 6: Georgia, 7: Liberation Sans, 8: Liberation Serif, 9: Liberation Mono, 10: Ubuntu
    pub font_size: u16,    // 11 pt default
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub align: u8,         // 0: Left, 1: Center, 2: Right
    pub active_cell_row: usize,
    pub active_cell_col: usize,
    pub active_slide: usize,
    pub active_table_record: usize,
    pub show_about: bool,
}

impl WmZenithApp {
    pub fn new() -> Self {
        WmZenithApp {
            active_module: 0,
            ribbon_tab: 0,
            font_family_idx: 0,
            font_size: 11,
            bold: false,
            italic: false,
            underline: false,
            align: 0,
            active_cell_row: 0,
            active_cell_col: 0,
            active_slide: 0,
            active_table_record: 0,
            show_about: false,
        }
    }

    pub fn set_module(&mut self, m: u8) {
        self.active_module = m % 4;
    }

    pub fn set_ribbon_tab(&mut self, t: u8) {
        self.ribbon_tab = t % 5;
    }

    pub fn toggle_bold(&mut self) {
        self.bold = !self.bold;
    }

    pub fn toggle_italic(&mut self) {
        self.italic = !self.italic;
    }

    pub fn toggle_underline(&mut self) {
        self.underline = !self.underline;
    }

    pub fn cycle_font(&mut self) {
        self.font_family_idx = (self.font_family_idx + 1) % 11;
    }

    pub fn font_name(&self) -> &'static str {
        match self.font_family_idx {
            0 => "Calibri",
            1 => "Times New Roman",
            2 => "Arial",
            3 => "Segoe UI",
            4 => "Cambria",
            5 => "Consolas",
            6 => "Georgia",
            7 => "Liberation Sans",
            8 => "Liberation Serif",
            9 => "Liberation Mono",
            10 => "Ubuntu",
            _ => "Calibri",
        }
    }

    pub fn toggle_about(&mut self) {
        self.show_about = !self.show_about;
    }

    pub fn handle_click(&mut self, px: i32, py: i32, client: Rect) {
        let rel_x = px - client.x;
        let rel_y = py - client.y;

        if self.show_about {
            self.show_about = false;
            return;
        }

        // 1. Top Module Switcher Bar (rel_y: 0..26)
        if rel_y >= 0 && rel_y <= 26 {
            if rel_x >= client.w - 36 && rel_x <= client.w - 8 {
                self.show_about = true;
                return;
            }
            if rel_x >= 4 && rel_x <= 80 {
                self.active_module = 0; // Writer
                return;
            }
            if rel_x >= 85 && rel_x <= 155 {
                self.active_module = 1; // Calc
                return;
            }
            if rel_x >= 160 && rel_x <= 245 {
                self.active_module = 2; // Present
                return;
            }
            if rel_x >= 250 && rel_x <= 320 {
                self.active_module = 3; // Base
                return;
            }
            return;
        }

        // 2. Ribbon Tabs (rel_y: 26..46)
        if rel_y >= 26 && rel_y <= 46 {
            let tab_idx = (rel_x / 65).max(0) as u8;
            self.ribbon_tab = tab_idx.min(4);
            return;
        }

        // 3. Ribbon Controls (rel_y: 46..84)
        if rel_y >= 46 && rel_y <= 84 {
            if rel_x >= 8 && rel_x <= 120 {
                self.cycle_font();
                return;
            }
            if rel_x >= 125 && rel_x <= 160 {
                self.font_size = if self.font_size >= 16 { 9 } else { self.font_size + 2 };
                return;
            }
            if rel_x >= 165 && rel_x <= 190 {
                self.toggle_bold();
                return;
            }
            if rel_x >= 195 && rel_x <= 220 {
                self.toggle_italic();
                return;
            }
            if rel_x >= 225 && rel_x <= 250 {
                self.toggle_underline();
                return;
            }
            if rel_x >= 255 && rel_x <= 275 {
                self.align = 0;
                return;
            }
            if rel_x >= 280 && rel_x <= 300 {
                self.align = 1;
                return;
            }
            if rel_x >= 305 && rel_x <= 325 {
                self.align = 2;
                return;
            }
            return;
        }

        // 4. Module Client Body Clicks (rel_y > 84)
        match self.active_module {
            1 => {
                // Calc grid clicks
                if rel_y >= 108 {
                    let col = ((rel_x - 36) / 80).max(0) as usize;
                    let row = ((rel_y - 128) / 22).max(0) as usize;
                    self.active_cell_col = col.min(7);
                    self.active_cell_row = row.min(19);
                }
            }
            2 => {
                // Present filmstrip clicks on left (rel_x < 110)
                if rel_x <= 110 && rel_y >= 90 {
                    let slide_idx = ((rel_y - 90) / 60).max(0) as usize;
                    self.active_slide = slide_idx.min(2);
                }
            }
            3 => {
                // Base record clicks
                if rel_y >= 120 {
                    let rec_idx = ((rel_y - 120) / 22).max(0) as usize;
                    self.active_table_record = rec_idx.min(3);
                }
            }
            _ => {}
        }
    }
}

pub struct WmTerminal {
    pub lines: [TermLine; TERM_MAX_LINES],
    pub line_count: usize,
    pub input_buf: [u8; TERM_INPUT_LEN],
    pub input_len: usize,
    pub cursor_idx: usize,
    pub blink_tick: u32,
}

impl WmTerminal {
    pub fn new() -> Self {
        let mut term = WmTerminal {
            lines: [TermLine::empty(); TERM_MAX_LINES],
            line_count: 0,
            input_buf: [0; TERM_INPUT_LEN],
            input_len: 0,
            cursor_idx: 0,
            blink_tick: 0,
        };
        term.push_line("NovaOS Terminal (NovaOS 2.0 x86_64 Enterprise)", Color::rgb(88, 166, 255));
        term.push_line("Type 'help' for commands, or explore: neofetch, ls, uname -a", Color::rgb(139, 148, 158));
        term.push_line("", Color::WHITE);
        term
    }

    pub fn push_line(&mut self, text: &str, color: Color) {
        if self.line_count >= TERM_MAX_LINES {
            for i in 0..TERM_MAX_LINES - 1 {
                self.lines[i] = self.lines[i + 1];
            }
            self.lines[TERM_MAX_LINES - 1] = TermLine::from_str(text, color);
        } else {
            self.lines[self.line_count] = TermLine::from_str(text, color);
            self.line_count += 1;
        }
    }

    pub fn handle_key(&mut self, key: i32) {
        let mut default_cfg = TimeConfig::default();
        let default_rtc = ffi::RtcTime::default();
        self.handle_key_with_context(key, &mut default_cfg, default_rtc);
    }

    pub fn handle_key_with_context(&mut self, key: i32, time_cfg: &mut TimeConfig, raw_rtc: ffi::RtcTime) {
        match key {
            10 | 13 => {
                self.execute_command_with_context(time_cfg, raw_rtc);
            }
            8 | 127 => {
                if self.cursor_idx > 0 && self.input_len > 0 {
                    for i in (self.cursor_idx - 1)..(self.input_len - 1) {
                        self.input_buf[i] = self.input_buf[i + 1];
                    }
                    self.input_len -= 1;
                    self.cursor_idx -= 1;
                }
            }
            0x103 => {
                if self.cursor_idx > 0 {
                    self.cursor_idx -= 1;
                }
            }
            0x104 => {
                if self.cursor_idx < self.input_len {
                    self.cursor_idx += 1;
                }
            }
            32..=126 => {
                if self.input_len < TERM_INPUT_LEN - 1 {
                    for i in (self.cursor_idx..self.input_len).rev() {
                        self.input_buf[i + 1] = self.input_buf[i];
                    }
                    self.input_buf[self.cursor_idx] = key as u8;
                    self.input_len += 1;
                    self.cursor_idx += 1;
                }
            }
            _ => {}
        }
    }

    pub fn execute_command(&mut self) {
        let mut default_cfg = TimeConfig::default();
        let default_rtc = ffi::RtcTime::default();
        self.execute_command_with_context(&mut default_cfg, default_rtc);
    }

    pub fn execute_command_with_context(&mut self, time_cfg: &mut TimeConfig, raw_rtc: ffi::RtcTime) {
        let mut cmd_buf = [0u8; TERM_INPUT_LEN];
        let len = self.input_len.min(TERM_INPUT_LEN);
        cmd_buf[..len].copy_from_slice(&self.input_buf[..len]);
        self.input_len = 0;
        self.cursor_idx = 0;

        let input_str = core::str::from_utf8(&cmd_buf[..len]).unwrap_or("");
        let trimmed = input_str.trim();

        // Echo prompt + entered command to terminal output
        let mut echo_buf = [0u8; 128];
        let pfx = b"admin@novaos:~$ ";
        let pfx_len = pfx.len();
        echo_buf[..pfx_len].copy_from_slice(pfx);
        let cmd_bytes = trimmed.as_bytes();
        let copy_cmd = cmd_bytes.len().min(128 - pfx_len);
        echo_buf[pfx_len..pfx_len + copy_cmd].copy_from_slice(&cmd_bytes[..copy_cmd]);
        let echo_str = core::str::from_utf8(&echo_buf[..pfx_len + copy_cmd]).unwrap_or("admin@novaos:~$ ");
        self.push_line(echo_str, Color::rgb(220, 230, 242));

        if trimmed.is_empty() {
            return;
        }

        let mut parts = trimmed.split_whitespace();
        let cmd = parts.next().unwrap_or("");
        let rest = if trimmed.len() > cmd.len() { trimmed[cmd.len()..].trim() } else { "" };

        match cmd {
            "help" => {
                self.push_line("NovaOS Shell v2.0 - Available Commands:", Color::rgb(88, 166, 255));
                self.push_line("  help       - Show this built-in command reference", Color::rgb(220, 230, 242));
                self.push_line("  neofetch   - Display system hardware, OS & memory specs", Color::rgb(220, 230, 242));
                self.push_line("  uname -a   - Print kernel & CPU Long Mode architecture", Color::rgb(220, 230, 242));
                self.push_line("  ls [dir]   - List files in /root or specified directory", Color::rgb(220, 230, 242));
                self.push_line("  cat <file> - Display file contents (e.g. cat HELLO.TXT)", Color::rgb(220, 230, 242));
                self.push_line("  whoami     - Display active logged-in user credentials", Color::rgb(220, 230, 242));
                self.push_line("  date       - Print date/time (flags: --tz, --24h, --12h, -u, --sync)", Color::rgb(220, 230, 242));
                self.push_line("  cal        - Display calendar schedule & upcoming events (--events)", Color::rgb(220, 230, 242));
                self.push_line("  calc <exp> - Evaluate math expression (e.g. calc 125 * 4, calc sqrt(144))", Color::rgb(220, 230, 242));
                self.push_line("  browser    - Launch Aero Web Browser (or 'aero <url>', '--shields')", Color::rgb(220, 230, 242));
                self.push_line("  spectra    - Launch Spectra Media Studio (or 'spectra <file>', '--info')", Color::rgb(220, 230, 242));
                self.push_line("  vellum     - Launch Vellum PDF Studio (or 'vellum <file>', '--info')", Color::rgb(220, 230, 242));
                self.push_line("  vortex     - Launch Vortex BitTorrent & Stream Studio ('vortex <url>', '--list')", Color::rgb(220, 230, 242));
                self.push_line("  zenith     - Launch Zenith Office Suite (Writer, Calc, Present, Base)", Color::rgb(220, 230, 242));
                self.push_line("  echo <msg> - Print message to standard output", Color::rgb(220, 230, 242));
                self.push_line("  clear      - Clear terminal screen back to clean prompt", Color::rgb(220, 230, 242));
                self.push_line("  about      - Information about NovaOS 2.0 release", Color::rgb(220, 230, 242));
            }
            "neofetch" | "fetch" => {
                self.push_line("   /\\_/\\     OS: NovaOS 2.0.0-enterprise x86_64", Color::rgb(88, 166, 255));
                self.push_line("  ( o.o )    Kernel: NovaOS SMP Long Mode (Dual 32/64-bit ABI)", Color::rgb(139, 92, 246));
                self.push_line("   > ^ <     Host: QEMU Virtual Machine | Uptime: 0h 18m", Color::rgb(56, 189, 248));
                self.push_line("             Arch: x86_64 (Intel/AMD Long Mode)", Color::rgb(63, 185, 80));
                self.push_line("             Display: 1024x768 32bpp XRGB Framebuffer", Color::rgb(210, 153, 34));
                self.push_line("             Compositor: NovaCompositor (Fluent + Aqua + GNOME)", Color::rgb(240, 246, 252));
                self.push_line("             Memory: 384 MB / 4096 MB (9%)", Color::rgb(88, 166, 255));
            }
            "uname" => {
                if rest == "-a" || rest.contains("-a") {
                    self.push_line("NovaOS 2.0.0-enterprise x86_64 pc-novaos (SMP 64-bit Long Mode)", Color::rgb(63, 185, 80));
                } else {
                    self.push_line("NovaOS", Color::rgb(63, 185, 80));
                }
            }
            "ls" => {
                self.push_line("HELLO.TXT    (68 B)      Document", Color::rgb(88, 166, 255));
                self.push_line("FILES.ELF    (40 KB)     Executable", Color::rgb(63, 185, 80));
                self.push_line("TERM.ELF     (48 KB)     Executable", Color::rgb(63, 185, 80));
                self.push_line("SETTINGS.ELF (52 KB)     Executable", Color::rgb(63, 185, 80));
                self.push_line("SYSTEM.CFG   (512 B)     Config", Color::rgb(210, 153, 34));
                self.push_line("ACCESS.CFG   (256 B)     Config", Color::rgb(210, 153, 34));
            }
            "whoami" => {
                self.push_line("admin (uid=0 gid=0 groups=0(wheel), 1(audio), 2(video))", Color::rgb(63, 185, 80));
            }
            "date" => {
                if rest.is_empty() {
                    let tz = &TIMEZONES[time_cfg.timezone_idx % TIMEZONES.len()];
                    let dt = calculate_datetime(raw_rtc, tz.offset_minutes + time_cfg.manual_offset_minutes, time_cfg.manual_offset_days);
                    let mut out = [0u8; 96];
                    let mut off = 0;
                    append_str(DAY_NAMES_SHORT[dt.day_of_week as usize % 7], &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    append_str(MONTH_NAMES_SHORT[dt.month.saturating_sub(1) as usize % 12], &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    format_u32_padded(dt.day as u32, 2, &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    format_u32_padded(dt.hour as u32, 2, &mut out, &mut off);
                    append_str(":", &mut out, &mut off);
                    format_u32_padded(dt.minute as u32, 2, &mut out, &mut off);
                    append_str(":", &mut out, &mut off);
                    format_u32_padded(dt.second as u32, 2, &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    append_str(tz.code, &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    format_u32_padded(dt.year as u32, 4, &mut out, &mut off);
                    append_str(" (UTC", &mut out, &mut off);
                    let sign = if tz.offset_minutes >= 0 { "+" } else { "-" };
                    append_str(sign, &mut out, &mut off);
                    let off_h = tz.offset_minutes.abs() / 60;
                    let off_m = tz.offset_minutes.abs() % 60;
                    format_u32_padded(off_h as u32, 2, &mut out, &mut off);
                    append_str(":", &mut out, &mut off);
                    format_u32_padded(off_m as u32, 2, &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    append_str(tz.name, &mut out, &mut off);
                    append_str(")", &mut out, &mut off);
                    let s = core::str::from_utf8(&out[..off]).unwrap_or("");
                    self.push_line(s, Color::rgb(240, 246, 252));
                } else if rest == "-u" || rest == "--utc" {
                    let dt = calculate_datetime(raw_rtc, 0, 0);
                    let mut out = [0u8; 96];
                    let mut off = 0;
                    append_str(DAY_NAMES_SHORT[dt.day_of_week as usize % 7], &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    append_str(MONTH_NAMES_SHORT[dt.month.saturating_sub(1) as usize % 12], &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    format_u32_padded(dt.day as u32, 2, &mut out, &mut off);
                    append_str(" ", &mut out, &mut off);
                    format_u32_padded(dt.hour as u32, 2, &mut out, &mut off);
                    append_str(":", &mut out, &mut off);
                    format_u32_padded(dt.minute as u32, 2, &mut out, &mut off);
                    append_str(":", &mut out, &mut off);
                    format_u32_padded(dt.second as u32, 2, &mut out, &mut off);
                    append_str(" UTC ", &mut out, &mut off);
                    format_u32_padded(dt.year as u32, 4, &mut out, &mut off);
                    let s = core::str::from_utf8(&out[..off]).unwrap_or("");
                    self.push_line(s, Color::rgb(240, 246, 252));
                } else if rest == "--24h" {
                    time_cfg.is_24h = true;
                    self.push_line("[OK] Switched system clock to 24-hour mode (HH:MM).", Color::rgb(63, 185, 80));
                } else if rest == "--12h" {
                    time_cfg.is_24h = false;
                    self.push_line("[OK] Switched system clock to 12-hour mode (hh:mm AM/PM).", Color::rgb(63, 185, 80));
                } else if rest == "--sync" {
                    time_cfg.manual_offset_minutes = 0;
                    time_cfg.manual_offset_days = 0;
                    self.push_line("[OK] System time synchronized with hardware CMOS RTC.", Color::rgb(63, 185, 80));
                } else if rest.starts_with("--tz") {
                    let arg = rest[4..].trim();
                    if arg.is_empty() || arg == "list" {
                        self.push_line("NovaOS World Timezones (22 International Regions):", Color::rgb(88, 166, 255));
                        for (idx, tz) in TIMEZONES.iter().enumerate() {
                            let mut buf = [0u8; 96];
                            let mut off = 0;
                            append_str("  [", &mut buf, &mut off);
                            format_u32(idx as u32, &mut buf, &mut off);
                            append_str("] ", &mut buf, &mut off);
                            append_str(tz.name, &mut buf, &mut off);
                            append_str(" (", &mut buf, &mut off);
                            append_str(tz.code, &mut buf, &mut off);
                            append_str(", UTC", &mut buf, &mut off);
                            let sign = if tz.offset_minutes >= 0 { "+" } else { "-" };
                            append_str(sign, &mut buf, &mut off);
                            format_u32((tz.offset_minutes.abs() / 60) as u32, &mut buf, &mut off);
                            let off_m = tz.offset_minutes.abs() % 60;
                            if off_m != 0 {
                                append_str(":", &mut buf, &mut off);
                                format_u32_padded(off_m as u32, 2, &mut buf, &mut off);
                            }
                            append_str(") - ", &mut buf, &mut off);
                            append_str(tz.city_country, &mut buf, &mut off);
                            if idx == 0 {
                                append_str(" [Default]", &mut buf, &mut off);
                            }
                            let s = core::str::from_utf8(&buf[..off]).unwrap_or("");
                            let col = if idx == time_cfg.timezone_idx {
                                Color::rgb(56, 189, 248)
                            } else {
                                Color::rgb(220, 230, 242)
                            };
                            self.push_line(s, col);
                        }
                        self.push_line("Run 'date --tz <name>' to switch timezone.", Color::rgb(139, 148, 158));
                    } else {
                        let mut found = None;
                        for (idx, tz) in TIMEZONES.iter().enumerate() {
                            if contains_ignore_ascii_case(tz.name, arg)
                                || contains_ignore_ascii_case(tz.city_country, arg)
                                || contains_ignore_ascii_case(tz.code, arg)
                            {
                                found = Some(idx);
                                break;
                            }
                        }
                        if let Some(idx) = found {
                            time_cfg.timezone_idx = idx;
                            let tz = &TIMEZONES[idx];
                            let mut buf = [0u8; 96];
                            let mut off = 0;
                            append_str("[OK] Timezone set to ", &mut buf, &mut off);
                            append_str(tz.name, &mut buf, &mut off);
                            append_str(" (", &mut buf, &mut off);
                            append_str(tz.code, &mut buf, &mut off);
                            append_str(") - ", &mut buf, &mut off);
                            append_str(tz.city_country, &mut buf, &mut off);
                            let s = core::str::from_utf8(&buf[..off]).unwrap_or("");
                            self.push_line(s, Color::rgb(63, 185, 80));
                        } else {
                            let mut buf = [0u8; 96];
                            let mut off = 0;
                            append_str("Error: timezone '", &mut buf, &mut off);
                            let rlen = arg.as_bytes().len().min(30);
                            append_str(&arg[..rlen], &mut buf, &mut off);
                            append_str("' not found. Run 'date --tz list' for all timezones.", &mut buf, &mut off);
                            let s = core::str::from_utf8(&buf[..off]).unwrap_or("");
                            self.push_line(s, Color::rgb(248, 81, 73));
                        }
                    }
                } else if rest == "--help" || rest == "-h" {
                    self.push_line("Usage: date [OPTIONS]", Color::rgb(88, 166, 255));
                    self.push_line("  date              Display wall-clock date and time in active timezone", Color::rgb(220, 230, 242));
                    self.push_line("  date -u, --utc    Display time in Coordinated Universal Time (UTC)", Color::rgb(220, 230, 242));
                    self.push_line("  date --24h        Switch clock display to 24-hour mode (HH:MM)", Color::rgb(220, 230, 242));
                    self.push_line("  date --12h        Switch clock display to 12-hour mode (hh:mm AM/PM)", Color::rgb(220, 230, 242));
                    self.push_line("  date --tz list    List all 22 supported international timezones", Color::rgb(220, 230, 242));
                    self.push_line("  date --tz <name>  Switch timezone (e.g. date --tz tokyo)", Color::rgb(220, 230, 242));
                    self.push_line("  date --sync       Reset manual offset and sync with CMOS RTC", Color::rgb(220, 230, 242));
                } else {
                    let mut buf = [0u8; 96];
                    let mut off = 0;
                    append_str("date: unrecognized option '", &mut buf, &mut off);
                    let rlen = rest.as_bytes().len().min(30);
                    append_str(&rest[..rlen], &mut buf, &mut off);
                    append_str("'. Try 'date --help'.", &mut buf, &mut off);
                    let s = core::str::from_utf8(&buf[..off]).unwrap_or("");
                    self.push_line(s, Color::rgb(248, 81, 73));
                }
            }
            "cal" | "calendar" => {
                if rest.is_empty() || rest == "--month" {
                    self.push_line("    November 2026", Color::rgb(0, 212, 255));
                    self.push_line("Su Mo Tu We Th Fr Sa", Color::rgb(139, 148, 158));
                    self.push_line(" 1  2  3  4  5  6  7", Color::rgb(220, 230, 242));
                    self.push_line(" 8  9 10 11 12 13 14", Color::rgb(220, 230, 242));
                    self.push_line("15 16 17 18 19 20 21", Color::rgb(240, 246, 252));
                    self.push_line("22 23 24 25 26 27 28", Color::rgb(220, 230, 242));
                    self.push_line("29 30               ", Color::rgb(220, 230, 242));
                    self.push_line("Today: Nov 19, 2026 (4 events scheduled)", Color::rgb(63, 185, 80));
                } else if rest == "--events" || rest == "-e" {
                    self.push_line("NovaOS Calendar - Scheduled Events:", Color::rgb(88, 166, 255));
                    self.push_line("  * Nov 19, 09:30 AM [Work] Daily Architecture Standup", Color::rgb(239, 68, 68));
                    self.push_line("  * Nov 19, 02:00 PM [NovaOS] NovaOS 2.0 GA Release Launch", Color::rgb(139, 92, 246));
                    self.push_line("  * Nov 19, 07:30 PM [Personal] Team Celebration Dinner", Color::rgb(16, 185, 129));
                    self.push_line("  * Nov 19, 11:00 PM [Reminders] Backup Cloud Snapshot Sync", Color::rgb(14, 165, 233));
                    self.push_line("  * Nov 25, 10:00 AM [Work] Security Architecture Review", Color::rgb(239, 68, 68));
                    self.push_line("  * Nov 26, All Day  [Holidays] Thanksgiving Holiday", Color::rgb(245, 158, 11));
                } else {
                    self.push_line("Usage: cal [--month | --events]", Color::rgb(88, 166, 255));
                }
            }
            "calc" | "calculator" => {
                if rest.is_empty() || rest == "--help" || rest == "-h" {
                    self.push_line("NovaOS Calculator (nova-calc) v2.0 - Math & Science Studio", Color::rgb(88, 166, 255));
                    self.push_line("Usage: calc <expr>  (e.g. 'calc 125 * 4', 'calc 2 ^ 8', 'calc sqrt(144)')", Color::rgb(220, 230, 242));
                    self.push_line("Supported ops: +, -, *, /, %, ^, sqrt(), abs()", Color::rgb(139, 148, 158));
                } else {
                    let trimmed_expr = rest.trim();
                    let mut evaluated = None;

                    if trimmed_expr.starts_with("sqrt(") && trimmed_expr.ends_with(')') {
                        let inner = &trimmed_expr[5..trimmed_expr.len() - 1].trim();
                        let v = parse_f64_from(inner.as_bytes());
                        if v >= 0.0 {
                            evaluated = Some(fast_sqrt64(v));
                        }
                    } else if trimmed_expr.starts_with("abs(") && trimmed_expr.ends_with(')') {
                        let inner = &trimmed_expr[4..trimmed_expr.len() - 1].trim();
                        let v = parse_f64_from(inner.as_bytes());
                        evaluated = Some(if v < 0.0 { -v } else { v });
                    } else {
                        let bytes = trimmed_expr.as_bytes();
                        let mut op_pos = None;
                        let mut found_op = 0u8;

                        for &target in &[b'+', b'-', b'*', b'/', b'%', b'^'] {
                            for (idx, &b) in bytes.iter().enumerate() {
                                if b == target && idx > 0 {
                                    op_pos = Some(idx);
                                    found_op = target;
                                    break;
                                }
                            }
                            if op_pos.is_some() {
                                break;
                            }
                        }

                        if let Some(pos) = op_pos {
                            let left_str = trimmed_expr[..pos].trim();
                            let right_str = trimmed_expr[pos + 1..].trim();
                            let left = parse_f64_from(left_str.as_bytes());
                            let right = parse_f64_from(right_str.as_bytes());

                            match found_op {
                                b'+' => evaluated = Some(left + right),
                                b'-' => evaluated = Some(left - right),
                                b'*' => evaluated = Some(left * right),
                                b'/' => {
                                    if right.abs() > 1e-12 {
                                        evaluated = Some(left / right);
                                    }
                                }
                                b'%' => {
                                    if right.abs() > 1e-12 {
                                        evaluated = Some(((left as i64) % (right as i64)) as f64);
                                    }
                                }
                                b'^' => {
                                    let exp = right as i32;
                                    let mut r = 1.0;
                                    for _ in 0..exp {
                                        r *= left;
                                    }
                                    evaluated = Some(r);
                                }
                                _ => {}
                            }
                        } else {
                            if trimmed_expr == "pi" || trimmed_expr == "PI" {
                                evaluated = Some(3.14159);
                            } else if trimmed_expr == "e" || trimmed_expr == "E" {
                                evaluated = Some(2.71828);
                            } else {
                                evaluated = Some(parse_f64_from(bytes));
                            }
                        }
                    }

                    if let Some(val) = evaluated {
                        let mut res_buf = [0u8; 48];
                        let mut off = 0;
                        append_str("= ", &mut res_buf, &mut off);
                        format_f64_into(val, &mut res_buf, &mut off);
                        let s = core::str::from_utf8(&res_buf[..off]).unwrap_or("= 0");
                        self.push_line(s, Color::rgb(63, 185, 80));
                    } else {
                        self.push_line("calc: math error or division by zero", Color::rgb(248, 81, 73));
                    }
                }
            }
            "browser" | "aero" => {
                if rest.is_empty() || rest == "--help" || rest == "-h" {
                    self.push_line("Aero Browser v1.0.0 (Pure Rust Engine)", Color::rgb(255, 107, 0));
                    self.push_line("NovaOS Default Web Browser: Zero Chromium/WebKit, Brave Shields, Multi-Tab", Color::rgb(88, 166, 255));
                    self.push_line("Usage: browser [url | --shields | --tabs | --version]", Color::rgb(220, 230, 242));
                    self.push_line("Active URL: about:newtab | Shields: Active (14 trackers blocked)", Color::rgb(63, 185, 80));
                } else if rest == "--shields" || rest == "-s" {
                    self.push_line("Aero Privacy Shields Telemetry:", Color::rgb(255, 107, 0));
                    self.push_line("  * Shield Status: UP (All protections active)", Color::rgb(63, 185, 80));
                    self.push_line("  * Trackers & Ads Blocked: 14", Color::rgb(56, 189, 248));
                    self.push_line("  * HTTPS Upgrades Enforced: 6", Color::rgb(74, 222, 128));
                    self.push_line("  * Bandwidth Saved: 42.8 KB", Color::rgb(88, 166, 255));
                } else if rest == "--version" || rest == "-v" {
                    self.push_line("Aero Browser 1.0.0 (pc-novaos-elf, Pure Rust Engine)", Color::rgb(88, 166, 255));
                    self.push_line("Architecture: aero-engine + aero-ui (Brave + Chrome + Firefox synthesis)", Color::rgb(220, 230, 242));
                } else {
                    self.push_line("Navigating Aero Browser to:", Color::rgb(88, 166, 255));
                    self.push_line(rest, Color::rgb(63, 185, 80));
                    self.push_line("[OK] 200 OK - Aero Engine rendered frame (Shields: Protected)", Color::rgb(240, 246, 252));
                }
            }
            "spectra" | "player" | "media" | "vlc" => {
                if rest.is_empty() || rest == "--help" || rest == "-h" {
                    self.push_line("Spectra v1.0.0 (Pure Rust Media Engine)", Color::rgb(56, 189, 248));
                    self.push_line("NovaOS Default Media Player: 4K UHD, 8K HDR, 32-Band Equalizer", Color::rgb(88, 166, 255));
                    self.push_line("Usage: spectra [file | --info | --visualizer | --version]", Color::rgb(220, 230, 242));
                    self.push_line("Active Media: video/synthetic_skyline_4k.mkv (4K UHD 60 FPS)", Color::rgb(63, 185, 80));
                } else if rest == "--info" || rest == "-i" {
                    self.push_line("Spectra Media Telemetry & Codec Inspector:", Color::rgb(56, 189, 248));
                    self.push_line("  * Video Pipeline: 4K UHD (3840x2160) @ 60 FPS", Color::rgb(63, 185, 80));
                    self.push_line("  * Hardware Scaling: Bilinear Multi-Resolution Letterbox", Color::rgb(56, 189, 248));
                    self.push_line("  * Codecs: H.264/AVC, AV1 8K, VP9, FLAC 24/96, MP3 320k", Color::rgb(74, 222, 128));
                    self.push_line("  * Audio Equalizer: 32-Band Dynamic Spectrum (Boost: 125%)", Color::rgb(88, 166, 255));
                } else if rest == "--version" || rest == "-v" {
                    self.push_line("Spectra 1.0.0 (pc-novaos-elf, Pure Rust Engine)", Color::rgb(88, 166, 255));
                    self.push_line("Architecture: spectra-core + spectra-ui (Zero C/C++, Zero FFmpeg)", Color::rgb(220, 230, 242));
                } else {
                    self.push_line("Loading media into Spectra:", Color::rgb(88, 166, 255));
                    self.push_line(rest, Color::rgb(63, 185, 80));
                    self.push_line("[OK] Stream format demuxed: 4K UHD @ 60 FPS (Audio: Stereo PCM 48kHz)", Color::rgb(240, 246, 252));
                }
            }
            "vellum" | "pdf" | "reader" => {
                if rest.is_empty() || rest == "--help" || rest == "-h" {
                    self.push_line("Vellum PDF Studio v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248));
                    self.push_line("NovaOS Default PDF Reader, Editor & Authoring Suite", Color::rgb(88, 166, 255));
                    self.push_line("Usage: vellum [file.pdf | --info | --extract-text | --version]", Color::rgb(220, 230, 242));
                    self.push_line("Active Doc: docs/architecture_manual.pdf (3 pages, PDF 1.7)", Color::rgb(63, 185, 80));
                } else if rest == "--info" || rest == "-i" {
                    self.push_line("Vellum Document Telemetry & PDF Inspector:", Color::rgb(56, 189, 248));
                    self.push_line("  * Document: NovaOS Architecture & Systems (3 pages)", Color::rgb(63, 185, 80));
                    self.push_line("  * Geometry: 612 x 792 pt (US Letter Standard)", Color::rgb(56, 189, 248));
                    self.push_line("  * Compression: RFC 1950/1951 Deflate/Inflate stream codec", Color::rgb(74, 222, 128));
                    self.push_line("  * Capabilities: In-place text edit, Annotations, Forms, Outlines", Color::rgb(88, 166, 255));
                } else if rest == "--extract-text" || rest == "-e" {
                    self.push_line("=== Extracted Text Streams (Vellum Core) ===", Color::rgb(56, 189, 248));
                    self.push_line("NovaOS Technical Report: Architecture & Systems", Color::rgb(240, 246, 252));
                    self.push_line("Vellum Studio -- Next-Generation Document Processing in Pure Rust", Color::rgb(220, 230, 242));
                    self.push_line("1. Executive Overview: Sovereign Rust desktop environment.", Color::rgb(63, 185, 80));
                } else if rest == "--version" || rest == "-v" {
                    self.push_line("Vellum PDF Studio 1.0.0 (pc-novaos-elf, Pure Rust 1.99.1)", Color::rgb(88, 166, 255));
                    self.push_line("Architecture: vellum-core + vellum-ui (Adobe + Preview + Evince synthesis)", Color::rgb(220, 230, 242));
                } else {
                    self.push_line("Opening document in Vellum Studio:", Color::rgb(88, 166, 255));
                    self.push_line(rest, Color::rgb(63, 185, 80));
                    self.push_line("[OK] PDF 1.7 Parsed - Display list generated (18 text spans)", Color::rgb(240, 246, 252));
                }
            }
            "vortex" | "torrent" | "idm" => {
                if rest.is_empty() || rest == "--help" || rest == "-h" {
                    self.push_line("Vortex Studio v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248));
                    self.push_line("Unified BitTorrent Client & Intelligent Stream Accelerator", Color::rgb(88, 166, 255));
                    self.push_line("Usage: vortex [file.torrent | magnet:? | url | --list | --sniff | --info | --version]", Color::rgb(220, 230, 242));
                    self.push_line("App Store: Registered in nova-store (Internet Category) | Status: Available", Color::rgb(63, 185, 80));
                } else if rest == "--list" || rest == "-l" {
                    self.push_line("Vortex Active Queue (4 tasks running):", Color::rgb(56, 189, 248));
                    self.push_line("  * [TORRENT] NovaOS-2.0-Enterprise.iso  74.0% | 14.1 MB/s | 54 Peers", Color::rgb(63, 185, 80));
                    self.push_line("  * [STREAM]  YouTube: NovaOS Kernel [FHD] 62.3% | 17.5 MB/s | 8 Streams", Color::rgb(251, 146, 60));
                    self.push_line("  * [TORRENT] Linux-Kernel-6.12.tar.xz  100.0% | Seeding   | 120 Peers", Color::rgb(56, 189, 248));
                    self.push_line("  * [STREAM]  Vimeo: Rust Concurrency   100.0% | Complete  | 4 Streams", Color::rgb(74, 222, 128));
                    self.push_line("Swarm Aggregation: Download: 31.6 MB/s | Seeding: 3.4 MB/s", Color::rgb(240, 246, 252));
                } else if rest == "--sniff" || rest == "-s" {
                    self.push_line("[*] Browser Video Sniffer: Active (Aero Browser Bridge)", Color::rgb(56, 189, 248));
                    self.push_line("[+] Detected Web Stream: YouTube Video (MPEG-4 AVC)", Color::rgb(63, 185, 80));
                    self.push_line("  * 1080p Full HD (88.4 MB) -> Ready to accelerate (8 parallel chunks)", Color::rgb(220, 230, 242));
                    self.push_line("  * 720p High Def (42.2 MB) -> Ready to accelerate (4 chunks)", Color::rgb(220, 230, 242));
                    self.push_line("  * Audio MP3 Only (6.4 MB) -> Direct download stream", Color::rgb(220, 230, 242));
                    self.push_line("[+] Pop-up Floating Badge: Injected beside video element.", Color::rgb(74, 222, 128));
                } else if rest == "--info" || rest == "-i" {
                    self.push_line("Vortex BitTorrent Telemetry & Swarm Inspector:", Color::rgb(56, 189, 248));
                    self.push_line("  * Swarm Target: NovaOS 2.0 Enterprise Desktop (x86_64)", Color::rgb(63, 185, 80));
                    self.push_line("  * Info Hash: 2fd4e1c67a2d28fced849ee1bb76e7391b93eb12", Color::rgb(56, 189, 248));
                    self.push_line("  * Piece Map: 962 / 1,300 pieces verified (74.0%)", Color::rgb(74, 222, 128));
                    self.push_line("  * Acceleration: IDM-style 16-channel range splitting active", Color::rgb(88, 166, 255));
                } else if rest == "--version" || rest == "-v" {
                    self.push_line("Vortex Studio 1.0.0 (pc-novaos-elf, Pure Rust 1.99.1)", Color::rgb(88, 166, 255));
                    self.push_line("Architecture: Transmission/uTorrent Swarm + IDM Range Accelerator", Color::rgb(220, 230, 242));
                } else {
                    self.push_line("Opening target in Vortex Studio:", Color::rgb(88, 166, 255));
                    self.push_line(rest, Color::rgb(63, 185, 80));
                    self.push_line("[OK] Swarm / Stream initialized (16 parallel chunk streams allocated)", Color::rgb(240, 246, 252));
                }
            }
            "zenith" | "office" | "writer" | "sheets" | "present" | "slides" | "base" | "excel" | "word" | "powerpoint" => {
                if rest.is_empty() || rest == "--help" || rest == "-h" {
                    self.push_line("Zenith Office Suite v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248));
                    self.push_line("NovaOS Default Office Suite: Writer, Calc, Present, Base", Color::rgb(88, 166, 255));
                    self.push_line("Usage: zenith [doc.znt | --writer | --calc | --present | --base | --fonts | --version]", Color::rgb(220, 230, 242));
                    self.push_line("Synthesis: MS Office (Word/Excel/PowerPoint/Access) + LibreOffice + Docs", Color::rgb(63, 185, 80));
                } else if rest == "--writer" || rest == "-w" {
                    self.push_line("Zenith Writer: Professional Word Processor", Color::rgb(0, 120, 212));
                    self.push_line("  * Document: NovaOS Executive Strategy (6 paragraphs, 1,420 words)", Color::rgb(63, 185, 80));
                    self.push_line("  * Font Family: Calibri / Times New Roman / Liberation Serif", Color::rgb(220, 230, 242));
                    self.push_line("  * Features: Headings H1-H6, Bullet Lists, Tables, Multi-Margin Layout", Color::rgb(74, 222, 128));
                } else if rest == "--calc" || rest == "-c" {
                    self.push_line("Zenith Calc: Dynamic Financial Spreadsheet", Color::rgb(16, 124, 65));
                    self.push_line("  * Workbook: FY2026 Fiscal Forecast (3 sheets, 20 active cells)", Color::rgb(63, 185, 80));
                    self.push_line("  * Formula Engine: SUM, AVERAGE, COUNT, MIN, MAX, IF, arithmetic", Color::rgb(74, 222, 128));
                    self.push_line("  * Status: Cell C5 = SUM(B2:B5) => $145,200 (Auto-recalculated)", Color::rgb(88, 166, 255));
                } else if rest == "--present" || rest == "-p" {
                    self.push_line("Zenith Present: 16:9 Presentation Studio", Color::rgb(216, 59, 1));
                    self.push_line("  * Slide Deck: NovaOS Keynote 2026 (3 widescreen slides)", Color::rgb(63, 185, 80));
                    self.push_line("  * Layouts: Title, Title & Content, Comparison Cards, Two Columns", Color::rgb(220, 230, 242));
                    self.push_line("  * Notes: Bottom speaker notes synchronized with slide transitions", Color::rgb(74, 222, 128));
                } else if rest == "--base" || rest == "-b" {
                    self.push_line("Zenith Base: Relational Database & Forms Engine", Color::rgb(168, 0, 0));
                    self.push_line("  * Database: ERP Enterprise Records (Tables: Customers, Orders)", Color::rgb(63, 185, 80));
                    self.push_line("  * Types: Integer, VarChar, Float, Date, Boolean (Primary Keys)", Color::rgb(220, 230, 242));
                    self.push_line("  * Query Runner: SELECT * FROM Customers WHERE Id = 1 => 1 row", Color::rgb(74, 222, 128));
                } else if rest == "--fonts" || rest == "-f" {
                    self.push_line("Zenith Typography: 11 Embedded Font Families", Color::rgb(56, 189, 248));
                    self.push_line("  * Microsoft: Calibri, Times New Roman, Arial, Segoe UI, Cambria, Consolas, Georgia", Color::rgb(63, 185, 80));
                    self.push_line("  * LibreOffice & Linux: Liberation Sans, Liberation Serif, Liberation Mono, Ubuntu", Color::rgb(74, 222, 128));
                } else if rest == "--version" || rest == "-v" {
                    self.push_line("Zenith Office Suite 1.0.0 (pc-novaos-elf, Pure Rust 1.99.1)", Color::rgb(88, 166, 255));
                    self.push_line("Architecture: zenith-core + zenith-ui + zenith-app (Zero C/C++)", Color::rgb(220, 230, 242));
                } else {
                    self.push_line("Opening document in Zenith Office:", Color::rgb(88, 166, 255));
                    self.push_line(rest, Color::rgb(63, 185, 80));
                    self.push_line("[OK] Zenith Document format recognized - Module initialized", Color::rgb(240, 246, 252));
                }
            }
            "echo" => {
                self.push_line(rest, Color::rgb(240, 246, 252));
            }
            "cat" => {
                if rest == "HELLO.TXT" || rest.ends_with("HELLO.TXT") {
                    self.push_line("Welcome to NovaOS 2.0 Enterprise Edition!", Color::rgb(63, 185, 80));
                    self.push_line("A modern, safe, high-performance operating system written in Rust and C.", Color::rgb(220, 230, 242));
                } else if rest.is_empty() {
                    self.push_line("cat: missing filename. Usage: cat <file>", Color::rgb(248, 81, 73));
                } else {
                    let mut buf = [0u8; 96];
                    let p1 = b"cat: ";
                    buf[..5].copy_from_slice(p1);
                    let r_bytes = rest.as_bytes();
                    let rlen = r_bytes.len().min(30);
                    buf[5..5 + rlen].copy_from_slice(&r_bytes[..rlen]);
                    let p2 = b": No such file";
                    buf[5 + rlen..5 + rlen + p2.len()].copy_from_slice(p2);
                    let s = core::str::from_utf8(&buf[..5 + rlen + p2.len()]).unwrap_or("cat: not found");
                    self.push_line(s, Color::rgb(248, 81, 73));
                }
            }
            "clear" => {
                self.line_count = 0;
            }
            "about" | "ver" | "version" => {
                self.push_line("NovaOS v2.0 GA (General Availability)", Color::rgb(88, 166, 255));
                self.push_line("Combining Windows 11 Fluent, macOS Aqua & Ubuntu GNOME UI.", Color::rgb(220, 230, 242));
            }
            _ => {
                let mut err = [0u8; 96];
                let p1 = b"novash: command not found: ";
                err[..p1.len()].copy_from_slice(p1);
                let c_bytes = cmd.as_bytes();
                let clen = c_bytes.len().min(40);
                err[p1.len()..p1.len() + clen].copy_from_slice(&c_bytes[..clen]);
                let p2 = b". Type 'help' for list.";
                let total = (p1.len() + clen + p2.len()).min(96);
                err[p1.len() + clen..total].copy_from_slice(&p2[..total - (p1.len() + clen)]);
                let err_str = core::str::from_utf8(&err[..total]).unwrap_or("novash: command not found");
                self.push_line(err_str, Color::rgb(248, 81, 73));
            }
        }
    }
}

pub struct Wm {
    pub screen_w: i32,
    pub screen_h: i32,
    pub titlebar_h: i32,
    pub taskbar_h: i32,
    pub menubar_h: i32,
    pub desktop_h: i32,
    pub windows: [Window; MAX_WINDOWS],
    pub z_order: [usize; MAX_WINDOWS],
    pub cursor_x: i32,
    pub cursor_y: i32,
    pub dragging: Option<usize>,
    pub drag_offset_x: i32,
    pub drag_offset_y: i32,
    pub prev_left_button: bool,
    pub cursor_theme: CursorTheme,
    pub active_menu: Option<usize>,
    pub hovered_menu: Option<usize>,
    pub hovered_item: Option<usize>,
    pub last_menu_event: Option<MenuClickEvent>,
    pub dock_apps: [DockApp; MAX_DOCK_ITEMS],
    pub dock_count: usize,
    pub hovered_dock_item: Option<usize>,
    pub last_dock_event: Option<DockEvent>,
    pub access_cfg: AccessibilityConfig,
    pub preview_snap: Option<SnapState>,
    pub start_menu_open: bool,
    pub hovered_start_item: Option<usize>,
    pub hovered_desktop_icon: Option<usize>,
    pub term: WmTerminal,
    pub time_cfg: TimeConfig,
    pub raw_rtc: ffi::RtcTime,
    pub calendar_open: bool,
    pub calendar_app: WmCalendarApp,
    pub calc_app: WmCalcApp,
    pub browser_app: WmBrowserApp,
    pub player_app: WmPlayerApp,
    pub vellum_app: WmVellumApp,
    pub vortex_app: WmVortexApp,
    pub zenith_app: WmZenithApp,
}

impl Wm {
    /// Constructs default WM geometry matching the 320x200 layout,
    /// ensuring deterministic compatibility with automated self-tests.
    pub fn new() -> Self {
        let default_apps = [
            DEFAULT_PINNED_APPS[0],
            DEFAULT_PINNED_APPS[1],
            DEFAULT_PINNED_APPS[2],
            DEFAULT_PINNED_APPS[3],
            DEFAULT_PINNED_APPS[4],
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
        ];
        Wm {
            screen_w: DEFAULT_SCREEN_W,
            screen_h: DEFAULT_SCREEN_H,
            titlebar_h: DEFAULT_TITLEBAR_H,
            taskbar_h: DEFAULT_TASKBAR_H,
            menubar_h: 0,
            desktop_h: DEFAULT_DESKTOP_H,
            windows: [
                Window::from_vga(20, 20, 100, 70, 3, 1, 1),
                Window::from_vga(140, 30, 100, 70, 10, 2, 2),
                Window::from_vga(60, 90, 100, 70, 14, 6, 3),
                Window::from_vga(170, 100, 90, 60, 12, 5, 4),
            ],
            z_order: [0, 1, 2, 3],
            cursor_x: DEFAULT_SCREEN_W / 2,
            cursor_y: DEFAULT_SCREEN_H / 2,
            dragging: None,
            drag_offset_x: 0,
            drag_offset_y: 0,
            prev_left_button: false,
            cursor_theme: CursorTheme::Classic,
            active_menu: None,
            hovered_menu: None,
            hovered_item: None,
            last_menu_event: None,
            dock_apps: default_apps,
            dock_count: 4,
            hovered_dock_item: None,
            last_dock_event: None,
            access_cfg: AccessibilityConfig::new(),
            preview_snap: None,
            start_menu_open: false,
            hovered_start_item: None,
            hovered_desktop_icon: None,
            term: WmTerminal::new(),
            time_cfg: TimeConfig::default(),
            raw_rtc: ffi::RtcTime::default(),
            calendar_open: false,
            calendar_app: WmCalendarApp::new(),
            calc_app: WmCalcApp::new(),
            browser_app: WmBrowserApp::new(),
            player_app: WmPlayerApp::new(),
            vellum_app: WmVellumApp::new(),
            vortex_app: WmVortexApp::new(),
            zenith_app: WmZenithApp::new(),
        }
    }

    /// Constructs WM layout scaled for high-resolution 32-bit linear framebuffers.
    pub fn new_for_display(width: i32, height: i32) -> Self {
        let taskbar_h = if height >= 600 { FB_TASKBAR_H } else { DEFAULT_TASKBAR_H };
        let titlebar_h = if height >= 600 { FB_TITLEBAR_H } else { DEFAULT_TITLEBAR_H };
        let menubar_h = if height >= 600 { FB_MENUBAR_H } else { 0 };
        let desktop_h = height - taskbar_h;

        let palette = Theme::Dark.palette();

        let (win_w, win_h) = if width >= 800 { (360, 250) } else { (160, 110) };
        let w0 = Window::new(40, 40, win_w, win_h, palette.surface_raised, palette.titlebar_active, 1);
        let w1 = Window::new(440, 60, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 2);
        let w2 = Window::new(120, 330, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 3);
        let w3 = Window::new(520, 350, win_w, win_h, palette.surface_raised, palette.titlebar_inactive, 4);

        let default_apps = [
            DEFAULT_PINNED_APPS[0],
            DEFAULT_PINNED_APPS[1],
            DEFAULT_PINNED_APPS[2],
            DEFAULT_PINNED_APPS[3],
            DEFAULT_PINNED_APPS[4],
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
            DockApp::new(0, "", "", false),
        ];

        let dock_count = if height >= 600 { 5 } else { 4 };

        Wm {
            screen_w: width,
            screen_h: height,
            titlebar_h,
            taskbar_h,
            menubar_h,
            desktop_h,
            windows: [w0, w1, w2, w3],
            z_order: [0, 1, 2, 3],
            cursor_x: width / 2,
            cursor_y: height / 2,
            dragging: None,
            drag_offset_x: 0,
            drag_offset_y: 0,
            prev_left_button: false,
            cursor_theme: CursorTheme::Classic,
            active_menu: None,
            hovered_menu: None,
            hovered_item: None,
            last_menu_event: None,
            dock_apps: default_apps,
            dock_count,
            hovered_dock_item: None,
            last_dock_event: None,
            access_cfg: AccessibilityConfig::new(),
            preview_snap: None,
            start_menu_open: false,
            hovered_start_item: None,
            hovered_desktop_icon: None,
            term: WmTerminal::new(),
            time_cfg: TimeConfig::default(),
            raw_rtc: ffi::RtcTime::default(),
            calendar_open: false,
            calendar_app: WmCalendarApp::new(),
            calc_app: WmCalcApp::new(),
            browser_app: WmBrowserApp::new(),
            player_app: WmPlayerApp::new(),
            vellum_app: WmVellumApp::new(),
            vortex_app: WmVortexApp::new(),
            zenith_app: WmZenithApp::new(),
        }
    }

    /// Sets the desktop theme and updates cursor and window chrome accordingly.
    pub fn set_theme(&mut self, theme: Theme) {
        self.access_cfg.theme = theme;
        self.cursor_theme = self.access_cfg.effective_cursor_theme();
        if theme.is_high_contrast() {
            for w in self.windows.iter_mut() {
                w.body_color = Color::BLACK;
                w.titlebar_color = Color::WHITE;
            }
        } else {
            let pal = theme.palette();
            for (idx, w) in self.windows.iter_mut().enumerate() {
                w.body_color = pal.surface_raised;
                w.titlebar_color = if idx == 0 { pal.titlebar_active } else { pal.titlebar_inactive };
            }
        }
    }

    /// Updates the global font scale multiplier and recalibrates chrome geometry.
    pub fn set_font_scale(&mut self, scale: FontScale) {
        self.access_cfg.font_scale = scale;
        let base_tb = if self.screen_h >= 600 { FB_TITLEBAR_H } else { DEFAULT_TITLEBAR_H };
        let base_mb = if self.screen_h >= 600 { FB_MENUBAR_H } else { 0 };
        let base_task = if self.screen_h >= 600 { FB_TASKBAR_H } else { DEFAULT_TASKBAR_H };
        self.titlebar_h = self.access_cfg.titlebar_height(base_tb);
        self.menubar_h = self.access_cfg.menubar_height(base_mb);
        self.taskbar_h = self.access_cfg.taskbar_height(base_task);
        self.desktop_h = self.screen_h - self.taskbar_h;
    }

    /// Applies an entire accessibility configuration to the window manager.
    pub fn apply_accessibility_config(&mut self, cfg: AccessibilityConfig) {
        self.access_cfg = cfg;
        self.cursor_theme = cfg.effective_cursor_theme();
        self.set_theme(cfg.theme);
        self.set_font_scale(cfg.font_scale);
    }

    /// Returns the active desktop work area respecting top menubars and bottom taskbars.
    pub fn work_area(&self) -> WorkArea {
        let top = self.menubar_h;
        let bottom = self.desktop_h;
        let h = if bottom > top { bottom - top } else { self.screen_h };
        WorkArea {
            x: 0,
            y: top,
            w: self.screen_w,
            h,
        }
    }

    /// Primary monitor boundary descriptor for multi-monitor geometry queries.
    pub fn primary_monitor(&self) -> MonitorGeometry {
        MonitorGeometry {
            x: 0,
            y: 0,
            w: self.screen_w,
            h: self.screen_h,
            is_primary: true,
        }
    }

    /// Resolves which monitor contains the given screen coordinates.
    pub fn monitor_for_point(&self, _px: i32, _py: i32) -> MonitorGeometry {
        self.primary_monitor()
    }

    /// Computes the usable work area on a given monitor.
    pub fn work_area_for_monitor(&self, mon: MonitorGeometry) -> WorkArea {
        if mon.is_primary {
            self.work_area()
        } else {
            WorkArea::new(mon.x, mon.y, mon.w, mon.h)
        }
    }

    /// Active pixel threshold for screen edge snap triggering.
    pub fn snap_edge_margin(&self) -> i32 {
        if self.screen_w >= 800 { 12 } else { SNAP_MARGIN }
    }

    /// Active pixel zone along the top/bottom edges for corner quadrant snap triggering.
    pub fn snap_corner_margin(&self) -> i32 {
        20
    }

    /// Detects potential snap targets (quadrants, halves, or maximize) based on cursor coordinates.
    pub fn detect_snap_target(&self, px: i32, py: i32) -> Option<SnapState> {
        let work = self.work_area();
        let edge = self.snap_edge_margin();
        let top_edge = if self.screen_w >= 800 { 2 } else { 1 };
        let corner = self.snap_corner_margin();

        let at_left = px <= work.x + edge;
        let at_right = px >= work.x + work.w - edge;
        let at_top = py <= work.y + top_edge;

        if at_left {
            if py <= work.y + corner {
                Some(SnapState::TopLeft)
            } else if py >= work.y + work.h - corner {
                Some(SnapState::BottomLeft)
            } else {
                Some(SnapState::Left)
            }
        } else if at_right {
            if py <= work.y + corner {
                Some(SnapState::TopRight)
            } else if py >= work.y + work.h - corner {
                Some(SnapState::BottomRight)
            } else {
                Some(SnapState::Right)
            }
        } else if at_top {
            Some(SnapState::Maximize)
        } else {
            None
        }
    }

    /// Docks a window to a target snap state within the current monitor work area.
    pub fn snap_window(&mut self, idx: usize, snap: SnapState) {
        if idx >= MAX_WINDOWS {
            return;
        }
        let work = self.work_area();
        let w = &mut self.windows[idx];
        if snap == SnapState::Normal {
            let (px, py, pw, ph) = w.pre_snap;
            w.snap = SnapState::Normal;
            w.x = px;
            w.y = py;
            w.w = pw;
            w.h = ph;
            return;
        }
        if w.snap == SnapState::Normal {
            w.pre_snap = (w.x, w.y, w.w, w.h);
        }
        w.snap = snap;
        let (sx, sy, sw, sh) = calculate_snap_rect(snap, work);
        w.x = sx;
        w.y = sy;
        w.w = sw;
        w.h = sh;
    }

    /// Toggles a window between maximized full-workarea state and normal floating state.
    pub fn toggle_maximize(&mut self, idx: usize) {
        if idx >= MAX_WINDOWS {
            return;
        }
        if self.windows[idx].snap == SnapState::Maximize {
            self.snap_window(idx, SnapState::Normal);
        } else {
            self.snap_window(idx, SnapState::Maximize);
        }
    }

    /// Tiles active open windows according to the requested productivity layout.
    pub fn tile_windows(&mut self, mode: TileMode) {
        match mode {
            TileMode::SideBySide => {
                let mut open_wins = [0usize; 2];
                let mut count = 0;
                for i in (0..MAX_WINDOWS).rev() {
                    let win_idx = self.z_order[i];
                    if self.windows[win_idx].is_open && !self.windows[win_idx].is_minimized {
                        open_wins[count] = win_idx;
                        count += 1;
                        if count == 2 {
                            break;
                        }
                    }
                }
                if count >= 1 {
                    self.snap_window(open_wins[0], SnapState::Left);
                }
                if count >= 2 {
                    self.snap_window(open_wins[1], SnapState::Right);
                }
            }
            TileMode::Grid2x2 => {
                let quadrants = [
                    SnapState::TopLeft,
                    SnapState::TopRight,
                    SnapState::BottomLeft,
                    SnapState::BottomRight,
                ];
                let mut count = 0;
                for i in (0..MAX_WINDOWS).rev() {
                    let win_idx = self.z_order[i];
                    if self.windows[win_idx].is_open && !self.windows[win_idx].is_minimized {
                        self.snap_window(win_idx, quadrants[count]);
                        count += 1;
                        if count == 4 {
                            break;
                        }
                    }
                }
            }
            TileMode::Cascade => {
                let work = self.work_area();
                let (cw, ch) = if self.screen_w >= 800 { (360, 250) } else { (160, 110) };
                let step_x = if self.screen_w >= 800 { 32 } else { 16 };
                let step_y = if self.screen_w >= 800 { 28 } else { 14 };

                let mut count = 0;
                for i in 0..MAX_WINDOWS {
                    let win_idx = self.z_order[i];
                    let w = &mut self.windows[win_idx];
                    if w.is_open && !w.is_minimized {
                        w.snap = SnapState::Normal;
                        w.w = cw;
                        w.h = ch;
                        w.x = (work.x + count * step_x).min(self.screen_w - cw);
                        w.y = (work.y + count * step_y).min(self.desktop_h - ch);
                        count += 1;
                    }
                }
            }
        }
    }

    /// Cycles through all available snap states for keyboard productivity navigation.
    pub fn cycle_snap(&mut self, idx: usize) {
        if idx >= MAX_WINDOWS {
            return;
        }
        let next_snap = match self.windows[idx].snap {
            SnapState::Normal => SnapState::Left,
            SnapState::Left => SnapState::Right,
            SnapState::Right => SnapState::TopLeft,
            SnapState::TopLeft => SnapState::TopRight,
            SnapState::TopRight => SnapState::BottomLeft,
            SnapState::BottomLeft => SnapState::BottomRight,
            SnapState::BottomRight => SnapState::Maximize,
            SnapState::Maximize => SnapState::Normal,
        };
        self.snap_window(idx, next_snap);
    }

    /// Handles standard desktop menu action IDs for window management.
    pub fn handle_window_action(&mut self, action_id: u32) -> bool {
        let active_idx = match self.active_window() {
            Some(idx) => idx,
            None => return false,
        };
        match action_id {
            ui::menu::ACTION_WINDOW_MINIMIZE => {
                self.minimize_window(active_idx);
                true
            }
            ui::menu::ACTION_WINDOW_MAXIMIZE => {
                self.toggle_maximize(active_idx);
                true
            }
            ui::menu::ACTION_WINDOW_SNAP_LEFT => {
                self.snap_window(active_idx, SnapState::Left);
                true
            }
            ui::menu::ACTION_WINDOW_SNAP_RIGHT => {
                self.snap_window(active_idx, SnapState::Right);
                true
            }
            ui::menu::ACTION_WINDOW_SNAP_TOP_LEFT => {
                self.snap_window(active_idx, SnapState::TopLeft);
                true
            }
            ui::menu::ACTION_WINDOW_SNAP_TOP_RIGHT => {
                self.snap_window(active_idx, SnapState::TopRight);
                true
            }
            ui::menu::ACTION_WINDOW_SNAP_BOTTOM_LEFT => {
                self.snap_window(active_idx, SnapState::BottomLeft);
                true
            }
            ui::menu::ACTION_WINDOW_SNAP_BOTTOM_RIGHT => {
                self.snap_window(active_idx, SnapState::BottomRight);
                true
            }
            ui::menu::ACTION_WINDOW_RESTORE => {
                self.snap_window(active_idx, SnapState::Normal);
                true
            }
            ui::menu::ACTION_WINDOW_TILE_SIDE_BY_SIDE => {
                self.tile_windows(TileMode::SideBySide);
                true
            }
            ui::menu::ACTION_WINDOW_TILE_GRID => {
                self.tile_windows(TileMode::Grid2x2);
                true
            }
            ui::menu::ACTION_WINDOW_CASCADE => {
                self.tile_windows(TileMode::Cascade);
                true
            }
            _ => false,
        }
    }

    /// Raises `win_index` to topmost in z-order.
    pub fn raise(&mut self, win_index: usize) {
        if let Some(pos) = self.z_order.iter().position(|&w| w == win_index) {
            for i in pos..MAX_WINDOWS - 1 {
                self.z_order[i] = self.z_order[i + 1];
            }
            self.z_order[MAX_WINDOWS - 1] = win_index;
        }
    }

    pub fn titlebar_at(&self, px: i32, py: i32) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if !w.is_open || w.is_minimized {
                continue;
            }
            if point_in_rect(px, py, w.x, w.y, w.w, self.titlebar_h) {
                return Some(idx);
            }
        }
        None
    }

    pub fn window_at(&self, px: i32, py: i32) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if !w.is_open || w.is_minimized {
                continue;
            }
            if point_in_rect(px, py, w.x, w.y, w.w, w.h) {
                return Some(idx);
            }
        }
        None
    }

    /// Returns the topmost window that is open and not minimized.
    pub fn active_window(&self) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if w.is_open && !w.is_minimized {
                return Some(idx);
            }
        }
        None
    }

    /// Hit-tests pointer coordinates against dock / taskbar items.
    pub fn dock_item_at(&self, px: i32, py: i32) -> Option<usize> {
        if py < self.desktop_h || py >= self.screen_h || px < 0 || px >= self.screen_w {
            return None;
        }

        if self.taskbar_h >= 24 {
            let taskbar_rect = Rect::new(0, self.desktop_h, self.screen_w, self.taskbar_h);
            let start_w = 36;
            let start_offset_x = start_w + 12;
            let max_item_w = if self.screen_w >= 800 { 120 } else { 80 };

            for i in 0..self.dock_count {
                if let Some(rect) = ui::dock::measure_dock_item(i, self.dock_count, taskbar_rect, start_offset_x, max_item_w) {
                    if point_in_rect(px, py, rect.x, rect.y, rect.w, rect.h) {
                        return Some(i);
                    }
                }
            }
            None
        } else {
            let count = self.dock_count.max(1);
            let button_w = self.screen_w / count as i32;
            if button_w <= 0 {
                return None;
            }
            let idx = (px / button_w) as usize;
            if idx < count {
                Some(idx)
            } else {
                None
            }
        }
    }

    pub fn taskbar_button_at(&self, px: i32, py: i32) -> Option<usize> {
        self.dock_item_at(px, py)
    }

    /// Returns the bounding rectangle for the interactive Start Menu popup.
    pub fn start_menu_rect(&self) -> Rect {
        let sm_w = if self.screen_w >= 800 { 380 } else { 280 };
        let sm_h = if self.desktop_h >= 500 { 470 } else { (self.desktop_h - 20).max(120) };
        let sm_x = 12;
        let sm_y = (self.desktop_h - sm_h - 6).max(self.menubar_h);
        Rect::new(sm_x, sm_y, sm_w, sm_h)
    }

    /// Computes hit-tested Start Menu application card rectangle.
    pub fn start_menu_item_rect(&self, idx: usize) -> Option<Rect> {
        if idx >= 12 {
            return None;
        }
        let sm = self.start_menu_rect();
        let pad = 14;
        let col_w = (sm.w - pad * 2 - 10) / 2;
        let col = (idx % 2) as i32;
        let row = (idx / 2) as i32;
        let card_x = sm.x + pad + col * (col_w + 10);
        let card_y = sm.y + 104 + row * 52;
        Some(Rect::new(card_x, card_y, col_w, 46))
    }

    /// Routes keyboard events to the focused top window (e.g. Console terminal).
    pub fn on_key(&mut self, key: i32) {
        if key <= 0 {
            return;
        }
        if self.active_window() == Some(0) && self.windows[0].is_open && !self.windows[0].is_minimized {
            self.term.handle_key_with_context(key, &mut self.time_cfg, self.raw_rtc);
        }
    }

    /// Reads CMOS RTC hardware time from kernel and updates raw_rtc.
    pub fn sync_rtc(&mut self) {
        let mut rtc = ffi::RtcTime::default();
        let rc = unsafe { ffi::sys_rtc_read(&mut rtc) };
        if rc == 0 && rtc.year >= 2020 {
            self.raw_rtc = rtc;
        } else if self.raw_rtc.year < 2020 {
            self.raw_rtc = ffi::RtcTime {
                year: 2026,
                month: 11,
                day: 19,
                hour: 14,
                minute: 45,
                second: 0,
            };
        }
        self.calendar_app.today_year = self.raw_rtc.year;
        self.calendar_app.today_month = self.raw_rtc.month;
        self.calendar_app.today_day = self.raw_rtc.day;
    }

    /// Returns the current dynamic date and time computed with active timezone and manual offset.
    pub fn current_datetime(&self) -> DateTime {
        let tz = &TIMEZONES[self.time_cfg.timezone_idx % TIMEZONES.len()];
        let total_offset = tz.offset_minutes + self.time_cfg.manual_offset_minutes;
        calculate_datetime(self.raw_rtc, total_offset, self.time_cfg.manual_offset_days)
    }

    /// Returns the bounding rectangle for the Calendar and International Time Hub flyout.
    pub fn calendar_rect(&self) -> Rect {
        let w = 320;
        let h = 330;
        let x = (self.screen_w - w - 8).max(4);
        let y = (self.desktop_h - h - 8).max(self.menubar_h + 4);
        Rect::new(x, y, w, h)
    }

    /// Hit-tests a pointer location against Start Menu application cards.
    pub fn start_menu_item_at(&self, px: i32, py: i32) -> Option<usize> {
        if !self.start_menu_open {
            return None;
        }
        for i in 0..12 {
            if let Some(r) = self.start_menu_item_rect(i) {
                if point_in_rect(px, py, r.x, r.y, r.w, r.h) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Returns the bounding rectangle for desktop shortcut icon at index `idx`.
    pub fn desktop_icon_rect(&self, idx: usize) -> Option<Rect> {
        if idx >= DESKTOP_SHORTCUTS.len() || self.desktop_h < 400 {
            return None;
        }
        let start_y = if self.menubar_h > 0 { self.menubar_h + 16 } else { 16 };
        let items_per_col = ((self.desktop_h - start_y - 10) / 78).max(1) as usize;
        let col = (idx / items_per_col) as i32;
        let row = (idx % items_per_col) as i32;
        let x = 18 + col * 86;
        let y = start_y + row * 78;
        if y + 70 > self.desktop_h {
            return None;
        }
        Some(Rect::new(x, y, 78, 70))
    }

    /// Hit-tests pointer coordinates against desktop shortcut icons.
    pub fn desktop_icon_at(&self, px: i32, py: i32) -> Option<usize> {
        if self.desktop_h < 400 {
            return None;
        }
        // If an open window covers the desktop icon, it's not clickable
        if self.window_at(px, py).is_some() {
            return None;
        }
        for i in 0..DESKTOP_SHORTCUTS.len() {
            if let Some(r) = self.desktop_icon_rect(i) {
                if point_in_rect(px, py, r.x, r.y, r.w, r.h) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Launches or restores application window with label `app_id`.
    pub fn launch_or_restore(&mut self, app_id: u8) {
        // 1. If matching window exists, un-minimize, raise, and focus it
        for (w_idx, w) in self.windows.iter_mut().enumerate() {
            if w.label == app_id {
                w.is_open = true;
                w.is_minimized = false;
                self.raise(w_idx);
                return;
            }
        }
        // 2. Allocate first closed / minimized slot
        for (w_idx, w) in self.windows.iter_mut().enumerate() {
            if !w.is_open || w.is_minimized {
                w.label = app_id;
                w.is_open = true;
                w.is_minimized = false;
                self.raise(w_idx);
                return;
            }
        }
        // 3. Fallback: reuse the background-most window slot
        let bg_idx = self.z_order[0];
        self.windows[bg_idx].label = app_id;
        self.windows[bg_idx].is_open = true;
        self.windows[bg_idx].is_minimized = false;
        self.raise(bg_idx);
    }

    /// Windows 11 style "Show Desktop" toggle: minimizes all windows or restores active.
    pub fn toggle_show_desktop(&mut self) {
        let any_visible = self.windows.iter().any(|w| w.is_open && !w.is_minimized);
        if any_visible {
            for w in self.windows.iter_mut() {
                if w.is_open {
                    w.is_minimized = true;
                }
            }
        } else {
            self.windows[0].is_minimized = false;
            self.windows[0].is_open = true;
            self.raise(0);
        }
    }

    /// Returns the MenuBar for the window at `win_index`.
    pub fn app_menubar(&self, win_index: usize) -> ui::menu::MenuBar {
        if win_index < MAX_WINDOWS {
            ui::menu::menubar_for_label(self.windows[win_index].label)
        } else {
            ui::menu::menubar_for_label(1)
        }
    }

    /// Returns the MenuBar for the currently focused (active) window.
    pub fn current_menubar(&self) -> ui::menu::MenuBar {
        if let Some(topmost) = self.active_window() {
            self.app_menubar(topmost)
        } else {
            ui::menu::SYSTEM_MENUBAR
        }
    }

    /// Hit-tests a point against the top menu bar items.
    pub fn menubar_item_at(&self, px: i32, py: i32) -> Option<usize> {
        if self.menubar_h <= 0 || py < 0 || py >= self.menubar_h {
            return None;
        }
        let menubar = self.current_menubar();
        let font = if self.menubar_h >= 20 { Font::regular() } else { Font::small() };
        let star_size = if self.menubar_h >= 20 { 12 } else { 8 };
        let star_x = 8;
        let app_sz = font.measure_str(menubar.app_name);
        let app_x = star_x + star_size + 8;
        let start_x = app_x + app_sz.w + 18;

        for (i, _) in menubar.menus.iter().enumerate() {
            if let Some(r) = ui::menu::measure_menubar_item(&menubar, i, start_x, 0, self.menubar_h, &font) {
                if point_in_rect(px, py, r.x, r.y, r.w, r.h) {
                    return Some(i);
                }
            }
        }
        None
    }

    /// Returns the screen bounds of the currently open dropdown menu.
    pub fn active_dropdown_rect(&self) -> Option<Rect> {
        if self.menubar_h <= 0 {
            return None;
        }
        let menu_idx = self.active_menu?;
        let menubar = self.current_menubar();
        if menu_idx >= menubar.menus.len() {
            return None;
        }
        let font = if self.menubar_h >= 20 { Font::regular() } else { Font::small() };
        let star_size = if self.menubar_h >= 20 { 12 } else { 8 };
        let star_x = 8;
        let app_sz = font.measure_str(menubar.app_name);
        let app_x = star_x + star_size + 8;
        let start_x = app_x + app_sz.w + 18;

        let item_rect = ui::menu::measure_menubar_item(&menubar, menu_idx, start_x, 0, self.menubar_h, &font)?;
        let menu = &menubar.menus[menu_idx];
        Some(ui::menu::measure_dropdown_rect(item_rect.x, self.menubar_h, menu, self.screen_w, &font))
    }

    /// Hit-tests a point against the items inside the active dropdown menu.
    pub fn dropdown_item_at(&self, px: i32, py: i32) -> Option<usize> {
        let rect = self.active_dropdown_rect()?;
        if !point_in_rect(px, py, rect.x, rect.y, rect.w, rect.h) {
            return None;
        }
        let menu_idx = self.active_menu?;
        let menubar = self.current_menubar();
        let menu = menubar.menus.get(menu_idx)?;
        let font = if rect.h > 100 { Font::regular() } else { Font::small() };
        let item_h = if font.line_height() > 10 { 22 } else { 14 };

        let mut cur_y = rect.y + 4;
        for (i, item) in menu.items.iter().enumerate() {
            let h = if item.is_separator { 6 } else { item_h };
            if py >= cur_y && py < cur_y + h {
                if !item.is_separator && item.is_enabled {
                    return Some(i);
                } else {
                    return None;
                }
            }
            cur_y += h;
        }
        None
    }

    /// Drains and returns the most recent menu click event, if any.
    pub fn take_menu_event(&mut self) -> Option<MenuClickEvent> {
        self.last_menu_event.take()
    }

    /// Computes the runtime state of all items currently displayed in the dock/taskbar.
    pub fn dock_items(&self) -> [Option<DockItem>; MAX_DOCK_ITEMS] {
        let mut items = [None; MAX_DOCK_ITEMS];
        let active_win = self.active_window();

        for i in 0..self.dock_count {
            let app = self.dock_apps[i];
            let mut matching_win = None;
            for (w_idx, w) in self.windows.iter().enumerate() {
                if w.is_open && w.label == app.app_id {
                    matching_win = Some((w_idx, w));
                    break;
                }
            }

            if let Some((w_idx, w)) = matching_win {
                let is_active = active_win == Some(w_idx);
                items[i] = Some(DockItem::new(
                    app,
                    true,
                    is_active,
                    w.is_minimized,
                    Some(w_idx),
                ));
            } else {
                items[i] = Some(DockItem::new(
                    app,
                    false,
                    false,
                    false,
                    None,
                ));
            }
        }
        items
    }

    /// Handles a click on the dock / taskbar item at index `idx`.
    pub fn on_dock_item_click(&mut self, idx: usize) {
        if idx >= self.dock_count {
            return;
        }
        let app = self.dock_apps[idx];
        let items = self.dock_items();
        let item = match items[idx] {
            Some(it) => it,
            None => return,
        };

        if item.is_running {
            if let Some(win_idx) = item.window_id {
                let is_active = self.active_window() == Some(win_idx);
                if is_active {
                    // Clicking active focused window minimizes it to taskbar
                    self.minimize_window(win_idx);
                } else if self.windows[win_idx].is_minimized {
                    // Clicking minimized window restores and focuses it
                    self.restore_window(win_idx);
                } else {
                    // Clicking inactive running window raises it to front
                    self.raise(win_idx);
                    self.last_dock_event = Some(DockEvent::Switch { window_id: win_idx });
                }
            }
        } else {
            // App is pinned but NOT currently running: Launch-on-click!
            self.last_dock_event = Some(DockEvent::Launch {
                app_id: app.app_id,
                exec_path: app.exec_path,
            });

            // Re-open window if matching slot exists
            for (w_idx, w) in self.windows.iter_mut().enumerate() {
                if w.label == app.app_id {
                    w.is_open = true;
                    w.is_minimized = false;
                    self.raise(w_idx);
                    return;
                }
            }

            // Or allocate first closed slot
            for (w_idx, w) in self.windows.iter_mut().enumerate() {
                if !w.is_open {
                    w.label = app.app_id;
                    w.is_open = true;
                    w.is_minimized = false;
                    self.raise(w_idx);
                    return;
                }
            }
        }
    }

    /// Minimizes `win_idx` to the dock / taskbar.
    pub fn minimize_window(&mut self, win_idx: usize) {
        if win_idx < MAX_WINDOWS {
            self.windows[win_idx].is_minimized = true;
            self.last_dock_event = Some(DockEvent::Minimize { window_id: win_idx });
        }
    }

    /// Restores a minimized window and brings it to front.
    pub fn restore_window(&mut self, win_idx: usize) {
        if win_idx < MAX_WINDOWS {
            self.windows[win_idx].is_minimized = false;
            self.windows[win_idx].is_open = true;
            self.raise(win_idx);
            self.last_dock_event = Some(DockEvent::Restore { window_id: win_idx });
        }
    }

    /// Closes window `win_idx` while preserving its pinned dock status.
    pub fn close_window(&mut self, win_idx: usize) {
        if win_idx < MAX_WINDOWS {
            self.windows[win_idx].is_open = false;
            self.windows[win_idx].is_minimized = false;
        }
    }

    /// Pins `app` to the dock / taskbar.
    pub fn pin_app(&mut self, app: DockApp) -> bool {
        for i in 0..self.dock_count {
            if self.dock_apps[i].app_id == app.app_id {
                self.dock_apps[i].is_pinned = true;
                self.last_dock_event = Some(DockEvent::Pin { app_id: app.app_id });
                return true;
            }
        }
        if self.dock_count < MAX_DOCK_ITEMS {
            self.dock_apps[self.dock_count] = app;
            self.dock_count += 1;
            self.last_dock_event = Some(DockEvent::Pin { app_id: app.app_id });
            true
        } else {
            false
        }
    }

    /// Unpins `app_id` from the dock.
    pub fn unpin_app(&mut self, app_id: u8) -> bool {
        for i in 0..self.dock_count {
            if self.dock_apps[i].app_id == app_id {
                let items = self.dock_items();
                let is_running = items[i].map_or(false, |it| it.is_running);
                if is_running {
                    self.dock_apps[i].is_pinned = false;
                } else {
                    for j in i..self.dock_count - 1 {
                        self.dock_apps[j] = self.dock_apps[j + 1];
                    }
                    self.dock_count -= 1;
                }
                self.last_dock_event = Some(DockEvent::Unpin { app_id });
                return true;
            }
        }
        false
    }

    /// Checks if `app_id` is pinned in the dock.
    pub fn is_app_pinned(&self, app_id: u8) -> bool {
        for i in 0..self.dock_count {
            if self.dock_apps[i].app_id == app_id && self.dock_apps[i].is_pinned {
                return true;
            }
        }
        false
    }

    /// Drains and returns the most recent dock event, if any.
    pub fn take_dock_event(&mut self) -> Option<DockEvent> {
        self.last_dock_event.take()
    }

    /// Evaluates dynamic cursor shape based on current pointer coordinates.
    pub fn cursor_shape_at(&self, cx: i32, cy: i32) -> CursorShape {
        if self.dragging.is_some() {
            return CursorShape::Move;
        }
        if self.start_menu_open {
            if self.hovered_start_item.is_some() {
                return CursorShape::Pointer;
            }
            let sm = self.start_menu_rect();
            if point_in_rect(cx, cy, sm.x, sm.y, sm.w, sm.h) {
                return CursorShape::Arrow;
            }
        }
        // Active dropdown menu inspection
        if let Some(rect) = self.active_dropdown_rect() {
            if point_in_rect(cx, cy, rect.x, rect.y, rect.w, rect.h) {
                if self.dropdown_item_at(cx, cy).is_some() {
                    return CursorShape::Pointer;
                }
                return CursorShape::Arrow;
            }
        }
        // Top menubar inspection
        if self.menubar_h > 0 && cy < self.menubar_h {
            if (cx >= 8 && cx < 36) || self.menubar_item_at(cx, cy).is_some() {
                return CursorShape::Pointer;
            }
            return CursorShape::Arrow;
        }
        if self.taskbar_h > 0 && cy >= self.desktop_h {
            if cx < 42 || cx >= self.screen_w - 6 || self.taskbar_button_at(cx, cy).is_some() {
                return CursorShape::Pointer;
            }
        }
        // Desktop shortcut icons inspection
        if self.hovered_desktop_icon.is_some() {
            return CursorShape::Pointer;
        }
        for i in (0..MAX_WINDOWS).rev() {
            let win_idx = self.z_order[i];
            let w = &self.windows[win_idx];
            if !w.is_open || w.is_minimized {
                continue;
            }
            if point_in_rect(cx, cy, w.x, w.y, w.w, w.h) {
                if cy < w.y + self.titlebar_h {
                    if self.titlebar_h >= 14 && w.w >= 80 {
                        let btn_size = if self.titlebar_h >= 20 { 12 } else { 8 };
                        let btn_y = w.y + (self.titlebar_h - btn_size) / 2;
                        let close_x = w.x + w.w - btn_size - 8;
                        let min_ctrl_x = close_x - (btn_size + 6) * 2;
                        if cy >= btn_y && cy < btn_y + btn_size && cx >= min_ctrl_x && cx < close_x + btn_size {
                            return CursorShape::Pointer;
                        }
                    }
                    return CursorShape::Move;
                }
                // Client area: Console and Editor use I-Beam text selection cursor
                if w.label == 1 || w.label == 3 {
                    return CursorShape::IBeam;
                } else {
                    return CursorShape::Arrow;
                }
            }
        }
        CursorShape::Arrow
    }

    pub fn on_left_press(&mut self, px: i32, py: i32) {
        // -1. If Calendar & International Time Hub is open:
        if self.calendar_open {
            let cr = self.calendar_rect();
            if point_in_rect(px, py, cr.x, cr.y, cr.w, cr.h) {
                // 1. 12H / 24H Mode Toggle Button
                let btn_mode = Rect::new(cr.x + cr.w - 86, cr.y + 12, 72, 20);
                if point_in_rect(px, py, btn_mode.x, btn_mode.y, btn_mode.w, btn_mode.h) {
                    self.time_cfg.is_24h = !self.time_cfg.is_24h;
                    return;
                }

                // 2. Sync RTC Button
                let btn_sync = Rect::new(cr.x + cr.w - 86, cr.y + 36, 72, 18);
                if point_in_rect(px, py, btn_sync.x, btn_sync.y, btn_sync.w, btn_sync.h) {
                    self.time_cfg.manual_offset_minutes = 0;
                    self.time_cfg.manual_offset_days = 0;
                    self.sync_rtc();
                    return;
                }

                // 2b. Open Full Calendar App Button
                let btn_open_app = Rect::new(cr.x + cr.w - 86, cr.y + 102, 72, 20);
                if point_in_rect(px, py, btn_open_app.x, btn_open_app.y, btn_open_app.w, btn_open_app.h) {
                    self.launch_or_restore(6);
                    self.calendar_open = false;
                    return;
                }

                // 3. Timezone Prev [<] / Next [>] Buttons
                let btn_tz_prev = Rect::new(cr.x + 14, cr.y + 64, 24, 24);
                if point_in_rect(px, py, btn_tz_prev.x, btn_tz_prev.y, btn_tz_prev.w, btn_tz_prev.h) {
                    if self.time_cfg.timezone_idx == 0 {
                        self.time_cfg.timezone_idx = TIMEZONES.len() - 1;
                    } else {
                        self.time_cfg.timezone_idx -= 1;
                    }
                    return;
                }

                let btn_tz_next = Rect::new(cr.x + cr.w - 38, cr.y + 64, 24, 24);
                if point_in_rect(px, py, btn_tz_next.x, btn_tz_next.y, btn_tz_next.w, btn_tz_next.h) {
                    self.time_cfg.timezone_idx = (self.time_cfg.timezone_idx + 1) % TIMEZONES.len();
                    return;
                }

                // 4. Calendar Day Cells
                let dt = self.current_datetime();
                let first_dow = day_of_week(dt.year, dt.month, 1);
                let days_this_month = days_in_month(dt.year, dt.month);
                let col_w = 41;
                let grid_x = cr.x + 16;
                let cell_y0 = cr.y + 124 + 18;
                for row in 0..6 {
                    for col in 0..7 {
                        let cx = grid_x + (col as i32) * col_w;
                        let cy = cell_y0 + (row as i32) * 22;
                        if px >= cx && px < cx + col_w && py >= cy && py < cy + 22 {
                            let idx = row * 7 + col;
                            if idx >= first_dow as usize && idx < first_dow as usize + days_this_month as usize {
                                let clicked_day = (idx - first_dow as usize + 1) as u8;
                                let diff = (clicked_day as i32) - (dt.day as i32);
                                self.time_cfg.manual_offset_days += diff;
                                return;
                            }
                        }
                    }
                }

                // 5. Manual Time & Date Adjust Controls
                let ctrl_y = cr.y + 294;
                let btn_h_min = Rect::new(cr.x + 48, ctrl_y, 20, 20);
                if point_in_rect(px, py, btn_h_min.x, btn_h_min.y, btn_h_min.w, btn_h_min.h) {
                    self.time_cfg.manual_offset_minutes -= 60;
                    return;
                }
                let btn_h_pls = Rect::new(cr.x + 72, ctrl_y, 20, 20);
                if point_in_rect(px, py, btn_h_pls.x, btn_h_pls.y, btn_h_pls.w, btn_h_pls.h) {
                    self.time_cfg.manual_offset_minutes += 60;
                    return;
                }

                let btn_m_min = Rect::new(cr.x + 134, ctrl_y, 20, 20);
                if point_in_rect(px, py, btn_m_min.x, btn_m_min.y, btn_m_min.w, btn_m_min.h) {
                    self.time_cfg.manual_offset_minutes -= 1;
                    return;
                }
                let btn_m_pls = Rect::new(cr.x + 158, ctrl_y, 20, 20);
                if point_in_rect(px, py, btn_m_pls.x, btn_m_pls.y, btn_m_pls.w, btn_m_pls.h) {
                    self.time_cfg.manual_offset_minutes += 1;
                    return;
                }

                let btn_d_min = Rect::new(cr.x + 220, ctrl_y, 20, 20);
                if point_in_rect(px, py, btn_d_min.x, btn_d_min.y, btn_d_min.w, btn_d_min.h) {
                    self.time_cfg.manual_offset_days -= 1;
                    return;
                }
                let btn_d_pls = Rect::new(cr.x + 244, ctrl_y, 20, 20);
                if point_in_rect(px, py, btn_d_pls.x, btn_d_pls.y, btn_d_pls.w, btn_d_pls.h) {
                    self.time_cfg.manual_offset_days += 1;
                    return;
                }

                return;
            } else {
                self.calendar_open = false;
                if (self.menubar_h > 0 && py < self.menubar_h && px >= self.screen_w - 140)
                    || (py >= self.desktop_h && px >= self.screen_w - 140 && px < self.screen_w - 6)
                {
                    return;
                }
            }
        }

        // 0. If Start Menu is open:
        if self.start_menu_open {
            let sm = self.start_menu_rect();
            if point_in_rect(px, py, sm.x, sm.y, sm.w, sm.h) {
                if let Some(item_idx) = self.start_menu_item_at(px, py) {
                    let target_app: u8 = match item_idx {
                        0 => 1, // Terminal
                        1 => 2, // Files
                        2 => 8, // Aero Browser
                        3 => 9, // Spectra (Default Media Player)
                        4 => 10, // Vellum (Default PDF Studio)
                        5 => 12, // Zenith Office Suite
                        6 => 7, // Calculator
                        7 => 6, // Calendar
                        8 => 5, // Settings
                        9 => 4, // Software Store
                        10 => 11, // Vortex BitTorrent & IDM
                        _ => 3, // Text Editor
                    };
                    self.launch_or_restore(target_app);
                    self.start_menu_open = false;
                    return;
                }
                // Check if power buttons were clicked
                let foot_y = sm.y + sm.h - 40;
                if py >= foot_y && py <= foot_y + 24 && px >= sm.x + sm.w - 160 && px < sm.x + sm.w - 12 {
                    self.start_menu_open = false;
                    return;
                }
                return;
            } else {
                self.start_menu_open = false;
            }
        }

        // 1. If dropdown menu is open:
        if self.active_menu.is_some() {
            if let Some(item_idx) = self.dropdown_item_at(px, py) {
                let menu_idx = self.active_menu.unwrap();
                let topmost = self.z_order[MAX_WINDOWS - 1];
                let menubar = self.current_menubar();
                let action_id = menubar.menus[menu_idx].items[item_idx].action_id;
                self.last_menu_event = Some(MenuClickEvent {
                    window_id: topmost,
                    menu_index: menu_idx,
                    item_index: item_idx,
                    action_id,
                });
                self.handle_window_action(action_id);
                self.active_menu = None;
                return;
            }
            // Clicking inside dropdown panel on separator/padding
            if let Some(drop_rect) = self.active_dropdown_rect() {
                if point_in_rect(px, py, drop_rect.x, drop_rect.y, drop_rect.w, drop_rect.h) {
                    return;
                }
            }
            // Clicking a top menu item toggles or switches
            if let Some(menu_idx) = self.menubar_item_at(px, py) {
                if self.active_menu == Some(menu_idx) {
                    self.active_menu = None;
                } else {
                    self.active_menu = Some(menu_idx);
                }
                return;
            }
            // Clicking outside dismisses dropdown
            self.active_menu = None;
        }

        // 2. Top menu bar clicks
        if self.menubar_h > 0 && py < self.menubar_h {
            if px >= 6 && px < 36 {
                self.start_menu_open = !self.start_menu_open;
                return;
            }
            if px >= self.screen_w - 140 {
                self.calendar_open = !self.calendar_open;
                return;
            }
            if let Some(menu_idx) = self.menubar_item_at(px, py) {
                self.active_menu = Some(menu_idx);
            }
            return;
        }

        // 3. Taskbar button clicks
        if py >= self.desktop_h {
            // Windows 11 Start badge on far left
            if px < 42 {
                self.start_menu_open = !self.start_menu_open;
                return;
            }
            // Windows 11 "Show Desktop" peek strip on far right edge
            if px >= self.screen_w - 6 {
                self.toggle_show_desktop();
                return;
            }
            // Taskbar clock on right edge
            if px >= self.screen_w - 140 {
                self.calendar_open = !self.calendar_open;
                return;
            }
            if let Some(idx) = self.taskbar_button_at(px, py) {
                self.on_dock_item_click(idx);
                return;
            }
            return;
        }

        // 4. Window titlebar clicks (raise and start drag)
        if let Some(idx) = self.titlebar_at(px, py) {
            let w = &mut self.windows[idx];
            if self.titlebar_h >= 14 && w.w >= 80 {
                let btn_size = if self.titlebar_h >= 20 { 12 } else { 8 };
                let btn_y = w.y + (self.titlebar_h - btn_size) / 2;
                let close_x = w.x + w.w - btn_size - 8;
                let max_x = close_x - btn_size - 6;
                let min_x = max_x - btn_size - 6;

                if py >= btn_y && py < btn_y + btn_size {
                    if px >= close_x && px < close_x + btn_size {
                        self.close_window(idx);
                        return;
                    } else if px >= max_x && px < max_x + btn_size {
                        self.toggle_maximize(idx);
                        return;
                    } else if px >= min_x && px < min_x + btn_size {
                        self.minimize_window(idx);
                        return;
                    }
                }
            }

            self.raise(idx);
            let w = &mut self.windows[idx];
            if w.snap != SnapState::Normal {
                let old_w = w.w.max(1);
                let rel_x = (px - w.x).clamp(0, old_w);
                let (_, _, pw, ph) = w.pre_snap;
                w.snap = SnapState::Normal;
                w.w = pw;
                w.h = ph;
                let new_offset_x = (pw * rel_x) / old_w;
                w.x = (px - new_offset_x).clamp(0, self.screen_w - pw);
                w.y = py.min(self.desktop_h - ph).max(0);
            }
            self.dragging = Some(idx);
            self.drag_offset_x = px - self.windows[idx].x;
            self.drag_offset_y = py - self.windows[idx].y;
            return;
        }

        // 5. Window body clicks (raise & route interactive events)
        if let Some(idx) = self.window_at(px, py) {
            self.raise(idx);
            let w = &self.windows[idx];
            if w.label == 6 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.calendar_app.handle_click(px, py, client);
                }
            } else if w.label == 7 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.calc_app.handle_click(px, py, client);
                }
            } else if w.label == 8 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.browser_app.handle_click(px, py, client);
                }
            } else if w.label == 9 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.player_app.handle_click(px, py, client);
                }
            } else if w.label == 10 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.vellum_app.handle_click(px, py, client);
                }
            } else if w.label == 11 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.vortex_app.handle_click(px, py, client);
                }
            } else if w.label == 12 && self.titlebar_h >= 20 {
                let body_h = w.h - self.titlebar_h;
                if body_h > 0 {
                    let client = Rect::new(w.x, w.y + self.titlebar_h, w.w, body_h);
                    self.zenith_app.handle_click(px, py, client);
                }
            }
            return;
        }

        // 6. Desktop shortcut icon clicks (launch or restore app)
        if let Some(icon_idx) = self.desktop_icon_at(px, py) {
            let spec = DESKTOP_SHORTCUTS[icon_idx];
            self.launch_or_restore(spec.app_id);
            return;
        }
    }

    pub fn on_drag_release(&mut self, idx: usize, px: i32) {
        self.on_drag_release_at(idx, px, self.cursor_y);
    }

    pub fn on_drag_release_at(&mut self, idx: usize, px: i32, py: i32) {
        self.preview_snap = None;
        if let Some(target) = self.detect_snap_target(px, py) {
            self.snap_window(idx, target);
        }
    }

    pub fn apply_mouse(&mut self, m: ffi::MouseState) {
        self.cursor_x = (self.cursor_x + m.dx).clamp(0, self.screen_w - 1);
        self.cursor_y = (self.cursor_y + m.dy).clamp(0, self.screen_h - 1);

        self.hovered_menu = self.menubar_item_at(self.cursor_x, self.cursor_y);
        self.hovered_item = self.dropdown_item_at(self.cursor_x, self.cursor_y);
        self.hovered_dock_item = self.dock_item_at(self.cursor_x, self.cursor_y);
        self.hovered_start_item = self.start_menu_item_at(self.cursor_x, self.cursor_y);
        self.hovered_desktop_icon = self.desktop_icon_at(self.cursor_x, self.cursor_y);

        // If dropdown is open, hovering over another top menu switches to that dropdown
        if self.active_menu.is_some() {
            if let Some(m_idx) = self.hovered_menu {
                self.active_menu = Some(m_idx);
            }
        }

        let just_pressed = m.left_button && !self.prev_left_button;
        let just_released = !m.left_button && self.prev_left_button;

        if just_pressed && self.dragging.is_none() {
            self.on_left_press(self.cursor_x, self.cursor_y);
        }

        if let Some(idx) = self.dragging {
            if m.left_button {
                let w = &mut self.windows[idx];
                w.x = (self.cursor_x - self.drag_offset_x).clamp(0, self.screen_w - w.w);
                w.y = (self.cursor_y - self.drag_offset_y).clamp(0, self.desktop_h - w.h);
                self.preview_snap = self.detect_snap_target(self.cursor_x, self.cursor_y);
            }
            if just_released {
                self.on_drag_release_at(idx, self.cursor_x, self.cursor_y);
                self.dragging = None;
                self.preview_snap = None;
            }
        }

        self.prev_left_button = m.left_button;
    }

    /// Renders desktop background, windows in z-order, taskbar, and cursor
    /// directly into a 32-bit (or generic) drawing surface with zero per-pixel syscalls.
    pub fn render<S: Surface + ?Sized>(&self, surface: &mut S) {
        let palette = self.access_cfg.effective_palette();
        let is_high_contrast = self.access_cfg.theme.is_high_contrast();

        // 1. Desktop background & Desktop Icons
        if self.screen_h >= 600 && !is_high_contrast {
            draw_modern_desktop_wallpaper(surface, self.screen_w, self.desktop_h);
            draw_desktop_icons(surface, self.menubar_h, self.hovered_desktop_icon, &palette);
        } else {
            surface.fill_rect(Rect::new(0, 0, self.screen_w, self.desktop_h), palette.desktop_bg);
        }

        // 2. Windows in z-order
        let active_win = self.active_window();
        for i in 0..MAX_WINDOWS {
            let win_idx = self.z_order[i];
            let w = &self.windows[win_idx];
            if !w.is_open || w.is_minimized {
                continue;
            }
            let is_active = Some(win_idx) == active_win;

            let (tb_color, border_col) = if is_high_contrast {
                if is_active {
                    (Color::WHITE, palette.border_focused) // White active titlebar, pure yellow border
                } else {
                    (Color::BLACK, palette.border_normal)  // Black inactive titlebar, pure white border
                }
            } else if is_active {
                (w.titlebar_color, palette.border_focused)
            } else {
                (w.titlebar_color.darken(40), palette.border_normal)
            };

            // Window outer drop shadow for active window on high-res (disabled in high contrast)
            if is_active && self.titlebar_h >= 20 && w.snap == SnapState::Normal && !is_high_contrast {
                surface.fill_rect(Rect::new(w.x + 3, w.y + w.h, w.w - 1, 2), Color::rgba(0, 0, 0, 45));
                surface.fill_rect(Rect::new(w.x + w.w, w.y + 3, 2, w.h), Color::rgba(0, 0, 0, 45));
            }

            // Window client body
            let body_y = w.y + self.titlebar_h;
            let body_h = w.h - self.titlebar_h;
            let body_color = if is_high_contrast { Color::BLACK } else { w.body_color };
            if body_h > 0 {
                surface.fill_rect(Rect::new(w.x, body_y, w.w, body_h), body_color);

                // Render client application view on high-res
                if w.label == 1 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_term_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.term, &palette);
                } else if w.label == 2 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_files_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 3 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_editor_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 4 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_store_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 5 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_settings_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &palette);
                } else if w.label == 6 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_calendar_app_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.calendar_app, &palette);
                } else if w.label == 7 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_calc_app_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.calc_app, &palette);
                } else if w.label == 8 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_browser_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.browser_app, &palette);
                } else if w.label == 9 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_player_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.player_app, &palette);
                } else if w.label == 10 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_vellum_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.vellum_app, &palette);
                } else if w.label == 11 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_vortex_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.vortex_app, &palette);
                } else if w.label == 12 && self.titlebar_h >= 20 && body_h >= 80 && w.w >= 140 {
                    draw_zenith_client_body(surface, Rect::new(w.x, body_y, w.w, body_h), &self.zenith_app, &palette);
                }
            }

            // Window titlebar
            if is_high_contrast {
                surface.fill_rect(Rect::new(w.x, w.y, w.w, self.titlebar_h), tb_color);
            } else if self.titlebar_h >= 20 {
                let half_h = self.titlebar_h / 2;
                surface.fill_rect(Rect::new(w.x, w.y, w.w, half_h), tb_color.lighten(12));
                surface.fill_rect(Rect::new(w.x, w.y + half_h, w.w, self.titlebar_h - half_h), tb_color);
            } else {
                surface.fill_rect(Rect::new(w.x, w.y, w.w, self.titlebar_h), tb_color);
            }

            // Titlebar top highlight sheen & bottom separator
            if self.titlebar_h > 1 && !is_high_contrast {
                let sheen_col = if is_active { tb_color.lighten(35) } else { tb_color.lighten(15) };
                surface.fill_rect(Rect::new(w.x + 1, w.y, w.w - 2, 1), sheen_col);
                surface.fill_rect(Rect::new(w.x, w.y + self.titlebar_h - 1, w.w, 1), palette.border_subtle);
            } else if is_high_contrast {
                surface.fill_rect(Rect::new(w.x, w.y + self.titlebar_h - 1, w.w, 1), palette.border_normal);
            }

            // Window border outline
            if is_high_contrast && is_active {
                ui::draw::draw_rect_outline(surface, Rect::new(w.x, w.y, w.w, w.h), 2, palette.border_focused);
            } else {
                ui::draw::draw_rect_outline(surface, Rect::new(w.x, w.y, w.w, w.h), 1, border_col);
                if is_active && self.titlebar_h >= 20 && !is_high_contrast {
                    ui::draw::draw_rect_outline(surface, Rect::new(w.x + 1, w.y + 1, w.w - 2, w.h - 2), 1, Color::rgba(255, 255, 255, 20));
                }
            }

            // Application Icon & Title Text
            let text_col = if is_high_contrast {
                if is_active { palette.titlebar_text_active } else { palette.titlebar_text_inactive }
            } else if is_active {
                palette.text_primary
            } else {
                palette.text_muted
            };

            if self.titlebar_h >= 20 {
                let icon_size = if self.titlebar_h >= 32 { 16 } else { 12 };
                let icon_x = w.x + 8;
                let icon_y = w.y + (self.titlebar_h - icon_size) / 2;
                draw_app_icon(surface, icon_x, icon_y, w.label, &palette);

                let font = Font::regular().scaled(self.access_cfg.font_scale);
                let text_x = icon_x + icon_size + 6;
                let text_y = w.y + (self.titlebar_h - font.line_height()) / 2;
                let title_str = match w.label {
                    1 => "Console - NovaOS Shell",
                    2 => "Files - Root Directory",
                    3 => "Editor - Scratchpad",
                    4 => "Software Store - Packages",
                    5 => "Settings - Control Panel",
                    6 => "Calendar - NovaOS Schedule & Events",
                    7 => "Calculator - NovaOS Math Studio",
                    8 => "Aero Browser - Fast & Private Web",
                    9 => "Spectra - Ultra HD Media Studio",
                    10 => "Vellum - Professional PDF Studio",
                    11 => "Vortex - BitTorrent & Stream Studio",
                    12 => "Zenith Office - Professional Productivity Suite",
                    _ => "System - Diagnostics",
                };
                ui::font::draw_text_font(surface, text_x, text_y, title_str, text_col, &font);
            } else {
                let font = Font::small().scaled(self.access_cfg.font_scale);
                let text_y = w.y + (self.titlebar_h - 8).max(0) / 2;
                ui::font::draw_char_font(surface, w.x + 4, text_y, b'0' + w.label, text_col, &font);
            }

            // Redesigned Window Control Buttons (Close, Maximize, Minimize)
            if self.titlebar_h >= 14 && w.w >= 80 {
                let btn_size = if self.titlebar_h >= 32 { 16 } else if self.titlebar_h >= 20 { 12 } else { 8 };
                let btn_y = w.y + (self.titlebar_h - btn_size) / 2;
                let close_x = w.x + w.w - btn_size - 8;
                let max_x = close_x - btn_size - 6;
                let min_x = max_x - btn_size - 6;

                // Close Button ('X')
                let close_col = if is_active || is_high_contrast { palette.status_danger } else { palette.status_danger.darken(35) };
                ui::draw::draw_rounded_rect(surface, Rect::new(close_x, btn_y, btn_size, btn_size), 2, close_col);
                if is_high_contrast {
                    ui::draw::draw_rect_outline(surface, Rect::new(close_x, btn_y, btn_size, btn_size), 1, Color::WHITE);
                }
                let x_span = ((btn_size - 4) / 2).max(1);
                let cx = close_x + btn_size / 2;
                let cy = btn_y + btn_size / 2;
                ui::draw::draw_line(surface, cx - x_span, cy - x_span, cx + x_span, cy + x_span, Color::WHITE);
                ui::draw::draw_line(surface, cx + x_span, cy - x_span, cx - x_span, cy + x_span, Color::WHITE);

                // Maximize Button ('[]')
                let max_col = if is_active || is_high_contrast { palette.status_success } else { palette.status_success.darken(35) };
                ui::draw::draw_rounded_rect(surface, Rect::new(max_x, btn_y, btn_size, btn_size), 2, max_col);
                if is_high_contrast {
                    ui::draw::draw_rect_outline(surface, Rect::new(max_x, btn_y, btn_size, btn_size), 1, Color::WHITE);
                }
                let sq_size = (btn_size - 6).max(3);
                let sq_x = max_x + (btn_size - sq_size) / 2;
                let sq_y = btn_y + (btn_size - sq_size) / 2;
                ui::draw::draw_rect_outline(surface, Rect::new(sq_x, sq_y, sq_size, sq_size), 1, Color::WHITE);

                // Minimize Button ('-')
                let min_col = if is_active || is_high_contrast { palette.status_warning } else { palette.status_warning.darken(35) };
                ui::draw::draw_rounded_rect(surface, Rect::new(min_x, btn_y, btn_size, btn_size), 2, min_col);
                if is_high_contrast {
                    ui::draw::draw_rect_outline(surface, Rect::new(min_x, btn_y, btn_size, btn_size), 1, Color::WHITE);
                }
                let bar_w = (btn_size - 4).max(3);
                let bar_x = min_x + (btn_size - bar_w) / 2;
                let bar_color = if is_high_contrast { Color::BLACK } else { Color::WHITE };
                surface.fill_rect(Rect::new(bar_x, cy, bar_w, 1), bar_color);
            }
        }

        // 2b. Visual snap preview ghost overlay
        if let Some(snap) = self.preview_snap {
            let (sx, sy, sw, sh) = calculate_snap_rect(snap, self.work_area());
            if sw > 8 && sh > 8 {
                if is_high_contrast {
                    ui::draw::draw_rect_outline(
                        surface,
                        Rect::new(sx + 2, sy + 2, sw - 4, sh - 4),
                        3,
                        Color::rgb(255, 255, 0),
                    );
                } else {
                    surface.fill_rect(
                        Rect::new(sx + 3, sy + 3, sw - 6, sh - 6),
                        Color::rgba(59, 130, 246, 45),
                    );
                    ui::draw::draw_rect_outline(
                        surface,
                        Rect::new(sx + 3, sy + 3, sw - 6, sh - 6),
                        2,
                        palette.accent_primary,
                    );
                }
            }
        }

        // Format dynamic wall-clock string according to active timezone and 12h/24h mode
        let mut clock_buf = [0u8; 32];
        let dt = self.current_datetime();
        let clock_str = format_clock(&dt, self.time_cfg.is_24h, false, &mut clock_buf);

        // 3. Taskbar / Dock
        if self.taskbar_h >= 24 {
            let dock_items = self.dock_items();
            let mut valid_items = [DockItem::new(self.dock_apps[0], false, false, false, None); MAX_DOCK_ITEMS];
            let mut valid_count = 0;
            for i in 0..self.dock_count {
                if let Some(it) = dock_items[i] {
                    valid_items[valid_count] = it;
                    valid_count += 1;
                }
            }
            let taskbar_rect = Rect::new(0, self.desktop_h, self.screen_w, self.taskbar_h);
            ui::dock::draw_dock_surface(
                surface,
                taskbar_rect,
                &valid_items[..valid_count],
                self.hovered_dock_item,
                clock_str,
                &palette,
            );
        } else {
            let taskbar_y = self.desktop_h;
            surface.fill_rect(Rect::new(0, taskbar_y, self.screen_w, self.taskbar_h), palette.taskbar_bg);
            surface.fill_rect(Rect::new(0, taskbar_y, self.screen_w, 1), palette.border_subtle);

            let count = self.dock_count.max(1);
            let button_w = self.screen_w / count as i32;
            let active_win = self.active_window();

            for i in 0..count {
                let bx = i as i32 * button_w;
                let is_active = active_win == Some(i);
                let is_minimized = i < MAX_WINDOWS && self.windows[i].is_minimized;
                let bg = if is_active {
                    palette.taskbar_item_active
                } else if is_minimized {
                    palette.surface_sunken
                } else {
                    palette.taskbar_item_inactive
                };

                let btn_rect = Rect::new(bx + 1, taskbar_y + 1, button_w - 2, self.taskbar_h - 2);
                surface.fill_rect(btn_rect, bg);

                if is_active && self.taskbar_h >= 8 {
                    surface.fill_rect(Rect::new(bx + 4, taskbar_y + self.taskbar_h - 2, button_w - 8, 2), palette.accent_primary);
                }

                let font = Font::small();
                let text_y = taskbar_y + (self.taskbar_h - 8).max(0) / 2;
                let text_x = bx + button_w / 2 - 4;
                let lbl_char = if i < MAX_WINDOWS { b'0' + self.windows[i].label } else { b'0' + (i as u8 + 1) };
                ui::font::draw_char_font(surface, text_x, text_y, lbl_char, palette.text_primary, &font);
            }
        }

        // 4. Persistent Top Menu Bar (always-on-top compositor layer)
        if self.menubar_h > 0 {
            let menubar = self.current_menubar();
            let menubar_rect = Rect::new(0, 0, self.screen_w, self.menubar_h);
            ui::menu::draw_menubar_surface(
                surface,
                menubar_rect,
                &menubar,
                self.active_menu,
                self.hovered_menu,
                clock_str,
                &palette,
            );
        }

        // 5. Active Dropdown Menu Overlay
        if let Some(drop_rect) = self.active_dropdown_rect() {
            if let Some(idx) = self.active_menu {
                let menubar = self.current_menubar();
                if idx < menubar.menus.len() {
                    let menu = &menubar.menus[idx];
                    ui::menu::draw_dropdown_surface(
                        surface,
                        drop_rect,
                        menu,
                        self.hovered_item,
                        &palette,
                    );
                }
            }
        }

        // 5b. Start Menu Popup Overlay
        if self.start_menu_open {
            self.draw_start_menu(surface, &palette);
        }

        // 5c. Calendar & International Time Hub Overlay
        if self.calendar_open {
            self.draw_calendar_flyout(surface, &palette);
        }

        // 6. Mouse cursor
        let shape = self.cursor_shape_at(self.cursor_x, self.cursor_y);
        let cur_theme = self.access_cfg.effective_cursor_theme();
        ui::cursor::draw_cursor_sprite(surface, self.cursor_x, self.cursor_y, shape, cur_theme);
    }

    pub fn draw_start_menu<S: Surface + ?Sized>(&self, s: &mut S, palette: &ThemePalette) {
        let sm = self.start_menu_rect();
        let font_reg = Font::regular().scaled(self.access_cfg.font_scale);
        let font_sm = Font::small().scaled(self.access_cfg.font_scale);

        // 1. Drop shadow (frosted floating window)
        s.fill_rect(Rect::new(sm.x + 4, sm.y + 4, sm.w, sm.h), Color::rgba(0, 0, 0, 80));
        s.fill_rect(Rect::new(sm.x + 2, sm.y + 2, sm.w, sm.h), Color::rgba(0, 0, 0, 50));

        // 2. Acrylic / Frosted glass background & subtle glass border
        ui::draw::draw_rounded_rect(s, sm, 10, Color::rgba(22, 27, 42, 244));
        ui::draw::draw_rect_outline(s, sm, 1, Color::rgba(255, 255, 255, 45));

        // 3. User profile card in header (Windows 11 / macOS account pill)
        let ava_x = sm.x + 16;
        let ava_y = sm.y + 14;
        ui::draw::draw_rounded_rect(s, Rect::new(ava_x, ava_y, 32, 32), 6, palette.accent_primary);
        ui::draw::draw_line(s, ava_x + 16, ava_y + 8, ava_x + 16, ava_y + 18, Color::WHITE);
        ui::draw::draw_line(s, ava_x + 10, ava_y + 24, ava_x + 22, ava_y + 24, Color::WHITE);
        ui::font::draw_text_font(s, ava_x + 40, ava_y + 2, "admin@novaos", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ava_x + 40, ava_y + 18, "Administrator (v2.0 Long Mode)", palette.text_muted, &font_sm);

        // 4. Windows 11 style Search bar
        let srch_y = ava_y + 38;
        let srch_w = sm.w - 32;
        let srch_rect = Rect::new(sm.x + 16, srch_y, srch_w, 24);
        ui::draw::draw_rounded_rect(s, srch_rect, 4, Color::rgba(255, 255, 255, 16));
        ui::draw::draw_rect_outline(s, srch_rect, 1, Color::rgba(255, 255, 255, 30));
        ui::font::draw_text_font(s, sm.x + 24, srch_y + 5, "Search apps, files, settings...", palette.text_muted, &font_sm);

        // 5. Section label
        let sec_y = srch_y + 30;
        ui::font::draw_text_font(s, sm.x + 18, sec_y, "PINNED APPLICATIONS", Color::rgba(255, 255, 255, 120), &font_sm);

        // 6. 12 App Cards (2 columns x 6 rows)
        let apps: [(&str, &str, u8); 12] = [
            ("Terminal", "NovaTerm CLI", 1),
            ("Files", "Root Storage", 2),
            ("Aero Browser", "Fast Web Engine", 8),
            ("Spectra", "Ultra HD Media", 9),
            ("Vellum", "PDF Studio", 10),
            ("Zenith Office", "Productivity", 12),
            ("Calculator", "Math Studio", 7),
            ("Calendar", "Events & Tasks", 6),
            ("Settings", "Control Panel", 5),
            ("App Store", "Nova Packages", 4),
            ("Vortex", "BitTorrent & IDM", 11),
            ("Text Editor", "Document Pad", 3),
        ];

        for i in 0..12 {
            if let Some(card_rect) = self.start_menu_item_rect(i) {
                let is_hov = self.hovered_start_item == Some(i);
                let (title, desc, icon_type) = apps[i];

                if is_hov {
                    ui::draw::draw_rounded_rect(s, card_rect, 6, Color::rgba(255, 255, 255, 28));
                    ui::draw::draw_rect_outline(s, card_rect, 1, Color::rgba(255, 255, 255, 55));
                } else {
                    ui::draw::draw_rounded_rect(s, card_rect, 6, Color::rgba(255, 255, 255, 10));
                    ui::draw::draw_rect_outline(s, card_rect, 1, Color::rgba(255, 255, 255, 20));
                }

                // Realistic App Icon
                let ic_x = card_rect.x + 6;
                let ic_y = card_rect.y + (card_rect.h - 26) / 2;
                draw_realistic_app_icon(s, ic_x, ic_y, 26, icon_type, palette);

                // App Title and Subtitle
                let tx_x = ic_x + 32;
                ui::font::draw_text_font(s, tx_x, card_rect.y + 6, title, Color::WHITE, &font_sm);
                ui::font::draw_text_font(s, tx_x, card_rect.y + 22, desc, palette.text_muted, &font_sm);
            }
        }

        // 7. Footer bar with OS badge and Power controls (spaced with generous margins)
        let foot_y = sm.y + sm.h - 40;
        s.fill_rect(Rect::new(sm.x + 12, foot_y - 6, sm.w - 24, 1), Color::rgba(255, 255, 255, 24));

        // OS badge: "● NovaOS 2.0 GA"
        ui::draw::draw_rounded_rect(s, Rect::new(sm.x + 16, foot_y + 6, 8, 8), 4, Color::rgb(34, 197, 94));
        ui::font::draw_text_font(s, sm.x + 28, foot_y + 4, "NovaOS 2.0 GA", Color::WHITE, &font_sm);

        // Power actions: [Restart] [Power]
        let pwr_w = 58;
        let pwr_x = sm.x + sm.w - 16 - pwr_w;
        ui::draw::draw_rounded_rect(s, Rect::new(pwr_x, foot_y, pwr_w, 22), 4, Color::rgba(239, 68, 68, 180));
        ui::draw::draw_rect_outline(s, Rect::new(pwr_x, foot_y, pwr_w, 22), 1, Color::rgba(255, 255, 255, 50));
        ui::font::draw_text_font(s, pwr_x + 10, foot_y + 4, "Power", Color::WHITE, &font_sm);

        let rst_w = 66;
        let rst_x = pwr_x - 10 - rst_w;
        ui::draw::draw_rounded_rect(s, Rect::new(rst_x, foot_y, rst_w, 22), 4, Color::rgba(71, 85, 105, 180));
        ui::draw::draw_rect_outline(s, Rect::new(rst_x, foot_y, rst_w, 22), 1, Color::rgba(255, 255, 255, 40));
        ui::font::draw_text_font(s, rst_x + 8, foot_y + 4, "Restart", Color::WHITE, &font_sm);
    }

    /// Renders the NovaOS Calendar & International Time Hub flyout panel.
    pub fn draw_calendar_flyout<S: Surface + ?Sized>(&self, s: &mut S, palette: &ThemePalette) {
        let cr = self.calendar_rect();
        let font_bold = Font::regular().scaled(self.access_cfg.font_scale);
        let font_sm = Font::small().scaled(self.access_cfg.font_scale);

        // 1. Drop shadow (frosted floating window)
        s.fill_rect(Rect::new(cr.x + 4, cr.y + 4, cr.w, cr.h), Color::rgba(0, 0, 0, 80));
        s.fill_rect(Rect::new(cr.x + 2, cr.y + 2, cr.w, cr.h), Color::rgba(0, 0, 0, 50));

        // 2. Acrylic / Frosted glass background & subtle glass border
        ui::draw::draw_rounded_rect(s, cr, 10, Color::rgba(22, 27, 42, 244));
        ui::draw::draw_rect_outline(s, cr, 1, Color::rgba(255, 255, 255, 45));

        let dt = self.current_datetime();

        // 3. Digital Clock in Header (e.g. "08:45:30 PM" or "20:45:30")
        let mut clock_buf = [0u8; 32];
        let clock_str = format_clock(&dt, self.time_cfg.is_24h, true, &mut clock_buf);
        ui::font::draw_text_font(s, cr.x + 16, cr.y + 14, clock_str, Color::rgb(0, 212, 255), &font_bold);

        // 4. 12H / 24H Toggle Button (Pill badge)
        let btn_mode = Rect::new(cr.x + cr.w - 86, cr.y + 12, 72, 20);
        ui::draw::draw_rounded_rect(s, btn_mode, 4, Color::rgba(255, 255, 255, 25));
        ui::draw::draw_rect_outline(s, btn_mode, 1, Color::rgba(255, 255, 255, 40));
        let mode_str = if self.time_cfg.is_24h { "24H Mode" } else { "12H Mode" };
        ui::font::draw_text_font(s, btn_mode.x + 8, btn_mode.y + 4, mode_str, palette.text_primary, &font_sm);

        // 5. Full Date String (e.g. "Thursday, 19 November 2026")
        let mut date_buf = [0u8; 64];
        let date_str = format_full_date(&dt, &mut date_buf);
        ui::font::draw_text_font(s, cr.x + 16, cr.y + 36, date_str, palette.text_muted, &font_sm);

        // 6. Sync RTC Button
        let btn_sync = Rect::new(cr.x + cr.w - 86, cr.y + 36, 72, 18);
        ui::draw::draw_rounded_rect(s, btn_sync, 4, Color::rgb(35, 134, 54));
        ui::draw::draw_rect_outline(s, btn_sync, 1, Color::rgb(63, 185, 80));
        ui::font::draw_text_font(s, btn_sync.x + 8, btn_sync.y + 3, "Sync RTC", Color::WHITE, &font_sm);

        // Divider
        s.fill_rect(Rect::new(cr.x + 14, cr.y + 58, cr.w - 28, 1), Color::rgba(255, 255, 255, 25));

        // 7. Timezone Selector Strip
        let tz = &TIMEZONES[self.time_cfg.timezone_idx % TIMEZONES.len()];
        // Prev Button [<]
        let btn_tz_prev = Rect::new(cr.x + 14, cr.y + 64, 24, 24);
        ui::draw::draw_rounded_rect(s, btn_tz_prev, 4, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_tz_prev.x + 8, btn_tz_prev.y + 5, "<", palette.text_primary, &font_bold);

        // Next Button [>]
        let btn_tz_next = Rect::new(cr.x + cr.w - 38, cr.y + 64, 24, 24);
        ui::draw::draw_rounded_rect(s, btn_tz_next, 4, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_tz_next.x + 8, btn_tz_next.y + 5, ">", palette.text_primary, &font_bold);

        // Timezone Details in Center
        ui::font::draw_text_font(s, cr.x + 44, cr.y + 64, tz.name, Color::rgb(88, 166, 255), &font_bold);
        let mut tz_buf = [0u8; 64];
        let tz_summary = format_tz_summary(tz, &mut tz_buf);
        ui::font::draw_text_font(s, cr.x + 44, cr.y + 78, tz_summary, palette.text_muted, &font_sm);

        // Divider
        s.fill_rect(Rect::new(cr.x + 14, cr.y + 96, cr.w - 28, 1), Color::rgba(255, 255, 255, 25));

        // 8. Monthly Calendar Header (e.g. "November 2026")
        let mut my_buf = [0u8; 32];
        let mut my_off = 0;
        append_str(MONTH_NAMES_FULL[dt.month.saturating_sub(1) as usize % 12], &mut my_buf, &mut my_off);
        append_str(" ", &mut my_buf, &mut my_off);
        format_u32(dt.year as u32, &mut my_buf, &mut my_off);
        let my_str = core::str::from_utf8(&my_buf[..my_off]).unwrap_or("");
        ui::font::draw_text_font(s, cr.x + 16, cr.y + 104, my_str, palette.text_primary, &font_bold);

        // Open Full Calendar App Button
        let btn_open_app = Rect::new(cr.x + cr.w - 86, cr.y + 102, 72, 20);
        ui::draw::draw_rounded_rect(s, btn_open_app, 4, Color::rgba(139, 92, 246, 180));
        ui::draw::draw_rect_outline(s, btn_open_app, 1, Color::rgba(255, 255, 255, 60));
        ui::font::draw_text_font(s, btn_open_app.x + 8, btn_open_app.y + 4, "Open App", Color::WHITE, &font_sm);

        // 9. Day of Week Column Headers (Su Mo Tu We Th Fr Sa)
        let col_w = 41;
        let grid_x = cr.x + 16;
        let grid_y = cr.y + 124;
        let dow_headers = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
        for (c, &name) in dow_headers.iter().enumerate() {
            ui::font::draw_text_font(s, grid_x + (c as i32) * col_w + 10, grid_y, name, palette.text_muted, &font_sm);
        }

        // 10. Calendar Grid Cells
        let first_dow = day_of_week(dt.year, dt.month, 1);
        let days_this_month = days_in_month(dt.year, dt.month);
        let prev_month = if dt.month == 1 { 12 } else { dt.month - 1 };
        let prev_year = if dt.month == 1 { dt.year - 1 } else { dt.year };
        let days_prev_month = days_in_month(prev_year, prev_month);

        let cell_y0 = grid_y + 18;
        for row in 0..6 {
            for col in 0..7 {
                let idx = row * 7 + col;
                let cx = grid_x + (col as i32) * col_w;
                let cy = cell_y0 + (row as i32) * 22;

                if idx < first_dow as usize {
                    // Previous month day
                    let d = days_prev_month - (first_dow as usize - 1 - idx) as u8;
                    let mut dbuf = [0u8; 8];
                    let mut doff = 0;
                    format_u32(d as u32, &mut dbuf, &mut doff);
                    let s_val = core::str::from_utf8(&dbuf[..doff]).unwrap_or("");
                    ui::font::draw_text_font(s, cx + 10, cy + 2, s_val, Color::rgb(80, 88, 98), &font_sm);
                } else if idx < first_dow as usize + days_this_month as usize {
                    // Current month day
                    let d = (idx - first_dow as usize + 1) as u8;
                    let mut dbuf = [0u8; 8];
                    let mut doff = 0;
                    format_u32(d as u32, &mut dbuf, &mut doff);
                    let s_val = core::str::from_utf8(&dbuf[..doff]).unwrap_or("");

                    if d == dt.day {
                        // Today highlight badge (electric cyan rounded box)
                        let badge = Rect::new(cx + 4, cy, 28, 20);
                        ui::draw::draw_rounded_rect(s, badge, 5, Color::rgb(0, 180, 216));
                        ui::font::draw_text_font(s, cx + if d < 10 { 12 } else { 8 }, cy + 3, s_val, Color::WHITE, &font_bold);
                    } else {
                        ui::font::draw_text_font(s, cx + if d < 10 { 12 } else { 8 }, cy + 3, s_val, palette.text_primary, &font_sm);
                    }
                } else {
                    // Next month day
                    let d = (idx - (first_dow as usize + days_this_month as usize) + 1) as u8;
                    let mut dbuf = [0u8; 8];
                    let mut doff = 0;
                    format_u32(d as u32, &mut dbuf, &mut doff);
                    let s_val = core::str::from_utf8(&dbuf[..doff]).unwrap_or("");
                    ui::font::draw_text_font(s, cx + 10, cy + 2, s_val, Color::rgb(80, 88, 98), &font_sm);
                }
            }
        }

        // Divider
        s.fill_rect(Rect::new(cr.x + 14, cr.y + 284, cr.w - 28, 1), Color::rgba(255, 255, 255, 25));

        // 11. Manual Time & Date Adjust Controls Bar
        let ctrl_y = cr.y + 294;
        // Hour: [-] [+]
        ui::font::draw_text_font(s, cr.x + 16, ctrl_y + 4, "Hour", palette.text_muted, &font_sm);
        let btn_h_min = Rect::new(cr.x + 48, ctrl_y, 20, 20);
        ui::draw::draw_rounded_rect(s, btn_h_min, 3, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_h_min.x + 6, btn_h_min.y + 3, "-", palette.text_primary, &font_bold);
        let btn_h_pls = Rect::new(cr.x + 72, ctrl_y, 20, 20);
        ui::draw::draw_rounded_rect(s, btn_h_pls, 3, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_h_pls.x + 5, btn_h_pls.y + 3, "+", palette.text_primary, &font_bold);

        // Min: [-] [+]
        ui::font::draw_text_font(s, cr.x + 106, ctrl_y + 4, "Min", palette.text_muted, &font_sm);
        let btn_m_min = Rect::new(cr.x + 134, ctrl_y, 20, 20);
        ui::draw::draw_rounded_rect(s, btn_m_min, 3, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_m_min.x + 6, btn_m_min.y + 3, "-", palette.text_primary, &font_bold);
        let btn_m_pls = Rect::new(cr.x + 158, ctrl_y, 20, 20);
        ui::draw::draw_rounded_rect(s, btn_m_pls, 3, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_m_pls.x + 5, btn_m_pls.y + 3, "+", palette.text_primary, &font_bold);

        // Day: [-] [+]
        ui::font::draw_text_font(s, cr.x + 192, ctrl_y + 4, "Day", palette.text_muted, &font_sm);
        let btn_d_min = Rect::new(cr.x + 220, ctrl_y, 20, 20);
        ui::draw::draw_rounded_rect(s, btn_d_min, 3, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_d_min.x + 6, btn_d_min.y + 3, "-", palette.text_primary, &font_bold);
        let btn_d_pls = Rect::new(cr.x + 244, ctrl_y, 20, 20);
        ui::draw::draw_rounded_rect(s, btn_d_pls, 3, Color::rgba(255, 255, 255, 25));
        ui::font::draw_text_font(s, btn_d_pls.x + 5, btn_d_pls.y + 3, "+", palette.text_primary, &font_bold);
    }
}

fn draw_modern_desktop_wallpaper<S: Surface + ?Sized>(s: &mut S, w: i32, h: i32) {
    // 1. High-precision continuous vertical celestial gradient
    // From deep cosmic midnight navy (#090D1A) to twilight sapphire-violet (#130F26)
    let step = 2; // 2px slices for ultra-smooth transition without contour rings
    let num_slices = (h + step - 1) / step;
    for i in 0..num_slices {
        let y = i * step;
        let slice_h = step.min(h - y);
        let t = (y * 255) / h.max(1);

        // Interpolate colors smoothly across the screen height
        let r = (9 * (255 - t) + 19 * t) / 255;
        let g = (13 * (255 - t) + 15 * t) / 255;
        let b = (26 * (255 - t) + 38 * t) / 255;
        s.fill_rect(Rect::new(0, y, w, slice_h), Color::rgb(r as u8, g as u8, b as u8));
    }

    // 2. Wide smooth atmospheric aurora sweeps across mid-screen
    // Seamless horizontal color bands blending cyan and royal violet without contour rings
    let sweep_center = (h * 55) / 100;
    let sweep_span = (h * 35) / 100;
    let sweep_step = 4;
    let sweep_slices = (sweep_span * 2) / sweep_step;

    for i in 0..sweep_slices {
        let y = sweep_center - sweep_span + (i * sweep_step);
        if y < 0 || y >= h {
            continue;
        }
        let dist = (y - sweep_center).abs();
        let falloff = ((sweep_span - dist).max(0) * 255) / sweep_span.max(1);
        let alpha = ((falloff * 24) / 255) as u8;

        if alpha > 0 {
            let cyan_w = (w * 65) / 100;
            let purp_w = w - cyan_w;
            s.fill_rect(Rect::new(0, y, purp_w, sweep_step), Color::rgba(139, 92, 246, (alpha * 8) / 10));
            s.fill_rect(Rect::new(purp_w, y, cyan_w, sweep_step), Color::rgba(14, 165, 233, alpha));
        }
    }

    // 3. Twinkling celestial diamond stars scattered across deep space
    const STARS: [(i32, i32, u8); 18] = [
        (85, 45, 2),  (160, 110, 1), (280, 70, 2),  (420, 140, 1),
        (540, 50, 3), (670, 95, 2),  (780, 60, 1),  (890, 130, 2),
        (960, 75, 2), (120, 220, 1), (340, 260, 2), (730, 240, 1),
        (850, 280, 2),(920, 210, 1), (210, 380, 1), (820, 410, 2),
        (150, 480, 1),(880, 490, 1),
    ];

    for &(sx, sy, brightness) in STARS.iter() {
        let actual_x = (sx * w) / 1024;
        let actual_y = (sy * h) / 768;
        match brightness {
            3 => {
                s.fill_rect(Rect::new(actual_x - 1, actual_y - 2, 3, 5), Color::rgba(255, 255, 255, 200));
                s.fill_rect(Rect::new(actual_x - 2, actual_y - 1, 5, 3), Color::rgba(255, 255, 255, 200));
                s.put_pixel(actual_x, actual_y, Color::WHITE);
            }
            2 => {
                s.put_pixel(actual_x, actual_y, Color::WHITE);
                s.put_pixel(actual_x - 1, actual_y, Color::rgba(125, 211, 252, 160));
                s.put_pixel(actual_x + 1, actual_y, Color::rgba(125, 211, 252, 160));
                s.put_pixel(actual_x, actual_y - 1, Color::rgba(125, 211, 252, 160));
                s.put_pixel(actual_x, actual_y + 1, Color::rgba(125, 211, 252, 160));
            }
            _ => {
                s.put_pixel(actual_x, actual_y, Color::rgba(226, 232, 240, 150));
            }
        }
    }
}

fn draw_realistic_app_icon<S: Surface + ?Sized>(
    s: &mut S,
    x: i32,
    y: i32,
    size: i32,
    icon_type: u8,
    palette: &ThemePalette,
) {
    match icon_type {
        0 => {
            // 0: This PC (Sleek modern workstation monitor with stand and glowing screen)
            let mon_w = (size * 80) / 100;
            let mon_h = (size * 52) / 100;
            let mon_x = x + (size - mon_w) / 2;
            let mon_y = y + 2;

            ui::draw::draw_rounded_rect(s, Rect::new(mon_x, mon_y, mon_w, mon_h), 3, Color::rgb(30, 41, 59));
            ui::draw::draw_rect_outline(s, Rect::new(mon_x, mon_y, mon_w, mon_h), 1, Color::rgb(71, 85, 105));

            let scr_inset = 2;
            let scr_rect = Rect::new(mon_x + scr_inset, mon_y + scr_inset, mon_w - scr_inset * 2, mon_h - scr_inset * 2 - 2);
            s.fill_rect(scr_rect, Color::rgb(14, 165, 233));
            s.fill_rect(Rect::new(scr_rect.x, scr_rect.y + scr_rect.h / 2, scr_rect.w, scr_rect.h / 2), Color::rgb(59, 130, 246));
            s.put_pixel(mon_x + mon_w / 2, mon_y + mon_h - 2, Color::rgb(34, 197, 94));

            let neck_w = (size * 18) / 100;
            let neck_h = (size * 22) / 100;
            let neck_x = x + (size - neck_w) / 2;
            let neck_y = mon_y + mon_h;
            s.fill_rect(Rect::new(neck_x, neck_y, neck_w, neck_h), Color::rgb(203, 213, 225));

            let base_w = (size * 55) / 100;
            let base_h = 3;
            let base_x = x + (size - base_w) / 2;
            let base_y = neck_y + neck_h - 1;
            ui::draw::draw_rounded_rect(s, Rect::new(base_x, base_y, base_w, base_h), 1, Color::rgb(226, 232, 240));
        }
        1 => {
            // 1: Nova Terminal (Dark obsidian console with 3 mini traffic lights and emerald prompt >_)
            let term_w = (size * 82) / 100;
            let term_h = (size * 78) / 100;
            let term_x = x + (size - term_w) / 2;
            let term_y = y + (size - term_h) / 2;

            ui::draw::draw_rounded_rect(s, Rect::new(term_x, term_y, term_w, term_h), 4, Color::rgb(15, 23, 42));
            ui::draw::draw_rect_outline(s, Rect::new(term_x, term_y, term_w, term_h), 1, Color::rgb(56, 189, 248));

            let bar_h = (term_h * 24) / 100;
            s.fill_rect(Rect::new(term_x + 1, term_y + 1, term_w - 2, bar_h), Color::rgb(30, 41, 59));
            s.put_pixel(term_x + 4, term_y + bar_h / 2 + 1, Color::rgb(239, 68, 68));
            s.put_pixel(term_x + 8, term_y + bar_h / 2 + 1, Color::rgb(234, 179, 8));
            s.put_pixel(term_x + 12, term_y + bar_h / 2 + 1, Color::rgb(34, 197, 94));

            let px = term_x + 6;
            let py = term_y + bar_h + 4;
            s.fill_rect(Rect::new(px, py, 2, 2), Color::rgb(34, 197, 94));
            s.fill_rect(Rect::new(px + 2, py + 2, 2, 2), Color::rgb(34, 197, 94));
            s.fill_rect(Rect::new(px, py + 4, 2, 2), Color::rgb(34, 197, 94));
            s.fill_rect(Rect::new(px + 6, py + 5, 4, 2), Color::rgb(240, 246, 252));
        }
        2 => {
            // 2: Files / Folder (Golden amber folder with tab and white document peeking out)
            let fld_w = (size * 84) / 100;
            let fld_h = (size * 70) / 100;
            let fld_x = x + (size - fld_w) / 2;
            let fld_y = y + (size - fld_h) / 2 + 2;

            let tab_w = (fld_w * 45) / 100;
            let tab_h = 4;
            ui::draw::draw_rounded_rect(s, Rect::new(fld_x + 2, fld_y - tab_h + 1, tab_w, tab_h + 2), 2, Color::rgb(217, 119, 6));
            ui::draw::draw_rounded_rect(s, Rect::new(fld_x, fld_y, fld_w, fld_h), 3, Color::rgb(217, 119, 6));

            let doc_w = (fld_w * 65) / 100;
            let doc_h = (fld_h * 50) / 100;
            let doc_x = fld_x + (fld_w - doc_w) / 2;
            let doc_y = fld_y - 2;
            s.fill_rect(Rect::new(doc_x, doc_y, doc_w, doc_h), Color::WHITE);
            s.fill_rect(Rect::new(doc_x + 2, doc_y + 3, doc_w - 4, 1), Color::rgb(59, 130, 246));
            s.fill_rect(Rect::new(doc_x + 2, doc_y + 6, doc_w - 6, 1), Color::rgb(148, 163, 184));

            let flap_h = (fld_h * 68) / 100;
            let flap_y = fld_y + fld_h - flap_h;
            ui::draw::draw_rounded_rect(s, Rect::new(fld_x, flap_y, fld_w, flap_h), 3, Color::rgb(245, 158, 11));
            ui::draw::draw_rect_outline(s, Rect::new(fld_x, flap_y, fld_w, flap_h), 1, Color::rgb(251, 191, 36));
        }
        3 => {
            // 3: README / Scratchpad (Crisp document with dog-ear fold and colored text lines)
            let doc_w = (size * 68) / 100;
            let doc_h = (size * 82) / 100;
            let doc_x = x + (size - doc_w) / 2;
            let doc_y = y + (size - doc_h) / 2;

            ui::draw::draw_rounded_rect(s, Rect::new(doc_x, doc_y, doc_w, doc_h), 2, Color::WHITE);
            ui::draw::draw_rect_outline(s, Rect::new(doc_x, doc_y, doc_w, doc_h), 1, Color::rgb(203, 213, 225));

            let fold = (doc_w * 28) / 100;
            s.fill_rect(Rect::new(doc_x + doc_w - fold, doc_y, fold, fold), Color::rgb(226, 232, 240));
            s.fill_rect(Rect::new(doc_x + doc_w - fold, doc_y + fold, fold, 1), Color::rgb(148, 163, 184));
            s.fill_rect(Rect::new(doc_x + doc_w - fold, doc_y, 1, fold), Color::rgb(148, 163, 184));

            s.fill_rect(Rect::new(doc_x + 3, doc_y + 4, doc_w - fold - 4, 3), Color::rgb(14, 165, 233));

            let l_pad = 4;
            s.fill_rect(Rect::new(doc_x + l_pad, doc_y + 11, doc_w - l_pad * 2, 2), Color::rgb(99, 102, 241));
            s.fill_rect(Rect::new(doc_x + l_pad, doc_y + 16, doc_w - l_pad * 2 - 4, 2), Color::rgb(16, 185, 129));
            s.fill_rect(Rect::new(doc_x + l_pad, doc_y + 21, doc_w - l_pad * 2 - 2, 2), Color::rgb(217, 119, 6));
            s.fill_rect(Rect::new(doc_x + l_pad, doc_y + 26, (doc_w * 40) / 100, 2), Color::rgb(148, 163, 184));
        }
        4 => {
            // 4: Software Store / App Packages (Royal purple store parcel with gold star badge)
            let pkg_w = (size * 78) / 100;
            let pkg_h = (size * 75) / 100;
            let pkg_x = x + (size - pkg_w) / 2;
            let pkg_y = y + (size - pkg_h) / 2 + 2;

            let hdl_w = (pkg_w * 45) / 100;
            let hdl_h = 5;
            let hdl_x = pkg_x + (pkg_w - hdl_w) / 2;
            ui::draw::draw_rect_outline(s, Rect::new(hdl_x, pkg_y - hdl_h + 1, hdl_w, hdl_h + 2), 1, Color::WHITE);

            ui::draw::draw_rounded_rect(s, Rect::new(pkg_x, pkg_y, pkg_w, pkg_h), 3, Color::rgb(126, 34, 206));
            ui::draw::draw_rect_outline(s, Rect::new(pkg_x, pkg_y, pkg_w, pkg_h), 1, Color::rgb(168, 85, 247));

            let star_cx = pkg_x + pkg_w / 2;
            let star_cy = pkg_y + pkg_h / 2 + 1;
            s.fill_rect(Rect::new(star_cx - 1, star_cy - 4, 3, 9), Color::rgb(250, 204, 21));
            s.fill_rect(Rect::new(star_cx - 4, star_cy - 1, 9, 3), Color::rgb(250, 204, 21));
            s.put_pixel(star_cx, star_cy, Color::WHITE);
        }
        5 => {
            // 5: Settings / Control Panel (Dual precision interlocking metallic gears)
            let gear_sz = (size * 80) / 100;
            let gx = x + (size - gear_sz) / 2;
            let gy = y + (size - gear_sz) / 2;

            let r1 = (gear_sz * 60) / 100;
            let cx1 = gx + r1 / 2 + 2;
            let cy1 = gy + r1 / 2 + 2;
            s.fill_rect(Rect::new(cx1 - 2, cy1 - r1 / 2 - 2, 5, r1 + 4), Color::rgb(148, 163, 184));
            s.fill_rect(Rect::new(cx1 - r1 / 2 - 2, cy1 - 2, r1 + 4, 5), Color::rgb(148, 163, 184));
            ui::draw::draw_rounded_rect(s, Rect::new(cx1 - r1 / 2, cy1 - r1 / 2, r1, r1), r1 / 2, Color::rgb(100, 116, 139));
            s.fill_rect(Rect::new(cx1 - 2, cy1 - 2, 4, 4), Color::rgb(15, 23, 42));

            let r2 = (gear_sz * 45) / 100;
            let cx2 = gx + gear_sz - r2 / 2 - 1;
            let cy2 = gy + gear_sz - r2 / 2 - 1;
            s.fill_rect(Rect::new(cx2 - 1, cy2 - r2 / 2 - 1, 3, r2 + 2), Color::rgb(203, 213, 225));
            s.fill_rect(Rect::new(cx2 - r2 / 2 - 1, cy2 - 1, r2 + 2, 3), Color::rgb(203, 213, 225));
            ui::draw::draw_rounded_rect(s, Rect::new(cx2 - r2 / 2, cy2 - r2 / 2, r2, r2), r2 / 2, Color::rgb(148, 163, 184));
            s.fill_rect(Rect::new(cx2 - 1, cy2 - 1, 3, 3), Color::rgb(59, 130, 246));
        }
        6 => {
            // 6: Calendar (Realistic 3D calendar with crimson header, binder rings, date '19', colored category dots)
            let cal_w = (size * 82) / 100;
            let cal_h = (size * 82) / 100;
            let cal_x = x + (size - cal_w) / 2;
            let cal_y = y + (size - cal_h) / 2;

            // Page drop shadow
            s.fill_rect(Rect::new(cal_x + 2, cal_y + 2, cal_w, cal_h), Color::rgba(0, 0, 0, 45));

            // Calendar page body (ivory paper)
            ui::draw::draw_rounded_rect(s, Rect::new(cal_x, cal_y, cal_w, cal_h), 4, Color::rgb(248, 250, 252));
            ui::draw::draw_rect_outline(s, Rect::new(cal_x, cal_y, cal_w, cal_h), 1, Color::rgb(203, 213, 225));

            // Crimson / Coral Header Banner
            let hdr_h = (cal_h * 32) / 100;
            ui::draw::draw_rounded_rect(s, Rect::new(cal_x, cal_y, cal_w, hdr_h), 3, Color::rgb(225, 29, 72));
            s.fill_rect(Rect::new(cal_x + 1, cal_y + 1, cal_w - 2, 1), Color::rgb(244, 63, 94)); // top sheen

            // Metallic Twin Binder Rings
            let ring_w = 3;
            let ring_h = 5;
            let r1_x = cal_x + (cal_w * 25) / 100;
            let r2_x = cal_x + (cal_w * 75) / 100 - ring_w;
            let r_y = cal_y - 2;
            ui::draw::draw_rounded_rect(s, Rect::new(r1_x, r_y, ring_w, ring_h), 1, Color::rgb(226, 232, 240));
            ui::draw::draw_rounded_rect(s, Rect::new(r2_x, r_y, ring_w, ring_h), 1, Color::rgb(226, 232, 240));

            // Bold Day Number "19" in center
            let font_reg = Font::regular();
            let d_sz = font_reg.measure_str("19");
            let dx = cal_x + (cal_w - d_sz.w) / 2;
            let dy = cal_y + hdr_h + (cal_h - hdr_h - d_sz.h) / 2 - 2;
            ui::font::draw_text_font(s, dx, dy, "19", Color::rgb(15, 23, 42), &font_reg);

            // Colored event category indicator dots at bottom (Emerald, Royal Purple, Amber)
            let dot_y = cal_y + cal_h - 5;
            let dot_cx = cal_x + cal_w / 2;
            s.fill_rect(Rect::new(dot_cx - 6, dot_y, 3, 3), Color::rgb(16, 185, 129));
            s.fill_rect(Rect::new(dot_cx - 1, dot_y, 3, 3), Color::rgb(139, 92, 246));
            s.fill_rect(Rect::new(dot_cx + 4, dot_y, 3, 3), Color::rgb(245, 158, 11));
        }
        7 => {
            // 7: Calculator (Realistic 3D matte slate chassis, LCD screen, warm amber operator keys)
            let calc_w = (size * 82) / 100;
            let calc_h = (size * 86) / 100;
            let calc_x = x + (size - calc_w) / 2;
            let calc_y = y + (size - calc_h) / 2;

            // Chassis drop shadow
            s.fill_rect(Rect::new(calc_x + 2, calc_y + 2, calc_w, calc_h), Color::rgba(0, 0, 0, 50));

            // Main chassis (Obsidian / Cosmic Slate)
            ui::draw::draw_rounded_rect(s, Rect::new(calc_x, calc_y, calc_w, calc_h), 4, Color::rgb(30, 36, 52));
            ui::draw::draw_rect_outline(s, Rect::new(calc_x, calc_y, calc_w, calc_h), 1, Color::rgb(56, 66, 94));

            // Digital LCD readout screen at top
            let scr_h = (calc_h * 26) / 100;
            let scr_w = calc_w - 6;
            ui::draw::draw_rounded_rect(s, Rect::new(calc_x + 3, calc_y + 3, scr_w, scr_h), 2, Color::rgb(10, 12, 18));
            ui::draw::draw_rect_outline(s, Rect::new(calc_x + 3, calc_y + 3, scr_w, scr_h), 1, Color::rgb(24, 30, 44));

            // LCD numbers readout
            let font_sm = Font::small();
            ui::font::draw_text_font(s, calc_x + scr_w - 18, calc_y + 4, "123", Color::rgb(0, 212, 255), &font_sm);

            // Tactile keypad grid: 3 rows x 3 columns
            let k_y = calc_y + scr_h + 5;
            let k_pad = 2;
            let btn_w = (calc_w - 6 - k_pad * 2) / 3;
            let btn_h = (calc_h - scr_h - 10 - k_pad * 2) / 3;

            for r in 0..3 {
                for c in 0..3 {
                    let bx = calc_x + 3 + c * (btn_w + k_pad);
                    let by = k_y + r * (btn_h + k_pad);
                    let is_op = c == 2;
                    let is_eq = r == 2 && c == 2;
                    let bg = if is_eq {
                        Color::rgb(234, 88, 12)  // Ubuntu Auburn '='
                    } else if is_op {
                        Color::rgb(217, 119, 6)  // macOS Warm Amber '+'
                    } else {
                        Color::rgb(45, 53, 76)   // Slate digit
                    };
                    ui::draw::draw_rounded_rect(s, Rect::new(bx, by, btn_w, btn_h), 1, bg);
                }
            }
        }
        8 => {
            // 8: Aero Browser (Sleek deep blue container, golden/orange privacy shield, cyan orbital ring)
            let b_w = (size * 84) / 100;
            let b_h = (size * 84) / 100;
            let b_x = x + (size - b_w) / 2;
            let b_y = y + (size - b_h) / 2;

            s.fill_rect(Rect::new(b_x + 2, b_y + 2, b_w, b_h), Color::rgba(0, 0, 0, 50));
            ui::draw::draw_rounded_rect(s, Rect::new(b_x, b_y, b_w, b_h), 6, Color::rgb(15, 23, 42));
            ui::draw::draw_rect_outline(s, Rect::new(b_x, b_y, b_w, b_h), 1, Color::rgb(51, 65, 85));

            let mid_x = b_x + b_w / 2;
            let mid_y = b_y + b_h / 2;
            ui::draw::draw_rect_outline(s, Rect::new(b_x + 4, b_y + 4, b_w - 8, b_h - 8), 1, Color::rgb(56, 189, 248));

            let sh_w = (b_w * 50) / 100;
            let sh_h = (b_h * 55) / 100;
            let sh_x = b_x + (b_w - sh_w) / 2;
            let sh_y = b_y + (b_h - sh_h) / 2;
            ui::draw::draw_rounded_rect(s, Rect::new(sh_x, sh_y, sh_w, sh_h), 3, Color::rgb(255, 107, 0));
            s.fill_rect(Rect::new(sh_x + 2, sh_y + 2, sh_w - 4, 3), Color::rgb(254, 215, 170));
            s.put_pixel(mid_x, mid_y, Color::rgb(74, 222, 128));
        }
        9 => {
            // 9: Spectra Media Studio (Obsidian cinema chassis, electric cyan play badge, amber status dot)
            let b_w = (size * 84) / 100;
            let b_h = (size * 84) / 100;
            let b_x = x + (size - b_w) / 2;
            let b_y = y + (size - b_h) / 2;

            s.fill_rect(Rect::new(b_x + 2, b_y + 2, b_w, b_h), Color::rgba(0, 0, 0, 50));
            ui::draw::draw_rounded_rect(s, Rect::new(b_x, b_y, b_w, b_h), 6, Color::rgb(14, 23, 42));
            ui::draw::draw_rect_outline(s, Rect::new(b_x, b_y, b_w, b_h), 1, Color::rgb(56, 189, 248));

            let mid_x = b_x + b_w / 2;
            let mid_y = b_y + b_h / 2;
            ui::draw::draw_line(s, mid_x - 3, mid_y - 6, mid_x - 3, mid_y + 6, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, mid_x - 2, mid_y - 5, mid_x + 4, mid_y, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, mid_x - 2, mid_y + 5, mid_x + 4, mid_y, Color::rgb(56, 189, 248));
            s.put_pixel(b_x + b_w - 6, b_y + 5, Color::rgb(245, 158, 11));
        }
        10 => {
            // 10: Vellum PDF Studio (Archival crimson shield, parchment sheet, folded corner, cyan 'V')
            let b_w = (size * 84) / 100;
            let b_h = (size * 84) / 100;
            let b_x = x + (size - b_w) / 2;
            let b_y = y + (size - b_h) / 2;

            s.fill_rect(Rect::new(b_x + 2, b_y + 2, b_w, b_h), Color::rgba(0, 0, 0, 50));
            ui::draw::draw_rounded_rect(s, Rect::new(b_x, b_y, b_w, b_h), 6, Color::rgb(190, 24, 93));
            ui::draw::draw_rect_outline(s, Rect::new(b_x, b_y, b_w, b_h), 1, Color::rgb(251, 113, 133));

            let p_w = (b_w * 65) / 100;
            let p_h = (b_h * 75) / 100;
            let p_x = b_x + (b_w - p_w) / 2;
            let p_y = b_y + (b_h - p_h) / 2;
            ui::draw::draw_rounded_rect(s, Rect::new(p_x, p_y, p_w, p_h), 2, Color::WHITE);
            s.fill_rect(Rect::new(p_x + p_w - 5, p_y, 5, 5), Color::rgb(226, 232, 240));
            ui::draw::draw_line(s, p_x + p_w - 5, p_y, p_x + p_w, p_y + 5, Color::rgb(148, 163, 184));
            s.fill_rect(Rect::new(p_x + 3, p_y + 6, p_w - 8, 2), Color::rgb(225, 29, 72));
            s.fill_rect(Rect::new(p_x + 3, p_y + 10, p_w - 6, 2), Color::rgb(148, 163, 184));
            s.fill_rect(Rect::new(p_x + 3, p_y + 14, p_w - 8, 2), Color::rgb(148, 163, 184));
            ui::font::draw_text_font(s, p_x + 3, p_y + 7, "V", Color::rgb(14, 165, 233), &Font::small());
        }
        11 => {
            // 11: Vortex BitTorrent & Stream Studio (Electric indigo/cyan badge with vortex chevron)
            let b_w = (size * 84) / 100;
            let b_h = (size * 84) / 100;
            let b_x = x + (size - b_w) / 2;
            let b_y = y + (size - b_h) / 2;

            s.fill_rect(Rect::new(b_x + 2, b_y + 2, b_w, b_h), Color::rgba(0, 0, 0, 50));
            ui::draw::draw_rounded_rect(s, Rect::new(b_x, b_y, b_w, b_h), 6, Color::rgb(15, 23, 42));
            ui::draw::draw_rect_outline(s, Rect::new(b_x, b_y, b_w, b_h), 1, Color::rgb(99, 102, 241));

            let mid_x = b_x + b_w / 2;
            let mid_y = b_y + b_h / 2;
            ui::draw::draw_line(s, mid_x - 5, mid_y - 3, mid_x, mid_y + 3, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, mid_x + 5, mid_y - 3, mid_x, mid_y + 3, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, mid_x, mid_y - 6, mid_x, mid_y + 3, Color::rgb(56, 189, 248));
            s.fill_rect(Rect::new(mid_x - 5, mid_y + 6, 11, 2), Color::rgb(99, 102, 241));
            s.put_pixel(b_x + 4, b_y + 4, Color::rgb(74, 222, 128));
            s.put_pixel(b_x + b_w - 5, b_y + 4, Color::rgb(245, 158, 11));
        }
        12 => {
            // 12: Zenith Office Suite (Fluent 4-quadrant tiles: Blue Writer, Green Calc, Orange Present, Wine Base)
            let b_w = (size * 84) / 100;
            let b_h = (size * 84) / 100;
            let b_x = x + (size - b_w) / 2;
            let b_y = y + (size - b_h) / 2;

            s.fill_rect(Rect::new(b_x + 2, b_y + 2, b_w, b_h), Color::rgba(0, 0, 0, 50));
            ui::draw::draw_rounded_rect(s, Rect::new(b_x, b_y, b_w, b_h), 6, Color::rgb(15, 23, 42));
            ui::draw::draw_rect_outline(s, Rect::new(b_x, b_y, b_w, b_h), 1, Color::rgb(0, 120, 212));

            let q_w = (b_w - 6) / 2;
            let q_h = (b_h - 6) / 2;
            // Top-left: Word/Writer blue
            s.fill_rect(Rect::new(b_x + 2, b_y + 2, q_w, q_h), Color::rgb(0, 120, 212));
            // Top-right: Excel/Calc green
            s.fill_rect(Rect::new(b_x + 4 + q_w, b_y + 2, q_w, q_h), Color::rgb(16, 124, 65));
            // Bottom-left: PowerPoint/Present orange
            s.fill_rect(Rect::new(b_x + 2, b_y + 4 + q_h, q_w, q_h), Color::rgb(216, 59, 1));
            // Bottom-right: Access/Base crimson
            s.fill_rect(Rect::new(b_x + 4 + q_w, b_y + 4 + q_h, q_w, q_h), Color::rgb(168, 0, 0));

            ui::font::draw_char_font(s, b_x + 4, b_y + 3, b'W', Color::WHITE, &Font::small());
            ui::font::draw_char_font(s, b_x + 6 + q_w, b_y + 3, b'X', Color::WHITE, &Font::small());
            ui::font::draw_char_font(s, b_x + 4, b_y + 5 + q_h, b'P', Color::WHITE, &Font::small());
            ui::font::draw_char_font(s, b_x + 6 + q_w, b_y + 5 + q_h, b'B', Color::WHITE, &Font::small());
        }
        _ => {
            draw_app_icon(s, x + (size - 14) / 2, y + (size - 14) / 2, icon_type, palette);
        }
    }
}

fn draw_desktop_icons<S: Surface + ?Sized>(
    s: &mut S,
    menubar_h: i32,
    hovered_icon: Option<usize>,
    palette: &ThemePalette,
) {
    let font = Font::small();
    let start_y = if menubar_h > 0 { menubar_h + 16 } else { 16 };
    let items_per_col = 6;
    for (i, spec) in DESKTOP_SHORTCUTS.iter().enumerate() {
        let col = (i / items_per_col) as i32;
        let row = (i % items_per_col) as i32;
        let x = 18 + col * 86;
        let y = start_y + row * 78;
        let rect = Rect::new(x, y, 78, 70);

        let is_hovered = hovered_icon == Some(i);
        if is_hovered {
            ui::draw::draw_rounded_rect(s, rect, 8, Color::rgba(255, 255, 255, 32));
            ui::draw::draw_rect_outline(s, rect, 1, Color::rgba(255, 255, 255, 65));
        }

        // Draw realistic 36x36 icon in top portion of card
        let icon_sz = 36;
        let icon_x = x + (rect.w - icon_sz) / 2;
        let icon_y = y + 4;
        draw_realistic_app_icon(s, icon_x, icon_y, icon_sz, spec.icon_type, palette);

        // Label text below icon with crisp drop shadow
        let label_w = font.measure_str(spec.label).w;
        let text_x = x + ((rect.w - label_w) / 2).max(2);
        let text_y = icon_y + icon_sz + 4;
        ui::font::draw_text_font(s, text_x + 1, text_y + 1, spec.label, Color::BLACK, &font);
        ui::font::draw_text_font(s, text_x, text_y, spec.label, Color::WHITE, &font);
    }
}

fn draw_app_icon<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, label: u8, palette: &ThemePalette) {
    match label {
        1 => {
            // Terminal prompt `>_`
            ui::draw::draw_rounded_rect(s, Rect::new(x, y, 12, 12), 2, palette.surface_sunken);
            ui::draw::draw_line(s, x + 3, y + 3, x + 5, y + 5, palette.status_success);
            ui::draw::draw_line(s, x + 5, y + 5, x + 3, y + 7, palette.status_success);
            s.fill_rect(Rect::new(x + 7, y + 7, 3, 1), Color::WHITE);
        }
        2 => {
            // Folder icon
            let folder_col = Color::rgb(245, 180, 50);
            s.fill_rect(Rect::new(x + 2, y + 2, 4, 2), folder_col.lighten(20));
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 4, 10, 7), 1, folder_col);
        }
        3 => {
            // Document sheet icon
            let doc_col = Color::WHITE;
            ui::draw::draw_rounded_rect(s, Rect::new(x + 2, y + 1, 8, 10), 1, doc_col);
            s.put_pixel(x + 9, y + 1, palette.surface_sunken);
            s.fill_rect(Rect::new(x + 4, y + 4, 4, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 4, y + 6, 4, 1), palette.text_muted);
            s.fill_rect(Rect::new(x + 4, y + 8, 3, 1), palette.text_muted);
        }
        4 => {
            // Store shopping bag / package icon
            ui::dock::draw_dock_app_icon(s, x, y, ui::dock::APP_ID_STORE, palette);
        }
        5 => {
            // Settings sliders icon
            ui::dock::draw_dock_app_icon(s, x, y, 5, palette);
        }
        6 => {
            // Calendar icon: red header bar + white card + colored dots
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::WHITE);
            s.fill_rect(Rect::new(x + 1, y + 1, 10, 3), Color::rgb(225, 29, 72));
            s.put_pixel(x + 3, y + 7, Color::rgb(14, 165, 233));
            s.put_pixel(x + 6, y + 7, Color::rgb(245, 158, 11));
            s.put_pixel(x + 9, y + 7, Color::rgb(34, 197, 94));
        }
        7 => {
            // Calculator icon: slate chassis, cyan LCD line, warm amber operator dot
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::rgb(30, 36, 52));
            s.fill_rect(Rect::new(x + 2, y + 2, 8, 2), Color::rgb(0, 212, 255));
            s.put_pixel(x + 3, y + 6, Color::rgb(203, 213, 225));
            s.put_pixel(x + 5, y + 6, Color::rgb(203, 213, 225));
            s.put_pixel(x + 8, y + 6, Color::rgb(217, 119, 6));
            s.put_pixel(x + 3, y + 8, Color::rgb(203, 213, 225));
            s.put_pixel(x + 5, y + 8, Color::rgb(203, 213, 225));
            s.put_pixel(x + 8, y + 8, Color::rgb(234, 88, 12));
        }
        8 => {
            // Aero Browser: celestial deep blue rounded badge, orange shield with cyan lock
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, x + 3, y + 3, x + 8, y + 3, Color::rgb(255, 107, 0));
            ui::draw::draw_line(s, x + 3, y + 4, x + 3, y + 7, Color::rgb(255, 107, 0));
            ui::draw::draw_line(s, x + 8, y + 4, x + 8, y + 7, Color::rgb(255, 107, 0));
            ui::draw::draw_line(s, x + 3, y + 7, x + 6, y + 9, Color::rgb(255, 107, 0));
            ui::draw::draw_line(s, x + 8, y + 7, x + 6, y + 9, Color::rgb(255, 107, 0));
            s.put_pixel(x + 5, y + 5, Color::rgb(0, 212, 255));
            s.put_pixel(x + 6, y + 5, Color::rgb(0, 212, 255));
        }
        9 => {
            // Nova Player: cyber-cyan rounded badge with play glyph and amber status dot
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::rgb(14, 23, 42));
            ui::draw::draw_line(s, x + 4, y + 3, x + 4, y + 9, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, x + 5, y + 4, x + 5, y + 8, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, x + 6, y + 5, x + 6, y + 7, Color::rgb(56, 189, 248));
            s.put_pixel(x + 7, y + 6, Color::rgb(56, 189, 248));
            s.put_pixel(x + 9, y + 2, Color::rgb(245, 158, 11));
        }
        10 => {
            // Vellum PDF Studio mini icon: Crimson paper with folded corner & 'V'
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::rgb(225, 29, 72));
            s.fill_rect(Rect::new(x + 3, y + 3, 6, 6), Color::WHITE);
            s.put_pixel(x + 5, y + 5, Color::rgb(14, 165, 233));
            s.put_pixel(x + 6, y + 5, Color::rgb(14, 165, 233));
        }
        11 => {
            // Vortex BitTorrent & Stream Studio: Electric indigo/cyan badge with download chevron
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, x + 3, y + 4, x + 6, y + 7, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, x + 9, y + 4, x + 6, y + 7, Color::rgb(56, 189, 248));
            ui::draw::draw_line(s, x + 6, y + 2, x + 6, y + 7, Color::rgb(56, 189, 248));
            s.fill_rect(Rect::new(x + 3, y + 9, 7, 1), Color::rgb(99, 102, 241));
        }
        12 => {
            // Zenith Office Suite: 4 quadrant tiles (Blue, Green, Orange, Crimson)
            ui::draw::draw_rounded_rect(s, Rect::new(x + 1, y + 1, 10, 10), 2, Color::rgb(15, 23, 42));
            s.fill_rect(Rect::new(x + 2, y + 2, 4, 4), Color::rgb(0, 120, 212));
            s.fill_rect(Rect::new(x + 6, y + 2, 4, 4), Color::rgb(16, 124, 65));
            s.fill_rect(Rect::new(x + 2, y + 6, 4, 4), Color::rgb(216, 59, 1));
            s.fill_rect(Rect::new(x + 6, y + 6, 4, 4), Color::rgb(168, 0, 0));
        }
        _ => {
            // System gear icon
            let gear_col = Color::rgb(140, 160, 180);
            s.fill_rect(Rect::new(x + 4, y + 2, 4, 8), gear_col);
            s.fill_rect(Rect::new(x + 2, y + 4, 8, 4), gear_col);
            s.fill_rect(Rect::new(x + 5, y + 5, 2, 2), palette.titlebar_active);
        }
    }
}

/// Renders a graphical file manager view inside the client area of Window 1.
fn draw_files_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, palette: &ThemePalette) {
    let f_small = Font::small();

    // 1. Client toolbar
    let tb_h = 24;
    let tb_rect = Rect::new(client.x, client.y, client.w, tb_h);
    s.fill_rect(tb_rect, palette.surface_raised);
    s.fill_rect(Rect::new(client.x, tb_rect.bottom() - 1, client.w, 1), palette.border_subtle);

    // Path pill
    let path_rect = Rect::new(client.x + 6, client.y + 3, 64, 18);
    ui::draw::draw_rounded_rect(s, path_rect, 2, palette.surface_sunken);
    ui::draw::draw_rect_outline(s, path_rect, 1, palette.border_subtle);
    ui::font::draw_text_font(s, path_rect.x + 6, path_rect.y + 2, "/root", palette.text_primary, &f_small);

    // Mini buttons: Open, Move, Delete
    let btns: &[(&str, Color)] = &[
        ("Open", palette.accent_primary),
        ("Move", palette.surface_raised.lighten(30)),
        ("Delete", palette.status_danger),
    ];
    let mut bx = path_rect.right() + 8;
    for &(label, col) in btns {
        if bx + 42 > client.right() - 4 {
            break;
        }
        let b_rect = Rect::new(bx, client.y + 3, 40, 18);
        ui::draw::draw_rounded_rect(s, b_rect, 2, col);
        let sz = f_small.measure_str(label);
        let tx = b_rect.x + (b_rect.w - sz.w) / 2;
        let ty = b_rect.y + (b_rect.h - f_small.line_height()) / 2;
        ui::font::draw_text_font(s, tx, ty, label, Color::WHITE, &f_small);
        bx += 44;
    }

    // 2. Table Column Headers
    let hdr_y = tb_rect.bottom();
    let hdr_h = 18;
    let hdr_rect = Rect::new(client.x, hdr_y, client.w, hdr_h);
    s.fill_rect(hdr_rect, palette.surface_raised.darken(10));
    s.fill_rect(Rect::new(client.x, hdr_rect.bottom() - 1, client.w, 1), palette.border_subtle);

    ui::font::draw_text_font(s, client.x + 24, hdr_y + 2, "Name", palette.text_muted, &f_small);
    if client.w >= 200 {
        ui::font::draw_text_font(s, client.x + 130, hdr_y + 2, "Size", palette.text_muted, &f_small);
    }
    if client.w >= 280 {
        ui::font::draw_text_font(s, client.x + 190, hdr_y + 2, "Type", palette.text_muted, &f_small);
    }

    // 3. File Rows
    let rows: &[(&str, &str, &str, Color)] = &[
        ("HELLO.TXT",     "68 B",   "Document", Color::WHITE),
        ("SHELL.ELF",     "4.0 KB", "Executable", palette.status_success),
        ("SYSTEM.CFG",    "512 B",  "Config", Color::rgb(140, 190, 240)),
        ("FIREWALL.CFG",  "1.2 KB", "Config", Color::rgb(140, 190, 240)),
    ];

    let mut ry = hdr_rect.bottom() + 1;
    let row_h = 20;
    for (idx, &(name, size, kind, icon_col)) in rows.iter().enumerate() {
        if ry + row_h > client.bottom() - 18 {
            break;
        }
        // Selection on row 0
        if idx == 0 {
            s.fill_rect(Rect::new(client.x + 1, ry, client.w - 2, row_h), palette.accent_primary.darken(40));
        }

        // Mini file icon
        ui::draw::draw_rounded_rect(s, Rect::new(client.x + 8, ry + 4, 8, 10), 1, icon_col);

        // Name
        let name_col = if idx == 0 { Color::WHITE } else { palette.text_primary };
        ui::font::draw_text_font(s, client.x + 24, ry + 3, name, name_col, &f_small);

        // Size
        if client.w >= 200 {
            ui::font::draw_text_font(s, client.x + 130, ry + 3, size, palette.text_secondary, &f_small);
        }

        // Type
        if client.w >= 280 {
            ui::font::draw_text_font(s, client.x + 190, ry + 3, kind, palette.text_secondary, &f_small);
        }

        ry += row_h;
    }

    // 4. Status Bar at bottom
    if client.h >= 100 {
        let sb_h = 18;
        let sb_rect = Rect::new(client.x, client.bottom() - sb_h, client.w, sb_h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(client.x, sb_rect.y, client.w, 1), palette.border_subtle);
        ui::font::draw_text_font(s, client.x + 8, sb_rect.y + 2, "4 files | 5.8 KB", palette.text_muted, &f_small);
    }
}

/// Renders a graphical settings control panel view inside the client area of Window 4 / Settings.
fn draw_settings_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, palette: &ThemePalette) {
    let f_small = Font::small();
    let font = Font::regular();

    // 1. Sidebar with category badge tabs (w = 84)
    let sb_w = 84;
    let sb_rect = Rect::new(client.x, client.y, sb_w, client.h);
    s.fill_rect(sb_rect, palette.surface_raised);
    s.fill_rect(Rect::new(sb_rect.right() - 1, sb_rect.y, 1, sb_rect.h), palette.border_subtle);

    let tabs: &[(&str, &str)] = &[
        ("SYS",  "System"),
        ("USR",  "Users"),
        ("SVC",  "Services"),
        ("FW",   "Firewall"),
        ("DISP", "Display"),
    ];

    let mut ty = client.y + 6;
    let tab_h = 22;
    for (i, &(badge, name)) in tabs.iter().enumerate() {
        if ty + tab_h > client.bottom() - 18 {
            break;
        }
        let t_rect = Rect::new(client.x + 4, ty, sb_w - 8, tab_h);
        if i == 0 {
            ui::draw::draw_rounded_rect(s, t_rect, 2, palette.accent_primary.darken(35));
            ui::draw::draw_rect_outline(s, t_rect, 1, palette.border_focused);
            s.fill_rect(Rect::new(t_rect.x + 1, t_rect.y + 3, 2, t_rect.h - 6), palette.accent_primary);
        }

        let b_sz = f_small.measure_str(badge);
        let b_rect = Rect::new(t_rect.x + 4, t_rect.y + 3, b_sz.w + 4, t_rect.h - 6);
        let b_bg = if i == 0 { palette.accent_primary } else { palette.surface_sunken };
        ui::draw::draw_rounded_rect(s, b_rect, 2, b_bg);
        ui::font::draw_text_font(s, b_rect.x + 2, b_rect.y + 2, badge, Color::WHITE, &f_small);

        let col = if i == 0 { Color::WHITE } else { palette.text_secondary };
        ui::font::draw_text_font(s, b_rect.right() + 4, t_rect.y + 4, name, col, &f_small);

        ty += tab_h + 2;
    }

    // 2. Top Header inside content area
    let cx = client.x + sb_w;
    let cw = client.w - sb_w;
    let hdr_h = 24;
    let hdr_rect = Rect::new(cx, client.y, cw, hdr_h);
    s.fill_rect(hdr_rect, palette.surface_raised);
    s.fill_rect(Rect::new(cx, hdr_rect.bottom() - 1, cw, 1), palette.border_subtle);

    ui::font::draw_text_font(s, cx + 10, client.y + 4, "System & Identity", palette.text_primary, &font);

    // Save button
    let save_rect = Rect::new(client.right() - 48, client.y + 3, 42, 18);
    ui::draw::draw_rounded_rect(s, save_rect, 2, palette.accent_primary);
    let s_sz = f_small.measure_str("Save");
    ui::font::draw_text_font(s, save_rect.x + (save_rect.w - s_sz.w)/2, save_rect.y + 2, "Save", Color::WHITE, &f_small);

    // 3. Settings Fields
    let fx = cx + 12;
    let mut fy = client.y + hdr_h + 8;

    ui::font::draw_text_font(s, fx, fy, "Hostname:", palette.text_muted, &f_small);
    let h_card = Rect::new(fx + 64, fy - 2, 80, 18);
    ui::draw::draw_rounded_rect(s, h_card, 2, palette.surface_sunken);
    ui::draw::draw_rect_outline(s, h_card, 1, palette.border_subtle);
    ui::font::draw_text_font(s, h_card.x + 6, h_card.y + 2, "novaos", palette.text_primary, &f_small);

    fy += 24;
    ui::font::draw_text_font(s, fx, fy, "Init (PID 1):", palette.text_muted, &f_small);
    let i_card = Rect::new(fx + 64, fy - 2, 80, 18);
    ui::draw::draw_rounded_rect(s, i_card, 2, palette.accent_primary.darken(35));
    ui::draw::draw_rect_outline(s, i_card, 1, palette.accent_primary);
    ui::font::draw_text_font(s, i_card.x + 6, i_card.y + 2, "SHELL.ELF", palette.accent_primary, &f_small);

    fy += 24;
    ui::font::draw_text_font(s, fx, fy, "Config File:", palette.text_muted, &f_small);
    ui::font::draw_text_font(s, fx + 64, fy, "SYSTEM.CFG (81 B)", palette.text_secondary, &f_small);

    // 4. Status Bar at bottom
    if client.h >= 100 {
        let sb_h = 18;
        let sb_rect = Rect::new(cx, client.bottom() - sb_h, cw, sb_h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(cx, sb_rect.y, cw, 1), palette.border_subtle);
        ui::font::draw_text_font(s, cx + 8, sb_rect.y + 2, "Ready | Synchronized", palette.text_muted, &f_small);
    }
}

/// Renders the complete, interactive NovaOS Calendar Application client body inside Window 6.
/// Synthesizes Apple Calendar (macOS), Windows 11 Calendar, and Ubuntu GNOME Calendar.
fn draw_calendar_app_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    cal: &WmCalendarApp,
    palette: &ThemePalette,
) {
    let font_sm = Font::small();
    let font_reg = Font::regular();

    // Responsive sidebar
    let sb_w = if client.w >= 300 { 110 } else { 0 };

    // 1. Sidebar (macOS categories + Windows 11 Acrylic Hero Card)
    if sb_w > 0 {
        let sb_rect = Rect::new(client.x, client.y, sb_w, client.h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(sb_rect.right() - 1, sb_rect.y, 1, sb_rect.h), palette.border_subtle);

        // Windows 11 Hero Date Card
        let hero_rect = Rect::new(client.x + 6, client.y + 6, sb_w - 12, 58);
        ui::draw::draw_rounded_rect(s, hero_rect, 6, Color::rgba(255, 255, 255, 14));
        ui::draw::draw_rect_outline(s, hero_rect, 1, Color::rgba(255, 255, 255, 26));

        // Selected Day Number (Big, e.g. "19")
        let mut dbuf = [0u8; 8];
        let mut doff = 0;
        format_u32(cal.selected_day as u32, &mut dbuf, &mut doff);
        let d_str = core::str::from_utf8(&dbuf[..doff]).unwrap_or("");
        ui::font::draw_text_font(s, hero_rect.x + 8, hero_rect.y + 4, d_str, Color::rgb(0, 212, 255), &font_reg);

        // Day of Week
        let sel_dow = day_of_week(cal.view_year, cal.view_month, cal.selected_day);
        let dow_str = DAY_NAMES_FULL[sel_dow as usize % 7];
        ui::font::draw_text_font(s, hero_rect.x + 8, hero_rect.y + 24, dow_str, Color::WHITE, &font_sm);

        // Month & Year
        let m_idx = cal.view_month.saturating_sub(1) as usize % 12;
        let mut my_buf = [0u8; 24];
        let mut my_off = 0;
        append_str(MONTH_NAMES_SHORT[m_idx], &mut my_buf, &mut my_off);
        append_str(" ", &mut my_buf, &mut my_off);
        format_u32(cal.view_year as u32, &mut my_buf, &mut my_off);
        let my_str = core::str::from_utf8(&my_buf[..my_off]).unwrap_or("");
        ui::font::draw_text_font(s, hero_rect.x + 8, hero_rect.y + 40, my_str, palette.text_muted, &font_sm);

        // macOS Category Filters Header
        let sec_y = client.y + 70;
        ui::font::draw_text_font(s, client.x + 8, sec_y, "CATEGORIES", palette.text_muted, &font_sm);

        // Category items with interactive pills
        let cats = [
            WmCategory::Work,
            WmCategory::Personal,
            WmCategory::NovaOS,
            WmCategory::Holidays,
            WmCategory::Reminders,
        ];
        let mut cy = sec_y + 14;
        for cat in cats.iter() {
            if cy + 18 > client.bottom() - 32 {
                break;
            }
            let is_enabled = cal.is_category_enabled(*cat);
            let pill_rect = Rect::new(client.x + 6, cy, sb_w - 12, 18);
            if is_enabled {
                ui::draw::draw_rounded_rect(s, pill_rect, 3, Color::rgba(255, 255, 255, 14));
            }

            // Colored dot
            let (r, g, b) = cat.color_rgb();
            let dot_col = if is_enabled { Color::rgb(r, g, b) } else { Color::rgb(80, 88, 98) };
            ui::draw::draw_rounded_rect(s, Rect::new(pill_rect.x + 4, cy + 4, 8, 8), 4, dot_col);

            let txt_col = if is_enabled { Color::WHITE } else { palette.text_muted };
            ui::font::draw_text_font(s, pill_rect.x + 16, cy + 3, cat.name(), txt_col, &font_sm);

            cy += 20;
        }

        // "+ Add Event" Button at bottom
        let add_btn = Rect::new(client.x + 8, client.bottom() - 26, sb_w - 16, 20);
        ui::draw::draw_rounded_rect(s, add_btn, 4, Color::rgb(35, 134, 54));
        ui::draw::draw_rect_outline(s, add_btn, 1, Color::rgb(63, 185, 80));
        ui::font::draw_text_font(s, add_btn.x + 10, add_btn.y + 3, "+ Add Event", Color::WHITE, &font_sm);
    }

    // 2. Main Content Canvas
    let cx = client.x + sb_w;
    let cw = client.w - sb_w;
    let hdr_h = 28;

    // Header Toolbar
    let hdr_rect = Rect::new(cx, client.y, cw, hdr_h);
    s.fill_rect(hdr_rect, palette.surface_raised);
    s.fill_rect(Rect::new(cx, hdr_rect.bottom() - 1, cw, 1), palette.border_subtle);

    // Prev Button [<]
    let btn_prev = Rect::new(cx + 8, client.y + 4, 20, 20);
    ui::draw::draw_rounded_rect(s, btn_prev, 3, Color::rgba(255, 255, 255, 20));
    ui::font::draw_text_font(s, btn_prev.x + 6, btn_prev.y + 3, "<", palette.text_primary, &font_sm);

    // Title: e.g. "November 2026"
    let m_idx = cal.view_month.saturating_sub(1) as usize % 12;
    let mut hdr_title_buf = [0u8; 32];
    let mut hdr_off = 0;
    append_str(MONTH_NAMES_FULL[m_idx], &mut hdr_title_buf, &mut hdr_off);
    append_str(" ", &mut hdr_title_buf, &mut hdr_off);
    format_u32(cal.view_year as u32, &mut hdr_title_buf, &mut hdr_off);
    let hdr_title = core::str::from_utf8(&hdr_title_buf[..hdr_off]).unwrap_or("");
    ui::font::draw_text_font(s, cx + 34, client.y + 5, hdr_title, Color::WHITE, &font_reg);

    // Next Button [>]
    let title_w = font_reg.measure_str(hdr_title).w;
    let btn_next = Rect::new(cx + 42 + title_w, client.y + 4, 20, 20);
    ui::draw::draw_rounded_rect(s, btn_next, 3, Color::rgba(255, 255, 255, 20));
    ui::font::draw_text_font(s, btn_next.x + 6, btn_next.y + 3, ">", palette.text_primary, &font_sm);

    // Today Button
    let btn_today = Rect::new(btn_next.right() + 8, client.y + 4, 46, 20);
    ui::draw::draw_rounded_rect(s, btn_today, 3, Color::rgba(255, 255, 255, 20));
    ui::font::draw_text_font(s, btn_today.x + 6, btn_today.y + 3, "Today", palette.text_primary, &font_sm);

    // View Switcher (macOS Segmented control): [ Month | Week | Day | Agenda ]
    let v_modes = [
        (WmViewMode::Month, "Month"),
        (WmViewMode::Week, "Week"),
        (WmViewMode::Day, "Day"),
        (WmViewMode::Agenda, "Agenda"),
    ];
    let seg_w = 40;
    let total_seg_w = seg_w * 4;
    let seg_x = cx + cw - total_seg_w - 8;
    for (i, &(mode, label)) in v_modes.iter().enumerate() {
        let seg_rect = Rect::new(seg_x + (i as i32) * seg_w, client.y + 4, seg_w, 20);
        let is_active = cal.view_mode == mode;
        if is_active {
            ui::draw::draw_rounded_rect(s, seg_rect, 3, palette.accent_primary);
            ui::font::draw_text_font(s, seg_rect.x + 4, seg_rect.y + 3, label, Color::WHITE, &font_sm);
        } else {
            ui::draw::draw_rounded_rect(s, seg_rect, 3, Color::rgba(255, 255, 255, 12));
            ui::font::draw_text_font(s, seg_rect.x + 4, seg_rect.y + 3, label, palette.text_muted, &font_sm);
        }
    }

    // 3. Render Mode Content
    match cal.view_mode {
        WmViewMode::Month => {
            // Day of week column headers
            let col_w = (cw - 12) / 7;
            let dow_y = client.y + hdr_h + 4;
            let dow_names = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
            for (c, &name) in dow_names.iter().enumerate() {
                ui::font::draw_text_font(s, cx + 6 + (c as i32) * col_w + 6, dow_y, name, palette.text_muted, &font_sm);
            }

            // Month Grid
            let grid_y = dow_y + 16;
            let bottom_card_h = if client.h >= 240 { 50 } else { 0 };
            let grid_h = client.bottom() - grid_y - bottom_card_h - 4;
            let row_h = (grid_h / 6).max(14);

            let first_dow = day_of_week(cal.view_year, cal.view_month, 1) as usize;
            let total_days = days_in_month(cal.view_year, cal.view_month) as usize;
            let prev_m = if cal.view_month == 1 { 12 } else { cal.view_month - 1 };
            let prev_y = if cal.view_month == 1 { cal.view_year - 1 } else { cal.view_year };
            let prev_m_days = days_in_month(prev_y, prev_m) as usize;

            for r in 0..6 {
                for c in 0..7 {
                    let cell_x = cx + 6 + (c as i32) * col_w;
                    let cell_y = grid_y + (r as i32) * row_h;
                    let idx = r * 7 + c;

                    // Subtle cell border
                    s.fill_rect(Rect::new(cell_x, cell_y, col_w, 1), Color::rgba(255, 255, 255, 8));

                    if idx < first_dow {
                        let d = prev_m_days - (first_dow - 1 - idx);
                        let mut buf = [0u8; 8];
                        let mut off = 0;
                        format_u32(d as u32, &mut buf, &mut off);
                        let str_val = core::str::from_utf8(&buf[..off]).unwrap_or("");
                        ui::font::draw_text_font(s, cell_x + 4, cell_y + 2, str_val, Color::rgb(70, 78, 90), &font_sm);
                    } else if idx < first_dow + total_days {
                        let d = (idx - first_dow + 1) as u8;
                        let is_today = cal.view_year == cal.today_year && cal.view_month == cal.today_month && d == cal.today_day;
                        let is_selected = d == cal.selected_day;

                        if is_selected {
                            ui::draw::draw_rounded_rect(s, Rect::new(cell_x + 1, cell_y + 1, col_w - 2, row_h - 2), 3, Color::rgba(0, 212, 255, 30));
                            ui::draw::draw_rect_outline(s, Rect::new(cell_x + 1, cell_y + 1, col_w - 2, row_h - 2), 1, Color::rgb(0, 212, 255));
                        }

                        let mut buf = [0u8; 8];
                        let mut off = 0;
                        format_u32(d as u32, &mut buf, &mut off);
                        let str_val = core::str::from_utf8(&buf[..off]).unwrap_or("");

                        if is_today {
                            let badge = Rect::new(cell_x + 3, cell_y + 1, 16, 14);
                            ui::draw::draw_rounded_rect(s, badge, 3, Color::rgb(225, 29, 72));
                            ui::font::draw_text_font(s, cell_x + 4, cell_y + 2, str_val, Color::WHITE, &font_sm);
                        } else {
                            let num_col = if is_selected { Color::WHITE } else { palette.text_primary };
                            ui::font::draw_text_font(s, cell_x + 4, cell_y + 2, str_val, num_col, &font_sm);
                        }

                        // Event indicator dots (Ubuntu GNOME Calendar dots)
                        let mut dot_x = cell_x + 4;
                        let dot_y = cell_y + row_h - 4;
                        let mut dots_shown = 0;
                        for i in 0..cal.event_count {
                            let ev = &cal.events[i];
                            if ev.year == cal.view_year && ev.month == cal.view_month && ev.day == d && cal.is_category_enabled(ev.category) {
                                let (r, g, b) = ev.category.color_rgb();
                                s.fill_rect(Rect::new(dot_x, dot_y, 4, 3), Color::rgb(r, g, b));
                                dot_x += 6;
                                dots_shown += 1;
                                if dots_shown >= 3 {
                                    break;
                                }
                            }
                        }
                    } else {
                        let d = idx - (first_dow + total_days) + 1;
                        let mut buf = [0u8; 8];
                        let mut off = 0;
                        format_u32(d as u32, &mut buf, &mut off);
                        let str_val = core::str::from_utf8(&buf[..off]).unwrap_or("");
                        ui::font::draw_text_font(s, cell_x + 4, cell_y + 2, str_val, Color::rgb(70, 78, 90), &font_sm);
                    }
                }
            }

            // Bottom Selected Day Event Preview card (Windows 11 card)
            if bottom_card_h > 0 {
                let card_y = client.bottom() - bottom_card_h - 2;
                let card_rect = Rect::new(cx + 6, card_y, cw - 12, bottom_card_h);
                ui::draw::draw_rounded_rect(s, card_rect, 4, Color::rgba(255, 255, 255, 14));
                ui::draw::draw_rect_outline(s, card_rect, 1, Color::rgba(255, 255, 255, 25));

                // Find events for selected day
                let mut found = 0;
                let mut ey = card_y + 4;
                for i in 0..cal.event_count {
                    let ev = &cal.events[i];
                    if ev.year == cal.view_year && ev.month == cal.view_month && ev.day == cal.selected_day && cal.is_category_enabled(ev.category) {
                        found += 1;
                        if ey + 14 <= card_rect.bottom() {
                            let (r, g, b) = ev.category.color_rgb();
                            s.fill_rect(Rect::new(card_rect.x + 6, ey + 2, 3, 12), Color::rgb(r, g, b));
                            ui::font::draw_text_font(s, card_rect.x + 14, ey + 1, ev.time_start, Color::rgb(0, 212, 255), &font_sm);
                            ui::font::draw_text_font(s, card_rect.x + 72, ey + 1, ev.title, Color::WHITE, &font_sm);
                            ey += 18;
                        }
                    }
                }
                if found == 0 {
                    ui::font::draw_text_font(s, card_rect.x + 10, card_y + 16, "No events scheduled for selected date.", palette.text_muted, &font_sm);
                }
            }
        }
        WmViewMode::Week => {
            let col_w = (cw - 8) / 7;
            let col_y = client.y + hdr_h + 4;
            for c in 0..7 {
                let wx = cx + 4 + (c as i32) * col_w;
                let d = ((cal.selected_day as i32 - 3 + c as i32).max(1) as u8).min(days_in_month(cal.view_year, cal.view_month));
                let dow_idx = day_of_week(cal.view_year, cal.view_month, d);
                let dow_str = DAY_NAMES_SHORT[dow_idx as usize % 7];

                let h_card = Rect::new(wx, col_y, col_w - 2, 22);
                let is_sel = d == cal.selected_day;
                if is_sel {
                    ui::draw::draw_rounded_rect(s, h_card, 3, palette.accent_primary);
                } else {
                    ui::draw::draw_rounded_rect(s, h_card, 3, Color::rgba(255, 255, 255, 10));
                }
                ui::font::draw_text_font(s, wx + 4, col_y + 4, dow_str, Color::WHITE, &font_sm);

                let mut dbuf = [0u8; 8];
                let mut doff = 0;
                format_u32(d as u32, &mut dbuf, &mut doff);
                let d_str = core::str::from_utf8(&dbuf[..doff]).unwrap_or("");
                ui::font::draw_text_font(s, wx + col_w - 18, col_y + 4, d_str, Color::WHITE, &font_sm);

                // Vertical column separator
                s.fill_rect(Rect::new(wx + col_w - 1, col_y + 24, 1, client.bottom() - col_y - 28), Color::rgba(255, 255, 255, 12));
            }
        }
        WmViewMode::Day => {
            let dy = client.y + hdr_h + 8;
            ui::font::draw_text_font(s, cx + 12, dy, "Daily Schedule & Hour Timeline", Color::WHITE, &font_reg);

            let mut ey = dy + 22;
            let hours: &[&str] = &["09:00 AM", "11:00 AM", "02:00 PM", "04:00 PM", "07:30 PM", "11:00 PM"];
            for (idx, &hr) in hours.iter().enumerate() {
                if ey + 24 > client.bottom() - 10 {
                    break;
                }
                s.fill_rect(Rect::new(cx + 12, ey + 10, cw - 24, 1), Color::rgba(255, 255, 255, 14));
                ui::font::draw_text_font(s, cx + 12, ey, hr, palette.text_muted, &font_sm);

                // Show matching event
                if idx < cal.event_count {
                    let ev = &cal.events[idx];
                    let (r, g, b) = ev.category.color_rgb();
                    let card = Rect::new(cx + 80, ey - 4, cw - 100, 22);
                    ui::draw::draw_rounded_rect(s, card, 3, Color::rgb(r, g, b).darken(40));
                    ui::draw::draw_rect_outline(s, card, 1, Color::rgb(r, g, b));
                    ui::font::draw_text_font(s, card.x + 8, card.y + 4, ev.title, Color::WHITE, &font_sm);
                }
                ey += 30;
            }
        }
        WmViewMode::Agenda => {
            let mut ey = client.y + hdr_h + 8;
            for i in 0..cal.event_count {
                let ev = &cal.events[i];
                if ey + 32 > client.bottom() - 6 {
                    break;
                }
                if cal.is_category_enabled(ev.category) {
                    let card = Rect::new(cx + 10, ey, cw - 20, 28);
                    ui::draw::draw_rounded_rect(s, card, 4, Color::rgba(255, 255, 255, 14));
                    ui::draw::draw_rect_outline(s, card, 1, Color::rgba(255, 255, 255, 25));

                    let (r, g, b) = ev.category.color_rgb();
                    s.fill_rect(Rect::new(card.x + 2, card.y + 2, 4, card.h - 4), Color::rgb(r, g, b));

                    // Date & Time badge
                    ui::font::draw_text_font(s, card.x + 12, card.y + 6, ev.time_start, Color::rgb(0, 212, 255), &font_sm);
                    // Title
                    ui::font::draw_text_font(s, card.x + 76, card.y + 6, ev.title, Color::WHITE, &font_sm);
                    // Location
                    if card.w >= 220 {
                        ui::font::draw_text_font(s, card.right() - 90, card.y + 6, ev.location, palette.text_muted, &font_sm);
                    }
                    ey += 32;
                }
            }
        }
    }
}

/// Renders the complete, interactive NovaOS Calculator Application client body inside Window 7.
/// Synthesizes Apple Calculator (macOS), Windows 11 Fluent Calculator, and Ubuntu GNOME Calculator.
fn draw_calc_app_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    calc: &WmCalcApp,
    palette: &ThemePalette,
) {
    let font_sm = Font::small();
    let font_reg = Font::regular();

    // 1. Cosmic Slate background
    s.fill_rect(client, Color::rgb(19, 22, 34));

    // 2. Mode Selector Pill Bar
    let mode_bar_h = 26;
    s.fill_rect(Rect::new(client.x, client.y, client.w, mode_bar_h), Color::rgb(26, 30, 46));
    s.fill_rect(Rect::new(client.x, client.y + mode_bar_h - 1, client.w, 1), Color::rgb(40, 48, 72));

    let modes = [WmCalcMode::Standard, WmCalcMode::Scientific, WmCalcMode::Programmer, WmCalcMode::History];
    let pad = 4;
    let tab_w = (client.w - pad * 2) / 4;
    for (i, m) in modes.iter().enumerate() {
        let tx = client.x + pad + i as i32 * tab_w;
        let tr = Rect::new(tx, client.y + 2, tab_w - 4, mode_bar_h - 4);
        let is_act = calc.mode == *m;
        let tab_col = if is_act { Color::rgb(124, 58, 237) } else { Color::rgb(32, 38, 56) };
        ui::draw::draw_rounded_rect(s, tr, 4, tab_col);
        ui::draw::draw_rect_outline(s, tr, 1, Color::rgb(56, 66, 94));

        let name = m.name();
        let name_sz = font_sm.measure_str(name);
        let nx = tr.x + (tr.w - name_sz.w) / 2;
        let ny = tr.y + (tr.h - name_sz.h) / 2;
        let text_col = if is_act { Color::WHITE } else { palette.text_muted };
        ui::font::draw_text_font(s, nx, ny, name, text_col, &font_sm);
    }

    // 3. Digital LCD Screen
    let screen_h = if calc.mode == WmCalcMode::Programmer { 64 } else { 48 };
    let screen_y = client.y + mode_bar_h + 4;
    let screen_rect = Rect::new(client.x + 8, screen_y, client.w - 16, screen_h);
    ui::draw::draw_rounded_rect(s, screen_rect, 6, Color::rgb(12, 14, 22));
    ui::draw::draw_rect_outline(s, screen_rect, 1, Color::rgb(40, 48, 72));

    // Memory badge [M]
    if calc.has_memory {
        let m_rect = Rect::new(screen_rect.x + 6, screen_rect.y + 6, 18, 14);
        ui::draw::draw_rounded_rect(s, m_rect, 3, Color::rgb(124, 58, 237));
        ui::font::draw_text_font(s, m_rect.x + 4, m_rect.y + 1, "M", Color::WHITE, &font_sm);
    }

    // Mode badge in Programmer
    if calc.mode == WmCalcMode::Programmer {
        let base_name = match calc.prog_base {
            0 => "HEX",
            1 => "DEC",
            2 => "OCT",
            _ => "BIN",
        };
        ui::font::draw_text_font(s, screen_rect.x + 28, screen_rect.y + 6, base_name, Color::rgb(245, 158, 11), &font_sm);
    }

    // Top Expression line
    let expr_str = calc.expr_str();
    if !expr_str.is_empty() {
        let e_sz = font_sm.measure_str(expr_str);
        let ex = (screen_rect.right() - e_sz.w - 8).max(screen_rect.x + 50);
        ui::font::draw_text_font(s, ex, screen_rect.y + 5, expr_str, palette.text_muted, &font_sm);
    }

    // Main readout digits (macOS bold high-contrast display)
    let disp_str = calc.display_str();
    let d_sz = font_reg.measure_str(disp_str);
    let dx = (screen_rect.right() - d_sz.w - 10).max(screen_rect.x + 8);
    let dy = if calc.mode == WmCalcMode::Programmer { screen_rect.y + 20 } else { screen_rect.y + 22 };
    ui::font::draw_text_font(s, dx, dy, disp_str, Color::WHITE, &font_reg);

    // Live programmer multi-base conversions
    if calc.mode == WmCalcMode::Programmer {
        let iv = calc.current_int();
        let mut hex_buf = [0u8; 24];
        let mut h_off = 0;
        append_str("HEX: ", &mut hex_buf, &mut h_off);
        let mut h_val = [0u8; 16];
        let mut hv_off = 0;
        format_u32(iv as u32, &mut h_val, &mut hv_off);
        append_str(core::str::from_utf8(&h_val[..hv_off]).unwrap_or("0"), &mut hex_buf, &mut h_off);
        let h_str = core::str::from_utf8(&hex_buf[..h_off]).unwrap_or("");
        ui::font::draw_text_font(s, screen_rect.x + 8, screen_rect.y + 44, h_str, palette.text_muted, &font_sm);

        let mut dec_buf = [0u8; 24];
        let mut d_off = 0;
        append_str("DEC: ", &mut dec_buf, &mut d_off);
        format_u32(iv as u32, &mut dec_buf, &mut d_off);
        let dec_str = core::str::from_utf8(&dec_buf[..d_off]).unwrap_or("");
        ui::font::draw_text_font(s, screen_rect.x + (screen_rect.w / 2), screen_rect.y + 44, dec_str, palette.text_muted, &font_sm);
    }

    // 4. Memory Bar or History
    let mut key_start_y = screen_y + screen_h + 4;

    if calc.mode == WmCalcMode::History {
        let h_rect = Rect::new(client.x + 8, key_start_y, client.w - 16, client.bottom() - key_start_y - 6);
        ui::draw::draw_rounded_rect(s, h_rect, 6, Color::rgb(12, 14, 22));
        ui::draw::draw_rect_outline(s, h_rect, 1, Color::rgb(40, 48, 72));

        ui::font::draw_text_font(s, h_rect.x + 10, h_rect.y + 6, "Calculation History Tape", Color::rgb(245, 158, 11), &font_sm);

        let clr_btn = Rect::new(h_rect.right() - 86, h_rect.y + 4, 78, 18);
        ui::draw::draw_rounded_rect(s, clr_btn, 3, Color::rgb(45, 53, 76));
        ui::draw::draw_rect_outline(s, clr_btn, 1, Color::rgb(56, 66, 94));
        ui::font::draw_text_font(s, clr_btn.x + 6, clr_btn.y + 2, "Clear History", Color::WHITE, &font_sm);

        if calc.history_count == 0 {
            ui::font::draw_text_font(s, h_rect.x + 12, h_rect.y + 36, "There are no calculations in history yet.", palette.text_muted, &font_sm);
            return;
        }

        let item_h = 32;
        let mut cur_y = h_rect.y + 28;
        for i in (0..calc.history_count).rev() {
            if cur_y + item_h > h_rect.bottom() {
                break;
            }
            let item_rect = Rect::new(h_rect.x + 6, cur_y, h_rect.w - 12, item_h - 4);
            ui::draw::draw_rounded_rect(s, item_rect, 4, Color::rgb(32, 38, 56));
            ui::draw::draw_rect_outline(s, item_rect, 1, Color::rgb(56, 66, 94));

            let e_str = core::str::from_utf8(&calc.history_expr[i][..calc.history_expr_len[i]]).unwrap_or("");
            let r_str = core::str::from_utf8(&calc.history_res[i][..calc.history_res_len[i]]).unwrap_or("");

            ui::font::draw_text_font(s, item_rect.x + 8, item_rect.y + 6, e_str, palette.text_muted, &font_sm);

            let r_sz = font_reg.measure_str(r_str);
            ui::font::draw_text_font(s, item_rect.right() - r_sz.w - 8, item_rect.y + 4, r_str, Color::WHITE, &font_reg);

            cur_y += item_h;
        }
        return;
    }

    if calc.mode == WmCalcMode::Standard || calc.mode == WmCalcMode::Scientific {
        let mem_h = 20;
        let btns = ["MC", "MR", "M+", "M-", "MS"];
        let btn_w = (client.w - 16) / 5;
        for (i, &b) in btns.iter().enumerate() {
            let bx = client.x + 8 + i as i32 * btn_w;
            let br = Rect::new(bx, key_start_y, btn_w - 4, mem_h);
            ui::draw::draw_rounded_rect(s, br, 3, Color::rgb(45, 53, 76));
            ui::draw::draw_rect_outline(s, br, 1, Color::rgb(56, 66, 94));

            let b_sz = font_sm.measure_str(b);
            ui::font::draw_text_font(s, br.x + (br.w - b_sz.w) / 2, br.y + (br.h - b_sz.h) / 2, b, Color::rgb(207, 216, 234), &font_sm);
        }
        key_start_y += mem_h + 4;
    }

    // 5. Keypad Grid
    let pad = 4;
    let avail_h = client.bottom() - key_start_y - 6;
    if avail_h <= 0 {
        return;
    }

    let is_sci = calc.mode == WmCalcMode::Scientific;
    let is_prog = calc.mode == WmCalcMode::Programmer;
    let cols = if is_sci { 5 } else { 4 };
    let rows = 6;
    let btn_w = (client.w - 16 - pad * (cols - 1)) / cols;
    let btn_h = (avail_h - pad * (rows - 1)) / rows;

    for r in 0..rows {
        for c in 0..cols {
            let bx = client.x + 8 + c * (btn_w + pad);
            let by = key_start_y + r * (btn_h + pad);
            let br = Rect::new(bx, by, btn_w, btn_h);

            let label = if is_sci {
                let grid = [
                    ["sin", "cos", "tan", "CE", "C"],
                    ["pi", "e", "^", "sqrt", "/"],
                    ["ln", "log", "x^2", "1/x", "*"],
                    ["n!", "7", "8", "9", "-"],
                    ["|x|", "4", "5", "6", "+"],
                    ["+/-", "1", "2", "3", "="],
                ];
                grid[r as usize][c as usize]
            } else if is_prog {
                let grid = [
                    ["AND", "OR", "XOR", "NOT"],
                    ["<<", ">>", "CE", "C"],
                    ["A", "B", "7", "8"],
                    ["C", "D", "4", "5"],
                    ["E", "F", "1", "2"],
                    ["HEX", "DEC", "0", "="],
                ];
                grid[r as usize][c as usize]
            } else {
                let grid = [
                    ["%", "CE", "C", "del"],
                    ["1/x", "x^2", "sqrt", "/"],
                    ["7", "8", "9", "*"],
                    ["4", "5", "6", "-"],
                    ["1", "2", "3", "+"],
                    ["+/-", "0", ".", "="],
                ];
                grid[r as usize][c as usize]
            };

            let is_op = label == "/" || label == "*" || label == "-" || label == "+" || label == "^";
            let is_equals = label == "=";
            let is_digit = label.as_bytes().iter().all(|&b| (b >= b'0' && b <= b'9') || b == b'.' || b == b'+' || b == b'-');
            let is_hex = label.len() == 1 && (label >= "A" && label <= "F");

            let is_active_op = match calc.pending_op {
                b'+' => label == "+",
                b'-' => label == "-",
                b'*' => label == "*",
                b'/' => label == "/",
                b'^' => label == "^",
                _ => false,
            };

            let bg_col = if is_active_op {
                Color::rgb(245, 158, 11)
            } else if is_equals {
                Color::rgb(234, 88, 12)
            } else if is_op {
                Color::rgb(217, 119, 6)
            } else if is_hex {
                Color::rgb(124, 58, 237)
            } else if is_digit {
                Color::rgb(32, 38, 56)
            } else {
                Color::rgb(45, 53, 76)
            };

            ui::draw::draw_rounded_rect(s, br, 4, bg_col);
            ui::draw::draw_rect_outline(s, br, 1, Color::rgb(56, 66, 94));

            let text_col = if is_op || is_equals || is_active_op || is_hex {
                Color::WHITE
            } else if is_digit {
                Color::rgb(241, 245, 249)
            } else {
                Color::rgb(207, 216, 234)
            };

            let l_sz = font_sm.measure_str(label);
            ui::font::draw_text_font(s, br.x + (br.w - l_sz.w) / 2, br.y + (br.h - l_sz.h) / 2, label, text_col, &font_sm);
        }
    }
}

/// Renders the complete, interactive Aero Browser client body inside Window 8.
/// Synthesizes Brave Shields privacy defense, Google Chrome rounded tabs & Omnibox,
/// and Mozilla Firefox distraction-free reader mode in pure Rust.
fn draw_browser_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    browser: &WmBrowserApp,
    palette: &ThemePalette,
) {
    let font_reg = Font::regular();
    let font_sm = Font::small();

    // 1. Chrome Tab Strip (y: 0..28)
    let strip_rect = Rect::new(client.x, client.y, client.w, 28);
    s.fill_rect(strip_rect, Color::rgb(15, 23, 42));

    let tab_titles = ["Aero New Tab", "Brave Shields", "NovaOS Portal"];
    let tab_w = 110;
    for (i, &title) in tab_titles.iter().enumerate() {
        let is_act = browser.active_tab == i;
        let tx = client.x + 8 + (i as i32) * (tab_w + 4);
        let tab_bg = if is_act { Color::rgb(30, 41, 59) } else { Color::rgb(20, 27, 45) };
        ui::draw::draw_rounded_rect(s, Rect::new(tx, client.y + 4, tab_w, 24), 4, tab_bg);
        if is_act {
            s.fill_rect(Rect::new(tx + 2, client.y + 4, tab_w - 4, 2), Color::rgb(255, 107, 0));
        }
        let text_col = if is_act { Color::WHITE } else { palette.text_muted };
        ui::font::draw_text_font(s, tx + 8, client.y + 9, title, text_col, &font_sm);
    }

    // 2. Navigation & Omnibox Bar (y: 28..62)
    let nav_y = client.y + 28;
    let nav_rect = Rect::new(client.x, nav_y, client.w, 34);
    s.fill_rect(nav_rect, Color::rgb(30, 41, 59));
    ui::draw::draw_line(s, client.x, nav_y + 33, client.x + client.w, nav_y + 33, Color::rgb(51, 65, 85));

    // Nav icons
    ui::font::draw_text_font(s, client.x + 10, nav_y + 9, "<-", Color::WHITE, &font_sm);
    ui::font::draw_text_font(s, client.x + 30, nav_y + 9, "->", Color::WHITE, &font_sm);
    ui::font::draw_text_font(s, client.x + 50, nav_y + 9, "R", Color::WHITE, &font_sm);
    ui::font::draw_text_font(s, client.x + 68, nav_y + 9, "H", Color::WHITE, &font_sm);

    // Omnibox URL Pill
    let omni_x = client.x + 92;
    let omni_w = (client.w - 220).max(120);
    let omni_rect = Rect::new(omni_x, nav_y + 5, omni_w, 24);
    ui::draw::draw_rounded_rect(s, omni_rect, 12, Color::rgb(15, 23, 42));
    ui::draw::draw_rect_outline(s, omni_rect, 1, Color::rgb(56, 189, 248));

    // Security Lock Indicator
    let lock_col = if browser.is_secure { Color::rgb(74, 222, 128) } else { Color::rgb(251, 146, 60) };
    ui::draw::draw_rounded_rect(s, Rect::new(omni_x + 6, nav_y + 9, 12, 12), 2, lock_col);
    ui::font::draw_text_font(s, omni_x + 24, nav_y + 9, browser.url_str(), Color::WHITE, &font_sm);

    // Right Action Buttons
    let shields_btn_x = omni_x + omni_w + 10;
    let shield_bg = if browser.shields_flyout_open { Color::rgb(255, 107, 0) } else { Color::rgb(45, 25, 20) };
    ui::draw::draw_rounded_rect(s, Rect::new(shields_btn_x, nav_y + 5, 36, 24), 6, shield_bg);
    ui::draw::draw_rect_outline(s, Rect::new(shields_btn_x, nav_y + 5, 36, 24), 1, Color::rgb(255, 107, 0));
    ui::font::draw_text_font(s, shields_btn_x + 6, nav_y + 9, "14", Color::WHITE, &font_sm);

    let reader_btn_x = shields_btn_x + 42;
    let reader_bg = if browser.reader_mode { Color::rgb(59, 130, 246) } else { Color::rgb(20, 27, 45) };
    ui::draw::draw_rounded_rect(s, Rect::new(reader_btn_x, nav_y + 5, 28, 24), 6, reader_bg);
    ui::font::draw_text_font(s, reader_btn_x + 8, nav_y + 9, "Rd", Color::WHITE, &font_sm);

    let devtools_btn_x = reader_btn_x + 34;
    let devtools_bg = if browser.devtools_open { Color::rgb(14, 165, 233) } else { Color::rgb(20, 27, 45) };
    ui::draw::draw_rounded_rect(s, Rect::new(devtools_btn_x, nav_y + 5, 28, 24), 6, devtools_bg);
    ui::font::draw_text_font(s, devtools_btn_x + 8, nav_y + 9, "DT", Color::WHITE, &font_sm);

    // 3. Bookmarks Bar (y: 62..84)
    let bm_y = nav_y + 34;
    s.fill_rect(Rect::new(client.x, bm_y, client.w, 22), Color::rgb(20, 27, 45));
    ui::draw::draw_line(s, client.x, bm_y + 21, client.x + client.w, bm_y + 21, Color::rgb(51, 65, 85));
    ui::font::draw_text_font(s, client.x + 10, bm_y + 5, "* NovaOS Portal", Color::rgb(245, 158, 11), &font_sm);
    ui::font::draw_text_font(s, client.x + 110, bm_y + 5, "* Brave Shields", Color::rgb(255, 107, 0), &font_sm);
    ui::font::draw_text_font(s, client.x + 210, bm_y + 5, "* Rust Lang", Color::rgb(226, 232, 240), &font_sm);
    ui::font::draw_text_font(s, client.x + 300, bm_y + 5, "* Zenith Cloud", Color::rgb(56, 189, 248), &font_sm);

    // 4. Web Page Viewport
    let vp_y = bm_y + 22;
    let devtools_h = if browser.devtools_open { 90 } else { 0 };
    let vp_h = (client.h - (vp_y - client.y) - devtools_h).max(0);

    let vp_bg = if browser.reader_mode {
        Color::rgb(244, 236, 216)
    } else {
        Color::rgb(15, 23, 42)
    };
    s.fill_rect(Rect::new(client.x, vp_y, client.w, vp_h), vp_bg);

    if browser.reader_mode {
        let text_col = Color::rgb(45, 45, 45);
        ui::font::draw_text_font(s, client.x + 24, vp_y + 16, "Aero Reader Mode - Pure Rust Layout Engine", text_col, &font_reg);
        ui::font::draw_text_font(s, client.x + 24, vp_y + 40, "Distraction-free viewing with custom typography and themes.", text_col, &font_sm);
        ui::font::draw_text_font(s, client.x + 24, vp_y + 58, "Eliminates ads, tracking scripts, and sidebars completely.", text_col, &font_sm);
    } else {
        match browser.active_tab {
            0 => {
                ui::font::draw_text_font(s, client.x + 30, vp_y + 18, "Aero Browser", Color::rgb(255, 107, 0), &font_reg);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 42, "Pure-Rust Web Engine - Zero Chromium/WebKit", Color::rgb(148, 163, 184), &font_sm);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 64, "Privacy First: Brave Shields active, 14 trackers blocked.", Color::rgb(74, 222, 128), &font_sm);

                let card_w = 90;
                let cards = ["Docs", "Store", "GitHub", "Terminal"];
                for (ci, &cname) in cards.iter().enumerate() {
                    let cx = client.x + 30 + (ci as i32) * (card_w + 12);
                    ui::draw::draw_rounded_rect(s, Rect::new(cx, vp_y + 90, card_w, 40), 6, Color::rgb(30, 41, 59));
                    ui::draw::draw_rect_outline(s, Rect::new(cx, vp_y + 90, card_w, 40), 1, Color::rgb(51, 65, 85));
                    ui::font::draw_text_font(s, cx + 14, vp_y + 104, cname, Color::WHITE, &font_sm);
                }

                if vp_h >= 170 {
                    let vid_x = client.x + 30;
                    let vid_y = vp_y + 140;
                    let vid_w = (client.w - 60).min(380).max(180);
                    let vid_h = 58;
                    if vid_y + vid_h <= vp_y + vp_h - 6 {
                        ui::draw::draw_rounded_rect(s, Rect::new(vid_x, vid_y, vid_w, vid_h), 6, Color::rgb(20, 24, 38));
                        ui::draw::draw_rect_outline(s, Rect::new(vid_x, vid_y, vid_w, vid_h), 1, Color::rgb(51, 65, 85));
                        ui::font::draw_text_font(s, vid_x + 10, vid_y + 8, "HTML5 Stream: NovaOS 2.0 Keynote", Color::rgb(240, 246, 252), &font_sm);
                        ui::font::draw_text_font(s, vid_x + 10, vid_y + 26, "Duration: 12:45 | 1080p FHD (MPEG-4 AVC)", Color::rgb(148, 163, 184), &font_sm);

                        // Floating IDM / Vortex Stream Sniffer Badge
                        let badge_w = 175;
                        let badge_h = 20;
                        let badge_x = vid_x + vid_w - badge_w - 6;
                        let badge_y = vid_y + 6;
                        if badge_x > vid_x + 100 {
                            ui::draw::draw_rounded_rect(s, Rect::new(badge_x, badge_y, badge_w, badge_h), 4, Color::rgb(14, 165, 233));
                            ui::draw::draw_rect_outline(s, Rect::new(badge_x, badge_y, badge_w, badge_h), 1, Color::WHITE);
                            ui::font::draw_text_font(s, badge_x + 6, badge_y + 3, "[v] Download Video | 1080p v", Color::WHITE, &font_sm);
                        }
                    }
                }
            }
            1 => {
                ui::font::draw_text_font(s, client.x + 30, vp_y + 18, "Brave Privacy Shields Telemetry", Color::rgb(255, 107, 0), &font_reg);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 42, "* Trackers & Ads Blocked: 14", Color::rgb(56, 189, 248), &font_sm);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 60, "* HTTPS Upgrades Enforced: 6", Color::rgb(74, 222, 128), &font_sm);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 78, "* Bandwidth Saved: 42.8 KB", Color::rgb(226, 232, 240), &font_sm);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 96, "* Canvas Fingerprinting Gating: Active", Color::rgb(245, 158, 11), &font_sm);
            }
            _ => {
                ui::font::draw_text_font(s, client.x + 30, vp_y + 18, "NovaOS: Next-Generation Enterprise OS", Color::rgb(88, 166, 255), &font_reg);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 42, "Pure Rust Microkernel Architecture with 64-bit Long Mode.", Color::rgb(226, 232, 240), &font_sm);
                ui::font::draw_text_font(s, client.x + 30, vp_y + 60, "Aero Browser is fully integrated into the desktop compositor.", Color::rgb(74, 222, 128), &font_sm);
            }
        }
    }

    // 5. DevTools Inspector Panel at bottom if open
    if browser.devtools_open && devtools_h > 0 {
        let dt_y = client.y + client.h - devtools_h;
        s.fill_rect(Rect::new(client.x, dt_y, client.w, devtools_h), Color::rgb(15, 23, 42));
        ui::draw::draw_line(s, client.x, dt_y, client.x + client.w, dt_y, Color::rgb(51, 65, 85));

        s.fill_rect(Rect::new(client.x, dt_y, client.w, 22), Color::rgb(30, 41, 59));
        let dt_tabs = ["Elements", "Console", "Network", "Shields"];
        let mut tx = client.x + 10;
        for (idx, &dt_title) in dt_tabs.iter().enumerate() {
            let is_act = browser.devtools_tab == (idx as u8);
            let col = if is_act { Color::rgb(56, 189, 248) } else { palette.text_muted };
            ui::font::draw_text_font(s, tx, dt_y + 5, dt_title, col, &font_sm);
            if is_act {
                s.fill_rect(Rect::new(tx, dt_y + 20, 46, 2), Color::rgb(56, 189, 248));
            }
            tx += 60;
        }

        match browser.devtools_tab {
            0 => {
                ui::font::draw_text_font(s, client.x + 12, dt_y + 28, "<html>", Color::rgb(192, 132, 252), &font_sm);
                ui::font::draw_text_font(s, client.x + 24, dt_y + 44, "<head><title>NovaOS Aero</title></head>", Color::rgb(148, 163, 184), &font_sm);
                ui::font::draw_text_font(s, client.x + 24, dt_y + 60, "<body><div id=\"main\" class=\"aero-app\">", Color::rgb(56, 189, 248), &font_sm);
            }
            1 => {
                ui::font::draw_text_font(s, client.x + 12, dt_y + 28, "> [OK] Aero Engine 1.0 initialized in pure Rust.", Color::rgb(74, 222, 128), &font_sm);
                ui::font::draw_text_font(s, client.x + 12, dt_y + 46, "> [SHIELDS] 14 third-party trackers blocked.", Color::rgb(255, 107, 0), &font_sm);
            }
            2 => {
                ui::font::draw_text_font(s, client.x + 12, dt_y + 28, "* GET / 200 OK (text/html, 1.2 KB, 8ms)", Color::rgb(74, 222, 128), &font_sm);
                ui::font::draw_text_font(s, client.x + 12, dt_y + 46, "* BLOCKED /tracker.js (Brave Shields)", Color::rgb(239, 68, 68), &font_sm);
            }
            _ => {
                ui::font::draw_text_font(s, client.x + 12, dt_y + 28, "* Blocklist Rules: 26 active pattern matchers", Color::rgb(255, 107, 0), &font_sm);
                ui::font::draw_text_font(s, client.x + 12, dt_y + 46, "* Cryptographic HTTPS Auto-Upgrade: Enabled", Color::rgb(74, 222, 128), &font_sm);
            }
        }
    }

    // 6. Shields Flyout Popup if open
    if browser.shields_flyout_open {
        let flyout_w = 230;
        let flyout_h = 160;
        let flyout_x = client.x + client.w - flyout_w - 10;
        let flyout_y = nav_y + 36;

        ui::draw::draw_rounded_rect(s, Rect::new(flyout_x, flyout_y, flyout_w, flyout_h), 8, Color::rgb(24, 30, 44));
        ui::draw::draw_rect_outline(s, Rect::new(flyout_x, flyout_y, flyout_w, flyout_h), 1, Color::rgb(51, 65, 85));

        ui::font::draw_text_font(s, flyout_x + 12, flyout_y + 10, "AERO SHIELDS", Color::rgb(255, 107, 0), &font_sm);
        let toggle_txt = if browser.shields_up { "UP [ON]" } else { "DOWN [OFF]" };
        let toggle_col = if browser.shields_up { Color::rgb(34, 197, 94) } else { Color::rgb(239, 68, 68) };
        ui::font::draw_text_font(s, flyout_x + flyout_w - 75, flyout_y + 10, toggle_txt, toggle_col, &font_sm);

        ui::draw::draw_rounded_rect(s, Rect::new(flyout_x + 10, flyout_y + 32, flyout_w - 20, 50), 6, Color::rgb(15, 23, 42));
        ui::font::draw_text_font(s, flyout_x + 18, flyout_y + 40, "14 Trackers Blocked", Color::rgb(56, 189, 248), &font_sm);
        ui::font::draw_text_font(s, flyout_x + 18, flyout_y + 58, "42 KB Data Saved", Color::rgb(74, 222, 128), &font_sm);

        ui::font::draw_text_font(s, flyout_x + 12, flyout_y + 92, "[x] Block cross-site trackers", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, flyout_x + 12, flyout_y + 110, "[x] Upgrade HTTPS", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, flyout_x + 12, flyout_y + 128, "[x] Shield protection active", Color::rgb(74, 222, 128), &font_sm);
    }
}

/// Renders the Nova Player Ultra HD Default Media Studio client view inside Window 9.
fn draw_player_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    player: &WmPlayerApp,
    palette: &ThemePalette,
) {
    let font_reg = Font::regular();
    let font_sm = Font::small();

    // 1. Deep Obsidian Cinema Canvas (#0A0E17)
    s.fill_rect(client, Color::rgb(10, 14, 23));

    // Track metadata
    let tracks: [(&str, &str, &str, &str); 4] = [
        ("Synthetic Skyline 4K", "4K UHD 60 FPS", "AV1 HDR10+", "03:00"),
        ("Deep Space Accretion Disk", "8K FUHD 60 FPS", "HEVC 10-Bit", "04:15"),
        ("Ocean Aurora Borealis", "1080p 60 FPS", "VP9 Pro", "02:45"),
        ("Synthwave Sunset Overdrive", "FLAC 24/96", "Hi-Res Audio", "03:30"),
    ];
    let (track_title, track_res, track_codec, _track_dur) = tracks[player.active_track.min(3)];

    // 2. Top Header Bar (Acrylic floating glass, h = 24)
    let hdr_h = 24;
    s.fill_rect(Rect::new(client.x, client.y, client.w, hdr_h), Color::rgba(15, 23, 42, 240));
    ui::draw::draw_line(s, client.x, client.y + hdr_h - 1, client.x + client.w, client.y + hdr_h - 1, Color::rgb(30, 41, 59));

    // Spectra badge [S]
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 8, client.y + 3, 18, 18), 3, Color::rgb(14, 165, 233));
    ui::font::draw_text_font(s, client.x + 13, client.y + 4, "S", Color::WHITE, &font_reg);

    // Track Title and Format Badges
    ui::font::draw_text_font(s, client.x + 32, client.y + 5, track_title, Color::WHITE, &font_reg);

    let res_x = client.x + 34 + font_reg.measure_str(track_title).w;
    if res_x + 90 < client.w - 90 {
        ui::draw::draw_rounded_rect(s, Rect::new(res_x, client.y + 4, 52, 16), 3, Color::rgb(30, 41, 59));
        ui::draw::draw_rect_outline(s, Rect::new(res_x, client.y + 4, 52, 16), 1, Color::rgb(56, 189, 248));
        ui::font::draw_text_font(s, res_x + 4, client.y + 6, track_res, Color::rgb(56, 189, 248), &font_sm);

        let cod_x = res_x + 58;
        if cod_x + 70 < client.w - 90 {
            ui::draw::draw_rounded_rect(s, Rect::new(cod_x, client.y + 4, 64, 16), 3, Color::rgb(30, 41, 59));
            ui::draw::draw_rect_outline(s, Rect::new(cod_x, client.y + 4, 64, 16), 1, Color::rgb(74, 222, 128));
            ui::font::draw_text_font(s, cod_x + 4, client.y + 6, track_codec, Color::rgb(74, 222, 128), &font_sm);
        }
    }

    // Top Right Action Buttons: Spectrum & Playlist
    let btn_viz_x = client.x + client.w - 75;
    let viz_bg = if player.show_visualizer { Color::rgb(14, 165, 233) } else { Color::rgb(30, 41, 59) };
    ui::draw::draw_rounded_rect(s, Rect::new(btn_viz_x, client.y + 3, 32, 18), 3, viz_bg);
    ui::font::draw_text_font(s, btn_viz_x + 6, client.y + 5, "VIZ", Color::WHITE, &font_sm);

    let btn_pl_x = client.x + client.w - 38;
    let pl_bg = if player.show_playlist { Color::rgb(245, 158, 11) } else { Color::rgb(30, 41, 59) };
    ui::draw::draw_rounded_rect(s, Rect::new(btn_pl_x, client.y + 3, 30, 18), 3, pl_bg);
    ui::font::draw_text_font(s, btn_pl_x + 8, client.y + 5, "PL", Color::WHITE, &font_sm);

    // 3. VLC Top Menu Bar Strip (y: client.y + 24 to client.y + 44)
    let menu_bar_y = client.y + 24;
    let menu_bar_h = 20;
    s.fill_rect(Rect::new(client.x, menu_bar_y, client.w, menu_bar_h), Color::rgb(18, 24, 38));
    ui::draw::draw_line(s, client.x, menu_bar_y + menu_bar_h - 1, client.x + client.w, menu_bar_y + menu_bar_h - 1, Color::rgb(48, 54, 61));

    let menu_defs = [
        ("Media", 6, 48),
        ("Playback", 50, 106),
        ("Audio", 108, 146),
        ("Video", 148, 186),
        ("Subtitle", 188, 238),
        ("Tools", 240, 278),
        ("View", 280, 314),
        ("Help", 316, 348),
    ];

    for (m_idx, (m_label, mx1, mx2)) in menu_defs.iter().enumerate() {
        let is_active = player.active_menu == Some(m_idx);
        let m_rect = Rect::new(client.x + mx1, menu_bar_y + 1, mx2 - mx1, menu_bar_h - 2);
        if is_active {
            ui::draw::draw_rounded_rect(s, m_rect, 3, Color::rgb(14, 165, 233));
            ui::font::draw_text_font(s, client.x + mx1 + 4, menu_bar_y + 4, m_label, Color::WHITE, &font_sm);
        } else {
            ui::font::draw_text_font(s, client.x + mx1 + 4, menu_bar_y + 4, m_label, Color::rgb(203, 213, 225), &font_sm);
        }
    }

    // 4. Main Cinema Viewport (Letterbox with Cyberpunk Grid / Aurora)
    let vp_y = client.y + 44;
    let bot_h = 58;
    let vp_h = (client.h - 44 - bot_h - 6).max(40);
    let _vp_rect = Rect::new(client.x, vp_y, client.w, vp_h);

    // Perspective horizon grid
    let horizon_y = vp_y + (vp_h * 55) / 100;
    ui::draw::draw_line(s, client.x, horizon_y, client.right(), horizon_y, Color::rgb(30, 58, 138));

    // Vertical perspective grid lines
    let cx = client.x + client.w / 2;
    for i in -6..=6 {
        let bottom_offset = i * (client.w / 14);
        let bx = cx + bottom_offset;
        ui::draw::draw_line(s, cx, horizon_y, bx, vp_y + vp_h, Color::rgb(20, 35, 75));
    }
    // Horizontal perspective grid lines
    let mut gy = horizon_y + 6;
    let mut step = 6;
    while gy < vp_y + vp_h {
        ui::draw::draw_line(s, client.x, gy, client.right(), gy, Color::rgb(20, 35, 75));
        step = (step * 14) / 10;
        gy += step;
    }

    // Neon Cyber Sun / Black Hole Accretion Disc in background
    let sun_y = horizon_y - 28;
    ui::draw::draw_rounded_rect(s, Rect::new(cx - 36, sun_y, 72, 36), 18, Color::rgb(244, 63, 94));
    ui::draw::draw_line(s, cx - 44, sun_y + 18, cx + 44, sun_y + 18, Color::rgb(251, 191, 36));

    // Audio / Visualizer 32-Band FFT overlay if active
    if player.show_visualizer || player.active_track == 3 {
        let viz_bar_count = 24.min((client.w - 40) / 10) as usize;
        let viz_start_x = cx - ((viz_bar_count as i32 * 10) / 2);
        let base_y = vp_y + vp_h - 10;

        for b in 0..viz_bar_count {
            let bx = viz_start_x + (b as i32) * 10;
            let h_val = (((b * 7 + 13) % 45) + 12) as i32;
            let bar_h = if player.is_playing { h_val } else { 4 };
            let bar_col = match b % 4 {
                0 => Color::rgb(56, 189, 248),
                1 => Color::rgb(74, 222, 128),
                2 => Color::rgb(245, 158, 11),
                _ => Color::rgb(244, 63, 94),
            };
            s.fill_rect(Rect::new(bx, base_y - bar_h, 7, bar_h), bar_col);
            s.fill_rect(Rect::new(bx, base_y - bar_h - 3, 7, 2), Color::WHITE);
        }
    }

    // Centered Play/Pause Watermark if paused
    if !player.is_playing {
        let pw_w = 110;
        let pw_h = 42;
        let pw_rect = Rect::new(cx - pw_w / 2, vp_y + (vp_h - pw_h) / 2, pw_w, pw_h);
        ui::draw::draw_rounded_rect(s, pw_rect, 8, Color::rgba(0, 0, 0, 180));
        ui::draw::draw_rect_outline(s, pw_rect, 1, Color::rgb(56, 189, 248));
        ui::font::draw_text_font(s, pw_rect.x + 22, pw_rect.y + 12, "PAUSED", Color::WHITE, &font_reg);
    }

    // 5. Sliding Playlist Panel (if open)
    if player.show_playlist {
        let pl_w = 210.min(client.w / 2);
        let pl_x = client.x + client.w - pl_w - 8;
        let pl_y = client.y + 48;
        let pl_h = vp_h - 8;
        let pl_rect = Rect::new(pl_x, pl_y, pl_w, pl_h);

        ui::draw::draw_rounded_rect(s, pl_rect, 8, Color::rgba(15, 23, 42, 230));
        ui::draw::draw_rect_outline(s, pl_rect, 1, Color::rgb(56, 189, 248));

        ui::font::draw_text_font(s, pl_x + 12, pl_y + 8, "MEDIA PLAYLIST", Color::rgb(56, 189, 248), &font_sm);
        ui::draw::draw_line(s, pl_x + 8, pl_y + 24, pl_x + pl_w - 8, pl_y + 24, Color::rgb(51, 65, 85));

        for (idx, &(t_title, _, _, t_time)) in tracks.iter().enumerate() {
            let row_y = pl_y + 30 + (idx as i32) * 32;
            if row_y + 28 > pl_y + pl_h {
                break;
            }
            let is_sel = player.active_track == idx;
            let row_bg = if is_sel { Color::rgba(56, 189, 248, 40) } else { Color::rgba(255, 255, 255, 6) };
            ui::draw::draw_rounded_rect(s, Rect::new(pl_x + 6, row_y, pl_w - 12, 26), 4, row_bg);
            if is_sel {
                ui::draw::draw_rect_outline(s, Rect::new(pl_x + 6, row_y, pl_w - 12, 26), 1, Color::rgb(56, 189, 248));
                ui::font::draw_text_font(s, pl_x + 10, row_y + 6, ">", Color::rgb(56, 189, 248), &font_sm);
            }
            let text_col = if is_sel { Color::WHITE } else { palette.text_muted };
            let title_short = if t_title.len() > 14 { &t_title[..14] } else { t_title };
            ui::font::draw_text_font(s, pl_x + 22, row_y + 6, title_short, text_col, &font_sm);
            ui::font::draw_text_font(s, pl_x + pl_w - 44, row_y + 6, t_time, Color::rgb(148, 163, 184), &font_sm);
        }
    }

    // 6. Floating Acrylic Bottom Island (VLC-Style Transport Controls)
    let bot_y = client.y + client.h - bot_h - 4;
    let bot_rect = Rect::new(client.x + 8, bot_y, client.w - 16, bot_h);
    ui::draw::draw_rounded_rect(s, bot_rect, 8, Color::rgba(18, 24, 38, 240));
    ui::draw::draw_rect_outline(s, bot_rect, 1, Color::rgba(255, 255, 255, 25));

    // Top Row: Timeline [00:45] =======[O]================ [03:00]
    let cur_pos = player.position_secs;
    let mut time_buf = [0u8; 16];
    let mut t_off = 0;
    format_u32_padded(cur_pos / 60, 2, &mut time_buf, &mut t_off);
    append_str(":", &mut time_buf, &mut t_off);
    format_u32_padded(cur_pos % 60, 2, &mut time_buf, &mut t_off);
    let time_cur_str = core::str::from_utf8(&time_buf[..t_off]).unwrap_or("00:00");
    ui::font::draw_text_font(s, bot_rect.x + 8, bot_y + 6, time_cur_str, Color::rgb(148, 163, 184), &font_sm);

    let scrub_x = bot_rect.x + 48;
    let scrub_w = (bot_rect.w - 96).max(20);
    let scrub_y = bot_y + 9;
    ui::draw::draw_rounded_rect(s, Rect::new(scrub_x, scrub_y, scrub_w, 4), 2, Color::rgb(48, 54, 61));

    // Buffer bar (65%)
    let buf_w = (scrub_w * 65) / 100;
    ui::draw::draw_rounded_rect(s, Rect::new(scrub_x, scrub_y, buf_w, 4), 2, Color::rgb(71, 85, 105));

    // Played bar (progress based on position_secs / 180)
    let play_pct = (cur_pos as i32 * 100) / 180;
    let prog_w = (scrub_w * play_pct.clamp(0, 100)) / 100;
    ui::draw::draw_rounded_rect(s, Rect::new(scrub_x, scrub_y, prog_w, 4), 2, Color::rgb(56, 189, 248));
    // Playhead dot
    s.fill_rect(Rect::new(scrub_x + prog_w - 3, scrub_y - 2, 7, 8), Color::WHITE);

    // Duration timestamp
    ui::font::draw_text_font(s, bot_rect.right() - 40, bot_y + 6, "03:00", Color::rgb(148, 163, 184), &font_sm);

    // Bottom Row: Transport buttons (y: bot_y + 24)
    let btn_y = bot_y + 24;

    // Play/Pause button [ > ] / [ || ] (x: 14..46)
    let play_bg = if player.is_playing { Color::rgb(14, 165, 233) } else { Color::rgb(56, 189, 248) };
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 14, btn_y, 32, 24), 4, play_bg);
    let play_icon = if player.is_playing { "||" } else { ">" };
    let pl_off = if player.is_playing { 11 } else { 13 };
    ui::font::draw_text_font(s, client.x + 14 + pl_off, btn_y + 4, play_icon, Color::rgb(15, 23, 42), &font_reg);

    // Stop button [ ■ ] (x: 48..74)
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 48, btn_y, 26, 24), 4, Color::rgba(255, 255, 255, 14));
    s.fill_rect(Rect::new(client.x + 56, btn_y + 7, 10, 10), Color::rgb(239, 68, 68));

    // Prev [|<] (x: 76..102)
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 76, btn_y, 26, 24), 4, Color::rgba(255, 255, 255, 14));
    ui::font::draw_text_font(s, client.x + 82, btn_y + 4, "|<", Color::WHITE, &font_sm);

    // Next [>|] (x: 104..130)
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 104, btn_y, 26, 24), 4, Color::rgba(255, 255, 255, 14));
    ui::font::draw_text_font(s, client.x + 110, btn_y + 4, ">|", Color::WHITE, &font_sm);

    // Fullscreen [[ ]] (x: 132..158)
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 132, btn_y, 26, 24), 4, Color::rgba(255, 255, 255, 14));
    ui::font::draw_text_font(s, client.x + 137, btn_y + 4, "[ ]", Color::WHITE, &font_sm);

    // EQ button (x: 160..188)
    let eq_bg = if player.show_visualizer { Color::rgb(14, 165, 233) } else { Color::rgba(255, 255, 255, 14) };
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 160, btn_y, 28, 24), 4, eq_bg);
    ui::font::draw_text_font(s, client.x + 166, btn_y + 4, "EQ", Color::WHITE, &font_sm);

    // Playlist button (x: 190..218)
    let pl_btn_bg = if player.show_playlist { Color::rgb(245, 158, 11) } else { Color::rgba(255, 255, 255, 14) };
    ui::draw::draw_rounded_rect(s, Rect::new(client.x + 190, btn_y, 28, 24), 4, pl_btn_bg);
    ui::font::draw_text_font(s, client.x + 197, btn_y + 4, "PL", Color::WHITE, &font_sm);

    // If wider window (client.w >= 450), show Loop & Shuffle
    if client.w >= 450 {
        // Loop button [REP] (x: 220..250)
        let loop_bg = if player.is_looping { Color::rgb(56, 189, 248) } else { Color::rgba(255, 255, 255, 14) };
        let loop_fg = if player.is_looping { Color::rgb(15, 23, 42) } else { Color::WHITE };
        ui::draw::draw_rounded_rect(s, Rect::new(client.x + 220, btn_y, 30, 24), 4, loop_bg);
        ui::font::draw_text_font(s, client.x + 224, btn_y + 4, "REP", loop_fg, &font_sm);

        // Shuffle button [RND] (x: 252..282)
        let shuf_bg = if player.is_shuffling { Color::rgb(56, 189, 248) } else { Color::rgba(255, 255, 255, 14) };
        let shuf_fg = if player.is_shuffling { Color::rgb(15, 23, 42) } else { Color::WHITE };
        ui::draw::draw_rounded_rect(s, Rect::new(client.x + 252, btn_y, 30, 24), 4, shuf_bg);
        ui::font::draw_text_font(s, client.x + 256, btn_y + 4, "RND", shuf_fg, &font_sm);
    }

    // Right Controls: Volume Mute & Slider Wedge
    let mute_x = client.x + client.w - 110;
    let mute_col = if player.is_muted { Color::rgb(245, 158, 11) } else { Color::rgb(74, 222, 128) };
    ui::draw::draw_rounded_rect(s, Rect::new(mute_x, btn_y + 2, 28, 20), 3, Color::rgba(255, 255, 255, 10));
    ui::font::draw_text_font(s, mute_x + 3, btn_y + 4, if player.is_muted { "MUT" } else { "VOL" }, mute_col, &font_sm);

    let v_slider_x = client.x + client.w - 75;
    let v_slider_w = 60;
    ui::draw::draw_rounded_rect(s, Rect::new(v_slider_x, btn_y + 9, v_slider_w, 6), 2, Color::rgb(48, 54, 61));
    let v_fill_w = if player.is_muted { 0 } else { ((player.volume_pct * v_slider_w as u32) / 125).min(v_slider_w as u32) as i32 };
    if v_fill_w > 0 {
        let v_color = if player.volume_pct > 100 { Color::rgb(245, 158, 11) } else { Color::rgb(74, 222, 128) };
        ui::draw::draw_rounded_rect(s, Rect::new(v_slider_x, btn_y + 9, v_fill_w, 6), 2, v_color);
    }
    let mut vol_buf = [0u8; 16];
    let mut v_off = 0;
    format_u32(player.volume_pct, &mut vol_buf, &mut v_off);
    append_str("%", &mut vol_buf, &mut v_off);
    let vol_pct_str = core::str::from_utf8(&vol_buf[..v_off]).unwrap_or("100%");
    ui::font::draw_text_font(s, client.x + client.w - 44, btn_y + 5, vol_pct_str, Color::rgb(148, 163, 184), &font_sm);

    // 7. Dropdown Menu Card (rendered on top if active_menu is open)
    if let Some(m_idx) = player.active_menu {
        let items: &[(&str, &str)] = match m_idx {
            0 => &[ // Media
                ("Open File...", "Ctrl+O"),
                ("Open Network...", "Ctrl+N"),
                ("Open Recent Media", ">"),
                ("Save Playlist...", "Ctrl+Y"),
                ("Quit Spectra", "Ctrl+Q"),
            ],
            1 => &[ // Playback
                ("Play / Pause", "Space"),
                ("Stop", "Ctrl+S"),
                ("Previous Track", "Ctrl+P"),
                ("Next Track", "Ctrl+N"),
                ("Repeat / Loop", "Ctrl+R"),
                ("Random / Shuffle", "Ctrl+Z"),
            ],
            2 => &[ // Audio
                ("Mute Audio", "M"),
                ("Volume Up (+5%)", "Ctrl+Up"),
                ("Volume Down (-5%)", "Ctrl+Dn"),
                ("Audio Visualizer (FFT)", "V"),
            ],
            3 => &[ // Video
                ("Aspect Ratio (Toggle)", "A"),
                ("Switch Video Track", "T"),
                ("Take Snapshot", "Shift+S"),
            ],
            4 => &[ // Subtitle
                ("Add Subtitle File...", "Ctrl+U"),
                ("Toggle Subtitles", "S"),
            ],
            5 => &[ // Tools
                ("Effects and Filters (EQ)", "Ctrl+E"),
                ("Media Information", "Ctrl+I"),
                ("Preferences", "Ctrl+P"),
            ],
            6 => &[ // View
                ("Playlist Window", "Ctrl+L"),
                ("Spectrum Visualizer", "V"),
            ],
            7 => &[ // Help
                ("About Spectra...", "F1"),
                ("Keyboard Shortcuts", "?"),
            ],
            _ => &[],
        };

        let card_w = 210;
        let card_h = (items.len() as i32 * 22) + 12;
        let card_x = (client.x + menu_defs[m_idx].1).min(client.right() - card_w - 4);
        let card_y = menu_bar_y + menu_bar_h;

        let card_rect = Rect::new(card_x, card_y, card_w, card_h);
        ui::draw::draw_rounded_rect(s, card_rect, 6, Color::rgb(15, 23, 42));
        ui::draw::draw_rect_outline(s, card_rect, 1, Color::rgb(56, 189, 248));

        for (i, &(label, shortcut)) in items.iter().enumerate() {
            let item_y = card_y + 6 + (i as i32 * 22);
            let item_rect = Rect::new(card_x + 4, item_y, card_w - 8, 20);
            if i == 0 {
                ui::draw::draw_rounded_rect(s, item_rect, 3, Color::rgba(56, 189, 248, 30));
            }
            ui::font::draw_text_font(s, card_x + 8, item_y + 3, label, Color::WHITE, &font_sm);
            let sc_sz = font_sm.measure_str(shortcut);
            ui::font::draw_text_font(s, card_x + card_w - sc_sz.w - 10, item_y + 3, shortcut, Color::rgb(148, 163, 184), &font_sm);
        }
    }

    // 8. About Modal Dialog (rendered centered if show_about is true)
    if player.show_about {
        let ab_w = 340.min(client.w - 20);
        let ab_h = 190.min(client.h - 20);
        let ab_x = client.x + (client.w - ab_w) / 2;
        let ab_y = client.y + (client.h - ab_h) / 2;
        let ab_rect = Rect::new(ab_x, ab_y, ab_w, ab_h);

        // Dim background overlay
        s.fill_rect(client, Color::rgba(0, 0, 0, 160));

        // Frosted modal card
        ui::draw::draw_rounded_rect(s, ab_rect, 10, Color::rgb(15, 23, 42));
        ui::draw::draw_rect_outline(s, ab_rect, 2, Color::rgb(56, 189, 248));

        // Title header
        ui::draw::draw_rounded_rect(s, Rect::new(ab_x + 16, ab_y + 14, 26, 26), 5, Color::rgb(14, 165, 233));
        ui::font::draw_text_font(s, ab_x + 23, ab_y + 17, "S", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 50, ab_y + 14, "SPECTRA MEDIA STUDIO", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 50, ab_y + 28, "v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248), &font_sm);

        ui::draw::draw_line(s, ab_x + 14, ab_y + 48, ab_x + ab_w - 14, ab_y + 48, Color::rgb(48, 54, 61));

        ui::font::draw_text_font(s, ab_x + 16, ab_y + 56, "High-Performance 4K/8K Audio & Video Player", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 74, "Built specifically for NovaOS Desktop & Web.", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 96, "Supported Decoders: AV1, HEVC, VP9, FLAC, MKV, MP4", Color::rgb(74, 222, 128), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 114, "Audio Pipeline: 32-Band FFT, 10-Band EQ, 125% Boost", Color::rgb(245, 158, 11), &font_sm);

        // Close button
        let btn_close_rect = Rect::new(ab_x + (ab_w - 80) / 2, ab_y + ab_h - 36, 80, 24);
        ui::draw::draw_rounded_rect(s, btn_close_rect, 4, Color::rgb(14, 165, 233));
        ui::font::draw_text_font(s, btn_close_rect.x + 18, btn_close_rect.y + 5, "Dismiss", Color::WHITE, &font_sm);
    }
}

/// Renders Vellum PDF Studio inside the client area of Window 10.
fn draw_vellum_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    vellum: &WmVellumApp,
    _palette: &ThemePalette,
) {
    let font_reg = Font::regular();
    let font_sm = Font::small();

    // 1. Studio Workspace Canvas (#0F172A slate backdrop)
    s.fill_rect(client, Color::rgb(15, 23, 42));

    // 2. Top Ribbon Header (h = 32)
    let ribbon_h = 32;
    let ribbon_rect = Rect::new(client.x, client.y, client.w, ribbon_h);
    s.fill_rect(ribbon_rect, Color::rgb(15, 23, 42));
    ui::draw::draw_line(s, client.x, client.y + ribbon_h - 1, client.right(), client.y + ribbon_h - 1, Color::rgb(30, 41, 59));

    // Logo Emblem [V]
    let logo_x = client.x + 8;
    let logo_y = client.y + 4;
    ui::draw::draw_rounded_rect(s, Rect::new(logo_x, logo_y, 22, 22), 4, Color::rgb(14, 165, 233));
    ui::font::draw_text_font(s, logo_x + 6, logo_y + 4, "V", Color::WHITE, &font_reg);

    // Title: VELLUM
    ui::font::draw_text_font(s, logo_x + 28, client.y + 7, "VELLUM", Color::rgb(241, 245, 249), &font_reg);

    // Mode Tabs: Read (70..110), Annotate (114..174), Edit (178..218), Form (222..260)
    let tabs = [
        ("Read", 70, 40, 0),
        ("Annotate", 114, 60, 1),
        ("Edit", 178, 40, 2),
        ("Form", 222, 38, 3),
    ];
    for &(label, rx, rw, m) in &tabs {
        let is_sel = vellum.mode == m;
        let tab_x = client.x + rx;
        let tab_rect = Rect::new(tab_x, client.y + 5, rw, 22);
        if is_sel {
            ui::draw::draw_rounded_rect(s, tab_rect, 4, Color::rgb(30, 41, 59));
            ui::draw::draw_rect_outline(s, tab_rect, 1, Color::rgb(56, 189, 248));
            s.fill_rect(Rect::new(tab_x + 4, tab_rect.bottom() - 2, rw - 8, 2), Color::rgb(56, 189, 248));
            ui::font::draw_text_font(s, tab_x + 6, client.y + 7, label, Color::WHITE, &font_sm);
        } else {
            ui::font::draw_text_font(s, tab_x + 6, client.y + 7, label, Color::rgb(148, 163, 184), &font_sm);
        }
    }

    // Centered Document Title Badge (if width allows)
    if client.w >= 640 {
        let title_str = "NovaOS Architecture & Systems.pdf";
        let tb_w = (title_str.len() as i32 * 6) + 16;
        let tb_x = client.x + (client.w - tb_w) / 2;
        if tb_x > client.x + 370 && tb_x + tb_w < client.x + client.w - 280 {
            ui::draw::draw_rounded_rect(s, Rect::new(tb_x, client.y + 5, tb_w, 20), 4, Color::rgba(0, 0, 0, 50));
            ui::draw::draw_rect_outline(s, Rect::new(tb_x, client.y + 5, tb_w, 20), 1, Color::rgb(51, 65, 85));
            ui::font::draw_text_font(s, tb_x + 8, client.y + 7, title_str, Color::rgb(203, 213, 225), &font_sm);
        }
    }

    // Right View Controls: [Sidebar] [Find] [Night] [Inspect] [?]
    let r_btn_y = client.y + 5;
    let r_btn_h = 22;

    // ? Button
    let btn_about_x = client.x + client.w - 28;
    ui::draw::draw_rounded_rect(s, Rect::new(btn_about_x, r_btn_y, 22, r_btn_h), 4, Color::rgba(255, 255, 255, 12));
    ui::font::draw_text_font(s, btn_about_x + 7, r_btn_y + 4, "?", Color::rgb(148, 163, 184), &font_sm);

    // Inspect
    let btn_insp_x = btn_about_x - 58;
    let insp_col = if vellum.show_inspector { Color::rgb(56, 189, 248) } else { Color::rgba(255, 255, 255, 12) };
    ui::draw::draw_rounded_rect(s, Rect::new(btn_insp_x, r_btn_y, 52, r_btn_h), 4, insp_col);
    ui::font::draw_text_font(s, btn_insp_x + 6, r_btn_y + 4, "Inspect", if vellum.show_inspector { Color::BLACK } else { Color::rgb(226, 232, 240) }, &font_sm);

    // Night
    let btn_night_x = btn_insp_x - 52;
    let night_col = if vellum.night_mode { Color::rgb(14, 165, 233) } else { Color::rgba(255, 255, 255, 12) };
    ui::draw::draw_rounded_rect(s, Rect::new(btn_night_x, r_btn_y, 46, r_btn_h), 4, night_col);
    ui::font::draw_text_font(s, btn_night_x + 6, r_btn_y + 4, "Night", Color::WHITE, &font_sm);

    // Find
    let btn_find_x = btn_night_x - 48;
    let find_col = if vellum.search_open { Color::rgb(56, 189, 248) } else { Color::rgba(255, 255, 255, 12) };
    ui::draw::draw_rounded_rect(s, Rect::new(btn_find_x, r_btn_y, 42, r_btn_h), 4, find_col);
    ui::font::draw_text_font(s, btn_find_x + 6, r_btn_y + 4, "Find", if vellum.search_open { Color::BLACK } else { Color::rgb(226, 232, 240) }, &font_sm);

    // Sidebar
    let btn_sb_x = btn_find_x - 58;
    let sb_col = if vellum.sidebar_open { Color::rgb(30, 41, 59) } else { Color::rgba(255, 255, 255, 12) };
    ui::draw::draw_rounded_rect(s, Rect::new(btn_sb_x, r_btn_y, 52, r_btn_h), 4, sb_col);
    if vellum.sidebar_open {
        ui::draw::draw_rect_outline(s, Rect::new(btn_sb_x, r_btn_y, 52, r_btn_h), 1, Color::rgb(56, 189, 248));
    }
    ui::font::draw_text_font(s, btn_sb_x + 6, r_btn_y + 4, "Sidebar", Color::WHITE, &font_sm);

    let mut content_top = client.y + ribbon_h;

    // 3. macOS Preview Markup Bar (when vellum.mode == 1)
    if vellum.mode == 1 {
        let markup_h = 26;
        let markup_rect = Rect::new(client.x, content_top, client.w, markup_h);
        s.fill_rect(markup_rect, Color::rgb(30, 41, 59));
        ui::draw::draw_line(s, client.x, markup_rect.bottom() - 1, client.right(), markup_rect.bottom() - 1, Color::rgb(51, 65, 85));

        let tools = [
            ("Select", 12, 34, 0),
            ("Text", 52, 28, 1),
            ("Highlight", 86, 56, 2),
            ("Underline", 148, 54, 3),
            ("Strike", 208, 42, 4),
            ("Pen", 256, 28, 5),
            ("Rect", 290, 30, 6),
            ("Box", 326, 44, 7),
            ("Note", 376, 32, 8),
            ("Redact", 414, 42, 9),
        ];
        for &(t_name, tx, tw, t_idx) in &tools {
            let is_sel = vellum.active_tool == t_idx;
            let t_rect = Rect::new(client.x + tx, content_top + 3, tw, 20);
            if is_sel {
                ui::draw::draw_rounded_rect(s, t_rect, 3, Color::rgb(14, 165, 233));
                ui::font::draw_text_font(s, t_rect.x + 4, t_rect.y + 3, t_name, Color::WHITE, &font_sm);
            } else {
                ui::font::draw_text_font(s, t_rect.x + 4, t_rect.y + 3, t_name, Color::rgb(203, 213, 225), &font_sm);
            }
        }

        // Color Swatches
        let swatch_start = (client.x + client.w - 120).max(client.x + 470);
        let swatches = [
            Color::rgb(250, 204, 21),  // Yellow
            Color::rgb(74, 222, 128),  // Green
            Color::rgb(56, 189, 248),  // Cyan
            Color::rgb(244, 63, 94),   // Coral
            Color::rgb(192, 132, 252), // Purple
        ];
        for (i, &col) in swatches.iter().enumerate() {
            let sx = swatch_start + (i as i32) * 20;
            let is_cur = vellum.active_color == i as u8;
            ui::draw::draw_rounded_rect(s, Rect::new(sx, content_top + 4, 16, 16), 8, col);
            if is_cur {
                ui::draw::draw_rect_outline(s, Rect::new(sx - 2, content_top + 2, 20, 20), 1, Color::WHITE);
            }
        }

        content_top += markup_h;
    }

    let status_h = 24;
    let content_bot = client.bottom() - status_h;
    let content_h = content_bot - content_top;

    // 4. Left Sidebar (w = 120)
    let mut canvas_left = client.x;
    if vellum.sidebar_open && client.w >= 480 {
        let sb_w = 120;
        let sb_rect = Rect::new(client.x, content_top, sb_w, content_h);
        s.fill_rect(sb_rect, Color::rgb(15, 23, 42));
        ui::draw::draw_line(s, sb_rect.right() - 1, sb_rect.y, sb_rect.right() - 1, sb_rect.bottom(), Color::rgb(30, 41, 59));

        // Sidebar Tabs: Pages / Outline / Annots
        let stab_w = (sb_w - 8) / 3;
        let stabs = [("Pages", 0), ("Outln", 1), ("Annots", 2)];
        for (i, &(st_name, st_idx)) in stabs.iter().enumerate() {
            let st_x = client.x + 4 + (i as i32) * stab_w;
            let is_sel = vellum.sidebar_tab == st_idx;
            let st_rect = Rect::new(st_x, content_top + 4, stab_w, 18);
            if is_sel {
                ui::draw::draw_rounded_rect(s, st_rect, 3, Color::rgb(14, 165, 233));
                ui::font::draw_text_font(s, st_x + 3, content_top + 6, st_name, Color::WHITE, &font_sm);
            } else {
                ui::font::draw_text_font(s, st_x + 3, content_top + 6, st_name, Color::rgb(148, 163, 184), &font_sm);
            }
        }

        // Thumbnails preview cards
        let card_w = sb_w - 24;
        let card_h = 68;
        for p in 0..vellum.total_pages.min(3) {
            let card_y = content_top + 28 + (p as i32) * 80;
            if card_y + card_h > content_bot {
                break;
            }
            let is_cur = vellum.page == p;
            let card_rect = Rect::new(client.x + 12, card_y, card_w, card_h);
            ui::draw::draw_rounded_rect(s, card_rect, 3, Color::WHITE);
            let border_col = if is_cur { Color::rgb(14, 165, 233) } else { Color::rgb(71, 85, 105) };
            ui::draw::draw_rect_outline(s, card_rect, if is_cur { 2 } else { 1 }, border_col);

            // Mini lines
            s.fill_rect(Rect::new(card_rect.x + 6, card_rect.y + 6, card_w - 12, 3), Color::rgb(203, 213, 225));
            s.fill_rect(Rect::new(card_rect.x + 6, card_rect.y + 12, card_w - 18, 2), Color::rgb(226, 232, 240));
            s.fill_rect(Rect::new(card_rect.x + 6, card_rect.y + 17, card_w - 14, 2), Color::rgb(226, 232, 240));

            let mut page_lbl_buf = [0u8; 16];
            let mut pl_off = 0;
            append_str("Page ", &mut page_lbl_buf, &mut pl_off);
            format_u32((p + 1) as u32, &mut page_lbl_buf, &mut pl_off);
            let page_label = core::str::from_utf8(&page_lbl_buf[..pl_off]).unwrap_or("Page 1");
            let label_col = if is_cur { Color::rgb(56, 189, 248) } else { Color::rgb(148, 163, 184) };
            ui::font::draw_text_font(s, card_rect.x + (card_w - 36) / 2, card_rect.bottom() + 2, page_label, label_col, &font_sm);
        }

        canvas_left += sb_w;
    }

    // 5. Right Inspector (w = 120)
    let mut canvas_right = client.right();
    if vellum.show_inspector && client.w >= 700 {
        let insp_w = 120;
        let insp_x = client.right() - insp_w;
        let insp_rect = Rect::new(insp_x, content_top, insp_w, content_h);
        s.fill_rect(insp_rect, Color::rgb(15, 23, 42));
        ui::draw::draw_line(s, insp_x, content_top, insp_x, content_bot, Color::rgb(30, 41, 59));

        ui::font::draw_text_font(s, insp_x + 8, content_top + 8, "PROPERTIES", Color::rgb(56, 189, 248), &font_sm);
        ui::font::draw_text_font(s, insp_x + 8, content_top + 26, "Size: Letter", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, insp_x + 8, content_top + 42, "Dim: 612x792", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, insp_x + 8, content_top + 60, "Format: PDF 1.7", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, insp_x + 8, content_top + 78, "Security: None", Color::rgb(74, 222, 128), &font_sm);

        canvas_right -= insp_w;
    }

    // 6. Paper Canvas
    let avail_w = canvas_right - canvas_left;
    let paper_w = (avail_w - 40).min(520).max(200);
    let paper_h = (content_h - 24).max(180);
    let paper_x = canvas_left + (avail_w - paper_w) / 2;
    let paper_y = content_top + (content_h - paper_h) / 2;
    let paper_rect = Rect::new(paper_x, paper_y, paper_w, paper_h);

    // Drop shadow
    s.fill_rect(Rect::new(paper_rect.x + 3, paper_rect.y + 3, paper_rect.w, paper_rect.h), Color::rgba(0, 0, 0, 80));

    // Paper background
    let paper_bg = if vellum.night_mode { Color::rgb(18, 24, 38) } else { Color::WHITE };
    let text_primary = if vellum.night_mode { Color::rgb(241, 245, 249) } else { Color::rgb(15, 23, 42) };
    let text_secondary = if vellum.night_mode { Color::rgb(148, 163, 184) } else { Color::rgb(71, 85, 105) };
    s.fill_rect(paper_rect, paper_bg);
    ui::draw::draw_rect_outline(s, paper_rect, 1, Color::rgb(203, 213, 225));

    // Document Typography & Content
    let text_x = paper_rect.x + 24;
    let mut ty = paper_rect.y + 20;

    match vellum.page {
        0 => {
            // Document Header
            ui::font::draw_text_font(s, text_x, ty, "NovaOS Technical Report: Architecture & Systems", text_primary, &font_reg);
            ty += 18;
            ui::font::draw_text_font(s, text_x, ty, "Vellum Studio -- Next-Generation Document Processing in Pure Rust", text_secondary, &font_sm);
            ty += 24;

            // Section 1
            ui::font::draw_text_font(s, text_x, ty, "1. Executive Overview", text_primary, &font_reg);
            ty += 18;

            // Highlight Annotation overlay across body text
            s.fill_rect(Rect::new(text_x - 2, ty - 2, paper_w - 44, 28), Color::rgba(254, 240, 138, 140));

            ui::font::draw_text_font(s, text_x, ty, "NovaOS introduces a unified desktop environment featuring sovereign,", text_primary, &font_sm);
            ty += 14;
            ui::font::draw_text_font(s, text_x, ty, "pure Rust userland applications. Vellum PDF Studio combines editing power,", text_primary, &font_sm);
            ty += 14;
            ui::font::draw_text_font(s, text_x, ty, "macOS Preview markup palettes, and Ubuntu Evince distraction-free reading.", text_primary, &font_sm);
            ty += 22;

            // Callout Box
            let box_rect = Rect::new(text_x, ty, paper_w - 48, 64);
            let box_bg = if vellum.night_mode { Color::rgb(30, 41, 59) } else { Color::rgb(241, 245, 249) };
            s.fill_rect(box_rect, box_bg);
            ui::draw::draw_rect_outline(s, box_rect, 1, Color::rgb(14, 165, 233));
            ui::font::draw_text_font(s, text_x + 8, ty + 8, "Core Architectural Highlights:", Color::rgb(14, 165, 233), &font_sm);
            ui::font::draw_text_font(s, text_x + 12, ty + 24, "- Pure Rust 1.99.1 with zero external C dependencies.", text_secondary, &font_sm);
            ui::font::draw_text_font(s, text_x + 12, ty + 40, "- Built-in RFC 1950/1951 Deflate/Inflate stream compression.", text_secondary, &font_sm);
        }
        1 => {
            ui::font::draw_text_font(s, text_x, ty, "2. Pure Rust Microkernel Architecture", text_primary, &font_reg);
            ty += 20;
            ui::font::draw_text_font(s, text_x, ty, "Memory safety is enforced from hardware boot to graphical presentation.", text_secondary, &font_sm);
            ty += 30;

            // Diagram Box
            let diag_rect = Rect::new(text_x, ty, paper_w - 48, 80);
            s.fill_rect(diag_rect, if vellum.night_mode { Color::rgb(30, 41, 59) } else { Color::rgb(248, 250, 252) });
            ui::draw::draw_rect_outline(s, diag_rect, 1, Color::rgb(100, 116, 139));
            ui::font::draw_text_font(s, text_x + 12, ty + 12, "[ Userland: Aero Browser | Vellum Studio | Spectra ]", Color::rgb(34, 197, 94), &font_sm);
            ui::font::draw_text_font(s, text_x + 12, ty + 34, "[ Compositor / Window Manager (wm-rs) -- ARGB ]", Color::rgb(56, 189, 248), &font_sm);
            ui::font::draw_text_font(s, text_x + 12, ty + 56, "[ NovaOS Pure Rust Microkernel Core -- VM, IPC ]", Color::rgb(244, 63, 94), &font_sm);
        }
        _ => {
            ui::font::draw_text_font(s, text_x, ty, "3. Vellum Document Studio Ecosystem", text_primary, &font_reg);
            ty += 20;
            ui::font::draw_text_font(s, text_x, ty, "Vellum provides end-to-end PDF lifecycle management: reading, in-place editing,", text_secondary, &font_sm);
            ty += 14;
            ui::font::draw_text_font(s, text_x, ty, "freehand annotation, form filling, text extraction, search, and writing.", text_secondary, &font_sm);
        }
    }

    // In Edit mode: draw Acrobat-style text selection dashed rectangle
    if vellum.mode == 2 {
        let edit_rect = Rect::new(text_x - 4, paper_rect.y + 16, paper_w - 40, 24);
        ui::draw::draw_rect_outline(s, edit_rect, 1, Color::rgb(14, 165, 233));
        s.fill_rect(Rect::new(text_x + 360, paper_rect.y + 19, 2, 16), Color::rgb(14, 165, 233));
    }

    // 7. Evince Search Bar (top right overlay)
    if vellum.search_open {
        let sb_w = 230;
        let sb_h = 28;
        let sb_x = client.right() - sb_w - 12;
        let sb_y = content_top + 6;
        let sb_rect = Rect::new(sb_x, sb_y, sb_w, sb_h);
        ui::draw::draw_rounded_rect(s, sb_rect, 6, Color::rgb(15, 23, 42));
        ui::draw::draw_rect_outline(s, sb_rect, 1, Color::rgb(56, 189, 248));

        ui::font::draw_text_font(s, sb_x + 8, sb_y + 7, "NovaOS", Color::WHITE, &font_sm);
        let mut m_buf = [0u8; 16];
        let mut m_off = 0;
        format_u32(vellum.match_idx as u32, &mut m_buf, &mut m_off);
        append_str("/", &mut m_buf, &mut m_off);
        format_u32(vellum.total_matches as u32, &mut m_buf, &mut m_off);
        let match_str = core::str::from_utf8(&m_buf[..m_off]).unwrap_or("1/3");
        ui::font::draw_text_font(s, sb_x + 130, sb_y + 7, match_str, Color::rgb(245, 158, 11), &font_sm);
        ui::font::draw_text_font(s, sb_x + 168, sb_y + 7, "<", Color::rgb(203, 213, 225), &font_sm);
        ui::font::draw_text_font(s, sb_x + 188, sb_y + 7, ">", Color::rgb(203, 213, 225), &font_sm);
        ui::font::draw_text_font(s, sb_x + 212, sb_y + 7, "X", Color::rgb(244, 63, 94), &font_sm);
    }

    // 8. Bottom Status Bar (h = 24)
    let sb_bot_rect = Rect::new(client.x, content_bot, client.w, status_h);
    s.fill_rect(sb_bot_rect, Color::rgb(15, 23, 42));
    ui::draw::draw_line(s, client.x, content_bot, client.right(), content_bot, Color::rgb(30, 41, 59));

    // Page navigation stepper: < Page 1 of 3 >
    ui::font::draw_text_font(s, client.x + 12, content_bot + 6, "<", Color::rgb(203, 213, 225), &font_sm);
    let mut p_buf = [0u8; 24];
    let mut p_off = 0;
    append_str("Page ", &mut p_buf, &mut p_off);
    format_u32((vellum.page + 1) as u32, &mut p_buf, &mut p_off);
    append_str(" of ", &mut p_buf, &mut p_off);
    format_u32(vellum.total_pages as u32, &mut p_buf, &mut p_off);
    let p_info = core::str::from_utf8(&p_buf[..p_off]).unwrap_or("Page 1 of 3");
    ui::font::draw_text_font(s, client.x + 26, content_bot + 6, p_info, Color::rgb(241, 245, 249), &font_sm);
    ui::font::draw_text_font(s, client.x + 98, content_bot + 6, ">", Color::rgb(203, 213, 225), &font_sm);

    // Dimensions
    ui::font::draw_text_font(s, client.x + 120, content_bot + 6, "612 x 792 pt", Color::rgb(148, 163, 184), &font_sm);

    // Zoom Controls
    let mut z_buf = [0u8; 16];
    let mut z_off = 0;
    format_u32(vellum.zoom_pct as u32, &mut z_buf, &mut z_off);
    append_str("%", &mut z_buf, &mut z_off);
    let z_str = core::str::from_utf8(&z_buf[..z_off]).unwrap_or("100%");
    ui::font::draw_text_font(s, client.right() - 84, content_bot + 6, "-", Color::rgb(203, 213, 225), &font_sm);
    ui::font::draw_text_font(s, client.right() - 68, content_bot + 6, z_str, Color::rgb(241, 245, 249), &font_sm);
    ui::font::draw_text_font(s, client.right() - 32, content_bot + 6, "+", Color::rgb(203, 213, 225), &font_sm);

    // 9. Modal About Dialog (if open)
    if vellum.show_about {
        s.fill_rect(client, Color::rgba(0, 0, 0, 160));

        let ab_w = 420.min(client.w - 32);
        let ab_h = 240.min(client.h - 32);
        let ab_x = client.x + (client.w - ab_w) / 2;
        let ab_y = client.y + (client.h - ab_h) / 2;
        let ab_rect = Rect::new(ab_x, ab_y, ab_w, ab_h);

        ui::draw::draw_rounded_rect(s, ab_rect, 8, Color::rgb(15, 23, 42));
        ui::draw::draw_rect_outline(s, ab_rect, 2, Color::rgb(56, 189, 248));

        // Header
        ui::draw::draw_rounded_rect(s, Rect::new(ab_x + 16, ab_y + 14, 26, 26), 5, Color::rgb(14, 165, 233));
        ui::font::draw_text_font(s, ab_x + 24, ab_y + 17, "V", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 52, ab_y + 14, "VELLUM PDF STUDIO", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 52, ab_y + 28, "v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248), &font_sm);

        ui::draw::draw_line(s, ab_x + 14, ab_y + 48, ab_x + ab_w - 14, ab_y + 48, Color::rgb(48, 54, 61));

        ui::font::draw_text_font(s, ab_x + 16, ab_y + 58, "High-Performance PDF Reader, Editor & Authoring Suite", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 76, "Engine Architecture: Adobe Edit + macOS Preview + Evince Read", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 98, "Pure Rust 1.99.1 | Zero C/C++ | Zero External Binaries", Color::rgb(74, 222, 128), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 116, "Full PDF 1.7 Spec: XRef, FlateDecode, Vector Paths, Forms & Redact", Color::rgb(245, 158, 11), &font_sm);

        // Dismiss Button
        let btn_close = Rect::new(ab_x + (ab_w - 80) / 2, ab_y + ab_h - 36, 80, 24);
        ui::draw::draw_rounded_rect(s, btn_close, 4, Color::rgb(14, 165, 233));
        ui::font::draw_text_font(s, btn_close.x + 18, btn_close.y + 5, "Dismiss", Color::WHITE, &font_sm);
    }
}

/// Renders Vortex BitTorrent & Stream Studio inside Window 11 client area.
fn draw_vortex_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    vortex: &WmVortexApp,
    _palette: &ThemePalette,
) {
    let font_reg = Font::regular();
    let font_sm = Font::small();

    // 1. Studio Workspace Canvas
    s.fill_rect(client, Color::rgb(15, 23, 42));

    // 2. Top Ribbon Toolbar (h = 36)
    let ribbon_h = 36;
    let ribbon_rect = Rect::new(client.x, client.y, client.w, ribbon_h);
    s.fill_rect(ribbon_rect, Color::rgb(20, 27, 45));
    ui::draw::draw_line(s, client.x, client.y + ribbon_h - 1, client.right(), client.y + ribbon_h - 1, Color::rgb(51, 65, 85));

    // Logo & Brand
    let logo_x = client.x + 8;
    let logo_y = client.y + 6;
    ui::draw::draw_rounded_rect(s, Rect::new(logo_x, logo_y, 24, 24), 4, Color::rgb(99, 102, 241));
    ui::draw::draw_line(s, logo_x + 5, logo_y + 9, logo_x + 12, logo_y + 16, Color::WHITE);
    ui::draw::draw_line(s, logo_x + 19, logo_y + 9, logo_x + 12, logo_y + 16, Color::WHITE);
    ui::draw::draw_line(s, logo_x + 12, logo_y + 5, logo_x + 12, logo_y + 16, Color::WHITE);
    ui::font::draw_text_font(s, logo_x + 30, client.y + 10, "VORTEX", Color::rgb(241, 245, 249), &font_reg);

    // Toolbar Buttons
    // + Torrent
    let btn_tor = Rect::new(client.x + 72, client.y + 6, 68, 24);
    ui::draw::draw_rounded_rect(s, btn_tor, 4, Color::rgb(30, 41, 59));
    ui::draw::draw_rect_outline(s, btn_tor, 1, Color::rgb(99, 102, 241));
    ui::font::draw_text_font(s, btn_tor.x + 6, btn_tor.y + 5, "+ Torrent", Color::rgb(199, 210, 254), &font_sm);

    // + Stream
    let btn_stm = Rect::new(client.x + 146, client.y + 6, 54, 24);
    ui::draw::draw_rounded_rect(s, btn_stm, 4, Color::rgb(30, 41, 59));
    ui::draw::draw_rect_outline(s, btn_stm, 1, Color::rgb(245, 158, 11));
    ui::font::draw_text_font(s, btn_stm.x + 6, btn_stm.y + 5, "+ Stream", Color::rgb(253, 230, 138), &font_sm);

    // Resume
    let btn_res = Rect::new(client.x + 206, client.y + 6, 54, 24);
    ui::draw::draw_rounded_rect(s, btn_res, 4, Color::rgb(30, 41, 59));
    ui::font::draw_text_font(s, btn_res.x + 6, btn_res.y + 5, "> Resume", Color::rgb(74, 222, 128), &font_sm);

    // Pause
    let btn_pau = Rect::new(client.x + 266, client.y + 6, 48, 24);
    ui::draw::draw_rounded_rect(s, btn_pau, 4, Color::rgb(30, 41, 59));
    ui::font::draw_text_font(s, btn_pau.x + 6, btn_pau.y + 5, "|| Pause", Color::rgb(251, 146, 60), &font_sm);

    // Delete
    let btn_del = Rect::new(client.x + 320, client.y + 6, 48, 24);
    ui::draw::draw_rounded_rect(s, btn_del, 4, Color::rgb(30, 41, 59));
    ui::font::draw_text_font(s, btn_del.x + 8, btn_del.y + 5, "X Del", Color::rgb(248, 113, 113), &font_sm);

    // Global speed badge & About button on right
    let btn_about_x = client.x + client.w - 36;
    ui::draw::draw_rounded_rect(s, Rect::new(btn_about_x, client.y + 6, 26, 24), 4, Color::rgba(255, 255, 255, 16));
    ui::font::draw_text_font(s, btn_about_x + 9, client.y + 10, "?", Color::rgb(203, 213, 225), &font_sm);

    if client.w >= 540 {
        let spd_text = if vortex.download_rate_mb > 0 { "v 31.6 MB/s | ^ 3.4 MB/s" } else { "v 0.0 KB/s | ^ 0.0 KB/s" };
        ui::font::draw_text_font(s, btn_about_x - 170, client.y + 11, spd_text, Color::rgb(56, 189, 248), &font_sm);
    }

    // 3. Category Sidebar (w = 110)
    let side_w = 110;
    let side_rect = Rect::new(client.x, client.y + ribbon_h, side_w, client.h - ribbon_h);
    s.fill_rect(side_rect, Color::rgb(15, 23, 42));
    ui::draw::draw_line(s, client.x + side_w, client.y + ribbon_h, client.x + side_w, client.bottom(), Color::rgb(51, 65, 85));

    let categories = [
        ("All Transfers (4)", 0),
        ("Torrents (2)", 1),
        ("Video Streams (2)", 2),
        ("Active (2)", 3),
        ("Completed (2)", 4),
    ];
    for (i, &(cat_name, cat_idx)) in categories.iter().enumerate() {
        let is_sel = vortex.filter == cat_idx;
        let cy = client.y + ribbon_h + 8 + (i as i32) * 26;
        let c_rect = Rect::new(client.x + 4, cy, side_w - 8, 22);
        if is_sel {
            ui::draw::draw_rounded_rect(s, c_rect, 4, Color::rgb(30, 41, 59));
            s.fill_rect(Rect::new(client.x + 4, cy + 3, 3, 16), Color::rgb(99, 102, 241));
            ui::font::draw_text_font(s, client.x + 10, cy + 4, cat_name, Color::WHITE, &font_sm);
        } else {
            ui::font::draw_text_font(s, client.x + 10, cy + 4, cat_name, Color::rgb(148, 163, 184), &font_sm);
        }
    }

    // 4. Main Queue Table (x: client.x + side_w .. client.right(), y: ribbon_h .. 170)
    let table_x = client.x + side_w + 1;
    let table_w = client.w - side_w - 1;
    let table_h = (client.h - ribbon_h - 110).max(80);

    // Header row
    let th_y = client.y + ribbon_h;
    s.fill_rect(Rect::new(table_x, th_y, table_w, 22), Color::rgb(20, 27, 45));
    ui::draw::draw_line(s, table_x, th_y + 21, client.right(), th_y + 21, Color::rgb(51, 65, 85));
    ui::font::draw_text_font(s, table_x + 8, th_y + 5, "Transfer Name", Color::rgb(148, 163, 184), &font_sm);
    if table_w >= 300 {
        ui::font::draw_text_font(s, table_x + 180, th_y + 5, "Size", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, table_x + 230, th_y + 5, "Progress", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, table_x + 310, th_y + 5, "Speed", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, table_x + 380, th_y + 5, "Peers / Streams", Color::rgb(148, 163, 184), &font_sm);
    }

    // 4 Task Rows
    let tasks = [
        ("[TORRENT] NovaOS-2.0-Enterprise.iso", "840 MB", 74, "14.1 MB/s", "54 Peers", Color::rgb(74, 222, 128)),
        ("[STREAM] YouTube: NovaOS Keynote FHD", "184 MB", 62, "17.5 MB/s", "8 Chunks", Color::rgb(251, 146, 60)),
        ("[TORRENT] Linux-Kernel-6.12.tar.xz", "142 MB", 100, "Seeding", "120 Peers", Color::rgb(56, 189, 248)),
        ("[STREAM] Vimeo: Concurrency In Depth", "96 MB", 100, "Complete", "4 Chunks", Color::rgb(168, 85, 247)),
    ];

    for (idx, &(t_name, t_size, t_pct, t_spd, t_extra, p_col)) in tasks.iter().enumerate() {
        let row_y = th_y + 22 + (idx as i32) * 26;
        if row_y + 24 > client.y + ribbon_h + table_h {
            break;
        }
        let is_sel = vortex.selected_task == idx;
        let row_rect = Rect::new(table_x, row_y, table_w, 24);
        if is_sel {
            s.fill_rect(row_rect, Color::rgb(30, 41, 59));
            ui::draw::draw_rect_outline(s, row_rect, 1, Color::rgb(99, 102, 241));
        } else if idx % 2 == 1 {
            s.fill_rect(row_rect, Color::rgba(255, 255, 255, 4));
        }

        ui::font::draw_text_font(s, table_x + 8, row_y + 5, t_name, if is_sel { Color::WHITE } else { Color::rgb(226, 232, 240) }, &font_sm);
        if table_w >= 300 {
            ui::font::draw_text_font(s, table_x + 180, row_y + 5, t_size, Color::rgb(148, 163, 184), &font_sm);

            // Mini progress bar
            let p_w = 60;
            let p_rect = Rect::new(table_x + 230, row_y + 7, p_w, 10);
            ui::draw::draw_rounded_rect(s, p_rect, 2, Color::rgb(15, 23, 42));
            let fill_w = (p_w * t_pct) / 100;
            if fill_w > 0 {
                s.fill_rect(Rect::new(p_rect.x, p_rect.y, fill_w, p_rect.h), p_col);
            }

            ui::font::draw_text_font(s, table_x + 310, row_y + 5, t_spd, p_col, &font_sm);
            ui::font::draw_text_font(s, table_x + 380, row_y + 5, t_extra, Color::rgb(203, 213, 225), &font_sm);
        }
    }

    // 5. Bottom Inspector Panel
    let insp_y = client.y + ribbon_h + table_h;
    let insp_h = client.bottom() - insp_y;
    if insp_h >= 60 {
        s.fill_rect(Rect::new(table_x, insp_y, table_w, insp_h), Color::rgb(15, 23, 42));
        ui::draw::draw_line(s, table_x, insp_y, client.right(), insp_y, Color::rgb(51, 65, 85));

        // Inspector Tab Bar
        let tabs = ["Visualizer", "Peers & Streams", "Metadata & Info"];
        let mut tx = table_x + 8;
        for (t_idx, &t_title) in tabs.iter().enumerate() {
            let is_act = vortex.inspector_tab == (t_idx as u8);
            let t_col = if is_act { Color::rgb(99, 102, 241) } else { Color::rgb(148, 163, 184) };
            ui::font::draw_text_font(s, tx, insp_y + 6, t_title, t_col, &font_sm);
            if is_act {
                s.fill_rect(Rect::new(tx, insp_y + 20, 68, 2), Color::rgb(99, 102, 241));
            }
            tx += 80;
        }

        let content_y = insp_y + 26;
        match vortex.inspector_tab {
            0 => {
                // Visualizer tab
                if vortex.selected_task == 0 || vortex.selected_task == 2 {
                    // Transmission / uTorrent style 2D Piece Grid
                    ui::font::draw_text_font(s, table_x + 8, content_y, "BitTorrent Piece Map: 962 / 1,300 pieces verified (SHA-1 verified, Rarest-First scheduler)", Color::rgb(74, 222, 128), &font_sm);
                    let grid_y = content_y + 16;
                    let block_w = 12;
                    let block_h = 8;
                    let cols = ((table_w - 20) / (block_w + 2)).min(28).max(4);
                    for i in 0..(cols * 3) {
                        let col = i % cols;
                        let row = i / cols;
                        let bx = table_x + 8 + col * (block_w + 2);
                        let by = grid_y + row * (block_h + 2);
                        let col_val = if i < (cols * 2) {
                            Color::rgb(34, 197, 94) // Done
                        } else if i < (cols * 2 + 3) {
                            Color::rgb(245, 158, 11) // In flight
                        } else {
                            Color::rgb(30, 41, 59) // Pending
                        };
                        s.fill_rect(Rect::new(bx, by, block_w, block_h), col_val);
                    }
                } else {
                    // IDM Multi-threaded Range Connection Bars
                    ui::font::draw_text_font(s, table_x + 8, content_y, "IDM Multi-threaded Range Accelerator: 8 active chunk streams, 17.5 MB/s bandwidth", Color::rgb(56, 189, 248), &font_sm);
                    let bar_w = ((table_w - 30) / 8).max(20);
                    let bar_y = content_y + 18;
                    for c in 0..8 {
                        let bx = table_x + 8 + c * (bar_w + 2);
                        let b_rect = Rect::new(bx, bar_y, bar_w, 18);
                        ui::draw::draw_rounded_rect(s, b_rect, 2, Color::rgb(30, 41, 59));
                        let pct = if c < 5 { 100 } else if c == 5 { 60 } else { 25 };
                        let f_w = (bar_w * pct) / 100;
                        if f_w > 0 {
                            s.fill_rect(Rect::new(bx, bar_y, f_w, 18), Color::rgb(14, 165, 233));
                        }
                    }
                }
            }
            1 => {
                // Peers & Connections
                ui::font::draw_text_font(s, table_x + 8, content_y, "* Swarm: 198.51.100.24:6881 [Transmission 4.0] 4.2 MB/s | Encrypted MSE/PE", Color::rgb(226, 232, 240), &font_sm);
                ui::font::draw_text_font(s, table_x + 8, content_y + 16, "* Swarm: 203.0.113.88:51413 [Vortex Swarm]    5.8 MB/s | Encrypted MSE/PE", Color::rgb(74, 222, 128), &font_sm);
                ui::font::draw_text_font(s, table_x + 8, content_y + 32, "* Swarm: 192.0.2.77:6881    [libtorrent 2.0]   3.1 MB/s | Encrypted MSE/PE", Color::rgb(56, 189, 248), &font_sm);
            }
            _ => {
                // Info & Metadata
                ui::font::draw_text_font(s, table_x + 8, content_y, "InfoHash: 2fd4e1c67a2d28fced849ee1bb76e7391b93eb12 | BEP-0003 & BEP-0009 Magnet", Color::rgb(245, 158, 11), &font_sm);
                ui::font::draw_text_font(s, table_x + 8, content_y + 16, "Pieces: 1,300 x 512 KB | Output Destination: /root/Downloads", Color::rgb(226, 232, 240), &font_sm);
                ui::font::draw_text_font(s, table_x + 8, content_y + 32, "Announce: udp://tracker.opentrackr.org:1337/announce (Working, 54 seeders)", Color::rgb(74, 222, 128), &font_sm);
            }
        }
    }

    // 6. Modal About Dialog
    if vortex.show_about {
        s.fill_rect(client, Color::rgba(0, 0, 0, 160));

        let ab_w = 420.min(client.w - 32);
        let ab_h = 240.min(client.h - 32);
        let ab_x = client.x + (client.w - ab_w) / 2;
        let ab_y = client.y + (client.h - ab_h) / 2;
        let ab_rect = Rect::new(ab_x, ab_y, ab_w, ab_h);

        ui::draw::draw_rounded_rect(s, ab_rect, 8, Color::rgb(15, 23, 42));
        ui::draw::draw_rect_outline(s, ab_rect, 2, Color::rgb(99, 102, 241));

        // Header
        ui::draw::draw_rounded_rect(s, Rect::new(ab_x + 16, ab_y + 14, 26, 26), 5, Color::rgb(99, 102, 241));
        ui::draw::draw_line(s, ab_x + 21, ab_y + 23, ab_x + 29, ab_y + 31, Color::WHITE);
        ui::draw::draw_line(s, ab_x + 37, ab_y + 23, ab_x + 29, ab_y + 31, Color::WHITE);
        ui::font::draw_text_font(s, ab_x + 52, ab_y + 14, "VORTEX STUDIO", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 52, ab_y + 28, "v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248), &font_sm);

        ui::draw::draw_line(s, ab_x + 14, ab_y + 48, ab_x + ab_w - 14, ab_y + 48, Color::rgb(48, 54, 61));

        ui::font::draw_text_font(s, ab_x + 16, ab_y + 58, "Unified BitTorrent Client & Intelligent Stream Accelerator", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 76, "Engine Architecture: Transmission Swarm + IDM Range Slicing", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 98, "Pure Rust 1.99.1 | Zero C/C++ | Zero External Binaries", Color::rgb(74, 222, 128), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 116, "Registered in NovaOS App Store (Internet Category)", Color::rgb(245, 158, 11), &font_sm);

        // Dismiss Button
        let btn_close = Rect::new(ab_x + (ab_w - 80) / 2, ab_y + ab_h - 36, 80, 24);
        ui::draw::draw_rounded_rect(s, btn_close, 4, Color::rgb(99, 102, 241));
        ui::font::draw_text_font(s, btn_close.x + 18, btn_close.y + 5, "Dismiss", Color::WHITE, &font_sm);
    }
}

/// Renders Zenith Office Suite inside Window 12 client area.
fn draw_zenith_client_body<S: Surface + ?Sized>(
    s: &mut S,
    client: Rect,
    zenith: &WmZenithApp,
    _palette: &ThemePalette,
) {
    let font_reg = Font::regular();
    let font_sm = Font::small();

    // 1. Dark office background workspace
    s.fill_rect(client, Color::rgb(24, 28, 36));

    // 2. Top Module Switcher Bar (h = 26)
    let switcher_h = 26;
    let switcher_rect = Rect::new(client.x, client.y, client.w, switcher_h);
    s.fill_rect(switcher_rect, Color::rgb(15, 23, 42));
    ui::draw::draw_line(s, client.x, client.y + switcher_h - 1, client.right(), client.y + switcher_h - 1, Color::rgb(51, 65, 85));

    let modules = [
        ("[W] Writer", 0, Color::rgb(0, 120, 212)),
        ("[X] Calc", 1, Color::rgb(16, 124, 65)),
        ("[P] Present", 2, Color::rgb(216, 59, 1)),
        ("[B] Base", 3, Color::rgb(168, 0, 0)),
    ];

    let mut mod_x = client.x + 4;
    for &(mod_name, mod_idx, mod_color) in modules.iter() {
        let is_active = zenith.active_module == mod_idx;
        let btn_w = 72;
        let btn_rect = Rect::new(mod_x, client.y + 2, btn_w, 22);
        if is_active {
            ui::draw::draw_rounded_rect(s, btn_rect, 3, mod_color);
            ui::font::draw_text_font(s, mod_x + 6, client.y + 6, mod_name, Color::WHITE, &font_sm);
        } else {
            ui::draw::draw_rounded_rect(s, btn_rect, 3, Color::rgb(30, 41, 59));
            ui::font::draw_text_font(s, mod_x + 6, client.y + 6, mod_name, Color::rgb(203, 213, 225), &font_sm);
        }
        mod_x += btn_w + 6;
    }

    // Right side: modal About button
    let btn_about_x = client.x + client.w - 32;
    ui::draw::draw_rounded_rect(s, Rect::new(btn_about_x, client.y + 2, 24, 22), 3, Color::rgba(255, 255, 255, 20));
    ui::font::draw_text_font(s, btn_about_x + 8, client.y + 6, "?", Color::WHITE, &font_sm);

    if client.w >= 520 {
        ui::font::draw_text_font(s, btn_about_x - 220, client.y + 7, "Zenith Office Suite 1.0 (Pure Rust)", Color::rgb(148, 163, 184), &font_sm);
    }

    // 3. Ribbon Tab Bar (h = 20)
    let ribbon_tabs_y = client.y + switcher_h;
    let tabs_rect = Rect::new(client.x, ribbon_tabs_y, client.w, 20);
    s.fill_rect(tabs_rect, Color::rgb(30, 41, 59));
    ui::draw::draw_line(s, client.x, ribbon_tabs_y + 19, client.right(), ribbon_tabs_y + 19, Color::rgb(51, 65, 85));

    let ribbon_tab_names = ["Home", "Insert", "Layout", "Formulas", "View"];
    let mut tab_x = client.x + 8;
    for (i, &t_name) in ribbon_tab_names.iter().enumerate() {
        let is_cur = zenith.ribbon_tab == (i as u8);
        let t_sz = font_sm.measure_str(t_name);
        let tab_w = t_sz.w + 16;
        if is_cur {
            s.fill_rect(Rect::new(tab_x, ribbon_tabs_y, tab_w, 20), Color::rgb(15, 23, 42));
            s.fill_rect(Rect::new(tab_x, ribbon_tabs_y + 18, tab_w, 2), Color::rgb(0, 120, 212));
            ui::font::draw_text_font(s, tab_x + 8, ribbon_tabs_y + 4, t_name, Color::WHITE, &font_sm);
        } else {
            ui::font::draw_text_font(s, tab_x + 8, ribbon_tabs_y + 4, t_name, Color::rgb(148, 163, 184), &font_sm);
        }
        tab_x += tab_w + 4;
    }

    // 4. Ribbon Command Controls Bar (h = 38)
    let controls_y = ribbon_tabs_y + 20;
    let controls_rect = Rect::new(client.x, controls_y, client.w, 38);
    s.fill_rect(controls_rect, Color::rgb(15, 23, 42));
    ui::draw::draw_line(s, client.x, controls_y + 37, client.right(), controls_y + 37, Color::rgb(51, 65, 85));

    // Font Family Dropdown Button
    let font_btn = Rect::new(client.x + 8, controls_y + 6, 110, 26);
    ui::draw::draw_rounded_rect(s, font_btn, 4, Color::rgb(30, 41, 59));
    ui::draw::draw_rect_outline(s, font_btn, 1, Color::rgb(71, 85, 105));
    ui::font::draw_text_font(s, font_btn.x + 6, font_btn.y + 6, zenith.font_name(), Color::WHITE, &font_sm);

    // Font Size Button
    let size_btn = Rect::new(client.x + 124, controls_y + 6, 36, 26);
    ui::draw::draw_rounded_rect(s, size_btn, 4, Color::rgb(30, 41, 59));
    ui::draw::draw_rect_outline(s, size_btn, 1, Color::rgb(71, 85, 105));
    let sz_str = if zenith.font_size == 9 { "9 pt" } else if zenith.font_size == 14 { "14 pt" } else if zenith.font_size >= 16 { "16 pt" } else { "11 pt" };
    ui::font::draw_text_font(s, size_btn.x + 4, size_btn.y + 6, sz_str, Color::WHITE, &font_sm);

    // [B] Bold button
    let bold_btn = Rect::new(client.x + 166, controls_y + 6, 26, 26);
    let bold_bg = if zenith.bold { Color::rgb(0, 120, 212) } else { Color::rgb(30, 41, 59) };
    ui::draw::draw_rounded_rect(s, bold_btn, 4, bold_bg);
    ui::font::draw_text_font(s, bold_btn.x + 9, bold_btn.y + 6, "B", Color::WHITE, &font_sm);

    // [I] Italic button
    let ital_btn = Rect::new(client.x + 196, controls_y + 6, 26, 26);
    let ital_bg = if zenith.italic { Color::rgb(0, 120, 212) } else { Color::rgb(30, 41, 59) };
    ui::draw::draw_rounded_rect(s, ital_btn, 4, ital_bg);
    ui::font::draw_text_font(s, ital_btn.x + 10, ital_btn.y + 6, "I", Color::WHITE, &font_sm);

    // [U] Underline button
    let und_btn = Rect::new(client.x + 226, controls_y + 6, 26, 26);
    let und_bg = if zenith.underline { Color::rgb(0, 120, 212) } else { Color::rgb(30, 41, 59) };
    ui::draw::draw_rounded_rect(s, und_btn, 4, und_bg);
    ui::font::draw_text_font(s, und_btn.x + 8, und_btn.y + 6, "U", Color::WHITE, &font_sm);

    // Align buttons [L] [C] [R]
    let align_l = Rect::new(client.x + 258, controls_y + 6, 22, 26);
    let align_c = Rect::new(client.x + 284, controls_y + 6, 22, 26);
    let align_r = Rect::new(client.x + 310, controls_y + 6, 22, 26);
    ui::draw::draw_rounded_rect(s, align_l, 4, if zenith.align == 0 { Color::rgb(0, 120, 212) } else { Color::rgb(30, 41, 59) });
    ui::font::draw_text_font(s, align_l.x + 6, align_l.y + 6, "L", Color::WHITE, &font_sm);
    ui::draw::draw_rounded_rect(s, align_c, 4, if zenith.align == 1 { Color::rgb(0, 120, 212) } else { Color::rgb(30, 41, 59) });
    ui::font::draw_text_font(s, align_c.x + 6, align_c.y + 6, "C", Color::WHITE, &font_sm);
    ui::draw::draw_rounded_rect(s, align_r, 4, if zenith.align == 2 { Color::rgb(0, 120, 212) } else { Color::rgb(30, 41, 59) });
    ui::font::draw_text_font(s, align_r.x + 6, align_r.y + 6, "R", Color::WHITE, &font_sm);

    // 5. Active Module Content Area
    let body_y = controls_y + 38;
    let body_h = (client.bottom() - body_y).max(0);

    match zenith.active_module {
        0 => {
            // --- Zenith Writer (Word / Google Docs / LibreOffice Writer) ---
            let page_w = (client.w - 80).clamp(240, 560);
            let page_h = body_h - 28;
            let page_x = client.x + (client.w - page_w) / 2;
            let page_y = body_y + 6;

            // Page shadow & sheet
            s.fill_rect(Rect::new(page_x + 3, page_y + 3, page_w, page_h), Color::rgba(0, 0, 0, 80));
            s.fill_rect(Rect::new(page_x, page_y, page_w, page_h), Color::rgb(255, 255, 255));

            // Margin Ruler strip at top of page
            s.fill_rect(Rect::new(page_x, page_y, page_w, 12), Color::rgb(241, 245, 249));
            ui::draw::draw_line(s, page_x, page_y + 11, page_x + page_w, page_y + 11, Color::rgb(203, 213, 225));
            for r in (0..page_w).step_by(40) {
                ui::draw::draw_line(s, page_x + r, page_y + 4, page_x + r, page_y + 11, Color::rgb(148, 163, 184));
            }

            let text_dark = Color::rgb(15, 23, 42);
            let text_blue = Color::rgb(0, 120, 212);
            let text_muted = Color::rgb(100, 116, 139);

            let content_x = page_x + 24;
            let mut cur_y = page_y + 24;

            // Title H1
            ui::font::draw_text_font(s, content_x, cur_y, "NovaOS Sovereign Architecture Proposal", text_blue, &font_reg);
            cur_y += 24;

            // Heading H2
            ui::font::draw_text_font(s, content_x, cur_y, "1. Executive Summary & Core Objectives", text_dark, &font_sm);
            cur_y += 18;

            // Body Paragraphs
            ui::font::draw_text_font(s, content_x, cur_y, "This document outlines the sovereign productivity suite designed natively for NovaOS.", text_dark, &font_sm);
            cur_y += 14;
            ui::font::draw_text_font(s, content_x, cur_y, "Written in pure Rust 1.99.1 with zero external C/C++ or FFmpeg dependencies.", text_dark, &font_sm);
            cur_y += 18;

            // Heading H2
            ui::font::draw_text_font(s, content_x, cur_y, "2. Microsoft Office & LibreOffice Font Parity", text_dark, &font_sm);
            cur_y += 18;
            ui::font::draw_text_font(s, content_x, cur_y, "* Microsoft Families: Calibri, Times New Roman, Arial, Segoe UI, Cambria, Consolas", text_muted, &font_sm);
            cur_y += 14;
            ui::font::draw_text_font(s, content_x, cur_y, "* LibreOffice & Ubuntu: Liberation Sans, Liberation Serif, Liberation Mono, Ubuntu", text_muted, &font_sm);
            cur_y += 20;

            // Embedded Table
            if page_h >= 200 {
                let tbl_w = (page_w - 48).min(380);
                let tbl_x = content_x;
                let th_rect = Rect::new(tbl_x, cur_y, tbl_w, 20);
                s.fill_rect(th_rect, Color::rgb(226, 232, 240));
                ui::draw::draw_rect_outline(s, th_rect, 1, Color::rgb(203, 213, 225));
                ui::font::draw_text_font(s, tbl_x + 8, cur_y + 4, "Component", text_dark, &font_sm);
                ui::font::draw_text_font(s, tbl_x + 130, cur_y + 4, "Synthesis Target", text_dark, &font_sm);
                ui::font::draw_text_font(s, tbl_x + 270, cur_y + 4, "Status", text_dark, &font_sm);
                cur_y += 20;

                let row1 = Rect::new(tbl_x, cur_y, tbl_w, 18);
                s.fill_rect(row1, Color::rgb(248, 250, 252));
                ui::draw::draw_rect_outline(s, row1, 1, Color::rgb(226, 232, 240));
                ui::font::draw_text_font(s, tbl_x + 8, cur_y + 3, "Zenith Writer", text_dark, &font_sm);
                ui::font::draw_text_font(s, tbl_x + 130, cur_y + 3, "Word / Google Docs", text_muted, &font_sm);
                ui::font::draw_text_font(s, tbl_x + 270, cur_y + 3, "[PASSED]", Color::rgb(16, 124, 65), &font_sm);
                cur_y += 18;

                let row2 = Rect::new(tbl_x, cur_y, tbl_w, 18);
                s.fill_rect(row2, Color::WHITE);
                ui::draw::draw_rect_outline(s, row2, 1, Color::rgb(226, 232, 240));
                ui::font::draw_text_font(s, tbl_x + 8, cur_y + 3, "Zenith Calc", text_dark, &font_sm);
                ui::font::draw_text_font(s, tbl_x + 130, cur_y + 3, "Excel / Google Sheets", text_muted, &font_sm);
                ui::font::draw_text_font(s, tbl_x + 270, cur_y + 3, "[PASSED]", Color::rgb(16, 124, 65), &font_sm);
            }

            // Bottom Status Bar
            let sb_y = client.bottom() - 20;
            s.fill_rect(Rect::new(client.x, sb_y, client.w, 20), Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, client.x, sb_y, client.right(), sb_y, Color::rgb(51, 65, 85));
            ui::font::draw_text_font(s, client.x + 12, sb_y + 4, "Page 1 of 1 | 1,420 words | 8,940 characters | Calibri 11pt | 100% Zoom", Color::rgb(148, 163, 184), &font_sm);
        }
        1 => {
            // --- Zenith Calc (Excel / Google Sheets / LibreOffice Calc) ---
            // Formula Bar (h = 24)
            let fx_y = body_y;
            s.fill_rect(Rect::new(client.x, fx_y, client.w, 24), Color::rgb(30, 41, 59));
            ui::draw::draw_line(s, client.x, fx_y + 23, client.right(), fx_y + 23, Color::rgb(51, 65, 85));

            // Name Box
            let name_rect = Rect::new(client.x + 6, fx_y + 3, 44, 18);
            ui::draw::draw_rounded_rect(s, name_rect, 2, Color::rgb(15, 23, 42));
            ui::font::draw_text_font(s, name_rect.x + 6, name_rect.y + 3, "C5", Color::rgb(74, 222, 128), &font_sm);

            // fx label
            ui::font::draw_text_font(s, client.x + 58, fx_y + 5, "fx", Color::rgb(148, 163, 184), &font_sm);

            // Formula Input
            let formula_rect = Rect::new(client.x + 76, fx_y + 3, client.w - 86, 18);
            ui::draw::draw_rounded_rect(s, formula_rect, 2, Color::rgb(15, 23, 42));
            ui::font::draw_text_font(s, formula_rect.x + 8, formula_rect.y + 3, "=SUM(B2:B5)", Color::rgb(226, 232, 240), &font_sm);

            // Matrix headers & Grid
            let grid_y = fx_y + 24;
            let col_w = 78;
            let row_h = 20;

            // Column Headers A, B, C, D, E, F
            s.fill_rect(Rect::new(client.x, grid_y, 36, row_h), Color::rgb(20, 27, 45));
            let col_letters = ["A", "B", "C", "D", "E", "F"];
            for (ci, &let_str) in col_letters.iter().enumerate() {
                let cx = client.x + 36 + (ci as i32) * col_w;
                let c_rect = Rect::new(cx, grid_y, col_w, row_h);
                s.fill_rect(c_rect, Color::rgb(30, 41, 59));
                ui::draw::draw_rect_outline(s, c_rect, 1, Color::rgb(51, 65, 85));
                ui::font::draw_text_font(s, cx + col_w / 2 - 4, grid_y + 4, let_str, Color::rgb(203, 213, 225), &font_sm);
            }

            // Cell Data Rows
            let sheet_data = [
                ("1", ["Quarter", "Revenue", "Expenses", "Net Profit", "Margin", ""]),
                ("2", ["Q1-2026", "$45,000", "$28,000", "$17,000", "37.7%", ""]),
                ("3", ["Q2-2026", "$52,000", "$31,000", "$21,000", "40.3%", ""]),
                ("4", ["Q3-2026", "$68,000", "$39,000", "$29,000", "42.6%", ""]),
                ("5", ["Q4-2026", "$85,000", "$47,200", "$37,800", "44.4%", ""]),
                ("6", ["TOTAL", "$250,000", "$145,200", "$104,800", "41.9%", ""]),
            ];

            for (ri, &(row_lbl, row_vals)) in sheet_data.iter().enumerate() {
                let ry = grid_y + row_h + (ri as i32) * row_h;
                if ry + row_h > client.bottom() - 20 {
                    break;
                }
                // Row header number
                let rh_rect = Rect::new(client.x, ry, 36, row_h);
                s.fill_rect(rh_rect, Color::rgb(30, 41, 59));
                ui::draw::draw_rect_outline(s, rh_rect, 1, Color::rgb(51, 65, 85));
                ui::font::draw_text_font(s, rh_rect.x + 12, ry + 4, row_lbl, Color::rgb(203, 213, 225), &font_sm);

                for (ci, &val) in row_vals.iter().enumerate() {
                    let cx = client.x + 36 + (ci as i32) * col_w;
                    let cell_rect = Rect::new(cx, ry, col_w, row_h);
                    let is_active_cell = zenith.active_cell_row == ri && zenith.active_cell_col == ci;

                    if is_active_cell {
                        s.fill_rect(cell_rect, Color::rgb(15, 23, 42));
                        ui::draw::draw_rect_outline(s, cell_rect, 2, Color::rgb(16, 124, 65));
                        s.fill_rect(Rect::new(cell_rect.right() - 4, cell_rect.bottom() - 4, 4, 4), Color::rgb(16, 124, 65));
                    } else if ri == 0 {
                        s.fill_rect(cell_rect, Color::rgb(24, 32, 47));
                        ui::draw::draw_rect_outline(s, cell_rect, 1, Color::rgb(45, 55, 72));
                    } else {
                        s.fill_rect(cell_rect, Color::rgb(15, 23, 42));
                        ui::draw::draw_rect_outline(s, cell_rect, 1, Color::rgb(33, 41, 54));
                    }

                    let t_col = if ri == 0 { Color::rgb(74, 222, 128) } else if ri == 5 { Color::rgb(245, 158, 11) } else { Color::WHITE };
                    ui::font::draw_text_font(s, cx + 6, ry + 4, val, t_col, &font_sm);
                }
            }

            // Bottom Sheet Tabs & Status Bar
            let sb_y = client.bottom() - 20;
            s.fill_rect(Rect::new(client.x, sb_y, client.w, 20), Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, client.x, sb_y, client.right(), sb_y, Color::rgb(51, 65, 85));

            let tab_rect = Rect::new(client.x + 10, sb_y + 2, 110, 16);
            ui::draw::draw_rounded_rect(s, tab_rect, 2, Color::rgb(16, 124, 65));
            ui::font::draw_text_font(s, tab_rect.x + 6, tab_rect.y + 2, "Sheet1: Forecast", Color::WHITE, &font_sm);

            ui::font::draw_text_font(s, client.x + 140, sb_y + 4, "READY | SUM=$145,200 | AVERAGE=$36,300 | COUNT=4 | MIN=$28,000", Color::rgb(148, 163, 184), &font_sm);
        }
        2 => {
            // --- Zenith Present (PowerPoint / Google Slides / LibreOffice Impress) ---
            let strip_w = 110;
            let strip_rect = Rect::new(client.x, body_y, strip_w, body_h);
            s.fill_rect(strip_rect, Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, client.x + strip_w, body_y, client.x + strip_w, client.bottom(), Color::rgb(51, 65, 85));

            let slides = [
                ("1. Keynote Title", Color::rgb(216, 59, 1)),
                ("2. Architecture", Color::rgb(16, 124, 65)),
                ("3. Sovereign Core", Color::rgb(0, 120, 212)),
            ];

            for (si, &(s_title, s_accent)) in slides.iter().enumerate() {
                let sy = body_y + 8 + (si as i32) * 58;
                let s_rect = Rect::new(client.x + 8, sy, strip_w - 16, 48);
                let is_sel = zenith.active_slide == si;

                s.fill_rect(s_rect, Color::rgb(30, 41, 59));
                if is_sel {
                    ui::draw::draw_rect_outline(s, s_rect, 2, Color::rgb(216, 59, 1));
                } else {
                    ui::draw::draw_rect_outline(s, s_rect, 1, Color::rgb(51, 65, 85));
                }

                s.fill_rect(Rect::new(s_rect.x + 4, s_rect.y + 4, 16, 12), s_accent);
                ui::font::draw_text_font(s, s_rect.x + 6, s_rect.y + 28, s_title, if is_sel { Color::WHITE } else { Color::rgb(148, 163, 184) }, &font_sm);
            }

            // Center 16:9 Presentation Stage
            let stage_x = client.x + strip_w + 12;
            let stage_w = (client.w - strip_w - 24).clamp(240, 640);
            let stage_h = (stage_w * 9) / 16;
            let stage_y = body_y + 12;

            let slide_canvas = Rect::new(stage_x, stage_y, stage_w, stage_h);
            s.fill_rect(Rect::new(stage_x + 4, stage_y + 4, stage_w, stage_h), Color::rgba(0, 0, 0, 90));
            ui::draw::draw_rounded_rect(s, slide_canvas, 6, Color::rgb(30, 41, 59));
            ui::draw::draw_rect_outline(s, slide_canvas, 1, Color::rgb(71, 85, 105));

            // Slide Header Banner
            s.fill_rect(Rect::new(stage_x, stage_y, stage_w, 36), Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, stage_x, stage_y + 35, stage_x + stage_w, stage_y + 35, Color::rgb(216, 59, 1));
            ui::font::draw_text_font(s, stage_x + 16, stage_y + 10, "NovaOS Sovereign Pure Rust Architecture", Color::WHITE, &font_reg);

            // Slide Content Bullets
            let bx = stage_x + 20;
            let mut by = stage_y + 50;
            ui::font::draw_text_font(s, bx, by, "* Pure Rust 1.99.1 - Zero external C/C++ runtime dependencies", Color::rgb(226, 232, 240), &font_sm);
            by += 20;
            ui::font::draw_text_font(s, bx, by, "* 11 Embedded Typography Families (Calibri, Times New Roman, Segoe UI, Ubuntu)", Color::rgb(226, 232, 240), &font_sm);
            by += 20;
            ui::font::draw_text_font(s, bx, by, "* Dynamic Calc formula engine with dependency graph recalculation", Color::rgb(74, 222, 128), &font_sm);
            by += 20;
            ui::font::draw_text_font(s, bx, by, "* High-performance 16:9 presentation studio with speaker notes", Color::rgb(56, 189, 248), &font_sm);

            // Bottom Speaker Notes
            let notes_y = stage_y + stage_h + 8;
            if notes_y + 24 <= client.bottom() {
                let notes_rect = Rect::new(stage_x, notes_y, stage_w, client.bottom() - notes_y - 4);
                ui::draw::draw_rounded_rect(s, notes_rect, 4, Color::rgb(15, 23, 42));
                ui::draw::draw_rect_outline(s, notes_rect, 1, Color::rgb(51, 65, 85));
                ui::font::draw_text_font(s, notes_rect.x + 8, notes_rect.y + 4, "Speaker Notes: Highlight sub-millisecond document loading time and zero FFmpeg/libvpx bloat.", Color::rgb(245, 158, 11), &font_sm);
            }
        }
        3 => {
            // --- Zenith Base (Access / LibreOffice Base) ---
            let tbl_w = 120;
            let tbl_rect = Rect::new(client.x, body_y, tbl_w, body_h);
            s.fill_rect(tbl_rect, Color::rgb(15, 23, 42));
            ui::draw::draw_line(s, client.x + tbl_w, body_y, client.x + tbl_w, client.bottom(), Color::rgb(51, 65, 85));

            ui::font::draw_text_font(s, client.x + 8, body_y + 6, "Tables (2)", Color::rgb(148, 163, 184), &font_sm);
            let tables = [("Customers", 4), ("Invoices", 6)];
            for (ti, &(tname, _recs)) in tables.iter().enumerate() {
                let ty = body_y + 24 + (ti as i32) * 26;
                let is_sel = ti == 0;
                let r = Rect::new(client.x + 4, ty, tbl_w - 8, 22);
                if is_sel {
                    ui::draw::draw_rounded_rect(s, r, 3, Color::rgb(168, 0, 0));
                    ui::font::draw_text_font(s, r.x + 6, r.y + 4, tname, Color::WHITE, &font_sm);
                } else {
                    ui::draw::draw_rounded_rect(s, r, 3, Color::rgb(30, 41, 59));
                    ui::font::draw_text_font(s, r.x + 6, r.y + 4, tname, Color::rgb(203, 213, 225), &font_sm);
                }
            }

            // Right Table View & Data Grid
            let grid_x = client.x + tbl_w + 6;
            let grid_w = client.w - tbl_w - 12;

            // Column Header
            let th_rect = Rect::new(grid_x, body_y + 6, grid_w, 20);
            s.fill_rect(th_rect, Color::rgb(30, 41, 59));
            ui::draw::draw_rect_outline(s, th_rect, 1, Color::rgb(51, 65, 85));
            ui::font::draw_text_font(s, grid_x + 8, body_y + 10, "Id (Int)", Color::rgb(148, 163, 184), &font_sm);
            ui::font::draw_text_font(s, grid_x + 70, body_y + 10, "Name (VarChar)", Color::rgb(148, 163, 184), &font_sm);
            ui::font::draw_text_font(s, grid_x + 190, body_y + 10, "Email (VarChar)", Color::rgb(148, 163, 184), &font_sm);
            ui::font::draw_text_font(s, grid_x + 310, body_y + 10, "Balance (Float)", Color::rgb(148, 163, 184), &font_sm);

            // Record Rows
            let records = [
                ("1", "Acme Corporation", "admin@acme.com", "$14,500.00"),
                ("2", "Global Logistics", "ops@globallog.com", "$28,320.50"),
                ("3", "Cyber Systems", "contact@cybersys.io", "$6,850.00"),
                ("4", "Quantum Tech", "info@quantum.org", "$45,100.00"),
            ];

            for (ri, &(r_id, r_name, r_mail, r_bal)) in records.iter().enumerate() {
                let ry = body_y + 28 + (ri as i32) * 22;
                let is_cur = zenith.active_table_record == ri;
                let row_r = Rect::new(grid_x, ry, grid_w, 20);

                if is_cur {
                    s.fill_rect(row_r, Color::rgb(45, 20, 20));
                    ui::draw::draw_rect_outline(s, row_r, 1, Color::rgb(168, 0, 0));
                } else if ri % 2 == 1 {
                    s.fill_rect(row_r, Color::rgba(255, 255, 255, 4));
                }

                ui::font::draw_text_font(s, grid_x + 8, ry + 4, r_id, Color::WHITE, &font_sm);
                ui::font::draw_text_font(s, grid_x + 70, ry + 4, r_name, Color::WHITE, &font_sm);
                ui::font::draw_text_font(s, grid_x + 190, ry + 4, r_mail, Color::rgb(203, 213, 225), &font_sm);
                ui::font::draw_text_font(s, grid_x + 310, ry + 4, r_bal, Color::rgb(74, 222, 128), &font_sm);
            }

            // Bottom SQL Console
            let sql_y = client.bottom() - 44;
            let sql_rect = Rect::new(grid_x, sql_y, grid_w, 38);
            s.fill_rect(sql_rect, Color::rgb(15, 23, 42));
            ui::draw::draw_rect_outline(s, sql_rect, 1, Color::rgb(168, 0, 0));
            ui::font::draw_text_font(s, grid_x + 8, sql_y + 4, "SQL > SELECT * FROM Customers WHERE Balance > 10000 ORDER BY Id ASC;", Color::rgb(245, 158, 11), &font_sm);
            ui::font::draw_text_font(s, grid_x + 8, sql_y + 20, "[OK] 3 rows returned in 0.12ms | Primary Key index used", Color::rgb(74, 222, 128), &font_sm);
        }
        _ => {}
    }

    // 6. Modal About Dialog
    if zenith.show_about {
        s.fill_rect(client, Color::rgba(0, 0, 0, 160));

        let ab_w = 440.min(client.w - 32);
        let ab_h = 250.min(client.h - 32);
        let ab_x = client.x + (client.w - ab_w) / 2;
        let ab_y = client.y + (client.h - ab_h) / 2;
        let ab_rect = Rect::new(ab_x, ab_y, ab_w, ab_h);

        ui::draw::draw_rounded_rect(s, ab_rect, 8, Color::rgb(15, 23, 42));
        ui::draw::draw_rect_outline(s, ab_rect, 2, Color::rgb(0, 120, 212));

        // Header icon & title
        ui::draw::draw_rounded_rect(s, Rect::new(ab_x + 16, ab_y + 14, 26, 26), 4, Color::rgb(0, 120, 212));
        ui::font::draw_text_font(s, ab_x + 22, ab_y + 18, "Z", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 52, ab_y + 14, "ZENITH OFFICE SUITE", Color::WHITE, &font_reg);
        ui::font::draw_text_font(s, ab_x + 52, ab_y + 28, "v1.0.0 (Pure Rust 1.99.1)", Color::rgb(56, 189, 248), &font_sm);

        ui::draw::draw_line(s, ab_x + 14, ab_y + 48, ab_x + ab_w - 14, ab_y + 48, Color::rgb(48, 54, 61));

        ui::font::draw_text_font(s, ab_x + 16, ab_y + 58, "Synthesized Microsoft Office, LibreOffice & Google Docs", Color::rgb(226, 232, 240), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 76, "Modules: Zenith Writer, Zenith Calc, Zenith Present, Zenith Base", Color::rgb(148, 163, 184), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 98, "11 Embedded Fonts: Calibri, Times New Roman, Segoe UI, Ubuntu", Color::rgb(74, 222, 128), &font_sm);
        ui::font::draw_text_font(s, ab_x + 16, ab_y + 116, "Preinstalled Default App in NovaOS & Integrated in Aero Browser", Color::rgb(245, 158, 11), &font_sm);

        // Dismiss Button
        let btn_close = Rect::new(ab_x + (ab_w - 80) / 2, ab_y + ab_h - 36, 80, 24);
        ui::draw::draw_rounded_rect(s, btn_close, 4, Color::rgb(0, 120, 212));
        ui::font::draw_text_font(s, btn_close.x + 18, btn_close.y + 5, "Dismiss", Color::WHITE, &font_sm);
    }
}

/// Renders the in-window terminal emulator client view inside Window 0 / Console Shell.
fn draw_term_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, term: &WmTerminal, _palette: &ThemePalette) {
    let font_mono = Font::mono();
    let font_small = Font::small();

    // 1. Dark terminal canvas
    let bg_color = Color::rgb(13, 17, 23); // #0D1117
    s.fill_rect(client, bg_color);

    // 2. Terminal header / status line
    let hdr_h = 18;
    let hdr_rect = Rect::new(client.x, client.y, client.w, hdr_h);
    s.fill_rect(hdr_rect, Color::rgb(22, 27, 34)); // #161B22
    s.fill_rect(Rect::new(client.x, client.y + hdr_h - 1, client.w, 1), Color::rgb(48, 54, 61));

    ui::font::draw_text_font(s, client.x + 8, client.y + 3, "sh - 80x24 (xterm-256color) | pid 14", Color::rgb(139, 148, 158), &font_small);
    let right_status = "UTF-8";
    let st_sz = font_small.measure_str(right_status);
    ui::font::draw_text_font(s, client.right() - st_sz.w - 12, client.y + 3, right_status, Color::rgb(88, 166, 255), &font_small);

    // 3. Render terminal output lines from term.lines
    let mut y = client.y + hdr_h + 6;
    let line_h = 16;
    let pad_x = client.x + 8;
    let max_display_lines = ((client.bottom() - y - line_h - 4) / line_h).max(1) as usize;

    let start_idx = if term.line_count > max_display_lines {
        term.line_count - max_display_lines
    } else {
        0
    };

    for i in start_idx..term.line_count {
        if y + line_h >= client.bottom() - 10 {
            break;
        }
        let line = &term.lines[i];
        if let Ok(line_str) = core::str::from_utf8(&line.text[..line.len]) {
            ui::font::draw_text_font(s, pad_x, y, line_str, line.color, &font_mono);
        }
        y += line_h;
    }

    // 4. Render active prompt line and real-time typed input!
    if y + line_h < client.bottom() {
        let user_str = "admin@novaos";
        let u_sz = font_mono.measure_str(user_str);
        ui::font::draw_text_font(s, pad_x, y, user_str, Color::rgb(63, 185, 80), &font_mono);

        let colon_x = pad_x + u_sz.w;
        ui::font::draw_text_font(s, colon_x, y, ":", Color::rgb(139, 148, 158), &font_mono);

        let path_str = "~";
        let p_sz = font_mono.measure_str(path_str);
        ui::font::draw_text_font(s, colon_x + 6, y, path_str, Color::rgb(210, 153, 34), &font_mono);

        let prompt_str = "$ ";
        let pr_sz = font_mono.measure_str(prompt_str);
        let prompt_end_x = colon_x + 6 + p_sz.w;
        ui::font::draw_text_font(s, prompt_end_x, y, prompt_str, Color::rgb(240, 246, 252), &font_mono);

        let input_x = prompt_end_x + pr_sz.w;
        // User typed characters from input_buf
        if let Ok(input_str) = core::str::from_utf8(&term.input_buf[..term.input_len]) {
            ui::font::draw_text_font(s, input_x, y, input_str, Color::rgb(240, 246, 252), &font_mono);
        }

        // Compute cursor position
        let before_cursor_str = core::str::from_utf8(&term.input_buf[..term.cursor_idx]).unwrap_or("");
        let cur_offset = font_mono.measure_str(before_cursor_str).w;
        let cursor_x = input_x + cur_offset;

        // Solid cyan blinking block cursor
        s.fill_rect(Rect::new(cursor_x + 1, y + 2, 8, line_h - 4), Color::rgb(88, 166, 255));
    }

    // 5. Subtle vertical scrollbar on right
    let bar_x = client.right() - 6;
    let bar_h = client.h - hdr_h - 4;
    s.fill_rect(Rect::new(bar_x, client.y + hdr_h + 2, 4, bar_h), Color::rgba(255, 255, 255, 12));
    s.fill_rect(Rect::new(bar_x, client.y + hdr_h + 10, 4, 30.min(bar_h)), Color::rgba(255, 255, 255, 50));
}

/// Renders the graphical text editor client view inside Window 2 / Editor - Scratchpad.
fn draw_editor_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, _palette: &ThemePalette) {
    let font_mono = Font::mono();
    let font_small = Font::small();

    // 1. Fill main background
    let bg_color = Color::rgb(13, 17, 23); // #0D1117
    s.fill_rect(client, bg_color);

    // 2. Toolbar (height 24px)
    let tb_h = 24;
    let tb_rect = Rect::new(client.x, client.y, client.w, tb_h);
    s.fill_rect(tb_rect, Color::rgb(22, 27, 34)); // #161B22
    s.fill_rect(Rect::new(client.x, client.y + tb_h - 1, client.w, 1), Color::rgb(48, 54, 61));

    // Toolbar buttons: [New] [Open] [Save] [Cut] [Copy] [Paste] [Font: Mono]
    let buttons: [(&str, i32); 7] = [
        ("New", 36),
        ("Open", 42),
        ("Save", 42),
        ("Cut", 36),
        ("Copy", 42),
        ("Paste", 46),
        ("Font: Mono", 84),
    ];

    let mut bx = client.x + 6;
    for (lbl, bw) in &buttons {
        if bx + bw > client.right() - 6 {
            break;
        }
        let b_rect = Rect::new(bx, client.y + 3, *bw, tb_h - 6);
        s.fill_rect(b_rect, Color::rgb(33, 38, 45));
        s.fill_rect(Rect::new(b_rect.x, b_rect.y, b_rect.w, 1), Color::rgb(56, 68, 77));
        s.fill_rect(Rect::new(b_rect.x, b_rect.bottom() - 1, b_rect.w, 1), Color::rgb(56, 68, 77));
        s.fill_rect(Rect::new(b_rect.x, b_rect.y, 1, b_rect.h), Color::rgb(56, 68, 77));
        s.fill_rect(Rect::new(b_rect.right() - 1, b_rect.y, 1, b_rect.h), Color::rgb(56, 68, 77));

        let tx = b_rect.x + (b_rect.w - (lbl.len() as i32 * 8)) / 2;
        let ty = b_rect.y + (b_rect.h - 8) / 2;
        ui::font::draw_text_font(s, tx, ty, lbl, Color::rgb(240, 246, 252), &font_small);
        bx += bw + 6;
    }

    // 3. Status Bar at bottom
    let sb_h = 18;
    let sb_y = client.bottom() - sb_h;
    if sb_y > client.y + tb_h + 30 {
        let sb_rect = Rect::new(client.x, sb_y, client.w, sb_h);
        s.fill_rect(sb_rect, Color::rgb(22, 27, 34));
        s.fill_rect(Rect::new(client.x, sb_y, client.w, 1), Color::rgb(48, 54, 61));
        ui::font::draw_text_font(s, client.x + 8, sb_y + 3, "HELLO.TXT *", Color::rgb(240, 246, 252), &font_small);
        let coord_str = "Ln 2, Col 18 | UTF-8 | 8x16 Mono";
        let c_sz = font_small.measure_str(coord_str);
        ui::font::draw_text_font(s, client.right() - c_sz.w - 10, sb_y + 3, coord_str, Color::rgb(88, 166, 255), &font_small);
    }

    // 4. Gutter & Document Canvas
    let gutter_w = 38;
    let doc_y = client.y + tb_h;
    let doc_h = (sb_y - doc_y).max(0);

    // Gutter background & border
    s.fill_rect(Rect::new(client.x, doc_y, gutter_w, doc_h), Color::rgb(18, 22, 28));
    s.fill_rect(Rect::new(client.x + gutter_w - 1, doc_y, 1, doc_h), Color::rgb(48, 54, 61));

    let line_h = 16;
    let text_x = client.x + gutter_w + 6;

    // Line 1: // NovaOS Scratchpad Editor
    let y1 = doc_y + 6;
    if y1 + line_h < sb_y {
        ui::font::draw_text_font(s, client.x + 16, y1, "1", Color::rgb(110, 118, 129), &font_small);
        ui::font::draw_text_font(s, text_x, y1, "// NovaOS Scratchpad Editor", Color::rgb(139, 148, 158), &font_mono);
    }

    // Line 2: Active line with highlight
    let y2 = y1 + line_h;
    if y2 + line_h < sb_y {
        s.fill_rect(Rect::new(client.x + gutter_w, y2, client.w - gutter_w, line_h), Color::rgb(22, 27, 34));
        ui::font::draw_text_font(s, client.x + 16, y2, "2", Color::rgb(240, 246, 252), &font_small);

        // fn main() {
        ui::font::draw_text_font(s, text_x, y2, "fn", Color::rgb(255, 123, 114), &font_mono);
        ui::font::draw_text_font(s, text_x + 20, y2, "main() {", Color::rgb(240, 246, 252), &font_mono);

        // Cursor at column 10
        s.fill_rect(Rect::new(text_x + 88, y2 + 1, 2, line_h - 2), Color::rgb(88, 166, 255));
    }

    // Line 3: let msg = "Hello from NovaOS!";
    let y3 = y2 + line_h;
    if y3 + line_h < sb_y {
        ui::font::draw_text_font(s, client.x + 16, y3, "3", Color::rgb(110, 118, 129), &font_small);
        ui::font::draw_text_font(s, text_x + 16, y3, "let", Color::rgb(255, 123, 114), &font_mono);
        ui::font::draw_text_font(s, text_x + 44, y3, "msg = ", Color::rgb(240, 246, 252), &font_mono);
        ui::font::draw_text_font(s, text_x + 92, y3, "\"Hello from NovaOS!\";", Color::rgb(126, 231, 135), &font_mono);
    }

    // Line 4: }
    let y4 = y3 + line_h;
    if y4 + line_h < sb_y {
        ui::font::draw_text_font(s, client.x + 16, y4, "4", Color::rgb(110, 118, 129), &font_small);
        ui::font::draw_text_font(s, text_x, y4, "}", Color::rgb(240, 246, 252), &font_mono);
    }
}

/// Renders the Software Store & App Launcher client view inside Window 3 / Software Store - Packages.
fn draw_store_client_body<S: Surface + ?Sized>(s: &mut S, client: Rect, palette: &ThemePalette) {
    let font_regular = Font::regular();
    let font_small = Font::small();

    // 1. Dark background
    let bg_color = Color::rgb(13, 17, 23);
    s.fill_rect(client, bg_color);

    // 2. Tab strip
    let tb_h = 24;
    let tb_rect = Rect::new(client.x, client.y, client.w, tb_h);
    s.fill_rect(tb_rect, Color::rgb(22, 27, 34));
    s.fill_rect(Rect::new(client.x, client.y + tb_h - 1, client.w, 1), Color::rgb(48, 54, 61));

    // Tabs: [All Packages] [Installed] [Repo Server] [Security]
    let tabs: [(&str, i32, bool); 4] = [
        ("All Packages", 84, true),
        ("Installed", 68, false),
        ("Repo Server", 76, false),
        ("Security", 60, false),
    ];
    let mut bx = client.x + 8;
    for (lbl, bw, is_active) in &tabs {
        if bx + bw > client.right() - 8 { break; }
        if *is_active {
            s.fill_rect(Rect::new(bx, client.y + 2, *bw, tb_h - 4), Color::rgb(33, 38, 45));
            s.fill_rect(Rect::new(bx, client.y + tb_h - 2, *bw, 2), palette.accent_primary);
            ui::font::draw_text_font(s, bx + 6, client.y + 7, lbl, Color::WHITE, &font_small);
        } else {
            ui::font::draw_text_font(s, bx + 6, client.y + 7, lbl, palette.text_muted, &font_small);
        }
        bx += bw + 6;
    }

    // 3. Status Bar at bottom
    let sb_h = 18;
    let sb_y = client.bottom() - sb_h;
    if sb_y > client.y + tb_h + 30 {
        let sb_rect = Rect::new(client.x, sb_y, client.w, sb_h);
        s.fill_rect(sb_rect, Color::rgb(22, 27, 34));
        s.fill_rect(Rect::new(client.x, sb_y, client.w, 1), Color::rgb(48, 54, 61));
        ui::font::draw_text_font(s, client.x + 8, sb_y + 3, "6 packages available | 2 installed", palette.accent_primary, &font_small);
        let repo_str = "HTTP Repo: 8080 Active | HMAC-SHA256";
        let r_sz = font_small.measure_str(repo_str);
        ui::font::draw_text_font(s, client.right() - r_sz.w - 10, sb_y + 3, repo_str, palette.status_success, &font_small);
    }

    // 4. Package rows
    let row_y = client.y + tb_h + 6;
    let row_h = 36;
    let rows: [(&str, &str, &str, bool, Color); 3] = [
        ("Editor", "v1.0", "Tiny text editor (demo package)", true, Color::rgb(88, 166, 255)),
        ("Game", "v2.1", "Tiny game (demo package)", false, Color::rgb(245, 180, 50)),
        ("Calc", "v1.2", "Scientific calculator utility", false, Color::rgb(57, 211, 83)),
    ];

    for (i, (name, ver, desc, is_installed, col)) in rows.iter().enumerate() {
        let ry = row_y + (i as i32 * (row_h + 4));
        if ry + row_h >= sb_y { break; }

        let r_rect = Rect::new(client.x + 8, ry, client.w - 16, row_h);
        s.fill_rect(r_rect, Color::rgb(22, 27, 34));
        s.fill_rect(Rect::new(r_rect.x, r_rect.y, 1, row_h), Color::rgb(48, 54, 61));
        s.fill_rect(Rect::new(r_rect.right() - 1, r_rect.y, 1, row_h), Color::rgb(48, 54, 61));
        s.fill_rect(Rect::new(r_rect.x, r_rect.y, r_rect.w, 1), Color::rgb(48, 54, 61));
        s.fill_rect(Rect::new(r_rect.x, r_rect.bottom() - 1, r_rect.w, 1), Color::rgb(48, 54, 61));

        // Color indicator pill
        s.fill_rect(Rect::new(r_rect.x + 4, ry + 4, 3, row_h - 8), *col);

        // App Name & Version
        ui::font::draw_text_font(s, r_rect.x + 12, ry + 6, name, Color::WHITE, &font_regular);
        ui::font::draw_text_font(s, r_rect.x + 64, ry + 8, ver, palette.text_muted, &font_small);

        // Description
        ui::font::draw_text_font(s, r_rect.x + 12, ry + 22, desc, palette.text_muted, &font_small);

        // Verified Badge
        let badge_x = r_rect.right() - 160;
        if badge_x > r_rect.x + 120 {
            ui::font::draw_text_font(s, badge_x, ry + 6, "[✓] Verified", palette.status_success, &font_small);
        }

        // Action Button
        let btn_w = 46;
        let btn_h = 18;
        let btn_x = r_rect.right() - btn_w - 8;
        let btn_y = ry + (row_h - btn_h) / 2;
        let b_rect = Rect::new(btn_x, btn_y, btn_w, btn_h);

        if *is_installed {
            s.fill_rect(b_rect, palette.status_success);
            ui::font::draw_text_font(s, btn_x + 6, btn_y + 3, "Launch", Color::WHITE, &font_small);
        } else {
            s.fill_rect(b_rect, palette.accent_primary);
            ui::font::draw_text_font(s, btn_x + 6, btn_y + 3, "Install", Color::WHITE, &font_small);
        }
    }
}

fn mouse(dx: i32, dy: i32, left: bool) -> ffi::MouseState {
    ffi::MouseState { dx, dy, left_button: left, right_button: false, middle_button: false }
}

/// Verification suite checking window management logic and framebuffer surface rendering.
#[inline(never)]
pub fn run_selftest() -> i32 {
    let mut code = 0;
    let mut wm = Wm::new();

    unsafe { ffi::sys_write(b"[wm-selftest] starting\n\0".as_ptr()) };

    // Case 0: Verify shared UI design system invariants
    let ds_code = ui::run_design_system_selftest();
    if ds_code == 0 {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: shared UI design system invariants hold\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: UI design system invariants\n\0".as_ptr()) };
        code |= 32;
    }

    // Case 1: dragging window 0 by its titlebar actually moves it,
    // and moving the cursor there and pressing raises it to the front of z_order.
    let start = wm.windows[0];
    wm.cursor_x = 25;
    wm.cursor_y = 25;
    wm.apply_mouse(mouse(0, 0, true)); // press
    let raised_to_front = wm.z_order[MAX_WINDOWS - 1] == 0;
    wm.apply_mouse(mouse(10, 8, true)); // drag
    let moved = wm.windows[0].x != start.x || wm.windows[0].y != start.y;
    wm.apply_mouse(mouse(0, 0, false)); // release
    let drag_ended = wm.dragging.is_none();
    if raised_to_front && moved && drag_ended {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: titlebar drag moves a window and raises it\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: titlebar drag\n\0".as_ptr()) };
        code |= 1;
    }

    // Case 2: clicking anywhere on a different window's body raises it.
    wm.cursor_x = 65;
    wm.cursor_y = 130;
    wm.apply_mouse(mouse(0, 0, true));
    let win2_raised = wm.z_order[MAX_WINDOWS - 1] == 2;
    wm.apply_mouse(mouse(0, 0, false));
    if win2_raised {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: clicking a window's body raises it\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: click-to-focus\n\0".as_ptr()) };
        code |= 2;
    }

    // Case 3: clicking a taskbar button raises that window.
    let button_w = wm.screen_w / MAX_WINDOWS as i32;
    wm.cursor_x = 1 * button_w + 2;
    wm.cursor_y = wm.desktop_h + 2;
    wm.apply_mouse(mouse(0, 0, true));
    let win1_raised_via_taskbar = wm.z_order[MAX_WINDOWS - 1] == 1;
    wm.apply_mouse(mouse(0, 0, false));
    if win1_raised_via_taskbar {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: taskbar button click raises its window\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: taskbar focus\n\0".as_ptr()) };
        code |= 4;
    }

    // Case 4: dragging window 3's titlebar to the left screen edge snaps it.
    let w3 = wm.windows[3];
    wm.cursor_x = w3.x + 5;
    wm.cursor_y = w3.y + 3;
    wm.apply_mouse(mouse(0, 0, true));
    wm.apply_mouse(mouse(-400, 0, true));
    wm.apply_mouse(mouse(0, 0, false));
    let snapped_left = wm.windows[3].snap == SnapState::Left
        && wm.windows[3].x == 0
        && wm.windows[3].y == 0
        && wm.windows[3].w == wm.screen_w / 2
        && wm.windows[3].h == wm.desktop_h;
    if snapped_left {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: dragging to the left edge snaps the window\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: edge snap\n\0".as_ptr()) };
        code |= 8;
    }

    // Case 5: grabbing a snapped window's titlebar un-snaps it back to floating geometry.
    wm.cursor_x = 5;
    wm.cursor_y = 3;
    wm.apply_mouse(mouse(0, 0, true));
    let w3_now = wm.windows[3];
    let unsnapped = w3_now.snap == SnapState::Normal
        && w3_now.w == w3.w
        && w3_now.h == w3.h
        && w3_now.x >= 0 && w3_now.x + w3_now.w <= wm.screen_w
        && w3_now.y >= 0 && w3_now.y + w3_now.h <= wm.desktop_h;
    wm.apply_mouse(mouse(0, 0, false));
    if unsnapped {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: re-dragging a snapped window un-snaps it, fully on-screen\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: un-snap\n\0".as_ptr()) };
        code |= 16;
    }

    // Case 6 (Month 2 Compositor): Verify framebuffer surface rendering via BufferSurface32
    wm.cursor_x = wm.screen_w / 2;
    wm.cursor_y = wm.desktop_h / 2;
    static mut TEST_BUF: [u32; 1024 * 100] = [0u32; 1024 * 100];
    let test_buf: &mut [u32] = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_BUF) as *mut u32, 1024 * 100)
    };
    let mut surface = ui::draw::BufferSurface32::new(test_buf, 320, 200);
    wm.render(&mut surface);

    let palette = Theme::Dark.palette();
    let bg_color = palette.desktop_bg.to_xrgb();
    let pixel_0_0 = surface.buffer[0];
    let pixel_taskbar = surface.buffer[195 * 320 + 10];

    if pixel_0_0 == bg_color && pixel_taskbar != bg_color {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: framebuffer surface rendering (BufferSurface32)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: framebuffer surface rendering\n\0".as_ptr()) };
        code |= 64;
    }

    // Case 7 (Month 2 Compositor): Verify multi-size font rendering and antialiasing
    let f_small = Font::small();
    let f_reg = Font::regular();
    let f_large = Font::large();
    let m_small = f_small.measure_str("NovaOS");
    let m_reg = f_reg.measure_str("NovaOS");
    let m_large = f_large.measure_str("NovaOS");

    let mut aa_buf = [0u32; 16 * 16];
    let mut aa_surface = ui::draw::BufferSurface32::new(&mut aa_buf, 16, 16);
    ui::font::draw_char_font(&mut aa_surface, 4, 0, b'A', Color::WHITE, &f_reg);
    let has_aa_blend = aa_surface.buffer.iter().any(|&p| p != 0 && p != 0x00FFFFFF);

    if m_large.h > m_reg.h && m_reg.h > m_small.h && has_aa_blend {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: real multi-size font rendering and antialiasing\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: font rendering\n\0".as_ptr()) };
        code |= 128;
    }

    // Case 8 (Month 2 Compositor): Verify cursor themes and redesigned window chrome
    wm = Wm::new_for_display(1024, 768);
    wm.raise(0); // Make window 0 active/topmost

    // Reuse test_buf (500 * 70 = 35,000 u32s <= 64,000) for testing high-res chrome
    test_buf[..500 * 70].fill(0);
    let mut hr_surface = ui::draw::BufferSurface32::new(&mut test_buf[..500 * 70], 500, 70);
    wm.render(&mut hr_surface);

    // 1. Verify titlebar top highlight sheen exists on active window 0 (x=40..400, y=40..68)
    let sheen_pixel = hr_surface.buffer[40 * 500 + 100];
    let body_pixel = hr_surface.buffer[55 * 500 + 100];
    let has_sheen = sheen_pixel != body_pixel && sheen_pixel != 0;

    // 2. Verify window control buttons are rendered with distinct status colors
    // Window 0 is at x=40, w=360 (right edge at 400).
    // Close, Maximize, and Minimize buttons are within x in 330..400, y in 42..66
    let mut has_close_red = false;
    let mut has_max_green = false;
    let mut has_min_amber = false;
    for y in 42..66 {
        for x in 330..400 {
            let p = hr_surface.buffer[y * 500 + x];
            if p == palette.status_danger.to_xrgb() {
                has_close_red = true;
            } else if p == palette.status_success.to_xrgb() {
                has_max_green = true;
            } else if p == palette.status_warning.to_xrgb() {
                has_min_amber = true;
            }
        }
    }

    // 3. Verify dynamic cursor shape detection across various screen contexts
    let shape_desktop = wm.cursor_shape_at(5, 5);
    let shape_taskbar = wm.cursor_shape_at(100, 750);
    let shape_titlebar = wm.cursor_shape_at(100, 50);
    let shape_close_btn = wm.cursor_shape_at(385, 48);
    let shape_console = wm.cursor_shape_at(100, 100);

    let shapes_ok = shape_desktop == CursorShape::Arrow
        && shape_taskbar == CursorShape::Pointer
        && shape_titlebar == CursorShape::Move
        && shape_close_btn == CursorShape::Pointer
        && shape_console == CursorShape::IBeam;

    // 4. Verify cursor theme sprite rendering into BufferSurface32
    let mut cur_buf = [0u32; 16 * 16];
    let mut cur_surface = ui::draw::BufferSurface32::new(&mut cur_buf, 16, 16);
    ui::cursor::draw_cursor_sprite(&mut cur_surface, 0, 0, CursorShape::Arrow, CursorTheme::Classic);
    let cur_rendered_ok = cur_surface.buffer[0] == 0x00000000 && cur_surface.buffer[2 * 16 + 1] == 0x00FFFFFF;

    if has_sheen && has_close_red && has_max_green && has_min_amber && shapes_ok && cur_rendered_ok {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: cursor themes and redesigned window chrome\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: cursor themes and window chrome\n\0".as_ptr()) };
        code |= 256;
    }

    // Case 9 (Month 3 Core desktop apps): Persistent top menu bar and app menu dispatch
    wm = Wm::new_for_display(1024, 768);
    wm.raise(0); // Focus Window 0 (Terminal)

    // 1. Verify menubar title reflects active focused window
    let mb_term = wm.current_menubar();
    let is_terminal = mb_term.app_name == "Terminal" && mb_term.menus.len() == 4;

    wm.raise(1); // Focus Window 1 (Files)
    let mb_files = wm.current_menubar();
    let is_files = mb_files.app_name == "Files" && mb_files.menus.len() == 4;

    wm.raise(0); // Restore Window 0 (Terminal)

    // 2. Locate File menu item (menu 0)
    let font = Font::regular();
    let star_size = 12;
    let star_x = 8;
    let app_sz = font.measure_str("Terminal");
    let start_x = star_x + star_size + 8 + app_sz.w + 18;
    let file_rect = ui::menu::measure_menubar_item(&mb_term, 0, start_x, 0, wm.menubar_h, &font)
        .unwrap_or(Rect::new(0, 0, 0, 0));

    let file_hit = wm.menubar_item_at(file_rect.x + 4, file_rect.y + 4) == Some(0);
    let shape_menu = wm.cursor_shape_at(file_rect.x + 4, file_rect.y + 4);

    // 3. Click File menu item to open dropdown
    wm.cursor_x = file_rect.x + 4;
    wm.cursor_y = file_rect.y + 4;
    wm.apply_mouse(mouse(0, 0, true));
    wm.apply_mouse(mouse(0, 0, false));
    let dropdown_opened = wm.active_menu == Some(0) && wm.active_dropdown_rect().is_some();

    // 4. Locate and click "Quit Terminal" item (item 4, action 105)
    let drop_rect = wm.active_dropdown_rect().unwrap_or(Rect::new(0, 0, 0, 0));
    let quit_item_y = drop_rect.y + 4 + 22 + 22 + 6 + 22 + 10;
    let quit_hit = wm.dropdown_item_at(drop_rect.x + 10, quit_item_y) == Some(4);
    let shape_drop_item = wm.cursor_shape_at(drop_rect.x + 10, quit_item_y);

    wm.cursor_x = drop_rect.x + 10;
    wm.cursor_y = quit_item_y;
    wm.apply_mouse(mouse(0, 0, true));
    wm.apply_mouse(mouse(0, 0, false));

    let dropdown_closed = wm.active_menu.is_none();
    let menu_event = wm.take_menu_event();
    let event_ok = menu_event == Some(MenuClickEvent {
        window_id: 0,
        menu_index: 0,
        item_index: 4,
        action_id: ui::menu::ACTION_FILE_QUIT,
    });
    let event_drained = wm.take_menu_event().is_none();

    // 5. Render menubar into BufferSurface32
    test_buf[..1024 * 30].fill(0);
    let mut menu_surf = ui::draw::BufferSurface32::new(&mut test_buf[..1024 * 30], 1024, 30);
    wm.render(&mut menu_surf);

    let mb_bg = palette.surface_sunken.to_xrgb();
    let p_corner = menu_surf.buffer[0];
    let p_divider = menu_surf.buffer[23 * 1024 + 100];
    let divider_ok = p_divider == palette.border_subtle.to_xrgb();
    let bg_ok = p_corner == mb_bg;

    if is_terminal && is_files && file_hit && shape_menu == CursorShape::Pointer
        && dropdown_opened && quit_hit && shape_drop_item == CursorShape::Pointer
        && dropdown_closed && event_ok && event_drained && bg_ok && divider_ok
    {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: persistent top menu bar and app menu dispatch\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: persistent top menu bar\n\0".as_ptr()) };
        code |= 512;
    }

    // Case 10 (Month 3 Core desktop apps): Dock with app pinning and window switching
    wm = Wm::new_for_display(1024, 768);

    // 1. Verify default pinned applications
    let p_term = wm.is_app_pinned(APP_ID_TERMINAL);
    let p_files = wm.is_app_pinned(APP_ID_FILES);
    let p_edit = wm.is_app_pinned(APP_ID_EDITOR);
    let p_sys = wm.is_app_pinned(APP_ID_SYSTEM);
    let p_set = wm.is_app_pinned(APP_ID_SETTINGS);
    let initial_count_ok = wm.dock_count == 5;

    // 2. Verify window switching and minimize/restore toggle
    // Initially Window 3 (System) is active (topmost in z_order)
    let init_active = wm.active_window() == Some(3);
    // Clicking active Window 3 item in dock minimizes it!
    wm.on_dock_item_click(3);
    let win3_minimized = wm.windows[3].is_minimized;
    let min_event_ok = wm.take_dock_event() == Some(DockEvent::Minimize { window_id: 3 });

    // Clicking minimized Window 3 item in dock restores and raises it!
    wm.on_dock_item_click(3);
    let win3_restored = !wm.windows[3].is_minimized && wm.active_window() == Some(3);
    let res_event_ok = wm.take_dock_event() == Some(DockEvent::Restore { window_id: 3 });

    // 3. Focus switching between open windows
    // Clicking Window 1 (Files) raises it
    wm.on_dock_item_click(1);
    let win1_focused = wm.active_window() == Some(1);
    let switch_event_ok = wm.take_dock_event() == Some(DockEvent::Switch { window_id: 1 });

    // 4. Launch-on-click for pinned unlaunched app (Settings, App 5)
    let set_init_running = wm.dock_items()[4].unwrap().is_running;
    wm.on_dock_item_click(4);
    let launch_event_ok = wm.take_dock_event() == Some(DockEvent::Launch {
        app_id: APP_ID_SETTINGS,
        exec_path: "SETTINGS.ELF",
    });

    // 5. Window close and re-launch
    // Close Window 2 (Editor)
    wm.close_window(2);
    let win2_closed = !wm.windows[2].is_open;
    let edit_pinned_after_close = wm.is_app_pinned(APP_ID_EDITOR);
    let edit_not_running = !wm.dock_items()[2].unwrap().is_running;

    // Launch Editor by clicking its dock item
    wm.on_dock_item_click(2);
    let win2_reopened = wm.windows[2].is_open && wm.active_window() == Some(2);
    let relaunch_event_ok = wm.take_dock_event() == Some(DockEvent::Launch {
        app_id: APP_ID_EDITOR,
        exec_path: "EDIT.ELF",
    });

    // 6. Dynamic Pinning and Unpinning
    let custom_app = DockApp::new(6, "Calc", "CALC.ELF", true);
    let pin_ok = wm.pin_app(custom_app) && wm.is_app_pinned(6) && wm.dock_count == 6;
    let unpin_ok = wm.unpin_app(6) && !wm.is_app_pinned(6) && wm.dock_count == 5;

    // 7. Dock surface rasterization
    test_buf[..1024 * 40].fill(0);
    let mut dock_surf = ui::draw::BufferSurface32::new(&mut test_buf[..1024 * 40], 1024, 40);
    wm.render(&mut dock_surf);
    let dock_non_zero = dock_surf.buffer.iter().filter(|&&p| p != 0).count();
    let render_ok = dock_non_zero > 100;

    if p_term && p_files && p_edit && p_sys && p_set && initial_count_ok
        && init_active && win3_minimized && min_event_ok && win3_restored && res_event_ok
        && win1_focused && switch_event_ok && !set_init_running && launch_event_ok
        && win2_closed && edit_pinned_after_close && edit_not_running && win2_reopened
        && relaunch_event_ok && pin_ok && unpin_ok && render_ok
    {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: dock with app pinning and window switching\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: dock with app pinning and window switching\n\0".as_ptr()) };
        code |= 1024;
    }

    // Case 11 (Month 5 Power-user & system apps): Accessibility basics - font scaling and high-contrast mode
    wm = Wm::new_for_display(1024, 768);

    // 1. Verify accessibility configuration application
    let mut acc_cfg = AccessibilityConfig::high_contrast();
    acc_cfg.font_scale = FontScale::Large; // 150%
    wm.apply_accessibility_config(acc_cfg);

    // 2. Geometry invariants with scaled font
    let titlebar_scaled_ok = wm.titlebar_h >= 32;
    let taskbar_scaled_ok = wm.taskbar_h >= 48;
    let menubar_scaled_ok = wm.menubar_h >= 30;

    // 3. High-contrast palette invariants (WCAG AAA >= 7.0:1)
    let pal_hc = wm.access_cfg.effective_palette();
    let is_hc_theme = wm.access_cfg.theme.is_high_contrast();
    let text_cr = ui::color::contrast_ratio(pal_hc.text_primary, pal_hc.window_bg);
    let accent_cr = ui::color::contrast_ratio(pal_hc.accent_primary, pal_hc.desktop_bg);
    let contrast_ok = is_hc_theme && text_cr >= 70 && accent_cr >= 70 && pal_hc.desktop_bg == Color::BLACK;

    // 4. Compositor rendering with high-contrast theme and font scaling
    wm.raise(0); // Window 0 active
    test_buf[..1024 * 90].fill(0);
    let mut acc_surf = ui::draw::BufferSurface32::new(&mut test_buf[..1024 * 90], 1024, 90);
    wm.render(&mut acc_surf);

    let desktop_pixel = acc_surf.buffer[37 * 1024 + 5];
    let is_black_desktop = desktop_pixel == pal_hc.desktop_bg.to_xrgb();

    // Active window 0 is at x=40, y=40, w=360, titlebar_h >= 32
    let mut found_white_titlebar = false;
    let mut found_yellow_border = false;
    for y in 40..wm.titlebar_h + 40 {
        for x in 40..400 {
            let p = acc_surf.buffer[(y * 1024 + x) as usize];
            if p == 0x00FFFFFF {
                found_white_titlebar = true;
            } else if p == 0x00FFFF00 {
                found_yellow_border = true;
            }
        }
    }

    // 5. High-contrast cursor theme
    let cur_theme = wm.access_cfg.effective_cursor_theme();
    let cur_hc_ok = cur_theme == CursorTheme::HighContrast;

    // 6. Serialization roundtrip for ACCESS.CFG
    let mut cfg_bytes = [0u8; ACCESS_CFG_SIZE];
    acc_cfg.serialize(&mut cfg_bytes);
    let deserialized_cfg = AccessibilityConfig::deserialize(&cfg_bytes);
    let serialization_ok = deserialized_cfg == Some(acc_cfg);

    if titlebar_scaled_ok && taskbar_scaled_ok && menubar_scaled_ok && contrast_ok
        && is_black_desktop && found_white_titlebar && found_yellow_border && cur_hc_ok && serialization_ok
    {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: accessibility high-contrast mode and font scaling\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: accessibility high-contrast mode and font scaling\n\0".as_ptr()) };
        code |= 2048;
    }

    // Case 12 (Month 5 Power-user & system apps): Multi-window productivity polish - refining window snapping
    let case12_code = selftest_case_12_snapping_polish(&mut wm, test_buf);
    code |= case12_code;

    if code == 0 {
        unsafe { ffi::sys_write(b"[wm-selftest] all cases passed\n\0".as_ptr()) };
    }
    code
}

#[inline(never)]
fn selftest_case_12_snapping_polish(wm: &mut Wm, test_buf: &mut [u32]) -> i32 {
    *wm = Wm::new_for_display(1024, 768);
    let work = wm.work_area();
    let work_ok = work.x == 0 && work.y == 24 && work.w == 1024 && work.h == 708;

    // 1. Quadrant tiling: snap windows into all 4 quadrants
    wm.snap_window(0, SnapState::TopLeft);
    wm.snap_window(1, SnapState::TopRight);
    wm.snap_window(2, SnapState::BottomLeft);
    wm.snap_window(3, SnapState::BottomRight);

    let quad0_ok = wm.windows[0].x == 0 && wm.windows[0].y == 24
        && wm.windows[0].w == 512 && wm.windows[0].h == 354;
    let quad1_ok = wm.windows[1].x == 512 && wm.windows[1].y == 24
        && wm.windows[1].w == 512 && wm.windows[1].h == 354;
    let quad2_ok = wm.windows[2].x == 0 && wm.windows[2].y == 378
        && wm.windows[2].w == 512 && wm.windows[2].h == 354;
    let quad3_ok = wm.windows[3].x == 512 && wm.windows[3].y == 378
        && wm.windows[3].w == 512 && wm.windows[3].h == 354;

    // Smoother edges invariant: all 4 quadrants combined cover the entire work area with 0 gap and 0 overlap
    let gapless_w = wm.windows[0].w + wm.windows[1].w == work.w;
    let gapless_h = wm.windows[0].h + wm.windows[2].h == work.h;
    let quads_ok = quad0_ok && quad1_ok && quad2_ok && quad3_ok && gapless_w && gapless_h;

    // 2. Maximize toggle and restoration
    let orig_geom = wm.windows[0].pre_snap;
    wm.toggle_maximize(0);
    let max_ok = wm.windows[0].snap == SnapState::Maximize
        && wm.windows[0].x == 0 && wm.windows[0].y == 24
        && wm.windows[0].w == 1024 && wm.windows[0].h == 708;

    wm.toggle_maximize(0);
    let restore_ok = wm.windows[0].snap == SnapState::Normal
        && (wm.windows[0].x, wm.windows[0].y, wm.windows[0].w, wm.windows[0].h) == orig_geom;

    // 3. Automated side-by-side split tiling
    wm.tile_windows(TileMode::SideBySide);
    let top_win = wm.z_order[MAX_WINDOWS - 1];
    let second_win = wm.z_order[MAX_WINDOWS - 2];
    let sbs_ok = wm.windows[top_win].snap == SnapState::Left
        && wm.windows[second_win].snap == SnapState::Right;

    // 4. Snap preview ghost outline rendering on BufferSurface32
    wm.preview_snap = Some(SnapState::TopRight);
    test_buf[..1024 * 50].fill(0);
    let mut snap_surf = ui::draw::BufferSurface32::new(&mut test_buf[..1024 * 50], 1024, 50);
    wm.render(&mut snap_surf);
    // Preview outline for TopRight is at x=512+3..1024-3, y=24+3..50
    let preview_pixel = snap_surf.buffer[27 * 1024 + 520];
    let palette = Theme::Dark.palette();
    let preview_rendered = preview_pixel != 0 && preview_pixel != palette.desktop_bg.to_xrgb();

    // 5. Proportional un-snapping
    wm.raise(0);
    wm.snap_window(0, SnapState::Left);
    wm.cursor_x = 256; // 50% across the 512px snapped window
    wm.cursor_y = 35;
    wm.on_left_press(256, 35);
    let was_dragging = wm.dragging == Some(0);
    let unsnap_proportional_ok = wm.windows[0].snap == SnapState::Normal
        && (wm.cursor_x - wm.windows[0].x - wm.windows[0].w / 2).abs() <= 5
        && was_dragging;
    wm.apply_mouse(mouse(0, 0, false)); // release drag

    if work_ok && quads_ok && max_ok && restore_ok && sbs_ok && preview_rendered && unsnap_proportional_ok {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: multi-window productivity polish and window snapping\n\0".as_ptr()) };
        0
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: multi-window productivity polish and window snapping\n\0".as_ptr()) };
        4096
    }
}

fn load_access_cfg() -> Option<AccessibilityConfig> {
    let mut buf = [0u8; ACCESS_CFG_SIZE];
    let fd = unsafe { ffi::sys_open(b"/ACCESS.CFG\0".as_ptr()) };
    if fd < 0 {
        return None;
    }
    let n = unsafe { ffi::sys_read(fd, buf.as_mut_ptr(), ACCESS_CFG_SIZE as i32) };
    unsafe { ffi::sys_close(fd) };
    if n as usize >= ACCESS_CFG_SIZE {
        AccessibilityConfig::deserialize(&buf)
    } else {
        None
    }
}

fn starts_with_cstr(ptr: *const u8, prefix: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for (i, &want) in prefix.iter().enumerate() {
        let got = unsafe { *ptr.add(i) };
        if got == 0 || got != want {
            return false;
        }
    }
    true
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    if argc >= 2 {
        let ptr = unsafe { *argv.offset(1) };
        if starts_with_cstr(ptr, b"--selftest") {
            return run_selftest();
        }
    }

    unsafe {
        let msg = b"wm: entering framebuffer window manager (Month 2 Compositor)\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    // 1. Query display info
    let mut fb_info = ffi::NovaFbInfo {
        struct_size: core::mem::size_of::<ffi::NovaFbInfo>() as u32,
        ..Default::default()
    };
    let info_rc = unsafe { ffi::sys_fb_info(&mut fb_info) };

    let (screen_w, screen_h) = if info_rc == 0 && fb_info.width > 0 && fb_info.height > 0 {
        (fb_info.width as i32, fb_info.height as i32)
    } else {
        (FB_SCREEN_W, FB_SCREEN_H)
    };

    // 2. Take exclusive ownership of the display
    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"wm: failed to acquire framebuffer display (-EBUSY or -ENODEV)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    // 3. Create a 32-bit XRGB framebuffer surface
    let mut create_req = ffi::NovaFbCreate {
        width: screen_w as u32,
        height: screen_h as u32,
        flags: 0,
        handle: 0,
        pixels: 0,
        stride: 0,
        size: 0,
    };
    let create_rc = unsafe { ffi::sys_fb_create(&mut create_req) };
    if create_rc < 0 || create_req.pixels == 0 {
        unsafe {
            ffi::sys_fb_release();
            let msg = b"wm: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let mut wm = Wm::new_for_display(screen_w, screen_h);
    if let Some(cfg) = load_access_cfg() {
        wm.apply_accessibility_config(cfg);
    }

    // In production desktop runtime, boot into a clean, spacious desktop:
    // Only a centered, welcoming Terminal console window is open.
    // Core applications (Files, Scratchpad, App Store) are neatly docked
    // in the taskbar and desktop icons for instant 1-click launch/restore.
    wm.windows[1].is_minimized = true;
    wm.windows[2].is_minimized = true;
    wm.windows[3].is_minimized = true;
    wm.windows[0].is_open = true;
    wm.windows[0].is_minimized = false;
    if screen_w >= 800 && screen_h >= 600 {
        wm.windows[0].w = 560;
        wm.windows[0].h = 380;
        wm.windows[0].x = (screen_w - 560) / 2;
        wm.windows[0].y = (screen_h - 380) / 2 - 12;
        wm.windows[0].pre_snap = (wm.windows[0].x, wm.windows[0].y, wm.windows[0].w, wm.windows[0].h);
    }
    wm.raise(0);
    wm.sync_rtc();

    // Render and present the initial frame immediately so the screen is never left black
    {
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = ui::draw::BufferSurface32::new_with_pitch(pixel_slice, screen_w, screen_h, screen_w);
        wm.render(&mut surface);
        let present_req = ffi::NovaFbPresent {
            handle: fb_handle,
            src_x: 0,
            src_y: 0,
            width: screen_w,
            height: screen_h,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe { ffi::sys_fb_present(&present_req) };
    }

    let mut frame_count: u32 = 0;
    loop {
        frame_count = frame_count.wrapping_add(1);
        if frame_count % 30 == 0 {
            wm.sync_rtc();
        }

        let key = unsafe { ffi::sys_read_key() };
        if key == 27 {
            break;
        }
        if key > 0 {
            wm.on_key(key);
        }

        let mut m = ffi::MouseState { dx: 0, dy: 0, left_button: false, right_button: false, middle_button: false };
        unsafe { ffi::sys_mouse_read(&mut m) };
        wm.apply_mouse(m);

        // Render frame to surface buffer in userland memory (no syscalls during rendering)
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = ui::draw::BufferSurface32::new_with_pitch(pixel_slice, screen_w, screen_h, screen_w);
        wm.render(&mut surface);

        // Present frame with a single syscall
        let present_req = ffi::NovaFbPresent {
            handle: fb_handle,
            src_x: 0,
            src_y: 0,
            width: screen_w,
            height: screen_h,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe { ffi::sys_fb_present(&present_req) };
        unsafe { ffi::sys_yield() };
    }

    // 4. Clean up display and surface
    unsafe {
        ffi::sys_fb_destroy(fb_handle);
        ffi::sys_fb_release();
        let msg = b"wm: window manager exited, display released.\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wm_selftest_cases() {
        assert_eq!(run_selftest(), 0);
    }

    #[test]
    fn test_wm_high_res_construction() {
        let wm = Wm::new_for_display(1024, 768);
        assert_eq!(wm.screen_w, 1024);
        assert_eq!(wm.screen_h, 768);
        assert_eq!(wm.taskbar_h, 36);
        assert_eq!(wm.titlebar_h, 28);
        assert_eq!(wm.desktop_h, 732);
        assert_eq!(wm.cursor_x, 512);
        assert_eq!(wm.cursor_y, 384);
    }

    #[test]
    fn test_wm_snapping_proportions() {
        let mut wm = Wm::new_for_display(1024, 768);
        // Drag window 0 to left edge
        wm.cursor_x = 50;
        wm.cursor_y = 50;
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(-1000, 0, true));
        wm.apply_mouse(mouse(0, 0, false));

        assert_eq!(wm.windows[0].snap, SnapState::Left);
        assert_eq!(wm.windows[0].x, 0);
        assert_eq!(wm.windows[0].y, 24);
        assert_eq!(wm.windows[0].w, 512);
        assert_eq!(wm.windows[0].h, 708);
    }

    #[test]
    fn test_wm_legacy_snapping_proportions() {
        let mut wm = Wm::new();
        wm.cursor_x = 25;
        wm.cursor_y = 25;
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(-1000, 0, true));
        wm.apply_mouse(mouse(0, 0, false));

        assert_eq!(wm.windows[0].snap, SnapState::Left);
        assert_eq!(wm.windows[0].x, 0);
        assert_eq!(wm.windows[0].y, 0);
        assert_eq!(wm.windows[0].w, 160);
        assert_eq!(wm.windows[0].h, 188);
    }

    #[test]
    fn test_wm_quadrant_snapping() {
        let mut wm = Wm::new_for_display(1024, 768);
        let work = wm.work_area();
        assert_eq!(work.y, 24);
        assert_eq!(work.h, 708);

        wm.snap_window(0, SnapState::TopLeft);
        assert_eq!(wm.windows[0].x, 0);
        assert_eq!(wm.windows[0].y, 24);
        assert_eq!(wm.windows[0].w, 512);
        assert_eq!(wm.windows[0].h, 354);

        wm.snap_window(1, SnapState::TopRight);
        assert_eq!(wm.windows[1].x, 512);
        assert_eq!(wm.windows[1].y, 24);
        assert_eq!(wm.windows[1].w, 512);
        assert_eq!(wm.windows[1].h, 354);

        wm.snap_window(2, SnapState::BottomLeft);
        assert_eq!(wm.windows[2].x, 0);
        assert_eq!(wm.windows[2].y, 378);
        assert_eq!(wm.windows[2].w, 512);
        assert_eq!(wm.windows[2].h, 354);

        wm.snap_window(3, SnapState::BottomRight);
        assert_eq!(wm.windows[3].x, 512);
        assert_eq!(wm.windows[3].y, 378);
        assert_eq!(wm.windows[3].w, 512);
        assert_eq!(wm.windows[3].h, 354);
    }

    #[test]
    fn test_wm_maximize_toggle() {
        let mut wm = Wm::new_for_display(1024, 768);
        let orig = (wm.windows[0].x, wm.windows[0].y, wm.windows[0].w, wm.windows[0].h);
        wm.toggle_maximize(0);
        assert_eq!(wm.windows[0].snap, SnapState::Maximize);
        assert_eq!(wm.windows[0].x, 0);
        assert_eq!(wm.windows[0].y, 24);
        assert_eq!(wm.windows[0].w, 1024);
        assert_eq!(wm.windows[0].h, 708);

        wm.toggle_maximize(0);
        assert_eq!(wm.windows[0].snap, SnapState::Normal);
        assert_eq!((wm.windows[0].x, wm.windows[0].y, wm.windows[0].w, wm.windows[0].h), orig);
    }

    #[test]
    fn test_wm_tile_side_by_side() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.tile_windows(TileMode::SideBySide);
        let top = wm.z_order[MAX_WINDOWS - 1];
        let second = wm.z_order[MAX_WINDOWS - 2];
        assert_eq!(wm.windows[top].snap, SnapState::Left);
        assert_eq!(wm.windows[second].snap, SnapState::Right);
        assert_eq!(wm.windows[top].w + wm.windows[second].w, 1024);
        assert_eq!(wm.windows[top].h, 708);
        assert_eq!(wm.windows[second].h, 708);
    }

    #[test]
    fn test_wm_tile_grid2x2() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.tile_windows(TileMode::Grid2x2);
        let w_top = wm.z_order[3];
        let w_2nd = wm.z_order[2];
        let w_3rd = wm.z_order[1];
        let w_4th = wm.z_order[0];
        assert_eq!(wm.windows[w_top].snap, SnapState::TopLeft);
        assert_eq!(wm.windows[w_2nd].snap, SnapState::TopRight);
        assert_eq!(wm.windows[w_3rd].snap, SnapState::BottomLeft);
        assert_eq!(wm.windows[w_4th].snap, SnapState::BottomRight);
    }

    #[test]
    fn test_wm_proportional_unsnap() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0);
        wm.snap_window(0, SnapState::Left);
        assert_eq!(wm.windows[0].w, 512);

        // Click at 50% across snapped window
        wm.cursor_x = 256;
        wm.cursor_y = 35;
        wm.on_left_press(256, 35);

        assert_eq!(wm.windows[0].snap, SnapState::Normal);
        // Restored width is 360, center should be near cursor_x (256)
        let restored_center = wm.windows[0].x + wm.windows[0].w / 2;
        assert!((restored_center - 256).abs() <= 5);
    }

    #[test]
    fn test_wm_window_action_dispatch() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0);
        assert!(wm.handle_window_action(ui::menu::ACTION_WINDOW_SNAP_LEFT));
        assert_eq!(wm.windows[0].snap, SnapState::Left);

        assert!(wm.handle_window_action(ui::menu::ACTION_WINDOW_SNAP_RIGHT));
        assert_eq!(wm.windows[0].snap, SnapState::Right);

        assert!(wm.handle_window_action(ui::menu::ACTION_WINDOW_MAXIMIZE));
        assert_eq!(wm.windows[0].snap, SnapState::Maximize);

        assert!(wm.handle_window_action(ui::menu::ACTION_WINDOW_RESTORE));
        assert_eq!(wm.windows[0].snap, SnapState::Normal);
    }

    #[test]
    fn test_wm_font_rendering_high_res() {
        let wm = Wm::new_for_display(1024, 768);
        let mut buf = vec![0u32; 1024 * 768];
        let mut surface = ui::draw::BufferSurface32::new(&mut buf, 1024, 768);
        wm.render(&mut surface);

        // Window 1 titlebar should contain antialiased text pixels
        let title_region = Rect::new(40, 40, 360, 28);
        let mut non_zero_pixels = 0;
        let mut aa_pixels = 0;
        let bg_color = Theme::Dark.palette().titlebar_active.to_xrgb();

        for y in title_region.y..title_region.bottom() {
            for x in title_region.x..title_region.right() {
                let p = surface.buffer[(y * 1024 + x) as usize];
                if p != bg_color {
                    non_zero_pixels += 1;
                    if p != 0x00FFFFFF && p != 0 {
                        aa_pixels += 1;
                    }
                }
            }
        }

        assert!(non_zero_pixels > 50, "Window titlebar should contain rendered text pixels");
        assert!(aa_pixels > 0, "Window titlebar should contain antialiased blended pixels");
    }

    #[test]
    fn test_wm_cursor_shapes() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0); // Window 0 is at (40, 40, 360, 250)

        // Desktop corner
        assert_eq!(wm.cursor_shape_at(5, 5), CursorShape::Arrow);

        // Taskbar at bottom
        assert_eq!(wm.cursor_shape_at(200, 750), CursorShape::Pointer);

        // Window 0 titlebar
        assert_eq!(wm.cursor_shape_at(100, 50), CursorShape::Move);

        // Window 0 close button (right side of titlebar, around x=385, y=48)
        assert_eq!(wm.cursor_shape_at(385, 48), CursorShape::Pointer);

        // Window 0 console body (x=100, y=100)
        assert_eq!(wm.cursor_shape_at(100, 100), CursorShape::IBeam);

        // When dragging, cursor shape is always Move
        wm.dragging = Some(0);
        assert_eq!(wm.cursor_shape_at(5, 5), CursorShape::Move);
    }

    #[test]
    fn test_wm_window_chrome_buttons() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0);
        let mut buf = vec![0u32; 1024 * 80];
        let mut surface = ui::draw::BufferSurface32::new(&mut buf, 1024, 80);
        wm.render(&mut surface);

        let palette = Theme::Dark.palette();
        let danger_red = palette.status_danger.to_xrgb();
        let success_green = palette.status_success.to_xrgb();
        let warning_amber = palette.status_warning.to_xrgb();

        let mut found_red = false;
        let mut found_green = false;
        let mut found_amber = false;

        for y in 40..68 {
            for x in 330..400 {
                let p = surface.buffer[(y * 1024 + x) as usize];
                if p == danger_red {
                    found_red = true;
                } else if p == success_green {
                    found_green = true;
                } else if p == warning_amber {
                    found_amber = true;
                }
            }
        }

        assert!(found_red, "Active window should have a red Close button");
        assert!(found_green, "Active window should have a green Maximize button");
        assert!(found_amber, "Active window should have an amber Minimize button");
    }

    #[test]
    fn test_wm_menubar_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0); // Terminal

        assert_eq!(wm.current_menubar().app_name, "Terminal");
        assert_eq!(wm.menubar_h, 24);
        assert!(wm.active_menu.is_none());

        // Find File menu
        let font = Font::regular();
        let star_size = 12;
        let star_x = 8;
        let app_sz = font.measure_str("Terminal");
        let start_x = star_x + star_size + 8 + app_sz.w + 18;
        let file_rect = ui::menu::measure_menubar_item(&wm.current_menubar(), 0, start_x, 0, wm.menubar_h, &font).unwrap();

        // Hover over File menu
        wm.cursor_x = file_rect.x + 2;
        wm.cursor_y = file_rect.y + 2;
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.hovered_menu, Some(0));
        assert_eq!(wm.cursor_shape_at(wm.cursor_x, wm.cursor_y), CursorShape::Pointer);

        // Click to open File menu
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.active_menu, Some(0));
        assert!(wm.active_dropdown_rect().is_some());

        // Switch menu by hovering over Edit menu (menu 1)
        let edit_rect = ui::menu::measure_menubar_item(&wm.current_menubar(), 1, start_x, 0, wm.menubar_h, &font).unwrap();
        wm.cursor_x = edit_rect.x + 2;
        wm.cursor_y = edit_rect.y + 2;
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.active_menu, Some(1));

        // Switch back to File menu
        wm.cursor_x = file_rect.x + 2;
        wm.cursor_y = file_rect.y + 2;
        wm.apply_mouse(mouse(0, 0, false));
        assert_eq!(wm.active_menu, Some(0));

        // Click "Close Window" (item 3, action 104)
        let drop_rect = wm.active_dropdown_rect().unwrap();
        let close_y = drop_rect.y + 4 + 22 + 22 + 6 + 10;
        wm.cursor_x = drop_rect.x + 10;
        wm.cursor_y = close_y;
        wm.apply_mouse(mouse(0, 0, true));
        wm.apply_mouse(mouse(0, 0, false));

        assert!(wm.active_menu.is_none());
        let ev = wm.take_menu_event().expect("should emit menu event");
        assert_eq!(ev.window_id, 0);
        assert_eq!(ev.menu_index, 0);
        assert_eq!(ev.item_index, 3);
        assert_eq!(ev.action_id, ui::menu::ACTION_FILE_CLOSE);
    }

    #[test]
    fn test_wm_dock_pinning_and_switching() {
        let mut wm = Wm::new_for_display(1024, 768);
        assert_eq!(wm.dock_count, 5);
        assert!(wm.is_app_pinned(APP_ID_TERMINAL));
        assert!(wm.is_app_pinned(APP_ID_SETTINGS));

        // Window 3 (System) starts active
        assert_eq!(wm.active_window(), Some(3));

        // Click active Window 3 dock button -> minimizes
        wm.on_dock_item_click(3);
        assert!(wm.windows[3].is_minimized);
        assert_eq!(wm.take_dock_event(), Some(DockEvent::Minimize { window_id: 3 }));

        // Click again -> restores and raises
        wm.on_dock_item_click(3);
        assert!(!wm.windows[3].is_minimized);
        assert_eq!(wm.active_window(), Some(3));
        assert_eq!(wm.take_dock_event(), Some(DockEvent::Restore { window_id: 3 }));

        // Switch to Window 0 (Terminal)
        wm.on_dock_item_click(0);
        assert_eq!(wm.active_window(), Some(0));
        assert_eq!(wm.take_dock_event(), Some(DockEvent::Switch { window_id: 0 }));
    }

    #[test]
    fn test_wm_dock_launch_on_click() {
        let mut wm = Wm::new_for_display(1024, 768);

        // Click Settings (item 4, not running) -> launches Settings!
        let items = wm.dock_items();
        assert!(!items[4].unwrap().is_running);
        wm.on_dock_item_click(4);

        let ev = wm.take_dock_event().expect("should emit launch event");
        assert_eq!(ev, DockEvent::Launch { app_id: APP_ID_SETTINGS, exec_path: "SETTINGS.ELF" });

        // Close Window 1 (Files)
        wm.close_window(1);
        assert!(!wm.windows[1].is_open);
        assert!(wm.is_app_pinned(APP_ID_FILES));

        // Click Files dock item (item 1) -> launches and reopens window
        wm.on_dock_item_click(1);
        assert!(wm.windows[1].is_open);
        assert_eq!(wm.active_window(), Some(1));
    }

    #[test]
    fn test_wm_accessibility_high_contrast_and_scaling() {
        let mut wm = Wm::new_for_display(1024, 768);
        assert_eq!(wm.titlebar_h, 28);
        assert_eq!(wm.taskbar_h, 36);

        // Apply high contrast & 150% font scale
        let mut cfg = AccessibilityConfig::high_contrast();
        cfg.font_scale = FontScale::Large; // 150%
        wm.apply_accessibility_config(cfg);

        assert!(wm.titlebar_h >= 32);
        assert!(wm.taskbar_h >= 48);
        assert!(wm.menubar_h >= 30);
        assert_eq!(wm.cursor_theme, CursorTheme::HighContrast);
        assert_eq!(wm.access_cfg.theme, Theme::HighContrast);

        let mut buf = vec![0u32; 1024 * 768];
        let mut surface = ui::draw::BufferSurface32::new(&mut buf, 1024, 768);
        wm.render(&mut surface);

        // Desktop background outside menubar is pure black
        let desk_pixel = surface.buffer[((wm.menubar_h + 2) * 1024 + 2) as usize];
        assert_eq!(desk_pixel, 0);

        // Active window titlebar has white background and yellow border
        let mut found_white = false;
        let mut found_yellow = false;
        for y in 40..wm.titlebar_h + 40 {
            for x in 40..400 {
                let p = surface.buffer[(y * 1024 + x) as usize];
                if p == 0x00FFFFFF {
                    found_white = true;
                } else if p == 0x00FFFF00 {
                    found_yellow = true;
                }
            }
        }
        assert!(found_white, "Active window should have high-contrast white titlebar");
        assert!(found_yellow, "Active window should have high-contrast yellow border");
    }

    #[test]
    fn test_wm_start_menu_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);
        assert!(!wm.start_menu_open);

        // Click Windows 11 Start badge on taskbar (far left, y >= desktop_h)
        wm.on_left_press(16, 750);
        assert!(wm.start_menu_open, "Clicking start badge should open start menu");

        // Start menu bounds
        let sm_rect = wm.start_menu_rect();
        assert_eq!(sm_rect.x, 12);
        assert_eq!(sm_rect.w, 380);
        assert!(sm_rect.h >= 350);

        // Hit-test item 0 (Terminal)
        let item0_rect = wm.start_menu_item_rect(0).expect("item 0 should exist");
        assert_eq!(wm.start_menu_item_at(item0_rect.x + 5, item0_rect.y + 5), Some(0));

        // Clicking item 1 (Files) should launch/restore Files and close Start Menu
        wm.windows[1].is_minimized = true;
        let item1_rect = wm.start_menu_item_rect(1).expect("item 1 should exist");
        wm.on_left_press(item1_rect.x + 10, item1_rect.y + 10);
        assert!(!wm.start_menu_open, "Clicking app card should close start menu");
        assert!(!wm.windows[1].is_minimized, "Files window should be unminimized");

        // Verify Zenith (item 5) and Vortex (item 10) item hit-testing
        wm.start_menu_open = true;
        let item5_rect = wm.start_menu_item_rect(5).expect("Zenith item 5 should exist");
        assert_eq!(wm.start_menu_item_at(item5_rect.x + 5, item5_rect.y + 5), Some(5));
        let item10_rect = wm.start_menu_item_rect(10).expect("Vortex item 10 should exist");
        assert_eq!(wm.start_menu_item_at(item10_rect.x + 5, item10_rect.y + 5), Some(10));
        let item11_rect = wm.start_menu_item_rect(11).expect("Text Editor item 11 should exist");
        assert_eq!(wm.start_menu_item_at(item11_rect.x + 5, item11_rect.y + 5), Some(11));
        assert!(wm.start_menu_item_rect(12).is_none());

        // Clicking top menubar star badge toggles start menu
        wm.on_left_press(16, 10);
        assert!(wm.start_menu_open, "Clicking menubar star badge should toggle start menu");

        // Clicking outside start menu closes it
        wm.on_left_press(800, 400);
        assert!(!wm.start_menu_open, "Clicking outside should dismiss start menu");
    }

    #[test]
    fn test_wm_desktop_icons_and_peek_strip() {
        let mut wm = Wm::new_for_display(1024, 768);

        // Check desktop shortcuts exist
        assert_eq!(DESKTOP_SHORTCUTS.len(), 12);
        let icon0 = wm.desktop_icon_rect(0).expect("Desktop icon 0 should exist");
        assert_eq!(icon0.x, 18);
        assert_eq!(wm.desktop_icon_at(icon0.x + 10, icon0.y + 10), Some(0));

        // Minimize window 1, then click desktop icon for Files (idx 1)
        wm.windows[1].is_minimized = true;
        let icon1 = wm.desktop_icon_rect(1).expect("Files icon should exist");
        wm.on_left_press(icon1.x + 10, icon1.y + 10);
        assert!(!wm.windows[1].is_minimized, "Clicking Files desktop icon should unminimize it");

        // Windows 11 Show Desktop peek strip (far right edge)
        assert!(!wm.windows[0].is_minimized);
        wm.on_left_press(1022, 750); // far right
        // All open windows should be minimized
        assert!(wm.windows[0].is_minimized);
        // Click peek strip again to restore
        wm.on_left_press(1022, 750);
        assert!(!wm.windows[0].is_minimized);
    }

    #[test]
    fn test_wm_terminal_interactive_input() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0); // Focus Terminal (Window 0)
        assert_eq!(wm.term.input_len, 0);

        // Type 'help'
        wm.on_key(b'h' as i32);
        wm.on_key(b'e' as i32);
        wm.on_key(b'l' as i32);
        wm.on_key(b'p' as i32);
        assert_eq!(&wm.term.input_buf[..wm.term.input_len], b"help");

        // Test Backspace (8)
        wm.on_key(8);
        assert_eq!(&wm.term.input_buf[..wm.term.input_len], b"hel");

        // Type 'p' again
        wm.on_key(b'p' as i32);
        assert_eq!(&wm.term.input_buf[..wm.term.input_len], b"help");

        // Press Enter (10) to execute command
        let old_lines = wm.term.line_count;
        wm.on_key(10);
        assert_eq!(wm.term.input_len, 0, "Input buffer should be cleared after execution");
        assert!(wm.term.line_count > old_lines, "Terminal should output help reference lines");
    }

    #[test]
    fn test_wm_modern_wallpaper_render() {
        let wm = Wm::new_for_display(1024, 768);
        let mut buf = vec![0u32; 1024 * 768];
        let mut surface = ui::draw::BufferSurface32::new(&mut buf, 1024, 768);
        wm.render(&mut surface);

        // Verify desktop background contains gradient / aurora pixels (not all identical)
        let p_top = surface.buffer[35 * 1024 + 100];
        let p_mid = surface.buffer[350 * 1024 + 500];
        assert_ne!(p_top, 0);
        assert_ne!(p_mid, 0);
        assert_ne!(p_top, p_mid, "Wallpaper should be a smooth gradient / aurora bloom");
    }

    #[test]
    fn test_wm_timezone_and_date_calculation() {
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(2026));
        assert!(is_leap_year(2000));
        assert!(!is_leap_year(1900));

        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 11), 30);
        assert_eq!(days_in_month(2026, 12), 31);

        // Nov 19 2026 is Thursday (dow = 4)
        assert_eq!(day_of_week(2026, 11, 19), 4);
        // Jan 1 2026 is Thursday (dow = 4)
        assert_eq!(day_of_week(2026, 1, 1), 4);

        let base_rtc = ffi::RtcTime {
            year: 2026,
            month: 11,
            day: 19,
            hour: 14,
            minute: 45,
            second: 30,
        };

        // Asia/Dhaka (+360 min -> UTC+6:00): 14:45 + 6h = 20:45 on Nov 19
        let dt_dhaka = calculate_datetime(base_rtc, 360, 0);
        assert_eq!(dt_dhaka.year, 2026);
        assert_eq!(dt_dhaka.month, 11);
        assert_eq!(dt_dhaka.day, 19);
        assert_eq!(dt_dhaka.hour, 20);
        assert_eq!(dt_dhaka.minute, 45);
        assert_eq!(dt_dhaka.second, 30);
        assert_eq!(dt_dhaka.day_of_week, 4);

        // America/New_York (-300 min -> UTC-5:00): 14:45 - 5h = 09:45 on Nov 19
        let dt_ny = calculate_datetime(base_rtc, -300, 0);
        assert_eq!(dt_ny.hour, 9);
        assert_eq!(dt_ny.minute, 45);
        assert_eq!(dt_ny.day, 19);

        // Day rollover forward: 23:30 + 120 min = 01:30 next day (Nov 20)
        let rtc_late = ffi::RtcTime {
            year: 2026,
            month: 11,
            day: 19,
            hour: 23,
            minute: 30,
            second: 0,
        };
        let dt_rollover_fwd = calculate_datetime(rtc_late, 120, 0);
        assert_eq!(dt_rollover_fwd.day, 20);
        assert_eq!(dt_rollover_fwd.hour, 1);
        assert_eq!(dt_rollover_fwd.minute, 30);

        // Day rollover backward: 01:15 - 180 min = 22:15 previous day (Nov 18)
        let rtc_early = ffi::RtcTime {
            year: 2026,
            month: 11,
            day: 19,
            hour: 1,
            minute: 15,
            second: 0,
        };
        let dt_rollover_back = calculate_datetime(rtc_early, -180, 0);
        assert_eq!(dt_rollover_back.day, 18);
        assert_eq!(dt_rollover_back.hour, 22);
        assert_eq!(dt_rollover_back.minute, 15);
    }

    #[test]
    fn test_wm_clock_format_12h_24h() {
        let dt = DateTime {
            year: 2026,
            month: 11,
            day: 19,
            hour: 20,
            minute: 45,
            second: 30,
            day_of_week: 4,
        };

        let mut buf = [0u8; 32];
        let str_12h = format_clock(&dt, false, false, &mut buf);
        assert_eq!(str_12h, "8:45 PM");

        let mut buf2 = [0u8; 32];
        let str_24h = format_clock(&dt, true, false, &mut buf2);
        assert_eq!(str_24h, "20:45");

        let mut buf3 = [0u8; 32];
        let str_12h_sec = format_clock(&dt, false, true, &mut buf3);
        assert_eq!(str_12h_sec, "8:45:30 PM");

        let mut buf4 = [0u8; 64];
        let date_full = format_full_date(&dt, &mut buf4);
        assert_eq!(date_full, "Thursday, 19 November 2026");
    }

    #[test]
    fn test_wm_calendar_and_timezone_flyout_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.sync_rtc();
        assert!(!wm.calendar_open);

        // 1. Click taskbar clock to open calendar
        wm.on_left_press(1024 - 40, 750);
        assert!(wm.calendar_open, "Clicking taskbar clock should open calendar flyout");

        let cr = wm.calendar_rect();
        assert_eq!(cr.w, 320);

        // 2. Toggle 12H / 24H button inside calendar
        assert!(!wm.time_cfg.is_24h);
        let btn_mode = Rect::new(cr.x + cr.w - 86, cr.y + 12, 72, 20);
        wm.on_left_press(btn_mode.x + 10, btn_mode.y + 5);
        assert!(wm.time_cfg.is_24h, "Clicking mode button should toggle 24H mode");

        // 3. Click timezone next [>] button
        assert_eq!(wm.time_cfg.timezone_idx, 0); // Asia/Dhaka default
        let btn_tz_next = Rect::new(cr.x + cr.w - 38, cr.y + 64, 24, 24);
        wm.on_left_press(btn_tz_next.x + 5, btn_tz_next.y + 5);
        assert_eq!(wm.time_cfg.timezone_idx, 1, "Clicking next TZ button should switch to Asia/Kolkata");

        // 4. Click timezone prev [<] button
        let btn_tz_prev = Rect::new(cr.x + 14, cr.y + 64, 24, 24);
        wm.on_left_press(btn_tz_prev.x + 5, btn_tz_prev.y + 5);
        assert_eq!(wm.time_cfg.timezone_idx, 0, "Clicking prev TZ button should return to Asia/Dhaka");

        // 5. Click hour [+] manual button
        let ctrl_y = cr.y + 294;
        let btn_h_pls = Rect::new(cr.x + 72, ctrl_y, 20, 20);
        wm.on_left_press(btn_h_pls.x + 5, btn_h_pls.y + 5);
        assert_eq!(wm.time_cfg.manual_offset_minutes, 60, "Clicking + Hour should add 60 minutes");

        // 6. Click Sync RTC button to reset manual offset
        let btn_sync = Rect::new(cr.x + cr.w - 86, cr.y + 36, 72, 18);
        wm.on_left_press(btn_sync.x + 10, btn_sync.y + 5);
        assert_eq!(wm.time_cfg.manual_offset_minutes, 0, "Sync RTC should reset manual offset");

        // 7. Click outside calendar to dismiss
        wm.on_left_press(100, 100);
        assert!(!wm.calendar_open, "Clicking outside should dismiss calendar");
    }

    #[test]
    fn test_wm_terminal_date_command() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.raise(0);
        wm.sync_rtc();

        // 1. Run 'date'
        for b in b"date" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let last_line = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(last_line.contains("BST") || last_line.contains("Asia/Dhaka") || last_line.contains("2026"));

        // 2. Run 'date --24h'
        for b in b"date --24h" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        assert!(wm.time_cfg.is_24h, "date --24h should enable 24-hour mode");

        // 3. Run 'date --tz tokyo'
        for b in b"date --tz tokyo" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        assert_eq!(TIMEZONES[wm.time_cfg.timezone_idx].name, "Asia/Tokyo");
    }

    #[test]
    fn test_wm_calendar_app_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);
        wm.sync_rtc();

        // 1. Initial Calendar App state
        assert_eq!(wm.calendar_app.view_year, 2026);
        assert_eq!(wm.calendar_app.view_month, 11);
        assert_eq!(wm.calendar_app.selected_day, 19);
        assert_eq!(wm.calendar_app.view_mode, WmViewMode::Month);
        assert!(wm.calendar_app.event_count >= 8);

        // 2. Navigation tests
        wm.calendar_app.next_month();
        assert_eq!(wm.calendar_app.view_month, 12);
        wm.calendar_app.prev_month();
        assert_eq!(wm.calendar_app.view_month, 11);
        wm.calendar_app.go_today();
        assert_eq!(wm.calendar_app.selected_day, 19);

        // 3. View mode switching
        wm.calendar_app.view_mode = WmViewMode::Week;
        assert_eq!(wm.calendar_app.view_mode, WmViewMode::Week);
        wm.calendar_app.view_mode = WmViewMode::Day;
        assert_eq!(wm.calendar_app.view_mode, WmViewMode::Day);
        wm.calendar_app.view_mode = WmViewMode::Agenda;
        assert_eq!(wm.calendar_app.view_mode, WmViewMode::Agenda);
        wm.calendar_app.view_mode = WmViewMode::Month;

        // 4. Category filtering
        assert!(wm.calendar_app.is_category_enabled(WmCategory::Work));
        wm.calendar_app.toggle_category(WmCategory::Work);
        assert!(!wm.calendar_app.is_category_enabled(WmCategory::Work));
        wm.calendar_app.toggle_category(WmCategory::Work);
        assert!(wm.calendar_app.is_category_enabled(WmCategory::Work));

        // 5. Add sample event
        let old_count = wm.calendar_app.event_count;
        wm.calendar_app.add_sample_event();
        assert_eq!(wm.calendar_app.event_count, old_count + 1);

        // 6. Launch Calendar via Desktop Shortcut 3
        let cal_icon = wm.desktop_icon_rect(3).expect("Calendar desktop icon should exist");
        wm.on_left_press(cal_icon.x + 10, cal_icon.y + 10);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 6, "Clicking Calendar desktop icon should launch window 6");

        // 7. Interactive click inside Window 6: Switch to Week view via segmented pill
        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);
        let sb_w = 110;
        let cx = client.x + sb_w;
        let cw = client.w - sb_w;
        let seg_w = 40;
        let seg_x = cx + cw - (seg_w * 4) - 8;
        // Week is index 1
        wm.on_left_press(seg_x + seg_w + 5, client.y + 10);
        assert_eq!(wm.calendar_app.view_mode, WmViewMode::Week, "Clicking Week pill should switch view mode");

        // 8. Test Terminal 'cal' and 'cal --events'
        wm.raise(0);
        for b in b"cal" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("Today") || l1.contains("scheduled"));

        for b in b"cal --events" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("Thanksgiving") || l2.contains("NovaOS"));
    }

    #[test]
    fn test_wm_calc_app_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);

        // 1. Initial Calculator state
        assert_eq!(wm.calc_app.display_str(), "0");
        assert_eq!(wm.calc_app.mode, WmCalcMode::Standard);
        assert_eq!(wm.calc_app.history_count, 2);

        // 2. Arithmetic: 125 + 75 = 200
        wm.calc_app.input_digit(b'1');
        wm.calc_app.input_digit(b'2');
        wm.calc_app.input_digit(b'5');
        assert_eq!(wm.calc_app.display_str(), "125");
        wm.calc_app.apply_op(b'+');
        wm.calc_app.input_digit(b'7');
        wm.calc_app.input_digit(b'5');
        assert_eq!(wm.calc_app.display_str(), "75");
        wm.calc_app.equals();
        assert_eq!(wm.calc_app.display_str(), "200");

        // 3. Subtraction & Sign Toggle: 200 - 350 = -150
        wm.calc_app.apply_op(b'-');
        wm.calc_app.input_digit(b'3');
        wm.calc_app.input_digit(b'5');
        wm.calc_app.input_digit(b'0');
        wm.calc_app.equals();
        assert_eq!(wm.calc_app.display_str(), "-150");
        wm.calc_app.toggle_sign();
        assert_eq!(wm.calc_app.display_str(), "150");

        // 4. Unary operations: sqrt(144) = 12, sqr(12) = 144
        wm.calc_app.clear();
        wm.calc_app.input_digit(b'1');
        wm.calc_app.input_digit(b'4');
        wm.calc_app.input_digit(b'4');
        wm.calc_app.apply_unary("sqrt");
        assert_eq!(wm.calc_app.display_str(), "12");
        wm.calc_app.apply_unary("x^2");
        assert_eq!(wm.calc_app.display_str(), "144");

        // 5. Memory Management: Store 50, Add 25, Recall -> 75
        wm.calc_app.clear();
        wm.calc_app.input_digit(b'5');
        wm.calc_app.input_digit(b'0');
        wm.calc_app.memory_store();
        assert!(wm.calc_app.has_memory);
        wm.calc_app.input_digit(b'2');
        wm.calc_app.input_digit(b'5');
        wm.calc_app.memory_add();
        wm.calc_app.clear();
        wm.calc_app.memory_recall();
        assert_eq!(wm.calc_app.display_str(), "75");
        wm.calc_app.memory_clear();
        assert!(!wm.calc_app.has_memory);

        // 6. Programmer Mode: Hex and Dec conversions
        wm.calc_app.set_mode(WmCalcMode::Programmer);
        wm.calc_app.clear();
        wm.calc_app.prog_base = 0; // Hex
        wm.calc_app.input_digit(b'F');
        wm.calc_app.input_digit(b'F');
        assert_eq!(wm.calc_app.current_int(), 255);
        wm.calc_app.prog_base = 1; // Dec
        wm.calc_app.set_current_int(255);
        assert_eq!(wm.calc_app.display_str(), "255");

        // 7. Desktop Icon Launch: Click Desktop Icon 4 (Calculator)
        let calc_icon = wm.desktop_icon_rect(4).expect("Calculator desktop icon should exist");
        wm.on_left_press(calc_icon.x + 10, calc_icon.y + 10);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 7, "Clicking Calculator desktop icon should launch window 7");

        // 8. Interactive Click inside Window 7: Click mode pill for Scientific
        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);
        let tab_w = (client.w - 8) / 4;
        // Scientific is tab index 1
        wm.on_left_press(client.x + 4 + tab_w + 10, client.y + 10);
        assert_eq!(wm.calc_app.mode, WmCalcMode::Scientific);

        // 9. Shell terminal 'calc 25 * 4' command
        wm.raise(0);
        for b in b"calc 25 * 4" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("100"), "Terminal calc 25 * 4 should output 100, got: {}", l1);

        for b in b"calc sqrt(144)" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("12"), "Terminal calc sqrt(144) should output 12, got: {}", l2);
    }

    #[test]
    fn test_wm_aero_browser_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);

        // 1. Initial State
        assert_eq!(wm.browser_app.active_tab, 0);
        assert_eq!(wm.browser_app.url_str(), "about:newtab");
        assert!(wm.browser_app.shields_up);
        assert_eq!(wm.browser_app.trackers_blocked, 14);
        assert!(!wm.browser_app.shields_flyout_open);
        assert!(!wm.browser_app.reader_mode);
        assert!(!wm.browser_app.devtools_open);

        // 2. Tab Navigation
        wm.browser_app.switch_tab(1);
        assert_eq!(wm.browser_app.active_tab, 1);
        assert_eq!(wm.browser_app.url_str(), "about:shields");

        wm.browser_app.switch_tab(2);
        assert_eq!(wm.browser_app.active_tab, 2);
        assert_eq!(wm.browser_app.url_str(), "https://novaos.org");

        // 3. Launch via Desktop Shortcut 5 (Aero Web)
        let browser_icon = wm.desktop_icon_rect(5).expect("Aero Browser desktop shortcut should exist");
        wm.on_left_press(browser_icon.x + 10, browser_icon.y + 10);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 8, "Clicking browser desktop icon should launch window 8");

        // 4. Interactive clicks inside window 8
        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);

        // Click Tab 0 (Aero New Tab)
        wm.on_left_press(client.x + 20, client.y + 10);
        assert_eq!(wm.browser_app.active_tab, 0);

        // Click Shields button (flyout toggle)
        let shields_x = client.w - 110;
        wm.on_left_press(client.x + shields_x + 10, client.y + 42);
        assert!(wm.browser_app.shields_flyout_open, "Clicking shields icon should open flyout");

        // Toggle shields UP/DOWN inside flyout
        let flyout_w = 230;
        let flyout_x = client.w - flyout_w - 10;
        wm.on_left_press(client.x + flyout_x + flyout_w - 50, client.y + 78);
        assert!(!wm.browser_app.shields_up, "Clicking toggle pill should turn shields down");
        wm.on_left_press(client.x + flyout_x + flyout_w - 50, client.y + 78);
        assert!(wm.browser_app.shields_up, "Clicking toggle pill again should turn shields up");

        // Dismiss flyout by clicking outside it
        wm.on_left_press(client.x + 10, client.y + 10);
        assert!(!wm.browser_app.shields_flyout_open);

        // Click Reader Mode button
        let reader_x = client.w - 70;
        wm.on_left_press(client.x + reader_x + 10, client.y + 42);
        assert!(wm.browser_app.reader_mode, "Clicking reader button should toggle reader mode");

        // Click DevTools button
        let devtools_x = client.w - 36;
        wm.on_left_press(client.x + devtools_x + 10, client.y + 42);
        assert!(wm.browser_app.devtools_open, "Clicking devtools button should toggle inspector");

        // 5. Shell Terminal commands: browser and aero
        wm.raise(0);
        for b in b"browser --shields" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("Bandwidth Saved") || l1.contains("42.8 KB"), "Terminal browser --shields output: {}", l1);

        for b in b"aero https://novaos.org" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("200 OK") || l2.contains("Aero Engine"), "Terminal aero output: {}", l2);
    }

    #[test]
    fn test_wm_nova_player_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);

        // 1. Initial State
        assert!(wm.player_app.is_playing);
        assert_eq!(wm.player_app.active_track, 0);
        assert_eq!(wm.player_app.volume_pct, 100);
        assert!(!wm.player_app.is_muted);
        assert!(!wm.player_app.show_visualizer);
        assert!(!wm.player_app.show_playlist);

        // 2. Track Navigation and Controls Unit Logic
        wm.player_app.next_track();
        assert_eq!(wm.player_app.active_track, 1);
        wm.player_app.prev_track();
        assert_eq!(wm.player_app.active_track, 0);

        wm.player_app.toggle_play();
        assert!(!wm.player_app.is_playing);
        wm.player_app.toggle_play();
        assert!(wm.player_app.is_playing);

        wm.player_app.toggle_viz();
        assert!(wm.player_app.show_visualizer);
        wm.player_app.toggle_viz();
        assert!(!wm.player_app.show_visualizer);

        wm.player_app.toggle_playlist();
        assert!(wm.player_app.show_playlist);
        wm.player_app.toggle_playlist();
        assert!(!wm.player_app.show_playlist);

        // 3. Launch via Desktop Shortcut 6 (Spectra)
        assert_eq!(DESKTOP_SHORTCUTS[6].label, "Spectra");
        let player_icon = wm.desktop_icon_rect(6).expect("Spectra desktop shortcut should exist");
        wm.on_left_press(player_icon.x + 10, player_icon.y + 10);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 9, "Clicking Spectra desktop icon should launch window 9");

        // 4. Interactive clicks inside window 9
        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);

        // Click play/pause button (x: 30, y: client.h - 30)
        wm.on_left_press(client.x + 30, client.y + client.h - 30);
        assert!(!wm.player_app.is_playing, "Clicking play button should pause playback");
        wm.on_left_press(client.x + 30, client.y + client.h - 30);
        assert!(wm.player_app.is_playing, "Clicking play button again should resume playback");

        // Click Next button (x: 115, y: client.h - 30)
        wm.on_left_press(client.x + 115, client.y + client.h - 30);
        assert_eq!(wm.player_app.active_track, 1, "Clicking next button should switch track");

        // Click Prev button (x: 90, y: client.h - 30)
        wm.on_left_press(client.x + 90, client.y + client.h - 30);
        assert_eq!(wm.player_app.active_track, 0, "Clicking prev button should return to track 0");

        // Click EQ button on bottom toolbar (x: 175)
        wm.on_left_press(client.x + 175, client.y + client.h - 30);
        assert!(wm.player_app.show_visualizer, "Clicking EQ button should toggle visualizer");

        // Click Volume mute toggle (x: client.w - 95)
        wm.on_left_press(client.x + client.w - 95, client.y + client.h - 30);
        assert!(wm.player_app.is_muted, "Clicking volume label should toggle mute");

        // Click Top right PL button (x: client.w - 20, y: 15)
        wm.on_left_press(client.x + client.w - 20, client.y + 15);
        assert!(wm.player_app.show_playlist, "Clicking top PL button should toggle playlist");

        // Test VLC Menubar click: Help Menu (idx 7, x: 330, y: 35)
        wm.on_left_press(client.x + 330, client.y + 35);
        assert_eq!(wm.player_app.active_menu, Some(7), "Clicking Help menubar item should open Help dropdown");

        // Click "About Spectra..." (item 0 in Help dropdown: y in 48..68)
        wm.on_left_press(client.x + 200, client.y + 55);
        assert!(wm.player_app.show_about, "Clicking About Spectra should show about modal dialog");
        assert_eq!(wm.player_app.active_menu, None, "Dropdown menu should close after action");

        // Click anywhere to dismiss About modal dialog
        wm.on_left_press(client.x + 100, client.y + 100);
        assert!(!wm.player_app.show_about, "Clicking should dismiss About modal dialog");

        // 5. Shell Terminal commands: player, player --info, player --version
        wm.raise(0);
        for b in b"player --info" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("Equalizer") || l1.contains("32-Band"), "Terminal player --info output: {}", l1);

        for b in b"spectra --version" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("Pure Rust Engine") || l2.contains("spectra-core"), "Terminal spectra --version output: {}", l2);

        for b in b"player movie.mkv" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l3 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l3.contains("demuxed") || l3.contains("4K UHD"), "Terminal player movie.mkv output: {}", l3);
    }

    #[test]
    fn test_wm_vellum_pdf_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);

        // 1. Initial State
        assert_eq!(wm.vellum_app.page, 0);
        assert_eq!(wm.vellum_app.total_pages, 3);
        assert_eq!(wm.vellum_app.mode, 0); // Read
        assert_eq!(wm.vellum_app.zoom_pct, 100);
        assert!(wm.vellum_app.sidebar_open);
        assert!(!wm.vellum_app.night_mode);
        assert!(!wm.vellum_app.search_open);

        // 2. Unit Logic
        wm.vellum_app.next_page();
        assert_eq!(wm.vellum_app.page, 1);
        wm.vellum_app.prev_page();
        assert_eq!(wm.vellum_app.page, 0);

        wm.vellum_app.zoom_in();
        assert!(wm.vellum_app.zoom_pct > 100);
        wm.vellum_app.zoom_out();
        assert_eq!(wm.vellum_app.zoom_pct, 100);

        wm.vellum_app.set_mode(1); // Annotate
        assert_eq!(wm.vellum_app.mode, 1);
        assert_eq!(wm.vellum_app.active_tool, 2); // Highlight default
        wm.vellum_app.set_tool(5); // Pen
        assert_eq!(wm.vellum_app.active_tool, 5);
        wm.vellum_app.set_color(2); // Cyan
        assert_eq!(wm.vellum_app.active_color, 2);

        wm.vellum_app.toggle_night();
        assert!(wm.vellum_app.night_mode);
        wm.vellum_app.toggle_night();
        assert!(!wm.vellum_app.night_mode);

        wm.vellum_app.toggle_search();
        assert!(wm.vellum_app.search_open);
        wm.vellum_app.toggle_search();
        assert!(!wm.vellum_app.search_open);

        // 3. Launch via Desktop Shortcut 7 (Vellum)
        assert_eq!(DESKTOP_SHORTCUTS[7].label, "Vellum");
        let vellum_icon = wm.desktop_icon_rect(7).expect("Vellum desktop shortcut should exist");
        wm.on_left_press(vellum_icon.x + 10, vellum_icon.y + 10);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 10, "Clicking Vellum desktop icon should launch window 10");

        // 4. Interactive clicks inside window 10
        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);

        // Click Annotate tab (x: 90, y: 15)
        wm.on_left_press(client.x + 90, client.y + 15);
        assert_eq!(wm.vellum_app.mode, 1, "Clicking Annotate tab should switch to Annotate mode");

        // Click Strikeout tool on markup bar (x: 220, y: 45)
        wm.on_left_press(client.x + 220, client.y + 45);
        assert_eq!(wm.vellum_app.active_tool, 4, "Clicking Strike tool should select strikeout");

        // Click Find button on top ribbon (x: client.w - 85, y: 15)
        wm.on_left_press(client.x + client.w - 85, client.y + 15);
        assert!(wm.vellum_app.search_open, "Clicking Find button should open Evince search bar");

        // Click Next search hit (x: client.w - 45, y: 48)
        wm.on_left_press(client.x + client.w - 45, client.y + 48);
        assert_eq!(wm.vellum_app.match_idx, 2, "Clicking Next hit should advance match counter");

        // Click Night mode toggle (x: client.w - 45, y: 15)
        wm.on_left_press(client.x + client.w - 45, client.y + 15);
        assert!(wm.vellum_app.night_mode, "Clicking Night button should toggle night reading mode");

        // Click About button [?] (x: client.w - 15, y: 15)
        wm.on_left_press(client.x + client.w - 15, client.y + 15);
        assert!(wm.vellum_app.show_about, "Clicking ? button should show About Vellum modal dialog");

        // Dismiss modal dialog
        wm.on_left_press(client.x + 100, client.y + 100);
        assert!(!wm.vellum_app.show_about, "Clicking should dismiss About modal dialog");

        // 5. Terminal shell commands
        wm.raise(0);
        for b in b"vellum --info" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("Capabilities") || l1.contains("PDF Inspector") || l1.contains("612 x 792"), "Terminal vellum --info output: {}", l1);

        for b in b"vellum --version" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("Pure Rust 1.99.1") || l2.contains("vellum-core"), "Terminal vellum --version output: {}", l2);

        for b in b"pdf document.pdf" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l3 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l3.contains("PDF 1.7 Parsed") || l3.contains("Vellum"), "Terminal pdf command output: {}", l3);
    }

    #[test]
    fn test_wm_vortex_torrent_stream_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);

        // 1. Initial State
        assert_eq!(wm.vortex_app.filter, 0);
        assert_eq!(wm.vortex_app.selected_task, 0);
        assert_eq!(wm.vortex_app.inspector_tab, 0);
        assert!(!wm.vortex_app.show_about);
        assert_eq!(wm.vortex_app.download_rate_mb, 31);
        assert_eq!(wm.vortex_app.torrent_pieces_done, 962);
        assert_eq!(wm.vortex_app.torrent_pieces_total, 1300);

        // 2. Unit Logic
        wm.vortex_app.set_filter(2);
        assert_eq!(wm.vortex_app.filter, 2);
        wm.vortex_app.set_filter(0);

        wm.vortex_app.set_selected_task(1);
        assert_eq!(wm.vortex_app.selected_task, 1);

        wm.vortex_app.set_inspector_tab(2);
        assert_eq!(wm.vortex_app.inspector_tab, 2);

        wm.vortex_app.toggle_about();
        assert!(wm.vortex_app.show_about);
        wm.vortex_app.toggle_about();
        assert!(!wm.vortex_app.show_about);

        // 3. Launch via App Store / Launch API (Window 11)
        wm.launch_or_restore(11);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 11, "launch_or_restore(11) should activate Window 11");

        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);

        // 4. Interactive clicks inside Window 11
        // Click Pause button (x: client.x + 280, y: client.y + 15)
        wm.on_left_press(client.x + 280, client.y + 15);
        assert_eq!(wm.vortex_app.download_rate_mb, 0, "Clicking Pause should set rate to 0");

        // Click Resume button (x: client.x + 220, y: client.y + 15)
        wm.on_left_press(client.x + 220, client.y + 15);
        assert_eq!(wm.vortex_app.download_rate_mb, 31, "Clicking Resume should restore download rate");

        // Click + Stream button (x: client.x + 160, y: client.y + 15)
        wm.on_left_press(client.x + 160, client.y + 15);
        assert_eq!(wm.vortex_app.selected_task, 1, "Clicking + Stream should select stream task");

        // Click + Torrent button (x: client.x + 90, y: client.y + 15)
        wm.on_left_press(client.x + 90, client.y + 15);
        assert_eq!(wm.vortex_app.selected_task, 0, "Clicking + Torrent should select torrent task");

        // Click Sidebar Category (x: client.x + 30, y: client.y + 75 -> cat 1 Torrents)
        wm.on_left_press(client.x + 30, client.y + 75);
        assert_eq!(wm.vortex_app.filter, 1, "Clicking Sidebar category should change filter");

        // Click Inspector tab 1 (x: client.x + 200, y: client.y + 185)
        wm.on_left_press(client.x + 200, client.y + 185);
        assert_eq!(wm.vortex_app.inspector_tab, 1, "Clicking Inspector tab should switch tab");

        // Click About button [?] (x: client.x + client.w - 20, y: client.y + 15)
        wm.on_left_press(client.x + client.w - 20, client.y + 15);
        assert!(wm.vortex_app.show_about, "Clicking ? button should show About dialog");

        // Dismiss modal dialog
        wm.on_left_press(client.x + 100, client.y + 100);
        assert!(!wm.vortex_app.show_about, "Clicking anywhere should dismiss dialog");

        // 5. Terminal shell commands
        wm.raise(0);
        for b in b"vortex --list" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("Swarm Aggregation") || l1.contains("MB/s"), "Terminal vortex --list output: {}", l1);

        for b in b"vortex --sniff" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("Sniffer") || l2.contains("Badge"), "Terminal vortex --sniff output: {}", l2);

        for b in b"vortex --info" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l3 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l3.contains("Acceleration") || l3.contains("IDM") || l3.contains("Info Hash"), "Terminal vortex --info output: {}", l3);

        for b in b"vortex --version" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l4 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l4.contains("Pure Rust") || l4.contains("Architecture") || l4.contains("Transmission") || l4.contains("Vortex"), "Terminal vortex --version output: {}", l4);

        for b in b"torrent linux.torrent" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l5 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l5.contains("Swarm / Stream initialized") || l5.contains("Vortex"), "Terminal torrent command output: {}", l5);
    }

    #[test]
    fn test_wm_zenith_office_suite_interaction() {
        let mut wm = Wm::new_for_display(1024, 768);

        // 1. Initial State
        assert_eq!(wm.zenith_app.active_module, 0); // Writer
        assert_eq!(wm.zenith_app.ribbon_tab, 0);    // Home
        assert_eq!(wm.zenith_app.font_name(), "Calibri");
        assert_eq!(wm.zenith_app.font_size, 11);
        assert!(!wm.zenith_app.bold);
        assert!(!wm.zenith_app.italic);
        assert!(!wm.zenith_app.underline);
        assert_eq!(wm.zenith_app.align, 0);
        assert!(!wm.zenith_app.show_about);

        // 2. Formatting Toggles
        wm.zenith_app.toggle_bold();
        assert!(wm.zenith_app.bold);
        wm.zenith_app.toggle_italic();
        assert!(wm.zenith_app.italic);
        wm.zenith_app.toggle_underline();
        assert!(wm.zenith_app.underline);

        // 3. Font Switching across Microsoft & LibreOffice Families
        wm.zenith_app.cycle_font();
        assert_eq!(wm.zenith_app.font_name(), "Times New Roman");
        wm.zenith_app.cycle_font();
        assert_eq!(wm.zenith_app.font_name(), "Arial");

        // 4. Module Switching
        wm.zenith_app.set_module(1); // Calc
        assert_eq!(wm.zenith_app.active_module, 1);
        wm.zenith_app.set_module(2); // Present
        assert_eq!(wm.zenith_app.active_module, 2);
        wm.zenith_app.set_module(3); // Base
        assert_eq!(wm.zenith_app.active_module, 3);
        wm.zenith_app.set_module(0); // Back to Writer
        assert_eq!(wm.zenith_app.active_module, 0);

        // 5. Desktop Shortcut & Window Click Interactions
        assert_eq!(DESKTOP_SHORTCUTS[8].label, "Office");
        assert_eq!(DESKTOP_SHORTCUTS[8].app_id, 12);
        wm.launch_or_restore(DESKTOP_SHORTCUTS[8].app_id);
        let top_win_idx = wm.z_order[MAX_WINDOWS - 1];
        assert_eq!(wm.windows[top_win_idx].label, 12, "Clicking Office icon should launch Window 12");

        let w = &wm.windows[top_win_idx];
        let client = Rect::new(w.x, w.y + wm.titlebar_h, w.w, w.h - wm.titlebar_h);

        // Click [X] Calc button in module switcher (x: client.x + 100, y: client.y + 10)
        wm.on_left_press(client.x + 100, client.y + 10);
        assert_eq!(wm.zenith_app.active_module, 1, "Clicking [X] Calc button should switch to Calc spreadsheet");

        // Click [P] Present button in module switcher (x: client.x + 180, y: client.y + 10)
        wm.on_left_press(client.x + 180, client.y + 10);
        assert_eq!(wm.zenith_app.active_module, 2, "Clicking [P] Present should switch to Present slide studio");

        // Click [B] Base button in module switcher (x: client.x + 280, y: client.y + 10)
        wm.on_left_press(client.x + 280, client.y + 10);
        assert_eq!(wm.zenith_app.active_module, 3, "Clicking [B] Base should switch to Base database");

        // Click [W] Writer button in module switcher (x: client.x + 40, y: client.y + 10)
        wm.on_left_press(client.x + 40, client.y + 10);
        assert_eq!(wm.zenith_app.active_module, 0, "Clicking [W] Writer should switch to Writer");

        // Click About button [?] (x: client.x + client.w - 20, y: client.y + 10)
        wm.on_left_press(client.x + client.w - 20, client.y + 10);
        assert!(wm.zenith_app.show_about, "Clicking ? should show About modal dialog");

        // Dismiss modal dialog
        wm.on_left_press(client.x + 100, client.y + 100);
        assert!(!wm.zenith_app.show_about, "Clicking anywhere should dismiss dialog");

        // 6. Terminal Shell Commands
        wm.raise(0);
        for b in b"zenith --writer" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l1 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l1.contains("Headings") || l1.contains("Writer") || l1.contains("Word"), "Terminal zenith --writer output: {}", l1);

        for b in b"office --calc" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l2 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l2.contains("SUM") || l2.contains("Calc") || l2.contains("Spreadsheet"), "Terminal office --calc output: {}", l2);

        for b in b"zenith --fonts" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l3 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l3.contains("Calibri") || l3.contains("Liberation") || l3.contains("Fonts"), "Terminal zenith --fonts output: {}", l3);

        for b in b"zenith --version" {
            wm.on_key(*b as i32);
        }
        wm.on_key(10);
        let l4 = core::str::from_utf8(&wm.term.lines[wm.term.line_count - 1].text).unwrap();
        assert!(l4.contains("Pure Rust 1.99.1") || l4.contains("zenith-core") || l4.contains("Zenith"), "Terminal zenith --version output: {}", l4);
    }
}
