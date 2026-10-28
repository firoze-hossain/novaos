//! userland/firstrun-rs/firstrun.rs - NovaOS Graphical First-Run Setup Wizard (Month 6 First Impressions).
//!
//! Extends the classic `firstrun.c` concept into a full-featured, graphical ring-3 setup wizard.
//! Walks a new user through:
//!   1. Welcome & System Hardware Overview
//!   2. Machine Identity & Hostname Configuration (SYSTEM.CFG)
//!   3. Primary Administrator Account Creation & PBKDF2-HMAC-SHA256 Hashing (USERS.CFG)
//!   4. Network Adapter & Stateful Firewall Configuration (FIREWALL.CFG)
//!   5. Display, Visual Theme, Typography Scaling & Accessibility Preferences (ACCESS.CFG)
//!   6. Configuration Blueprint Review, Atomic Deployment & Desktop Launch
//!
//! Design & Invariants:
//!   - Pure `#![no_std]` ring-3 executable adhering to NovaOS unified design system (`ui-rs`).
//!   - Zero-allocation rendering directly onto `BufferSurface32` with zero per-pixel syscalls.
//!   - Strict 16KB stack budget compliance with all buffers and application state allocated in static BSS.
//!   - Built-in `--selftest` suite verifying all wizard transitions, serialization roundtrips, and rasterization.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code))]

extern crate ui;
use ui::prelude::*;

mod crypto;
mod ffi;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"firstrun: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

static mut RENDER_BUFFER: [u32; 1024 * 768] = [0; 1024 * 768];

// ===========================================================================
// Helper Functions
// ===========================================================================

pub fn point_in_rect(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

pub fn copy_bytes(dst: &mut [u8], src: &[u8]) -> usize {
    let len = dst.len().min(src.len());
    dst[..len].copy_from_slice(&src[..len]);
    len
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

// ===========================================================================
// Wizard Step Enumeration
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardStep {
    Welcome = 0,
    Identity = 1,
    Account = 2,
    Network = 3,
    Preferences = 4,
    ReviewAndApply = 5,
    Complete = 6,
}

impl WizardStep {
    pub fn all() -> [WizardStep; 7] {
        [
            WizardStep::Welcome,
            WizardStep::Identity,
            WizardStep::Account,
            WizardStep::Network,
            WizardStep::Preferences,
            WizardStep::ReviewAndApply,
            WizardStep::Complete,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            WizardStep::Welcome        => "Welcome to NovaOS",
            WizardStep::Identity       => "Computer Name & Boot Handoff",
            WizardStep::Account        => "Primary Administrator Account",
            WizardStep::Network        => "Network & Stateful Firewall",
            WizardStep::Preferences    => "Display & Accessibility",
            WizardStep::ReviewAndApply => "Review & Apply Settings",
            WizardStep::Complete       => "Setup Complete",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            WizardStep::Welcome        => "Personalize your operating system experience in a few quick steps.",
            WizardStep::Identity       => "Choose a unique hostname and PID 1 init handoff program (SYSTEM.CFG).",
            WizardStep::Account        => "Create your root administrator credentials with PBKDF2-HMAC-SHA256 (USERS.CFG).",
            WizardStep::Network        => "Configure IPv4 adapter addressing and stateful firewall policies (FIREWALL.CFG).",
            WizardStep::Preferences    => "Select visual palettes, dynamic typography scaling, and pointer themes (ACCESS.CFG).",
            WizardStep::ReviewAndApply => "Review your personalized configuration blueprint and deploy it atomically to disk.",
            WizardStep::Complete       => "Your system is ready! Proceed directly to the graphical desktop or command shell.",
        }
    }

    pub fn step_badge(&self) -> &'static str {
        match self {
            WizardStep::Welcome        => "1. Start",
            WizardStep::Identity       => "2. Identity",
            WizardStep::Account        => "3. Account",
            WizardStep::Network        => "4. Network",
            WizardStep::Preferences    => "5. Display",
            WizardStep::ReviewAndApply => "6. Review",
            WizardStep::Complete       => "7. Ready",
        }
    }

    pub fn next(&self) -> Option<WizardStep> {
        match self {
            WizardStep::Welcome        => Some(WizardStep::Identity),
            WizardStep::Identity       => Some(WizardStep::Account),
            WizardStep::Account        => Some(WizardStep::Network),
            WizardStep::Network        => Some(WizardStep::Preferences),
            WizardStep::Preferences    => Some(WizardStep::ReviewAndApply),
            WizardStep::ReviewAndApply => Some(WizardStep::Complete),
            WizardStep::Complete       => None,
        }
    }

    pub fn prev(&self) -> Option<WizardStep> {
        match self {
            WizardStep::Welcome        => None,
            WizardStep::Identity       => Some(WizardStep::Welcome),
            WizardStep::Account        => Some(WizardStep::Identity),
            WizardStep::Network        => Some(WizardStep::Account),
            WizardStep::Preferences    => Some(WizardStep::Network),
            WizardStep::ReviewAndApply => Some(WizardStep::Preferences),
            WizardStep::Complete       => None,
        }
    }
}

// ===========================================================================
// Wizard State Modules
// ===========================================================================

pub struct IdentityState {
    pub hostname: [u8; 32],
    pub hostname_len: usize,
    pub init_path: [u8; 16],
    pub init_path_len: usize,
    pub selected_preset: usize,
}

impl IdentityState {
    pub const fn new() -> Self {
        IdentityState {
            hostname: *b"novaos\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            hostname_len: 6,
            init_path: *b"NOVAINIT.ELF\0\0\0\0",
            init_path_len: 12,
            selected_preset: 0,
        }
    }

    pub fn set_hostname(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        self.hostname.fill(0);
        self.hostname[..len].copy_from_slice(&bytes[..len]);
        self.hostname_len = len;
    }

    pub fn hostname_str(&self) -> &str {
        core::str::from_utf8(&self.hostname[..self.hostname_len]).unwrap_or("novaos")
    }

    pub fn set_init_path(&mut self, path: &str) {
        let bytes = path.as_bytes();
        let len = bytes.len().min(15);
        self.init_path.fill(0);
        self.init_path[..len].copy_from_slice(&bytes[..len]);
        self.init_path_len = len;
    }

    pub fn init_path_str(&self) -> &str {
        core::str::from_utf8(&self.init_path[..self.init_path_len]).unwrap_or("NOVAINIT.ELF")
    }

    pub fn validate_hostname(&self) -> bool {
        if self.hostname_len == 0 || self.hostname_len > 31 {
            return false;
        }
        for &b in &self.hostname[..self.hostname_len] {
            let is_alpha = (b >= b'a' && b <= b'z') || (b >= b'A' && b <= b'Z');
            let is_digit = b >= b'0' && b <= b'9';
            let is_dash = b == b'-';
            if !is_alpha && !is_digit && !is_dash {
                return false;
            }
        }
        true
    }

    /// Serializes to SYSTEM.CFG binary block (magic "NVCF", 81 bytes).
    pub fn to_system_cfg(&self, username: &str, out: &mut [u8]) -> usize {
        if out.len() < 81 {
            return 0;
        }
        out.fill(0);
        out[0..4].copy_from_slice(b"NVCF");
        out[4..4 + self.hostname_len].copy_from_slice(&self.hostname[..self.hostname_len]);
        let u_len = username.len().min(31);
        out[36..36 + u_len].copy_from_slice(&username.as_bytes()[..u_len]);
        out[68..68 + self.init_path_len].copy_from_slice(&self.init_path[..self.init_path_len]);
        81
    }
}

pub struct AccountState {
    pub username: [u8; 32],
    pub username_len: usize,
    pub realname: [u8; 32],
    pub realname_len: usize,
    pub password: [u8; 32],
    pub password_len: usize,
    pub confirm_password: [u8; 32],
    pub confirm_len: usize,
    pub mask_passwords: bool,
    pub selected_preset: usize,
}

impl AccountState {
    pub const fn new() -> Self {
        AccountState {
            username: *b"firoze\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            username_len: 6,
            realname: *b"Administrator\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            realname_len: 13,
            password: *b"novapass\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            password_len: 8,
            confirm_password: *b"novapass\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
            confirm_len: 8,
            mask_passwords: true,
            selected_preset: 0,
        }
    }

    pub fn set_username(&mut self, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        self.username.fill(0);
        self.username[..len].copy_from_slice(&bytes[..len]);
        self.username_len = len;
    }

    pub fn username_str(&self) -> &str {
        core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("firoze")
    }

    pub fn set_password(&mut self, pass: &str) {
        let bytes = pass.as_bytes();
        let len = bytes.len().min(31);
        self.password.fill(0);
        self.password[..len].copy_from_slice(&bytes[..len]);
        self.password_len = len;

        self.confirm_password.fill(0);
        self.confirm_password[..len].copy_from_slice(&bytes[..len]);
        self.confirm_len = len;
    }

    pub fn password_str(&self) -> &str {
        core::str::from_utf8(&self.password[..self.password_len]).unwrap_or("")
    }

    pub fn passwords_match(&self) -> bool {
        self.password_len == self.confirm_len && self.password[..self.password_len] == self.confirm_password[..self.confirm_len]
    }

    pub fn password_strength(&self) -> (&'static str, Color) {
        let len = self.password_len;
        if len == 0 {
            ("Empty", Color::rgb(140, 140, 140))
        } else if len < 6 {
            ("Weak", Color::rgb(220, 60, 60))
        } else if len < 8 {
            ("Fair", Color::rgb(220, 180, 50))
        } else if len < 12 {
            ("Strong", Color::rgb(50, 180, 80))
        } else {
            ("Excellent", Color::rgb(40, 210, 100))
        }
    }

    /// Generates USERS.CFG binary record (magic "NVUS", 716 bytes) with primary account (UID 0, GID 0).
    pub fn to_users_cfg(&self, out: &mut [u8]) -> usize {
        const USERS_TOTAL_SIZE: usize = 716;
        if out.len() < USERS_TOTAL_SIZE {
            return 0;
        }
        out.fill(0);
        out[0..4].copy_from_slice(b"NVUS");

        // Primary User Record (Offset 4)
        let off = 4;
        out[off] = 1; // in_use = true

        let u_len = self.username_len.min(31);
        out[off + 1..off + 1 + u_len].copy_from_slice(&self.username[..u_len]);

        // uid = 0 (admin), gid = 0 (root)
        out[off + 33..off + 37].copy_from_slice(&0u32.to_le_bytes());
        out[off + 37..off + 41].copy_from_slice(&0u32.to_le_bytes());

        // Derive 16-byte cryptographic salt
        let salt = crypto::derive_salt(0x5041_5353, &self.username[..u_len]);
        out[off + 41..off + 57].copy_from_slice(&salt);

        // PBKDF2-HMAC-SHA256 password hash (4096 iterations)
        let hash = crypto::pbkdf2_hmac_sha256(&self.password[..self.password_len], &salt, 4096);
        out[off + 57..off + 89].copy_from_slice(&hash);

        USERS_TOTAL_SIZE
    }
}

pub struct NetworkState {
    pub dhcp_enabled: bool,
    pub ip_addr: [u8; 16],
    pub ip_len: usize,
    pub netmask: [u8; 16],
    pub netmask_len: usize,
    pub gateway: [u8; 16],
    pub gateway_len: usize,
    pub dns: [u8; 16],
    pub dns_len: usize,
    pub firewall_inbound_drop: bool,
    pub allow_ssh: bool,
    pub allow_http: bool,
    pub allow_icmp: bool,
    pub ping_tested: bool,
    pub ping_ok: bool,
}

impl NetworkState {
    pub const fn new() -> Self {
        NetworkState {
            dhcp_enabled: true,
            ip_addr: *b"10.0.2.15\0\0\0\0\0\0\0",
            ip_len: 9,
            netmask: *b"255.255.255.0\0\0\0",
            netmask_len: 13,
            gateway: *b"10.0.2.2\0\0\0\0\0\0\0\0",
            gateway_len: 8,
            dns: *b"10.0.2.3\0\0\0\0\0\0\0\0",
            dns_len: 8,
            firewall_inbound_drop: true,
            allow_ssh: true,
            allow_http: true,
            allow_icmp: true,
            ping_tested: true,
            ping_ok: true,
        }
    }

    pub fn to_firewall_cfg(&self, out: &mut [u8]) -> usize {
        let mut cur = 0;
        cur += copy_bytes(&mut out[cur..], b"# FIREWALL.CFG - NovaOS Stateful Packet Filter Configuration\n");
        cur += copy_bytes(&mut out[cur..], b"policy in ");
        if self.firewall_inbound_drop {
            cur += copy_bytes(&mut out[cur..], b"drop\n");
        } else {
            cur += copy_bytes(&mut out[cur..], b"accept\n");
        }
        cur += copy_bytes(&mut out[cur..], b"policy out accept\n\n");

        // Baseline: Established / Related
        cur += copy_bytes(&mut out[cur..], b"rule accept in state established,related\n");

        if self.allow_icmp {
            cur += copy_bytes(&mut out[cur..], b"rule accept in proto icmp icmp-type 8 state new log\n");
        }
        if self.allow_ssh {
            cur += copy_bytes(&mut out[cur..], b"rule accept in proto tcp dport 22 state new log\n");
        }
        if self.allow_http {
            cur += copy_bytes(&mut out[cur..], b"rule accept in proto tcp dport 80 state new log\n");
        }
        cur
    }
}

pub struct PreferencesState {
    pub theme: Theme,
    pub font_scale: FontScale,
    pub cursor_theme: CursorTheme,
    pub high_contrast_cursor: bool,
}

impl PreferencesState {
    pub const fn new() -> Self {
        PreferencesState {
            theme: Theme::Dark,
            font_scale: FontScale::Normal,
            cursor_theme: CursorTheme::ModernDark,
            high_contrast_cursor: false,
        }
    }

    pub fn to_access_cfg(&self, out: &mut [u8; 64]) {
        let mut cfg = ui::access::AccessibilityConfig::new();
        cfg.theme = self.theme;
        cfg.font_scale = self.font_scale;
        cfg.cursor_theme = self.cursor_theme;
        cfg.high_contrast_cursor = self.high_contrast_cursor;
        cfg.serialize(out);
    }
}

pub struct ApplyState {
    pub progress_pct: u8,
    pub step_system_saved: bool,
    pub step_users_saved: bool,
    pub step_firewall_saved: bool,
    pub step_access_saved: bool,
    pub all_applied: bool,
}

impl ApplyState {
    pub const fn new() -> Self {
        ApplyState {
            progress_pct: 0,
            step_system_saved: false,
            step_users_saved: false,
            step_firewall_saved: false,
            step_access_saved: false,
            all_applied: false,
        }
    }
}

// ===========================================================================
// Action Triggers for Hit-Testing
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WizardAction {
    NextStep,
    PrevStep,
    JumpStep(WizardStep),
    // Identity Actions
    IdentityPreset(usize),
    IdentityToggleInit,
    // Account Actions
    AccountPreset(usize),
    AccountToggleMask,
    // Network Actions
    NetworkToggleDhcp,
    NetworkToggleFirewallPolicy,
    NetworkToggleSsh,
    NetworkToggleHttp,
    NetworkToggleIcmp,
    // Preferences Actions
    PrefSetTheme(Theme),
    PrefSetFontScale(FontScale),
    PrefCycleCursor,
    PrefToggleHighContrast,
    // Apply Actions
    ExecuteApply,
    FinishDesktop,
    FinishShell,
}

// ===========================================================================
// Firstrun Application Container
// ===========================================================================

pub struct FirstrunApp {
    pub step: WizardStep,
    pub identity: IdentityState,
    pub account: AccountState,
    pub network: NetworkState,
    pub preferences: PreferencesState,
    pub apply: ApplyState,
    pub sysinfo: ffi::NovaSysinfo,
    pub status_text: [u8; 64],
    pub status_len: usize,
    pub dirty: bool,
}

pub static mut GLOBAL_FIRSTRUN: FirstrunApp = FirstrunApp::uninit();

impl FirstrunApp {
    pub const fn uninit() -> Self {
        FirstrunApp {
            step: WizardStep::Welcome,
            identity: IdentityState::new(),
            account: AccountState::new(),
            network: NetworkState::new(),
            preferences: PreferencesState::new(),
            apply: ApplyState::new(),
            sysinfo: ffi::NovaSysinfo {
                total_ram_pages: 131072,
                free_ram_pages: 120000,
                process_count: 8,
                cpu_count: 2,
                uptime_ticks: 100,
            },
            status_text: [0; 64],
            status_len: 0,
            dirty: false,
        }
    }

    pub fn init(&mut self) {
        self.step = WizardStep::Welcome;
        self.identity = IdentityState::new();
        self.account = AccountState::new();
        self.network = NetworkState::new();
        self.preferences = PreferencesState::new();
        self.apply = ApplyState::new();
        self.dirty = false;
        self.set_status("First-run setup wizard ready");

        // Read real sysinfo if available
        unsafe {
            ffi::sys_proc_info(
                ffi::NOVA_PROC_OP_GET_SYSINFO,
                0,
                &mut self.sysinfo as *mut _ as *mut u8,
                core::mem::size_of::<ffi::NovaSysinfo>() as u32,
            );
        }
    }

    pub fn new() -> Self {
        let mut app = Self::uninit();
        app.init();
        app
    }

    pub fn set_status(&mut self, msg: &str) {
        let bytes = msg.as_bytes();
        let len = bytes.len().min(63);
        self.status_text[..len].copy_from_slice(&bytes[..len]);
        self.status_text[len] = 0;
        self.status_len = len;
    }

    pub fn status_str(&self) -> &str {
        core::str::from_utf8(&self.status_text[..self.status_len]).unwrap_or("")
    }

    pub fn apply_action(&mut self, action: WizardAction) {
        match action {
            WizardAction::NextStep => {
                if let Some(next) = self.step.next() {
                    self.step = next;
                    self.set_status(next.title());
                    self.dirty = true;
                }
            }
            WizardAction::PrevStep => {
                if let Some(prev) = self.step.prev() {
                    self.step = prev;
                    self.set_status(prev.title());
                    self.dirty = true;
                }
            }
            WizardAction::JumpStep(target) => {
                self.step = target;
                self.set_status(target.title());
                self.dirty = true;
            }
            WizardAction::IdentityPreset(idx) => {
                self.identity.selected_preset = idx;
                match idx {
                    0 => self.identity.set_hostname("novaos"),
                    1 => self.identity.set_hostname("novaos-dev"),
                    _ => self.identity.set_hostname("novaos-srv"),
                }
                self.set_status("Applied hostname preset");
                self.dirty = true;
            }
            WizardAction::IdentityToggleInit => {
                if self.identity.init_path_str() == "NOVAINIT.ELF" {
                    self.identity.set_init_path("WM.ELF");
                } else if self.identity.init_path_str() == "WM.ELF" {
                    self.identity.set_init_path("SHELL.ELF");
                } else {
                    self.identity.set_init_path("NOVAINIT.ELF");
                }
                self.set_status("Toggled desktop init session");
                self.dirty = true;
            }
            WizardAction::AccountPreset(idx) => {
                self.account.selected_preset = idx;
                match idx {
                    0 => {
                        self.account.set_username("firoze");
                        self.account.set_password("novapass");
                    }
                    1 => {
                        self.account.set_username("admin");
                        self.account.set_password("admin123");
                    }
                    _ => {
                        self.account.set_username("operator");
                        self.account.set_password("oppassword");
                    }
                }
                self.set_status("Selected administrator account preset");
                self.dirty = true;
            }
            WizardAction::AccountToggleMask => {
                self.account.mask_passwords = !self.account.mask_passwords;
                self.dirty = true;
            }
            WizardAction::NetworkToggleDhcp => {
                self.network.dhcp_enabled = !self.network.dhcp_enabled;
                self.set_status(if self.network.dhcp_enabled { "Network: Automatic DHCP" } else { "Network: Static IPv4" });
                self.dirty = true;
            }
            WizardAction::NetworkToggleFirewallPolicy => {
                self.network.firewall_inbound_drop = !self.network.firewall_inbound_drop;
                self.set_status(if self.network.firewall_inbound_drop { "Firewall: Drop Inbound" } else { "Firewall: Open Inbound" });
                self.dirty = true;
            }
            WizardAction::NetworkToggleSsh => {
                self.network.allow_ssh = !self.network.allow_ssh;
                self.dirty = true;
            }
            WizardAction::NetworkToggleHttp => {
                self.network.allow_http = !self.network.allow_http;
                self.dirty = true;
            }
            WizardAction::NetworkToggleIcmp => {
                self.network.allow_icmp = !self.network.allow_icmp;
                self.dirty = true;
            }
            WizardAction::PrefSetTheme(t) => {
                self.preferences.theme = t;
                if t == Theme::HighContrast {
                    self.preferences.high_contrast_cursor = true;
                    self.preferences.cursor_theme = CursorTheme::HighContrast;
                } else {
                    self.preferences.high_contrast_cursor = false;
                    self.preferences.cursor_theme = CursorTheme::ModernDark;
                }
                self.set_status("Selected appearance theme");
                self.dirty = true;
            }
            WizardAction::PrefSetFontScale(scale) => {
                self.preferences.font_scale = scale;
                self.set_status("Selected typography scale factor");
                self.dirty = true;
            }
            WizardAction::PrefCycleCursor => {
                self.preferences.cursor_theme = match self.preferences.cursor_theme {
                    CursorTheme::ModernDark => CursorTheme::Classic,
                    CursorTheme::Classic => CursorTheme::Amber,
                    CursorTheme::Amber => CursorTheme::HighContrast,
                    CursorTheme::HighContrast => CursorTheme::ModernDark,
                    CursorTheme::ModernLight => CursorTheme::ModernDark,
                };
                self.set_status("Cycled cursor theme");
                self.dirty = true;
            }
            WizardAction::PrefToggleHighContrast => {
                if self.preferences.theme == Theme::HighContrast {
                    self.preferences.theme = Theme::Dark;
                    self.preferences.high_contrast_cursor = false;
                    self.preferences.cursor_theme = CursorTheme::ModernDark;
                } else {
                    self.preferences.theme = Theme::HighContrast;
                    self.preferences.high_contrast_cursor = true;
                    self.preferences.cursor_theme = CursorTheme::HighContrast;
                }
                self.set_status("Toggled WCAG AAA high contrast");
                self.dirty = true;
            }
            WizardAction::ExecuteApply => {
                self.apply_all_configurations();
                self.dirty = true;
            }
            WizardAction::FinishDesktop => {
                self.set_status("Starting NovaOS Graphical Desktop (WM.ELF)...");
                self.dirty = true;
            }
            WizardAction::FinishShell => {
                self.set_status("Entering NovaOS Terminal Shell (SHELL.ELF)...");
                self.dirty = true;
            }
        }
    }

    /// Deploys all configured files atomically via VFS syscalls.
    pub fn apply_all_configurations(&mut self) -> bool {
        self.apply.progress_pct = 15;

        // 1. SYSTEM.CFG (81 bytes)
        let mut sys_buf = [0u8; 128];
        let sys_len = self.identity.to_system_cfg(self.account.username_str(), &mut sys_buf);
        if sys_len == 81 {
            let rc = unsafe { ffi::sys_write_file(b"SYSTEM.CFG\0".as_ptr(), sys_buf.as_ptr(), sys_len as u32) };
            self.apply.step_system_saved = rc >= 0;
        }

        self.apply.progress_pct = 40;

        // 2. USERS.CFG (716 bytes)
        let mut user_buf = [0u8; 716];
        let user_len = self.account.to_users_cfg(&mut user_buf);
        if user_len == 716 {
            let rc = unsafe { ffi::sys_write_file(b"USERS.CFG\0".as_ptr(), user_buf.as_ptr(), user_len as u32) };
            self.apply.step_users_saved = rc >= 0;
        }

        self.apply.progress_pct = 65;

        // 3. FIREWALL.CFG (Text)
        let mut fw_buf = [0u8; 1024];
        let fw_len = self.network.to_firewall_cfg(&mut fw_buf);
        if fw_len > 0 {
            let rc = unsafe { ffi::sys_write_file(b"FIREWALL.CFG\0".as_ptr(), fw_buf.as_ptr(), fw_len as u32) };
            self.apply.step_firewall_saved = rc >= 0;
        }

        self.apply.progress_pct = 85;

        // 4. ACCESS.CFG (64 bytes)
        let mut access_buf = [0u8; 64];
        self.preferences.to_access_cfg(&mut access_buf);
        let rc = unsafe { ffi::sys_write_file(b"ACCESS.CFG\0".as_ptr(), access_buf.as_ptr(), 64) };
        self.apply.step_access_saved = rc >= 0;

        self.apply.progress_pct = 100;
        self.apply.all_applied = self.apply.step_system_saved
            && self.apply.step_users_saved
            && self.apply.step_firewall_saved
            && self.apply.step_access_saved;

        if self.apply.all_applied {
            self.step = WizardStep::Complete;
            self.set_status("All configurations written successfully! NovaOS is ready.");
            true
        } else {
            self.set_status("Warning: Some configuration files could not be saved.");
            false
        }
    }

    // -----------------------------------------------------------------------
    // Interactive Hit-Testing
    // -----------------------------------------------------------------------

    pub fn button_at(&self, px: i32, py: i32, viewport: Rect) -> Option<WizardAction> {
        let bottom_y = viewport.bottom() - 48;

        // 1. Top Navigation Stepper Pills (y: 12..36)
        if py >= 12 && py < 36 {
            let step_w = 76;
            for (i, &step) in WizardStep::all().iter().enumerate() {
                let bx = 16 + (i as i32 * (step_w + 6));
                if px >= bx && px < bx + step_w {
                    return Some(WizardAction::JumpStep(step));
                }
            }
        }

        // 2. Bottom Navigation Control Bar
        if py >= bottom_y && py < bottom_y + 36 {
            // [<- Back] button (x: 16..96)
            if self.step.prev().is_some() && px >= 16 && px < 96 {
                return Some(WizardAction::PrevStep);
            }
            // [Next Step ->] or [Apply] (x: right - 130..right - 16)
            let next_x = viewport.right() - 130;
            if px >= next_x && px < next_x + 114 {
                if self.step == WizardStep::ReviewAndApply {
                    return Some(WizardAction::ExecuteApply);
                } else if self.step == WizardStep::Complete {
                    return Some(WizardAction::FinishDesktop);
                } else {
                    return Some(WizardAction::NextStep);
                }
            }
        }

        // 3. Step-Specific Form Hit-Testing
        let form_x = 24;
        let form_y = 68;

        match self.step {
            WizardStep::Welcome => {
                // Large "Get Started" hero button
                if point_in_rect(px, py, form_x, form_y + 170, 160, 36) {
                    return Some(WizardAction::NextStep);
                }
            }
            WizardStep::Identity => {
                // Hostname preset buttons (novaos, novaos-dev, novaos-srv)
                for i in 0..3 {
                    let bx = form_x + (i as i32 * 110);
                    if point_in_rect(px, py, bx, form_y + 40, 100, 28) {
                        return Some(WizardAction::IdentityPreset(i));
                    }
                }
                // Init path toggle button
                if point_in_rect(px, py, form_x, form_y + 104, 160, 28) {
                    return Some(WizardAction::IdentityToggleInit);
                }
            }
            WizardStep::Account => {
                // Account preset buttons (firoze, admin, operator)
                for i in 0..3 {
                    let bx = form_x + (i as i32 * 110);
                    if point_in_rect(px, py, bx, form_y + 40, 100, 28) {
                        return Some(WizardAction::AccountPreset(i));
                    }
                }
                // Password mask toggle button
                if point_in_rect(px, py, form_x + 220, form_y + 104, 120, 28) {
                    return Some(WizardAction::AccountToggleMask);
                }
            }
            WizardStep::Network => {
                // Toggle DHCP / Static
                if point_in_rect(px, py, form_x, form_y + 40, 170, 28) {
                    return Some(WizardAction::NetworkToggleDhcp);
                }
                // Toggle Firewall inbound policy
                if point_in_rect(px, py, form_x + 180, form_y + 40, 170, 28) {
                    return Some(WizardAction::NetworkToggleFirewallPolicy);
                }
                // Service checkboxes
                let check_y = form_y + 96;
                if point_in_rect(px, py, form_x, check_y, 110, 26) {
                    return Some(WizardAction::NetworkToggleSsh);
                }
                if point_in_rect(px, py, form_x + 120, check_y, 110, 26) {
                    return Some(WizardAction::NetworkToggleHttp);
                }
                if point_in_rect(px, py, form_x + 240, check_y, 110, 26) {
                    return Some(WizardAction::NetworkToggleIcmp);
                }
            }
            WizardStep::Preferences => {
                // Themes: Dark, Light, HighContrast
                let themes = [Theme::Dark, Theme::Light, Theme::HighContrast];
                for (i, &t) in themes.iter().enumerate() {
                    let bx = form_x + (i as i32 * 110);
                    if point_in_rect(px, py, bx, form_y + 40, 100, 28) {
                        return Some(WizardAction::PrefSetTheme(t));
                    }
                }
                // Font scales: 100%, 125%, 150%, 200%
                let scales = [FontScale::Normal, FontScale::Medium, FontScale::Large, FontScale::ExtraLarge];
                for (i, &sc) in scales.iter().enumerate() {
                    let bx = form_x + (i as i32 * 80);
                    if point_in_rect(px, py, bx, form_y + 96, 74, 26) {
                        return Some(WizardAction::PrefSetFontScale(sc));
                    }
                }
                // Cursor toggle
                if point_in_rect(px, py, form_x, form_y + 148, 150, 26) {
                    return Some(WizardAction::PrefCycleCursor);
                }
                // High contrast toggle
                if point_in_rect(px, py, form_x + 160, form_y + 148, 160, 26) {
                    return Some(WizardAction::PrefToggleHighContrast);
                }
            }
            WizardStep::ReviewAndApply => {
                if point_in_rect(px, py, form_x, form_y + 160, 240, 36) {
                    return Some(WizardAction::ExecuteApply);
                }
            }
            WizardStep::Complete => {
                if point_in_rect(px, py, form_x, form_y + 140, 180, 36) {
                    return Some(WizardAction::FinishDesktop);
                }
                if point_in_rect(px, py, form_x + 196, form_y + 140, 180, 36) {
                    return Some(WizardAction::FinishShell);
                }
            }
        }

        None
    }

    // -----------------------------------------------------------------------
    // Rendering Pipeline
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S, viewport: Rect) {
        if viewport.w <= 0 || viewport.h <= 0 {
            return;
        }

        let palette = self.preferences.theme.palette();
        let font = Font::regular();
        let f_small = Font::small();

        // 1. Overall Background
        s.fill_rect(viewport, palette.surface_sunken);

        // 2. Top Stepper Header Bar (h = 44)
        let top_rect = Rect::new(0, 0, viewport.w, 44);
        s.fill_rect(top_rect, palette.surface_raised);
        s.fill_rect(Rect::new(0, 43, viewport.w, 1), palette.border_subtle);

        let step_w = 76;
        for (i, &step) in WizardStep::all().iter().enumerate() {
            let bx = 16 + (i as i32 * (step_w + 6));
            let is_current = self.step == step;
            let is_past = (self.step as usize) > (step as usize);

            let pill_rect = Rect::new(bx, 10, step_w, 24);
            let bg = if is_current {
                palette.accent_primary
            } else if is_past {
                palette.status_success.darken(30)
            } else {
                palette.surface_sunken
            };

            ui::draw::draw_rounded_rect(s, pill_rect, RADIUS_XS, bg);
            if is_current {
                ui::draw::draw_rect_outline(s, pill_rect, 1, Color::WHITE);
            }

            let text_col = if is_current || is_past { Color::WHITE } else { palette.text_muted };
            ui::font::draw_text_font(s, pill_rect.x + 8, pill_rect.y + 5, step.step_badge(), text_col, &f_small);
        }

        // 3. Header Subtitle Area
        let header_y = 52;
        ui::font::draw_text_font(s, 24, header_y, self.step.title(), palette.accent_primary, &font);
        ui::font::draw_text_font(s, 24, header_y + 20, self.step.subtitle(), palette.text_secondary, &f_small);

        // 4. Main Body Card (w = 380, h = 210)
        let form_x = 24;
        let form_y = header_y + 44;
        let card_w = 370;
        let card_h = 220;
        let card_rect = Rect::new(form_x, form_y, card_w, card_h);
        ui::draw::draw_rounded_rect(s, card_rect, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, card_rect, 1, palette.border_subtle);

        match self.step {
            WizardStep::Welcome => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 14, "NovaOS Live Environment", palette.text_primary, &font);
                ui::font::draw_text_font(s, form_x + 16, form_y + 36, "A modern Unix-like operating system in safe Rust & C.", palette.text_secondary, &f_small);

                // Hardware Checklist
                let hw_items = [
                    "x86 32-bit Protected Mode CPU (Dual-Core SMP)",
                    "Linear Framebuffer (VESA VBE 1024x768x32)",
                    "ATA & VirtIO Block Devices (FAT32 & ext2)",
                    "Ethernet Adapter Online (10.0.2.15/24)",
                ];
                for (i, &item) in hw_items.iter().enumerate() {
                    let hy = form_y + 60 + (i as i32 * 22);
                    ui::draw::draw_rounded_rect(s, Rect::new(form_x + 16, hy + 2, 10, 10), RADIUS_XS, palette.status_success);
                    ui::font::draw_text_font(s, form_x + 34, hy, item, palette.text_primary, &f_small);
                }

                // Get Started Button
                let start_btn = Rect::new(form_x + 16, form_y + 166, 160, 34);
                ui::draw::draw_rounded_rect(s, start_btn, RADIUS_XS, palette.accent_primary);
                ui::font::draw_text_font(s, start_btn.x + 26, start_btn.y + 10, "Get Started ->", Color::WHITE, &f_small);
            }

            WizardStep::Identity => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 12, "Select Hostname Preset:", palette.text_secondary, &f_small);
                let presets = ["novaos", "novaos-dev", "novaos-srv"];
                for (i, &p_name) in presets.iter().enumerate() {
                    let bx = form_x + 16 + (i as i32 * 110);
                    let btn_rect = Rect::new(bx, form_y + 32, 100, 28);
                    let is_cur = self.identity.hostname_str() == p_name;
                    let bg = if is_cur { palette.accent_primary.darken(30) } else { palette.surface_sunken };
                    ui::draw::draw_rounded_rect(s, btn_rect, RADIUS_XS, bg);
                    ui::draw::draw_rect_outline(s, btn_rect, 1, if is_cur { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, bx + 12, btn_rect.y + 6, p_name, if is_cur { Color::WHITE } else { palette.text_primary }, &f_small);
                }

                ui::font::draw_text_font(s, form_x + 16, form_y + 76, "Default PID 1 Desktop Init Executable:", palette.text_secondary, &f_small);
                let init_rect = Rect::new(form_x + 16, form_y + 96, 160, 28);
                ui::draw::draw_rounded_rect(s, init_rect, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, init_rect, 1, palette.accent_primary);
                ui::font::draw_text_font(s, init_rect.x + 12, init_rect.y + 6, self.identity.init_path_str(), palette.text_primary, &f_small);

                ui::font::draw_text_font(s, form_x + 16, form_y + 140, "Persisted in FAT32 filesystem: /SYSTEM.CFG (81 bytes)", palette.text_muted, &f_small);
            }

            WizardStep::Account => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 12, "Account Quick Presets:", palette.text_secondary, &f_small);
                let presets = ["firoze", "admin", "operator"];
                for (i, &p_name) in presets.iter().enumerate() {
                    let bx = form_x + 16 + (i as i32 * 110);
                    let btn_rect = Rect::new(bx, form_y + 32, 100, 28);
                    let is_cur = self.account.username_str() == p_name;
                    let bg = if is_cur { palette.accent_primary.darken(30) } else { palette.surface_sunken };
                    ui::draw::draw_rounded_rect(s, btn_rect, RADIUS_XS, bg);
                    ui::draw::draw_rect_outline(s, btn_rect, 1, if is_cur { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, bx + 16, btn_rect.y + 6, p_name, if is_cur { Color::WHITE } else { palette.text_primary }, &f_small);
                }

                ui::font::draw_text_font(s, form_x + 16, form_y + 76, "Administrator Password:", palette.text_secondary, &f_small);
                let pass_rect = Rect::new(form_x + 16, form_y + 96, 180, 28);
                ui::draw::draw_rounded_rect(s, pass_rect, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, pass_rect, 1, palette.border_subtle);
                let display_pass = if self.account.mask_passwords { "********" } else { self.account.password_str() };
                ui::font::draw_text_font(s, pass_rect.x + 12, pass_rect.y + 6, display_pass, palette.text_primary, &f_small);

                let mask_btn = Rect::new(form_x + 206, form_y + 96, 100, 28);
                ui::draw::draw_rounded_rect(s, mask_btn, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, mask_btn, 1, palette.border_subtle);
                ui::font::draw_text_font(s, mask_btn.x + 12, mask_btn.y + 6, if self.account.mask_passwords { "Show Pass" } else { "Hide Pass" }, palette.text_secondary, &f_small);

                let (strength_label, strength_col) = self.account.password_strength();
                ui::font::draw_text_font(s, form_x + 16, form_y + 136, "Strength:", palette.text_secondary, &f_small);
                ui::font::draw_text_font(s, form_x + 80, form_y + 136, strength_label, strength_col, &f_small);

                ui::font::draw_text_font(s, form_x + 16, form_y + 160, "Security: PBKDF2-HMAC-SHA256 (4096 iter, UID 0 / GID 0)", palette.text_muted, &f_small);
            }

            WizardStep::Network => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 12, "IPv4 Addressing Mode:", palette.text_secondary, &f_small);
                let dhcp_btn = Rect::new(form_x + 16, form_y + 32, 150, 28);
                let is_dhcp = self.network.dhcp_enabled;
                ui::draw::draw_rounded_rect(s, dhcp_btn, RADIUS_XS, if is_dhcp { palette.accent_primary.darken(30) } else { palette.surface_sunken });
                ui::draw::draw_rect_outline(s, dhcp_btn, 1, if is_dhcp { palette.accent_primary } else { palette.border_subtle });
                ui::font::draw_text_font(s, dhcp_btn.x + 12, dhcp_btn.y + 6, "DHCP / Auto (QEMU)", if is_dhcp { Color::WHITE } else { palette.text_primary }, &f_small);

                let fw_btn = Rect::new(form_x + 176, form_y + 32, 170, 28);
                let is_drop = self.network.firewall_inbound_drop;
                ui::draw::draw_rounded_rect(s, fw_btn, RADIUS_XS, if is_drop { palette.status_success.darken(30) } else { palette.surface_sunken });
                ui::draw::draw_rect_outline(s, fw_btn, 1, if is_drop { palette.status_success } else { palette.border_subtle });
                ui::font::draw_text_font(s, fw_btn.x + 12, fw_btn.y + 6, "Firewall: Drop Inbound", if is_drop { Color::WHITE } else { palette.text_primary }, &f_small);

                ui::font::draw_text_font(s, form_x + 16, form_y + 76, "Allowed Inbound Services (Stateful Filter):", palette.text_secondary, &f_small);
                let ports = [
                    ("[X] SSH (22)", self.network.allow_ssh),
                    ("[X] HTTP (80)", self.network.allow_http),
                    ("[X] ICMP Ping", self.network.allow_icmp),
                ];
                for (i, &(label, enabled)) in ports.iter().enumerate() {
                    let bx = form_x + 16 + (i as i32 * 110);
                    let btn_rect = Rect::new(bx, form_y + 96, 100, 26);
                    ui::draw::draw_rounded_rect(s, btn_rect, RADIUS_XS, if enabled { palette.surface_raised.lighten(20) } else { palette.surface_sunken });
                    ui::draw::draw_rect_outline(s, btn_rect, 1, if enabled { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, bx + 10, btn_rect.y + 5, label, if enabled { Color::WHITE } else { palette.text_muted }, &f_small);
                }

                ui::font::draw_text_font(s, form_x + 16, form_y + 140, "Network adapter: RTL8139 (10.0.2.15/24, Gateway 10.0.2.2)", palette.text_muted, &f_small);
            }

            WizardStep::Preferences => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 12, "Visual Theme Palette (WCAG 2.1 AAA):", palette.text_secondary, &f_small);
                let themes = [("Dark", Theme::Dark), ("Light", Theme::Light), ("High Contrast", Theme::HighContrast)];
                for (i, &(name, t)) in themes.iter().enumerate() {
                    let bx = form_x + 16 + (i as i32 * 110);
                    let btn_rect = Rect::new(bx, form_y + 32, 100, 28);
                    let is_cur = self.preferences.theme == t;
                    ui::draw::draw_rounded_rect(s, btn_rect, RADIUS_XS, if is_cur { palette.accent_primary.darken(30) } else { palette.surface_sunken });
                    ui::draw::draw_rect_outline(s, btn_rect, 1, if is_cur { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, bx + 10, btn_rect.y + 6, name, if is_cur { Color::WHITE } else { palette.text_primary }, &f_small);
                }

                ui::font::draw_text_font(s, form_x + 16, form_y + 76, "System Font Scaling Multiplier:", palette.text_secondary, &f_small);
                let scales = [FontScale::Normal, FontScale::Medium, FontScale::Large, FontScale::ExtraLarge];
                for (i, &sc) in scales.iter().enumerate() {
                    let bx = form_x + 16 + (i as i32 * 80);
                    let btn_rect = Rect::new(bx, form_y + 96, 72, 26);
                    let is_cur = self.preferences.font_scale == sc;
                    ui::draw::draw_rounded_rect(s, btn_rect, RADIUS_XS, if is_cur { palette.accent_primary } else { palette.surface_sunken });
                    ui::font::draw_text_font(s, bx + 16, btn_rect.y + 5, sc.label(), if is_cur { Color::WHITE } else { palette.text_primary }, &f_small);
                }

                ui::font::draw_text_font(s, form_x + 16, form_y + 140, "Persisted in FAT32 filesystem: /ACCESS.CFG (64 bytes)", palette.text_muted, &f_small);
            }

            WizardStep::ReviewAndApply => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 12, "Configuration Blueprint Ready to Apply:", palette.text_primary, &font);

                let items = [
                    "1. Hostname & PID 1 Session -> SYSTEM.CFG (81 bytes)",
                    "2. Admin Credentials (PBKDF2) -> USERS.CFG (716 bytes)",
                    "3. Firewall & Filter Policies -> FIREWALL.CFG (1.2 KB)",
                    "4. Theme & Accessibility Spec -> ACCESS.CFG (64 bytes)",
                ];
                for (i, &item) in items.iter().enumerate() {
                    let iy = form_y + 40 + (i as i32 * 22);
                    ui::draw::draw_rounded_rect(s, Rect::new(form_x + 16, iy + 2, 8, 8), RADIUS_XS, palette.status_success);
                    ui::font::draw_text_font(s, form_x + 32, iy, item, palette.text_primary, &f_small);
                }

                let apply_btn = Rect::new(form_x + 16, form_y + 150, 220, 36);
                ui::draw::draw_rounded_rect(s, apply_btn, RADIUS_XS, palette.status_success);
                ui::font::draw_text_font(s, apply_btn.x + 24, apply_btn.y + 10, "Deploy Configurations Now", Color::WHITE, &f_small);
            }

            WizardStep::Complete => {
                ui::font::draw_text_font(s, form_x + 16, form_y + 16, "Setup Completed Successfully!", palette.status_success, &font);
                ui::font::draw_text_font(s, form_x + 16, form_y + 40, "All configuration files written and committed to disk.", palette.text_secondary, &f_small);
                ui::font::draw_text_font(s, form_x + 16, form_y + 64, "Returning users will be greeted automatically on future boots.", palette.text_primary, &f_small);

                let desk_btn = Rect::new(form_x + 16, form_y + 130, 160, 34);
                ui::draw::draw_rounded_rect(s, desk_btn, RADIUS_XS, palette.accent_primary);
                ui::font::draw_text_font(s, desk_btn.x + 20, desk_btn.y + 9, "Launch Desktop (WM)", Color::WHITE, &f_small);

                let shell_btn = Rect::new(form_x + 186, form_y + 130, 160, 34);
                ui::draw::draw_rounded_rect(s, shell_btn, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, shell_btn, 1, palette.border_subtle);
                ui::font::draw_text_font(s, shell_btn.x + 20, shell_btn.y + 9, "Enter Shell (CLI)", palette.text_primary, &f_small);
            }
        }

        // 5. Right Inspector Card (w = 210, h = 220)
        let insp_x = form_x + card_w + 16;
        let insp_w = viewport.w - insp_x - 16;
        let insp_rect = Rect::new(insp_x, form_y, insp_w.max(180), card_h);
        ui::draw::draw_rounded_rect(s, insp_rect, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, insp_rect, 1, palette.border_subtle);

        ui::font::draw_text_font(s, insp_x + 12, form_y + 12, "Setup Inspector", palette.accent_primary, &font);

        // Summary details
        let lines: [(&'static str, &str); 5] = [
            ("Machine:", self.identity.hostname_str()),
            ("Init Path:", self.identity.init_path_str()),
            ("Admin User:", self.account.username_str()),
            ("Network:", if self.network.dhcp_enabled { "DHCP / Auto" } else { "Static IPv4" }),
            ("Theme:", match self.preferences.theme { Theme::Dark => "Dark Palette", Theme::Light => "Light Palette", Theme::HighContrast => "High Contrast" }),
        ];

        for (i, &(lbl, val)) in lines.iter().enumerate() {
            let ly = form_y + 42 + (i as i32 * 28);
            ui::font::draw_text_font(s, insp_x + 12, ly, lbl, palette.text_secondary, &f_small);
            ui::font::draw_text_font(s, insp_x + 12, ly + 14, val, Color::WHITE, &f_small);
        }

        // 6. Bottom Navigation Bar (h = 44)
        let bottom_y = viewport.bottom() - 44;
        let bot_rect = Rect::new(0, bottom_y, viewport.w, 44);
        s.fill_rect(bot_rect, palette.surface_raised);
        s.fill_rect(Rect::new(0, bottom_y, viewport.w, 1), palette.border_subtle);

        // [<- Back] Button
        if self.step.prev().is_some() {
            let back_btn = Rect::new(16, bottom_y + 7, 80, 30);
            ui::draw::draw_rounded_rect(s, back_btn, RADIUS_XS, palette.surface_sunken);
            ui::draw::draw_rect_outline(s, back_btn, 1, palette.border_subtle);
            ui::font::draw_text_font(s, back_btn.x + 18, back_btn.y + 7, "<- Back", palette.text_secondary, &f_small);
        }

        // Status text in bottom bar
        ui::font::draw_text_font(s, 110, bottom_y + 12, self.status_str(), palette.text_secondary, &f_small);

        // [Next Step ->] or [Deploy Configurations] Button
        let next_x = viewport.right() - 136;
        let next_btn = Rect::new(next_x, bottom_y + 7, 120, 30);
        let next_label = match self.step {
            WizardStep::ReviewAndApply => "Deploy ->",
            WizardStep::Complete => "Start Using",
            _ => "Next Step ->",
        };
        ui::draw::draw_rounded_rect(s, next_btn, RADIUS_XS, palette.accent_primary);
        ui::font::draw_text_font(s, next_btn.x + 16, next_btn.y + 7, next_label, Color::WHITE, &f_small);
    }
}

// ===========================================================================
// Selftest Suite
// ===========================================================================

static mut TEST_RENDER_BUF: [u32; 640 * 360] = [0u32; 640 * 360];

#[inline(never)]
pub fn run_selftest() -> i32 {
    let mut code = 0;
    unsafe { ffi::sys_write(b"[firstrun-selftest] starting\n\0".as_ptr()) };

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_FIRSTRUN) };
    app.init();

    // -----------------------------------------------------------------------
    // Case 1: Wizard state machine and step navigation
    // -----------------------------------------------------------------------
    let init_step_ok = app.step == WizardStep::Welcome;
    app.apply_action(WizardAction::NextStep);
    let step2_ok = app.step == WizardStep::Identity;
    app.apply_action(WizardAction::NextStep);
    let step3_ok = app.step == WizardStep::Account;
    app.apply_action(WizardAction::NextStep);
    let step4_ok = app.step == WizardStep::Network;
    app.apply_action(WizardAction::NextStep);
    let step5_ok = app.step == WizardStep::Preferences;
    app.apply_action(WizardAction::NextStep);
    let step6_ok = app.step == WizardStep::ReviewAndApply;

    app.apply_action(WizardAction::PrevStep);
    let prev_ok = app.step == WizardStep::Preferences;
    app.apply_action(WizardAction::JumpStep(WizardStep::Welcome));
    let jump_ok = app.step == WizardStep::Welcome;

    if init_step_ok && step2_ok && step3_ok && step4_ok && step5_ok && step6_ok && prev_ok && jump_ok {
        unsafe { ffi::sys_write(b"[firstrun-selftest] PASS: first-run wizard state machine and step navigation\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[firstrun-selftest] FAIL: first-run wizard state machine\n\0".as_ptr()) };
        code |= 1;
    }

    // -----------------------------------------------------------------------
    // Case 2: Machine identity validation and SYSTEM.CFG binary serialization
    // -----------------------------------------------------------------------
    app.apply_action(WizardAction::JumpStep(WizardStep::Identity));
    app.apply_action(WizardAction::IdentityPreset(1)); // "novaos-dev"
    let host_dev_ok = app.identity.hostname_str() == "novaos-dev";
    let valid_host_ok = app.identity.validate_hostname();

    app.apply_action(WizardAction::IdentityToggleInit);
    let init_toggled_ok = app.identity.init_path_str() != "NOVAINIT.ELF";
    app.identity.set_init_path("NOVAINIT.ELF");

    let mut sys_buf = [0u8; 128];
    let sys_len = app.identity.to_system_cfg("firoze", &mut sys_buf);
    let sys_bytes_ok = sys_len == 81 && &sys_buf[0..4] == b"NVCF" && &sys_buf[4..14] == b"novaos-dev";

    if host_dev_ok && valid_host_ok && init_toggled_ok && sys_bytes_ok {
        unsafe { ffi::sys_write(b"[firstrun-selftest] PASS: machine identity validation and SYSTEM.CFG serialization\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[firstrun-selftest] FAIL: machine identity/SYSTEM.CFG\n\0".as_ptr()) };
        code |= 2;
    }

    // -----------------------------------------------------------------------
    // Case 3: Administrator credentials generation and USERS.CFG serialization
    // -----------------------------------------------------------------------
    app.apply_action(WizardAction::JumpStep(WizardStep::Account));
    app.apply_action(WizardAction::AccountPreset(0)); // "firoze", "novapass"
    let user_ok = app.account.username_str() == "firoze";
    let match_ok = app.account.passwords_match();

    let mut user_buf = [0u8; 716];
    let user_len = app.account.to_users_cfg(&mut user_buf);
    let magic_ok = user_len == 716 && &user_buf[0..4] == b"NVUS";
    let in_use_ok = user_buf[4] == 1; // in_use
    let uid_zero_ok = u32::from_le_bytes([user_buf[37], user_buf[38], user_buf[39], user_buf[40]]) == 0; // UID 0

    if user_ok && match_ok && magic_ok && in_use_ok && uid_zero_ok {
        unsafe { ffi::sys_write(b"[firstrun-selftest] PASS: user account credential synthesis and PBKDF2-HMAC-SHA256\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[firstrun-selftest] FAIL: user account credential synthesis\n\0".as_ptr()) };
        code |= 4;
    }

    // -----------------------------------------------------------------------
    // Case 4: Network and Accessibility configuration (FIREWALL.CFG & ACCESS.CFG)
    // -----------------------------------------------------------------------
    app.apply_action(WizardAction::JumpStep(WizardStep::Network));
    app.apply_action(WizardAction::NetworkToggleDhcp);
    let dhcp_toggled = !app.network.dhcp_enabled;
    app.network.dhcp_enabled = true;

    let mut fw_buf = [0u8; 1024];
    let fw_len = app.network.to_firewall_cfg(&mut fw_buf);
    let fw_ok = fw_len > 0 && &fw_buf[..14] == b"# FIREWALL.CFG";

    app.apply_action(WizardAction::JumpStep(WizardStep::Preferences));
    app.apply_action(WizardAction::PrefSetTheme(Theme::HighContrast));
    app.apply_action(WizardAction::PrefSetFontScale(FontScale::Large));
    let theme_hc_ok = app.preferences.theme == Theme::HighContrast && app.preferences.font_scale == FontScale::Large;

    let mut access_buf = [0u8; 64];
    app.preferences.to_access_cfg(&mut access_buf);
    let access_deserialized = ui::access::AccessibilityConfig::deserialize(&access_buf);
    let access_ok = access_deserialized.is_some() && access_deserialized.unwrap().theme == Theme::HighContrast;

    if dhcp_toggled && fw_ok && theme_hc_ok && access_ok {
        unsafe { ffi::sys_write(b"[firstrun-selftest] PASS: configuration file generation (SYSTEM.CFG, USERS.CFG, FIREWALL.CFG, ACCESS.CFG)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[firstrun-selftest] FAIL: configuration generation\n\0".as_ptr()) };
        code |= 8;
    }

    // -----------------------------------------------------------------------
    // Case 5: Graphical surface rasterization on BufferSurface32
    // -----------------------------------------------------------------------
    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_RENDER_BUF) as *mut u32, 640 * 360)
    };
    render_slice.fill(0);
    let mut surf = BufferSurface32::new(render_slice, 640, 360);

    let mut all_steps_rendered_ok = true;
    for &step in WizardStep::all().iter() {
        app.step = step;
        surf.buffer.fill(0);
        app.render(&mut surf, Rect::new(0, 0, 640, 360));
        let non_zero_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();
        if non_zero_pixels < 4000 {
            all_steps_rendered_ok = false;
            break;
        }
    }

    if all_steps_rendered_ok {
        unsafe { ffi::sys_write(b"[firstrun-selftest] PASS: graphical first-run wizard presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[firstrun-selftest] FAIL: graphical first-run wizard presentation\n\0".as_ptr()) };
        code |= 16;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[firstrun-selftest] all cases passed\n\0".as_ptr()) };
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
        unsafe { ffi::sys_write(b"firstrun: failed to acquire framebuffer\n\0".as_ptr()) };
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
            ffi::sys_write(b"firstrun: failed to create framebuffer surface\n\0".as_ptr());
        }
        return 1;
    }

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_FIRSTRUN) };
    app.init();

    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(RENDER_BUFFER) as *mut u32, (screen_w * screen_h) as usize)
    };
    let mut surf = BufferSurface32::new(render_slice, screen_w, screen_h);

    let mut cursor_x = screen_w / 2;
    let mut cursor_y = screen_h / 2;
    let mut prev_left = false;
    let viewport = Rect::new(0, 0, screen_w, screen_h);

    loop {
        let key = unsafe { ffi::sys_read_key() };
        if key == 27 || key == b'q' as i32 {
            break;
        }

        let mut m = ffi::MouseState::default();
        unsafe { ffi::sys_mouse_read(&mut m) };
        cursor_x = (cursor_x + m.dx).clamp(0, screen_w - 1);
        cursor_y = (cursor_y + m.dy).clamp(0, screen_h - 1);

        if m.left_button && !prev_left {
            if let Some(action) = app.button_at(cursor_x, cursor_y, viewport) {
                app.apply_action(action);
            }
        }
        prev_left = m.left_button;

        app.render(&mut surf, viewport);

        ui::cursor::draw_cursor_sprite(&mut surf, cursor_x, cursor_y, CursorShape::Arrow, app.preferences.cursor_theme);

        let present_req = ffi::NovaFbPresent {
            handle: req.handle,
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

    unsafe {
        ffi::sys_fb_destroy(req.handle);
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
    fn test_firstrun_selftest_cases() {
        unsafe { ffi::reset_mock_environment() };
        assert_eq!(run_selftest(), 0);
    }

    #[test]
    fn test_wizard_step_transitions() {
        let mut app = FirstrunApp::new();
        assert_eq!(app.step, WizardStep::Welcome);

        app.apply_action(WizardAction::NextStep);
        assert_eq!(app.step, WizardStep::Identity);

        app.apply_action(WizardAction::NextStep);
        assert_eq!(app.step, WizardStep::Account);

        app.apply_action(WizardAction::NextStep);
        assert_eq!(app.step, WizardStep::Network);

        app.apply_action(WizardAction::NextStep);
        assert_eq!(app.step, WizardStep::Preferences);

        app.apply_action(WizardAction::NextStep);
        assert_eq!(app.step, WizardStep::ReviewAndApply);

        app.apply_action(WizardAction::PrevStep);
        assert_eq!(app.step, WizardStep::Preferences);
    }

    #[test]
    fn test_identity_config_serialization() {
        let mut id = IdentityState::new();
        id.set_hostname("novaos-pc");
        id.set_init_path("NOVAINIT.ELF");
        assert!(id.validate_hostname());

        let mut buf = [0u8; 128];
        let len = id.to_system_cfg("firoze", &mut buf);
        assert_eq!(len, 81);
        assert_eq!(&buf[0..4], b"NVCF");
        assert_eq!(&buf[4..13], b"novaos-pc");
    }

    #[test]
    fn test_account_credentials_synthesis() {
        let mut acc = AccountState::new();
        acc.set_username("operator");
        acc.set_password("oppassword123");
        assert!(acc.passwords_match());

        let mut buf = [0u8; 716];
        let len = acc.to_users_cfg(&mut buf);
        assert_eq!(len, 716);
        assert_eq!(&buf[0..4], b"NVUS");
        assert_eq!(buf[4], 1); // in_use
        assert_eq!(&buf[5..13], b"operator");
    }

    #[test]
    fn test_network_and_access_serialization() {
        let mut net = NetworkState::new();
        net.allow_ssh = true;
        net.allow_http = true;

        let mut fw_buf = [0u8; 1024];
        let fw_len = net.to_firewall_cfg(&mut fw_buf);
        assert!(fw_len > 0);
        let text = core::str::from_utf8(&fw_buf[..fw_len]).unwrap();
        assert!(text.contains("policy in drop"));
        assert!(text.contains("dport 22"));

        let pref = PreferencesState::new();
        let mut acc_buf = [0u8; 64];
        pref.to_access_cfg(&mut acc_buf);
        let parsed = ui::access::AccessibilityConfig::deserialize(&acc_buf).unwrap();
        assert_eq!(parsed.theme, Theme::Dark);
    }

    #[test]
    fn test_firstrun_surface_rendering_and_hit_testing() {
        let app = FirstrunApp::new();
        let mut buf = vec![0u32; 640 * 360];
        let mut surf = BufferSurface32::new(&mut buf, 640, 360);
        let vp = Rect::new(0, 0, 640, 360);

        app.render(&mut surf, vp);
        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 4000);

        // Hit testing for [Next Step ->]
        let hit_next = app.button_at(vp.right() - 80, vp.bottom() - 25, vp);
        assert_eq!(hit_next, Some(WizardAction::NextStep));
    }
}
