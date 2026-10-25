//! userland/settings-rs/settings.rs - NovaOS Graphical Settings App (Month 3 Core Desktop Apps)
//!
//! A modular plugin-panel configuration application for NovaOS.
//! Serves as the central shell that every future system preferences panel plugs into.
//!
//! Features:
//!   - Pluggable panel architecture supporting independent settings categories.
//!   - Core built-in panels:
//!       1. System & Identity: reads and writes `SYSTEM.CFG` (hostname, default user, init path).
//!       2. Accounts & Users: reads and writes `USERS.CFG` (UID/GID records, PBKDF2 salt/hash).
//!       3. Services & Startup: reads and writes `SERVICES.CFG` (supervised daemons, restart policies).
//!       4. Network & Firewall: reads and writes `FIREWALL.CFG` (inbound policy, connection rules).
//!       5. Display & Themes: desktop appearance, cursor themes, window chrome styles.
//!   - Interactive mouse navigation: sidebar category switching, button actions, toggles.
//!   - High-contrast visual design conforming to NovaOS Unified Design System (`ui-rs`).
//!   - Zero-allocation rendering into `BufferSurface32` with zero per-pixel syscalls.
//!   - Standalone framebuffer presentation and deterministic `--selftest` verification mode.

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
        let msg = b"settings: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

pub const MAX_PANELS: usize = 5;
pub const FILE_IO_BUF_LEN: usize = 4096;

static mut FILE_IO_BUF: [u8; FILE_IO_BUF_LEN] = [0; FILE_IO_BUF_LEN];
static mut RENDER_BUFFER: [u32; 1024 * 768] = [0; 1024 * 768];

// ===========================================================================
// Configuration Data Models & Serialization
// ===========================================================================

/// Matches `kernel/config/sysconfig.h` (81 bytes packed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemConfig {
    pub magic: [u8; 4], // "NVCF"
    pub hostname: [u8; 32],
    pub username: [u8; 32],
    pub init_path: [u8; 13],
}

impl SystemConfig {
    pub fn new() -> Self {
        let mut cfg = SystemConfig {
            magic: *b"NVCF",
            hostname: [0; 32],
            username: [0; 32],
            init_path: [0; 13],
        };
        cfg.set_hostname("novaos");
        cfg.set_username("firoze");
        cfg.set_init_path("SHELL.ELF");
        cfg
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 81 {
            return None;
        }
        if &bytes[0..4] != b"NVCF" {
            return None;
        }
        let mut cfg = SystemConfig {
            magic: *b"NVCF",
            hostname: [0; 32],
            username: [0; 32],
            init_path: [0; 13],
        };
        cfg.hostname.copy_from_slice(&bytes[4..36]);
        cfg.username.copy_from_slice(&bytes[36..68]);
        cfg.init_path.copy_from_slice(&bytes[68..81]);
        Some(cfg)
    }

    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        if out.len() < 81 {
            return 0;
        }
        out[0..4].copy_from_slice(&self.magic);
        out[4..36].copy_from_slice(&self.hostname);
        out[36..68].copy_from_slice(&self.username);
        out[68..81].copy_from_slice(&self.init_path);
        81
    }

    pub fn hostname_str(&self) -> &str {
        let len = self.hostname.iter().position(|&c| c == 0).unwrap_or(32);
        core::str::from_utf8(&self.hostname[..len]).unwrap_or("novaos")
    }

    pub fn username_str(&self) -> &str {
        let len = self.username.iter().position(|&c| c == 0).unwrap_or(32);
        core::str::from_utf8(&self.username[..len]).unwrap_or("firoze")
    }

    pub fn init_path_str(&self) -> &str {
        let len = self.init_path.iter().position(|&c| c == 0).unwrap_or(13);
        core::str::from_utf8(&self.init_path[..len]).unwrap_or("SHELL.ELF")
    }

    pub fn set_hostname(&mut self, name: &str) {
        self.hostname.fill(0);
        let b = name.as_bytes();
        let len = b.len().min(31);
        self.hostname[..len].copy_from_slice(&b[..len]);
    }

    pub fn set_username(&mut self, name: &str) {
        self.username.fill(0);
        let b = name.as_bytes();
        let len = b.len().min(31);
        self.username[..len].copy_from_slice(&b[..len]);
    }

    pub fn set_init_path(&mut self, path: &str) {
        self.init_path.fill(0);
        let b = path.as_bytes();
        let len = b.len().min(12);
        self.init_path[..len].copy_from_slice(&b[..len]);
    }
}

/// Matches `kernel/config/userscfg.h` account record (89 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserAccount {
    pub in_use: bool,
    pub username: [u8; 32],
    pub username_len: u8,
    pub uid: u32,
    pub gid: u32,
    pub salt: [u8; 16],
    pub hash: [u8; 32],
}

impl UserAccount {
    pub fn new(name: &str, uid: u32, gid: u32) -> Self {
        let mut acc = UserAccount {
            in_use: true,
            username: [0; 32],
            username_len: 0,
            uid,
            gid,
            salt: [0x5A; 16],
            hash: [0xA5; 32],
        };
        let b = name.as_bytes();
        let len = b.len().min(31);
        acc.username[..len].copy_from_slice(&b[..len]);
        acc.username_len = len as u8;
        acc
    }

    pub fn username_str(&self) -> &str {
        core::str::from_utf8(&self.username[..self.username_len as usize]).unwrap_or("")
    }
}

pub const MAX_USERS: usize = 8;
pub const USER_RECORD_SIZE: usize = 1 + 32 + 4 + 4 + 16 + 32; // 89 bytes
pub const USERS_TOTAL_SIZE: usize = 4 + (USER_RECORD_SIZE * MAX_USERS); // 716 bytes ("NVUS" + 712)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsersConfig {
    pub accounts: [UserAccount; MAX_USERS],
    pub count: usize,
}

impl UsersConfig {
    pub fn new() -> Self {
        let mut cfg = UsersConfig {
            accounts: [UserAccount {
                in_use: false,
                username: [0; 32],
                username_len: 0,
                uid: 0,
                gid: 0,
                salt: [0; 16],
                hash: [0; 32],
            }; MAX_USERS],
            count: 0,
        };
        cfg.add_user("root", 0, 0);
        cfg.add_user("firoze", 1000, 1000);
        cfg
    }

    pub fn add_user(&mut self, name: &str, uid: u32, gid: u32) -> bool {
        if self.count >= MAX_USERS {
            return false;
        }
        self.accounts[self.count] = UserAccount::new(name, uid, gid);
        self.count += 1;
        true
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < USERS_TOTAL_SIZE || &bytes[0..4] != b"NVUS" {
            return None;
        }
        let mut cfg = UsersConfig {
            accounts: [UserAccount {
                in_use: false,
                username: [0; 32],
                username_len: 0,
                uid: 0,
                gid: 0,
                salt: [0; 16],
                hash: [0; 32],
            }; MAX_USERS],
            count: 0,
        };
        for i in 0..MAX_USERS {
            let base = 4 + i * USER_RECORD_SIZE;
            let ulen = bytes[base] as usize;
            if ulen > 0 && ulen <= 32 {
                let mut acc = UserAccount {
                    in_use: true,
                    username: [0; 32],
                    username_len: ulen as u8,
                    uid: u32::from_le_bytes([bytes[base+33], bytes[base+34], bytes[base+35], bytes[base+36]]),
                    gid: u32::from_le_bytes([bytes[base+37], bytes[base+38], bytes[base+39], bytes[base+40]]),
                    salt: [0; 16],
                    hash: [0; 32],
                };
                acc.username[..ulen].copy_from_slice(&bytes[base+1..base+1+ulen]);
                acc.salt.copy_from_slice(&bytes[base+41..base+57]);
                acc.hash.copy_from_slice(&bytes[base+57..base+89]);
                cfg.accounts[cfg.count] = acc;
                cfg.count += 1;
            }
        }
        Some(cfg)
    }

    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        if out.len() < USERS_TOTAL_SIZE {
            return 0;
        }
        out[0..4].copy_from_slice(b"NVUS");
        out[4..USERS_TOTAL_SIZE].fill(0);
        for i in 0..self.count {
            let base = 4 + i * USER_RECORD_SIZE;
            let acc = &self.accounts[i];
            out[base] = acc.username_len;
            out[base + 1..base + 1 + 32].copy_from_slice(&acc.username);
            let off = base + 33;
            out[off..off + 4].copy_from_slice(&acc.uid.to_le_bytes());
            out[off + 4..off + 8].copy_from_slice(&acc.gid.to_le_bytes());
            out[off + 8..off + 24].copy_from_slice(&acc.salt);
            out[off + 24..off + 56].copy_from_slice(&acc.hash);
        }
        USERS_TOTAL_SIZE
    }
}

/// Restart policy for supervised services in `SERVICES.CFG`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Always,
    Never,
    OnCrash,
}

impl RestartPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            RestartPolicy::Always => "always",
            RestartPolicy::Never => "never",
            RestartPolicy::OnCrash => "on-crash",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "always" => RestartPolicy::Always,
            "on-crash" => RestartPolicy::OnCrash,
            _ => RestartPolicy::Never,
        }
    }
}

/// Matches `tools/fixtures/SERVICES.CFG` pipe-delimited line format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceRecord {
    pub name: [u8; 24],
    pub name_len: usize,
    pub exec_path: [u8; 16],
    pub exec_len: usize,
    pub arg: [u8; 16],
    pub arg_len: usize,
    pub depends_on: [u8; 24],
    pub dep_len: usize,
    pub restart: RestartPolicy,
    pub trusted: bool,
}

impl ServiceRecord {
    pub fn new(name: &str, exec: &str, arg: &str, dep: &str, restart: RestartPolicy, trusted: bool) -> Self {
        let mut rec = ServiceRecord {
            name: [0; 24],
            name_len: 0,
            exec_path: [0; 16],
            exec_len: 0,
            arg: [0; 16],
            arg_len: 0,
            depends_on: [0; 24],
            dep_len: 0,
            restart,
            trusted,
        };
        let nb = name.as_bytes();
        let nl = nb.len().min(23);
        rec.name[..nl].copy_from_slice(&nb[..nl]);
        rec.name_len = nl;

        let eb = exec.as_bytes();
        let el = eb.len().min(15);
        rec.exec_path[..el].copy_from_slice(&eb[..el]);
        rec.exec_len = el;

        let ab = arg.as_bytes();
        let al = ab.len().min(15);
        rec.arg[..al].copy_from_slice(&ab[..al]);
        rec.arg_len = al;

        let db = dep.as_bytes();
        let dl = db.len().min(23);
        rec.depends_on[..dl].copy_from_slice(&db[..dl]);
        rec.dep_len = dl;

        rec
    }

    pub fn name_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    pub fn exec_str(&self) -> &str {
        core::str::from_utf8(&self.exec_path[..self.exec_len]).unwrap_or("")
    }

    pub fn arg_str(&self) -> &str {
        core::str::from_utf8(&self.arg[..self.arg_len]).unwrap_or("-")
    }

    pub fn dep_str(&self) -> &str {
        core::str::from_utf8(&self.depends_on[..self.dep_len]).unwrap_or("-")
    }
}

pub const MAX_SERVICES: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServicesConfig {
    pub services: [ServiceRecord; MAX_SERVICES],
    pub count: usize,
}

impl ServicesConfig {
    pub fn new() -> Self {
        let mut cfg = ServicesConfig {
            services: [ServiceRecord {
                name: [0; 24],
                name_len: 0,
                exec_path: [0; 16],
                exec_len: 0,
                arg: [0; 16],
                arg_len: 0,
                depends_on: [0; 24],
                dep_len: 0,
                restart: RestartPolicy::Never,
                trusted: false,
            }; MAX_SERVICES],
            count: 0,
        };
        cfg.add("network-check", "PING.ELF", "10.0.2.2", "-", RestartPolicy::Always, false);
        cfg.add("greeting", "HELLO.ELF", "-", "network-check", RestartPolicy::Never, false);
        cfg.add("shell", "SHELL.ELF", "-", "-", RestartPolicy::Always, true);
        cfg
    }

    pub fn add(&mut self, name: &str, exec: &str, arg: &str, dep: &str, restart: RestartPolicy, trusted: bool) -> bool {
        if self.count >= MAX_SERVICES {
            return false;
        }
        self.services[self.count] = ServiceRecord::new(name, exec, arg, dep, restart, trusted);
        self.count += 1;
        true
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut cfg = ServicesConfig {
            services: [ServiceRecord {
                name: [0; 24],
                name_len: 0,
                exec_path: [0; 16],
                exec_len: 0,
                arg: [0; 16],
                arg_len: 0,
                depends_on: [0; 24],
                dep_len: 0,
                restart: RestartPolicy::Never,
                trusted: false,
            }; MAX_SERVICES],
            count: 0,
        };

        let mut pos = 0;
        let mut line_start = 0;
        while pos < bytes.len() {
            if bytes[pos] == b'\n' || pos == bytes.len() - 1 {
                let end = if bytes[pos] == b'\n' { pos } else { pos + 1 };
                let line = &bytes[line_start..end];
                line_start = pos + 1;

                if !line.is_empty() && !line.starts_with(b"name|") {
                    let mut fields = [b"".as_slice(); 6];
                    let mut f_idx = 0;
                    let mut f_start = 0;
                    for i in 0..line.len() {
                        if line[i] == b'|' {
                            if f_idx < 6 {
                                fields[f_idx] = &line[f_start..i];
                                f_idx += 1;
                            }
                            f_start = i + 1;
                        }
                    }
                    if f_idx < 6 && f_start <= line.len() {
                        fields[f_idx] = &line[f_start..line.len()];
                        f_idx += 1;
                    }
                    if f_idx >= 6 {
                        let name = core::str::from_utf8(fields[0]).unwrap_or("");
                        let exec = core::str::from_utf8(fields[1]).unwrap_or("");
                        let arg = core::str::from_utf8(fields[2]).unwrap_or("-");
                        let dep = core::str::from_utf8(fields[3]).unwrap_or("-");
                        let restart = RestartPolicy::parse(core::str::from_utf8(fields[4]).unwrap_or("never"));
                        let trusted = fields[5] == b"yes";
                        cfg.add(name, exec, arg, dep, restart, trusted);
                    }
                }
            }
            pos += 1;
        }
        Some(cfg)
    }

    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        let header = b"name|exec_path|arg|depends_on|restart|trusted\n";
        if out.len() < header.len() {
            return 0;
        }
        out[..header.len()].copy_from_slice(header);
        let mut cur = header.len();

        for i in 0..self.count {
            let s = &self.services[i];
            let parts: &[&[u8]] = &[
                &s.name[..s.name_len],
                b"|",
                &s.exec_path[..s.exec_len],
                b"|",
                &s.arg[..s.arg_len],
                b"|",
                &s.depends_on[..s.dep_len],
                b"|",
                s.restart.as_str().as_bytes(),
                b"|",
                if s.trusted { b"yes\n" } else { b"no\n" },
            ];
            for part in parts {
                if cur + part.len() > out.len() {
                    return cur;
                }
                out[cur..cur + part.len()].copy_from_slice(part);
                cur += part.len();
            }
        }
        cur
    }
}

/// Matches `tools/fixtures/FIREWALL.CFG` rules summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirewallConfig {
    pub inbound_drop: bool,
    pub allow_established: bool,
    pub allow_ping: bool,
    pub allow_dns: bool,
}

impl FirewallConfig {
    pub fn new() -> Self {
        FirewallConfig {
            inbound_drop: true,
            allow_established: true,
            allow_ping: true,
            allow_dns: true,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let text = core::str::from_utf8(bytes).unwrap_or("");
        let mut cfg = FirewallConfig::new();
        cfg.inbound_drop = text.contains("policy in drop");
        cfg.allow_established = text.contains("established,related");
        cfg.allow_ping = text.contains("icmp");
        cfg.allow_dns = text.contains("53");
        Some(cfg)
    }

    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        let header = b"policy in ";
        let policy_str: &[u8] = if self.inbound_drop { b"drop\npolicy out accept\n\n" } else { b"accept\npolicy out accept\n\n" };
        let mut cur = 0;
        if out.len() < 256 {
            return 0;
        }
        out[cur..cur + header.len()].copy_from_slice(header);
        cur += header.len();
        out[cur..cur + policy_str.len()].copy_from_slice(policy_str);
        cur += policy_str.len();

        if self.allow_established {
            let r = b"rule accept in state established,related\n";
            out[cur..cur + r.len()].copy_from_slice(r);
            cur += r.len();
        }
        if self.allow_ping {
            let r = b"rule accept in proto icmp icmp-type 8 from 10.0.2.0/24 state new\n";
            out[cur..cur + r.len()].copy_from_slice(r);
            cur += r.len();
        }
        if self.allow_dns {
            let r = b"rule accept in proto udp sport 53 state established\n";
            out[cur..cur + r.len()].copy_from_slice(r);
            cur += r.len();
        }
        cur
    }
}

/// Display and appearance settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayConfig {
    pub theme: Theme,
    pub cursor_theme: CursorTheme,
    pub titlebar_sheen: bool,
    pub window_shadows: bool,
}

impl DisplayConfig {
    pub fn new() -> Self {
        DisplayConfig {
            theme: Theme::Dark,
            cursor_theme: CursorTheme::ModernDark,
            titlebar_sheen: true,
            window_shadows: true,
        }
    }
}

// ===========================================================================
// Plugin Panel Enumeration & Architecture
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    System = 0,
    Users = 1,
    Services = 2,
    Firewall = 3,
    Display = 4,
}

impl PanelKind {
    pub fn all() -> [PanelKind; 5] {
        [
            PanelKind::System,
            PanelKind::Users,
            PanelKind::Services,
            PanelKind::Firewall,
            PanelKind::Display,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            PanelKind::System   => "System & Identity",
            PanelKind::Users    => "User Accounts",
            PanelKind::Services => "Startup & Services",
            PanelKind::Firewall => "Network & Firewall",
            PanelKind::Display  => "Display & Appearance",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            PanelKind::System   => "Hostname, root user, and PID 1 init program configuration (SYSTEM.CFG)",
            PanelKind::Users    => "Local accounts, UID/GID mapping, PBKDF2 salt and hash records (USERS.CFG)",
            PanelKind::Services => "Service supervisor daemons, restart policies, and dependency tree (SERVICES.CFG)",
            PanelKind::Firewall => "Stateful packet filter rules, default traffic policies, and CIDR masks (FIREWALL.CFG)",
            PanelKind::Display  => "Theme palettes, compositor visual effects, and cursor bitmap styles",
        }
    }

    pub fn icon_badge(&self) -> &'static str {
        match self {
            PanelKind::System   => "SYS",
            PanelKind::Users    => "USR",
            PanelKind::Services => "SVC",
            PanelKind::Firewall => "FW",
            PanelKind::Display  => "DISP",
        }
    }
}

/// Action triggers for interactive button hit-testing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsAction {
    SelectPanel(PanelKind),
    SaveCurrent,
    ReloadAll,
    SystemToggleInit,
    SystemPresetHostname(usize),
    UsersAddDemo,
    ServicesToggleRestart(usize),
    FirewallToggleInbound,
    FirewallTogglePing,
    DisplayCycleTheme,
    DisplayCycleCursor,
}

// ===========================================================================
// Settings Application Container
// ===========================================================================

pub struct SettingsApp {
    pub active_panel: PanelKind,
    pub system_cfg: SystemConfig,
    pub users_cfg: UsersConfig,
    pub services_cfg: ServicesConfig,
    pub firewall_cfg: FirewallConfig,
    pub display_cfg: DisplayConfig,
    pub status_text: [u8; 64],
    pub status_len: usize,
    pub hovered_btn: Option<SettingsAction>,
    pub selected_user_row: usize,
    pub selected_service_row: usize,
    pub dirty: bool,
}

impl SettingsApp {
    pub fn new() -> Self {
        let mut app = SettingsApp {
            active_panel: PanelKind::System,
            system_cfg: SystemConfig::new(),
            users_cfg: UsersConfig::new(),
            services_cfg: ServicesConfig::new(),
            firewall_cfg: FirewallConfig::new(),
            display_cfg: DisplayConfig::new(),
            status_text: [0; 64],
            status_len: 0,
            hovered_btn: None,
            selected_user_row: 0,
            selected_service_row: 0,
            dirty: false,
        };
        app.set_status("Settings shell ready");
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

    pub fn select_panel(&mut self, panel: PanelKind) {
        self.active_panel = panel;
        self.set_status(panel.title());
    }

    // -----------------------------------------------------------------------
    // VFS Persistence
    // -----------------------------------------------------------------------

    pub fn load_system_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let fd = unsafe { ffi::sys_open(b"SYSTEM.CFG\0".as_ptr()) };
        if fd < 0 {
            self.set_status("Failed to open SYSTEM.CFG");
            return false;
        }
        let n = unsafe { ffi::sys_read(fd, buf_ptr, 128) };
        unsafe { ffi::sys_close(fd) };
        if n >= 81 {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, n as usize) };
            if let Some(cfg) = SystemConfig::from_bytes(slice) {
                self.system_cfg = cfg;
                self.set_status("SYSTEM.CFG loaded (81 B)");
                return true;
            }
        }
        self.set_status("Invalid SYSTEM.CFG format");
        false
    }

    pub fn save_system_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr, 128) };
        let len = self.system_cfg.to_bytes(slice);
        if len > 0 {
            let rc = unsafe { ffi::sys_write_file(b"SYSTEM.CFG\0".as_ptr(), buf_ptr, len as u32) };
            if rc >= 0 {
                self.set_status("Saved SYSTEM.CFG (81 B)");
                self.dirty = false;
                return true;
            }
        }
        self.set_status("Failed to write SYSTEM.CFG");
        false
    }

    pub fn load_users_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let fd = unsafe { ffi::sys_open(b"USERS.CFG\0".as_ptr()) };
        if fd < 0 {
            self.set_status("Failed to open USERS.CFG");
            return false;
        }
        let n = unsafe { ffi::sys_read(fd, buf_ptr, USERS_TOTAL_SIZE as i32 + 10) };
        unsafe { ffi::sys_close(fd) };
        if n >= USERS_TOTAL_SIZE as i32 {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, n as usize) };
            if let Some(cfg) = UsersConfig::from_bytes(slice) {
                self.users_cfg = cfg;
                self.set_status("USERS.CFG loaded");
                return true;
            }
        }
        self.set_status("Invalid USERS.CFG format");
        false
    }

    pub fn save_users_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr, USERS_TOTAL_SIZE) };
        let len = self.users_cfg.to_bytes(slice);
        if len > 0 {
            let rc = unsafe { ffi::sys_write_file(b"USERS.CFG\0".as_ptr(), buf_ptr, len as u32) };
            if rc >= 0 {
                self.set_status("Saved USERS.CFG (716 B)");
                self.dirty = false;
                return true;
            }
        }
        self.set_status("Failed to write USERS.CFG");
        false
    }

    pub fn load_services_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let fd = unsafe { ffi::sys_open(b"SERVICES.CFG\0".as_ptr()) };
        if fd < 0 {
            self.set_status("Failed to open SERVICES.CFG");
            return false;
        }
        let n = unsafe { ffi::sys_read(fd, buf_ptr, 1024) };
        unsafe { ffi::sys_close(fd) };
        if n > 0 {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, n as usize) };
            if let Some(cfg) = ServicesConfig::from_bytes(slice) {
                self.services_cfg = cfg;
                self.set_status("SERVICES.CFG loaded");
                return true;
            }
        }
        self.set_status("Invalid SERVICES.CFG format");
        false
    }

    pub fn save_services_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr, 1024) };
        let len = self.services_cfg.to_bytes(slice);
        if len > 0 {
            let rc = unsafe { ffi::sys_write_file(b"SERVICES.CFG\0".as_ptr(), buf_ptr, len as u32) };
            if rc >= 0 {
                self.set_status("Saved SERVICES.CFG");
                self.dirty = false;
                return true;
            }
        }
        self.set_status("Failed to write SERVICES.CFG");
        false
    }

    pub fn load_firewall_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let fd = unsafe { ffi::sys_open(b"FIREWALL.CFG\0".as_ptr()) };
        if fd < 0 {
            self.set_status("Failed to open FIREWALL.CFG");
            return false;
        }
        let n = unsafe { ffi::sys_read(fd, buf_ptr, 1024) };
        unsafe { ffi::sys_close(fd) };
        if n > 0 {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, n as usize) };
            if let Some(cfg) = FirewallConfig::from_bytes(slice) {
                self.firewall_cfg = cfg;
                self.set_status("FIREWALL.CFG loaded");
                return true;
            }
        }
        self.set_status("Invalid FIREWALL.CFG format");
        false
    }

    pub fn save_firewall_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr, 1024) };
        let len = self.firewall_cfg.to_bytes(slice);
        if len > 0 {
            let rc = unsafe { ffi::sys_write_file(b"FIREWALL.CFG\0".as_ptr(), buf_ptr, len as u32) };
            if rc >= 0 {
                self.set_status("Saved FIREWALL.CFG");
                self.dirty = false;
                return true;
            }
        }
        self.set_status("Failed to write FIREWALL.CFG");
        false
    }

    pub fn save_current_panel(&mut self) -> bool {
        match self.active_panel {
            PanelKind::System   => self.save_system_cfg(),
            PanelKind::Users    => self.save_users_cfg(),
            PanelKind::Services => self.save_services_cfg(),
            PanelKind::Firewall => self.save_firewall_cfg(),
            PanelKind::Display  => {
                self.set_status("Display preferences applied");
                self.dirty = false;
                true
            }
        }
    }

    pub fn reload_all(&mut self) {
        self.load_system_cfg();
        self.load_users_cfg();
        self.load_services_cfg();
        self.load_firewall_cfg();
        self.set_status("Reloaded all configuration files");
    }

    // -----------------------------------------------------------------------
    // Layout and Mouse Hit-Testing
    // -----------------------------------------------------------------------

    pub fn button_at(&self, px: i32, py: i32, viewport: Rect) -> Option<SettingsAction> {
        let sb_w = 160;

        // 1. Sidebar Category Tabs
        if px >= viewport.x && px < viewport.x + sb_w {
            let tab_y_start = viewport.y + 44;
            let tab_h = 36;
            if py >= tab_y_start && py < tab_y_start + (tab_h * 5) {
                let idx = ((py - tab_y_start) / tab_h) as usize;
                if idx < 5 {
                    return Some(SettingsAction::SelectPanel(PanelKind::all()[idx]));
                }
            }
            return None;
        }

        // 2. Top Header Action Buttons (`[Save]`, `[Reload]`)
        let hdr_y = viewport.y + 8;
        let hdr_h = 28;
        if py >= hdr_y && py < hdr_y + hdr_h {
            let save_btn = Rect::new(viewport.right() - 80, hdr_y, 70, hdr_h);
            if point_in_rect(px, py, save_btn.x, save_btn.y, save_btn.w, save_btn.h) {
                return Some(SettingsAction::SaveCurrent);
            }
            let reload_btn = Rect::new(save_btn.x - 76, hdr_y, 70, hdr_h);
            if point_in_rect(px, py, reload_btn.x, reload_btn.y, reload_btn.w, reload_btn.h) {
                return Some(SettingsAction::ReloadAll);
            }
        }

        // 3. Active Panel Specific Interactive Controls
        let content_x = viewport.x + sb_w + 16;
        let content_y = viewport.y + 48 + 12;

        match self.active_panel {
            PanelKind::System => {
                // Preset hostname pills: [novaos] (x+0), [novaos-dev] (x+70), [novaos-srv] (x+155)
                let pill_y = content_y + 42;
                if py >= pill_y && py < pill_y + 24 {
                    if point_in_rect(px, py, content_x, pill_y, 64, 24) {
                        return Some(SettingsAction::SystemPresetHostname(0));
                    }
                    if point_in_rect(px, py, content_x + 70, pill_y, 80, 24) {
                        return Some(SettingsAction::SystemPresetHostname(1));
                    }
                    if point_in_rect(px, py, content_x + 156, pill_y, 80, 24) {
                        return Some(SettingsAction::SystemPresetHostname(2));
                    }
                }
                // Init program toggle pill [SHELL.ELF] vs [NOVAINIT.ELF]
                let init_y = content_y + 112;
                if py >= init_y && py < init_y + 24 {
                    if point_in_rect(px, py, content_x, init_y, 140, 24) {
                        return Some(SettingsAction::SystemToggleInit);
                    }
                }
            }
            PanelKind::Users => {
                // Add demo user button
                let add_y = content_y + 200;
                if point_in_rect(px, py, content_x, add_y, 110, 26) {
                    return Some(SettingsAction::UsersAddDemo);
                }
            }
            PanelKind::Services => {
                // Toggle restart policy on services table rows
                let table_y = content_y + 36;
                let row_h = 24;
                for i in 0..self.services_cfg.count {
                    let ry = table_y + (i as i32 * row_h);
                    if py >= ry && py < ry + row_h {
                        // Click on restart column (x + 220..x + 300)
                        if px >= content_x + 220 && px < content_x + 310 {
                            return Some(SettingsAction::ServicesToggleRestart(i));
                        }
                    }
                }
            }
            PanelKind::Firewall => {
                // Inbound policy pill
                let pol_y = content_y + 40;
                if point_in_rect(px, py, content_x, pol_y, 110, 26) {
                    return Some(SettingsAction::FirewallToggleInbound);
                }
                // Ping ICMP toggle
                let ping_y = content_y + 104;
                if point_in_rect(px, py, content_x, ping_y, 200, 24) {
                    return Some(SettingsAction::FirewallTogglePing);
                }
            }
            PanelKind::Display => {
                // Theme cycle button
                let th_y = content_y + 40;
                if point_in_rect(px, py, content_x, th_y, 130, 26) {
                    return Some(SettingsAction::DisplayCycleTheme);
                }
                // Cursor cycle button
                let cur_y = content_y + 112;
                if point_in_rect(px, py, content_x, cur_y, 130, 26) {
                    return Some(SettingsAction::DisplayCycleCursor);
                }
            }
        }

        None
    }

    pub fn cursor_shape_at(&self, px: i32, py: i32, viewport: Rect) -> CursorShape {
        if self.button_at(px, py, viewport).is_some() {
            CursorShape::Pointer
        } else {
            CursorShape::Arrow
        }
    }

    pub fn on_left_click(&mut self, px: i32, py: i32, viewport: Rect) {
        if let Some(action) = self.button_at(px, py, viewport) {
            match action {
                SettingsAction::SelectPanel(panel) => {
                    self.select_panel(panel);
                }
                SettingsAction::SaveCurrent => {
                    self.save_current_panel();
                }
                SettingsAction::ReloadAll => {
                    self.reload_all();
                }
                SettingsAction::SystemToggleInit => {
                    if self.system_cfg.init_path_str() == "SHELL.ELF" {
                        self.system_cfg.set_init_path("NOVAINIT.ELF");
                    } else {
                        self.system_cfg.set_init_path("SHELL.ELF");
                    }
                    self.dirty = true;
                    self.set_status("Toggled init program");
                }
                SettingsAction::SystemPresetHostname(idx) => {
                    let presets = ["novaos", "novaos-dev", "novaos-srv"];
                    if idx < presets.len() {
                        self.system_cfg.set_hostname(presets[idx]);
                        self.dirty = true;
                        self.set_status("Updated hostname preset");
                    }
                }
                SettingsAction::UsersAddDemo => {
                    if self.users_cfg.count < MAX_USERS {
                        let uid = 1000 + self.users_cfg.count as u32;
                        self.users_cfg.add_user("guest", uid, uid);
                        self.dirty = true;
                        self.set_status("Added guest user account");
                    } else {
                        self.set_status("User table full (8 max)");
                    }
                }
                SettingsAction::ServicesToggleRestart(idx) => {
                    if idx < self.services_cfg.count {
                        let cur = self.services_cfg.services[idx].restart;
                        let next = match cur {
                            RestartPolicy::Always  => RestartPolicy::OnCrash,
                            RestartPolicy::OnCrash => RestartPolicy::Never,
                            RestartPolicy::Never   => RestartPolicy::Always,
                        };
                        self.services_cfg.services[idx].restart = next;
                        self.dirty = true;
                        self.set_status("Toggled service restart policy");
                    }
                }
                SettingsAction::FirewallToggleInbound => {
                    self.firewall_cfg.inbound_drop = !self.firewall_cfg.inbound_drop;
                    self.dirty = true;
                    self.set_status("Toggled inbound firewall policy");
                }
                SettingsAction::FirewallTogglePing => {
                    self.firewall_cfg.allow_ping = !self.firewall_cfg.allow_ping;
                    self.dirty = true;
                    self.set_status("Toggled ICMP ping rule");
                }
                SettingsAction::DisplayCycleTheme => {
                    self.display_cfg.theme = match self.display_cfg.theme {
                        Theme::Dark => Theme::Light,
                        Theme::Light => Theme::HighContrast,
                        Theme::HighContrast => Theme::Dark,
                    };
                    self.dirty = true;
                    self.set_status("Theme changed");
                }
                SettingsAction::DisplayCycleCursor => {
                    self.display_cfg.cursor_theme = match self.display_cfg.cursor_theme {
                        CursorTheme::ModernDark => CursorTheme::Classic,
                        CursorTheme::Classic => CursorTheme::Amber,
                        CursorTheme::Amber => CursorTheme::ModernLight,
                        CursorTheme::ModernLight => CursorTheme::ModernDark,
                    };
                    self.dirty = true;
                    self.set_status("Cursor theme changed");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Rendering Pipeline
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S, viewport: Rect) {
        if viewport.w <= 0 || viewport.h <= 0 {
            return;
        }
        let palette = self.display_cfg.theme.palette();
        let font = Font::regular();
        let f_small = Font::small();

        // 1. Shell Background
        s.fill_rect(viewport, palette.surface_sunken);

        // 2. Left Sidebar (w = 160)
        let sb_w = 160;
        let sb_rect = Rect::new(viewport.x, viewport.y, sb_w, viewport.h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(sb_rect.right() - 1, sb_rect.y, 1, sb_rect.h), palette.border_subtle);

        // Sidebar Branding Title
        ui::draw::draw_rounded_rect(s, Rect::new(viewport.x + 12, viewport.y + 12, 20, 20), RADIUS_SM, palette.accent_primary);
        ui::font::draw_char_font(s, viewport.x + 18, viewport.y + 14, b'S', Color::WHITE, &font);
        ui::font::draw_text_font(s, viewport.x + 38, viewport.y + 15, "Settings", palette.text_primary, &font);

        // Sidebar Category Tabs
        let tab_y_start = viewport.y + 48;
        let tab_h = 36;
        for (i, panel) in PanelKind::all().iter().enumerate() {
            let ty = tab_y_start + (i as i32 * tab_h);
            let tab_rect = Rect::new(viewport.x + 6, ty, sb_w - 12, tab_h - 4);
            let is_active = self.active_panel == *panel;

            if is_active {
                ui::draw::draw_rounded_rect(s, tab_rect, RADIUS_XS, palette.accent_primary.darken(35));
                ui::draw::draw_rect_outline(s, tab_rect, 1, palette.border_focused);
                // Active indicator pill on left edge
                s.fill_rect(Rect::new(tab_rect.x + 2, tab_rect.y + 4, 3, tab_rect.h - 8), palette.accent_primary);
            }

            // Category badge pill
            let badge = panel.icon_badge();
            let b_sz = f_small.measure_str(badge);
            let b_rect = Rect::new(tab_rect.x + 8, tab_rect.y + 4, b_sz.w + 6, tab_rect.h - 8);
            let badge_bg = if is_active { palette.accent_primary } else { palette.surface_sunken };
            ui::draw::draw_rounded_rect(s, b_rect, 2, badge_bg);
            ui::font::draw_text_font(s, b_rect.x + 3, b_rect.y + 3, badge, Color::WHITE, &f_small);

            // Category name
            let text_col = if is_active { Color::WHITE } else { palette.text_primary };
            ui::font::draw_text_font(s, b_rect.right() + 6, tab_rect.y + 8, panel.title(), text_col, &f_small);
        }

        // 3. Top Header Bar (h = 48)
        let hdr_x = viewport.x + sb_w;
        let hdr_w = viewport.w - sb_w;
        let hdr_rect = Rect::new(hdr_x, viewport.y, hdr_w, 48);
        s.fill_rect(hdr_rect, palette.surface_raised);
        s.fill_rect(Rect::new(hdr_x, hdr_rect.bottom() - 1, hdr_w, 1), palette.border_subtle);

        // Header Titles
        ui::font::draw_text_font(s, hdr_x + 16, viewport.y + 8, self.active_panel.title(), palette.text_primary, &font);
        ui::font::draw_text_font(s, hdr_x + 16, viewport.y + 28, self.active_panel.subtitle(), palette.text_muted, &f_small);

        // Action Buttons: [Save Changes], [Reload]
        let save_col = if self.dirty { palette.status_warning } else { palette.accent_primary };
        let save_rect = Rect::new(viewport.right() - 80, viewport.y + 10, 70, 26);
        ui::draw::draw_rounded_rect(s, save_rect, RADIUS_XS, save_col);
        ui::draw::draw_rect_outline(s, save_rect, 1, palette.border_focused);
        let save_sz = f_small.measure_str("Save");
        ui::font::draw_text_font(s, save_rect.x + (save_rect.w - save_sz.w)/2, save_rect.y + 5, "Save", Color::WHITE, &f_small);

        let reload_rect = Rect::new(save_rect.x - 76, viewport.y + 10, 70, 26);
        ui::draw::draw_rounded_rect(s, reload_rect, RADIUS_XS, palette.surface_raised.lighten(30));
        ui::draw::draw_rect_outline(s, reload_rect, 1, palette.border_subtle);
        let rel_sz = f_small.measure_str("Reload");
        ui::font::draw_text_font(s, reload_rect.x + (reload_rect.w - rel_sz.w)/2, reload_rect.y + 5, "Reload", palette.text_primary, &f_small);

        // 4. Panel Content Area
        let content_x = hdr_x + 16;
        let content_y = viewport.y + 48 + 12;

        match self.active_panel {
            PanelKind::System => {
                ui::font::draw_text_font(s, content_x, content_y, "Hostname Identification", palette.accent_primary, &font);
                ui::font::draw_text_font(s, content_x, content_y + 20, "Configured node name on the local subnet:", palette.text_secondary, &f_small);

                // Current Hostname Display Card
                let host_card = Rect::new(content_x, content_y + 38, 240, 28);
                ui::draw::draw_rounded_rect(s, host_card, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, host_card, 1, palette.border_subtle);
                ui::font::draw_text_font(s, host_card.x + 8, host_card.y + 6, self.system_cfg.hostname_str(), palette.text_primary, &font);

                // Presets
                ui::font::draw_text_font(s, content_x + 252, content_y + 44, "Presets: [novaos]  [novaos-dev]  [novaos-srv]", palette.text_muted, &f_small);

                // Init Program Selector
                ui::font::draw_text_font(s, content_x, content_y + 86, "PID 1 Startup Supervisor (init_path)", palette.accent_primary, &font);
                ui::font::draw_text_font(s, content_x, content_y + 106, "Program exec'd by the kernel at boot completion:", palette.text_secondary, &f_small);

                let init_card = Rect::new(content_x, content_y + 124, 150, 28);
                ui::draw::draw_rounded_rect(s, init_card, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, init_card, 1, palette.accent_primary);
                ui::font::draw_text_font(s, init_card.x + 10, init_card.y + 6, self.system_cfg.init_path_str(), palette.accent_primary, &font);

                ui::font::draw_text_font(s, content_x + 160, content_y + 130, "(Click to toggle between SHELL.ELF and NOVAINIT.ELF)", palette.text_muted, &f_small);

                // Default User
                ui::font::draw_text_font(s, content_x, content_y + 172, "Primary Administrative User", palette.accent_primary, &font);
                let user_str = self.system_cfg.username_str();
                ui::font::draw_text_font(s, content_x, content_y + 194, user_str, palette.text_primary, &font);
            }

            PanelKind::Users => {
                ui::font::draw_text_font(s, content_x, content_y, "Registered User Accounts (USERS.CFG)", palette.accent_primary, &font);

                // Accounts Table Header
                let th_y = content_y + 24;
                s.fill_rect(Rect::new(content_x, th_y, 400, 20), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 8, th_y + 3, "Username", palette.text_muted, &f_small);
                ui::font::draw_text_font(s, content_x + 140, th_y + 3, "UID", palette.text_muted, &f_small);
                ui::font::draw_text_font(s, content_x + 200, th_y + 3, "GID", palette.text_muted, &f_small);
                ui::font::draw_text_font(s, content_x + 260, th_y + 3, "Auth & Security", palette.text_muted, &f_small);

                let mut ry = th_y + 22;
                for i in 0..self.users_cfg.count {
                    let u = &self.users_cfg.accounts[i];
                    let r_rect = Rect::new(content_x, ry, 400, 22);
                    if i % 2 == 1 {
                        s.fill_rect(r_rect, palette.surface_raised.darken(10));
                    }
                    ui::font::draw_text_font(s, content_x + 8, ry + 4, u.username_str(), palette.text_primary, &f_small);

                    let mut uid_buf = [0u8; 8];
                    let uid_len = format_u32(u.uid, &mut uid_buf);
                    let uid_s = core::str::from_utf8(&uid_buf[..uid_len]).unwrap_or("0");
                    ui::font::draw_text_font(s, content_x + 140, ry + 4, uid_s, palette.text_secondary, &f_small);

                    let mut gid_buf = [0u8; 8];
                    let gid_len = format_u32(u.gid, &mut gid_buf);
                    let gid_s = core::str::from_utf8(&gid_buf[..gid_len]).unwrap_or("0");
                    ui::font::draw_text_font(s, content_x + 200, ry + 4, gid_s, palette.text_secondary, &f_small);

                    ui::font::draw_text_font(s, content_x + 260, ry + 4, "PBKDF2 / Salted", palette.status_success, &f_small);
                    ry += 24;
                }

                // Add User Button
                let add_btn = Rect::new(content_x, ry + 12, 110, 26);
                ui::draw::draw_rounded_rect(s, add_btn, RADIUS_XS, palette.surface_raised.lighten(25));
                ui::draw::draw_rect_outline(s, add_btn, 1, palette.border_subtle);
                ui::font::draw_text_font(s, add_btn.x + 14, add_btn.y + 5, "+ Add Guest", palette.text_primary, &f_small);
            }

            PanelKind::Services => {
                ui::font::draw_text_font(s, content_x, content_y, "Supervised Daemons (SERVICES.CFG)", palette.accent_primary, &font);

                // Services Table Header
                let th_y = content_y + 24;
                s.fill_rect(Rect::new(content_x, th_y, 440, 20), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 8, th_y + 3, "Service Name", palette.text_muted, &f_small);
                ui::font::draw_text_font(s, content_x + 130, th_y + 3, "Binary", palette.text_muted, &f_small);
                ui::font::draw_text_font(s, content_x + 230, th_y + 3, "Restart", palette.text_muted, &f_small);
                ui::font::draw_text_font(s, content_x + 330, th_y + 3, "Privileges", palette.text_muted, &f_small);

                let mut ry = th_y + 22;
                for i in 0..self.services_cfg.count {
                    let svc = &self.services_cfg.services[i];
                    let r_rect = Rect::new(content_x, ry, 440, 22);
                    if i % 2 == 1 {
                        s.fill_rect(r_rect, palette.surface_raised.darken(10));
                    }
                    ui::font::draw_text_font(s, content_x + 8, ry + 4, svc.name_str(), palette.text_primary, &f_small);
                    ui::font::draw_text_font(s, content_x + 130, ry + 4, svc.exec_str(), palette.text_secondary, &f_small);

                    // Clickable restart pill
                    let r_pill = Rect::new(content_x + 226, ry + 2, 70, 18);
                    let r_col = match svc.restart {
                        RestartPolicy::Always => palette.status_success,
                        RestartPolicy::OnCrash => palette.status_warning,
                        RestartPolicy::Never => palette.text_muted,
                    };
                    ui::draw::draw_rounded_rect(s, r_pill, 2, r_col.darken(30));
                    ui::font::draw_text_font(s, r_pill.x + 8, r_pill.y + 2, svc.restart.as_str(), r_col, &f_small);

                    let priv_text = if svc.trusted { "Trusted (CAP)" } else { "Sandboxed" };
                    let priv_col = if svc.trusted { palette.accent_primary } else { palette.text_muted };
                    ui::font::draw_text_font(s, content_x + 330, ry + 4, priv_text, priv_col, &f_small);
                    ry += 24;
                }
            }

            PanelKind::Firewall => {
                ui::font::draw_text_font(s, content_x, content_y, "Packet Filtering & Firewall (FIREWALL.CFG)", palette.accent_primary, &font);

                // Default Inbound Policy Pill
                ui::font::draw_text_font(s, content_x, content_y + 24, "Default Inbound Policy (Unmatched packets):", palette.text_secondary, &f_small);
                let pol_btn = Rect::new(content_x, content_y + 44, 110, 26);
                let pol_col = if self.firewall_cfg.inbound_drop { palette.status_danger } else { palette.status_success };
                ui::draw::draw_rounded_rect(s, pol_btn, RADIUS_XS, pol_col.darken(30));
                ui::draw::draw_rect_outline(s, pol_btn, 1, pol_col);
                let pol_txt = if self.firewall_cfg.inbound_drop { "DROP (Default)" } else { "ACCEPT" };
                ui::font::draw_text_font(s, pol_btn.x + 10, pol_btn.y + 5, pol_txt, Color::WHITE, &f_small);

                // Active Rules Checklist
                ui::font::draw_text_font(s, content_x, content_y + 88, "Active Stateful Connection Rules:", palette.accent_primary, &font);

                // Rule 1: Established / Related
                let c1_box = Rect::new(content_x, content_y + 110, 14, 14);
                ui::draw::draw_checkbox(s, c1_box, CheckboxState::Checked, WidgetState::Normal, "", &palette);
                ui::font::draw_text_font(s, content_x + 22, content_y + 110, "Accept in state established,related (inbound flows)", palette.text_primary, &f_small);

                // Rule 2: ICMP Ping
                let c2_box = Rect::new(content_x, content_y + 134, 14, 14);
                let c2_st = if self.firewall_cfg.allow_ping { CheckboxState::Checked } else { CheckboxState::Unchecked };
                ui::draw::draw_checkbox(s, c2_box, c2_st, WidgetState::Normal, "", &palette);
                ui::font::draw_text_font(s, content_x + 22, content_y + 134, "Accept inbound ICMP echo requests (ping)", palette.text_primary, &f_small);
            }

            PanelKind::Display => {
                ui::font::draw_text_font(s, content_x, content_y, "Visual Themes & Compositor", palette.accent_primary, &font);

                // Theme Mode
                ui::font::draw_text_font(s, content_x, content_y + 24, "Design System Theme Palette:", palette.text_secondary, &f_small);
                let th_btn = Rect::new(content_x, content_y + 44, 130, 26);
                ui::draw::draw_rounded_rect(s, th_btn, RADIUS_XS, palette.surface_raised.lighten(30));
                ui::draw::draw_rect_outline(s, th_btn, 1, palette.accent_primary);
                let th_name = match self.display_cfg.theme {
                    Theme::Dark => "Dark Palette",
                    Theme::Light => "Light Palette",
                    Theme::HighContrast => "High Contrast",
                };
                ui::font::draw_text_font(s, th_btn.x + 14, th_btn.y + 5, th_name, palette.text_primary, &f_small);

                // Cursor Theme
                ui::font::draw_text_font(s, content_x, content_y + 88, "Compositor Cursor Bitmaps:", palette.text_secondary, &f_small);
                let cur_btn = Rect::new(content_x, content_y + 108, 130, 26);
                ui::draw::draw_rounded_rect(s, cur_btn, RADIUS_XS, palette.surface_raised.lighten(30));
                ui::draw::draw_rect_outline(s, cur_btn, 1, palette.accent_primary);
                let cur_name = match self.display_cfg.cursor_theme {
                    CursorTheme::Classic => "Classic 8x12",
                    CursorTheme::ModernDark => "Modern Dark",
                    CursorTheme::ModernLight => "Modern Light",
                    CursorTheme::Amber => "Amber CRT",
                };
                ui::font::draw_text_font(s, cur_btn.x + 14, cur_btn.y + 5, cur_name, palette.text_primary, &f_small);

                // Display info
                ui::font::draw_text_font(s, content_x, content_y + 154, "Resolution: 1024x768 @ 32bpp linear framebuffer", palette.text_muted, &f_small);
            }
        }

        // 5. Bottom Status Bar (h = 24)
        let sb_h = 24;
        let stat_rect = Rect::new(hdr_x, viewport.bottom() - sb_h, hdr_w, sb_h);
        s.fill_rect(stat_rect, palette.surface_raised);
        s.fill_rect(Rect::new(hdr_x, stat_rect.y, hdr_w, 1), palette.border_subtle);

        ui::font::draw_text_font(s, stat_rect.x + 12, stat_rect.y + 5, self.status_str(), palette.text_secondary, &f_small);

        let dirty_str = if self.dirty { "Unsaved changes *" } else { "Synchronized" };
        let dirty_col = if self.dirty { palette.status_warning } else { palette.text_muted };
        let d_sz = f_small.measure_str(dirty_str);
        ui::font::draw_text_font(s, stat_rect.right() - d_sz.w - 12, stat_rect.y + 5, dirty_str, dirty_col, &f_small);
    }
}

fn format_u32(val: u32, buf: &mut [u8; 8]) -> usize {
    if val == 0 {
        buf[0] = b'0';
        return 1;
    }
    let mut tmp = [0u8; 8];
    let mut v = val;
    let mut i = 0;
    while v > 0 && i < 8 {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
        i += 1;
    }
    for j in 0..i {
        buf[j] = tmp[i - 1 - j];
    }
    i
}

fn point_in_rect(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

// ===========================================================================
// Selftest Suite
// ===========================================================================

pub fn run_selftest() -> i32 {
    let mut code = 0;
    unsafe { ffi::sys_write(b"[settings-selftest] starting\n\0".as_ptr()) };

    let mut app = SettingsApp::new();

    // Case 1: Plugin panel registration & category switching
    let all_panels = PanelKind::all();
    let panels_ok = all_panels.len() == 5
        && all_panels[0] == PanelKind::System
        && all_panels[1] == PanelKind::Users
        && all_panels[2] == PanelKind::Services
        && all_panels[3] == PanelKind::Firewall
        && all_panels[4] == PanelKind::Display;

    app.select_panel(PanelKind::Users);
    let select_users_ok = app.active_panel == PanelKind::Users;
    app.select_panel(PanelKind::Services);
    let select_services_ok = app.active_panel == PanelKind::Services;
    app.select_panel(PanelKind::System);
    let select_sys_ok = app.active_panel == PanelKind::System;

    if panels_ok && select_users_ok && select_services_ok && select_sys_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: plugin panel architecture and switching\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: plugin panel architecture\n\0".as_ptr()) };
        code |= 1;
    }

    // Case 2: SYSTEM.CFG parsing, mutation, and binary serialization
    let mut sys_buf = [0u8; 128];
    let len = app.system_cfg.to_bytes(&mut sys_buf);
    let sys_bytes_ok = len == 81 && &sys_buf[0..4] == b"NVCF";

    app.system_cfg.set_hostname("novaos-dev");
    app.system_cfg.set_init_path("NOVAINIT.ELF");
    let host_ok = app.system_cfg.hostname_str() == "novaos-dev";
    let init_ok = app.system_cfg.init_path_str() == "NOVAINIT.ELF";

    let len2 = app.system_cfg.to_bytes(&mut sys_buf);
    let parsed_sys = SystemConfig::from_bytes(&sys_buf[..len2]).unwrap();
    let roundtrip_ok = parsed_sys.hostname_str() == "novaos-dev" && parsed_sys.init_path_str() == "NOVAINIT.ELF";

    if sys_bytes_ok && host_ok && init_ok && roundtrip_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: SYSTEM.CFG serialization and deserialization\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: SYSTEM.CFG serialization\n\0".as_ptr()) };
        code |= 2;
    }

    // Case 3: USERS.CFG parsing, account addition, and binary serialization
    let mut users_buf = [0u8; USERS_TOTAL_SIZE];
    let u_len = app.users_cfg.to_bytes(&mut users_buf);
    let users_bytes_ok = u_len == USERS_TOTAL_SIZE && &users_buf[0..4] == b"NVUS";

    let add_ok = app.users_cfg.add_user("testuser", 1002, 1002);
    let u_len2 = app.users_cfg.to_bytes(&mut users_buf);
    let parsed_users = UsersConfig::from_bytes(&users_buf[..u_len2]).unwrap();
    let user_found = parsed_users.accounts.iter().take(parsed_users.count).any(|u| u.username_str() == "testuser");

    if users_bytes_ok && add_ok && user_found {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: USERS.CFG account records serialization\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: USERS.CFG serialization\n\0".as_ptr()) };
        code |= 4;
    }

    // Case 4: SERVICES.CFG pipe-delimited parsing and mutation
    let sample_services = b"name|exec_path|arg|depends_on|restart|trusted\nnetwork-check|PING.ELF|10.0.2.2|-|always|no\nshell|SHELL.ELF|-|-|always|yes\n";
    let parsed_services = ServicesConfig::from_bytes(sample_services).unwrap();
    let svc_count_ok = parsed_services.count == 2;
    let s0_ok = parsed_services.services[0].name_str() == "network-check" && parsed_services.services[0].restart == RestartPolicy::Always;

    let mut out_svc = [0u8; 512];
    let written = parsed_services.to_bytes(&mut out_svc);
    let svc_roundtrip = written > 0 && out_svc[..written].starts_with(b"name|exec_path|");

    if svc_count_ok && s0_ok && svc_roundtrip {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: SERVICES.CFG pipe-delimited parser\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: SERVICES.CFG parser\n\0".as_ptr()) };
        code |= 8;
    }

    // Case 5: Mouse hit-testing, action dispatch, and dynamic cursor shapes
    let vp = Rect::new(0, 0, 640, 480);
    // Tab 1 (Users tab: x=6..154, y=48+36=84..116)
    let hit_tab1 = app.button_at(50, 95, vp) == Some(SettingsAction::SelectPanel(PanelKind::Users));
    app.on_left_click(50, 95, vp);
    let tab1_selected = app.active_panel == PanelKind::Users;

    // Header Save Button (x=640-80=560..630, y=8..36)
    let hit_save = app.button_at(580, 20, vp) == Some(SettingsAction::SaveCurrent);

    let cursor_tab = app.cursor_shape_at(50, 95, vp);
    let cursor_btn = app.cursor_shape_at(580, 20, vp);
    let cursor_blank = app.cursor_shape_at(400, 300, vp);
    let cursor_ok = cursor_tab == CursorShape::Pointer
        && cursor_btn == CursorShape::Pointer
        && cursor_blank == CursorShape::Arrow;

    if hit_tab1 && tab1_selected && hit_save && cursor_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: mouse hit-testing and cursor shape resolution\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: mouse hit-testing\n\0".as_ptr()) };
        code |= 16;
    }

    // Case 6: Graphical surface rendering via BufferSurface32
    static mut TEST_RENDER_BUF: [u32; 640 * 240] = [0u32; 640 * 240];
    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_RENDER_BUF) as *mut u32, 640 * 240)
    };
    render_slice.fill(0);

    let mut surf = BufferSurface32::new(render_slice, 640, 240);
    app.select_panel(PanelKind::System);
    app.render(&mut surf, Rect::new(0, 0, 640, 240));

    let palette = Theme::Dark.palette();
    let sidebar_pixel = surf.buffer[10 * 640 + 10]; // inside sidebar
    let content_pixel = surf.buffer[80 * 640 + 200]; // inside content area
    let has_sidebar = sidebar_pixel == palette.surface_raised.to_xrgb();
    let has_content_bg = content_pixel == palette.surface_sunken.to_xrgb();
    let non_zero_count = surf.buffer.iter().filter(|&&p| p != 0).count();
    let render_rich = non_zero_count > 5000;

    let status_pixel = surf.buffer[235 * 640 + 200];
    let has_status = status_pixel != 0;

    if has_sidebar && has_content_bg && render_rich && has_status {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: graphical surface rendering (BufferSurface32)\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: graphical surface rendering\n\0".as_ptr()) };
        code |= 32;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[settings-selftest] all cases passed\n\0".as_ptr()) };
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
        unsafe { ffi::sys_write(b"settings: failed to acquire framebuffer\n\0".as_ptr()) };
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
            ffi::sys_write(b"settings: failed to create framebuffer surface\n\0".as_ptr());
        }
        return 1;
    }

    let mut app = SettingsApp::new();
    app.reload_all();

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
            app.on_left_click(cursor_x, cursor_y, viewport);
        }
        prev_left = m.left_button;

        app.render(&mut surf, viewport);

        // Draw cursor sprite
        let cur_shape = app.cursor_shape_at(cursor_x, cursor_y, viewport);
        ui::cursor::draw_cursor_sprite(&mut surf, cursor_x, cursor_y, cur_shape, app.display_cfg.cursor_theme);

        // Present surface to framebuffer
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
    fn test_settings_selftest_cases() {
        assert_eq!(run_selftest(), 0);
    }

    #[test]
    fn test_system_config_serialization() {
        let mut cfg = SystemConfig::new();
        assert_eq!(cfg.hostname_str(), "novaos");
        assert_eq!(cfg.username_str(), "firoze");
        assert_eq!(cfg.init_path_str(), "SHELL.ELF");

        cfg.set_hostname("novaos-test");
        cfg.set_init_path("NOVAINIT.ELF");
        assert_eq!(cfg.hostname_str(), "novaos-test");
        assert_eq!(cfg.init_path_str(), "NOVAINIT.ELF");

        let mut buf = [0u8; 128];
        let len = cfg.to_bytes(&mut buf);
        assert_eq!(len, 81);

        let parsed = SystemConfig::from_bytes(&buf[..len]).expect("should parse");
        assert_eq!(parsed.hostname_str(), "novaos-test");
        assert_eq!(parsed.init_path_str(), "NOVAINIT.ELF");
    }

    #[test]
    fn test_users_config_serialization() {
        let mut cfg = UsersConfig::new();
        assert_eq!(cfg.count, 2);
        assert_eq!(cfg.accounts[0].username_str(), "root");
        assert_eq!(cfg.accounts[1].username_str(), "firoze");

        assert!(cfg.add_user("operator", 1005, 1005));
        assert_eq!(cfg.count, 3);
        assert_eq!(cfg.accounts[2].username_str(), "operator");

        let mut buf = [0u8; USERS_TOTAL_SIZE];
        let len = cfg.to_bytes(&mut buf);
        assert_eq!(len, USERS_TOTAL_SIZE);

        let parsed = UsersConfig::from_bytes(&buf[..len]).expect("should parse");
        assert_eq!(parsed.count, 3);
        assert_eq!(parsed.accounts[2].username_str(), "operator");
    }

    #[test]
    fn test_services_config_serialization() {
        let cfg = ServicesConfig::new();
        assert_eq!(cfg.count, 3);
        assert_eq!(cfg.services[0].name_str(), "network-check");
        assert_eq!(cfg.services[1].name_str(), "greeting");
        assert_eq!(cfg.services[2].name_str(), "shell");

        let mut buf = [0u8; 1024];
        let len = cfg.to_bytes(&mut buf);
        assert!(len > 0);

        let parsed = ServicesConfig::from_bytes(&buf[..len]).expect("should parse");
        assert_eq!(parsed.count, 3);
        assert_eq!(parsed.services[0].name_str(), "network-check");
        assert_eq!(parsed.services[2].name_str(), "shell");
        assert!(parsed.services[2].trusted);
    }

    #[test]
    fn test_settings_app_surface_rendering() {
        let app = SettingsApp::new();
        let mut buf = vec![0u32; 640 * 480];
        let mut surf = BufferSurface32::new(&mut buf, 640, 480);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));

        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 1000, "Should render sidebar, panel content, and status bar");
    }
}
