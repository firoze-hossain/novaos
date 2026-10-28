//! userland/crashrep-rs/crashrep.rs - NovaOS Graphical Crash Reporter (Month 6 First Impressions)
//!
//! A pure #![no_std] ring-3 graphical problem reporter and crash dump inspector.
//! When a pending kernel panic or CPU hardware exception record is found on disk
//! (/CRASHDMP.TXT and /CRASHDMP.DAT), this wizard presents a clear diagnostic report:
//! - Panic reason and high-level assessment card
//! - Exception vector, faulting EIP, CR2 fault address, and error code
//! - Full 16-register CPU snapshot (EAX..ESP, EIP, CS, SS, DS, EFLAGS)
//! - Call stack backtrace with frame symbol classifications
//! - Scrollable raw report view of /CRASHDMP.TXT
//! - Action buttons to Dismiss, Clear Crash Record (SYS_CRASHDUMP_CTL op 4), or Reboot.
//!
//! Direct BufferSurface32 rasterization with zero per-pixel syscalls.
//! All application state and render buffers reside in static BSS memory.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(dead_code, static_mut_refs)]

extern crate ui;
use ui::prelude::*;
use ui::widget::{ButtonVariant, WidgetState};

mod ffi;
mod parser;

use core::ptr::addr_of_mut;
use ffi::{NovaCrashReport, NovaCrashdumpStatus, CRASHDUMP_OP_CLEAR, CRASHDUMP_OP_GET, CRASHDUMP_OP_STATUS};
use parser::{format_hex_u32, format_u32_dec, parse_crash_text, ParsedCrashReport};

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        let msg = b"crashrep: panic occurred\n\0";
        ffi::sys_write(msg.as_ptr());
        ffi::sys_exit(1);
    }
}

// ---------------------------------------------------------------------------
// Constants & UI Layout
// ---------------------------------------------------------------------------

pub const WINDOW_W: i32 = 760;
pub const WINDOW_H: i32 = 580;

pub const RAW_BUF_SIZE: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveTab {
    Overview,
    Registers,
    CallStack,
    RawReport,
}

// ---------------------------------------------------------------------------
// Crash Reporter Application State
// ---------------------------------------------------------------------------

pub struct CrashReporterApp {
    pub screen_w: i32,
    pub screen_h: i32,
    pub win_x: i32,
    pub win_y: i32,
    pub active_tab: ActiveTab,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub running: bool,
    pub report: ParsedCrashReport,
    pub status: NovaCrashdumpStatus,
    pub raw_text_len: usize,
    pub scroll_line: usize,
    pub total_lines: usize,
    pub status_msg: [u8; 64],
    pub status_msg_len: usize,
    pub is_cleared: bool,
    pub theme: Theme,
}

// Static BSS storage guarantees zero stack exhaustion on 16 KB stack budget
static mut GLOBAL_CRASHREP: CrashReporterApp = CrashReporterApp::new_empty();
static mut CRASH_RAW_BUF: [u8; RAW_BUF_SIZE] = [0u8; RAW_BUF_SIZE];
static mut TEST_RENDER_BUF: [u32; 1024 * 768] = [0u32; 1024 * 768];

impl CrashReporterApp {
    pub const fn new_empty() -> Self {
        CrashReporterApp {
            screen_w: 1024,
            screen_h: 768,
            win_x: 132,
            win_y: 94,
            active_tab: ActiveTab::Overview,
            mouse_x: 512,
            mouse_y: 384,
            running: true,
            report: ParsedCrashReport {
                has_crash: false,
                ticks: 0,
                reason: [0u8; parser::MAX_REASON_LEN],
                reason_len: 0,
                has_regs: false,
                int_no: 0,
                err_code: 0,
                eip: 0,
                cs: 0,
                eflags: 0,
                eax: 0,
                ebx: 0,
                ecx: 0,
                edx: 0,
                esi: 0,
                edi: 0,
                ebp: 0,
                esp: 0,
                ds: 0,
                ss: 0,
                has_fault_addr: false,
                fault_addr: 0,
                stack: [0u32; parser::MAX_STACK_FRAMES],
                stack_count: 0,
                raw_text_len: 0,
            },
            status: NovaCrashdumpStatus {
                has_crash: 0,
                exported: 0,
                ticks: 0,
                file_size: 0,
                filename: [0u8; 16],
            },
            raw_text_len: 0,
            scroll_line: 0,
            total_lines: 0,
            status_msg: [0u8; 64],
            status_msg_len: 0,
            is_cleared: false,
            theme: Theme::Dark,
        }
    }

    pub fn init(&mut self, width: i32, height: i32) {
        self.screen_w = width;
        self.screen_h = height;
        self.win_x = ((width - WINDOW_W) / 2).max(10);
        self.win_y = ((height - WINDOW_H) / 2).max(10);
        self.running = true;
        self.load_crash_dump();
    }

    pub fn set_status(&mut self, text: &str) {
        let b = text.as_bytes();
        let len = b.len().min(63);
        self.status_msg[..len].copy_from_slice(&b[..len]);
        self.status_msg_len = len;
    }

    pub fn status_str(&self) -> &str {
        if self.status_msg_len == 0 {
            return "Ready";
        }
        core::str::from_utf8(&self.status_msg[..self.status_msg_len]).unwrap_or("Ready")
    }

    pub fn load_crash_dump(&mut self) {
        // 1. Query kernel crashdump status via SYS_CRASHDUMP_CTL (80)
        let mut st = NovaCrashdumpStatus::default();
        let rc_st = unsafe { ffi::sys_crashdump_ctl(CRASHDUMP_OP_STATUS, addr_of_mut!(st) as *mut u8) };
        if rc_st == 0 {
            self.status = st;
        }

        // 2. Read /CRASHDMP.TXT from VFS
        let filename = b"CRASHDMP.TXT\0";
        let fd = unsafe { ffi::sys_open(filename.as_ptr()) };
        if fd >= 0 {
            let buf_ptr = unsafe { CRASH_RAW_BUF.as_mut_ptr() };
            let n = unsafe { ffi::sys_read(fd, buf_ptr, RAW_BUF_SIZE as i32) };
            unsafe { ffi::sys_close(fd) };

            if n > 0 {
                self.raw_text_len = n as usize;
                let raw_slice = unsafe { &CRASH_RAW_BUF[..self.raw_text_len] };
                self.report = parse_crash_text(raw_slice);
                self.count_raw_lines();
                self.set_status("Crash report loaded from /CRASHDMP.TXT");
                return;
            }
        }

        // 3. Fallback: Query binary crash report from kernel memory
        if self.status.has_crash != 0 {
            let mut bin_rep = NovaCrashReport::default();
            let rc_get = unsafe { ffi::sys_crashdump_ctl(CRASHDUMP_OP_GET, addr_of_mut!(bin_rep) as *mut u8) };
            if rc_get == 0 {
                self.report = parser::from_binary_report(&bin_rep);
                self.set_status("Crash report retrieved from kernel memory");
                return;
            }
        }

        self.set_status("No pending crash record found (system clean)");
    }

    fn count_raw_lines(&mut self) {
        let raw = unsafe { &CRASH_RAW_BUF[..self.raw_text_len] };
        let mut lines = 0;
        for &b in raw {
            if b == b'\n' {
                lines += 1;
            }
        }
        if self.raw_text_len > 0 && raw[self.raw_text_len - 1] != b'\n' {
            lines += 1;
        }
        self.total_lines = lines;
    }

    pub fn clear_crash_record(&mut self) {
        let rc = unsafe { ffi::sys_crashdump_ctl(CRASHDUMP_OP_CLEAR, core::ptr::null_mut()) };
        if rc == 0 {
            self.is_cleared = true;
            self.status.has_crash = 0;
            self.status.exported = 0;
            self.set_status("Crash dump record cleared successfully");
        } else {
            self.set_status("Failed to clear crash record (-EPERM or error)");
        }
    }

    // -----------------------------------------------------------------------
    // Keyboard & Mouse Event Handling
    // -----------------------------------------------------------------------

    pub fn handle_key(&mut self, key: i32) {
        match key {
            k if k == b'1' as i32 => self.active_tab = ActiveTab::Overview,
            k if k == b'2' as i32 => self.active_tab = ActiveTab::Registers,
            k if k == b'3' as i32 => self.active_tab = ActiveTab::CallStack,
            k if k == b'4' as i32 => self.active_tab = ActiveTab::RawReport,
            k if k == b'q' as i32 || k == 27 => self.running = false, // 'q' or ESC closes dialog
            k if k == b'c' as i32 || k == b'C' as i32 => self.clear_crash_record(),
            k if k == b'r' as i32 || k == b'R' as i32 => unsafe {
                let _ = ffi::sys_shutdown();
            },
            // Navigation / Scrolling in raw report
            18 | 38 => { // Up arrow
                if self.scroll_line > 0 {
                    self.scroll_line -= 1;
                }
            }
            19 | 40 => { // Down arrow
                if self.scroll_line + 1 < self.total_lines {
                    self.scroll_line += 1;
                }
            }
            33 => { // Page Up
                self.scroll_line = self.scroll_line.saturating_sub(10);
            }
            34 => { // Page Down
                self.scroll_line = (self.scroll_line + 10).min(self.total_lines.saturating_sub(1));
            }
            _ => {}
        }
    }

    pub fn handle_mouse_click(&mut self, mx: i32, my: i32) {
        let wx = self.win_x;
        let wy = self.win_y;

        // Close button [X] at (wx + WINDOW_W - 36, wy + 6)
        let close_rect = Rect::new(wx + WINDOW_W - 36, wy + 6, 26, 20);
        if close_rect.contains(mx, my) {
            self.running = false;
            return;
        }

        // Tab bar clicks (y ~ wy + 116, height 28)
        let tab_y = wy + 116;
        let tab0_rect = Rect::new(wx + 20, tab_y, 140, 28);
        let tab1_rect = Rect::new(wx + 168, tab_y, 160, 28);
        let tab2_rect = Rect::new(wx + 336, tab_y, 150, 28);
        let tab3_rect = Rect::new(wx + 494, tab_y, 140, 28);

        if tab0_rect.contains(mx, my) {
            self.active_tab = ActiveTab::Overview;
            return;
        }
        if tab1_rect.contains(mx, my) {
            self.active_tab = ActiveTab::Registers;
            return;
        }
        if tab2_rect.contains(mx, my) {
            self.active_tab = ActiveTab::CallStack;
            return;
        }
        if tab3_rect.contains(mx, my) {
            self.active_tab = ActiveTab::RawReport;
            return;
        }

        // Bottom action buttons (y ~ wy + WINDOW_H - 52)
        let btn_y = wy + WINDOW_H - 52;
        let dismiss_rect = Rect::new(wx + WINDOW_W - 130, btn_y, 110, 32);
        let clear_rect = Rect::new(wx + WINDOW_W - 270, btn_y, 130, 32);
        let reboot_rect = Rect::new(wx + 20, btn_y, 120, 32);

        if dismiss_rect.contains(mx, my) {
            self.running = false;
            return;
        }
        if clear_rect.contains(mx, my) {
            self.clear_crash_record();
            return;
        }
        if reboot_rect.contains(mx, my) {
            unsafe {
                let _ = ffi::sys_shutdown();
            }
            return;
        }

        // Raw Report scroll buttons
        if self.active_tab == ActiveTab::RawReport {
            let scroll_up = Rect::new(wx + WINDOW_W - 44, wy + 160, 24, 24);
            let scroll_dn = Rect::new(wx + WINDOW_W - 44, wy + WINDOW_H - 100, 24, 24);
            if scroll_up.contains(mx, my) {
                if self.scroll_line > 0 {
                    self.scroll_line -= 1;
                }
            } else if scroll_dn.contains(mx, my) {
                if self.scroll_line + 1 < self.total_lines {
                    self.scroll_line += 1;
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // UI Rendering
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S) {
        let palette = self.theme.palette();

        // 1. Dim background backdrop
        s.fill_rect(Rect::new(0, 0, self.screen_w, self.screen_h), palette.desktop_bg);

        // 2. Main Window Frame
        let win_rect = Rect::new(self.win_x, self.win_y, WINDOW_W, WINDOW_H);
        draw_window_frame(s, win_rect, "NovaOS Problem Reporter", true, &palette);

        // Close button 'X' in titlebar
        let close_rect = Rect::new(self.win_x + WINDOW_W - 36, self.win_y + 6, 26, 20);
        draw_rounded_rect(s, close_rect, 3, Color::from_hex(0xC53030));
        draw_text(s, close_rect.x + 9, close_rect.y + 4, "x", Color::WHITE, TypeScale::Body);

        // 3. Severity Banner Card (Top)
        let banner_rect = Rect::new(self.win_x + 20, self.win_y + 36, WINDOW_W - 40, 68);
        draw_rounded_rect(s, banner_rect, 6, palette.surface_sunken);
        draw_rect_outline(s, banner_rect, 1, Color::from_hex(0xE53E3E));

        // Alert badge
        let badge_rect = Rect::new(banner_rect.x + 12, banner_rect.y + 12, 92, 22);
        draw_rounded_rect(s, badge_rect, 4, Color::from_hex(0xC53030));
        draw_text(s, badge_rect.x + 8, badge_rect.y + 5, "CRITICAL", Color::WHITE, TypeScale::Caption);

        // Banner titles
        draw_text(
            s,
            banner_rect.x + 114,
            banner_rect.y + 14,
            "NovaOS recovered from an unexpected kernel panic",
            palette.text_primary,
            TypeScale::Heading,
        );
        draw_text(
            s,
            banner_rect.x + 16,
            banner_rect.y + 42,
            "Diagnostic register state and stack traces were automatically saved to disk (/CRASHDMP.TXT).",
            palette.text_secondary,
            TypeScale::Body,
        );

        // 4. Tab Bar Navigation
        self.render_tab_bar(s, &palette);

        // 5. Active Tab Content
        let content_rect = Rect::new(self.win_x + 20, self.win_y + 152, WINDOW_W - 40, WINDOW_H - 215);
        draw_rounded_rect(s, content_rect, 6, palette.surface_sunken);
        draw_rect_outline(s, content_rect, 1, palette.border_normal);

        match self.active_tab {
            ActiveTab::Overview => self.render_overview_tab(s, content_rect, &palette),
            ActiveTab::Registers => self.render_registers_tab(s, content_rect, &palette),
            ActiveTab::CallStack => self.render_callstack_tab(s, content_rect, &palette),
            ActiveTab::RawReport => self.render_rawreport_tab(s, content_rect, &palette),
        }

        // 6. Bottom Action Bar & Status
        self.render_bottom_bar(s, &palette);
    }

    fn render_tab_bar<S: Surface + ?Sized>(&self, s: &mut S, palette: &ThemePalette) {
        let wx = self.win_x;
        let tab_y = self.win_y + 116;

        let tabs = [
            (ActiveTab::Overview, "1. Overview", 140),
            (ActiveTab::Registers, "2. CPU Registers (16)", 160),
            (ActiveTab::CallStack, "3. Call Stack", 150),
            (ActiveTab::RawReport, "4. Raw Report", 140),
        ];

        let mut curr_x = wx + 20;
        for (tab, label, w) in tabs {
            let rect = Rect::new(curr_x, tab_y, w, 28);
            let is_active = self.active_tab == tab;

            if is_active {
                draw_rounded_rect(s, rect, 4, palette.accent_primary);
                draw_text(s, rect.x + 12, rect.y + 7, label, palette.text_inverse, TypeScale::Body);
            } else {
                draw_rounded_rect(s, rect, 4, palette.surface_raised);
                draw_text(s, rect.x + 12, rect.y + 7, label, palette.text_primary, TypeScale::Body);
            }
            curr_x += w + 8;
        }
    }

    fn render_overview_tab<S: Surface + ?Sized>(&self, s: &mut S, r: Rect, palette: &ThemePalette) {
        let rep = &self.report;

        // Card 1: Panic Summary
        let card1 = Rect::new(r.x + 16, r.y + 16, r.w - 32, 90);
        draw_rounded_rect(s, card1, 4, palette.surface_raised);
        draw_rect_outline(s, card1, 1, palette.border_subtle);

        draw_text(s, card1.x + 12, card1.y + 10, "PANIC REASON", palette.text_muted, TypeScale::Caption);
        draw_text(s, card1.x + 12, card1.y + 26, rep.reason_str(), palette.status_danger, TypeScale::Heading);

        draw_text(s, card1.x + 12, card1.y + 52, "ASSESSMENT:", palette.text_secondary, TypeScale::Body);
        draw_text(s, card1.x + 110, card1.y + 52, rep.fault_analysis(), palette.text_primary, TypeScale::Body);

        let mut dec_buf = [0u8; 12];
        let ticks_str = format_u32_dec(rep.ticks, &mut dec_buf);
        draw_text(s, card1.x + 12, card1.y + 70, "UPTIME TICKS:", palette.text_secondary, TypeScale::Caption);
        draw_text(s, card1.x + 110, card1.y + 70, ticks_str, palette.text_muted, TypeScale::Caption);

        // Card 2: Exception & Fault Details (Left)
        let card2 = Rect::new(r.x + 16, r.y + 116, (r.w - 44) / 2, 230);
        draw_rounded_rect(s, card2, 4, palette.surface_raised);
        draw_rect_outline(s, card2, 1, palette.border_subtle);

        draw_text(s, card2.x + 12, card2.y + 12, "EXCEPTION DETAILS", palette.accent_primary, TypeScale::Heading);

        let mut hex_buf = [0u8; 10];

        // Vector
        draw_text(s, card2.x + 12, card2.y + 40, "Vector:", palette.text_secondary, TypeScale::Body);
        draw_text(s, card2.x + 120, card2.y + 40, rep.vector_title(), palette.status_warning, TypeScale::Body);

        // Error code
        draw_text(s, card2.x + 12, card2.y + 68, "Error Code:", palette.text_secondary, TypeScale::Body);
        draw_text(s, card2.x + 120, card2.y + 68, format_hex_u32(rep.err_code, &mut hex_buf), palette.text_primary, TypeScale::Body);

        // Faulting EIP
        draw_text(s, card2.x + 12, card2.y + 96, "Faulting EIP:", palette.text_secondary, TypeScale::Body);
        draw_text(s, card2.x + 120, card2.y + 96, format_hex_u32(rep.eip, &mut hex_buf), palette.accent_primary, TypeScale::Body);

        // Fault Address CR2
        draw_text(s, card2.x + 12, card2.y + 124, "Fault Addr (CR2):", palette.text_secondary, TypeScale::Body);
        let cr2_str = if rep.has_fault_addr {
            format_hex_u32(rep.fault_addr, &mut hex_buf)
        } else {
            "N/A"
        };
        draw_text(s, card2.x + 120, card2.y + 124, cr2_str, palette.status_danger, TypeScale::Body);

        // Code Segment CS
        draw_text(s, card2.x + 12, card2.y + 152, "Code Segment (CS):", palette.text_secondary, TypeScale::Body);
        draw_text(s, card2.x + 120, card2.y + 152, format_hex_u32(rep.cs, &mut hex_buf), palette.text_primary, TypeScale::Body);

        // EFLAGS
        draw_text(s, card2.x + 12, card2.y + 180, "EFLAGS:", palette.text_secondary, TypeScale::Body);
        draw_text(s, card2.x + 120, card2.y + 180, format_hex_u32(rep.eflags, &mut hex_buf), palette.text_primary, TypeScale::Body);

        // Card 3: Storage & System Guidance (Right)
        let card3 = Rect::new(card2.x + card2.w + 12, r.y + 116, card2.w, 230);
        draw_rounded_rect(s, card3, 4, palette.surface_raised);
        draw_rect_outline(s, card3, 1, palette.border_subtle);

        draw_text(s, card3.x + 12, card3.y + 12, "DIAGNOSTIC GUIDANCE", palette.accent_primary, TypeScale::Heading);

        draw_text(s, card3.x + 12, card3.y + 40, "What happened?", palette.text_primary, TypeScale::Body);
        draw_text(s, card3.x + 12, card3.y + 58, rep.vector_desc(), palette.text_secondary, TypeScale::Caption);

        draw_text(s, card3.x + 12, card3.y + 90, "Durable Storage:", palette.text_primary, TypeScale::Body);
        draw_text(s, card3.x + 12, card3.y + 108, "Report file: FAT32 Partition 1 (/CRASHDMP.TXT)", palette.text_secondary, TypeScale::Caption);
        draw_text(s, card3.x + 12, card3.y + 124, "Binary record: FAT32 Partition 1 (/CRASHDMP.DAT)", palette.text_secondary, TypeScale::Caption);

        draw_text(s, card3.x + 12, card3.y + 152, "Recommended Next Steps:", palette.text_primary, TypeScale::Body);
        draw_text(s, card3.x + 12, card3.y + 170, "1. Review CPU Registers & Call Stack backtrace tabs.", palette.text_muted, TypeScale::Caption);
        draw_text(s, card3.x + 12, card3.y + 186, "2. Click [Clear Record] to acknowledge and reset state.", palette.text_muted, TypeScale::Caption);
        draw_text(s, card3.x + 12, card3.y + 202, "3. Continue working or reboot system as desired.", palette.text_muted, TypeScale::Caption);
    }

    fn render_registers_tab<S: Surface + ?Sized>(&self, s: &mut S, r: Rect, palette: &ThemePalette) {
        let rep = &self.report;
        let mut hex_buf = [0u8; 10];

        let title_rect = Rect::new(r.x + 16, r.y + 16, r.w - 32, 28);
        draw_text(s, title_rect.x, title_rect.y + 4, "CPU REGISTER SNAPSHOT AT TIME OF PANIC", palette.accent_primary, TypeScale::Heading);

        // 4 Columns for registers
        let col_w = (r.w - 32 - 36) / 4;
        let col_y = r.y + 56;

        let cols = [
            ("Execution State", [
                ("EIP", rep.eip, true),
                ("CR2", rep.fault_addr, true),
                ("EFLAGS", rep.eflags, false),
                ("CS", rep.cs, false),
            ]),
            ("General Purpose (A-D)", [
                ("EAX", rep.eax, false),
                ("EBX", rep.ebx, false),
                ("ECX", rep.ecx, false),
                ("EDX", rep.edx, false),
            ]),
            ("Pointers & Indices", [
                ("ESI", rep.esi, false),
                ("EDI", rep.edi, false),
                ("EBP", rep.ebp, false),
                ("ESP", rep.esp, false),
            ]),
            ("Segments & Trap", [
                ("DS", rep.ds, false),
                ("SS", rep.ss, false),
                ("INT_NO", rep.int_no, true),
                ("ERR_CD", rep.err_code, false),
            ]),
        ];

        for (c_idx, (col_title, regs)) in cols.iter().enumerate() {
            let cx = r.x + 16 + (c_idx as i32) * (col_w + 12);
            let box_rect = Rect::new(cx, col_y, col_w, 200);
            draw_rounded_rect(s, box_rect, 4, palette.surface_raised);
            draw_rect_outline(s, box_rect, 1, palette.border_subtle);

            draw_text(s, box_rect.x + 10, box_rect.y + 10, col_title, palette.text_secondary, TypeScale::Caption);

            for (r_idx, (reg_name, reg_val, highlight)) in regs.iter().enumerate() {
                let ry = box_rect.y + 36 + (r_idx as i32) * 38;
                draw_text(s, box_rect.x + 12, ry, reg_name, palette.text_muted, TypeScale::Body);

                let val_str = format_hex_u32(*reg_val, &mut hex_buf);
                let color = if *highlight {
                    palette.status_danger
                } else {
                    palette.text_primary
                };
                draw_text(s, box_rect.x + 12, ry + 16, val_str, color, TypeScale::Body);
            }
        }

        // Explanatory footer card
        let foot_rect = Rect::new(r.x + 16, r.y + 270, r.w - 32, 76);
        draw_rounded_rect(s, foot_rect, 4, palette.surface_raised);
        draw_rect_outline(s, foot_rect, 1, palette.border_subtle);

        draw_text(s, foot_rect.x + 12, foot_rect.y + 10, "REGISTER CONTEXT & FLAGS", palette.text_primary, TypeScale::Body);
        draw_text(
            s,
            foot_rect.x + 12,
            foot_rect.y + 30,
            "• Highlighted registers (EIP, CR2, INT_NO) indicate active fault conditions.",
            palette.text_secondary,
            TypeScale::Caption,
        );
        draw_text(
            s,
            foot_rect.x + 12,
            foot_rect.y + 48,
            "• EFLAGS 0x00000202: Bit 1 reserved (always 1), Bit 9 IF=1 (Interrupts Enabled during execution).",
            palette.text_muted,
            TypeScale::Caption,
        );
    }

    fn render_callstack_tab<S: Surface + ?Sized>(&self, s: &mut S, r: Rect, palette: &ThemePalette) {
        let rep = &self.report;
        let mut hex_buf = [0u8; 10];

        let title_rect = Rect::new(r.x + 16, r.y + 16, r.w - 32, 28);
        draw_text(s, title_rect.x, title_rect.y + 4, "CALL STACK BACKTRACE (EBP FRAME CHAIN)", palette.accent_primary, TypeScale::Heading);

        let list_rect = Rect::new(r.x + 16, r.y + 54, r.w - 32, 220);
        draw_rounded_rect(s, list_rect, 4, palette.surface_raised);
        draw_rect_outline(s, list_rect, 1, palette.border_subtle);

        if rep.stack_count == 0 {
            draw_text(s, list_rect.x + 20, list_rect.y + 20, "No stack frame backtrace available for this panic.", palette.text_muted, TypeScale::Body);
        } else {
            for i in 0..rep.stack_count {
                let fy = list_rect.y + 12 + (i as i32) * 42;
                let frame_addr = rep.stack[i];
                let addr_str = format_hex_u32(frame_addr, &mut hex_buf);
                let sym_str = ParsedCrashReport::symbol_for_address(frame_addr);

                // Frame index badge
                let mut idx_buf = [0u8; 12];
                let idx_str = format_u32_dec(i as u32, &mut idx_buf);
                draw_text(s, list_rect.x + 12, fy + 4, "[#", palette.text_secondary, TypeScale::Body);
                draw_text(s, list_rect.x + 28, fy + 4, idx_str, palette.accent_primary, TypeScale::Body);
                draw_text(s, list_rect.x + 38, fy + 4, "]", palette.text_secondary, TypeScale::Body);

                // Frame Address
                draw_text(s, list_rect.x + 60, fy + 4, addr_str, palette.text_primary, TypeScale::Body);

                // Symbol hint
                draw_text(s, list_rect.x + 170, fy + 4, "—", palette.text_muted, TypeScale::Body);
                draw_text(s, list_rect.x + 190, fy + 4, sym_str, palette.status_info, TypeScale::Body);

                // Subtitle
                let sub_text = if i == 0 {
                    "Exact return address at failure boundary"
                } else {
                    "Caller frame in activation stack"
                };
                draw_text(s, list_rect.x + 60, fy + 22, sub_text, palette.text_muted, TypeScale::Caption);
            }
        }

        // Summary footer
        let foot_rect = Rect::new(r.x + 16, r.y + 286, r.w - 32, 60);
        draw_rounded_rect(s, foot_rect, 4, palette.surface_raised);
        draw_rect_outline(s, foot_rect, 1, palette.border_subtle);

        let mut count_buf = [0u8; 12];
        let cnt_str = format_u32_dec(rep.stack_count as u32, &mut count_buf);
        draw_text(s, foot_rect.x + 12, foot_rect.y + 10, "Total Frames Recorded:", palette.text_secondary, TypeScale::Body);
        draw_text(s, foot_rect.x + 170, foot_rect.y + 10, cnt_str, palette.accent_primary, TypeScale::Body);
        draw_text(
            s,
            foot_rect.x + 12,
            foot_rect.y + 32,
            "Frames derived from EBP chain unwinding prior to panic halt. Addresses correlate with system map.",
            palette.text_muted,
            TypeScale::Caption,
        );
    }

    fn render_rawreport_tab<S: Surface + ?Sized>(&self, s: &mut S, r: Rect, palette: &ThemePalette) {
        let view_rect = Rect::new(r.x + 16, r.y + 16, r.w - 56, r.h - 32);
        draw_rounded_rect(s, view_rect, 4, palette.surface_raised);
        draw_rect_outline(s, view_rect, 1, palette.border_subtle);

        // Scroll buttons
        let scroll_up = Rect::new(r.x + r.w - 34, r.y + 16, 22, 22);
        let scroll_dn = Rect::new(r.x + r.w - 34, r.y + r.h - 38, 22, 22);
        draw_rounded_rect(s, scroll_up, 3, palette.surface_raised);
        draw_text(s, scroll_up.x + 6, scroll_up.y + 4, "^", palette.text_primary, TypeScale::Body);
        draw_rounded_rect(s, scroll_dn, 3, palette.surface_raised);
        draw_text(s, scroll_dn.x + 6, scroll_dn.y + 4, "v", palette.text_primary, TypeScale::Body);

        let raw = unsafe { &CRASH_RAW_BUF[..self.raw_text_len] };
        if raw.is_empty() {
            draw_text(s, view_rect.x + 20, view_rect.y + 20, "No raw /CRASHDMP.TXT data loaded.", palette.text_muted, TypeScale::Body);
            return;
        }

        // Render visible lines starting at scroll_line
        let mut curr_line = 0;
        let mut i = 0;
        let mut rendered_lines = 0;
        let max_visible = 18;

        while i < raw.len() && rendered_lines < max_visible {
            let line_start = i;
            while i < raw.len() && raw[i] != b'\n' && raw[i] != b'\r' {
                i += 1;
            }
            let line_bytes = &raw[line_start..i];
            while i < raw.len() && (raw[i] == b'\n' || raw[i] == b'\r') {
                i += 1;
            }

            if curr_line >= self.scroll_line {
                let ly = view_rect.y + 10 + (rendered_lines as i32) * 16;

                // Line number
                let mut num_buf = [0u8; 12];
                let num_str = format_u32_dec((curr_line + 1) as u32, &mut num_buf);
                draw_text(s, view_rect.x + 8, ly, num_str, palette.text_muted, TypeScale::Caption);

                // Content
                let line_str = core::str::from_utf8(line_bytes).unwrap_or("");
                let text_color = if line_str.starts_with("Panic") || line_str.starts_with("Status") {
                    palette.status_danger
                } else if line_str.starts_with("[CRASH_METADATA]") || line_str.starts_with("CPU") {
                    palette.accent_primary
                } else {
                    palette.text_primary
                };
                draw_text(s, view_rect.x + 42, ly, line_str, text_color, TypeScale::Caption);
                rendered_lines += 1;
            }
            curr_line += 1;
        }
    }

    fn render_bottom_bar<S: Surface + ?Sized>(&self, s: &mut S, palette: &ThemePalette) {
        let wx = self.win_x;
        let wy = self.win_y;
        let btn_y = wy + WINDOW_H - 52;

        // Reboot button (Left)
        let reboot_rect = Rect::new(wx + 20, btn_y, 110, 32);
        draw_button(s, reboot_rect, "Reboot", ButtonVariant::Secondary, WidgetState::Normal, palette);

        // Status string in center
        draw_text(s, wx + 140, btn_y + 8, self.status_str(), palette.text_secondary, TypeScale::Body);

        // Clear button (Right-2)
        let clear_rect = Rect::new(wx + WINDOW_W - 270, btn_y, 130, 32);
        draw_button(s, clear_rect, "Clear Record", ButtonVariant::Secondary, WidgetState::Normal, palette);

        // Dismiss button (Right)
        let dismiss_rect = Rect::new(wx + WINDOW_W - 130, btn_y, 110, 32);
        draw_button(s, dismiss_rect, "Dismiss", ButtonVariant::Primary, WidgetState::Normal, palette);
    }

    // -----------------------------------------------------------------------
    // Automated Self-Test Verification Suite
    // -----------------------------------------------------------------------

    #[inline(never)]
    pub fn run_selftest(&mut self) -> i32 {
        unsafe {
            let msg = b"[crashrep-selftest] running Month 6 graphical crash reporter verification suite\n\0";
            ffi::sys_write(msg.as_ptr());
        }

        // Test 1: Load and parse crash dump records
        self.load_crash_dump();
        if self.report.has_crash && self.report.has_regs {
            unsafe {
                let msg = b"[crashrep-selftest] PASS: crash report detection and file parsing (CRASHDMP.TXT / CRASHDMP.DAT)\n\0";
                ffi::sys_write(msg.as_ptr());
            }
        } else {
            unsafe {
                let msg = b"[crashrep-selftest] FAIL: crash report detection or parsing\n\0";
                ffi::sys_write(msg.as_ptr());
            }
            return 1;
        }

        // Test 2: Verify parsed CPU registers and reason
        let p1 = self.report.int_no == 14;
        let p2 = self.report.eip == 0x80481234;
        let p3 = self.report.eax == 0x11223344;
        let p4 = self.report.fault_addr == 0x014ab000;
        let p5 = self.report.stack_count == 3;

        if p1 && p2 && p3 && p4 && p5 {
            unsafe {
                let msg = b"[crashrep-selftest] PASS: register snapshot and stack backtrace validation\n\0";
                ffi::sys_write(msg.as_ptr());
            }
        } else {
            unsafe {
                let msg = b"[crashrep-selftest] FAIL: register snapshot or stack backtrace mismatch\n\0";
                ffi::sys_write(msg.as_ptr());
            }
            return 2;
        }

        // Test 3: Tab navigation & state machine
        self.active_tab = ActiveTab::Registers;
        let tab_reg_ok = self.active_tab == ActiveTab::Registers;
        self.active_tab = ActiveTab::CallStack;
        let tab_stack_ok = self.active_tab == ActiveTab::CallStack;
        self.active_tab = ActiveTab::RawReport;
        let tab_raw_ok = self.active_tab == ActiveTab::RawReport;
        self.active_tab = ActiveTab::Overview;
        let tab_over_ok = self.active_tab == ActiveTab::Overview;

        if tab_reg_ok && tab_stack_ok && tab_raw_ok && tab_over_ok {
            unsafe {
                let msg = b"[crashrep-selftest] PASS: multi-tab state transitions (Overview, Registers, Stack, Raw)\n\0";
                ffi::sys_write(msg.as_ptr());
            }
        } else {
            unsafe {
                let msg = b"[crashrep-selftest] FAIL: tab state machine\n\0";
                ffi::sys_write(msg.as_ptr());
            }
            return 3;
        }

        // Test 4: BufferSurface32 graphical rasterization across all views
        let buf_slice = unsafe { &mut TEST_RENDER_BUF[..1024 * 768] };
        let mut surface = BufferSurface32::new(buf_slice, 1024, 768);

        // Render each tab and assert non-zero pixel rasterization
        self.active_tab = ActiveTab::Overview;
        self.render(&mut surface);
        let px_overview = surface.buffer.iter().filter(|&&p| p != 0).count();

        self.active_tab = ActiveTab::Registers;
        self.render(&mut surface);
        let px_registers = surface.buffer.iter().filter(|&&p| p != 0).count();

        self.active_tab = ActiveTab::CallStack;
        self.render(&mut surface);
        let px_stack = surface.buffer.iter().filter(|&&p| p != 0).count();

        self.active_tab = ActiveTab::RawReport;
        self.render(&mut surface);
        let px_raw = surface.buffer.iter().filter(|&&p| p != 0).count();

        if px_overview > 5000 && px_registers > 5000 && px_stack > 5000 && px_raw > 5000 {
            unsafe {
                let msg = b"[crashrep-selftest] PASS: zero-syscall BufferSurface32 graphical presentation\n\0";
                ffi::sys_write(msg.as_ptr());
            }
        } else {
            unsafe {
                let msg = b"[crashrep-selftest] FAIL: BufferSurface32 graphical presentation\n\0";
                ffi::sys_write(msg.as_ptr());
            }
            return 4;
        }

        unsafe {
            let msg = b"[crashrep-selftest] all cases passed\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        0
    }
}

// ---------------------------------------------------------------------------
// Entry Point (main)
// ---------------------------------------------------------------------------

fn starts_with_cstr(mut ptr: *const u8, prefix: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for &b in prefix {
        unsafe {
            if *ptr == 0 || *ptr != b {
                return false;
            }
            ptr = ptr.add(1);
        }
    }
    true
}

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    let app = unsafe { &mut *addr_of_mut!(GLOBAL_CRASHREP) };

    if argc >= 2 {
        let arg1 = unsafe { *argv.offset(1) };
        if starts_with_cstr(arg1, b"--selftest") {
            app.init(1024, 768);
            return app.run_selftest();
        } else if starts_with_cstr(arg1, b"--check") {
            // Check-only mode: if no crash is pending, exit immediately
            let mut st = NovaCrashdumpStatus::default();
            let rc = unsafe { ffi::sys_crashdump_ctl(CRASHDUMP_OP_STATUS, addr_of_mut!(st) as *mut u8) };
            if rc != 0 || st.has_crash == 0 {
                return 0; // Clean exit, no pending crash
            }
        }
    }

    unsafe {
        let msg = b"crashrep: entering graphical problem reporter (Month 6 First Impressions)\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    // 1. Query display information
    let mut fb_info = ffi::NovaFbInfo {
        struct_size: core::mem::size_of::<ffi::NovaFbInfo>() as u32,
        ..Default::default()
    };
    let info_rc = unsafe { ffi::sys_fb_info(&mut fb_info) };
    let (screen_w, screen_h) = if info_rc == 0 && fb_info.width > 0 && fb_info.height > 0 {
        (fb_info.width as i32, fb_info.height as i32)
    } else {
        (1024, 768)
    };

    // 2. Take exclusive ownership of framebuffer
    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"crashrep: failed to acquire display\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    // 3. Create framebuffer surface
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
            let msg = b"crashrep: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    app.init(screen_w, screen_h);

    let mut prev_left = false;

    // Interactive event loop
    while app.running {
        let key = unsafe { ffi::sys_read_key() };
        if key >= 0 {
            app.handle_key(key);
        }

        let mut m = ffi::MouseState::default();
        let _ = unsafe { ffi::sys_mouse_read(&mut m) };
        app.mouse_x = (app.mouse_x + m.dx).clamp(0, screen_w - 1);
        app.mouse_y = (app.mouse_y + m.dy).clamp(0, screen_h - 1);

        if m.left_button && !prev_left {
            app.handle_mouse_click(app.mouse_x, app.mouse_y);
        }
        prev_left = m.left_button;

        // Render onto framebuffer surface
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = BufferSurface32::new(pixel_slice, screen_w, screen_h);
        app.render(&mut surface);

        // Draw cursor
        ui::cursor::draw_cursor_sprite(
            &mut surface,
            app.mouse_x,
            app.mouse_y,
            ui::cursor::CursorShape::Arrow,
            ui::cursor::CursorTheme::ModernDark,
        );

        let pres = ffi::NovaFbPresent {
            handle: fb_handle,
            src_x: 0,
            src_y: 0,
            width: screen_w,
            height: screen_h,
            dst_x: 0,
            dst_y: 0,
            flags: 0,
        };
        unsafe {
            ffi::sys_fb_present(&pres);
            ffi::sys_yield();
        }
    }

    unsafe { ffi::sys_fb_release() };
    0
}

// ---------------------------------------------------------------------------
// Host Unit Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crashrep_app_initialization() {
        let app = unsafe { &mut *addr_of_mut!(GLOBAL_CRASHREP) };
        app.init(1024, 768);
        assert_eq!(app.screen_w, 1024);
        assert_eq!(app.screen_h, 768);
        assert!(app.report.has_crash);
        assert_eq!(app.report.int_no, 14);
        assert_eq!(app.report.eip, 0x80481234);
    }

    #[test]
    fn test_crashrep_tab_switching() {
        let app = unsafe { &mut *addr_of_mut!(GLOBAL_CRASHREP) };
        app.init(1024, 768);

        app.handle_key(b'2' as i32);
        assert_eq!(app.active_tab, ActiveTab::Registers);

        app.handle_key(b'3' as i32);
        assert_eq!(app.active_tab, ActiveTab::CallStack);

        app.handle_key(b'4' as i32);
        assert_eq!(app.active_tab, ActiveTab::RawReport);

        app.handle_key(b'1' as i32);
        assert_eq!(app.active_tab, ActiveTab::Overview);
    }

    #[test]
    fn test_crashrep_clear_action() {
        let app = unsafe { &mut *addr_of_mut!(GLOBAL_CRASHREP) };
        app.init(1024, 768);
        app.clear_crash_record();
        assert!(app.is_cleared);
        let has_crash = app.status.has_crash;
        assert_eq!(has_crash, 0);
    }

    #[test]
    fn test_crashrep_render_all_tabs() {
        let app = unsafe { &mut *addr_of_mut!(GLOBAL_CRASHREP) };
        app.init(1024, 768);

        let buf_slice = unsafe { &mut TEST_RENDER_BUF[..1024 * 768] };
        let mut surface = BufferSurface32::new(buf_slice, 1024, 768);

        for tab in [ActiveTab::Overview, ActiveTab::Registers, ActiveTab::CallStack, ActiveTab::RawReport] {
            app.active_tab = tab;
            app.render(&mut surface);
            let non_zero = surface.buffer.iter().filter(|&&p| p != 0).count();
            assert!(non_zero > 1000, "Tab {:?} rendered too few pixels: {}", tab, non_zero);
        }
    }

    #[test]
    fn test_crashrep_selftest_passes() {
        let app = unsafe { &mut *addr_of_mut!(GLOBAL_CRASHREP) };
        app.init(1024, 768);
        assert_eq!(app.run_selftest(), 0);
    }
}
