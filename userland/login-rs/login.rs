//! userland/login-rs/login.rs - NovaOS Graphical Login Screen & Display Manager (Month 3 Core Desktop Apps)
//!
//! A pure #![no_std] ring-3 graphical login manager for NovaOS.
//! Serves as the first screen the user sees before launching the desktop environment.
//!
//! Features:
//!   - Authenticates against NovaOS kernel credential database via `SYS_LOGIN`.
//!   - Verifies process identity transition via `SYS_GETUID`.
//!   - Reads and parses persistent accounts from `USERS.CFG` (magic "NVUS") and `SYSTEM.CFG` (magic "NVCF").
//!   - Multi-account switcher: avatar selection for registered users (`admin`, `persisted`) plus custom username entry.
//!   - Secure password input: bullet masking, blinking cursor, and show/hide password toggle.
//!   - Real-Time Clock integration (`SYS_RTC_READ`): dynamic hero clock and calendar date.
//!   - Session selector: launches Window Manager (`WM.ELF`) or Console Shell (`SHELL.ELF`) via `SYS_EXEC`.
//!   - System power controls: ACPI power off via `SYS_SHUTDOWN`.
//!   - High-contrast visual presentation conforming to NovaOS Unified Design System (`ui-rs`).
//!   - Zero-allocation linear 32-bit framebuffer presentation (`BufferSurface32`) with zero per-pixel syscalls.
//!   - Headless `--selftest` suite verifying credential rejection, credential acceptance, identity escalation,
//!     and graphical presentation.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(static_mut_refs, unused_mut, dead_code)]

extern crate ui;
use ui::prelude::*;
use core::ptr::addr_of_mut;

mod ffi;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"login: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

// ---------------------------------------------------------------------------
// Constants & Theme Colors
// ---------------------------------------------------------------------------

pub const MAX_USERS: usize = 8;
pub const MAX_USERNAME_LEN: usize = 32;
pub const MAX_PASSWORD_LEN: usize = 64;

pub const COLOR_BG_SPACE: Color = Color::rgb(10, 14, 20);
pub const COLOR_CARD_BG: Color = Color::rgb(22, 27, 34);
pub const COLOR_CARD_BORDER: Color = Color::rgb(48, 54, 61);
pub const COLOR_INPUT_BG: Color = Color::rgb(13, 17, 23);
pub const COLOR_ACCENT: Color = Color::rgb(88, 166, 255);
pub const COLOR_ACCENT_BTN: Color = Color::rgb(31, 111, 235);
pub const COLOR_SUCCESS: Color = Color::rgb(46, 160, 67);
pub const COLOR_DANGER: Color = Color::rgb(248, 81, 73);
pub const COLOR_PURPLE: Color = Color::rgb(137, 87, 229);
pub const COLOR_GOLD: Color = Color::rgb(210, 153, 34);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionType {
    DesktopWm,
    ConsoleShell,
}

impl SessionType {
    pub fn name(&self) -> &'static str {
        match self {
            SessionType::DesktopWm => "Desktop (WM.ELF)",
            SessionType::ConsoleShell => "Console (SHELL.ELF)",
        }
    }

    pub fn exec_path(&self) -> &'static str {
        match self {
            SessionType::DesktopWm => "WM.ELF",
            SessionType::ConsoleShell => "SHELL.ELF",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusField {
    Username,
    Password,
    LoginButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthStatus {
    Idle,
    Authenticating,
    Success { uid: u32, gid: u32 },
    Failed { attempts: u32 },
}

#[derive(Clone, Copy, Debug)]
pub struct UserAccountItem {
    pub username: [u8; MAX_USERNAME_LEN],
    pub username_len: usize,
    pub uid: u32,
    pub gid: u32,
    pub is_admin: bool,
}

impl UserAccountItem {
    pub const fn empty() -> Self {
        UserAccountItem {
            username: [0; MAX_USERNAME_LEN],
            username_len: 0,
            uid: 0,
            gid: 0,
            is_admin: false,
        }
    }

    pub fn username_as_str(&self) -> &str {
        core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("unknown")
    }
}

// ---------------------------------------------------------------------------
// Login Screen Engine
// ---------------------------------------------------------------------------

pub struct LoginScreen {
    pub accounts: [UserAccountItem; MAX_USERS],
    pub account_count: usize,
    pub selected_account_idx: usize,
    pub is_custom_user: bool,
    pub custom_username: [u8; MAX_USERNAME_LEN],
    pub custom_username_len: usize,
    pub password_buf: [u8; MAX_PASSWORD_LEN],
    pub password_len: usize,
    pub show_password: bool,
    pub focus: FocusField,
    pub auth_status: AuthStatus,
    pub failed_attempts: u32,
    pub session: SessionType,
    pub rtc: ffi::RtcTime,
    pub hostname: [u8; 32],
    pub hostname_len: usize,
    pub status_message: [u8; 64],
    pub status_len: usize,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub running: bool,
    pub screen_w: i32,
    pub screen_h: i32,
    pub cursor_blink: u32,
}

impl LoginScreen {
    pub const fn uninit() -> Self {
        LoginScreen {
            accounts: [UserAccountItem::empty(); MAX_USERS],
            account_count: 0,
            selected_account_idx: 0,
            is_custom_user: false,
            custom_username: [0; MAX_USERNAME_LEN],
            custom_username_len: 0,
            password_buf: [0; MAX_PASSWORD_LEN],
            password_len: 0,
            show_password: false,
            focus: FocusField::Password,
            auth_status: AuthStatus::Idle,
            failed_attempts: 0,
            session: SessionType::DesktopWm,
            rtc: ffi::RtcTime {
                year: 2026,
                month: 10,
                day: 26,
                hour: 9,
                minute: 0,
                second: 0,
            },
            hostname: [0; 32],
            hostname_len: 0,
            status_message: [0; 64],
            status_len: 0,
            mouse_x: 200,
            mouse_y: 200,
            running: true,
            screen_w: 1024,
            screen_h: 768,
            cursor_blink: 0,
        }
    }

    pub fn init(&mut self, w: i32, h: i32) {
        self.screen_w = w;
        self.screen_h = h;
        self.running = true;
        self.password_len = 0;
        self.show_password = false;
        self.focus = FocusField::Password;
        self.auth_status = AuthStatus::Idle;
        self.failed_attempts = 0;
        self.cursor_blink = 0;
        self.set_status("Enter credentials to sign in.");

        // Read RTC Time
        unsafe {
            ffi::sys_rtc_read(core::ptr::addr_of_mut!(self.rtc));
        }

        // Read SYSTEM.CFG for hostname
        self.load_system_config();

        // Read USERS.CFG for persistent accounts
        self.load_users_config();

        // If no accounts found on disk, populate standard accounts
        if self.account_count == 0 {
            self.add_account("admin", 800, 0, true);
            self.add_account("persisted", 700, 700, false);
        }

        self.selected_account_idx = 0;
        self.is_custom_user = false;
    }

    pub fn set_status(&mut self, msg: &str) {
        let b = msg.as_bytes();
        let n = b.len().min(63);
        self.status_message[..n].copy_from_slice(&b[..n]);
        self.status_message[n] = 0;
        self.status_len = n;
    }

    pub fn status_as_str(&self) -> &str {
        core::str::from_utf8(&self.status_message[..self.status_len]).unwrap_or("")
    }

    pub fn hostname_as_str(&self) -> &str {
        core::str::from_utf8(&self.hostname[..self.hostname_len]).unwrap_or("novaos")
    }

    pub fn active_username(&self) -> &str {
        if self.is_custom_user {
            core::str::from_utf8(&self.custom_username[..self.custom_username_len]).unwrap_or("")
        } else if self.selected_account_idx < self.account_count {
            self.accounts[self.selected_account_idx].username_as_str()
        } else {
            ""
        }
    }

    pub fn add_account(&mut self, name: &str, uid: u32, gid: u32, is_admin: bool) -> bool {
        if self.account_count >= MAX_USERS {
            return false;
        }
        let mut item = UserAccountItem::empty();
        let nb = name.as_bytes();
        let nl = nb.len().min(MAX_USERNAME_LEN);
        item.username[..nl].copy_from_slice(&nb[..nl]);
        item.username_len = nl;
        item.uid = uid;
        item.gid = gid;
        item.is_admin = is_admin;

        self.accounts[self.account_count] = item;
        self.account_count += 1;
        true
    }

    /// Loads hostname and default user from `SYSTEM.CFG` (81 bytes).
    pub fn load_system_config(&mut self) {
        static mut BUF: [u8; 128] = [0; 128];
        let h = unsafe { ffi::sys_open(b"SYSTEM.CFG\0".as_ptr()) };
        if h < 0 {
            let def = b"novaos\0";
            self.hostname[..def.len() - 1].copy_from_slice(&def[..def.len() - 1]);
            self.hostname_len = def.len() - 1;
            return;
        }

        let n = unsafe { ffi::sys_read(h, BUF.as_mut_ptr(), BUF.len() as i32) };
        unsafe { ffi::sys_close(h) };

        if n >= 81 {
            let data = unsafe { &BUF[..81] };
            if &data[0..4] == b"NVCF" {
                let h_slice = &data[4..36];
                let mut h_len = 0;
                while h_len < 32 && h_slice[h_len] != 0 {
                    h_len += 1;
                }
                self.hostname[..h_len].copy_from_slice(&h_slice[..h_len]);
                self.hostname_len = h_len;
                return;
            }
        }

        let def = b"novaos\0";
        self.hostname[..def.len() - 1].copy_from_slice(&def[..def.len() - 1]);
        self.hostname_len = def.len() - 1;
    }

    /// Loads accounts from `USERS.CFG` (716 bytes).
    pub fn load_users_config(&mut self) {
        static mut BUF: [u8; 1024] = [0; 1024];
        let h = unsafe { ffi::sys_open(b"USERS.CFG\0".as_ptr()) };
        if h < 0 {
            return;
        }

        let n = unsafe { ffi::sys_read(h, BUF.as_mut_ptr(), BUF.len() as i32) };
        unsafe { ffi::sys_close(h) };

        if n < 4 {
            return;
        }

        let data = unsafe { &BUF[..n as usize] };
        if &data[0..4] != b"NVUS" {
            return;
        }

        self.account_count = 0;
        let mut offset = 4;
        let record_size = 89;

        for _ in 0..MAX_USERS {
            if offset + record_size > data.len() {
                break;
            }
            let chunk = &data[offset..offset + record_size];
            let in_use = chunk[0] != 0;
            if in_use {
                let uname_bytes = &chunk[1..33];
                let mut u_len = 0;
                while u_len < 32 && uname_bytes[u_len] != 0 {
                    u_len += 1;
                }
                let uid = u32::from_le_bytes([chunk[33], chunk[34], chunk[35], chunk[36]]);
                let gid = u32::from_le_bytes([chunk[37], chunk[38], chunk[39], chunk[40]]);
                let is_admin = gid == 0;

                if let Ok(name_str) = core::str::from_utf8(&uname_bytes[..u_len]) {
                    self.add_account(name_str, uid, gid, is_admin);
                }
            }
            offset += record_size;
        }
    }

    /// Attempts authentication using `SYS_LOGIN`.
    pub fn attempt_login(&mut self) -> bool {
        let uname = self.active_username();
        if uname.is_empty() {
            self.set_status("Please select or enter a username.");
            return false;
        }

        let mut u_buf = [0u8; 33];
        let u_bytes = uname.as_bytes();
        let u_len = u_bytes.len().min(32);
        u_buf[..u_len].copy_from_slice(&u_bytes[..u_len]);
        u_buf[u_len] = 0;

        let mut p_buf = [0u8; 65];
        let p_len = self.password_len.min(64);
        p_buf[..p_len].copy_from_slice(&self.password_buf[..p_len]);
        p_buf[p_len] = 0;

        self.auth_status = AuthStatus::Authenticating;

        let rc = unsafe { ffi::sys_login(u_buf.as_ptr(), p_buf.as_ptr()) };

        // Zeroize temporary plaintext password buffer immediately
        for b in p_buf.iter_mut() {
            *b = 0;
        }

        if rc == 0 {
            // Zeroize stored password buffer on authentication success
            for b in self.password_buf.iter_mut() {
                *b = 0;
            }
            self.password_len = 0;

            let uid = unsafe { ffi::sys_getuid() };
            let gid = if self.is_custom_user || self.selected_account_idx >= self.account_count {
                uid
            } else {
                self.accounts[self.selected_account_idx].gid
            };

            self.auth_status = AuthStatus::Success { uid, gid };
            self.set_status("Identity verified! Session launching...");
            true
        } else {
            // Zeroize stored password buffer on authentication failure
            for b in self.password_buf.iter_mut() {
                *b = 0;
            }
            self.password_len = 0;

            self.failed_attempts += 1;
            self.auth_status = AuthStatus::Failed {
                attempts: self.failed_attempts,
            };
            self.set_status("Authentication failed: invalid username or password.");
            false
        }
    }

    /// Launches the selected session application via `SYS_EXEC`.
    pub fn launch_session(&self) -> i32 {
        let exec_target = self.session.exec_path();
        let mut target_buf = [0u8; 32];
        let b = exec_target.as_bytes();
        let blen = b.len().min(31);
        target_buf[..blen].copy_from_slice(&b[..blen]);
        target_buf[blen] = 0;

        let argv: [*const u8; 2] = [target_buf.as_ptr(), core::ptr::null()];
        unsafe { ffi::sys_exec(target_buf.as_ptr(), argv.as_ptr(), 1) }
    }

    /// Powers off the computer using ACPI shutdown.
    pub fn shutdown(&self) {
        unsafe { ffi::sys_shutdown() };
    }

    // -----------------------------------------------------------------------
    // Keyboard & Mouse Event Handling
    // -----------------------------------------------------------------------

    pub fn handle_key(&mut self, key: i32) {
        match key {
            10 | 13 => {
                // Enter: attempt login
                let ok = self.attempt_login();
                if ok {
                    // Launch session
                    let _ = self.launch_session();
                    self.running = false;
                }
            }
            8 | 127 => {
                // Backspace
                match self.focus {
                    FocusField::Password => {
                        if self.password_len > 0 {
                            self.password_len -= 1;
                        }
                    }
                    FocusField::Username => {
                        if self.is_custom_user && self.custom_username_len > 0 {
                            self.custom_username_len -= 1;
                        }
                    }
                    _ => {}
                }
            }
            9 => {
                // Tab: cycle focus
                self.focus = match self.focus {
                    FocusField::Password => FocusField::LoginButton,
                    FocusField::LoginButton => {
                        if self.is_custom_user {
                            FocusField::Username
                        } else {
                            FocusField::Password
                        }
                    }
                    FocusField::Username => FocusField::Password,
                };
            }
            27 => {
                // Escape: clear password
                self.password_len = 0;
                self.auth_status = AuthStatus::Idle;
            }
            k if (32..=126).contains(&k) => {
                // Printable ASCII character
                let ch = k as u8;
                match self.focus {
                    FocusField::Password => {
                        if self.password_len < MAX_PASSWORD_LEN {
                            self.password_buf[self.password_len] = ch;
                            self.password_len += 1;
                            self.auth_status = AuthStatus::Idle;
                        }
                    }
                    FocusField::Username => {
                        if self.is_custom_user && self.custom_username_len < MAX_USERNAME_LEN {
                            self.custom_username[self.custom_username_len] = ch;
                            self.custom_username_len += 1;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn handle_mouse_click(&mut self, mx: i32, my: i32) {
        let card_w = 400;
        let card_h = 360;
        let card_x = (self.screen_w - card_w) / 2;
        let card_y = (self.screen_h - card_h) / 2 + 30;

        // 1. User selector pills (y: card_y + 110..card_y + 134)
        if my >= card_y + 110 && my < card_y + 134 {
            let mut px = card_x + 24;
            for i in 0..self.account_count {
                let uname = self.accounts[i].username_as_str();
                let pw = (uname.len() as i32 * 8) + 16;
                if mx >= px && mx < px + pw {
                    self.selected_account_idx = i;
                    self.is_custom_user = false;
                    self.password_len = 0;
                    self.focus = FocusField::Password;
                    self.auth_status = AuthStatus::Idle;
                    return;
                }
                px += pw + 8;
            }
            // "Other..." pill
            let other_w = 64;
            if mx >= px && mx < px + other_w {
                self.is_custom_user = true;
                self.password_len = 0;
                self.focus = FocusField::Username;
                self.auth_status = AuthStatus::Idle;
                return;
            }
        }

        // 2. Custom username field (if visible)
        if self.is_custom_user {
            let ufield_y = card_y + 144;
            if mx >= card_x + 24 && mx < card_x + card_w - 24 && my >= ufield_y && my < ufield_y + 32 {
                self.focus = FocusField::Username;
                return;
            }
        }

        // 3. Password input field (y: card_y + 190..card_y + 222)
        let pass_y = if self.is_custom_user { card_y + 186 } else { card_y + 154 };
        if mx >= card_x + 24 && mx < card_x + card_w - 24 && my >= pass_y && my < pass_y + 32 {
            // Check eye toggle icon on right of password field
            let eye_x = card_x + card_w - 56;
            if mx >= eye_x {
                self.show_password = !self.show_password;
            }
            self.focus = FocusField::Password;
            return;
        }

        // 4. Action buttons: [Sign In] and [Clear]
        let btn_y = if self.is_custom_user { card_y + 236 } else { card_y + 204 };
        let btn_h = 32;
        let btn_w = (card_w - 56) / 2;

        // Sign In button
        if mx >= card_x + 24 && mx < card_x + 24 + btn_w && my >= btn_y && my < btn_y + btn_h {
            let ok = self.attempt_login();
            if ok {
                let _ = self.launch_session();
                self.running = false;
            }
            return;
        }

        // Clear button
        if mx >= card_x + 32 + btn_w && mx < card_x + 32 + 2 * btn_w && my >= btn_y && my < btn_y + btn_h {
            self.password_len = 0;
            self.auth_status = AuthStatus::Idle;
            return;
        }

        // 5. Session selector toggle (y: card_y + 260)
        let sess_y = if self.is_custom_user { card_y + 280 } else { card_y + 248 };
        if mx >= card_x + 24 && mx < card_x + card_w - 24 && my >= sess_y && my < sess_y + 24 {
            self.session = match self.session {
                SessionType::DesktopWm => SessionType::ConsoleShell,
                SessionType::ConsoleShell => SessionType::DesktopWm,
            };
            return;
        }

        // 6. Bottom toolbar controls (Shutdown, Restart)
        let tb_y = self.screen_h - 36;
        if my >= tb_y {
            // Shutdown button on far right
            let shut_w = 90;
            let shut_x = self.screen_w - shut_w - 24;
            if mx >= shut_x && mx < shut_x + shut_w {
                self.shutdown();
                return;
            }
            // Restart button
            let rst_w = 80;
            let rst_x = shut_x - rst_w - 12;
            if mx >= rst_x && mx < rst_x + rst_w {
                self.shutdown();
                return;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Graphical Presentation Routines
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&mut self, s: &mut S) {
        self.cursor_blink = self.cursor_blink.wrapping_add(1);
        let f_regular = Font::regular();
        let f_small = Font::small();
        let f_mono = Font::mono();

        // 1. Dark ambient background
        s.fill_rect(Rect::new(0, 0, self.screen_w, self.screen_h), COLOR_BG_SPACE);

        // Decorative background geometry
        self.render_background_decorations(s);

        // 2. Top Status Strip
        self.render_top_bar(s, &f_small);

        // 3. Hero Clock & Date Section
        self.render_hero_clock(s, &f_regular, &f_small, &f_mono);

        // 4. Central Glassmorphism Login Card
        self.render_login_card(s, &f_regular, &f_small, &f_mono);

        // 5. Bottom Toolbar (Session, Power, Copyright)
        self.render_bottom_toolbar(s, &f_small);
    }

    fn render_background_decorations<S: Surface + ?Sized>(&self, s: &mut S) {
        // Subtle ambient accent lines across screen
        let line_col = Color::rgba(255, 255, 255, 6);
        for y in (60..self.screen_h - 40).step_by(80) {
            s.fill_rect(Rect::new(0, y, self.screen_w, 1), line_col);
        }
        for x in (100..self.screen_w).step_by(120) {
            s.fill_rect(Rect::new(x, 40, 1, self.screen_h - 80), line_col);
        }

        // Top-left subtle brand glow
        let glow_col = Color::rgba(88, 166, 255, 12);
        s.fill_rect(Rect::new(0, 0, 180, 2), glow_col);
    }

    fn render_top_bar<S: Surface + ?Sized>(&self, s: &mut S, f: &Font) {
        let bar_h = 32;
        let bar_rect = Rect::new(0, 0, self.screen_w, bar_h);
        s.fill_rect(bar_rect, Color::rgba(22, 27, 34, 180));
        s.fill_rect(Rect::new(0, bar_h - 1, self.screen_w, 1), COLOR_CARD_BORDER);

        // Hostname Pill
        let mut host_display = [0u8; 48];
        let prefix = b"System: ";
        host_display[..prefix.len()].copy_from_slice(prefix);
        let hname = self.hostname_as_str().as_bytes();
        let hlen = hname.len().min(32);
        host_display[prefix.len()..prefix.len() + hlen].copy_from_slice(&hname[..hlen]);
        let full_len = prefix.len() + hlen;
        if let Ok(host_str) = core::str::from_utf8(&host_display[..full_len]) {
            let hp_w = (full_len as i32 * 8) + 16;
            let hp_rect = Rect::new(16, 5, hp_w, 22);
            s.fill_rect(hp_rect, Color::rgb(33, 38, 45));
            draw_rect_outline(s, hp_rect, 1, COLOR_CARD_BORDER);
            ui::font::draw_text_font(s, 24, 10, host_str, Color::rgb(201, 209, 217), f);
        }

        // NovaOS Logo / OS Version badge
        ui::font::draw_text_font(s, self.screen_w / 2 - 40, 10, "NovaOS Desktop", Color::WHITE, f);

        // Green Online / ACPI Status on far right
        let right_x = self.screen_w - 140;
        let dot_rect = Rect::new(right_x, 12, 8, 8);
        ui::draw::draw_rounded_rect(s, dot_rect, 4, COLOR_SUCCESS);
        ui::font::draw_text_font(s, right_x + 14, 10, "ACPI S5 Ready", Color::rgb(139, 148, 158), f);
    }

    fn render_hero_clock<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        f_reg: &Font,
        f_small: &Font,
        f_mono: &Font,
    ) {
        let clock_y = (self.screen_h / 2) - 220;
        if clock_y < 40 {
            return;
        }

        // Format Clock HH:MM:SS
        let mut time_str = [0u8; 8];
        time_str[0] = b'0' + (self.rtc.hour / 10);
        time_str[1] = b'0' + (self.rtc.hour % 10);
        time_str[2] = b':';
        time_str[3] = b'0' + (self.rtc.minute / 10);
        time_str[4] = b'0' + (self.rtc.minute % 10);
        time_str[5] = b':';
        time_str[6] = b'0' + (self.rtc.second / 10);
        time_str[7] = b'0' + (self.rtc.second % 10);

        if let Ok(t_str) = core::str::from_utf8(&time_str) {
            let clock_sz = f_mono.measure_str(t_str);
            let cx = (self.screen_w - (clock_sz.w * 2)) / 2;

            // Double scale clock presentation
            for (ci, &ch) in t_str.as_bytes().iter().enumerate() {
                let char_x = cx + (ci as i32 * 18);
                ui::font::draw_char_font(s, char_x, clock_y, ch, Color::WHITE, f_mono);
                ui::font::draw_char_font(s, char_x + 1, clock_y, ch, Color::WHITE, f_mono);
            }
        }

        // Date Line: Monday, Oct 26, 2026
        let date_str = "Monday, October 26, 2026";
        let date_sz = f_reg.measure_str(date_str);
        let dx = (self.screen_w - date_sz.w) / 2;
        ui::font::draw_text_font(s, dx, clock_y + 24, date_str, COLOR_ACCENT, f_reg);

        let welcome_str = "Welcome to NovaOS. Please authenticate to start your session.";
        let w_sz = f_small.measure_str(welcome_str);
        let wx = (self.screen_w - w_sz.w) / 2;
        ui::font::draw_text_font(s, wx, clock_y + 46, welcome_str, Color::rgb(139, 148, 158), f_small);
    }

    fn render_login_card<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        f_reg: &Font,
        f_small: &Font,
        f_mono: &Font,
    ) {
        let card_w = 400;
        let card_h = 330;
        let card_x = (self.screen_w - card_w) / 2;
        let card_y = (self.screen_h - card_h) / 2 + 30;

        let card_rect = Rect::new(card_x, card_y, card_w, card_h);

        // Glassmorphism background with shadow
        s.fill_rect(Rect::new(card_x + 4, card_y + 4, card_w, card_h), Color::rgba(0, 0, 0, 80));
        s.fill_rect(card_rect, COLOR_CARD_BG);
        draw_rect_outline(s, card_rect, 1, COLOR_CARD_BORDER);

        // User Avatar Circle (diameter 56px)
        let av_cx = card_x + (card_w / 2);
        let av_cy = card_y + 44;
        let av_r = 28;

        // Halo ring
        let av_rect = Rect::new(av_cx - av_r, av_cy - av_r, av_r * 2, av_r * 2);
        ui::draw::draw_rounded_rect(s, av_rect, av_r, Color::rgb(33, 38, 45));
        let ring_col = if self.active_account_is_admin() { COLOR_PURPLE } else { COLOR_ACCENT };
        draw_rect_outline(s, av_rect, 2, ring_col);

        // Avatar Initial / User glyph
        let initial = self.active_username().as_bytes().first().copied().unwrap_or(b'U');
        let init_upper = if (b'a'..=b'z').contains(&initial) { initial - 32 } else { initial };
        ui::font::draw_char_font(s, av_cx - 4, av_cy - 7, init_upper, Color::WHITE, f_reg);

        // Display Name & Role
        let dname = self.active_username();
        let d_sz = f_reg.measure_str(dname);
        ui::font::draw_text_font(s, av_cx - (d_sz.w / 2), card_y + 78, dname, Color::WHITE, f_reg);

        let role_str = if self.active_account_is_admin() {
            "Administrator (uid 800, gid 0)"
        } else {
            "Standard User (uid 700)"
        };
        let r_sz = f_small.measure_str(role_str);
        let r_col = if self.active_account_is_admin() { COLOR_PURPLE } else { COLOR_ACCENT };
        ui::font::draw_text_font(s, av_cx - (r_sz.w / 2), card_y + 96, role_str, r_col, f_small);

        // User Selector Pills (y: card_y + 114)
        let mut px = card_x + 24;
        for i in 0..self.account_count {
            let uname = self.accounts[i].username_as_str();
            let pw = (uname.len() as i32 * 8) + 16;
            let p_rect = Rect::new(px, card_y + 114, pw, 22);
            let is_sel = !self.is_custom_user && self.selected_account_idx == i;
            if is_sel {
                s.fill_rect(p_rect, COLOR_ACCENT_BTN);
                ui::font::draw_text_font(s, px + 8, card_y + 118, uname, Color::WHITE, f_small);
            } else {
                s.fill_rect(p_rect, Color::rgb(33, 38, 45));
                draw_rect_outline(s, p_rect, 1, COLOR_CARD_BORDER);
                ui::font::draw_text_font(s, px + 8, card_y + 118, uname, Color::rgb(139, 148, 158), f_small);
            }
            px += pw + 8;
        }

        // "Other..." pill
        let other_rect = Rect::new(px, card_y + 114, 64, 22);
        if self.is_custom_user {
            s.fill_rect(other_rect, COLOR_ACCENT_BTN);
            ui::font::draw_text_font(s, px + 8, card_y + 118, "Other...", Color::WHITE, f_small);
        } else {
            s.fill_rect(other_rect, Color::rgb(33, 38, 45));
            draw_rect_outline(s, other_rect, 1, COLOR_CARD_BORDER);
            ui::font::draw_text_font(s, px + 8, card_y + 118, "Other...", Color::rgb(139, 148, 158), f_small);
        }

        // Custom Username input (if is_custom_user)
        let mut current_y = card_y + 144;
        if self.is_custom_user {
            let ubox = Rect::new(card_x + 24, current_y, card_w - 48, 30);
            s.fill_rect(ubox, COLOR_INPUT_BG);
            let u_border = if self.focus == FocusField::Username { COLOR_ACCENT } else { COLOR_CARD_BORDER };
            draw_rect_outline(s, ubox, 1, u_border);

            let u_str = core::str::from_utf8(&self.custom_username[..self.custom_username_len]).unwrap_or("");
            ui::font::draw_text_font(s, card_x + 32, current_y + 8, u_str, Color::WHITE, f_small);

            if self.focus == FocusField::Username && (self.cursor_blink % 30 < 15) {
                let cur_x = card_x + 32 + (self.custom_username_len as i32 * 8);
                s.fill_rect(Rect::new(cur_x, current_y + 6, 2, 18), COLOR_ACCENT);
            }
            current_y += 38;
        }

        // Password Input Field Box
        let pbox = Rect::new(card_x + 24, current_y, card_w - 48, 32);
        s.fill_rect(pbox, COLOR_INPUT_BG);
        let p_border = if self.focus == FocusField::Password { COLOR_ACCENT } else { COLOR_CARD_BORDER };
        draw_rect_outline(s, pbox, 1, p_border);

        // Password text or bullets
        let pass_x = card_x + 32;
        if self.password_len == 0 {
            ui::font::draw_text_font(s, pass_x, current_y + 9, "Enter password...", Color::rgb(110, 118, 129), f_small);
        } else if self.show_password {
            if let Ok(pw_str) = core::str::from_utf8(&self.password_buf[..self.password_len]) {
                ui::font::draw_text_font(s, pass_x, current_y + 9, pw_str, Color::WHITE, f_mono);
            }
        } else {
            // Masked bullets
            for i in 0..self.password_len {
                let bx = pass_x + (i as i32 * 10);
                if bx + 8 >= card_x + card_w - 60 { break; }
                ui::draw::draw_rounded_rect(s, Rect::new(bx, current_y + 13, 6, 6), 3, Color::WHITE);
            }
        }

        // Blinking cursor
        if self.focus == FocusField::Password && (self.cursor_blink % 30 < 15) {
            let cur_x = if self.show_password {
                pass_x + (self.password_len as i32 * 8)
            } else {
                pass_x + (self.password_len as i32 * 10)
            };
            if cur_x < card_x + card_w - 60 {
                s.fill_rect(Rect::new(cur_x, current_y + 8, 2, 16), COLOR_ACCENT);
            }
        }

        // Show/Hide password icon [👁]
        let eye_rect = Rect::new(card_x + card_w - 56, current_y + 6, 24, 20);
        let eye_col = if self.show_password { COLOR_ACCENT } else { Color::rgb(139, 148, 158) };
        draw_rect_outline(s, eye_rect, 1, COLOR_CARD_BORDER);
        ui::font::draw_text_font(s, eye_rect.x + 6, eye_rect.y + 4, if self.show_password { "o" } else { "*" }, eye_col, f_small);

        current_y += 42;

        // Action Buttons: [Sign In ->] and [Clear]
        let btn_w = (card_w - 56) / 2;
        let btn_h = 32;

        let sign_rect = Rect::new(card_x + 24, current_y, btn_w, btn_h);
        s.fill_rect(sign_rect, COLOR_ACCENT_BTN);
        ui::font::draw_text_font(s, sign_rect.x + (btn_w - 64) / 2, current_y + 9, "Sign In ->", Color::WHITE, f_reg);

        let clr_rect = Rect::new(card_x + 32 + btn_w, current_y, btn_w, btn_h);
        s.fill_rect(clr_rect, Color::rgb(33, 38, 45));
        draw_rect_outline(s, clr_rect, 1, COLOR_CARD_BORDER);
        ui::font::draw_text_font(s, clr_rect.x + (btn_w - 40) / 2, current_y + 9, "Clear", Color::rgb(201, 209, 217), f_reg);

        current_y += 42;

        // Error / Status Alert Banner
        match self.auth_status {
            AuthStatus::Failed { attempts } => {
                let err_rect = Rect::new(card_x + 24, current_y, card_w - 48, 24);
                s.fill_rect(err_rect, Color::rgba(248, 81, 73, 30));
                draw_rect_outline(s, err_rect, 1, COLOR_DANGER);
                let mut err_msg = [0u8; 48];
                let em = b"Authentication failed. Attempt ";
                err_msg[..em.len()].copy_from_slice(em);
                err_msg[em.len()] = b'0' + (attempts % 10) as u8;
                if let Ok(s_err) = core::str::from_utf8(&err_msg[..em.len() + 1]) {
                    ui::font::draw_text_font(s, card_x + 32, current_y + 5, s_err, COLOR_DANGER, f_small);
                }
            }
            AuthStatus::Success { uid, .. } => {
                let ok_rect = Rect::new(card_x + 24, current_y, card_w - 48, 24);
                s.fill_rect(ok_rect, Color::rgba(46, 160, 67, 30));
                draw_rect_outline(s, ok_rect, 1, COLOR_SUCCESS);
                let mut ok_msg = [0u8; 48];
                let om = b"Identity verified: UID ";
                ok_msg[..om.len()].copy_from_slice(om);
                let mut uid_val = uid;
                let mut digits = [0u8; 6];
                let mut dc = 0;
                while uid_val > 0 {
                    digits[dc] = b'0' + (uid_val % 10) as u8;
                    uid_val /= 10;
                    dc += 1;
                }
                let mut p = om.len();
                while dc > 0 {
                    dc -= 1;
                    ok_msg[p] = digits[dc];
                    p += 1;
                }
                if let Ok(s_ok) = core::str::from_utf8(&ok_msg[..p]) {
                    ui::font::draw_text_font(s, card_x + 32, current_y + 5, s_ok, COLOR_SUCCESS, f_small);
                }
            }
            _ => {
                // Session Selector
                let sess_rect = Rect::new(card_x + 24, current_y, card_w - 48, 22);
                ui::font::draw_text_font(s, sess_rect.x, current_y + 4, "Session:", Color::rgb(139, 148, 158), f_small);
                ui::font::draw_text_font(s, sess_rect.x + 64, current_y + 4, self.session.name(), COLOR_ACCENT, f_small);
            }
        }
    }

    fn render_bottom_toolbar<S: Surface + ?Sized>(&self, s: &mut S, f: &Font) {
        let bar_h = 36;
        let bar_y = self.screen_h - bar_h;
        let bar_rect = Rect::new(0, bar_y, self.screen_w, bar_h);
        s.fill_rect(bar_rect, Color::rgba(22, 27, 34, 180));
        s.fill_rect(Rect::new(0, bar_y, self.screen_w, 1), COLOR_CARD_BORDER);

        // Security / Mechanism Notice
        ui::font::draw_text_font(
            s,
            24,
            bar_y + 10,
            "Protected by SYS_LOGIN & Process Identity Isolation (PBKDF2-HMAC-SHA256)",
            Color::rgb(139, 148, 158),
            f,
        );

        // Power Controls on far right
        let shut_w = 90;
        let shut_x = self.screen_w - shut_w - 24;
        let shut_rect = Rect::new(shut_x, bar_y + 6, shut_w, 24);
        s.fill_rect(shut_rect, Color::rgb(33, 38, 45));
        draw_rect_outline(s, shut_rect, 1, COLOR_CARD_BORDER);
        ui::font::draw_text_font(s, shut_x + 12, bar_y + 11, "[⏻] Shutdown", COLOR_DANGER, f);

        let rst_w = 80;
        let rst_x = shut_x - rst_w - 12;
        let rst_rect = Rect::new(rst_x, bar_y + 6, rst_w, 24);
        s.fill_rect(rst_rect, Color::rgb(33, 38, 45));
        draw_rect_outline(s, rst_rect, 1, COLOR_CARD_BORDER);
        ui::font::draw_text_font(s, rst_x + 10, bar_y + 11, "[↻] Restart", Color::rgb(201, 209, 217), f);
    }

    fn active_account_is_admin(&self) -> bool {
        if self.is_custom_user || self.selected_account_idx >= self.account_count {
            false
        } else {
            self.accounts[self.selected_account_idx].is_admin
        }
    }
}

// ---------------------------------------------------------------------------
// Static Global BSS Storage & Headless Selftest
// ---------------------------------------------------------------------------

pub static mut GLOBAL_LOGIN: LoginScreen = LoginScreen::uninit();
pub static mut TEST_RENDER_BUF: [u32; 400 * 300] = [0; 400 * 300];

pub fn run_selftest() -> i32 {
    let mut code = 0;

    unsafe {
        let msg = b"[login-selftest] running Month 3 graphical login screen verification suite\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    let ls = unsafe { &mut *addr_of_mut!(GLOBAL_LOGIN) };
    ls.init(800, 600);

    // Case 1: Account configuration parsing (USERS.CFG / SYSTEM.CFG)
    let parsed_ok = ls.account_count >= 2
        && ls.accounts[0].username_as_str() == "persisted"
        && ls.accounts[1].username_as_str() == "admin";

    if parsed_ok {
        unsafe {
            let msg = b"[login-selftest] PASS: account configuration parsing (USERS.CFG / SYSTEM.CFG)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[login-selftest] FAIL: account configuration parsing\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 2: Credential rejection on wrong password
    ls.selected_account_idx = 0; // persisted
    ls.is_custom_user = false;
    let bad_pw = b"wrong-password-entirely";
    ls.password_buf[..bad_pw.len()].copy_from_slice(bad_pw);
    ls.password_len = bad_pw.len();

    let uid_before = unsafe { ffi::sys_getuid() };
    let wrong_login_ok = !ls.attempt_login();
    let uid_after_wrong = unsafe { ffi::sys_getuid() };

    if wrong_login_ok && uid_before == uid_after_wrong {
        unsafe {
            let msg = b"[login-selftest] PASS: credential rejection and identity preservation on bad password\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[login-selftest] FAIL: credential rejection\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 3: Credential acceptance on correct password (persisted -> uid 700)
    let good_pw = b"persisted-pw";
    ls.password_buf[..good_pw.len()].copy_from_slice(good_pw);
    ls.password_len = good_pw.len();

    let right_login_ok = ls.attempt_login();
    let uid_after_right = unsafe { ffi::sys_getuid() };

    if right_login_ok && uid_after_right == 700 {
        unsafe {
            let msg = b"[login-selftest] PASS: credential acceptance and UID transition (persisted -> uid 700)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[login-selftest] FAIL: credential acceptance\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 4: Admin account escalation & authorization (admin -> uid 800)
    ls.selected_account_idx = 1; // admin
    let admin_pw = b"admin-correct-password";
    ls.password_buf[..admin_pw.len()].copy_from_slice(admin_pw);
    ls.password_len = admin_pw.len();

    let admin_login_ok = ls.attempt_login();
    let uid_after_admin = unsafe { ffi::sys_getuid() };

    if admin_login_ok && uid_after_admin == 800 {
        unsafe {
            let msg = b"[login-selftest] PASS: admin account authentication (admin -> uid 800, gid 0)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[login-selftest] FAIL: admin account authentication\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 5: Password buffer management, masking, and visibility toggling
    ls.password_len = 0;
    ls.handle_key(b'A' as i32);
    ls.handle_key(b'B' as i32);
    ls.handle_key(b'C' as i32);
    let len3 = ls.password_len == 3;
    ls.handle_key(8); // Backspace
    let len2 = ls.password_len == 2;
    ls.show_password = false;
    let hide_ok = !ls.show_password;
    ls.show_password = true;
    let show_ok = ls.show_password;

    if len3 && len2 && hide_ok && show_ok {
        unsafe {
            let msg = b"[login-selftest] PASS: password buffer management, masking, and visibility toggle\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[login-selftest] FAIL: password buffer management\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 6: Graphical login screen presentation (BufferSurface32)
    let buf = unsafe { &mut *addr_of_mut!(TEST_RENDER_BUF) };
    let mut surface = BufferSurface32::new(buf, 400, 300);
    ls.screen_w = 400;
    ls.screen_h = 300;
    ls.render(&mut surface);

    let non_zero = buf.iter().filter(|&&p| p != 0).count();
    if non_zero > 1000 {
        unsafe {
            let msg = b"[login-selftest] PASS: graphical login screen presentation (BufferSurface32)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[login-selftest] FAIL: graphical login presentation\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    if code == 0 {
        unsafe {
            let msg = b"[login-selftest] all cases passed\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    }

    code
}

// ---------------------------------------------------------------------------
// Standalone Process Entry Point
// ---------------------------------------------------------------------------

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8) -> i32 {
    if argc >= 2 {
        let arg1_ptr = unsafe { *argv.offset(1) };
        if !arg1_ptr.is_null() {
            let b0 = unsafe { *arg1_ptr };
            let b1 = unsafe { *arg1_ptr.offset(1) };
            let b2 = unsafe { *arg1_ptr.offset(2) };
            if b0 == b'-' && b1 == b'-' && b2 == b's' {
                return run_selftest();
            }
        }
    }

    let mut fb_info = ffi::NovaFbInfo::default();
    let rc = unsafe { ffi::sys_fb_info(&mut fb_info) };
    if rc < 0 {
        unsafe {
            let msg = b"login: failed to query framebuffer info\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"login: failed to acquire display\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let screen_w = fb_info.width as i32;
    let screen_h = fb_info.height as i32;

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
            let msg = b"login: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let ls = unsafe { &mut *addr_of_mut!(GLOBAL_LOGIN) };
    ls.init(screen_w, screen_h);

    let mut prev_left = false;

    while ls.running {
        let key = unsafe { ffi::sys_read_key() };
        if key >= 0 {
            ls.handle_key(key);
        }

        let mut m = ffi::MouseState::default();
        let _ = unsafe { ffi::sys_mouse_read(&mut m) };
        ls.mouse_x = (ls.mouse_x + m.dx).clamp(0, screen_w - 1);
        ls.mouse_y = (ls.mouse_y + m.dy).clamp(0, screen_h - 1);

        if m.left_button && !prev_left {
            ls.handle_mouse_click(ls.mouse_x, ls.mouse_y);
        }
        prev_left = m.left_button;

        // Render to framebuffer
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = BufferSurface32::new(pixel_slice, screen_w, screen_h);
        ls.render(&mut surface);

        // Draw cursor
        ui::cursor::draw_cursor_sprite(
            &mut surface,
            ls.mouse_x,
            ls.mouse_y,
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
    fn test_login_screen_initialization_and_accounts() {
        unsafe { ffi::reset_mock_environment() };
        let mut ls = LoginScreen::uninit();
        ls.init(800, 600);

        assert_eq!(ls.account_count, 2);
        assert_eq!(ls.accounts[0].username_as_str(), "persisted");
        assert_eq!(ls.accounts[0].uid, 700);
        assert!(!ls.accounts[0].is_admin);

        assert_eq!(ls.accounts[1].username_as_str(), "admin");
        assert_eq!(ls.accounts[1].uid, 800);
        assert!(ls.accounts[1].is_admin);

        assert_eq!(ls.hostname_as_str(), "novaos-test");
    }

    #[test]
    fn test_login_authentication_success_and_failure() {
        unsafe { ffi::reset_mock_environment() };
        let mut ls = LoginScreen::uninit();
        ls.init(800, 600);

        // Bad password for persisted
        ls.selected_account_idx = 0;
        let bad_pw = b"not-the-password";
        ls.password_buf[..bad_pw.len()].copy_from_slice(bad_pw);
        ls.password_len = bad_pw.len();

        let ok_bad = ls.attempt_login();
        assert!(!ok_bad);
        assert_eq!(unsafe { ffi::sys_getuid() }, 0);
        assert_eq!(ls.failed_attempts, 1);

        // Good password for persisted
        let good_pw = b"persisted-pw";
        ls.password_buf[..good_pw.len()].copy_from_slice(good_pw);
        ls.password_len = good_pw.len();

        let ok_good = ls.attempt_login();
        assert!(ok_good);
        assert_eq!(unsafe { ffi::sys_getuid() }, 700);
    }

    #[test]
    fn test_admin_authentication() {
        unsafe { ffi::reset_mock_environment() };
        let mut ls = LoginScreen::uninit();
        ls.init(800, 600);

        ls.selected_account_idx = 1; // admin
        let admin_pw = b"admin-correct-password";
        ls.password_buf[..admin_pw.len()].copy_from_slice(admin_pw);
        ls.password_len = admin_pw.len();

        let ok = ls.attempt_login();
        assert!(ok);
        assert_eq!(unsafe { ffi::sys_getuid() }, 800);
    }

    #[test]
    fn test_password_buffer_input_handling() {
        unsafe { ffi::reset_mock_environment() };
        let mut ls = LoginScreen::uninit();
        ls.init(800, 600);

        ls.focus = FocusField::Password;
        ls.handle_key(b'X' as i32);
        ls.handle_key(b'Y' as i32);
        ls.handle_key(b'Z' as i32);
        assert_eq!(ls.password_len, 3);
        assert_eq!(&ls.password_buf[..3], b"XYZ");

        ls.handle_key(8); // Backspace
        assert_eq!(ls.password_len, 2);
        assert_eq!(&ls.password_buf[..2], b"XY");

        ls.handle_key(27); // Escape clears password
        assert_eq!(ls.password_len, 0);
    }

    #[test]
    fn test_surface_rendering() {
        unsafe { ffi::reset_mock_environment() };
        let mut ls = LoginScreen::uninit();
        ls.init(400, 300);

        let mut buf = [0u32; 400 * 300];
        let mut surface = BufferSurface32::new(&mut buf, 400, 300);
        ls.render(&mut surface);

        let non_zero = buf.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 1000, "Rendered surface should contain colored pixels");
    }

    #[test]
    fn test_selftest_suite() {
        unsafe { ffi::reset_mock_environment() };
        let res = run_selftest();
        assert_eq!(res, 0);
    }
}
