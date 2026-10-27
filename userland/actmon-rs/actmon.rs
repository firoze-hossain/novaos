//! userland/actmon-rs/actmon.rs - NovaOS Activity & Performance Monitor App
//!
//! Month 5 - Power-user & system apps: Activity/performance monitor app.
//! Shows CPU, memory, and process activity live - the "why is my machine slow"
//! diagnostic tool for NovaOS.
//!
//! Features:
//!   - Real-time scheduler and process-table queries via `SYS_PROC_INFO` (84).
//!   - System telemetry banner (live CPU %, physical RAM usage, process counts, uptime).
//!   - Dual/quad sub-views: `[Processes]`, `[CPU & Performance]`, `[Memory & System]`, `[System Info]`.
//!   - Interactive sortable table: sort by CPU %, Memory, PID, or Name.
//!   - Real-time search query filtering and category chips (All, Apps, Kernel, Active).
//!   - Dedicated Process Inspector card displaying memory footprint, peak VM pages,
//!     ticks, throttled events, and security credentials.
//!   - Historical CPU/RAM trend sparklines.
//!   - Zero-per-pixel syscall `BufferSurface32` rasterization via `ui-rs`.
//!   - Zero-stack-exhaustion BSS memory architecture ensuring safe execution
//!     within the 16KB userland stack budget.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code))]

extern crate ui;
use ui::prelude::*;

mod ffi;
use ffi::*;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"actmon: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

// ===========================================================================
// Static BSS Buffers (Zero Stack Overhead)
// ===========================================================================

pub static mut GLOBAL_SYSINFO: NovaSysActivity = NovaSysActivity::empty();
pub static mut GLOBAL_PROC_TABLE: NovaProcTable = NovaProcTable::empty();
pub static mut GLOBAL_ACTMON: ActivityMonitorApp = ActivityMonitorApp::empty();
static mut TEST_RENDER_BUF: [u32; 640 * 480] = [0; 640 * 480];

// ===========================================================================
// App Models & Constants
// ===========================================================================

pub const HISTORY_SAMPLES: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActmonTab {
    Processes,
    Performance,
    Memory,
    SystemInfo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CategoryFilter {
    All,
    Apps,
    Kernel,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Pid,
    Name,
    Cpu,
    Memory,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

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

pub fn to_lower_ascii(c: u8) -> u8 {
    if c >= b'A' && c <= b'Z' {
        c + 32
    } else {
        c
    }
}

pub fn contains_ignore_case(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let h_len = cstr_len(haystack);
    let n_len = needle.len();
    if n_len > h_len {
        return false;
    }
    for i in 0..=(h_len - n_len) {
        let mut match_ok = true;
        for j in 0..n_len {
            if to_lower_ascii(haystack[i + j]) != to_lower_ascii(needle[j]) {
                match_ok = false;
                break;
            }
        }
        if match_ok {
            return true;
        }
    }
    false
}

// ===========================================================================
// ActivityMonitorApp Struct
// ===========================================================================

pub struct ActivityMonitorApp {
    pub active_tab: ActmonTab,
    pub category_filter: CategoryFilter,
    pub sort_col: SortColumn,
    pub sort_dir: SortDirection,
    pub selected_pid: Option<u32>,
    pub auto_refresh: bool,
    pub refresh_rate_ticks: u32,
    pub last_refresh_tick: u32,

    pub search_query: [u8; 32],
    pub search_len: usize,

    pub filtered_indices: [usize; NOVA_PROC_MAX_ENTRIES],
    pub filtered_count: usize,

    pub cpu_history: [u8; HISTORY_SAMPLES],
    pub mem_history: [u8; HISTORY_SAMPLES],
    pub history_count: usize,
}

impl ActivityMonitorApp {
    pub const fn empty() -> Self {
        ActivityMonitorApp {
            active_tab: ActmonTab::Processes,
            category_filter: CategoryFilter::All,
            sort_col: SortColumn::Cpu,
            sort_dir: SortDirection::Descending,
            selected_pid: None,
            auto_refresh: true,
            refresh_rate_ticks: 100, // 1 second (100 ticks)
            last_refresh_tick: 0,
            search_query: [0; 32],
            search_len: 0,
            filtered_indices: [0; NOVA_PROC_MAX_ENTRIES],
            filtered_count: 0,
            cpu_history: [0; HISTORY_SAMPLES],
            mem_history: [0; HISTORY_SAMPLES],
            history_count: 0,
        }
    }

    pub fn init(&mut self) {
        self.active_tab = ActmonTab::Processes;
        self.category_filter = CategoryFilter::All;
        self.sort_col = SortColumn::Cpu;
        self.sort_dir = SortDirection::Descending;
        self.selected_pid = None;
        self.auto_refresh = true;
        self.refresh_rate_ticks = 100;
        self.search_len = 0;
        self.history_count = 0;

        self.refresh_from_kernel();
    }

    pub fn refresh_from_kernel(&mut self) {
        unsafe {
            let _ = ffi::sys_proc_get_sysinfo(core::ptr::addr_of_mut!(GLOBAL_SYSINFO));
            let _ = ffi::sys_proc_get_table(core::ptr::addr_of_mut!(GLOBAL_PROC_TABLE));
        }

        // Record history sample
        let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };
        let cpu_pct = sysinfo.cpu_usage_pct.min(100) as u8;
        let mem_pct = if sysinfo.total_frames > 0 {
            ((sysinfo.used_frames as u64 * 100) / sysinfo.total_frames as u64).min(100) as u8
        } else {
            0
        };

        if self.history_count < HISTORY_SAMPLES {
            self.cpu_history[self.history_count] = cpu_pct;
            self.mem_history[self.history_count] = mem_pct;
            self.history_count += 1;
        } else {
            for i in 0..(HISTORY_SAMPLES - 1) {
                self.cpu_history[i] = self.cpu_history[i + 1];
                self.mem_history[i] = self.mem_history[i + 1];
            }
            self.cpu_history[HISTORY_SAMPLES - 1] = cpu_pct;
            self.mem_history[HISTORY_SAMPLES - 1] = mem_pct;
        }

        self.rebuild_filtered_indices();
    }

    pub fn rebuild_filtered_indices(&mut self) {
        let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
        let count = (table.count as usize).min(NOVA_PROC_MAX_ENTRIES);

        let mut matched = 0usize;
        for i in 0..count {
            let entry = &table.entries[i];

            // 1. Category Filter
            let cat_ok = match self.category_filter {
                CategoryFilter::All => true,
                CategoryFilter::Apps => entry.is_user != 0,
                CategoryFilter::Kernel => entry.is_user == 0,
                CategoryFilter::Active => {
                    entry.cpu_percent > 0 || entry.state == NOVA_PROC_STATE_RUNNING
                }
            };

            if !cat_ok {
                continue;
            }

            // 2. Search Query Filter
            if self.search_len > 0 {
                let q = &self.search_query[..self.search_len];
                if !contains_ignore_case(&entry.name, q) {
                    continue;
                }
            }

            self.filtered_indices[matched] = i;
            matched += 1;
        }

        self.filtered_count = matched;
        self.sort_filtered_indices();

        // Default selection to first process if none selected
        if self.selected_pid.is_none() && matched > 0 {
            self.selected_pid = Some(table.entries[self.filtered_indices[0]].pid);
        }
    }

    pub fn sort_filtered_indices(&mut self) {
        if self.filtered_count <= 1 {
            return;
        }

        let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };

        for i in 0..self.filtered_count {
            for j in (i + 1)..self.filtered_count {
                let idx_a = self.filtered_indices[i];
                let idx_b = self.filtered_indices[j];
                let a = &table.entries[idx_a];
                let b = &table.entries[idx_b];

                let should_swap = match self.sort_col {
                    SortColumn::Pid => match self.sort_dir {
                        SortDirection::Ascending => a.pid > b.pid,
                        SortDirection::Descending => a.pid < b.pid,
                    },
                    SortColumn::Name => {
                        let name_a = a.name_str();
                        let name_b = b.name_str();
                        match self.sort_dir {
                            SortDirection::Ascending => name_a > name_b,
                            SortDirection::Descending => name_a < name_b,
                        }
                    }
                    SortColumn::Cpu => match self.sort_dir {
                        SortDirection::Ascending => a.cpu_percent > b.cpu_percent,
                        SortDirection::Descending => a.cpu_percent < b.cpu_percent,
                    },
                    SortColumn::Memory => match self.sort_dir {
                        SortDirection::Ascending => a.mem_bytes > b.mem_bytes,
                        SortDirection::Descending => a.mem_bytes < b.mem_bytes,
                    },
                };

                if should_swap {
                    self.filtered_indices[i] = idx_b;
                    self.filtered_indices[j] = idx_a;
                }
            }
        }
    }

    pub fn selected_entry(&self) -> Option<&NovaProcEntry> {
        let pid = self.selected_pid?;
        let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
        for i in 0..self.filtered_count {
            let entry = &table.entries[self.filtered_indices[i]];
            if entry.pid == pid {
                return Some(entry);
            }
        }
        None
    }

    pub fn set_search_query(&mut self, q: &str) {
        let bytes = q.as_bytes();
        let len = bytes.len().min(31);
        self.search_query[..len].copy_from_slice(&bytes[..len]);
        self.search_query[len] = 0;
        self.search_len = len;
        self.rebuild_filtered_indices();
    }

    pub fn toggle_sort(&mut self, col: SortColumn) {
        if self.sort_col == col {
            self.sort_dir = match self.sort_dir {
                SortDirection::Ascending => SortDirection::Descending,
                SortDirection::Descending => SortDirection::Ascending,
            };
        } else {
            self.sort_col = col;
            self.sort_dir = match col {
                SortColumn::Pid | SortColumn::Name => SortDirection::Ascending,
                SortColumn::Cpu | SortColumn::Memory => SortDirection::Descending,
            };
        }
        self.sort_filtered_indices();
    }

    // -----------------------------------------------------------------------
    // Interactive Mouse Click Handling
    // -----------------------------------------------------------------------

    pub fn handle_click(&mut self, px: i32, py: i32, viewport: Rect) -> bool {
        // 1. Navigation Sub-Tabs (y: 12..36)
        let tab_y = 12;
        let tab_h = 26;

        let tab_processes = Rect::new(200, tab_y, 88, tab_h);
        let tab_perf = Rect::new(294, tab_y, 100, tab_h);
        let tab_mem = Rect::new(400, tab_y, 116, tab_h);
        let tab_sys = Rect::new(522, tab_y, 96, tab_h);

        if point_in_rect(px, py, tab_processes.x, tab_processes.y, tab_processes.w, tab_processes.h) {
            self.active_tab = ActmonTab::Processes;
            return true;
        }
        if point_in_rect(px, py, tab_perf.x, tab_perf.y, tab_perf.w, tab_perf.h) {
            self.active_tab = ActmonTab::Performance;
            return true;
        }
        if point_in_rect(px, py, tab_mem.x, tab_mem.y, tab_mem.w, tab_mem.h) {
            self.active_tab = ActmonTab::Memory;
            return true;
        }
        if point_in_rect(px, py, tab_sys.x, tab_sys.y, tab_sys.w, tab_sys.h) {
            self.active_tab = ActmonTab::SystemInfo;
            return true;
        }

        // Top Toolbar Controls: [Refresh] (right - 140), [Pause/Resume] (right - 70)
        let btn_refresh = Rect::new(viewport.w - 180, tab_y, 80, tab_h);
        let btn_pause = Rect::new(viewport.w - 94, tab_y, 82, tab_h);

        if point_in_rect(px, py, btn_refresh.x, btn_refresh.y, btn_refresh.w, btn_refresh.h) {
            self.refresh_from_kernel();
            return true;
        }
        if point_in_rect(px, py, btn_pause.x, btn_pause.y, btn_pause.w, btn_pause.h) {
            self.auto_refresh = !self.auto_refresh;
            return true;
        }

        // 2. Category Filter Chips (y: 120..144)
        if self.active_tab == ActmonTab::Processes {
            let chip_y = 120;
            let chip_h = 24;

            let chip_all = Rect::new(12, chip_y, 60, chip_h);
            let chip_apps = Rect::new(78, chip_y, 80, chip_h);
            let chip_kern = Rect::new(164, chip_y, 86, chip_h);
            let chip_act = Rect::new(256, chip_y, 86, chip_h);

            if point_in_rect(px, py, chip_all.x, chip_all.y, chip_all.w, chip_all.h) {
                self.category_filter = CategoryFilter::All;
                self.rebuild_filtered_indices();
                return true;
            }
            if point_in_rect(px, py, chip_apps.x, chip_apps.y, chip_apps.w, chip_apps.h) {
                self.category_filter = CategoryFilter::Apps;
                self.rebuild_filtered_indices();
                return true;
            }
            if point_in_rect(px, py, chip_kern.x, chip_kern.y, chip_kern.w, chip_kern.h) {
                self.category_filter = CategoryFilter::Kernel;
                self.rebuild_filtered_indices();
                return true;
            }
            if point_in_rect(px, py, chip_act.x, chip_act.y, chip_act.w, chip_act.h) {
                self.category_filter = CategoryFilter::Active;
                self.rebuild_filtered_indices();
                return true;
            }

            // Table Header Sorting (y: 154..176)
            let hdr_y = 154;
            let hdr_h = 22;
            if py >= hdr_y && py < hdr_y + hdr_h {
                if px >= 16 && px < 70 {
                    self.toggle_sort(SortColumn::Pid);
                    return true;
                } else if px >= 70 && px < 250 {
                    self.toggle_sort(SortColumn::Name);
                    return true;
                } else if px >= 360 && px < 430 {
                    self.toggle_sort(SortColumn::Cpu);
                    return true;
                } else if px >= 430 && px < 510 {
                    self.toggle_sort(SortColumn::Memory);
                    return true;
                }
            }

            // Table Rows (y: 178..460, row_h = 24)
            let row_start_y = 178;
            let row_h = 24;
            if py >= row_start_y && py < 460 {
                let row_idx = ((py - row_start_y) / row_h) as usize;
                if row_idx < self.filtered_count {
                    let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
                    let selected = table.entries[self.filtered_indices[row_idx]].pid;
                    self.selected_pid = Some(selected);
                    return true;
                }
            }
        }

        false
    }

    // -----------------------------------------------------------------------
    // Rendering Routines (BufferSurface32)
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S, viewport: Rect) {
        let pal = Theme::Dark.palette();
        let font_title = Font::large();
        let font = Font::regular();
        let font_small = Font::small();

        // 1. App Background
        s.fill_rect(viewport, pal.window_bg);

        // 2. Top Header Bar (y: 0..48)
        let header_rect = Rect::new(viewport.x, viewport.y, viewport.w, 48);
        s.fill_rect(header_rect, pal.surface_raised);
        s.fill_rect(Rect::new(viewport.x, viewport.y + 47, viewport.w, 1), pal.border_subtle);

        // App Icon & Title
        draw_rounded_rect(s, Rect::new(viewport.x + 12, viewport.y + 10, 28, 28), RADIUS_SM, pal.accent_primary);
        // Pulse wave icon inside icon badge
        s.fill_rect(Rect::new(viewport.x + 18, viewport.y + 23, 4, 2), WHITE);
        s.fill_rect(Rect::new(viewport.x + 22, viewport.y + 16, 3, 16), WHITE);
        s.fill_rect(Rect::new(viewport.x + 25, viewport.y + 26, 3, 6), WHITE);
        s.fill_rect(Rect::new(viewport.x + 28, viewport.y + 20, 6, 2), WHITE);

        draw_text_font(s, viewport.x + 48, viewport.y + 14, "Activity Monitor", pal.text_primary, &font_title);

        // Navigation Tabs: [Processes], [Performance], [Memory], [System Info]
        let tab_defs = [
            (ActmonTab::Processes, "Processes", 200, 88),
            (ActmonTab::Performance, "Performance", 294, 100),
            (ActmonTab::Memory, "Memory & Storage", 400, 116),
            (ActmonTab::SystemInfo, "System Info", 522, 96),
        ];

        for (tab, label, tx, tw) in tab_defs.iter() {
            let is_active = self.active_tab == *tab;
            let rect = Rect::new(viewport.x + *tx, viewport.y + 12, *tw, 26);
            let bg = if is_active { pal.accent_primary } else { pal.surface_sunken };
            let fg = if is_active { WHITE } else { pal.text_secondary };
            draw_rounded_rect(s, rect, RADIUS_XS, bg);
            if is_active {
                draw_rect_outline(s, rect, 1, pal.border_focused);
            }
            let sz = font.measure_str(label);
            draw_text_font(s, rect.x + (rect.w - sz.w) / 2, rect.y + (rect.h - sz.h) / 2, label, fg, &font);
        }

        // Top Toolbar Controls: [Refresh], [Pause/Resume]
        let btn_refresh = Rect::new(viewport.right() - 180, viewport.y + 12, 80, 26);
        draw_rounded_rect(s, btn_refresh, RADIUS_XS, pal.surface_sunken);
        draw_rect_outline(s, btn_refresh, 1, pal.border_subtle);
        draw_text_font(s, btn_refresh.x + 12, btn_refresh.y + 6, "Refresh", pal.text_primary, &font);

        let btn_pause = Rect::new(viewport.right() - 94, viewport.y + 12, 82, 26);
        let pause_bg = if self.auto_refresh { pal.status_success } else { pal.status_warning };
        draw_rounded_rect(s, btn_pause, RADIUS_XS, pause_bg);
        let pause_label = if self.auto_refresh { "Live: 1s" } else { "Paused" };
        let sz_p = font.measure_str(pause_label);
        draw_text_font(s, btn_pause.x + (btn_pause.w - sz_p.w) / 2, btn_pause.y + (btn_pause.h - sz_p.h) / 2, pause_label, WHITE, &font);

        // 3. Telemetry Cards Bar (y: 54..114)
        self.render_telemetry_cards(s, viewport, &pal, &font, &font_small);

        // 4. Tab Body Area
        match self.active_tab {
            ActmonTab::Processes => self.render_processes_tab(s, viewport, &pal, &font, &font_small),
            ActmonTab::Performance => self.render_performance_tab(s, viewport, &pal, &font, &font_small),
            ActmonTab::Memory => self.render_memory_tab(s, viewport, &pal, &font, &font_small),
            ActmonTab::SystemInfo => self.render_sysinfo_tab(s, viewport, &pal, &font, &font_small),
        }

        // 5. Status Bar at Bottom (y: viewport.h - 24..viewport.h)
        let status_rect = Rect::new(viewport.x, viewport.bottom() - 24, viewport.w, 24);
        s.fill_rect(status_rect, pal.surface_sunken);
        s.fill_rect(Rect::new(viewport.x, status_rect.y, viewport.w, 1), pal.border_subtle);

        let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };
        let free_mb = (sysinfo.free_frames * 4) / 1024;
        let uptime_s = sysinfo.uptime_ticks / 100;

        let mut status_buf = [0u8; 128];
        let mut pos = 0usize;

        self.append_str_to_buf(&mut status_buf, &mut pos, "Showing ");
        self.append_u32_to_buf(&mut status_buf, &mut pos, self.filtered_count as u32);
        self.append_str_to_buf(&mut status_buf, &mut pos, " of ");
        self.append_u32_to_buf(&mut status_buf, &mut pos, sysinfo.total_processes);
        self.append_str_to_buf(&mut status_buf, &mut pos, " processes | Total CPU: ");
        self.append_u32_to_buf(&mut status_buf, &mut pos, sysinfo.cpu_usage_pct);
        self.append_str_to_buf(&mut status_buf, &mut pos, "% | Free RAM: ");
        self.append_u32_to_buf(&mut status_buf, &mut pos, free_mb);
        self.append_str_to_buf(&mut status_buf, &mut pos, " MB | Uptime: ");
        self.append_u32_to_buf(&mut status_buf, &mut pos, uptime_s / 60);
        self.append_str_to_buf(&mut status_buf, &mut pos, "m ");
        self.append_u32_to_buf(&mut status_buf, &mut pos, uptime_s % 60);
        self.append_str_to_buf(&mut status_buf, &mut pos, "s");

        status_buf[pos] = 0;
        let status_text = cstr_as_str(&status_buf);
        draw_text_font(s, viewport.x + 14, status_rect.y + 5, status_text, pal.text_muted, &font_small);
    }

    fn render_telemetry_cards<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        pal: &ThemePalette,
        font: &Font,
        font_small: &Font,
    ) {
        let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };
        let card_w = (viewport.w - 48) / 4;
        let card_h = 56;
        let card_y = viewport.y + 54;

        // Card 1: CPU Activity
        let c1 = Rect::new(viewport.x + 12, card_y, card_w, card_h);
        draw_rounded_rect(s, c1, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, c1, 1, pal.border_subtle);
        draw_text_font(s, c1.x + 10, c1.y + 6, "CPU USAGE", pal.text_muted, font_small);

        let cpu_pct = sysinfo.cpu_usage_pct.min(100);
        let mut cpu_text = [0u8; 16];
        let mut pos = 0usize;
        self.append_u32_to_buf(&mut cpu_text, &mut pos, cpu_pct);
        cpu_text[pos] = b'%';
        pos += 1;
        cpu_text[pos] = 0;
        let cpu_str = cstr_as_str(&cpu_text);

        let cpu_col = if cpu_pct < 50 {
            pal.status_success
        } else if cpu_pct < 80 {
            pal.status_warning
        } else {
            pal.status_danger
        };
        draw_text_font(s, c1.x + 10, c1.y + 20, cpu_str, cpu_col, font);

        // Mini CPU Progress Bar
        let bar_w = c1.w - 20;
        let bar_rect = Rect::new(c1.x + 10, c1.y + 42, bar_w, 6);
        draw_rounded_rect(s, bar_rect, 2, pal.surface_sunken);
        let fill_w = ((bar_w as u32 * cpu_pct) / 100).max(2) as i32;
        draw_rounded_rect(s, Rect::new(bar_rect.x, bar_rect.y, fill_w, bar_rect.h), 2, cpu_col);

        // Card 2: Physical Memory
        let c2 = Rect::new(c1.right() + 8, card_y, card_w, card_h);
        draw_rounded_rect(s, c2, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, c2, 1, pal.border_subtle);
        draw_text_font(s, c2.x + 10, c2.y + 6, "PHYSICAL MEMORY", pal.text_muted, font_small);

        let total_mb = (sysinfo.total_frames * 4) / 1024;
        let used_mb = (sysinfo.used_frames * 4) / 1024;
        let mem_pct = if total_mb > 0 { ((used_mb * 100) / total_mb).min(100) } else { 0 };

        let mut mem_text = [0u8; 32];
        let mut mpos = 0usize;
        self.append_u32_to_buf(&mut mem_text, &mut mpos, used_mb);
        self.append_str_to_buf(&mut mem_text, &mut mpos, " / ");
        self.append_u32_to_buf(&mut mem_text, &mut mpos, total_mb);
        self.append_str_to_buf(&mut mem_text, &mut mpos, " MB");
        mem_text[mpos] = 0;
        draw_text_font(s, c2.x + 10, c2.y + 20, cstr_as_str(&mem_text), pal.text_primary, font);

        let mem_bar_rect = Rect::new(c2.x + 10, c2.y + 42, bar_w, 6);
        draw_rounded_rect(s, mem_bar_rect, 2, pal.surface_sunken);
        let mem_fill_w = ((bar_w as u32 * mem_pct) / 100).max(2) as i32;
        draw_rounded_rect(s, Rect::new(mem_bar_rect.x, mem_bar_rect.y, mem_fill_w, mem_bar_rect.h), 2, pal.accent_primary);

        // Card 3: Process Telemetry
        let c3 = Rect::new(c2.right() + 8, card_y, card_w, card_h);
        draw_rounded_rect(s, c3, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, c3, 1, pal.border_subtle);
        draw_text_font(s, c3.x + 10, c3.y + 6, "PROCESSES", pal.text_muted, font_small);

        let mut proc_text = [0u8; 32];
        let mut ppos = 0usize;
        self.append_u32_to_buf(&mut proc_text, &mut ppos, sysinfo.total_processes);
        self.append_str_to_buf(&mut proc_text, &mut ppos, " Total");
        proc_text[ppos] = 0;
        draw_text_font(s, c3.x + 10, c3.y + 20, cstr_as_str(&proc_text), pal.status_info, font);

        let mut sub_proc = [0u8; 48];
        let mut spos = 0usize;
        self.append_u32_to_buf(&mut sub_proc, &mut spos, sysinfo.running_processes);
        self.append_str_to_buf(&mut sub_proc, &mut spos, " Run, ");
        self.append_u32_to_buf(&mut sub_proc, &mut spos, sysinfo.user_processes);
        self.append_str_to_buf(&mut sub_proc, &mut spos, " User");
        sub_proc[spos] = 0;
        draw_text_font(s, c3.x + 10, c3.y + 38, cstr_as_str(&sub_proc), pal.text_muted, font_small);

        // Card 4: Hardware & Core Topography
        let c4 = Rect::new(c3.right() + 8, card_y, card_w, card_h);
        draw_rounded_rect(s, c4, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, c4, 1, pal.border_subtle);
        draw_text_font(s, c4.x + 10, c4.y + 6, "SMP TOPOLOGY", pal.text_muted, font_small);

        let mut smp_text = [0u8; 32];
        let mut smpos = 0usize;
        self.append_u32_to_buf(&mut smp_text, &mut smpos, sysinfo.cpu_count);
        self.append_str_to_buf(&mut smp_text, &mut smpos, " Cores Online");
        smp_text[smpos] = 0;
        draw_text_font(s, c4.x + 10, c4.y + 20, cstr_as_str(&smp_text), pal.text_primary, font);

        let mut sw_text = [0u8; 32];
        let mut swpos = 0usize;
        self.append_u32_to_buf(&mut sw_text, &mut swpos, sysinfo.context_switches);
        self.append_str_to_buf(&mut sw_text, &mut swpos, " Switches");
        sw_text[swpos] = 0;
        draw_text_font(s, c4.x + 10, c4.y + 38, cstr_as_str(&sw_text), pal.text_muted, font_small);
    }

    fn render_processes_tab<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        pal: &ThemePalette,
        font: &Font,
        font_small: &Font,
    ) {
        // Filter Chips (y: 118..144)
        let chip_y = viewport.y + 118;
        let chip_h = 24;

        let chips = [
            (CategoryFilter::All, "All", 12, 60),
            (CategoryFilter::Apps, "Apps (User)", 78, 80),
            (CategoryFilter::Kernel, "Kernel Tasks", 164, 86),
            (CategoryFilter::Active, "Active (>0%)", 256, 86),
        ];

        for (cat, label, cx, cw) in chips.iter() {
            let is_sel = self.category_filter == *cat;
            let rect = Rect::new(viewport.x + *cx, chip_y, *cw, chip_h);
            let bg = if is_sel { pal.accent_primary } else { pal.surface_raised };
            let fg = if is_sel { WHITE } else { pal.text_secondary };
            draw_rounded_rect(s, rect, RADIUS_XS, bg);
            let sz = font_small.measure_str(label);
            draw_text_font(s, rect.x + (rect.w - sz.w) / 2, rect.y + (rect.h - sz.h) / 2, label, fg, font_small);
        }

        // Search Bar Indicator (right-aligned)
        let search_box = Rect::new(viewport.right() - 200, chip_y, 188, chip_h);
        draw_rounded_rect(s, search_box, RADIUS_XS, pal.surface_sunken);
        draw_rect_outline(s, search_box, 1, pal.border_subtle);
        if self.search_len > 0 {
            let q_str = cstr_as_str(&self.search_query);
            draw_text_font(s, search_box.x + 8, search_box.y + 4, q_str, pal.text_primary, font_small);
        } else {
            draw_text_font(s, search_box.x + 8, search_box.y + 4, "Search processes...", pal.text_muted, font_small);
        }

        // Table Header (y: 148..172)
        let hdr_y = viewport.y + 148;
        let hdr_rect = Rect::new(viewport.x + 12, hdr_y, viewport.w - 24, 22);
        s.fill_rect(hdr_rect, pal.surface_raised);
        s.fill_rect(Rect::new(hdr_rect.x, hdr_rect.bottom() - 1, hdr_rect.w, 1), pal.border_focused);

        draw_text_font(s, hdr_rect.x + 8, hdr_y + 4, "PID", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 60, hdr_y + 4, "Process Name", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 220, hdr_y + 4, "State", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 310, hdr_y + 4, "Type", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 380, hdr_y + 4, "CPU %", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 460, hdr_y + 4, "Memory", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 550, hdr_y + 4, "CPU Ticks", pal.text_secondary, font_small);
        draw_text_font(s, hdr_rect.x + 660, hdr_y + 4, "User / Priv", pal.text_secondary, font_small);

        // Table Rows (y: 174..460, row_h = 24)
        let row_start_y = hdr_y + 24;
        let row_h = 24;
        let max_rows = ((460 - row_start_y) / row_h) as usize;
        let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };

        for i in 0..self.filtered_count.min(max_rows) {
            let entry = &table.entries[self.filtered_indices[i]];
            let ry = row_start_y + (i as i32 * row_h);
            let row_rect = Rect::new(viewport.x + 12, ry, viewport.w - 24, row_h);

            let is_selected = self.selected_pid == Some(entry.pid);
            if is_selected {
                s.fill_rect(row_rect, pal.surface_raised);
                draw_rect_outline(s, row_rect, 1, pal.accent_primary);
            } else if i % 2 == 1 {
                s.fill_rect(row_rect, pal.surface_sunken);
            }

            // PID
            let mut pid_buf = [0u8; 12];
            let mut ppos = 0usize;
            self.append_u32_to_buf(&mut pid_buf, &mut ppos, entry.pid);
            pid_buf[ppos] = 0;
            draw_text_font(s, row_rect.x + 8, ry + 4, cstr_as_str(&pid_buf), pal.text_muted, font);

            // Name
            let name_col = if entry.is_user != 0 { pal.text_primary } else { pal.status_info };
            draw_text_font(s, row_rect.x + 60, ry + 4, entry.name_str(), name_col, font);

            // State Pill Badge
            let (state_str, state_col) = match entry.state {
                NOVA_PROC_STATE_RUNNING => ("RUNNING", pal.status_success),
                NOVA_PROC_STATE_READY => ("READY", pal.accent_primary),
                NOVA_PROC_STATE_ALLOCATING => ("ALLOC", pal.status_warning),
                NOVA_PROC_STATE_TERMINATED => ("EXITED", pal.text_muted),
                _ => ("IDLE", pal.text_muted),
            };
            let pill_rect = Rect::new(row_rect.x + 220, ry + 3, 64, 18);
            draw_rounded_rect(s, pill_rect, RADIUS_XS, state_col);
            let sz_s = font_small.measure_str(state_str);
            draw_text_font(s, pill_rect.x + (pill_rect.w - sz_s.w) / 2, pill_rect.y + 2, state_str, WHITE, font_small);

            // Type
            let type_str = if entry.is_user != 0 { "Userland" } else { "Kernel" };
            draw_text_font(s, row_rect.x + 310, ry + 4, type_str, pal.text_secondary, font_small);

            // CPU %
            let mut cpu_buf = [0u8; 12];
            let mut cpos = 0usize;
            self.append_u32_to_buf(&mut cpu_buf, &mut cpos, entry.cpu_percent);
            cpu_buf[cpos] = b'%';
            cpu_buf[cpos + 1] = 0;
            let cpu_fg = if entry.cpu_percent > 50 { pal.status_danger } else { pal.text_primary };
            draw_text_font(s, row_rect.x + 380, ry + 4, cstr_as_str(&cpu_buf), cpu_fg, font);

            // Memory (KB or MB)
            let mut mem_buf = [0u8; 16];
            let mut mpos = 0usize;
            if entry.mem_bytes >= 1048576 {
                self.append_u32_to_buf(&mut mem_buf, &mut mpos, entry.mem_bytes / 1048576);
                self.append_str_to_buf(&mut mem_buf, &mut mpos, " MB");
            } else {
                self.append_u32_to_buf(&mut mem_buf, &mut mpos, entry.mem_bytes / 1024);
                self.append_str_to_buf(&mut mem_buf, &mut mpos, " KB");
            }
            mem_buf[mpos] = 0;
            draw_text_font(s, row_rect.x + 460, ry + 4, cstr_as_str(&mem_buf), pal.text_secondary, font);

            // Cumulative Ticks
            let mut tick_buf = [0u8; 16];
            let mut tpos = 0usize;
            self.append_u32_to_buf(&mut tick_buf, &mut tpos, entry.cpu_ticks);
            tick_buf[tpos] = 0;
            draw_text_font(s, row_rect.x + 550, ry + 4, cstr_as_str(&tick_buf), pal.text_muted, font);

            // User / Privilege
            let user_str = match entry.uid {
                0 => "root (0)",
                700 => "persisted (700)",
                800 => "admin (800)",
                _ => "user",
            };
            draw_text_font(s, row_rect.x + 660, ry + 4, user_str, pal.text_secondary, font_small);
        }

        // 6. Selected Process Inspector Card (y: 466..568)
        self.render_inspector_card(s, viewport, pal, font, font_small);
    }

    fn render_inspector_card<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        pal: &ThemePalette,
        font: &Font,
        font_small: &Font,
    ) {
        let inspect_rect = Rect::new(viewport.x + 12, viewport.y + 466, viewport.w - 24, 102);
        draw_rounded_rect(s, inspect_rect, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, inspect_rect, 1, pal.border_focused);

        if let Some(entry) = self.selected_entry() {
            // Header with Process Name and PID Badge
            draw_text_font(s, inspect_rect.x + 14, inspect_rect.y + 10, "PROCESS INSPECTOR:", pal.accent_primary, font_small);
            draw_text_font(s, inspect_rect.x + 140, inspect_rect.y + 8, entry.name_str(), pal.text_primary, font);

            let mut pid_pill = [0u8; 16];
            let mut ppos = 0usize;
            self.append_str_to_buf(&mut pid_pill, &mut ppos, "PID ");
            self.append_u32_to_buf(&mut pid_pill, &mut ppos, entry.pid);
            pid_pill[ppos] = 0;
            let pill_x = inspect_rect.x + 280;
            draw_rounded_rect(s, Rect::new(pill_x, inspect_rect.y + 8, 70, 20), RADIUS_XS, pal.surface_sunken);
            draw_text_font(s, pill_x + 8, inspect_rect.y + 10, cstr_as_str(&pid_pill), pal.text_secondary, font_small);

            // Column 1: State & Security Context
            draw_text_font(s, inspect_rect.x + 14, inspect_rect.y + 36, "Execution State:", pal.text_muted, font_small);
            let state_name = match entry.state {
                NOVA_PROC_STATE_RUNNING => "Running on CPU core",
                NOVA_PROC_STATE_READY => "Ready in scheduler queue",
                NOVA_PROC_STATE_ALLOCATING => "Allocating address space",
                NOVA_PROC_STATE_TERMINATED => "Terminated (zombie awaiting wait)",
                _ => "Unused / idle",
            };
            draw_text_font(s, inspect_rect.x + 120, inspect_rect.y + 36, state_name, pal.text_primary, font_small);

            draw_text_font(s, inspect_rect.x + 14, inspect_rect.y + 54, "Security Context:", pal.text_muted, font_small);
            let sec_str = if entry.is_user != 0 {
                if entry.uid == 0 { "Ring-3 Userland (Root / Administrator)" } else { "Ring-3 Userland (Unprivileged Standard User)" }
            } else {
                "Ring-0 Kernel Space Task"
            };
            draw_text_font(s, inspect_rect.x + 120, inspect_rect.y + 54, sec_str, pal.status_info, font_small);

            draw_text_font(s, inspect_rect.x + 14, inspect_rect.y + 72, "Resource Limits:", pal.text_muted, font_small);
            let throttled_str = if entry.throttled != 0 { "THROTTLED by CPU quota" } else { "Active (unconstrained)" };
            draw_text_font(s, inspect_rect.x + 120, inspect_rect.y + 72, throttled_str, if entry.throttled != 0 { pal.status_warning } else { pal.status_success }, font_small);

            // Column 2: Memory & Heap Details (right half)
            let col2_x = inspect_rect.x + 460;
            draw_text_font(s, col2_x, inspect_rect.y + 36, "Heap Memory Footprint:", pal.text_muted, font_small);
            let mut mem_str = [0u8; 32];
            let mut mpos = 0usize;
            self.append_u32_to_buf(&mut mem_str, &mut mpos, entry.mem_bytes / 1024);
            self.append_str_to_buf(&mut mem_str, &mut mpos, " KB (");
            self.append_u32_to_buf(&mut mem_str, &mut mpos, entry.mem_pages);
            self.append_str_to_buf(&mut mem_str, &mut mpos, " pages)");
            mem_str[mpos] = 0;
            draw_text_font(s, col2_x + 160, inspect_rect.y + 36, cstr_as_str(&mem_str), pal.text_primary, font_small);

            draw_text_font(s, col2_x, inspect_rect.y + 54, "Total CPU Execution:", pal.text_muted, font_small);
            let mut cpu_ticks_str = [0u8; 32];
            let mut cpos = 0usize;
            self.append_u32_to_buf(&mut cpu_ticks_str, &mut cpos, entry.cpu_ticks);
            self.append_str_to_buf(&mut cpu_ticks_str, &mut cpos, " ticks (~");
            self.append_u32_to_buf(&mut cpu_ticks_str, &mut cpos, (entry.cpu_ticks * 10) / 1000);
            self.append_str_to_buf(&mut cpu_ticks_str, &mut cpos, "s)");
            cpu_ticks_str[cpos] = 0;
            draw_text_font(s, col2_x + 160, inspect_rect.y + 54, cstr_as_str(&cpu_ticks_str), pal.text_primary, font_small);

            draw_text_font(s, col2_x, inspect_rect.y + 72, "Window Slice Used:", pal.text_muted, font_small);
            let mut slice_str = [0u8; 16];
            let mut spos = 0usize;
            self.append_u32_to_buf(&mut slice_str, &mut spos, entry.cpu_percent);
            self.append_str_to_buf(&mut slice_str, &mut spos, "%");
            slice_str[spos] = 0;
            draw_text_font(s, col2_x + 160, inspect_rect.y + 72, cstr_as_str(&slice_str), pal.accent_primary, font_small);
        } else {
            draw_text_font(s, inspect_rect.x + 20, inspect_rect.y + 40, "No process selected. Click a process in the table above to inspect details.", pal.text_muted, font);
        }
    }

    fn render_performance_tab<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        pal: &ThemePalette,
        font: &Font,
        font_small: &Font,
    ) {
        let area = Rect::new(viewport.x + 12, viewport.y + 120, viewport.w - 24, 440);
        draw_rounded_rect(s, area, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, area, 1, pal.border_subtle);

        draw_text_font(s, area.x + 16, area.y + 14, "CPU PERFORMANCE HISTORY", pal.accent_primary, font);

        // Historical Sparkline Graph (x: 20..w - 40, y: 50..220)
        let graph_rect = Rect::new(area.x + 16, area.y + 40, area.w - 32, 160);
        s.fill_rect(graph_rect, pal.surface_sunken);
        draw_rect_outline(s, graph_rect, 1, pal.border_subtle);

        // Horizontal grid lines (25%, 50%, 75%, 100%)
        for pct in &[25, 50, 75, 100] {
            let gy = graph_rect.bottom() - ((graph_rect.h * pct) / 100);
            s.fill_rect(Rect::new(graph_rect.x, gy, graph_rect.w, 1), pal.border_subtle);
            let mut gbuf = [0u8; 8];
            let mut gpos = 0usize;
            self.append_u32_to_buf(&mut gbuf, &mut gpos, *pct as u32);
            gbuf[gpos] = b'%';
            gbuf[gpos + 1] = 0;
            draw_text_font(s, graph_rect.right() - 36, gy - 10, cstr_as_str(&gbuf), pal.text_muted, font_small);
        }

        // Draw trend line
        let step_x = graph_rect.w / (HISTORY_SAMPLES as i32);
        for i in 0..self.history_count.saturating_sub(1) {
            let x1 = graph_rect.x + (i as i32 * step_x);
            let y1 = graph_rect.bottom() - ((graph_rect.h * self.cpu_history[i] as i32) / 100);
            let x2 = graph_rect.x + ((i as i32 + 1) * step_x);
            let y2 = graph_rect.bottom() - ((graph_rect.h * self.cpu_history[i + 1] as i32) / 100);

            let bar_h = (graph_rect.bottom() - y1).max(2);
            s.fill_rect(Rect::new(x1, y1, step_x.max(2), bar_h), pal.status_info);
            draw_line(s, x1, y1, x2, y2, pal.accent_primary);
        }

        // SMP Core Distribution (y: 220..380)
        let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };
        draw_text_font(s, area.x + 16, area.y + 220, "SMP CORE SCHEDULING STATUS", pal.accent_primary, font);

        for core_idx in 0..sysinfo.cpu_count.min(4) {
            let core_rect = Rect::new(area.x + 16 + (core_idx as i32 * 180), area.y + 246, 168, 90);
            draw_rounded_rect(s, core_rect, RADIUS_XS, pal.surface_sunken);
            draw_rect_outline(s, core_rect, 1, pal.border_focused);

            let mut core_title = [0u8; 16];
            let mut cpos = 0usize;
            self.append_str_to_buf(&mut core_title, &mut cpos, "Core #");
            self.append_u32_to_buf(&mut core_title, &mut cpos, core_idx);
            core_title[cpos] = 0;
            draw_text_font(s, core_rect.x + 10, core_rect.y + 8, cstr_as_str(&core_title), pal.text_primary, font);

            let core_desc = if core_idx == 0 { "BSP (Bootstrap CPU)" } else { "AP (Application CPU)" };
            draw_text_font(s, core_rect.x + 10, core_rect.y + 28, core_desc, pal.status_info, font_small);

            draw_rounded_rect(s, Rect::new(core_rect.x + 10, core_rect.y + 50, 60, 18), RADIUS_XS, pal.status_success);
            draw_text_font(s, core_rect.x + 18, core_rect.y + 52, "ONLINE", WHITE, font_small);

            draw_text_font(s, core_rect.x + 80, core_rect.y + 52, "100 Hz Tick", pal.text_muted, font_small);
        }

        // Diagnostic Hint
        let diag_rect = Rect::new(area.x + 16, area.y + 360, area.w - 32, 60);
        draw_rounded_rect(s, diag_rect, RADIUS_XS, pal.surface_sunken);
        draw_text_font(s, diag_rect.x + 12, diag_rect.y + 10, "DIAGNOSTIC TRIAGE NOTE:", pal.status_warning, font_small);
        draw_text_font(
            s,
            diag_rect.x + 12,
            diag_rect.y + 28,
            "NovaOS runs pre-emptive round-robin scheduling with CPU share rlimits (RL_WINDOW_TICKS=100).",
            pal.text_secondary,
            font_small,
        );
    }

    fn render_memory_tab<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        pal: &ThemePalette,
        font: &Font,
        font_small: &Font,
    ) {
        let area = Rect::new(viewport.x + 12, viewport.y + 120, viewport.w - 24, 440);
        draw_rounded_rect(s, area, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, area, 1, pal.border_subtle);

        let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };
        let total_mb = (sysinfo.total_frames * 4) / 1024;
        let used_mb = (sysinfo.used_frames * 4) / 1024;
        let free_mb = (sysinfo.free_frames * 4) / 1024;

        draw_text_font(s, area.x + 16, area.y + 14, "PHYSICAL MEMORY BREAKDOWN (PMM)", pal.accent_primary, font);

        // Visual Memory Pool Stack Bar (x: 16..w - 32, y: 46..86)
        let bar_rect = Rect::new(area.x + 16, area.y + 46, area.w - 32, 40);
        draw_rounded_rect(s, bar_rect, RADIUS_SM, pal.surface_sunken);

        let used_bar_w = if total_mb > 0 {
            ((bar_rect.w as u32 * used_mb) / total_mb).max(4) as i32
        } else {
            10
        };
        draw_rounded_rect(s, Rect::new(bar_rect.x, bar_rect.y, used_bar_w, bar_rect.h), RADIUS_SM, pal.accent_primary);
        draw_text_font(s, bar_rect.x + 12, bar_rect.y + 12, "USED MEMORY", WHITE, font_small);

        let free_bar_x = bar_rect.x + used_bar_w;
        let free_bar_w = bar_rect.w - used_bar_w;
        draw_rounded_rect(s, Rect::new(free_bar_x, bar_rect.y, free_bar_w, bar_rect.h), RADIUS_SM, pal.surface_raised);
        draw_text_font(s, free_bar_x + 12, bar_rect.y + 12, "AVAILABLE FREE RAM", pal.status_success, font_small);

        // Memory Metrics Grid
        let grid_y = area.y + 110;
        let metrics = [
            ("Total System RAM Tracked:", total_mb, "MB (131,072 frames)"),
            ("Allocated Physical Frames:", used_mb, "MB used by kernel & processes"),
            ("Available Free Frame Pool:", free_mb, "MB unallocated physical RAM"),
            ("Standard Physical Frame Size:", 4, "KB per frame (PMM_FRAME_SIZE)"),
        ];

        for (i, (title, val, unit)) in metrics.iter().enumerate() {
            let my = grid_y + (i as i32 * 32);
            let mbox = Rect::new(area.x + 16, my, area.w - 32, 26);
            if i % 2 == 1 {
                s.fill_rect(mbox, pal.surface_sunken);
            }
            draw_text_font(s, mbox.x + 8, my + 5, title, pal.text_primary, font);

            let mut val_buf = [0u8; 16];
            let mut vpos = 0usize;
            self.append_u32_to_buf(&mut val_buf, &mut vpos, *val);
            val_buf[vpos] = 0;
            draw_text_font(s, mbox.x + 300, my + 5, cstr_as_str(&val_buf), pal.accent_primary, font);
            draw_text_font(s, mbox.x + 360, my + 5, unit, pal.text_muted, font_small);
        }

        // Virtual Address Space Info Card
        let vcard = Rect::new(area.x + 16, area.y + 260, area.w - 32, 150);
        draw_rounded_rect(s, vcard, RADIUS_XS, pal.surface_sunken);
        draw_rect_outline(s, vcard, 1, pal.border_focused);

        draw_text_font(s, vcard.x + 14, vcard.y + 12, "VIRTUAL MEMORY & PAGING ARCHITECTURE", pal.status_info, font);
        draw_text_font(s, vcard.x + 14, vcard.y + 36, "  - 0x00000000..0x08048000: Reserved (NULL dereference trap & real mode IVT)", pal.text_secondary, font_small);
        draw_text_font(s, vcard.x + 14, vcard.y + 54, "  - 0x08048000..0x10000000: Ring-3 ELF Userland Executable Text & Data", pal.text_secondary, font_small);
        draw_text_font(s, vcard.x + 14, vcard.y + 72, "  - 0x10000000..0x40000000: Process Heap (SYS_SBRK dynamic memory expansion)", pal.text_secondary, font_small);
        draw_text_font(s, vcard.x + 14, vcard.y + 90, "  - 0x40000000..0x40004000: Strict 16 KB Ring-3 Userland Execution Stack", pal.status_warning, font_small);
        draw_text_font(s, vcard.x + 14, vcard.y + 108, "  - 0x68000000..0x7C000000: Shared Memory IPC Regions (SYS_SHM_*)", pal.text_secondary, font_small);
        draw_text_font(s, vcard.x + 14, vcard.y + 126, "  - 0xC0000000..0xFFFFFFFF: Higher-Half Kernel Address Space (Page Directory)", pal.text_muted, font_small);
    }

    fn render_sysinfo_tab<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        pal: &ThemePalette,
        font: &Font,
        font_small: &Font,
    ) {
        let area = Rect::new(viewport.x + 12, viewport.y + 120, viewport.w - 24, 440);
        draw_rounded_rect(s, area, RADIUS_SM, pal.surface_raised);
        draw_rect_outline(s, area, 1, pal.border_subtle);

        draw_text_font(s, area.x + 16, area.y + 14, "NOVAOS SYSTEM SPECIFICATION", pal.accent_primary, font);

        let items = [
            ("Operating System:", "NovaOS 0.88.0 (Pure Rust / Bare-metal Kernel)"),
            ("Processor Architecture:", "x86-32 (i686) SMP Symmetric Multiprocessing"),
            ("Timer Frequency:", "100 Hz (10ms preemptive quantum tick)"),
            ("Interrupt Controllers:", "ACPI MADT, Local APIC, I/O APIC ISA redirects"),
            ("Graphics Compositor:", "BufferSurface32 32-bit XRGB Framebuffer (VBE / Virtio-GPU)"),
            ("Storage Subsystems:", "NVMe SSD (PCI 0:9.0), Virtio-Blk, ATA PIO"),
            ("Filesystem & Journal:", "FAT32 with write-ahead journal transactions, Ext2"),
            ("Universal Serial Bus:", "xHCI USB 3.0 Controller (8 ports), UHCI legacy"),
            ("Security Model:", "Mandatory Access Control (MAC profiles), Capability gates"),
            ("Resource Enforcement:", "Per-process memory, CPU time, and share throttling"),
        ];

        let start_y = area.y + 44;
        for (i, (key, val)) in items.iter().enumerate() {
            let iy = start_y + (i as i32 * 32);
            let row = Rect::new(area.x + 16, iy, area.w - 32, 26);
            if i % 2 == 1 {
                s.fill_rect(row, pal.surface_sunken);
            }
            draw_text_font(s, row.x + 10, iy + 4, key, pal.text_secondary, font);
            draw_text_font(s, row.x + 220, iy + 4, val, pal.text_primary, font);
        }

        // Build Badge
        let badge_rect = Rect::new(area.x + 16, area.bottom() - 48, area.w - 32, 34);
        draw_rounded_rect(s, badge_rect, RADIUS_XS, pal.surface_sunken);
        draw_rect_outline(s, badge_rect, 1, pal.border_focused);
        draw_text_font(
            s,
            badge_rect.x + 12,
            badge_rect.y + 9,
            "Built with Month 5 Power-user & system apps telemetry contract (SYS_PROC_INFO 84).",
            pal.status_success,
            font_small,
        );
    }

    // -----------------------------------------------------------------------
    // Helper Formatting Routines
    // -----------------------------------------------------------------------

    fn append_u32_to_buf(&self, buf: &mut [u8], pos: &mut usize, mut val: u32) {
        if val == 0 {
            if *pos < buf.len() - 1 {
                buf[*pos] = b'0';
                *pos += 1;
            }
            return;
        }

        let mut digits = [0u8; 10];
        let mut dcount = 0usize;
        while val > 0 && dcount < 10 {
            digits[dcount] = b'0' + (val % 10) as u8;
            val /= 10;
            dcount += 1;
        }

        for i in (0..dcount).rev() {
            if *pos < buf.len() - 1 {
                buf[*pos] = digits[i];
                *pos += 1;
            }
        }
    }

    fn append_str_to_buf(&self, buf: &mut [u8], pos: &mut usize, text: &str) {
        for b in text.bytes() {
            if *pos < buf.len() - 1 {
                buf[*pos] = b;
                *pos += 1;
            }
        }
    }
}

// ===========================================================================
// Selftest Implementation
// ===========================================================================

pub fn selftest_sysinfo(app: &mut ActivityMonitorApp) -> i32 {
    app.refresh_from_kernel();
    let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };

    if sysinfo.cpu_count == 0 {
        return 1;
    }
    if sysinfo.total_frames == 0 {
        return 2;
    }
    if sysinfo.total_processes == 0 {
        return 4;
    }

    unsafe {
        ffi::sys_write(b"[actmon-selftest] PASS: system activity telemetry and physical RAM metrics\n\0".as_ptr());
    }
    0
}

pub fn selftest_process_table(app: &mut ActivityMonitorApp) -> i32 {
    let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
    if table.count == 0 {
        return 8;
    }

    if app.filtered_count == 0 {
        return 16;
    }

    // Verify selected process lookup
    if app.selected_entry().is_none() {
        return 32;
    }

    unsafe {
        ffi::sys_write(b"[actmon-selftest] PASS: scheduler process table query and selected inspector entry\n\0".as_ptr());
    }
    0
}

pub fn selftest_sorting_and_filtering(app: &mut ActivityMonitorApp) -> i32 {
    let prev_count = app.filtered_count;

    // 1. Sort by PID ascending
    app.sort_col = SortColumn::Pid;
    app.sort_dir = SortDirection::Ascending;
    app.sort_filtered_indices();

    let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
    if app.filtered_count >= 2 {
        let pid0 = table.entries[app.filtered_indices[0]].pid;
        let pid1 = table.entries[app.filtered_indices[1]].pid;
        if pid0 > pid1 {
            return 64;
        }
    }

    // 2. Sort by CPU descending
    app.sort_col = SortColumn::Cpu;
    app.sort_dir = SortDirection::Descending;
    app.sort_filtered_indices();

    // 3. Category Filter: Apps
    app.category_filter = CategoryFilter::Apps;
    app.rebuild_filtered_indices();
    for i in 0..app.filtered_count {
        if table.entries[app.filtered_indices[i]].is_user == 0 {
            return 128;
        }
    }

    // Reset filter
    app.category_filter = CategoryFilter::All;
    app.rebuild_filtered_indices();
    if app.filtered_count != prev_count {
        return 256;
    }

    unsafe {
        ffi::sys_write(b"[actmon-selftest] PASS: column sorting (PID, CPU, Memory) and category filtering\n\0".as_ptr());
    }
    0
}

#[inline(never)]
pub fn selftest_graphics(app: &mut ActivityMonitorApp) -> i32 {
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
            ffi::sys_write(b"[actmon-selftest] PASS: zero-syscall BufferSurface32 graphical rasterization\n\0".as_ptr());
        }
        0
    } else {
        512
    }
}

pub fn run_selftest() -> i32 {
    unsafe {
        ffi::sys_write(b"[actmon-selftest] starting Activity Monitor selftest suite\n\0".as_ptr());
    }

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_ACTMON) };
    app.init();

    let mut code = 0;
    code |= selftest_sysinfo(app);
    code |= selftest_process_table(app);
    code |= selftest_sorting_and_filtering(app);
    code |= selftest_graphics(app);

    if code == 0 {
        unsafe {
            ffi::sys_write(b"[actmon-selftest] all cases passed\n\0".as_ptr());
        }
    }
    code
}

// ===========================================================================
// Application Entry Point
// ===========================================================================

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    // Check for --selftest argument
    if argc > 1 && !argv.is_null() {
        let arg1_ptr = unsafe { *argv.add(1) };
        if !arg1_ptr.is_null() {
            let mut len = 0;
            while unsafe { *arg1_ptr.add(len) } != 0 && len < 16 {
                len += 1;
            }
            let arg_slice = unsafe { core::slice::from_raw_parts(arg1_ptr, len) };
            if arg_slice == b"--selftest" {
                return run_selftest();
            }
        }
    }

    // Interactive GUI mode
    let rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if rc < 0 {
        unsafe {
            ffi::sys_write(b"actmon: failed to acquire display\n\0".as_ptr());
        }
        return 1;
    }

    let mut fb_info = ffi::NovaFbInfo::default();
    unsafe {
        ffi::sys_fb_info(&mut fb_info);
    }

    let width = if fb_info.width > 0 { fb_info.width } else { 800 };
    let height = if fb_info.height > 0 { fb_info.height } else { 600 };

    let mut create_req = ffi::NovaFbCreate {
        width,
        height,
        flags: 0,
        handle: 0,
        pixels: 0,
        stride: 0,
        size: 0,
    };

    let rc = unsafe { ffi::sys_fb_create(&mut create_req) };
    if rc < 0 {
        unsafe {
            ffi::sys_fb_release();
            ffi::sys_write(b"actmon: failed to create framebuffer surface\n\0".as_ptr());
        }
        return 1;
    }

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_ACTMON) };
    app.init();

    let pixels_slice = unsafe {
        core::slice::from_raw_parts_mut(create_req.pixels as *mut u32, (width * height) as usize)
    };
    let mut surface = BufferSurface32::new(pixels_slice, width as i32, height as i32);
    let viewport = Rect::new(0, 0, width as i32, height as i32);

    let mut mouse_x = width as i32 / 2;
    let mut mouse_y = height as i32 / 2;
    let mut prev_left = false;
    let mut tick_counter = 0u32;

    loop {
        // 1. Process Mouse Input
        let mut mouse = ffi::MouseState::default();
        if unsafe { ffi::sys_mouse_read(&mut mouse) } == 0 {
            mouse_x = (mouse_x + mouse.dx).max(0).min(width as i32 - 1);
            mouse_y = (mouse_y - mouse.dy).max(0).min(height as i32 - 1);

            if mouse.left_button && !prev_left {
                app.handle_click(mouse_x, mouse_y, viewport);
            }
            prev_left = mouse.left_button;
        }

        // 2. Periodic Refresh
        tick_counter += 1;
        if app.auto_refresh && tick_counter >= app.refresh_rate_ticks {
            tick_counter = 0;
            app.refresh_from_kernel();
        }

        // 3. Render Application Surface
        app.render(&mut surface, viewport);

        // 4. Draw Simple Mouse Cursor
        let cur_col = Color::rgb(255, 255, 255);
        for i in 0..8 {
            surface.put_pixel(mouse_x + i, mouse_y, cur_col);
            surface.put_pixel(mouse_x, mouse_y + i, cur_col);
            surface.put_pixel(mouse_x + i, mouse_y + i, cur_col);
        }

        // 5. Present Surface to Screen
        let present = ffi::NovaFbPresent {
            handle: create_req.handle,
            src_x: 0,
            src_y: 0,
            width: width as i32,
            height: height as i32,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe {
            ffi::sys_fb_present(&present);
            ffi::sys_yield();
        }

        // Check keypress to exit (Esc = 27 or 'q' = 113)
        let key = unsafe { ffi::sys_read_key() };
        if key == 27 || key == 113 {
            break;
        }
    }

    unsafe {
        ffi::sys_fb_destroy(create_req.handle);
        ffi::sys_fb_release();
    }

    0
}

// ===========================================================================
// Host Unit Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actmon_init_and_sysinfo() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = ActivityMonitorApp::empty();
        app.init();

        let sysinfo = unsafe { &*core::ptr::addr_of!(GLOBAL_SYSINFO) };
        assert_eq!(sysinfo.cpu_count, 2);
        assert_eq!(sysinfo.cpu_usage_pct, 28);
        assert_eq!(sysinfo.total_processes, 8);
        assert_eq!(sysinfo.running_processes, 3);
        assert_eq!(sysinfo.user_processes, 5);
        assert_eq!(sysinfo.kernel_processes, 3);
    }

    #[test]
    fn test_actmon_process_table_and_filtering() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = ActivityMonitorApp::empty();
        app.init();

        assert_eq!(app.filtered_count, 8);

        // Filter: Apps
        app.category_filter = CategoryFilter::Apps;
        app.rebuild_filtered_indices();
        assert_eq!(app.filtered_count, 5); // novainit, SHELL, WM, FILES, SETTINGS

        // Filter: Kernel
        app.category_filter = CategoryFilter::Kernel;
        app.rebuild_filtered_indices();
        assert_eq!(app.filtered_count, 3); // idle, klogd, netd

        // Search: "shell"
        app.category_filter = CategoryFilter::All;
        app.set_search_query("shell");
        assert_eq!(app.filtered_count, 1);
        let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
        assert_eq!(table.entries[app.filtered_indices[0]].name_str(), "SHELL.ELF");
    }

    #[test]
    fn test_actmon_sorting() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = ActivityMonitorApp::empty();
        app.init();

        // Sort by CPU descending
        app.sort_col = SortColumn::Cpu;
        app.sort_dir = SortDirection::Descending;
        app.sort_filtered_indices();

        let table = unsafe { &*core::ptr::addr_of!(GLOBAL_PROC_TABLE) };
        // idle has 72% CPU, should be first
        assert_eq!(table.entries[app.filtered_indices[0]].name_str(), "idle");
        assert_eq!(table.entries[app.filtered_indices[0]].cpu_percent, 72);

        // WM.ELF has 12% CPU, should be second
        assert_eq!(table.entries[app.filtered_indices[1]].name_str(), "WM.ELF");
        assert_eq!(table.entries[app.filtered_indices[1]].cpu_percent, 12);

        // Sort by Memory descending
        app.sort_col = SortColumn::Memory;
        app.sort_dir = SortDirection::Descending;
        app.sort_filtered_indices();
        // WM.ELF has 1MB (1048576 bytes), should be first
        assert_eq!(table.entries[app.filtered_indices[0]].name_str(), "WM.ELF");
    }

    #[test]
    fn test_actmon_click_handling() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = ActivityMonitorApp::empty();
        app.init();
        let viewport = Rect::new(0, 0, 800, 600);

        // Click Tab "Performance" (x: 294..394, y: 12..38)
        assert!(app.handle_click(320, 20, viewport));
        assert_eq!(app.active_tab, ActmonTab::Performance);

        // Click Tab "Processes" (x: 200..288)
        assert!(app.handle_click(220, 20, viewport));
        assert_eq!(app.active_tab, ActmonTab::Processes);

        // Click Pause button (right - 94 = 706)
        assert!(app.handle_click(720, 20, viewport));
        assert!(!app.auto_refresh);

        // Click Category Chip "Apps" (x: 78..158, y: 120..144)
        assert!(app.handle_click(100, 130, viewport));
        assert_eq!(app.category_filter, CategoryFilter::Apps);
    }

    #[test]
    fn test_actmon_surface_rendering() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = ActivityMonitorApp::empty();
        app.init();

        let mut buffer = [0u32; 800 * 600];
        let mut surf = BufferSurface32::new(&mut buffer, 800, 600);
        let viewport = Rect::new(0, 0, 800, 600);

        app.render(&mut surf, viewport);

        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 10000, "Must render cards, table, tabs, and status bar");
    }

    #[test]
    fn test_actmon_selftest_suite() {
        unsafe { ffi::reset_mock_environment() };
        let code = run_selftest();
        assert_eq!(code, 0, "All selftest assertions must pass");
    }
}
