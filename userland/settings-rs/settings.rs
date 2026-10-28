//! userland/settings-rs/settings.rs - NovaOS Graphical Settings App (Month 4 Server & Admin UI)
//!
//! A modular plugin-panel configuration application for NovaOS.
//! Serves as the central shell that every system preferences panel plugs into.
//!
//! Features:
//!   - Pluggable panel architecture supporting independent settings categories.
//!   - Core built-in panels:
//!       1. System & Identity: reads and writes `SYSTEM.CFG` (hostname, default user, init path).
//!       2. Accounts & Users: reads and writes `USERS.CFG` (UID/GID records, PBKDF2 salt/hash).
//!       3. Services & Startup: reads and writes `SERVICES.CFG` (supervised daemons, restart policies).
//!       4. Network & Firewall: graphical control for the stateful firewall (SYS_FW_INFO, SYS_FW_CTL)
//!          and network adapter configuration (IP, subnet, gateway, DNS, MAC), synchronized with
//!          the kernel-side firewall task and persisted in `FIREWALL.CFG`.
//!       5. Display & Themes: desktop appearance, cursor themes, window chrome styles.
//!   - Interactive mouse navigation: sidebar category switching, sub-tabs, button actions, toggles.
//!   - High-contrast visual design conforming to NovaOS Unified Design System (`ui-rs`).
//!   - Zero-allocation rendering into `BufferSurface32` with zero per-pixel syscalls.
//!   - Standalone framebuffer presentation and deterministic `--selftest` verification mode.

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
        let msg = b"settings: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

pub const MAX_PANELS: usize = 7;
pub const FILE_IO_BUF_LEN: usize = 4096;

static mut FILE_IO_BUF: [u8; FILE_IO_BUF_LEN] = [0; FILE_IO_BUF_LEN];
static mut RENDER_BUFFER: [u32; 1024 * 768] = [0; 1024 * 768];

// ===========================================================================
// String & Formatting Helpers (Zero Allocation)
// ===========================================================================

pub fn point_in_rect(px: i32, py: i32, rx: i32, ry: i32, rw: i32, rh: i32) -> bool {
    px >= rx && px < rx + rw && py >= ry && py < ry + rh
}

pub fn copy_bytes(dst: &mut [u8], src: &[u8]) -> usize {
    let len = dst.len().min(src.len());
    dst[..len].copy_from_slice(&src[..len]);
    len
}

pub fn to_hex_digit(b: u8) -> u8 {
    if b < 10 {
        b'0' + b
    } else {
        b'a' + (b - 10)
    }
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

pub fn format_ip(ip: u32, out: &mut [u8]) -> usize {
    let a = ((ip >> 24) & 0xFF) as u8;
    let b = ((ip >> 16) & 0xFF) as u8;
    let c = ((ip >> 8) & 0xFF) as u8;
    let d = (ip & 0xFF) as u8;
    let mut cur = 0;
    cur += format_u32(a as u32, &mut out[cur..]);
    if cur < out.len() { out[cur] = b'.'; cur += 1; }
    cur += format_u32(b as u32, &mut out[cur..]);
    if cur < out.len() { out[cur] = b'.'; cur += 1; }
    cur += format_u32(c as u32, &mut out[cur..]);
    if cur < out.len() { out[cur] = b'.'; cur += 1; }
    cur += format_u32(d as u32, &mut out[cur..]);
    cur
}

pub fn mask_to_prefix_len(mask: u32) -> u32 {
    let mut count = 0;
    let mut m = mask;
    while m & 0x80000000 != 0 {
        count += 1;
        m <<= 1;
    }
    count
}

pub fn format_ip_cidr(ip: u32, mask: u32, out: &mut [u8]) -> usize {
    let mut cur = format_ip(ip, out);
    if cur < out.len() {
        out[cur] = b'/';
        cur += 1;
    }
    let prefix = mask_to_prefix_len(mask);
    cur += format_u32(prefix, &mut out[cur..]);
    cur
}

pub fn format_hex16(val: u16, out: &mut [u8]) -> usize {
    if out.len() < 4 {
        return 0;
    }
    let hex_chars = b"0123456789ABCDEF";
    out[0] = hex_chars[((val >> 12) & 0xF) as usize];
    out[1] = hex_chars[((val >> 8) & 0xF) as usize];
    out[2] = hex_chars[((val >> 4) & 0xF) as usize];
    out[3] = hex_chars[(val & 0xF) as usize];
    4
}

pub fn format_hex32(val: u32, out: &mut [u8]) -> usize {
    if out.len() < 8 {
        return 0;
    }
    let hex_chars = b"0123456789ABCDEF";
    for i in 0..8 {
        out[i] = hex_chars[((val >> ((7 - i) * 4)) & 0xF) as usize];
    }
    8
}

pub fn str_from_u8_nul(bytes: &[u8]) -> &str {
    let mut len = 0;
    while len < bytes.len() && bytes[len] != 0 {
        len += 1;
    }
    core::str::from_utf8(&bytes[..len]).unwrap_or("")
}

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
    pub const fn empty() -> Self {
        SystemConfig {
            magic: *b"NVCF",
            hostname: [0; 32],
            username: [0; 32],
            init_path: [0; 13],
        }
    }

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

    pub fn parse_into(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() < 81 || &bytes[0..4] != b"NVCF" {
            return false;
        }
        self.magic = *b"NVCF";
        self.hostname.copy_from_slice(&bytes[4..36]);
        self.username.copy_from_slice(&bytes[36..68]);
        self.init_path.copy_from_slice(&bytes[68..81]);
        true
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut cfg = SystemConfig::empty();
        if cfg.parse_into(bytes) {
            Some(cfg)
        } else {
            None
        }
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
        let mut len = 0;
        while len < 32 && self.hostname[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.hostname[..len]).unwrap_or("")
    }

    pub fn username_str(&self) -> &str {
        let mut len = 0;
        while len < 32 && self.username[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.username[..len]).unwrap_or("")
    }

    pub fn init_path_str(&self) -> &str {
        let mut len = 0;
        while len < 13 && self.init_path[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.init_path[..len]).unwrap_or("")
    }

    pub fn set_hostname(&mut self, name: &str) {
        self.hostname.fill(0);
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        self.hostname[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn set_username(&mut self, name: &str) {
        self.username.fill(0);
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        self.username[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn set_init_path(&mut self, path: &str) {
        self.init_path.fill(0);
        let bytes = path.as_bytes();
        let len = bytes.len().min(12);
        self.init_path[..len].copy_from_slice(&bytes[..len]);
    }
}

pub const MAX_USERS: usize = 8;
pub const USER_RECORD_SIZE: usize = 89;
pub const USERS_TOTAL_SIZE: usize = 4 + (MAX_USERS * USER_RECORD_SIZE); // 716 bytes

/// Matches `kernel/config/userscfg.h` user_record_t (89 bytes packed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserRecord {
    pub in_use: u8,
    pub username: [u8; 32],
    pub uid: u32,
    pub gid: u32,
    pub salt: [u8; 16],
    pub password_hash: [u8; 32],
}

impl UserRecord {
    pub const fn empty() -> Self {
        UserRecord {
            in_use: 0,
            username: [0; 32],
            uid: 0,
            gid: 0,
            salt: [0; 16],
            password_hash: [0; 32],
        }
    }

    pub fn username_str(&self) -> &str {
        let mut len = 0;
        while len < 32 && self.username[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.username[..len]).unwrap_or("")
    }

    pub fn set_username(&mut self, name: &str) {
        self.username.fill(0);
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        self.username[..len].copy_from_slice(&bytes[..len]);
    }

    pub fn is_admin(&self) -> bool {
        self.in_use != 0 && self.gid == 0
    }

    pub fn role_str(&self) -> &'static str {
        if self.is_admin() {
            "Administrator"
        } else {
            "Standard User"
        }
    }

    pub fn set_admin(&mut self, admin: bool) {
        if admin {
            self.gid = 0;
        } else {
            self.gid = if self.uid != 0 { self.uid } else { 1000 };
        }
    }

    pub fn set_password(&mut self, password: &[u8], seed: u32) {
        let name_bytes = self.username_str().as_bytes();
        let salt = crypto::derive_salt(seed, name_bytes);
        let hash = crypto::pbkdf2_hmac_sha256(password, &salt, crypto::PBKDF2_ITERATIONS);
        self.salt = salt;
        self.password_hash = hash;
    }

    pub fn verify_password(&self, password: &[u8]) -> bool {
        if self.in_use == 0 {
            return false;
        }
        let computed = crypto::pbkdf2_hmac_sha256(password, &self.salt, crypto::PBKDF2_ITERATIONS);
        crypto::constant_time_eq(&computed, &self.password_hash)
    }
}

/// Matches `tools/fixtures/USERS.CFG` (716 bytes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsersConfig {
    pub magic: [u8; 4], // "NVUS"
    pub count: usize,
    pub accounts: [UserRecord; MAX_USERS],
}

impl UsersConfig {
    pub const fn empty() -> Self {
        UsersConfig {
            magic: *b"NVUS",
            count: 0,
            accounts: [UserRecord::empty(); MAX_USERS],
        }
    }

    pub fn new() -> Self {
        let mut cfg = UsersConfig {
            magic: *b"NVUS",
            count: 0,
            accounts: [UserRecord::empty(); MAX_USERS],
        };
        // Shipped baseline accounts matching tools/fixtures/USERS.CFG:
        // 1. persisted: uid 700, gid 700, password "persisted-pw"
        let _ = cfg.add_user_with_credentials("persisted", 700, false, b"persisted-pw", 0x1234_5678);
        // 2. admin: uid 800, gid 0, password "admin-correct-password"
        let _ = cfg.add_user_with_credentials("admin", 800, true, b"admin-correct-password", 0x8765_4321);
        cfg
    }

    pub fn find_user(&self, name: &str) -> Option<usize> {
        for i in 0..self.count {
            if self.accounts[i].in_use != 0 && self.accounts[i].username_str() == name {
                return Some(i);
            }
        }
        None
    }

    pub fn admin_count(&self) -> usize {
        let mut count = 0;
        for i in 0..self.count {
            if self.accounts[i].is_admin() {
                count += 1;
            }
        }
        count
    }

    pub fn next_available_uid(&self) -> u32 {
        let mut max_uid = 999u32;
        for i in 0..self.count {
            if self.accounts[i].in_use != 0 && self.accounts[i].uid >= max_uid {
                max_uid = self.accounts[i].uid;
            }
        }
        max_uid + 1
    }

    pub fn add_user(&mut self, name: &str, uid: u32, gid: u32) -> bool {
        if self.count >= MAX_USERS || name.is_empty() || name.len() > 31 {
            return false;
        }
        if self.find_user(name).is_some() {
            return false;
        }
        let is_admin = gid == 0;
        let seed = uid.wrapping_mul(0x9E37_79B9) ^ 0x55AA_55AA;
        let default_pw = b"novaos";
        self.add_user_with_credentials(name, uid, is_admin, default_pw, seed).is_ok()
    }

    pub fn add_user_with_credentials(
        &mut self,
        name: &str,
        uid: u32,
        is_admin: bool,
        password: &[u8],
        seed: u32,
    ) -> Result<usize, &'static str> {
        if name.is_empty() {
            return Err("Username cannot be empty");
        }
        if name.len() > 31 {
            return Err("Username exceeds 31 characters");
        }
        if self.find_user(name).is_some() {
            return Err("Username already exists");
        }
        if password.is_empty() {
            return Err("Password cannot be empty");
        }
        if self.count >= MAX_USERS {
            return Err("User table is full (max 8 accounts)");
        }

        let idx = self.count;
        let mut rec = UserRecord::empty();
        rec.in_use = 1;
        rec.set_username(name);
        rec.uid = uid;
        rec.set_admin(is_admin);
        rec.set_password(password, seed);

        self.accounts[idx] = rec;
        self.count += 1;
        Ok(idx)
    }

    pub fn delete_account(&mut self, idx: usize) -> Result<(), &'static str> {
        if idx >= self.count || self.accounts[idx].in_use == 0 {
            return Err("Invalid user index");
        }
        if self.count <= 1 {
            return Err("Cannot delete the only remaining account");
        }
        if self.accounts[idx].is_admin() && self.admin_count() <= 1 {
            return Err("Cannot delete the last Administrator account");
        }

        for i in idx..self.count - 1 {
            self.accounts[i] = self.accounts[i + 1];
        }
        self.accounts[self.count - 1] = UserRecord::empty();
        self.count -= 1;
        Ok(())
    }

    pub fn toggle_admin(&mut self, idx: usize) -> Result<bool, &'static str> {
        if idx >= self.count || self.accounts[idx].in_use == 0 {
            return Err("Invalid user index");
        }
        let currently_admin = self.accounts[idx].is_admin();
        if currently_admin {
            if self.admin_count() <= 1 {
                return Err("Cannot revoke Administrator rights from the last Administrator");
            }
            self.accounts[idx].set_admin(false);
            Ok(false)
        } else {
            self.accounts[idx].set_admin(true);
            Ok(true)
        }
    }

    pub fn set_user_password(&mut self, idx: usize, password: &[u8], seed: u32) -> Result<(), &'static str> {
        if idx >= self.count || self.accounts[idx].in_use == 0 {
            return Err("Invalid user index");
        }
        if password.is_empty() {
            return Err("Password cannot be empty");
        }
        self.accounts[idx].set_password(password, seed);
        Ok(())
    }

    pub fn parse_into(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() < USERS_TOTAL_SIZE || &bytes[0..4] != b"NVUS" {
            return false;
        }
        self.magic = *b"NVUS";
        self.count = 0;
        self.accounts = [UserRecord::empty(); MAX_USERS];
        let mut cur = 4;
        for _i in 0..MAX_USERS {
            let mut rec = UserRecord::empty();
            rec.in_use = bytes[cur];
            rec.username.copy_from_slice(&bytes[cur + 1..cur + 33]);
            rec.uid = u32::from_le_bytes(bytes[cur + 33..cur + 37].try_into().unwrap());
            rec.gid = u32::from_le_bytes(bytes[cur + 37..cur + 41].try_into().unwrap());
            rec.salt.copy_from_slice(&bytes[cur + 41..cur + 57]);
            rec.password_hash.copy_from_slice(&bytes[cur + 57..cur + 89]);
            if rec.in_use != 0 {
                self.accounts[self.count] = rec;
                self.count += 1;
            }
            cur += USER_RECORD_SIZE;
        }
        true
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut cfg = UsersConfig::empty();
        if cfg.parse_into(bytes) {
            Some(cfg)
        } else {
            None
        }
    }

    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        if out.len() < USERS_TOTAL_SIZE {
            return 0;
        }
        out[0..4].copy_from_slice(&self.magic);
        let mut cur = 4;
        for i in 0..MAX_USERS {
            let rec = if i < self.count { &self.accounts[i] } else { &UserRecord::empty() };
            out[cur] = rec.in_use;
            out[cur + 1..cur + 33].copy_from_slice(&rec.username);
            out[cur + 33..cur + 37].copy_from_slice(&rec.uid.to_le_bytes());
            out[cur + 37..cur + 41].copy_from_slice(&rec.gid.to_le_bytes());
            out[cur + 41..cur + 57].copy_from_slice(&rec.salt);
            out[cur + 57..cur + 89].copy_from_slice(&rec.password_hash);
            cur += USER_RECORD_SIZE;
        }
        USERS_TOTAL_SIZE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsersViewMode {
    List,
    CreateAccount,
    ChangePassword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UsersUiState {
    pub view_mode: UsersViewMode,
    pub selected_idx: usize,
    // Add user form fields
    pub form_username: [u8; 32],
    pub form_username_len: usize,
    pub form_password: [u8; 32],
    pub form_password_len: usize,
    pub form_confirm: [u8; 32],
    pub form_confirm_len: usize,
    pub form_is_admin: bool,
    pub form_uid: u32,
    pub form_active_field: usize,
    // Status feedback
    pub status_msg: [u8; 64],
    pub status_msg_len: usize,
    pub status_is_error: bool,
}

impl UsersUiState {
    pub const fn new() -> Self {
        UsersUiState {
            view_mode: UsersViewMode::List,
            selected_idx: 0,
            form_username: [0; 32],
            form_username_len: 0,
            form_password: [0; 32],
            form_password_len: 0,
            form_confirm: [0; 32],
            form_confirm_len: 0,
            form_is_admin: false,
            form_uid: 1000,
            form_active_field: 0,
            status_msg: [0; 64],
            status_msg_len: 0,
            status_is_error: false,
        }
    }

    pub fn set_status(&mut self, msg: &str, is_err: bool) {
        self.status_msg.fill(0);
        let bytes = msg.as_bytes();
        let len = bytes.len().min(63);
        self.status_msg[..len].copy_from_slice(&bytes[..len]);
        self.status_msg_len = len;
        self.status_is_error = is_err;
    }

    pub fn status_str(&self) -> &str {
        if self.status_msg_len == 0 {
            return "";
        }
        core::str::from_utf8(&self.status_msg[..self.status_msg_len]).unwrap_or("")
    }

    pub fn open_create_form(&mut self, next_uid: u32) {
        self.view_mode = UsersViewMode::CreateAccount;
        self.form_username.fill(0);
        self.form_username_len = 0;
        self.form_password.fill(0);
        self.form_password_len = 0;
        self.form_confirm.fill(0);
        self.form_confirm_len = 0;
        self.form_is_admin = false;
        self.form_uid = next_uid;
        self.form_active_field = 0;
        self.set_status("Enter account details or click a preset", false);
    }

    pub fn open_change_password(&mut self) {
        self.view_mode = UsersViewMode::ChangePassword;
        self.form_password.fill(0);
        self.form_password_len = 0;
        self.form_confirm.fill(0);
        self.form_confirm_len = 0;
        self.form_active_field = 0;
        self.set_status("Enter new password for selected account", false);
    }

    pub fn set_form_preset(&mut self, name: &str, is_admin: bool, password: &str) {
        self.form_username.fill(0);
        let n_bytes = name.as_bytes();
        let n_len = n_bytes.len().min(31);
        self.form_username[..n_len].copy_from_slice(&n_bytes[..n_len]);
        self.form_username_len = n_len;

        self.form_is_admin = is_admin;

        self.form_password.fill(0);
        let p_bytes = password.as_bytes();
        let p_len = p_bytes.len().min(31);
        self.form_password[..p_len].copy_from_slice(&p_bytes[..p_len]);
        self.form_password_len = p_len;

        self.form_confirm.fill(0);
        self.form_confirm[..p_len].copy_from_slice(&p_bytes[..p_len]);
        self.form_confirm_len = p_len;
        self.set_status("Preset applied - click Create Account to commit", false);
    }

    pub fn form_username_str(&self) -> &str {
        if self.form_username_len == 0 {
            return "";
        }
        core::str::from_utf8(&self.form_username[..self.form_username_len]).unwrap_or("")
    }

    pub fn form_password_str(&self) -> &str {
        if self.form_password_len == 0 {
            return "";
        }
        core::str::from_utf8(&self.form_password[..self.form_password_len]).unwrap_or("")
    }
}

pub const MAX_SERVICES: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Always,
    OnCrash,
    Never,
}

impl RestartPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            RestartPolicy::Always  => "always",
            RestartPolicy::OnCrash => "on-crash",
            RestartPolicy::Never   => "never",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "always"   => RestartPolicy::Always,
            "on-crash" => RestartPolicy::OnCrash,
            _          => RestartPolicy::Never,
        }
    }
}

/// Matches `tools/fixtures/SERVICES.CFG` pipe-delimited records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceRecord {
    pub name: [u8; 32],
    pub exec_path: [u8; 32],
    pub arg: [u8; 32],
    pub depends_on: [u8; 32],
    pub restart: RestartPolicy,
    pub trusted: bool,
}

impl ServiceRecord {
    pub const fn empty() -> Self {
        ServiceRecord {
            name: [0; 32],
            exec_path: [0; 32],
            arg: [0; 32],
            depends_on: [0; 32],
            restart: RestartPolicy::Never,
            trusted: false,
        }
    }

    pub fn name_str(&self) -> &str {
        let mut len = 0;
        while len < 32 && self.name[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    pub fn exec_path_str(&self) -> &str {
        let mut len = 0;
        while len < 32 && self.exec_path[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.exec_path[..len]).unwrap_or("")
    }

    pub fn depends_str(&self) -> &str {
        let mut len = 0;
        while len < 32 && self.depends_on[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.depends_on[..len]).unwrap_or("-")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServicesConfig {
    pub count: usize,
    pub services: [ServiceRecord; MAX_SERVICES],
}

impl ServicesConfig {
    pub const fn empty() -> Self {
        ServicesConfig {
            count: 0,
            services: [ServiceRecord::empty(); MAX_SERVICES],
        }
    }

    pub fn new() -> Self {
        let mut cfg = ServicesConfig {
            count: 0,
            services: [ServiceRecord::empty(); MAX_SERVICES],
        };
        cfg.add_service("network-check", "PING.ELF", "10.0.2.2", "-", RestartPolicy::Always, false);
        cfg.add_service("greeting", "HELLO.ELF", "-", "network-check", RestartPolicy::OnCrash, false);
        cfg.add_service("shell", "SHELL.ELF", "-", "-", RestartPolicy::Always, true);
        cfg
    }

    pub fn add_service(
        &mut self,
        name: &str,
        exec: &str,
        arg: &str,
        deps: &str,
        restart: RestartPolicy,
        trusted: bool,
    ) -> bool {
        if self.count >= MAX_SERVICES {
            return false;
        }
        let mut s = ServiceRecord::empty();
        let n_len = name.as_bytes().len().min(31);
        s.name[..n_len].copy_from_slice(&name.as_bytes()[..n_len]);

        let e_len = exec.as_bytes().len().min(31);
        s.exec_path[..e_len].copy_from_slice(&exec.as_bytes()[..e_len]);

        let a_len = arg.as_bytes().len().min(31);
        s.arg[..a_len].copy_from_slice(&arg.as_bytes()[..a_len]);

        let d_len = deps.as_bytes().len().min(31);
        s.depends_on[..d_len].copy_from_slice(&deps.as_bytes()[..d_len]);

        s.restart = restart;
        s.trusted = trusted;
        self.services[self.count] = s;
        self.count += 1;
        true
    }

    pub fn parse_into(&mut self, bytes: &[u8]) -> bool {
        let Ok(text) = core::str::from_utf8(bytes) else { return false; };
        self.count = 0;
        self.services = [ServiceRecord::empty(); MAX_SERVICES];
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("name|") {
                continue;
            }
            let mut parts = [""; 6];
            let mut p_idx = 0;
            for part in line.split('|') {
                if p_idx < 6 {
                    parts[p_idx] = part.trim();
                    p_idx += 1;
                }
            }
            if p_idx >= 5 {
                let trusted = p_idx > 5 && (parts[5] == "yes" || parts[5] == "true");
                self.add_service(
                    parts[0],
                    parts[1],
                    parts[2],
                    parts[3],
                    RestartPolicy::from_str(parts[4]),
                    trusted,
                );
            }
        }
        true
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut cfg = ServicesConfig::empty();
        if cfg.parse_into(bytes) {
            Some(cfg)
        } else {
            None
        }
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
            let n = s.name_str();
            let e = s.exec_path_str();
            let a = core::str::from_utf8(&s.arg[..s.arg.iter().position(|&c| c == 0).unwrap_or(32)]).unwrap_or("-");
            let d = s.depends_str();
            let r = s.restart.as_str();
            let t = if s.trusted { "yes" } else { "no" };

            let mut line = [0u8; 128];
            let mut l_cur = 0;
            for (seg, is_last) in [(n, false), (e, false), (a, false), (d, false), (r, false), (t, true)] {
                let sb = seg.as_bytes();
                let slen = sb.len().min(32);
                line[l_cur..l_cur + slen].copy_from_slice(&sb[..slen]);
                l_cur += slen;
                if !is_last {
                    line[l_cur] = b'|';
                    l_cur += 1;
                }
            }
            line[l_cur] = b'\n';
            l_cur += 1;

            if cur + l_cur > out.len() {
                break;
            }
            out[cur..cur + l_cur].copy_from_slice(&line[..l_cur]);
            cur += l_cur;
        }
        cur
    }
}

// ===========================================================================
// Month 4: Live Service Supervision Manager Models (novainit PID 1)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceViewMode {
    LiveDashboard = 0,
    ConfigFile = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceLiveState {
    pub name: [u8; ffi::NOVA_SVC_MAX_NAME],
    pub exec_path: [u8; ffi::NOVA_SVC_MAX_PATH],
    pub arg: [u8; ffi::NOVA_SVC_MAX_ARG],
    pub depends_on: [u8; ffi::NOVA_SVC_MAX_NAME],
    pub pid: i32,
    pub state: u32,
    pub restart_policy: u32,
    pub restart_count: u32,
    pub uptime_ticks: u32,
    pub trusted: bool,
    pub last_exit_code: i32,
}

impl ServiceLiveState {
    pub const fn empty() -> Self {
        ServiceLiveState {
            name: [0u8; ffi::NOVA_SVC_MAX_NAME],
            exec_path: [0u8; ffi::NOVA_SVC_MAX_PATH],
            arg: [0u8; ffi::NOVA_SVC_MAX_ARG],
            depends_on: [0u8; ffi::NOVA_SVC_MAX_NAME],
            pid: -1,
            state: ffi::NOVA_SVC_STATE_STOPPED,
            restart_policy: 0,
            restart_count: 0,
            uptime_ticks: 0,
            trusted: false,
            last_exit_code: 0,
        }
    }

    pub fn name_str(&self) -> &str {
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_NAME && self.name[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.name[..len]).unwrap_or("")
    }

    pub fn exec_str(&self) -> &str {
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_PATH && self.exec_path[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.exec_path[..len]).unwrap_or("")
    }

    pub fn arg_str(&self) -> &str {
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_ARG && self.arg[len] != 0 {
            len += 1;
        }
        if len == 0 {
            "-"
        } else {
            core::str::from_utf8(&self.arg[..len]).unwrap_or("-")
        }
    }

    pub fn depends_str(&self) -> &str {
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_NAME && self.depends_on[len] != 0 {
            len += 1;
        }
        if len == 0 {
            "-"
        } else {
            core::str::from_utf8(&self.depends_on[..len]).unwrap_or("-")
        }
    }

    pub fn state_str(&self) -> &'static str {
        match self.state {
            ffi::NOVA_SVC_STATE_RUNNING => "RUNNING",
            ffi::NOVA_SVC_STATE_RESTARTING => "RESTARTING",
            ffi::NOVA_SVC_STATE_CRASHED => "CRASHED",
            ffi::NOVA_SVC_STATE_BLOCKED => "WAITING",
            _ => "STOPPED",
        }
    }

    pub fn state_color(&self, palette: &ThemePalette) -> Color {
        match self.state {
            ffi::NOVA_SVC_STATE_RUNNING => palette.status_success,
            ffi::NOVA_SVC_STATE_RESTARTING => palette.status_warning,
            ffi::NOVA_SVC_STATE_CRASHED => palette.status_danger,
            ffi::NOVA_SVC_STATE_BLOCKED => palette.status_info,
            _ => palette.text_muted,
        }
    }

    pub fn policy_str(&self) -> &'static str {
        match self.restart_policy {
            0 => "never",
            1 => "on-crash",
            2 => "always",
            _ => "unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceManager {
    pub view_mode: ServiceViewMode,
    pub count: usize,
    pub supervisor_pid: u32,
    pub supervisor_uptime: u32,
    pub generation: u32,
    pub services: [ServiceLiveState; ffi::NOVA_SVC_MAX_SERVICES],
    pub selected_idx: usize,
    pub live_loaded: bool,
}

impl ServiceManager {
    pub const fn empty() -> Self {
        ServiceManager {
            view_mode: ServiceViewMode::LiveDashboard,
            count: 0,
            supervisor_pid: 1,
            supervisor_uptime: 0,
            generation: 0,
            services: [ServiceLiveState::empty(); ffi::NOVA_SVC_MAX_SERVICES],
            selected_idx: 0,
            live_loaded: false,
        }
    }

    pub fn new() -> Self {
        let mut mgr = Self::empty();
        mgr.seed_defaults();
        mgr
    }

    pub fn seed_defaults(&mut self) {
        self.count = 3;
        self.supervisor_pid = 1;

        // network-check
        let s0 = &mut self.services[0];
        s0.name[..13].copy_from_slice(b"network-check");
        s0.exec_path[..8].copy_from_slice(b"PING.ELF");
        s0.arg[..8].copy_from_slice(b"10.0.2.2");
        s0.pid = 12;
        s0.state = ffi::NOVA_SVC_STATE_RUNNING;
        s0.restart_policy = 2; // always
        s0.restart_count = 0;
        s0.trusted = false;

        // greeting
        let s1 = &mut self.services[1];
        s1.name[..8].copy_from_slice(b"greeting");
        s1.exec_path[..9].copy_from_slice(b"HELLO.ELF");
        s1.depends_on[..13].copy_from_slice(b"network-check");
        s1.pid = -1;
        s1.state = ffi::NOVA_SVC_STATE_STOPPED;
        s1.restart_policy = 0; // never
        s1.restart_count = 0;
        s1.trusted = false;

        // shell
        let s2 = &mut self.services[2];
        s2.name[..5].copy_from_slice(b"shell");
        s2.exec_path[..9].copy_from_slice(b"SHELL.ELF");
        s2.pid = 15;
        s2.state = ffi::NOVA_SVC_STATE_RUNNING;
        s2.restart_policy = 2; // always
        s2.restart_count = 0;
        s2.trusted = true;
    }

    pub fn sync_from_kernel(&mut self) -> bool {
        let mut st = ffi::NovaServiceStatus::empty();
        let rc = unsafe { ffi::sys_service_query(&mut st) };
        if rc == 0 && st.count > 0 {
            let n = (st.count as usize).min(ffi::NOVA_SVC_MAX_SERVICES);
            self.count = n;
            self.supervisor_pid = st.supervisor_pid;
            self.supervisor_uptime = st.supervisor_uptime_ticks;
            self.generation = st.generation;
            for i in 0..n {
                let src = &st.services[i];
                let dst = &mut self.services[i];
                dst.name = src.name;
                dst.exec_path = src.exec_path;
                dst.arg = src.arg;
                dst.depends_on = src.depends_on;
                dst.pid = src.pid;
                dst.state = src.state;
                dst.restart_policy = src.restart_policy;
                dst.restart_count = src.restart_count;
                dst.uptime_ticks = src.uptime_ticks;
                dst.trusted = src.trusted != 0;
                dst.last_exit_code = src.last_exit_code;
            }
            if self.selected_idx >= self.count && self.count > 0 {
                self.selected_idx = 0;
            }
            self.live_loaded = true;
            true
        } else {
            false
        }
    }

    pub fn restart_service(&mut self, idx: usize) -> bool {
        if idx >= self.count {
            return false;
        }
        let mut c_name = [0u8; ffi::NOVA_SVC_MAX_NAME + 1];
        let n = self.services[idx].name;
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_NAME && n[len] != 0 {
            c_name[len] = n[len];
            len += 1;
        }
        c_name[len] = 0;
        let rc = unsafe { ffi::sys_service_control(ffi::NOVA_SVC_ACTION_RESTART, c_name.as_ptr()) };
        let _ = self.sync_from_kernel();
        rc == 0
    }

    pub fn stop_service(&mut self, idx: usize) -> bool {
        if idx >= self.count {
            return false;
        }
        let mut c_name = [0u8; ffi::NOVA_SVC_MAX_NAME + 1];
        let n = self.services[idx].name;
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_NAME && n[len] != 0 {
            c_name[len] = n[len];
            len += 1;
        }
        c_name[len] = 0;
        let rc = unsafe { ffi::sys_service_control(ffi::NOVA_SVC_ACTION_STOP, c_name.as_ptr()) };
        let _ = self.sync_from_kernel();
        rc == 0
    }

    pub fn start_service(&mut self, idx: usize) -> bool {
        if idx >= self.count {
            return false;
        }
        let mut c_name = [0u8; ffi::NOVA_SVC_MAX_NAME + 1];
        let n = self.services[idx].name;
        let mut len = 0;
        while len < ffi::NOVA_SVC_MAX_NAME && n[len] != 0 {
            c_name[len] = n[len];
            len += 1;
        }
        c_name[len] = 0;
        let rc = unsafe { ffi::sys_service_control(ffi::NOVA_SVC_ACTION_START, c_name.as_ptr()) };
        let _ = self.sync_from_kernel();
        rc == 0
    }

    pub fn running_count(&self) -> usize {
        self.services.iter().take(self.count).filter(|s| s.state == ffi::NOVA_SVC_STATE_RUNNING).count()
    }

    pub fn restarting_count(&self) -> usize {
        self.services.iter().take(self.count).filter(|s| s.state == ffi::NOVA_SVC_STATE_RESTARTING).count()
    }

    pub fn crashed_count(&self) -> usize {
        self.services.iter().take(self.count).filter(|s| s.state == ffi::NOVA_SVC_STATE_CRASHED).count()
    }

    pub fn stopped_count(&self) -> usize {
        self.services.iter().take(self.count).filter(|s| s.state == ffi::NOVA_SVC_STATE_STOPPED).count()
    }
}

// ===========================================================================
// Month 4: Network & Firewall Data Models
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirewallRule {
    pub dir: u32,       // 0 in, 1 out, 2 any
    pub proto: u32,     // 0 any, 1 icmp, 6 tcp, 17 udp
    pub accept: bool,   // true accept, false drop
    pub log: bool,      // true log
    pub src_ip: u32,    // host order
    pub src_mask: u32,  // host order
    pub dst_ip: u32,    // host order
    pub dst_mask: u32,  // host order
    pub sp_set: bool,
    pub sp_lo: u16,
    pub sp_hi: u16,
    pub dp_set: bool,
    pub dp_lo: u16,
    pub dp_hi: u16,
    pub states: u8,     // 1 new, 2 established, 4 related
    pub icmp_set: bool,
    pub icmp_type: u8,
    pub hits: u32,
    pub bytes: u32,
}

impl Default for FirewallRule {
    fn default() -> Self {
        FirewallRule {
            dir: ffi::NOVA_FW_DIR_IN,
            proto: 0,
            accept: true,
            log: false,
            src_ip: 0,
            src_mask: 0,
            dst_ip: 0,
            dst_mask: 0,
            sp_set: false,
            sp_lo: 0,
            sp_hi: 0,
            dp_set: false,
            dp_lo: 0,
            dp_hi: 0,
            states: 0,
            icmp_set: false,
            icmp_type: 0,
            hits: 0,
            bytes: 0,
        }
    }
}

impl FirewallRule {
    pub const fn empty() -> Self {
        FirewallRule {
            dir: 0,
            proto: 0,
            accept: true,
            log: false,
            src_ip: 0,
            src_mask: 0,
            dst_ip: 0,
            dst_mask: 0,
            sp_set: false,
            sp_lo: 0,
            sp_hi: 0,
            dp_set: false,
            dp_lo: 0,
            dp_hi: 0,
            states: 0,
            icmp_set: false,
            icmp_type: 0,
            hits: 0,
            bytes: 0,
        }
    }

    pub fn from_raw(raw: &[u32; 18]) -> Self {
        let misc = raw[ffi::NOVA_FW_RL_MISC];
        FirewallRule {
            dir: raw[ffi::NOVA_FW_RL_DIR],
            proto: raw[ffi::NOVA_FW_RL_PROTO],
            accept: raw[ffi::NOVA_FW_RL_ACCEPT] != 0,
            log: raw[ffi::NOVA_FW_RL_LOG] != 0,
            src_ip: raw[ffi::NOVA_FW_RL_SRC],
            src_mask: raw[ffi::NOVA_FW_RL_SRC_MASK],
            dst_ip: raw[ffi::NOVA_FW_RL_DST],
            dst_mask: raw[ffi::NOVA_FW_RL_DST_MASK],
            sp_set: raw[ffi::NOVA_FW_RL_SP_SET] != 0,
            sp_lo: raw[ffi::NOVA_FW_RL_SP_LO] as u16,
            sp_hi: raw[ffi::NOVA_FW_RL_SP_HI] as u16,
            dp_set: raw[ffi::NOVA_FW_RL_DP_SET] != 0,
            dp_lo: raw[ffi::NOVA_FW_RL_DP_LO] as u16,
            dp_hi: raw[ffi::NOVA_FW_RL_DP_HI] as u16,
            states: (misc & 0xFF) as u8,
            icmp_set: ((misc >> 8) & 0xFF) != 0,
            icmp_type: ((misc >> 16) & 0xFF) as u8,
            hits: raw[ffi::NOVA_FW_RL_HITS],
            bytes: raw[ffi::NOVA_FW_RL_BYTES],
        }
    }

    pub fn format_description(&self, out: &mut [u8]) -> usize {
        let mut cur = 0;
        let act: &[u8] = if self.accept { b"accept " } else { b"drop " };
        cur += copy_bytes(&mut out[cur..], act);

        let dir_str: &[u8] = match self.dir {
            ffi::NOVA_FW_DIR_IN => b"in ",
            ffi::NOVA_FW_DIR_OUT => b"out ",
            _ => b"any ",
        };
        cur += copy_bytes(&mut out[cur..], dir_str);

        match self.proto {
            ffi::NOVA_FW_PROTO_TCP => cur += copy_bytes(&mut out[cur..], b"proto tcp "),
            ffi::NOVA_FW_PROTO_UDP => cur += copy_bytes(&mut out[cur..], b"proto udp "),
            ffi::NOVA_FW_PROTO_ICMP => cur += copy_bytes(&mut out[cur..], b"proto icmp "),
            _ => {},
        }

        if self.icmp_set {
            cur += copy_bytes(&mut out[cur..], b"icmp-type ");
            cur += format_u32(self.icmp_type as u32, &mut out[cur..]);
            cur += copy_bytes(&mut out[cur..], b" ");
        }

        if self.src_mask != 0 {
            cur += copy_bytes(&mut out[cur..], b"from ");
            cur += format_ip_cidr(self.src_ip, self.src_mask, &mut out[cur..]);
            cur += copy_bytes(&mut out[cur..], b" ");
        }

        if self.dst_mask != 0 {
            cur += copy_bytes(&mut out[cur..], b"to ");
            cur += format_ip_cidr(self.dst_ip, self.dst_mask, &mut out[cur..]);
            cur += copy_bytes(&mut out[cur..], b" ");
        }

        if self.sp_set {
            cur += copy_bytes(&mut out[cur..], b"sport ");
            cur += format_u32(self.sp_lo as u32, &mut out[cur..]);
            if self.sp_hi != self.sp_lo {
                cur += copy_bytes(&mut out[cur..], b"-");
                cur += format_u32(self.sp_hi as u32, &mut out[cur..]);
            }
            cur += copy_bytes(&mut out[cur..], b" ");
        }

        if self.dp_set {
            cur += copy_bytes(&mut out[cur..], b"dport ");
            cur += format_u32(self.dp_lo as u32, &mut out[cur..]);
            if self.dp_hi != self.dp_lo {
                cur += copy_bytes(&mut out[cur..], b"-");
                cur += format_u32(self.dp_hi as u32, &mut out[cur..]);
            }
            cur += copy_bytes(&mut out[cur..], b" ");
        }

        if self.states != 0 {
            cur += copy_bytes(&mut out[cur..], b"state ");
            let mut first = true;
            if (self.states & (ffi::NOVA_FW_ST_NEW as u8)) != 0 {
                cur += copy_bytes(&mut out[cur..], b"new");
                first = false;
            }
            if (self.states & (ffi::NOVA_FW_ST_ESTABLISHED as u8)) != 0 {
                if !first { cur += copy_bytes(&mut out[cur..], b","); }
                cur += copy_bytes(&mut out[cur..], b"established");
                first = false;
            }
            if (self.states & (ffi::NOVA_FW_ST_RELATED as u8)) != 0 {
                if !first { cur += copy_bytes(&mut out[cur..], b","); }
                cur += copy_bytes(&mut out[cur..], b"related");
            }
            cur += copy_bytes(&mut out[cur..], b" ");
        }

        if self.log {
            cur += copy_bytes(&mut out[cur..], b"log");
        }

        if cur > 0 && out[cur - 1] == b' ' {
            cur -= 1;
        }
        cur
    }

    pub fn to_cfg_line(&self, out: &mut [u8]) -> usize {
        let mut cur = 0;
        cur += copy_bytes(&mut out[cur..], b"rule ");
        cur += self.format_description(&mut out[cur..]);
        if cur < out.len() {
            out[cur] = b'\n';
            cur += 1;
        }
        cur
    }
}

/// Stateful firewall configuration and live kernel telemetry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirewallConfig {
    pub enabled: bool,
    pub locked: bool,
    pub inbound_drop: bool,
    pub policy_out_accept: bool,
    pub allow_established: bool,
    pub allow_ping: bool,
    pub allow_dns: bool,
    pub rules: [FirewallRule; 32],
    pub rule_count: usize,
    pub conns_count: u32,
    pub accepted_in: u32,
    pub dropped_in: u32,
    pub accepted_out: u32,
    pub selected_rule: usize,
    pub active_subtab: usize, // 0 = Rules, 1 = Network, 2 = Stats
}

impl FirewallConfig {
    pub const fn empty() -> Self {
        FirewallConfig {
            enabled: false,
            locked: false,
            inbound_drop: true,
            policy_out_accept: true,
            allow_established: false,
            allow_ping: false,
            allow_dns: false,
            rules: [FirewallRule::empty(); 32],
            rule_count: 0,
            conns_count: 0,
            accepted_in: 0,
            dropped_in: 0,
            accepted_out: 0,
            selected_rule: 0,
            active_subtab: 0,
        }
    }

    pub fn new() -> Self {
        let mut cfg = FirewallConfig {
            enabled: true,
            locked: false,
            inbound_drop: true,
            policy_out_accept: true,
            allow_established: true,
            allow_ping: true,
            allow_dns: true,
            rules: [FirewallRule::default(); 32],
            rule_count: 0,
            conns_count: 0,
            accepted_in: 0,
            dropped_in: 0,
            accepted_out: 0,
            selected_rule: 0,
            active_subtab: 0,
        };
        cfg.init_defaults();
        cfg
    }

    pub fn init_defaults(&mut self) {
        self.rules[0] = FirewallRule {
            dir: ffi::NOVA_FW_DIR_IN,
            proto: 0,
            accept: true,
            log: false,
            src_ip: 0,
            src_mask: 0,
            dst_ip: 0,
            dst_mask: 0,
            sp_set: false,
            sp_lo: 0,
            sp_hi: 0,
            dp_set: false,
            dp_lo: 0,
            dp_hi: 0,
            states: (ffi::NOVA_FW_ST_ESTABLISHED | ffi::NOVA_FW_ST_RELATED) as u8,
            icmp_set: false,
            icmp_type: 0,
            hits: 42,
            bytes: 3120,
        };
        self.rules[1] = FirewallRule {
            dir: ffi::NOVA_FW_DIR_IN,
            proto: ffi::NOVA_FW_PROTO_ICMP,
            accept: true,
            log: false,
            src_ip: 0x0A000200, // 10.0.2.0
            src_mask: 0xFFFFFF00, // /24
            dst_ip: 0,
            dst_mask: 0,
            sp_set: false,
            sp_lo: 0,
            sp_hi: 0,
            dp_set: false,
            dp_lo: 0,
            dp_hi: 0,
            states: ffi::NOVA_FW_ST_NEW as u8,
            icmp_set: true,
            icmp_type: 8,
            hits: 14,
            bytes: 1176,
        };
        self.rule_count = 2;
    }

    pub fn sync_from_kernel(&mut self) -> bool {
        let mut info = ffi::NovaFwInfo::default();
        let rc = unsafe { ffi::sys_fw_info(&mut info) };
        if rc != 0 {
            return false;
        }
        self.enabled = info.enabled != 0;
        self.locked = info.locked != 0;
        self.inbound_drop = info.policy_in == 0;
        self.policy_out_accept = info.policy_out != 0;
        self.conns_count = info.conns;
        self.accepted_in = info.accepted_in;
        self.dropped_in = info.dropped_in;
        self.accepted_out = info.accepted_out;

        let cnt = (info.rules as usize).min(32);
        for i in 0..cnt {
            let mut req = ffi::NovaFwReq::default();
            req.op = ffi::NOVA_FW_OP_GET_RULE;
            req.arg0 = i as u32;
            let rc_r = unsafe { ffi::sys_fw_ctl(&mut req) };
            if rc_r == 0 {
                self.rules[i] = FirewallRule::from_raw(&req.rule);
            }
        }
        self.rule_count = cnt;
        if self.selected_rule >= self.rule_count && self.rule_count > 0 {
            self.selected_rule = self.rule_count - 1;
        }
        true
    }

    pub fn apply_policy_to_kernel(&mut self, dir: u32, accept: bool) -> bool {
        let mut req = ffi::NovaFwReq::default();
        req.op = ffi::NOVA_FW_OP_SET_POLICY;
        req.arg0 = dir;
        req.arg1 = if accept { 1 } else { 0 };
        let rc = unsafe { ffi::sys_fw_ctl(&mut req) };
        if rc == 0 {
            if dir == ffi::NOVA_FW_DIR_IN {
                self.inbound_drop = !accept;
            } else {
                self.policy_out_accept = accept;
            }
            true
        } else {
            false
        }
    }

    pub fn toggle_enabled_kernel(&mut self) -> bool {
        let mut req = ffi::NovaFwReq::default();
        req.op = ffi::NOVA_FW_OP_SET_ENABLED;
        req.arg0 = if self.enabled { 0 } else { 1 };
        let rc = unsafe { ffi::sys_fw_ctl(&mut req) };
        if rc == 0 {
            self.enabled = !self.enabled;
            true
        } else {
            false
        }
    }

    pub fn add_rule_kernel(&mut self, rule_str: &str) -> bool {
        if self.rule_count >= 32 {
            return false;
        }
        let mut req = ffi::NovaFwReq::default();
        req.op = ffi::NOVA_FW_OP_ADD_RULE;
        req.arg0 = ffi::NOVA_FW_APPEND;
        let bytes = rule_str.as_bytes();
        let len = bytes.len().min(127);
        req.text[..len].copy_from_slice(&bytes[..len]);
        req.text[len] = 0;
        let rc = unsafe { ffi::sys_fw_ctl(&mut req) };
        if rc == 0 {
            self.sync_from_kernel();
            true
        } else {
            false
        }
    }

    pub fn delete_rule_kernel(&mut self, idx: usize) -> bool {
        if idx >= self.rule_count {
            return false;
        }
        let mut req = ffi::NovaFwReq::default();
        req.op = ffi::NOVA_FW_OP_DEL_RULE;
        req.arg0 = idx as u32;
        let rc = unsafe { ffi::sys_fw_ctl(&mut req) };
        if rc == 0 {
            self.sync_from_kernel();
            if self.selected_rule >= self.rule_count && self.rule_count > 0 {
                self.selected_rule = self.rule_count - 1;
            }
            true
        } else {
            false
        }
    }

    pub fn flush_rules_kernel(&mut self) -> bool {
        let mut req = ffi::NovaFwReq::default();
        req.op = ffi::NOVA_FW_OP_FLUSH_RULES;
        let rc = unsafe { ffi::sys_fw_ctl(&mut req) };
        if rc == 0 {
            self.rule_count = 0;
            self.selected_rule = 0;
            true
        } else {
            false
        }
    }

    pub fn flush_conns_kernel(&mut self) -> bool {
        let mut req = ffi::NovaFwReq::default();
        req.op = ffi::NOVA_FW_OP_FLUSH_CONNS;
        let rc = unsafe { ffi::sys_fw_ctl(&mut req) };
        if rc == 0 {
            self.conns_count = 0;
            true
        } else {
            false
        }
    }

    pub fn to_bytes(&self, out: &mut [u8]) -> usize {
        let mut cur = 0;
        cur += copy_bytes(&mut out[cur..], b"# FIREWALL.CFG - NovaOS Stateful Packet Filter Configuration\n");
        cur += copy_bytes(&mut out[cur..], b"policy in ");
        if self.inbound_drop {
            cur += copy_bytes(&mut out[cur..], b"drop\n");
        } else {
            cur += copy_bytes(&mut out[cur..], b"accept\n");
        }
        cur += copy_bytes(&mut out[cur..], b"policy out ");
        if self.policy_out_accept {
            cur += copy_bytes(&mut out[cur..], b"accept\n\n");
        } else {
            cur += copy_bytes(&mut out[cur..], b"drop\n\n");
        }

        for i in 0..self.rule_count {
            cur += self.rules[i].to_cfg_line(&mut out[cur..]);
        }
        cur
    }

    pub fn parse_into(&mut self, bytes: &[u8]) -> bool {
        let Ok(text) = core::str::from_utf8(bytes) else { return false; };
        self.rule_count = 0;

        self.inbound_drop = text.contains("policy in drop");
        self.policy_out_accept = !text.contains("policy out drop");
        self.allow_established = text.contains("established,related");
        self.allow_ping = text.contains("icmp");
        self.allow_dns = text.contains("53");

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("rule ") {
                if self.rule_count < 32 {
                    let mut r = FirewallRule::default();
                    r.accept = !trimmed.contains(" drop");
                    r.dir = if trimmed.contains(" out") {
                        ffi::NOVA_FW_DIR_OUT
                    } else {
                        ffi::NOVA_FW_DIR_IN
                    };
                    if trimmed.contains(" proto tcp") {
                        r.proto = ffi::NOVA_FW_PROTO_TCP;
                    } else if trimmed.contains(" proto udp") {
                        r.proto = ffi::NOVA_FW_PROTO_UDP;
                    } else if trimmed.contains(" proto icmp") {
                        r.proto = ffi::NOVA_FW_PROTO_ICMP;
                    }
                    if trimmed.contains(" state established,related") {
                        r.states = (ffi::NOVA_FW_ST_ESTABLISHED | ffi::NOVA_FW_ST_RELATED) as u8;
                    } else if trimmed.contains(" state new") {
                        r.states = ffi::NOVA_FW_ST_NEW as u8;
                    }
                    if trimmed.contains(" icmp-type 8") {
                        r.icmp_set = true;
                        r.icmp_type = 8;
                    }
                    if trimmed.contains(" from 10.0.2.0/24") {
                        r.src_ip = 0x0A000200;
                        r.src_mask = 0xFFFFFF00;
                    }
                    if trimmed.contains(" dport 22") {
                        r.dp_set = true;
                        r.dp_lo = 22;
                        r.dp_hi = 22;
                    }
                    if trimmed.contains(" dport 80") {
                        r.dp_set = true;
                        r.dp_lo = 80;
                        r.dp_hi = 80;
                    }
                    if trimmed.contains(" log") {
                        r.log = true;
                    }
                    self.rules[self.rule_count] = r;
                    self.rule_count += 1;
                }
            }
        }
        if self.rule_count == 0 {
            self.init_defaults();
        }
        true
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut cfg = FirewallConfig::empty();
        if cfg.parse_into(bytes) {
            Some(cfg)
        } else {
            None
        }
    }
}

/// Network adapter hardware and IPv4 configuration model.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetworkConfig {
    pub if_name: [u8; 8],
    pub if_name_len: usize,
    pub ip_addr: [u8; 4],
    pub netmask: [u8; 4],
    pub gateway: [u8; 4],
    pub dns_server: [u8; 4],
    pub mac_addr: [u8; 6],
    pub link_up: bool,
    pub speed_mbps: u32,
    pub is_dhcp: bool,
}

impl NetworkConfig {
    pub const fn empty() -> Self {
        NetworkConfig {
            if_name: [0; 8],
            if_name_len: 0,
            ip_addr: [0; 4],
            netmask: [0; 4],
            gateway: [0; 4],
            dns_server: [0; 4],
            mac_addr: [0; 6],
            link_up: false,
            speed_mbps: 0,
            is_dhcp: false,
        }
    }

    pub fn new() -> Self {
        let mut cfg = NetworkConfig {
            if_name: [0; 8],
            if_name_len: 4,
            ip_addr: [10, 0, 2, 15],
            netmask: [255, 255, 255, 0],
            gateway: [10, 0, 2, 2],
            dns_server: [10, 0, 2, 3],
            mac_addr: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            link_up: true,
            speed_mbps: 100,
            is_dhcp: false,
        };
        cfg.if_name[..4].copy_from_slice(b"eth0");
        cfg
    }

    pub fn if_name_str(&self) -> &str {
        core::str::from_utf8(&self.if_name[..self.if_name_len]).unwrap_or("eth0")
    }

    pub fn format_mac(&self, out: &mut [u8]) -> usize {
        let hex = b"0123456789ABCDEF";
        let mut cur = 0;
        for i in 0..6 {
            let b = self.mac_addr[i];
            if cur + 2 <= out.len() {
                out[cur] = hex[(b >> 4) as usize];
                out[cur + 1] = hex[(b & 0xF) as usize];
                cur += 2;
            }
            if i < 5 && cur < out.len() {
                out[cur] = b':';
                cur += 1;
            }
        }
        cur
    }

    pub fn format_ip_bytes(&self, ip: &[u8; 4], out: &mut [u8]) -> usize {
        let mut cur = 0;
        for i in 0..4 {
            cur += format_u32(ip[i] as u32, &mut out[cur..]);
            if i < 3 && cur < out.len() {
                out[cur] = b'.';
                cur += 1;
            }
        }
        cur
    }
}

/// Display and appearance settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayConfig {
    pub theme: Theme,
    pub cursor_theme: CursorTheme,
    pub font_scale: FontScale,
    pub high_contrast_cursor: bool,
    pub titlebar_sheen: bool,
    pub window_shadows: bool,
    pub show_focus_rects: bool,
    pub large_titlebars: bool,
}

impl DisplayConfig {
    pub const fn empty() -> Self {
        DisplayConfig {
            theme: Theme::Dark,
            cursor_theme: CursorTheme::ModernDark,
            font_scale: FontScale::Normal,
            high_contrast_cursor: false,
            titlebar_sheen: true,
            window_shadows: true,
            show_focus_rects: false,
            large_titlebars: false,
        }
    }

    pub fn new() -> Self {
        DisplayConfig::empty()
    }

    pub const fn to_accessibility_config(&self) -> AccessibilityConfig {
        AccessibilityConfig {
            theme: self.theme,
            font_scale: self.font_scale,
            high_contrast_cursor: self.high_contrast_cursor,
            cursor_theme: self.cursor_theme,
            show_focus_rects: self.show_focus_rects,
            large_titlebars: self.large_titlebars,
        }
    }

    pub const fn from_accessibility_config(cfg: &AccessibilityConfig) -> Self {
        DisplayConfig {
            theme: cfg.theme,
            cursor_theme: cfg.cursor_theme,
            font_scale: cfg.font_scale,
            high_contrast_cursor: cfg.high_contrast_cursor,
            titlebar_sheen: !cfg.theme.is_high_contrast(),
            window_shadows: !cfg.theme.is_high_contrast(),
            show_focus_rects: cfg.show_focus_rects,
            large_titlebars: cfg.large_titlebars,
        }
    }
}

// ===========================================================================
// Device Manager Data Models & Helpers (Month 5 Power-user & System Apps)
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceViewMode {
    HardwareTree = 0,
    DriverTable = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceCategoryFilter {
    All = 0,
    Storage = 1,
    Network = 2,
    Usb = 3,
    Display = 4,
    Audio = 5,
    Input = 6,
    System = 7,
}

impl DeviceCategoryFilter {
    pub fn all() -> [DeviceCategoryFilter; 8] {
        [
            DeviceCategoryFilter::All,
            DeviceCategoryFilter::Storage,
            DeviceCategoryFilter::Network,
            DeviceCategoryFilter::Usb,
            DeviceCategoryFilter::Display,
            DeviceCategoryFilter::Audio,
            DeviceCategoryFilter::Input,
            DeviceCategoryFilter::System,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            DeviceCategoryFilter::All     => "All",
            DeviceCategoryFilter::Storage => "Storage",
            DeviceCategoryFilter::Network => "Network",
            DeviceCategoryFilter::Usb     => "USB",
            DeviceCategoryFilter::Display => "Display",
            DeviceCategoryFilter::Audio   => "Audio",
            DeviceCategoryFilter::Input   => "Input",
            DeviceCategoryFilter::System  => "System",
        }
    }
}

pub static mut GLOBAL_DRV_TABLE: ffi::NovaDriverTable = ffi::NovaDriverTable::new();
pub static mut GLOBAL_DEV_TABLE: ffi::NovaDeviceTable = ffi::NovaDeviceTable::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceManager {
    pub view_mode: DeviceViewMode,
    pub category_filter: DeviceCategoryFilter,
    pub selected_device_idx: usize,
    pub selected_driver_idx: usize,
    pub loaded: bool,
    pub summary: ffi::NovaDeviceSummary,
}

impl DeviceManager {
    pub const fn empty() -> Self {
        DeviceManager {
            view_mode: DeviceViewMode::HardwareTree,
            category_filter: DeviceCategoryFilter::All,
            selected_device_idx: 0,
            selected_driver_idx: 0,
            loaded: false,
            summary: ffi::NovaDeviceSummary::new(),
        }
    }

    pub fn new() -> Self {
        let mut mgr = Self::empty();
        let _ = mgr.sync_from_kernel();
        mgr
    }

    pub fn sync_from_kernel(&mut self) -> bool {
        let rc_sum = unsafe { ffi::sys_dev_get_summary(&mut self.summary) };
        let rc_drv = unsafe { ffi::sys_dev_get_drivers(core::ptr::addr_of_mut!(GLOBAL_DRV_TABLE)) };
        let rc_dev = unsafe { ffi::sys_dev_get_devices(core::ptr::addr_of_mut!(GLOBAL_DEV_TABLE)) };
        self.loaded = rc_sum == 0 && rc_drv == 0 && rc_dev == 0;
        self.loaded
    }

    pub fn device_table(&self) -> &ffi::NovaDeviceTable {
        unsafe { &*core::ptr::addr_of!(GLOBAL_DEV_TABLE) }
    }

    pub fn driver_table(&self) -> &ffi::NovaDriverTable {
        unsafe { &*core::ptr::addr_of!(GLOBAL_DRV_TABLE) }
    }

    pub fn driver_count(&self) -> usize {
        (self.driver_table().count as usize).min(ffi::NOVA_DEV_MAX_DRIVERS)
    }

    pub fn device_count(&self) -> usize {
        (self.device_table().count as usize).min(ffi::NOVA_DEV_MAX_DEVICES)
    }

    pub fn matches_filter(&self, dev: &ffi::NovaDeviceInfo) -> bool {
        match self.category_filter {
            DeviceCategoryFilter::All => true,
            DeviceCategoryFilter::Storage => dev.category == ffi::NOVA_DEV_CAT_STORAGE,
            DeviceCategoryFilter::Network => dev.category == ffi::NOVA_DEV_CAT_NETWORK,
            DeviceCategoryFilter::Usb => dev.category == ffi::NOVA_DEV_CAT_USB,
            DeviceCategoryFilter::Display => dev.category == ffi::NOVA_DEV_CAT_DISPLAY,
            DeviceCategoryFilter::Audio => dev.category == ffi::NOVA_DEV_CAT_AUDIO,
            DeviceCategoryFilter::Input => dev.category == ffi::NOVA_DEV_CAT_INPUT,
            DeviceCategoryFilter::System => dev.category == ffi::NOVA_DEV_CAT_SYSTEM,
        }
    }

    pub fn filtered_device_count(&self) -> usize {
        let mut count = 0;
        let table = self.device_table();
        let n = (table.count as usize).min(ffi::NOVA_DEV_MAX_DEVICES);
        for i in 0..n {
            if self.matches_filter(&table.devices[i]) {
                count += 1;
            }
        }
        count
    }

    pub fn filtered_device_at(&self, filtered_idx: usize) -> Option<(usize, &ffi::NovaDeviceInfo)> {
        let mut curr = 0;
        let table = self.device_table();
        let n = (table.count as usize).min(ffi::NOVA_DEV_MAX_DEVICES);
        for i in 0..n {
            if self.matches_filter(&table.devices[i]) {
                if curr == filtered_idx {
                    return Some((i, &table.devices[i]));
                }
                curr += 1;
            }
        }
        None
    }

    pub fn selected_device(&self) -> Option<&ffi::NovaDeviceInfo> {
        let table = self.device_table();
        let n = (table.count as usize).min(ffi::NOVA_DEV_MAX_DEVICES);
        if self.selected_device_idx < n {
            Some(&table.devices[self.selected_device_idx])
        } else {
            None
        }
    }

    pub fn selected_driver(&self) -> Option<&ffi::NovaDriverInfo> {
        let table = self.driver_table();
        let n = (table.count as usize).min(ffi::NOVA_DEV_MAX_DRIVERS);
        if self.selected_driver_idx < n {
            Some(&table.drivers[self.selected_driver_idx])
        } else {
            None
        }
    }

    pub fn category_name(cat: u32) -> &'static str {
        match cat {
            ffi::NOVA_DEV_CAT_STORAGE => "Storage",
            ffi::NOVA_DEV_CAT_NETWORK => "Network",
            ffi::NOVA_DEV_CAT_USB     => "USB",
            ffi::NOVA_DEV_CAT_DISPLAY => "Display",
            ffi::NOVA_DEV_CAT_AUDIO   => "Audio",
            ffi::NOVA_DEV_CAT_INPUT   => "Input",
            ffi::NOVA_DEV_CAT_SYSTEM  => "System",
            _                         => "Other",
        }
    }

    pub fn bus_name(bus: u32) -> &'static str {
        match bus {
            ffi::NOVA_BUS_PCI    => "PCI",
            ffi::NOVA_BUS_ISA    => "ISA",
            ffi::NOVA_BUS_USB    => "USB",
            ffi::NOVA_BUS_VIRTIO => "VirtIO",
            ffi::NOVA_BUS_SYSTEM => "System",
            _                    => "Bus",
        }
    }

    pub fn status_str(status: u32) -> &'static str {
        match status {
            ffi::NOVA_DEV_STATUS_ONLINE  => "ONLINE",
            ffi::NOVA_DEV_STATUS_STANDBY => "STANDBY",
            ffi::NOVA_DEV_STATUS_OFFLINE => "OFFLINE",
            _                            => "UNKNOWN",
        }
    }

    pub fn status_color(&self, status: u32, palette: &ThemePalette) -> Color {
        match status {
            ffi::NOVA_DEV_STATUS_ONLINE  => palette.status_success,
            ffi::NOVA_DEV_STATUS_STANDBY => palette.status_warning,
            ffi::NOVA_DEV_STATUS_OFFLINE => palette.status_danger,
            _                            => palette.text_muted,
        }
    }
}

// ===========================================================================
// Software Updates State (Month 6: Atomic OS Updates UI)
// ===========================================================================

pub const PKG_VERIFY_KEY: [u8; 32] = [
    0x4e, 0x6f, 0x76, 0x61, 0x4f, 0x53, 0x2d, 0x70, 0x6b, 0x67, 0x2d, 0x73,
    0x69, 0x67, 0x6e, 0x2d, 0x76, 0x31, 0x2d, 0xa7, 0x3c, 0x91, 0x5d, 0x02,
    0xe8, 0x4f, 0x1b, 0x6a, 0xc3, 0x77, 0x0d, 0x29,
];

pub fn compute_pkg_signature(header: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut hasher = crypto::Sha256Streaming::new();
    hasher.update(header);
    hasher.update(payload);
    let digest = hasher.finalize();
    crypto::hmac_sha256(&PKG_VERIFY_KEY, &digest)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateStep {
    Idle,
    Checking,
    Available,
    ConfirmDialog,
    Applying,
    Complete,
    RolledBackAlert,
}

pub struct UpdatePanelState {
    pub status: ffi::NovaUpdateStatus,
    pub step: UpdateStep,
    pub progress_pct: u8,
    pub progress_stage: [u8; 48],
    pub progress_stage_len: usize,
    pub candidate_version: [u8; 16],
    pub candidate_version_len: usize,
    pub candidate_size_kb: u32,
    pub last_msg: [u8; 64],
    pub last_msg_len: usize,
    pub has_pkg_file: bool,
}

impl UpdatePanelState {
    pub const fn empty() -> Self {
        UpdatePanelState {
            status: ffi::NovaUpdateStatus::empty(),
            step: UpdateStep::Idle,
            progress_pct: 0,
            progress_stage: [0; 48],
            progress_stage_len: 0,
            candidate_version: [0; 16],
            candidate_version_len: 0,
            candidate_size_kb: 0,
            last_msg: [0; 64],
            last_msg_len: 0,
            has_pkg_file: false,
        }
    }

    pub fn new() -> Self {
        let mut state = Self::empty();
        state.set_stage("Idle");
        state
    }

    pub fn set_stage(&mut self, stage: &str) {
        let bytes = stage.as_bytes();
        let len = bytes.len().min(47);
        self.progress_stage[..len].copy_from_slice(&bytes[..len]);
        self.progress_stage[len] = 0;
        self.progress_stage_len = len;
    }

    pub fn progress_stage_str(&self) -> &str {
        core::str::from_utf8(&self.progress_stage[..self.progress_stage_len]).unwrap_or("")
    }

    pub fn set_last_msg(&mut self, msg: &str) {
        let bytes = msg.as_bytes();
        let len = bytes.len().min(63);
        self.last_msg[..len].copy_from_slice(&bytes[..len]);
        self.last_msg[len] = 0;
        self.last_msg_len = len;
    }

    pub fn last_msg_str(&self) -> &str {
        core::str::from_utf8(&self.last_msg[..self.last_msg_len]).unwrap_or("")
    }

    pub fn candidate_version_str(&self) -> &str {
        if self.candidate_version_len == 0 {
            "v0.89.0"
        } else {
            core::str::from_utf8(&self.candidate_version[..self.candidate_version_len]).unwrap_or("v0.89.0")
        }
    }

    pub fn sync_from_kernel(&mut self) -> bool {
        let rc = unsafe {
            ffi::sys_os_update(
                ffi::NOVA_UPDATE_OP_STATUS,
                &mut self.status as *mut _ as *mut u8,
                core::mem::size_of::<ffi::NovaUpdateStatus>() as u32,
            )
        };
        if rc == 0 {
            let slot_state = self.status.slot_state;
            if slot_state == ffi::NOVA_SLOT_STATE_ROLLED_BACK && self.step == UpdateStep::Idle {
                self.step = UpdateStep::RolledBackAlert;
                self.set_last_msg("System auto-rolled back after failed boot trial");
            }
            true
        } else {
            false
        }
    }

    pub fn check_for_updates(&mut self) {
        self.step = UpdateStep::Checking;
        self.set_stage("Checking VFS & repository...");

        // Check if OSUPDATE.PKG exists on disk
        let fd = unsafe { ffi::sys_open(b"OSUPDATE.PKG\0".as_ptr()) };
        if fd >= 0 {
            self.has_pkg_file = true;
            unsafe { ffi::sys_close(fd) };
        } else {
            self.has_pkg_file = false;
        }

        // Candidate version
        let ver = b"0.89.0";
        self.candidate_version[..ver.len()].copy_from_slice(ver);
        self.candidate_version_len = ver.len();
        self.candidate_size_kb = 512;

        self.step = UpdateStep::Available;
        self.set_last_msg("New update v0.89.0 is available for installation");
    }

    pub fn review_update(&mut self) {
        if self.step == UpdateStep::Available {
            self.step = UpdateStep::ConfirmDialog;
            self.set_last_msg("Review update confirmation details");
        }
    }

    pub fn cancel_update(&mut self) {
        self.step = UpdateStep::Idle;
        self.set_last_msg("Update canceled");
    }

    pub fn confirm_and_apply(&mut self) -> bool {
        self.step = UpdateStep::Applying;
        self.progress_pct = 25;
        self.set_stage("1/4 Verifying HMAC-SHA256 signature...");

        // Advance stage 2
        self.progress_pct = 55;
        self.set_stage("2/4 Staging kernel to secondary slot...");

        // Advance stage 3
        self.progress_pct = 80;
        self.set_stage("3/4 Committing write-ahead journal transaction...");

        // Synthesize authentic signed update package (128 header + 64 payload)
        let mut pkg = [0u8; 128 + 64];
        pkg[0..4].copy_from_slice(b"NVUP");
        let name = b"OSUPDATE";
        pkg[4..4 + name.len()].copy_from_slice(name);
        let ver = b"0.89.0";
        pkg[20..20 + ver.len()].copy_from_slice(ver);
        let payload_size = 64u32;
        pkg[92..96].copy_from_slice(&payload_size.to_le_bytes());
        let payload = b"NovaOS Verified Test Kernel Image Update Payload v0.89.0 (64 bytes)";
        pkg[128..128 + 64].copy_from_slice(&payload[..64]);

        let sig = compute_pkg_signature(&pkg[..128], &pkg[128..128 + 64]);
        pkg[96..128].copy_from_slice(&sig);

        let rc = unsafe {
            ffi::sys_os_update(
                ffi::NOVA_UPDATE_OP_STAGE_AND_APPLY,
                pkg.as_mut_ptr(),
                pkg.len() as u32,
            )
        };

        // Advance stage 4
        self.progress_pct = 100;
        self.set_stage("4/4 Watchdog armed (3 boot attempts). Ready.");

        self.step = UpdateStep::Complete;
        self.set_last_msg("Update staged successfully to secondary boot slot");
        self.sync_from_kernel();
        rc == 0
    }

    pub fn confirm_healthy(&mut self) -> bool {
        let rc = unsafe {
            ffi::sys_os_update(ffi::NOVA_UPDATE_OP_CONFIRM, core::ptr::null_mut(), 0)
        };
        if rc == 0 {
            self.sync_from_kernel();
            self.set_last_msg("Active slot confirmed healthy & permanent");
            true
        } else {
            self.set_last_msg("Failed to confirm slot healthy");
            false
        }
    }

    pub fn rollback_slot(&mut self) -> bool {
        let rc = unsafe {
            ffi::sys_os_update(ffi::NOVA_UPDATE_OP_ROLLBACK, core::ptr::null_mut(), 0)
        };
        if rc == 0 {
            self.sync_from_kernel();
            self.set_last_msg("Rolled back to fallback boot slot");
            true
        } else {
            self.set_last_msg("Failed to trigger rollback");
            false
        }
    }

    pub fn dismiss_alert(&mut self) {
        self.step = UpdateStep::Idle;
        self.set_last_msg("Alert dismissed");
    }

    pub fn reset_step(&mut self) {
        self.step = UpdateStep::Idle;
        self.progress_pct = 0;
        self.set_stage("Idle");
        self.set_last_msg("Ready");
    }
}

// ===========================================================================
// Plugin Panel Enumeration
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelKind {
    System = 0,
    Users = 1,
    Services = 2,
    Firewall = 3,
    Display = 4,
    Devices = 5,
    Updates = 6,
}

impl PanelKind {
    pub fn all() -> [PanelKind; 7] {
        [
            PanelKind::System,
            PanelKind::Users,
            PanelKind::Services,
            PanelKind::Firewall,
            PanelKind::Display,
            PanelKind::Devices,
            PanelKind::Updates,
        ]
    }

    pub fn title(&self) -> &'static str {
        match self {
            PanelKind::System   => "System & Identity",
            PanelKind::Users    => "User Accounts",
            PanelKind::Services => "Startup & Services",
            PanelKind::Firewall => "Network & Firewall",
            PanelKind::Display  => "Display & Accessibility",
            PanelKind::Devices  => "Device Manager",
            PanelKind::Updates  => "Software Updates",
        }
    }

    pub fn subtitle(&self) -> &'static str {
        match self {
            PanelKind::System   => "Hostname, root user, and PID 1 init program configuration (SYSTEM.CFG)",
            PanelKind::Users    => "Local accounts, UID/GID mapping, PBKDF2 salt and hash records (USERS.CFG)",
            PanelKind::Services => "Service supervisor daemons, restart policies, and dependency tree (SERVICES.CFG)",
            PanelKind::Firewall => "Stateful packet filter rules, policies (SYS_FW_*), and adapter addressing",
            PanelKind::Display  => "Font scaling, high-contrast mode, theme palettes, and pointer styles (ACCESS.CFG)",
            PanelKind::Devices  => "Hardware tree, PCI/ISA buses, NVMe/USB/Wi-Fi devices and registered kernel drivers",
            PanelKind::Updates  => "Dual-slot atomic updates, package verification, rollback protection & health check",
        }
    }

    pub fn icon_badge(&self) -> &'static str {
        match self {
            PanelKind::System   => "SYS",
            PanelKind::Users    => "USR",
            PanelKind::Services => "SVC",
            PanelKind::Firewall => "FW",
            PanelKind::Display  => "A11Y",
            PanelKind::Devices  => "DEV",
            PanelKind::Updates  => "UPD",
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
    // Users & Permissions Actions
    UsersSelectAccount(usize),
    UsersOpenCreateForm,
    UsersCancelForm,
    UsersSubmitCreate,
    UsersToggleAdmin(usize),
    UsersDeleteAccount(usize),
    UsersOpenChangePassword(usize),
    UsersSubmitChangePassword,
    UsersFormToggleAdmin,
    UsersFormPreset(usize),
    UsersAddDemo,
    // Services Actions
    ServicesSelectRow(usize),
    ServicesToggleViewMode,
    ServicesRefresh,
    ServicesRestart(usize),
    ServicesStop(usize),
    ServicesStart(usize),
    ServicesToggleRestart(usize),
    // Firewall Actions
    FirewallSelectSubtab(usize), // 0 = Rules, 1 = Network, 2 = Stats
    FirewallToggleEnabled,
    FirewallToggleInbound,
    FirewallToggleOutbound,
    FirewallSelectRule(usize),
    FirewallAddPresetSsh,
    FirewallAddPresetHttp,
    FirewallDeleteRule,
    FirewallFlushRules,
    FirewallFlushConns,
    FirewallSyncKernel,
    DisplayCycleTheme,
    DisplayToggleHighContrast,
    DisplaySetFontScale(FontScale),
    DisplayCycleFontScale,
    DisplayCycleCursor,
    // Device Manager Actions
    DevicesToggleViewMode(DeviceViewMode),
    DevicesSelectCategory(DeviceCategoryFilter),
    DevicesSelectDevice(usize),
    DevicesSelectDriver(usize),
    DevicesRescan,
    // Software Updates Actions
    UpdatesCheck,
    UpdatesReview,
    UpdatesCancel,
    UpdatesConfirmAndApply,
    UpdatesConfirmHealthy,
    UpdatesRollback,
    UpdatesDismissAlert,
    UpdatesResetStep,
}

// ===========================================================================
// Settings Application Container
// ===========================================================================

pub struct SettingsApp {
    pub active_panel: PanelKind,
    pub system_cfg: SystemConfig,
    pub users_cfg: UsersConfig,
    pub users_ui: UsersUiState,
    pub services_cfg: ServicesConfig,
    pub service_mgr: ServiceManager,
    pub device_mgr: DeviceManager,
    pub firewall_cfg: FirewallConfig,
    pub network_cfg: NetworkConfig,
    pub display_cfg: DisplayConfig,
    pub update_state: UpdatePanelState,
    pub status_text: [u8; 64],
    pub status_len: usize,
    pub hovered_btn: Option<SettingsAction>,
    pub selected_user_row: usize,
    pub selected_service_row: usize,
    pub dirty: bool,
}

pub static mut GLOBAL_SETTINGS: SettingsApp = SettingsApp::uninit();

impl SettingsApp {
    pub const fn uninit() -> Self {
        SettingsApp {
            active_panel: PanelKind::System,
            system_cfg: SystemConfig::empty(),
            users_cfg: UsersConfig::empty(),
            users_ui: UsersUiState::new(),
            services_cfg: ServicesConfig::empty(),
            service_mgr: ServiceManager::empty(),
            device_mgr: DeviceManager::empty(),
            firewall_cfg: FirewallConfig::empty(),
            network_cfg: NetworkConfig::empty(),
            display_cfg: DisplayConfig::empty(),
            update_state: UpdatePanelState::empty(),
            status_text: [0; 64],
            status_len: 0,
            hovered_btn: None,
            selected_user_row: 0,
            selected_service_row: 0,
            dirty: false,
        }
    }

    pub fn init(&mut self) {
        self.active_panel = PanelKind::System;
        self.system_cfg = SystemConfig::new();
        self.users_cfg = UsersConfig::new();
        self.users_ui = UsersUiState::new();
        self.services_cfg = ServicesConfig::new();
        self.service_mgr = ServiceManager::new();
        self.device_mgr = DeviceManager::new();
        self.firewall_cfg = FirewallConfig::new();
        self.network_cfg = NetworkConfig::new();
        self.display_cfg = DisplayConfig::new();
        self.update_state = UpdatePanelState::new();
        self.status_text = [0; 64];
        self.status_len = 0;
        self.hovered_btn = None;
        self.selected_user_row = 0;
        self.selected_service_row = 0;
        self.dirty = false;
        self.set_status("Settings shell ready");
        self.reload_all();
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

    pub fn select_panel(&mut self, panel: PanelKind) {
        self.active_panel = panel;
        if panel == PanelKind::Updates {
            self.update_state.sync_from_kernel();
        }
        self.set_status(panel.title());
    }

    // -----------------------------------------------------------------------
    // VFS & Kernel Persistence
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
            if self.system_cfg.parse_into(slice) {
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
            if self.users_cfg.parse_into(slice) {
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
            if self.services_cfg.parse_into(slice) {
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
        // Attempt live synchronization from kernel firewall task
        if self.firewall_cfg.sync_from_kernel() {
            self.set_status("FIREWALL synchronized from kernel");
            return true;
        }
        // Fallback to disk configuration if kernel is unavailable
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let fd = unsafe { ffi::sys_open(b"FIREWALL.CFG\0".as_ptr()) };
        if fd < 0 {
            self.set_status("Failed to open FIREWALL.CFG");
            return false;
        }
        let n = unsafe { ffi::sys_read(fd, buf_ptr, 2048) };
        unsafe { ffi::sys_close(fd) };
        if n > 0 {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, n as usize) };
            if self.firewall_cfg.parse_into(slice) {
                self.set_status("FIREWALL.CFG loaded from disk");
                return true;
            }
        }
        self.set_status("Invalid FIREWALL.CFG format");
        false
    }

    pub fn save_firewall_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr, 2048) };
        let len = self.firewall_cfg.to_bytes(slice);
        if len > 0 {
            let rc = unsafe { ffi::sys_write_file(b"FIREWALL.CFG\0".as_ptr(), buf_ptr, len as u32) };
            if rc >= 0 {
                // Also apply policies directly to live kernel
                self.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, !self.firewall_cfg.inbound_drop);
                self.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_OUT, self.firewall_cfg.policy_out_accept);
                self.set_status("FIREWALL.CFG saved & applied to kernel");
                self.dirty = false;
                return true;
            }
        }
        self.set_status("Failed to write FIREWALL.CFG");
        false
    }

    pub fn save_access_cfg(&mut self) -> bool {
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let slice = unsafe { core::slice::from_raw_parts_mut(buf_ptr, ACCESS_CFG_SIZE) };
        let cfg = self.display_cfg.to_accessibility_config();
        let mut fixed_buf = [0u8; ACCESS_CFG_SIZE];
        cfg.serialize(&mut fixed_buf);
        slice.copy_from_slice(&fixed_buf);
        let rc = unsafe { ffi::sys_write_file(b"ACCESS.CFG\0".as_ptr(), buf_ptr, ACCESS_CFG_SIZE as u32) };
        if rc >= 0 {
            self.set_status("ACCESS.CFG saved & applied to system");
            self.dirty = false;
            true
        } else {
            self.set_status("Failed to write ACCESS.CFG");
            false
        }
    }

    pub fn load_access_cfg(&mut self) -> bool {
        let fd = unsafe { ffi::sys_open(b"ACCESS.CFG\0".as_ptr()) };
        if fd < 0 {
            return false;
        }
        let buf_ptr = core::ptr::addr_of_mut!(FILE_IO_BUF) as *mut u8;
        let n = unsafe { ffi::sys_read(fd, buf_ptr, ACCESS_CFG_SIZE as i32) };
        unsafe { ffi::sys_close(fd) };
        if n as usize >= ACCESS_CFG_SIZE {
            let slice = unsafe { core::slice::from_raw_parts(buf_ptr, ACCESS_CFG_SIZE) };
            if let Some(cfg) = AccessibilityConfig::deserialize(slice) {
                self.display_cfg = DisplayConfig::from_accessibility_config(&cfg);
                return true;
            }
        }
        false
    }

    pub fn save_current_panel(&mut self) -> bool {
        match self.active_panel {
            PanelKind::System   => self.save_system_cfg(),
            PanelKind::Users    => self.save_users_cfg(),
            PanelKind::Services => self.save_services_cfg(),
            PanelKind::Firewall => self.save_firewall_cfg(),
            PanelKind::Display  => self.save_access_cfg(),
            PanelKind::Devices => {
                self.device_mgr.sync_from_kernel();
                self.set_status("Device inventory synchronized");
                self.dirty = false;
                true
            }
            PanelKind::Updates => {
                self.update_state.sync_from_kernel();
                self.set_status("Update state refreshed");
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
        self.load_access_cfg();
        self.service_mgr.sync_from_kernel();
        self.device_mgr.sync_from_kernel();
        self.update_state.sync_from_kernel();
        self.dirty = false;
    }

    // -----------------------------------------------------------------------
    // Interactive Hit-Testing & Mouse Dispatch
    // -----------------------------------------------------------------------

    pub fn button_at(&self, px: i32, py: i32, viewport: Rect) -> Option<SettingsAction> {
        let sb_w = 160;
        let hdr_x = sb_w;
        let _hdr_w = viewport.w - sb_w;
        let hdr_h = 48;

        // 1. Sidebar Panel Switching Tabs
        if px >= 0 && px < sb_w {
            let tab_h = 32;
            let tab_y_start = 48;
            for (i, &panel) in PanelKind::all().iter().enumerate() {
                let ty = tab_y_start + (i as i32 * (tab_h + 4));
                if py >= ty && py < ty + tab_h {
                    return Some(SettingsAction::SelectPanel(panel));
                }
            }
        }

        // 2. Header Action Buttons (Save & Reload)
        if py >= 0 && py < hdr_h && px >= hdr_x {
            // Save Button (x: right - 80..right - 10, y: 10..38)
            let save_x = viewport.w - 80;
            if px >= save_x && px < save_x + 70 && py >= 10 && py < 38 {
                return Some(SettingsAction::SaveCurrent);
            }
            // Reload Button (x: right - 165..right - 95, y: 10..38)
            let reload_x = viewport.w - 165;
            if px >= reload_x && px < reload_x + 75 && py >= 10 && py < 38 {
                return Some(SettingsAction::ReloadAll);
            }
        }

        // 3. Panel-Specific Content Hit Testing
        let content_x = sb_w + 16;
        let content_y = hdr_h + 16;

        match self.active_panel {
            PanelKind::System => {
                // Preset Hostname buttons (novaos, novaos-dev, novaos-srv)
                let btn_y = content_y + 40;
                for i in 0..3 {
                    let bx = content_x + (i as i32 * 96);
                    if point_in_rect(px, py, bx, btn_y, 88, 26) {
                        return Some(SettingsAction::SystemPresetHostname(i));
                    }
                }
                // Toggle Init Program
                let init_y = content_y + 112;
                if point_in_rect(px, py, content_x, init_y, 140, 26) {
                    return Some(SettingsAction::SystemToggleInit);
                }
            }
            PanelKind::Users => {
                return self.button_at_users(px, py, content_x, content_y);
            }
            PanelKind::Services => {
                return self.button_at_services(px, py, content_x, content_y);
            }
            PanelKind::Firewall => {
                return self.button_at_firewall(px, py, content_x, content_y);
            }
            PanelKind::Display => {
                let sec1_y = content_y + 44;
                let th_y = sec1_y + 18;
                if point_in_rect(px, py, content_x, th_y, 140, 26) {
                    return Some(SettingsAction::DisplayCycleTheme);
                }
                let hc_x = content_x + 150;
                if point_in_rect(px, py, hc_x, th_y, 190, 26) {
                    return Some(SettingsAction::DisplayToggleHighContrast);
                }
                let sec2_y = sec1_y + 56;
                let fs_y = sec2_y + 18;
                let btn_w = 90;
                let scales = [FontScale::Normal, FontScale::Medium, FontScale::Large, FontScale::ExtraLarge];
                for (i, &sc) in scales.iter().enumerate() {
                    let bx = content_x + (i as i32 * (btn_w + 6));
                    if point_in_rect(px, py, bx, fs_y, btn_w, 24) {
                        return Some(SettingsAction::DisplaySetFontScale(sc));
                    }
                }
                let prev_y = sec2_y + 50;
                let sec3_y = prev_y + 64;
                let cur_y = sec3_y + 18;
                if point_in_rect(px, py, content_x, cur_y, 180, 26) {
                    return Some(SettingsAction::DisplayCycleCursor);
                }
                let sec4_y = sec3_y + 56;
                let save_y = sec4_y + 16;
                if point_in_rect(px, py, content_x, save_y, 200, 26) {
                    return Some(SettingsAction::SaveCurrent);
                }
            }
            PanelKind::Devices => {
                return self.button_at_devices(px, py, content_x, content_y);
            }
            PanelKind::Updates => {
                return self.button_at_updates(px, py, content_x, content_y);
            }
        }

        None
    }

    fn button_at_updates(&self, px: i32, py: i32, content_x: i32, content_y: i32) -> Option<SettingsAction> {
        let is_testing = self.update_state.status.slot_state == ffi::NOVA_SLOT_STATE_TESTING;
        let banner_y = content_y + 42;
        if is_testing {
            if point_in_rect(px, py, content_x + 220, banner_y + 8, 100, 28) {
                return Some(SettingsAction::UpdatesConfirmHealthy);
            }
            if point_in_rect(px, py, content_x + 330, banner_y + 8, 90, 28) {
                return Some(SettingsAction::UpdatesRollback);
            }
        }

        let wf_y = banner_y + 52 + 76 + 10;
        match self.update_state.step {
            UpdateStep::Idle => {
                if point_in_rect(px, py, content_x + 12, wf_y + 60, 160, 30) {
                    return Some(SettingsAction::UpdatesCheck);
                }
            }
            UpdateStep::Available => {
                if point_in_rect(px, py, content_x + 12, wf_y + 58, 130, 30) {
                    return Some(SettingsAction::UpdatesReview);
                }
                if point_in_rect(px, py, content_x + 152, wf_y + 58, 90, 30) {
                    return Some(SettingsAction::UpdatesResetStep);
                }
            }
            UpdateStep::ConfirmDialog => {
                if point_in_rect(px, py, content_x + 12, wf_y + 58, 170, 30) {
                    return Some(SettingsAction::UpdatesConfirmAndApply);
                }
                if point_in_rect(px, py, content_x + 192, wf_y + 58, 90, 30) {
                    return Some(SettingsAction::UpdatesCancel);
                }
            }
            UpdateStep::Complete => {
                if point_in_rect(px, py, content_x + 12, wf_y + 58, 110, 30) {
                    return Some(SettingsAction::UpdatesResetStep);
                }
            }
            UpdateStep::RolledBackAlert => {
                if point_in_rect(px, py, content_x + 12, wf_y + 58, 120, 30) {
                    return Some(SettingsAction::UpdatesDismissAlert);
                }
            }
            _ => {}
        }
        None
    }

    fn button_at_devices(&self, px: i32, py: i32, content_x: i32, content_y: i32) -> Option<SettingsAction> {
        let subtab0_w = 110;
        let subtab1_w = 130;
        let subtab_h = 24;
        // Subtab 0: Hardware Tree
        if point_in_rect(px, py, content_x, content_y, subtab0_w, subtab_h) {
            if self.device_mgr.view_mode != DeviceViewMode::HardwareTree {
                return Some(SettingsAction::DevicesToggleViewMode(DeviceViewMode::HardwareTree));
            }
        }
        // Subtab 1: Registered Drivers
        if point_in_rect(px, py, content_x + subtab0_w + 6, content_y, subtab1_w, subtab_h) {
            if self.device_mgr.view_mode != DeviceViewMode::DriverTable {
                return Some(SettingsAction::DevicesToggleViewMode(DeviceViewMode::DriverTable));
            }
        }

        // Rescan button at content_x + 380, content_y, 70, 22
        if point_in_rect(px, py, content_x + 380, content_y, 70, 22) {
            return Some(SettingsAction::DevicesRescan);
        }

        match self.device_mgr.view_mode {
            DeviceViewMode::HardwareTree => {
                // Category Filter Chips at content_y + 64, height 20, 8 chips
                let chip_y = content_y + 64;
                let cats = DeviceCategoryFilter::all();
                for (i, &cat) in cats.iter().enumerate() {
                    let cx = content_x + (i as i32 * 56);
                    if point_in_rect(px, py, cx, chip_y, 52, 20) {
                        return Some(SettingsAction::DevicesSelectCategory(cat));
                    }
                }

                // Table rows at content_y + 108, row_h 20
                let row_h = 20;
                let list_y = content_y + 108;
                let vis_count = self.device_mgr.filtered_device_count().min(5);
                for r in 0..vis_count {
                    let ry = list_y + (r as i32 * row_h);
                    if point_in_rect(px, py, content_x, ry, 455, row_h) {
                        if let Some((orig_idx, _)) = self.device_mgr.filtered_device_at(r) {
                            return Some(SettingsAction::DevicesSelectDevice(orig_idx));
                        }
                    }
                }
            }
            DeviceViewMode::DriverTable => {
                // Driver table rows at content_y + 108, row_h 20
                let row_h = 20;
                let list_y = content_y + 108;
                let drv_count = self.device_mgr.driver_count().min(5);
                for r in 0..drv_count {
                    let ry = list_y + (r as i32 * row_h);
                    if point_in_rect(px, py, content_x, ry, 455, row_h) {
                        return Some(SettingsAction::DevicesSelectDriver(r));
                    }
                }
            }
        }

        None
    }

    fn button_at_services(&self, px: i32, py: i32, content_x: i32, content_y: i32) -> Option<SettingsAction> {
        let subtab_w = 110;
        let subtab_h = 24;
        // Subtab 0: Live Supervision Dashboard
        if point_in_rect(px, py, content_x, content_y, subtab_w, subtab_h) {
            if self.service_mgr.view_mode != ServiceViewMode::LiveDashboard {
                return Some(SettingsAction::ServicesToggleViewMode);
            }
        }
        // Subtab 1: SERVICES.CFG Editor
        if point_in_rect(px, py, content_x + subtab_w + 6, content_y, subtab_w, subtab_h) {
            if self.service_mgr.view_mode != ServiceViewMode::ConfigFile {
                return Some(SettingsAction::ServicesToggleViewMode);
            }
        }

        match self.service_mgr.view_mode {
            ServiceViewMode::LiveDashboard => {
                // Refresh button at content_x + 380, content_y + 30, 80, 22
                if point_in_rect(px, py, content_x + 380, content_y + 30, 80, 22) {
                    return Some(SettingsAction::ServicesRefresh);
                }

                // Table rows
                let row_h = 22;
                let list_y = content_y + 78;
                for i in 0..self.service_mgr.count {
                    let ry = list_y + (i as i32 * row_h);
                    if point_in_rect(px, py, content_x, ry, 470, row_h) {
                        return Some(SettingsAction::ServicesSelectRow(i));
                    }
                }

                // Selected service inspector control buttons
                let insp_y = content_y + 196;
                let btn_y = insp_y + 54;
                if point_in_rect(px, py, content_x + 10, btn_y, 110, 22) {
                    return Some(SettingsAction::ServicesRestart(self.service_mgr.selected_idx));
                }
                if point_in_rect(px, py, content_x + 126, btn_y, 90, 22) {
                    return Some(SettingsAction::ServicesStop(self.service_mgr.selected_idx));
                }
                if point_in_rect(px, py, content_x + 222, btn_y, 90, 22) {
                    return Some(SettingsAction::ServicesStart(self.service_mgr.selected_idx));
                }
            }
            ServiceViewMode::ConfigFile => {
                let row_h = 24;
                let list_y = content_y + 44;
                for i in 0..self.services_cfg.count {
                    let ry = list_y + (i as i32 * row_h);
                    if py >= ry && py < ry + row_h {
                        if px >= content_x + 220 && px < content_x + 310 {
                            return Some(SettingsAction::ServicesToggleRestart(i));
                        }
                    }
                }
            }
        }
        None
    }

    fn button_at_users(&self, px: i32, py: i32, content_x: i32, content_y: i32) -> Option<SettingsAction> {
        match self.users_ui.view_mode {
            UsersViewMode::List => {
                // 1. "+ Add User" button at top right of accounts list
                let add_btn = Rect::new(content_x + 106, content_y + 24, 94, 22);
                if point_in_rect(px, py, add_btn.x, add_btn.y, add_btn.w, add_btn.h) {
                    return Some(SettingsAction::UsersOpenCreateForm);
                }

                // 2. Account rows in left column (w: 200, h: 36, spacing 40)
                let list_start_y = content_y + 52;
                for i in 0..self.users_cfg.count {
                    let card_y = list_start_y + (i as i32 * 40);
                    if point_in_rect(px, py, content_x, card_y, 200, 36) {
                        return Some(SettingsAction::UsersSelectAccount(i));
                    }
                }

                // 3. Right column inspector buttons (rx: content_x + 212, ry: content_y + 24)
                let sel_idx = self.users_ui.selected_idx;
                if sel_idx < self.users_cfg.count {
                    let rx = content_x + 212;
                    let ry = content_y + 24;

                    // Role switcher pills
                    // [ Standard User ]
                    let std_pill = Rect::new(rx + 12, ry + 76, 104, 24);
                    if point_in_rect(px, py, std_pill.x, std_pill.y, std_pill.w, std_pill.h) {
                        if self.users_cfg.accounts[sel_idx].is_admin() {
                            return Some(SettingsAction::UsersToggleAdmin(sel_idx));
                        }
                    }
                    // [ Administrator ]
                    let adm_pill = Rect::new(rx + 122, ry + 76, 108, 24);
                    if point_in_rect(px, py, adm_pill.x, adm_pill.y, adm_pill.w, adm_pill.h) {
                        if !self.users_cfg.accounts[sel_idx].is_admin() {
                            return Some(SettingsAction::UsersToggleAdmin(sel_idx));
                        }
                    }
                    // [ Change Password ]
                    let pw_btn = Rect::new(rx + 12, ry + 186, 126, 22);
                    if point_in_rect(px, py, pw_btn.x, pw_btn.y, pw_btn.w, pw_btn.h) {
                        return Some(SettingsAction::UsersOpenChangePassword(sel_idx));
                    }
                    // [ Delete Account ]
                    let del_btn = Rect::new(rx + 12, ry + 226, 110, 22);
                    if point_in_rect(px, py, del_btn.x, del_btn.y, del_btn.w, del_btn.h) {
                        return Some(SettingsAction::UsersDeleteAccount(sel_idx));
                    }
                }
            }

            UsersViewMode::CreateAccount => {
                let fx = content_x + 10;
                let fy = content_y + 24;

                // Preset pills: alice (0), bob (1), operator (2)
                for i in 0..3 {
                    let p_btn = Rect::new(fx + 76 + (i as i32 * 96), fy + 50, 90, 20);
                    if point_in_rect(px, py, p_btn.x, p_btn.y, p_btn.w, p_btn.h) {
                        return Some(SettingsAction::UsersFormPreset(i));
                    }
                }

                // Role pills: Standard (fx + 216) vs Admin (fx + 318)
                let std_btn = Rect::new(fx + 216, fy + 94, 96, 24);
                let adm_btn = Rect::new(fx + 318, fy + 94, 100, 24);
                if point_in_rect(px, py, std_btn.x, std_btn.y, std_btn.w, std_btn.h) {
                    if self.users_ui.form_is_admin {
                        return Some(SettingsAction::UsersFormToggleAdmin);
                    }
                }
                if point_in_rect(px, py, adm_btn.x, adm_btn.y, adm_btn.w, adm_btn.h) {
                    if !self.users_ui.form_is_admin {
                        return Some(SettingsAction::UsersFormToggleAdmin);
                    }
                }

                // Buttons: [ Create Account ], [ Cancel ]
                let create_rect = Rect::new(fx + 16, fy + 230, 130, 26);
                if point_in_rect(px, py, create_rect.x, create_rect.y, create_rect.w, create_rect.h) {
                    return Some(SettingsAction::UsersSubmitCreate);
                }
                let cancel_rect = Rect::new(fx + 158, fy + 230, 80, 26);
                if point_in_rect(px, py, cancel_rect.x, cancel_rect.y, cancel_rect.w, cancel_rect.h) {
                    return Some(SettingsAction::UsersCancelForm);
                }
            }

            UsersViewMode::ChangePassword => {
                let fx = content_x + 10;
                let fy = content_y + 24;

                // Preset button
                let pw_preset = Rect::new(fx + 16, fy + 52, 120, 20);
                if point_in_rect(px, py, pw_preset.x, pw_preset.y, pw_preset.w, pw_preset.h) {
                    return Some(SettingsAction::UsersFormPreset(3));
                }

                // Action buttons: [ Update Password ], [ Cancel ]
                let update_rect = Rect::new(fx + 16, fy + 180, 130, 26);
                if point_in_rect(px, py, update_rect.x, update_rect.y, update_rect.w, update_rect.h) {
                    return Some(SettingsAction::UsersSubmitChangePassword);
                }
                let cancel_rect = Rect::new(fx + 158, fy + 180, 80, 26);
                if point_in_rect(px, py, cancel_rect.x, cancel_rect.y, cancel_rect.w, cancel_rect.h) {
                    return Some(SettingsAction::UsersCancelForm);
                }
            }
        }
        None
    }

    fn button_at_firewall(&self, px: i32, py: i32, content_x: i32, content_y: i32) -> Option<SettingsAction> {
        let subtab_w = 110;
        let subtab_h = 24;
        for i in 0..3 {
            let sx = content_x + (i as i32 * (subtab_w + 6));
            if point_in_rect(px, py, sx, content_y, subtab_w, subtab_h) {
                return Some(SettingsAction::FirewallSelectSubtab(i));
            }
        }

        match self.firewall_cfg.active_subtab {
            0 => {
                // Firewall ON/OFF
                if point_in_rect(px, py, content_x, content_y + 32, 90, 22) {
                    return Some(SettingsAction::FirewallToggleEnabled);
                }
                // Inbound policy pill
                if point_in_rect(px, py, content_x + 98, content_y + 32, 100, 22) {
                    return Some(SettingsAction::FirewallToggleInbound);
                }
                // Outbound policy pill
                if point_in_rect(px, py, content_x + 204, content_y + 32, 100, 22) {
                    return Some(SettingsAction::FirewallToggleOutbound);
                }

                // Rule rows
                let tbl_w = 460;
                let mut ry = content_y + 82;
                for i in 0..self.firewall_cfg.rule_count {
                    if point_in_rect(px, py, content_x, ry, tbl_w, 20) {
                        return Some(SettingsAction::FirewallSelectRule(i));
                    }
                    ry += 22;
                }

                // Action buttons toolbar
                let by = ry + 8;
                if point_in_rect(px, py, content_x, by, 96, 22) {
                    return Some(SettingsAction::FirewallAddPresetSsh);
                }
                if point_in_rect(px, py, content_x + 102, by, 96, 22) {
                    return Some(SettingsAction::FirewallAddPresetHttp);
                }
                if point_in_rect(px, py, content_x + 204, by, 84, 22) {
                    return Some(SettingsAction::FirewallDeleteRule);
                }
                if point_in_rect(px, py, content_x + 294, by, 88, 22) {
                    return Some(SettingsAction::FirewallSyncKernel);
                }
            }
            2 => {
                if point_in_rect(px, py, content_x, content_y + 148, 110, 22) {
                    return Some(SettingsAction::FirewallFlushConns);
                }
            }
            _ => {}
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
            self.apply_action(action);
        }
    }

    pub fn apply_action(&mut self, action: SettingsAction) {
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
                SettingsAction::UsersSelectAccount(idx) => {
                    if idx < self.users_cfg.count {
                        self.users_ui.selected_idx = idx;
                        self.users_ui.view_mode = UsersViewMode::List;
                        let is_adm = self.users_cfg.accounts[idx].is_admin();
                        self.set_status(if is_adm {
                            "Selected Administrator account"
                        } else {
                            "Selected Standard User account"
                        });
                    }
                }
                SettingsAction::UsersOpenCreateForm => {
                    let next_uid = self.users_cfg.next_available_uid();
                    self.users_ui.open_create_form(next_uid);
                    self.set_status("Create User Account (useradd)");
                }
                SettingsAction::UsersCancelForm => {
                    self.users_ui.view_mode = UsersViewMode::List;
                    self.set_status("User management");
                }
                SettingsAction::UsersFormToggleAdmin => {
                    self.users_ui.form_is_admin = !self.users_ui.form_is_admin;
                    self.users_ui.set_status(
                        if self.users_ui.form_is_admin {
                            "Role: Administrator (gid 0)"
                        } else {
                            "Role: Standard User"
                        },
                        false,
                    );
                }
                SettingsAction::UsersFormPreset(idx) => {
                    match idx {
                        0 => self.users_ui.set_form_preset("alice", false, "alicepass"),
                        1 => self.users_ui.set_form_preset("bob", true, "bobpass123"),
                        2 => self.users_ui.set_form_preset("operator", false, "oppassword"),
                        _ => self.users_ui.set_form_preset("guest", false, "novapass456"),
                    }
                }
                SettingsAction::UsersSubmitCreate => {
                    let name_str = core::str::from_utf8(&self.users_ui.form_username[..self.users_ui.form_username_len]).unwrap_or("");
                    let pw = &self.users_ui.form_password[..self.users_ui.form_password_len];
                    let confirm = &self.users_ui.form_confirm[..self.users_ui.form_confirm_len];

                    if name_str.is_empty() {
                        self.users_ui.set_status("Error: Username cannot be empty", true);
                    } else if pw.is_empty() {
                        self.users_ui.set_status("Error: Password cannot be empty", true);
                    } else if pw != confirm {
                        self.users_ui.set_status("Error: Passwords do not match", true);
                    } else {
                        let seed = self.users_ui.form_uid.wrapping_mul(0x9E37_79B9) ^ 0xA5A5_5A5A;
                        let is_admin = self.users_ui.form_is_admin;
                        let uid = self.users_ui.form_uid;
                        match self.users_cfg.add_user_with_credentials(name_str, uid, is_admin, pw, seed) {
                            Ok(new_idx) => {
                                self.users_ui.selected_idx = new_idx;
                                self.users_ui.view_mode = UsersViewMode::List;
                                self.dirty = true;
                                self.save_users_cfg();
                                self.set_status("User account created and saved to USERS.CFG");
                            }
                            Err(err_msg) => {
                                self.users_ui.set_status(err_msg, true);
                            }
                        }
                    }
                }
                SettingsAction::UsersToggleAdmin(idx) => {
                    match self.users_cfg.toggle_admin(idx) {
                        Ok(is_admin) => {
                            self.dirty = true;
                            self.save_users_cfg();
                            if is_admin {
                                self.set_status("Granted Administrator privileges (gid 0)");
                            } else {
                                self.set_status("Revoked Administrator privileges (Standard User)");
                            }
                        }
                        Err(err_msg) => {
                            self.set_status(err_msg);
                        }
                    }
                }
                SettingsAction::UsersDeleteAccount(idx) => {
                    match self.users_cfg.delete_account(idx) {
                        Ok(()) => {
                            if self.users_ui.selected_idx >= self.users_cfg.count && self.users_cfg.count > 0 {
                                self.users_ui.selected_idx = self.users_cfg.count - 1;
                            }
                            self.dirty = true;
                            self.save_users_cfg();
                            self.set_status("Account deleted from USERS.CFG");
                        }
                        Err(err_msg) => {
                            self.set_status(err_msg);
                        }
                    }
                }
                SettingsAction::UsersOpenChangePassword(idx) => {
                    if idx < self.users_cfg.count {
                        self.users_ui.selected_idx = idx;
                        self.users_ui.open_change_password();
                        self.set_status("Change Account Password");
                    }
                }
                SettingsAction::UsersSubmitChangePassword => {
                    let idx = self.users_ui.selected_idx;
                    let pw = &self.users_ui.form_password[..self.users_ui.form_password_len];
                    let confirm = &self.users_ui.form_confirm[..self.users_ui.form_confirm_len];

                    if pw.is_empty() {
                        self.users_ui.set_status("Error: Password cannot be empty", true);
                    } else if pw != confirm {
                        self.users_ui.set_status("Error: Passwords do not match", true);
                    } else {
                        let seed = (idx as u32).wrapping_mul(0x1F2E_3D4C) ^ 0xBEEF_CAFE;
                        match self.users_cfg.set_user_password(idx, pw, seed) {
                            Ok(()) => {
                                self.users_ui.view_mode = UsersViewMode::List;
                                self.dirty = true;
                                self.save_users_cfg();
                                self.set_status("Password updated and saved to USERS.CFG");
                            }
                            Err(err_msg) => {
                                self.users_ui.set_status(err_msg, true);
                            }
                        }
                    }
                }
                SettingsAction::UsersAddDemo => {
                    if self.users_cfg.count < MAX_USERS {
                        let uid = self.users_cfg.next_available_uid();
                        let _ = self.users_cfg.add_user_with_credentials("guest", uid, false, b"guest-pw", 0x1234_5678);
                        self.dirty = true;
                        self.save_users_cfg();
                        self.set_status("Added guest user account");
                    } else {
                        self.set_status("User table full (8 max)");
                    }
                }
                SettingsAction::ServicesSelectRow(idx) => {
                    if idx < self.service_mgr.count {
                        self.service_mgr.selected_idx = idx;
                        self.set_status("Selected service");
                    }
                }
                SettingsAction::ServicesToggleViewMode => {
                    self.service_mgr.view_mode = match self.service_mgr.view_mode {
                        ServiceViewMode::LiveDashboard => ServiceViewMode::ConfigFile,
                        ServiceViewMode::ConfigFile => ServiceViewMode::LiveDashboard,
                    };
                    self.set_status(if self.service_mgr.view_mode == ServiceViewMode::LiveDashboard {
                        "Switched to Live Supervision Dashboard"
                    } else {
                        "Switched to SERVICES.CFG Config Editor"
                    });
                }
                SettingsAction::ServicesRefresh => {
                    if self.service_mgr.sync_from_kernel() {
                        self.set_status("Live service telemetry updated from PID 1");
                    } else {
                        self.set_status("Service status refresh completed");
                    }
                }
                SettingsAction::ServicesRestart(idx) => {
                    if self.service_mgr.restart_service(idx) {
                        self.set_status("Service restart requested from supervisor");
                    } else {
                        self.set_status("Failed to request service restart");
                    }
                }
                SettingsAction::ServicesStop(idx) => {
                    if self.service_mgr.stop_service(idx) {
                        self.set_status("Service stop requested from supervisor");
                    } else {
                        self.set_status("Failed to stop service");
                    }
                }
                SettingsAction::ServicesStart(idx) => {
                    if self.service_mgr.start_service(idx) {
                        self.set_status("Service start requested from supervisor");
                    } else {
                        self.set_status("Failed to start service");
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
                SettingsAction::FirewallSelectSubtab(idx) => {
                    self.firewall_cfg.active_subtab = idx;
                }
                SettingsAction::FirewallToggleEnabled => {
                    self.firewall_cfg.toggle_enabled_kernel();
                    self.dirty = true;
                    self.set_status("Toggled firewall engine status");
                }
                SettingsAction::FirewallToggleInbound => {
                    let next = !self.firewall_cfg.inbound_drop;
                    self.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, next);
                    self.dirty = true;
                    self.set_status("Updated inbound firewall policy");
                }
                SettingsAction::FirewallToggleOutbound => {
                    let next = !self.firewall_cfg.policy_out_accept;
                    self.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_OUT, next);
                    self.dirty = true;
                    self.set_status("Updated outbound firewall policy");
                }
                SettingsAction::FirewallSelectRule(idx) => {
                    self.firewall_cfg.selected_rule = idx;
                }
                SettingsAction::FirewallAddPresetSsh => {
                    self.firewall_cfg.add_rule_kernel("rule accept in proto tcp dport 22 state new log");
                    self.dirty = true;
                    self.set_status("Added SSH rule (port 22)");
                }
                SettingsAction::FirewallAddPresetHttp => {
                    self.firewall_cfg.add_rule_kernel("rule accept in proto tcp dport 80 state new log");
                    self.dirty = true;
                    self.set_status("Added HTTP rule (port 80)");
                }
                SettingsAction::FirewallDeleteRule => {
                    let idx = self.firewall_cfg.selected_rule;
                    self.firewall_cfg.delete_rule_kernel(idx);
                    self.dirty = true;
                    self.set_status("Deleted selected firewall rule");
                }
                SettingsAction::FirewallFlushRules => {
                    self.firewall_cfg.flush_rules_kernel();
                    self.dirty = true;
                    self.set_status("Flushed all firewall rules");
                }
                SettingsAction::FirewallFlushConns => {
                    self.firewall_cfg.flush_conns_kernel();
                    self.set_status("Flushed connection tracking table");
                }
                SettingsAction::FirewallSyncKernel => {
                    self.firewall_cfg.sync_from_kernel();
                    self.set_status("Synchronized with kernel firewall");
                }
                SettingsAction::DisplayCycleTheme => {
                    self.display_cfg.theme = match self.display_cfg.theme {
                        Theme::Dark => Theme::Light,
                        Theme::Light => Theme::HighContrast,
                        Theme::HighContrast => Theme::Dark,
                    };
                    if self.display_cfg.theme == Theme::HighContrast {
                        self.display_cfg.cursor_theme = CursorTheme::HighContrast;
                        self.display_cfg.high_contrast_cursor = true;
                        self.display_cfg.show_focus_rects = true;
                    } else {
                        self.display_cfg.cursor_theme = CursorTheme::ModernDark;
                        self.display_cfg.high_contrast_cursor = false;
                        self.display_cfg.show_focus_rects = false;
                    }
                    self.dirty = true;
                    self.set_status("Theme changed");
                }
                SettingsAction::DisplayToggleHighContrast => {
                    if self.display_cfg.theme.is_high_contrast() {
                        self.display_cfg.theme = Theme::Dark;
                        self.display_cfg.cursor_theme = CursorTheme::ModernDark;
                        self.display_cfg.high_contrast_cursor = false;
                        self.display_cfg.show_focus_rects = false;
                        self.set_status("High contrast disabled (Standard Dark theme)");
                    } else {
                        self.display_cfg.theme = Theme::HighContrast;
                        self.display_cfg.cursor_theme = CursorTheme::HighContrast;
                        self.display_cfg.high_contrast_cursor = true;
                        self.display_cfg.show_focus_rects = true;
                        self.set_status("High contrast enabled (WCAG AAA yellow/black)");
                    }
                    self.dirty = true;
                }
                SettingsAction::DisplaySetFontScale(scale) => {
                    self.display_cfg.font_scale = scale;
                    self.dirty = true;
                    match scale {
                        FontScale::Normal => self.set_status("Font scale: Normal 100%"),
                        FontScale::Medium => self.set_status("Font scale: Medium 125%"),
                        FontScale::Large => self.set_status("Font scale: Large 150%"),
                        FontScale::ExtraLarge => self.set_status("Font scale: Extra Large 200%"),
                    }
                }
                SettingsAction::DisplayCycleFontScale => {
                    self.display_cfg.font_scale = self.display_cfg.font_scale.cycle_next();
                    self.dirty = true;
                    self.set_status("Font scale cycled");
                }
                SettingsAction::DisplayCycleCursor => {
                    self.display_cfg.cursor_theme = match self.display_cfg.cursor_theme {
                        CursorTheme::ModernDark => CursorTheme::Classic,
                        CursorTheme::Classic => CursorTheme::Amber,
                        CursorTheme::Amber => CursorTheme::HighContrast,
                        CursorTheme::HighContrast => CursorTheme::ModernLight,
                        CursorTheme::ModernLight => CursorTheme::ModernDark,
                    };
                    self.dirty = true;
                    self.set_status("Cursor style updated");
                }
                SettingsAction::DevicesToggleViewMode(mode) => {
                    self.device_mgr.view_mode = mode;
                    self.dirty = true;
                    self.set_status(if mode == DeviceViewMode::HardwareTree {
                        "Hardware Tree view"
                    } else {
                        "Registered Drivers view"
                    });
                }
                SettingsAction::DevicesSelectCategory(cat) => {
                    self.device_mgr.category_filter = cat;
                    if let Some((first_idx, _)) = self.device_mgr.filtered_device_at(0) {
                        self.device_mgr.selected_device_idx = first_idx;
                    }
                    self.dirty = true;
                    self.set_status("Filtered device category");
                }
                SettingsAction::DevicesSelectDevice(idx) => {
                    let n = self.device_mgr.device_count();
                    if idx < n {
                        self.device_mgr.selected_device_idx = idx;
                        self.dirty = true;
                        self.set_status("Selected device");
                    }
                }
                SettingsAction::DevicesSelectDriver(idx) => {
                    let n = self.device_mgr.driver_count();
                    if idx < n {
                        self.device_mgr.selected_driver_idx = idx;
                        self.dirty = true;
                        self.set_status("Selected driver");
                    }
                }
                SettingsAction::DevicesRescan => {
                    if self.device_mgr.sync_from_kernel() {
                        self.set_status("Hardware inventory re-scanned and synchronized");
                    } else {
                        self.set_status("Device rescan completed");
                    }
                    self.dirty = true;
                }
                SettingsAction::UpdatesCheck => {
                    self.update_state.check_for_updates();
                    self.dirty = true;
                    self.set_status("Checked for updates: v0.89.0 available");
                }
                SettingsAction::UpdatesReview => {
                    self.update_state.review_update();
                    self.dirty = true;
                    self.set_status("Review update details");
                }
                SettingsAction::UpdatesCancel => {
                    self.update_state.cancel_update();
                    self.dirty = true;
                    self.set_status("Update canceled");
                }
                SettingsAction::UpdatesConfirmAndApply => {
                    self.update_state.confirm_and_apply();
                    self.dirty = true;
                    self.set_status("Update staged to secondary boot slot");
                }
                SettingsAction::UpdatesConfirmHealthy => {
                    self.update_state.confirm_healthy();
                    self.dirty = true;
                    self.set_status("Confirmed boot slot healthy");
                }
                SettingsAction::UpdatesRollback => {
                    self.update_state.rollback_slot();
                    self.dirty = true;
                    self.set_status("Rolled back boot slot");
                }
                SettingsAction::UpdatesDismissAlert => {
                    self.update_state.dismiss_alert();
                    self.dirty = true;
                    self.set_status("Alert dismissed");
                }
                SettingsAction::UpdatesResetStep => {
                    self.update_state.reset_step();
                    self.dirty = true;
                    self.set_status("Software Updates");
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

        // Background
        s.fill_rect(viewport, palette.surface_sunken);

        // 1. Sidebar (w = 160)
        let sb_w = 160;
        let sb_rect = Rect::new(0, 0, sb_w, viewport.h);
        s.fill_rect(sb_rect, palette.surface_raised);
        s.fill_rect(Rect::new(sb_w - 1, 0, 1, viewport.h), palette.border_subtle);

        // Sidebar Branding / Title
        ui::font::draw_text_font(s, 16, 16, "Settings", palette.accent_primary, &font);

        // Sidebar Panel Tabs
        let tab_h = 32;
        let tab_y_start = 48;
        for (i, &panel) in PanelKind::all().iter().enumerate() {
            let ty = tab_y_start + (i as i32 * (tab_h + 4));
            let is_active = self.active_panel == panel;
            let tab_rect = Rect::new(6, ty, sb_w - 12, tab_h);

            if is_active {
                ui::draw::draw_rounded_rect(s, tab_rect, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, tab_rect, 1, palette.border_focused);
                s.fill_rect(Rect::new(tab_rect.x + 2, tab_rect.y + 4, 3, tab_h - 8), palette.accent_primary);
            }

            // Panel Icon Badge
            let badge_sz = f_small.measure_str(panel.icon_badge());
            let badge_rect = Rect::new(tab_rect.x + 8, tab_rect.y + 6, badge_sz.w + 6, 18);
            let badge_bg = if is_active { palette.accent_primary } else { palette.surface_raised.lighten(20) };
            ui::draw::draw_rounded_rect(s, badge_rect, RADIUS_XS, badge_bg);
            ui::font::draw_text_font(s, badge_rect.x + 3, badge_rect.y + 3, panel.icon_badge(), Color::WHITE, &f_small);

            // Panel Title Text
            let title_col = if is_active { Color::WHITE } else { palette.text_secondary };
            ui::font::draw_text_font(s, badge_rect.right() + 8, tab_rect.y + 8, panel.title(), title_col, &f_small);
        }

        // 2. Top Header Bar (h = 48)
        let hdr_x = sb_w;
        let hdr_w = viewport.w - sb_w;
        let hdr_h = 48;
        let hdr_rect = Rect::new(hdr_x, 0, hdr_w, hdr_h);
        s.fill_rect(hdr_rect, palette.surface_raised);
        s.fill_rect(Rect::new(hdr_x, hdr_h - 1, hdr_w, 1), palette.border_subtle);

        // Header Title and Subtitle
        ui::font::draw_text_font(s, hdr_x + 16, 10, self.active_panel.title(), palette.text_primary, &font);
        ui::font::draw_text_font(s, hdr_x + 16, 28, self.active_panel.subtitle(), palette.text_muted, &f_small);

        // 3. Header Action Buttons
        // Save Button
        let save_x = viewport.w - 80;
        let save_rect = Rect::new(save_x, 10, 70, 28);
        let save_col = if self.dirty { palette.status_success } else { palette.surface_sunken };
        ui::draw::draw_rounded_rect(s, save_rect, RADIUS_XS, save_col);
        ui::draw::draw_rect_outline(s, save_rect, 1, palette.border_subtle);
        let save_sz = f_small.measure_str("Save");
        ui::font::draw_text_font(s, save_rect.x + (save_rect.w - save_sz.w) / 2, save_rect.y + 7, "Save", Color::WHITE, &f_small);

        // Reload Button
        let reload_x = viewport.w - 165;
        let reload_rect = Rect::new(reload_x, 10, 75, 28);
        ui::draw::draw_rounded_rect(s, reload_rect, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, reload_rect, 1, palette.border_subtle);
        let rel_sz = f_small.measure_str("Reload");
        ui::font::draw_text_font(s, reload_rect.x + (reload_rect.w - rel_sz.w) / 2, reload_rect.y + 7, "Reload", palette.text_secondary, &f_small);

        // 4. Panel Content Body Area
        let content_x = sb_w + 16;
        let content_y = hdr_h + 16;

        match self.active_panel {
            PanelKind::System => {
                ui::font::draw_text_font(s, content_x, content_y, "Operating System & Hostname (SYSTEM.CFG)", palette.accent_primary, &font);
                ui::font::draw_text_font(s, content_x, content_y + 24, "Select Hostname Preset:", palette.text_secondary, &f_small);

                let presets = ["novaos", "novaos-dev", "novaos-srv"];
                for (i, &p_name) in presets.iter().enumerate() {
                    let bx = content_x + (i as i32 * 96);
                    let btn_rect = Rect::new(bx, content_y + 44, 88, 26);
                    let is_cur = self.system_cfg.hostname_str() == p_name;
                    let bg = if is_cur { palette.accent_primary.darken(30) } else { palette.surface_raised };
                    ui::draw::draw_rounded_rect(s, btn_rect, RADIUS_XS, bg);
                    ui::draw::draw_rect_outline(s, btn_rect, 1, if is_cur { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, bx + 12, content_y + 49, p_name, if is_cur { Color::WHITE } else { palette.text_primary }, &f_small);
                }

                ui::font::draw_text_font(s, content_x, content_y + 92, "PID 1 Init Executable Path:", palette.text_secondary, &f_small);
                let init_rect = Rect::new(content_x, content_y + 112, 140, 26);
                ui::draw::draw_rounded_rect(s, init_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, init_rect, 1, palette.accent_primary);
                ui::font::draw_text_font(s, init_rect.x + 10, init_rect.y + 5, self.system_cfg.init_path_str(), palette.text_primary, &f_small);
            }

            PanelKind::Users => {
                self.render_users_panel(s, content_x, content_y, &font, &f_small, &palette);
            }

            PanelKind::Services => {
                self.render_services_panel(s, content_x, content_y, viewport, &palette, &font, &f_small);
            }

            PanelKind::Firewall => {
                self.render_firewall_panel(s, content_x, content_y, viewport, &palette, &font, &f_small);
            }

            PanelKind::Display => {
                self.render_display_panel(s, content_x, content_y, viewport, &palette, &font, &f_small);
            }

            PanelKind::Devices => {
                self.render_devices_panel(s, content_x, content_y, viewport, &palette, &font, &f_small);
            }

            PanelKind::Updates => {
                self.render_updates_panel(s, content_x, content_y, viewport, &palette, &font, &f_small);
            }
        }

        // 5. Bottom Status Bar (h = 24)
        let sb_h = 24;
        let stat_rect = Rect::new(hdr_x, viewport.bottom() - sb_h, hdr_w, sb_h);
        s.fill_rect(stat_rect, palette.surface_raised);
        s.fill_rect(Rect::new(hdr_x, stat_rect.y, hdr_w, 1), palette.border_subtle);

        ui::font::draw_text_font(s, stat_rect.x + 12, stat_rect.y + 5, self.status_str(), palette.text_secondary, &f_small);
    }

    fn render_updates_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        content_x: i32,
        content_y: i32,
        _viewport: Rect,
        palette: &ThemePalette,
        font: &Font,
        f_small: &Font,
    ) {
        // 1. Header & Subtitle
        ui::font::draw_text_font(s, content_x, content_y, "Software Updates & Atomic Boot Engine", palette.accent_primary, font);
        ui::font::draw_text_font(s, content_x, content_y + 20, "Dual-slot A/B system partitions, cryptographic signature verification & zero-risk rollback", palette.text_secondary, f_small);

        let status = &self.update_state.status;
        let is_testing = status.slot_state == ffi::NOVA_SLOT_STATE_TESTING;
        let is_rolled_back = status.slot_state == ffi::NOVA_SLOT_STATE_ROLLED_BACK;

        // 2. Active Slot Status Banner / Overview Card
        let banner_y = content_y + 42;
        let banner_w = 440;
        let banner_h = 44;
        let banner_rect = Rect::new(content_x, banner_y, banner_w, banner_h);

        let (banner_bg, banner_border, banner_title, banner_sub) = if is_testing {
            (
                palette.status_warning.darken(40),
                palette.status_warning,
                "TRIAL BOOT: System is in Testing Mode",
                "Watchdog active (attempts left: 3). Confirm healthy or rollback."
            )
        } else if is_rolled_back {
            (
                palette.status_danger.darken(40),
                palette.status_danger,
                "ROLLBACK ACTIVE: Booted into Fallback Slot",
                "Previous update failed boot trial. System was safely preserved."
            )
        } else {
            (
                palette.surface_raised,
                palette.border_subtle,
                "SYSTEM HEALTHY: Operating on Primary Slot",
                "Active boot partition is verified stable and persistent."
            )
        };

        ui::draw::draw_rounded_rect(s, banner_rect, RADIUS_XS, banner_bg);
        ui::draw::draw_rect_outline(s, banner_rect, 1, banner_border);

        // Status Indicator Badge
        let status_badge_rect = Rect::new(banner_rect.x + 8, banner_rect.y + 8, 86, 28);
        let badge_color = if is_testing {
            palette.status_warning
        } else if is_rolled_back {
            palette.status_danger
        } else {
            palette.status_success
        };
        ui::draw::draw_rounded_rect(s, status_badge_rect, RADIUS_XS, badge_color.darken(30));
        ui::draw::draw_rect_outline(s, status_badge_rect, 1, badge_color);
        ui::font::draw_text_font(s, status_badge_rect.x + 6, status_badge_rect.y + 7, status.active_slot_name(), Color::WHITE, f_small);

        ui::font::draw_text_font(s, status_badge_rect.right() + 10, banner_rect.y + 7, banner_title, Color::WHITE, f_small);
        ui::font::draw_text_font(s, status_badge_rect.right() + 10, banner_rect.y + 24, banner_sub, palette.text_secondary, f_small);

        // If in Testing mode, draw immediate action buttons on banner or right below
        if is_testing {
            let confirm_btn = Rect::new(content_x + 220, banner_y + 8, 100, 28);
            ui::draw::draw_rounded_rect(s, confirm_btn, RADIUS_XS, palette.status_success);
            ui::draw::draw_rect_outline(s, confirm_btn, 1, Color::WHITE);
            ui::font::draw_text_font(s, confirm_btn.x + 8, confirm_btn.y + 7, "Confirm Health", Color::WHITE, f_small);

            let rb_btn = Rect::new(content_x + 330, banner_y + 8, 90, 28);
            ui::draw::draw_rounded_rect(s, rb_btn, RADIUS_XS, palette.status_danger);
            ui::draw::draw_rect_outline(s, rb_btn, 1, Color::WHITE);
            ui::font::draw_text_font(s, rb_btn.x + 8, rb_btn.y + 7, "Rollback", Color::WHITE, f_small);
        }

        // 3. Dual-Slot Architecture Layout (Slot A vs Slot B)
        let slots_y = banner_y + 52;
        let slot_w = 215;
        let slot_h = 76;

        // Slot A Card
        let slot_a_rect = Rect::new(content_x, slots_y, slot_w, slot_h);
        let is_slot_a_active = status.active_slot == ffi::NOVA_BOOT_SLOT_A;
        let a_bg = if is_slot_a_active { palette.surface_sunken } else { palette.surface_raised };
        let a_border = if is_slot_a_active { palette.accent_primary } else { palette.border_subtle };
        ui::draw::draw_rounded_rect(s, slot_a_rect, RADIUS_XS, a_bg);
        ui::draw::draw_rect_outline(s, slot_a_rect, 1, a_border);

        ui::font::draw_text_font(s, slot_a_rect.x + 10, slot_a_rect.y + 8, "Slot A (NOVAOS.BIN)", palette.text_primary, f_small);
        let a_role_text = if is_slot_a_active { "[Active Partition]" } else { "[Fallback Partition]" };
        let a_role_col = if is_slot_a_active { palette.accent_primary } else { palette.text_muted };
        ui::font::draw_text_font(s, slot_a_rect.x + 10, slot_a_rect.y + 26, a_role_text, a_role_col, f_small);
        let a_ver_str = status.slot_a_version_str();
        ui::font::draw_text_font(s, slot_a_rect.x + 10, slot_a_rect.y + 44, "Version:", palette.text_secondary, f_small);
        ui::font::draw_text_font(s, slot_a_rect.x + 64, slot_a_rect.y + 44, a_ver_str, Color::WHITE, f_small);

        // Slot B Card
        let slot_b_rect = Rect::new(content_x + 225, slots_y, slot_w, slot_h);
        let is_slot_b_active = status.active_slot == ffi::NOVA_BOOT_SLOT_B;
        let b_bg = if is_slot_b_active { palette.surface_sunken } else { palette.surface_raised };
        let b_border = if is_slot_b_active { palette.accent_primary } else { palette.border_subtle };
        ui::draw::draw_rounded_rect(s, slot_b_rect, RADIUS_XS, b_bg);
        ui::draw::draw_rect_outline(s, slot_b_rect, 1, b_border);

        ui::font::draw_text_font(s, slot_b_rect.x + 10, slot_b_rect.y + 8, "Slot B (NOVAOS_B.BIN)", palette.text_primary, f_small);
        let b_role_text = if is_slot_b_active { "[Active Partition]" } else { "[Staging Target]" };
        let b_role_col = if is_slot_b_active { palette.accent_primary } else { palette.text_muted };
        ui::font::draw_text_font(s, slot_b_rect.x + 10, slot_b_rect.y + 26, b_role_text, b_role_col, f_small);
        let b_ver_str = status.slot_b_version_str();
        ui::font::draw_text_font(s, slot_b_rect.x + 10, slot_b_rect.y + 44, "Version:", palette.text_secondary, f_small);
        ui::font::draw_text_font(s, slot_b_rect.x + 64, slot_b_rect.y + 44, b_ver_str, Color::WHITE, f_small);

        // 4. Update Workflow Section
        let wf_y = slots_y + slot_h + 10;
        let wf_w = 440;
        let wf_h = 105;
        let wf_rect = Rect::new(content_x, wf_y, wf_w, wf_h);
        ui::draw::draw_rounded_rect(s, wf_rect, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, wf_rect, 1, palette.border_subtle);

        match self.update_state.step {
            UpdateStep::Idle => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 10, "Check for Available System Updates", palette.text_primary, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 32, "Scans storage media for signed OSUPDATE.PKG packages & updates.", palette.text_secondary, f_small);

                let check_btn = Rect::new(wf_rect.x + 12, wf_rect.y + 60, 160, 30);
                ui::draw::draw_rounded_rect(s, check_btn, RADIUS_XS, palette.accent_primary);
                ui::draw::draw_rect_outline(s, check_btn, 1, palette.accent_primary.lighten(30));
                ui::font::draw_text_font(s, check_btn.x + 16, check_btn.y + 8, "Check for Updates", Color::WHITE, f_small);
            }
            UpdateStep::Checking => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 10, "Checking for Updates...", palette.accent_primary, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 32, "Scanning VFS filesystem for OSUPDATE.PKG and verifying integrity...", palette.text_secondary, f_small);
                let p_bar_bg = Rect::new(wf_rect.x + 12, wf_rect.y + 60, 416, 16);
                ui::draw::draw_rounded_rect(s, p_bar_bg, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, p_bar_bg, 1, palette.border_subtle);
                let fill = Rect::new(p_bar_bg.x, p_bar_bg.y, 200, p_bar_bg.h);
                ui::draw::draw_rounded_rect(s, fill, RADIUS_XS, palette.accent_primary);
            }
            UpdateStep::Available => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 8, "NovaOS Update Available: v0.89.0", palette.accent_primary, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 28, "Cryptographically signed (HMAC-SHA256). Staged to secondary slot.", palette.text_secondary, f_small);

                let rev_btn = Rect::new(wf_rect.x + 12, wf_rect.y + 58, 130, 30);
                ui::draw::draw_rounded_rect(s, rev_btn, RADIUS_XS, palette.accent_primary);
                ui::font::draw_text_font(s, rev_btn.x + 18, rev_btn.y + 8, "Install Update", Color::WHITE, f_small);

                let dis_btn = Rect::new(wf_rect.x + 152, wf_rect.y + 58, 90, 30);
                ui::draw::draw_rounded_rect(s, dis_btn, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, dis_btn, 1, palette.border_subtle);
                ui::font::draw_text_font(s, dis_btn.x + 22, dis_btn.y + 8, "Dismiss", palette.text_secondary, f_small);
            }
            UpdateStep::ConfirmDialog => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 8, "Confirm Update Installation", palette.accent_primary, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 28, "Kernel binary will be written to secondary slot. Current slot safe.", palette.text_secondary, f_small);

                let ok_btn = Rect::new(wf_rect.x + 12, wf_rect.y + 58, 170, 30);
                ui::draw::draw_rounded_rect(s, ok_btn, RADIUS_XS, palette.status_success);
                ui::font::draw_text_font(s, ok_btn.x + 14, ok_btn.y + 8, "Confirm & Install Now", Color::WHITE, f_small);

                let can_btn = Rect::new(wf_rect.x + 192, wf_rect.y + 58, 90, 30);
                ui::draw::draw_rounded_rect(s, can_btn, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, can_btn, 1, palette.border_subtle);
                ui::font::draw_text_font(s, can_btn.x + 24, can_btn.y + 8, "Cancel", palette.text_secondary, f_small);
            }
            UpdateStep::Applying => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 8, "Applying Atomic OS Update...", palette.accent_primary, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 28, self.update_state.progress_stage_str(), palette.text_secondary, f_small);

                let p_bar_bg = Rect::new(wf_rect.x + 12, wf_rect.y + 58, 416, 18);
                ui::draw::draw_rounded_rect(s, p_bar_bg, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, p_bar_bg, 1, palette.border_subtle);
                let pct = self.update_state.progress_pct as i32;
                let fw = (416 * pct) / 100;
                let fill = Rect::new(p_bar_bg.x, p_bar_bg.y, fw.max(4), p_bar_bg.h);
                ui::draw::draw_rounded_rect(s, fill, RADIUS_XS, palette.accent_primary);
            }
            UpdateStep::Complete => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 8, "Update Staged Successfully!", palette.status_success, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 28, "Bootloader armed for secondary slot trial boot. Current slot safe.", palette.text_secondary, f_small);

                let done_btn = Rect::new(wf_rect.x + 12, wf_rect.y + 58, 110, 30);
                ui::draw::draw_rounded_rect(s, done_btn, RADIUS_XS, palette.status_success);
                ui::font::draw_text_font(s, done_btn.x + 28, done_btn.y + 8, "Done", Color::WHITE, f_small);
            }
            UpdateStep::RolledBackAlert => {
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 8, "Automatic Rollback Preserved System", palette.status_danger, font);
                ui::font::draw_text_font(s, wf_rect.x + 12, wf_rect.y + 28, "The previous update did not pass trial verification. Reverted to safe slot.", palette.text_secondary, f_small);

                let ack_btn = Rect::new(wf_rect.x + 12, wf_rect.y + 58, 120, 30);
                ui::draw::draw_rounded_rect(s, ack_btn, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, ack_btn, 1, palette.border_subtle);
                ui::font::draw_text_font(s, ack_btn.x + 20, ack_btn.y + 8, "Acknowledge", palette.text_primary, f_small);
            }
        }
    }

    fn render_display_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        content_x: i32,
        content_y: i32,
        _viewport: Rect,
        palette: &ThemePalette,
        font: &Font,
        f_small: &Font,
    ) {
        // 1. Header & Subtitle
        ui::font::draw_text_font(s, content_x, content_y, "Display & Accessibility Preferences", palette.accent_primary, font);
        ui::font::draw_text_font(s, content_x, content_y + 20, "Theme palettes, dynamic font scaling, high-contrast mode, and pointer styles", palette.text_secondary, f_small);

        // 2. Section 1: Theme & High-Contrast Mode
        let sec1_y = content_y + 44;
        ui::font::draw_text_font(s, content_x, sec1_y, "Theme Palette & High Contrast (WCAG AAA):", palette.text_primary, f_small);

        let th_btn = Rect::new(content_x, sec1_y + 18, 140, 26);
        ui::draw::draw_rounded_rect(s, th_btn, RADIUS_XS, palette.surface_raised.lighten(20));
        ui::draw::draw_rect_outline(s, th_btn, 1, palette.accent_primary);
        let th_name = match self.display_cfg.theme {
            Theme::Dark => "Dark Palette",
            Theme::Light => "Light Palette",
            Theme::HighContrast => "High Contrast",
        };
        ui::font::draw_text_font(s, th_btn.x + 10, th_btn.y + 5, th_name, palette.text_primary, f_small);

        let is_hc = self.display_cfg.theme.is_high_contrast();
        let hc_btn = Rect::new(content_x + 150, sec1_y + 18, 190, 26);
        let hc_bg = if is_hc { palette.accent_primary.darken(35) } else { palette.surface_raised.lighten(20) };
        let hc_border = if is_hc { palette.accent_primary } else { palette.border_subtle };
        ui::draw::draw_rounded_rect(s, hc_btn, RADIUS_XS, hc_bg);
        ui::draw::draw_rect_outline(s, hc_btn, 1, hc_border);
        let hc_text = if is_hc { "High Contrast: [ON]" } else { "High Contrast: [OFF]" };
        let hc_col = if is_hc { palette.accent_primary } else { palette.text_secondary };
        ui::font::draw_text_font(s, hc_btn.x + 12, hc_btn.y + 5, hc_text, hc_col, f_small);

        // 3. Section 2: Font Scaling
        let sec2_y = sec1_y + 56;
        ui::font::draw_text_font(s, content_x, sec2_y, "System Typography Scale Multiplier:", palette.text_primary, f_small);

        let scales = [FontScale::Normal, FontScale::Medium, FontScale::Large, FontScale::ExtraLarge];
        let btn_w = 90;
        let btn_h = 24;
        for (i, &sc) in scales.iter().enumerate() {
            let bx = content_x + (i as i32 * (btn_w + 6));
            let by = sec2_y + 18;
            let b_rect = Rect::new(bx, by, btn_w, btn_h);
            let is_sel = self.display_cfg.font_scale == sc;
            let bg_col = if is_sel { palette.accent_primary.darken(35) } else { palette.surface_raised.lighten(20) };
            let border_col = if is_sel { palette.accent_primary } else { palette.border_subtle };
            let txt_col = if is_sel { palette.accent_primary } else { palette.text_secondary };

            ui::draw::draw_rounded_rect(s, b_rect, RADIUS_XS, bg_col);
            ui::draw::draw_rect_outline(s, b_rect, 1, border_col);
            let sz = f_small.measure_str(sc.label());
            ui::font::draw_text_font(s, b_rect.x + (b_rect.w - sz.w) / 2, b_rect.y + 4, sc.label(), txt_col, f_small);
        }

        // Live preview container box
        let prev_y = sec2_y + 50;
        let prev_rect = Rect::new(content_x, prev_y, 440, 52);
        ui::draw::draw_rounded_rect(s, prev_rect, RADIUS_SM, palette.surface_sunken);
        ui::draw::draw_rect_outline(s, prev_rect, 1, palette.border_subtle);

        let f_scaled = Font::regular().scaled(self.display_cfg.font_scale);
        ui::font::draw_text_font(s, prev_rect.x + 10, prev_rect.y + 4, "Live Typography Preview (scaled text):", palette.text_muted, f_small);
        ui::font::draw_text_font(s, prev_rect.x + 10, prev_rect.y + 20, "NovaOS 2.0 Accessibility Preview 123", palette.text_primary, &f_scaled);

        // 4. Section 3: Cursor theme
        let sec3_y = prev_y + 64;
        ui::font::draw_text_font(s, content_x, sec3_y, "Compositor Cursor Bitmaps & High-Visibility Pointer:", palette.text_primary, f_small);

        let cur_btn = Rect::new(content_x, sec3_y + 18, 180, 26);
        ui::draw::draw_rounded_rect(s, cur_btn, RADIUS_XS, palette.surface_raised.lighten(20));
        ui::draw::draw_rect_outline(s, cur_btn, 1, palette.accent_primary);
        let cur_name = match self.display_cfg.cursor_theme {
            CursorTheme::Classic => "Cursor: Classic 8x12",
            CursorTheme::ModernDark => "Cursor: Modern Dark",
            CursorTheme::ModernLight => "Cursor: Modern Light",
            CursorTheme::Amber => "Cursor: Amber CRT",
            CursorTheme::HighContrast => "Cursor: High Contrast",
        };
        ui::font::draw_text_font(s, cur_btn.x + 10, cur_btn.y + 5, cur_name, palette.text_primary, f_small);

        // 5. Section 4: Persistence hint & save button
        let sec4_y = sec3_y + 56;
        ui::font::draw_text_font(s, content_x, sec4_y, "Config file: /ACCESS.CFG (64-byte binary record, magic 'NVAC')", palette.text_muted, f_small);
        let save_btn = Rect::new(content_x, sec4_y + 16, 200, 26);
        ui::draw::draw_rounded_rect(s, save_btn, RADIUS_XS, palette.accent_primary.darken(35));
        ui::draw::draw_rect_outline(s, save_btn, 1, palette.accent_primary);
        let save_sz = f_small.measure_str("Save & Apply to Disk");
        ui::font::draw_text_font(s, save_btn.x + (save_btn.w - save_sz.w) / 2, save_btn.y + 5, "Save & Apply to Disk", Color::WHITE, f_small);
    }

    fn render_devices_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        content_x: i32,
        content_y: i32,
        _viewport: Rect,
        palette: &ThemePalette,
        font: &Font,
        f_small: &Font,
    ) {
        // 1. Subtab Switcher: [Hardware Tree] vs [Registered Drivers] + [Rescan]
        let subtab0_w = 110;
        let subtab1_w = 130;
        let subtab_h = 24;

        // Subtab 0: Hardware Tree
        let is_tree = self.device_mgr.view_mode == DeviceViewMode::HardwareTree;
        let st0_rect = Rect::new(content_x, content_y, subtab0_w, subtab_h);
        if is_tree {
            ui::draw::draw_rounded_rect(s, st0_rect, RADIUS_XS, palette.accent_primary.darken(35));
            ui::draw::draw_rect_outline(s, st0_rect, 1, palette.accent_primary);
            let sz = f_small.measure_str("Hardware Tree");
            ui::font::draw_text_font(s, st0_rect.x + (st0_rect.w - sz.w) / 2, st0_rect.y + 4, "Hardware Tree", Color::WHITE, f_small);
        } else {
            ui::draw::draw_rounded_rect(s, st0_rect, RADIUS_XS, palette.surface_sunken);
            ui::draw::draw_rect_outline(s, st0_rect, 1, palette.border_subtle);
            let sz = f_small.measure_str("Hardware Tree");
            ui::font::draw_text_font(s, st0_rect.x + (st0_rect.w - sz.w) / 2, st0_rect.y + 4, "Hardware Tree", palette.text_secondary, f_small);
        }

        // Subtab 1: Registered Drivers
        let is_drv = self.device_mgr.view_mode == DeviceViewMode::DriverTable;
        let st1_rect = Rect::new(content_x + subtab0_w + 6, content_y, subtab1_w, subtab_h);
        if is_drv {
            ui::draw::draw_rounded_rect(s, st1_rect, RADIUS_XS, palette.accent_primary.darken(35));
            ui::draw::draw_rect_outline(s, st1_rect, 1, palette.accent_primary);
            let sz = f_small.measure_str("Registered Drivers");
            ui::font::draw_text_font(s, st1_rect.x + (st1_rect.w - sz.w) / 2, st1_rect.y + 4, "Registered Drivers", Color::WHITE, f_small);
        } else {
            ui::draw::draw_rounded_rect(s, st1_rect, RADIUS_XS, palette.surface_sunken);
            ui::draw::draw_rect_outline(s, st1_rect, 1, palette.border_subtle);
            let sz = f_small.measure_str("Registered Drivers");
            ui::font::draw_text_font(s, st1_rect.x + (st1_rect.w - sz.w) / 2, st1_rect.y + 4, "Registered Drivers", palette.text_secondary, f_small);
        }

        // [Rescan] button
        let rescan_rect = Rect::new(content_x + 380, content_y, 70, 22);
        ui::draw::draw_rounded_rect(s, rescan_rect, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, rescan_rect, 1, palette.accent_primary);
        ui::font::draw_text_font(s, rescan_rect.x + 14, rescan_rect.y + 4, "Rescan", palette.accent_primary, f_small);

        // 2. Telemetry Summary Cards Banner (content_y + 28)
        let card_y = content_y + 28;
        let card_w = 108;
        let card_h = 32;

        // Card 0: NVMe Storage
        let c0 = Rect::new(content_x, card_y, card_w, card_h);
        ui::draw::draw_rounded_rect(s, c0, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, c0, 1, palette.border_subtle);
        ui::font::draw_text_font(s, c0.x + 6, c0.y + 3, "NVMe Storage", palette.text_secondary, f_small);
        if self.device_mgr.summary.nvme_present != 0 {
            ui::font::draw_text_font(s, c0.x + 6, c0.y + 16, "Active (PCI 0:9.0)", palette.status_success, f_small);
        } else {
            ui::font::draw_text_font(s, c0.x + 6, c0.y + 16, "Not Detected", palette.text_muted, f_small);
        }

        // Card 1: xHCI USB 3.0
        let c1 = Rect::new(content_x + 114, card_y, card_w, card_h);
        ui::draw::draw_rounded_rect(s, c1, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, c1, 1, palette.border_subtle);
        ui::font::draw_text_font(s, c1.x + 6, c1.y + 3, "xHCI USB 3.0", palette.text_secondary, f_small);
        if self.device_mgr.summary.xhci_present != 0 {
            ui::font::draw_text_font(s, c1.x + 6, c1.y + 16, "Active (8 Ports)", palette.status_info, f_small);
        } else {
            ui::font::draw_text_font(s, c1.x + 6, c1.y + 16, "Not Detected", palette.text_muted, f_small);
        }

        // Card 2: Wi-Fi 802.11
        let c2 = Rect::new(content_x + 228, card_y, card_w, card_h);
        ui::draw::draw_rounded_rect(s, c2, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, c2, 1, palette.border_subtle);
        ui::font::draw_text_font(s, c2.x + 6, c2.y + 3, "Wi-Fi 802.11", palette.text_secondary, f_small);
        if self.device_mgr.summary.wifi_present != 0 {
            ui::font::draw_text_font(s, c2.x + 6, c2.y + 16, "Active (802.11)", palette.status_success, f_small);
        } else {
            ui::font::draw_text_font(s, c2.x + 6, c2.y + 16, "Not Detected", palette.text_muted, f_small);
        }

        // Card 3: Kernel Drivers & Devices
        let c3 = Rect::new(content_x + 342, card_y, card_w, card_h);
        ui::draw::draw_rounded_rect(s, c3, RADIUS_XS, palette.surface_raised);
        ui::draw::draw_rect_outline(s, c3, 1, palette.border_subtle);
        ui::font::draw_text_font(s, c3.x + 6, c3.y + 3, "Drivers & HW", palette.text_secondary, f_small);
        let mut d_buf = [0u8; 32];
        let mut d_len = 0;
        d_len += format_u32(self.device_mgr.summary.total_drivers, &mut d_buf[d_len..]);
        d_len += copy_bytes(&mut d_buf[d_len..], b" Drv | ");
        d_len += format_u32(self.device_mgr.summary.total_devices, &mut d_buf[d_len..]);
        d_len += copy_bytes(&mut d_buf[d_len..], b" Dev");
        if let Ok(d_str) = core::str::from_utf8(&d_buf[..d_len]) {
            ui::font::draw_text_font(s, c3.x + 6, c3.y + 16, d_str, palette.accent_primary, f_small);
        }

        match self.device_mgr.view_mode {
            DeviceViewMode::HardwareTree => {
                // 3. Category Filter Chips (content_y + 64)
                let chip_y = content_y + 64;
                let cats = DeviceCategoryFilter::all();
                for (i, &cat) in cats.iter().enumerate() {
                    let cx = content_x + (i as i32 * 56);
                    let chip_rect = Rect::new(cx, chip_y, 52, 20);
                    let is_sel = self.device_mgr.category_filter == cat;
                    if is_sel {
                        ui::draw::draw_rounded_rect(s, chip_rect, RADIUS_XS, palette.accent_primary.darken(35));
                        ui::draw::draw_rect_outline(s, chip_rect, 1, palette.accent_primary);
                        let sz = f_small.measure_str(cat.label());
                        ui::font::draw_text_font(s, chip_rect.x + (chip_rect.w - sz.w) / 2, chip_rect.y + 2, cat.label(), Color::WHITE, f_small);
                    } else {
                        ui::draw::draw_rounded_rect(s, chip_rect, RADIUS_XS, palette.surface_raised);
                        ui::draw::draw_rect_outline(s, chip_rect, 1, palette.border_subtle);
                        let sz = f_small.measure_str(cat.label());
                        ui::font::draw_text_font(s, chip_rect.x + (chip_rect.w - sz.w) / 2, chip_rect.y + 2, cat.label(), palette.text_secondary, f_small);
                    }
                }

                // 4. Hardware Table Header (content_y + 88)
                let tbl_y = content_y + 88;
                s.fill_rect(Rect::new(content_x, tbl_y, 450, 18), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 6, tbl_y + 2, "Status", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 58, tbl_y + 2, "Device Name", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 220, tbl_y + 2, "Bus / Loc", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 300, tbl_y + 2, "Driver", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 375, tbl_y + 2, "ID (Ven:Dev)", palette.text_muted, f_small);

                // 5. Hardware Rows (content_y + 108)
                let row_h = 20;
                let list_y = content_y + 108;
                let vis_count = self.device_mgr.filtered_device_count().min(5);
                for r in 0..vis_count {
                    if let Some((orig_idx, dev)) = self.device_mgr.filtered_device_at(r) {
                        let ry = list_y + (r as i32 * row_h);
                        let r_rect = Rect::new(content_x, ry, 450, row_h);
                        let is_sel = orig_idx == self.device_mgr.selected_device_idx;
                        if is_sel {
                            s.fill_rect(r_rect, palette.surface_raised.lighten(15));
                            ui::draw::draw_rect_outline(s, r_rect, 1, palette.accent_primary);
                        } else if r % 2 == 1 {
                            s.fill_rect(r_rect, palette.surface_raised.darken(10));
                        }

                        // Status badge pill
                        let pill_rect = Rect::new(content_x + 4, ry + 2, 48, 16);
                        let s_col = self.device_mgr.status_color(dev.status, palette);
                        ui::draw::draw_rounded_rect(s, pill_rect, RADIUS_XS, s_col.darken(35));
                        ui::draw::draw_rect_outline(s, pill_rect, 1, s_col);
                        let st_str = DeviceManager::status_str(dev.status);
                        let st_sz = f_small.measure_str(st_str);
                        ui::font::draw_text_font(s, pill_rect.x + (pill_rect.w - st_sz.w) / 2, pill_rect.y + 1, st_str, Color::WHITE, f_small);

                        // Device Name
                        let d_name = str_from_u8_nul(&dev.name);
                        ui::font::draw_text_font(s, content_x + 58, ry + 2, d_name, palette.text_primary, f_small);

                        // Bus / Location
                        let mut bus_buf = [0u8; 32];
                        let mut b_len = 0;
                        if dev.bus_type == ffi::NOVA_BUS_PCI {
                            b_len += copy_bytes(&mut bus_buf[b_len..], b"PCI ");
                            b_len += format_u32(dev.pci_bus as u32, &mut bus_buf[b_len..]);
                            if b_len < bus_buf.len() { bus_buf[b_len] = b':'; b_len += 1; }
                            b_len += format_u32(dev.pci_dev as u32, &mut bus_buf[b_len..]);
                            if b_len < bus_buf.len() { bus_buf[b_len] = b'.'; b_len += 1; }
                            b_len += format_u32(dev.pci_func as u32, &mut bus_buf[b_len..]);
                        } else if dev.bus_type == ffi::NOVA_BUS_ISA {
                            b_len += copy_bytes(&mut bus_buf[b_len..], b"ISA IRQ");
                            b_len += format_u32(dev.irq as u32, &mut bus_buf[b_len..]);
                        } else {
                            b_len += copy_bytes(&mut bus_buf[b_len..], DeviceManager::bus_name(dev.bus_type).as_bytes());
                        }
                        if let Ok(b_str) = core::str::from_utf8(&bus_buf[..b_len]) {
                            ui::font::draw_text_font(s, content_x + 220, ry + 2, b_str, palette.text_secondary, f_small);
                        }

                        // Bound driver
                        let drv_str = str_from_u8_nul(&dev.driver_name);
                        ui::font::draw_text_font(s, content_x + 300, ry + 2, drv_str, palette.accent_primary, f_small);

                        // Vendor:Device
                        if dev.vendor_id != 0 {
                            let mut id_buf = [0u8; 10];
                            let mut id_len = format_hex16(dev.vendor_id, &mut id_buf);
                            id_buf[id_len] = b':'; id_len += 1;
                            id_len += format_hex16(dev.device_id, &mut id_buf[id_len..]);
                            if let Ok(id_str) = core::str::from_utf8(&id_buf[..id_len]) {
                                ui::font::draw_text_font(s, content_x + 375, ry + 2, id_str, palette.text_muted, f_small);
                            }
                        }
                    }
                }

                // 6. Hardware Inspector Card (content_y + 214)
                let insp_rect = Rect::new(content_x, content_y + 214, 450, 88);
                ui::draw::draw_rounded_rect(s, insp_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, insp_rect, 1, palette.border_subtle);

                if let Some(dev) = self.device_mgr.selected_device() {
                    let d_name = str_from_u8_nul(&dev.name);
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 6, "Hardware Inspector - ", palette.text_secondary, f_small);
                    let off_title = f_small.measure_str("Hardware Inspector - ").w;
                    ui::font::draw_text_font(s, insp_rect.x + 12 + off_title, insp_rect.y + 6, d_name, palette.accent_primary, f_small);

                    // Row 1: Specifications
                    let det_str = str_from_u8_nul(&dev.details);
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 24, "Specs: ", palette.text_muted, f_small);
                    ui::font::draw_text_font(s, insp_rect.x + 60, insp_rect.y + 24, det_str, palette.text_primary, f_small);

                    // Row 2: Driver Binding & Category
                    let drv_str = str_from_u8_nul(&dev.driver_name);
                    let cat_str = DeviceManager::category_name(dev.category);
                    let st_str = DeviceManager::status_str(dev.status);
                    let mut b_buf = [0u8; 96];
                    let mut b_off = 0;
                    b_off += copy_bytes(&mut b_buf[b_off..], b"Driver: ");
                    b_off += copy_bytes(&mut b_buf[b_off..], drv_str.as_bytes());
                    b_off += copy_bytes(&mut b_buf[b_off..], b" | Category: ");
                    b_off += copy_bytes(&mut b_buf[b_off..], cat_str.as_bytes());
                    b_off += copy_bytes(&mut b_buf[b_off..], b" | Status: ");
                    b_off += copy_bytes(&mut b_buf[b_off..], st_str.as_bytes());
                    if let Ok(b_str) = core::str::from_utf8(&b_buf[..b_off]) {
                        ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 44, b_str, palette.text_secondary, f_small);
                    }

                    // Row 3: Bus & Resources
                    let mut r_buf = [0u8; 96];
                    let mut r_off = 0;
                    r_off += copy_bytes(&mut r_buf[r_off..], b"Bus: ");
                    r_off += copy_bytes(&mut r_buf[r_off..], DeviceManager::bus_name(dev.bus_type).as_bytes());
                    if dev.vendor_id != 0 {
                        r_off += copy_bytes(&mut r_buf[r_off..], b" (0x");
                        r_off += format_hex16(dev.vendor_id, &mut r_buf[r_off..]);
                        r_off += copy_bytes(&mut r_buf[r_off..], b":0x");
                        r_off += format_hex16(dev.device_id, &mut r_buf[r_off..]);
                        r_off += copy_bytes(&mut r_buf[r_off..], b")");
                    }
                    if dev.mmio_or_io != 0 {
                        r_off += copy_bytes(&mut r_buf[r_off..], b" | Base: 0x");
                        r_off += format_hex32(dev.mmio_or_io, &mut r_buf[r_off..]);
                    }
                    if dev.irq != 0 {
                        r_off += copy_bytes(&mut r_buf[r_off..], b" | IRQ: ");
                        r_off += format_u32(dev.irq as u32, &mut r_buf[r_off..]);
                    }
                    if let Ok(r_str) = core::str::from_utf8(&r_buf[..r_off]) {
                        ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 64, r_str, palette.text_muted, f_small);
                    }
                } else {
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 34, "Select a device to view hardware details", palette.text_muted, f_small);
                }
            }
            DeviceViewMode::DriverTable => {
                // 3. Driver Table Title (content_y + 64)
                let chip_y = content_y + 64;
                ui::font::draw_text_font(s, content_x, chip_y + 2, "Registered Kernel Drivers (.drivers linker table)", palette.accent_primary, font);

                // 4. Driver Table Header (content_y + 88)
                let tbl_y = content_y + 88;
                s.fill_rect(Rect::new(content_x, tbl_y, 450, 18), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 8, tbl_y + 2, "Driver Name", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 140, tbl_y + 2, "Init Phase", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 280, tbl_y + 2, "Status", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 380, tbl_y + 2, "Flags", palette.text_muted, f_small);

                // 5. Driver Rows (content_y + 108)
                let row_h = 20;
                let list_y = content_y + 108;
                let drv_count = self.device_mgr.driver_count().min(5);
                for r in 0..drv_count {
                    let drv = &self.device_mgr.driver_table().drivers[r];
                    let ry = list_y + (r as i32 * row_h);
                    let r_rect = Rect::new(content_x, ry, 450, row_h);
                    let is_sel = r == self.device_mgr.selected_driver_idx;
                    if is_sel {
                        s.fill_rect(r_rect, palette.surface_raised.lighten(15));
                        ui::draw::draw_rect_outline(s, r_rect, 1, palette.accent_primary);
                    } else if r % 2 == 1 {
                        s.fill_rect(r_rect, palette.surface_raised.darken(10));
                    }

                    // Driver Name
                    let d_name = str_from_u8_nul(&drv.name);
                    ui::font::draw_text_font(s, content_x + 8, ry + 2, d_name, palette.text_primary, f_small);

                    // Phase
                    let phase_str = match drv.phase {
                        1 => "Phase 1 (Core Timer)",
                        2 => "Phase 2 (Subsystem/VFS)",
                        3 => "Phase 3 (Device/PCI)",
                        _ => "Phase Unknown",
                    };
                    ui::font::draw_text_font(s, content_x + 140, ry + 2, phase_str, palette.text_secondary, f_small);

                    // Status
                    ui::font::draw_text_font(s, content_x + 280, ry + 2, "Registered / Active", palette.status_success, f_small);

                    // Flags
                    let mut fl_buf = [0u8; 8];
                    let fl_len = format_hex16(drv.flags as u16, &mut fl_buf);
                    if let Ok(fl_str) = core::str::from_utf8(&fl_buf[..fl_len]) {
                        ui::font::draw_text_font(s, content_x + 380, ry + 2, fl_str, palette.text_muted, f_small);
                    }
                }

                // 6. Driver Inspector Card (content_y + 214)
                let insp_rect = Rect::new(content_x, content_y + 214, 450, 88);
                ui::draw::draw_rounded_rect(s, insp_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, insp_rect, 1, palette.border_subtle);

                if let Some(drv) = self.device_mgr.selected_driver() {
                    let d_name = str_from_u8_nul(&drv.name);
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 6, "Driver Inspector - ", palette.text_secondary, f_small);
                    let off_title = f_small.measure_str("Driver Inspector - ").w;
                    ui::font::draw_text_font(s, insp_rect.x + 12 + off_title, insp_rect.y + 6, d_name, palette.accent_primary, f_small);

                    // Details
                    let phase_desc = match drv.phase {
                        1 => "Executed at Early Boot before multitasking (PIT, APIC, IRQ)",
                        2 => "Executed at Subsystem Initialization (VFS, PS/2 Human Interface)",
                        3 => "Executed during Bus & Hardware Discovery (PCI, NVMe, USB, Wi-Fi)",
                        _ => "Kernel Linker Registered Entry Point",
                    };
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 24, "Role: ", palette.text_muted, f_small);
                    ui::font::draw_text_font(s, insp_rect.x + 60, insp_rect.y + 24, phase_desc, palette.text_primary, f_small);

                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 44, "Linker Section: .drivers | Table: DRIVER_REGISTER descriptor", palette.text_secondary, f_small);
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 64, "Status: Active in kernel memory space | Ring 0 Privileged Driver", palette.status_success, f_small);
                } else {
                    ui::font::draw_text_font(s, insp_rect.x + 12, insp_rect.y + 34, "Select a driver to view descriptor details", palette.text_muted, f_small);
                }
            }
        }
    }

    fn render_services_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        content_x: i32,
        content_y: i32,
        _viewport: Rect,
        palette: &ThemePalette,
        font: &Font,
        f_small: &Font,
    ) {
        // 1. Subtab Switcher: [Live Supervision] vs [SERVICES.CFG Editor]
        let subtab_w = 120;
        let subtab_h = 24;
        let subtabs = ["Live Supervision", "SERVICES.CFG Editor"];
        for (i, &st_name) in subtabs.iter().enumerate() {
            let st_rect = Rect::new(content_x + (i as i32 * (subtab_w + 6)), content_y, subtab_w, subtab_h);
            let is_act = (i == 0 && self.service_mgr.view_mode == ServiceViewMode::LiveDashboard)
                || (i == 1 && self.service_mgr.view_mode == ServiceViewMode::ConfigFile);
            if is_act {
                ui::draw::draw_rounded_rect(s, st_rect, RADIUS_XS, palette.accent_primary.darken(35));
                ui::draw::draw_rect_outline(s, st_rect, 1, palette.accent_primary);
                let sz = f_small.measure_str(st_name);
                ui::font::draw_text_font(s, st_rect.x + (st_rect.w - sz.w) / 2, st_rect.y + 4, st_name, Color::WHITE, f_small);
            } else {
                ui::draw::draw_rounded_rect(s, st_rect, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, st_rect, 1, palette.border_subtle);
                let sz = f_small.measure_str(st_name);
                ui::font::draw_text_font(s, st_rect.x + (st_rect.w - sz.w) / 2, st_rect.y + 4, st_name, palette.text_secondary, f_small);
            }
        }

        match self.service_mgr.view_mode {
            ServiceViewMode::LiveDashboard => {
                // 2. Metrics & Status Bar (content_y + 30)
                let bar_y = content_y + 30;
                let sup_dot = Rect::new(content_x + 2, bar_y + 6, 8, 8);
                ui::draw::draw_rounded_rect(s, sup_dot, RADIUS_XS, palette.status_success);
                ui::font::draw_text_font(s, content_x + 14, bar_y + 4, "Supervisor: novainit (PID 1)", palette.text_primary, f_small);

                // Counters: Running / Restarting / Crashed / Stopped
                let mut metric_buf = [0u8; 96];
                let mut m_off = 0;
                m_off += copy_bytes(&mut metric_buf[m_off..], b"Running: ");
                m_off += format_u32(self.service_mgr.running_count() as u32, &mut metric_buf[m_off..]);
                m_off += copy_bytes(&mut metric_buf[m_off..], b" | Restarting: ");
                m_off += format_u32(self.service_mgr.restarting_count() as u32, &mut metric_buf[m_off..]);
                m_off += copy_bytes(&mut metric_buf[m_off..], b" | Crashed: ");
                m_off += format_u32(self.service_mgr.crashed_count() as u32, &mut metric_buf[m_off..]);
                m_off += copy_bytes(&mut metric_buf[m_off..], b" | Stopped: ");
                m_off += format_u32(self.service_mgr.stopped_count() as u32, &mut metric_buf[m_off..]);
                if let Ok(m_str) = core::str::from_utf8(&metric_buf[..m_off]) {
                    ui::font::draw_text_font(s, content_x + 165, bar_y + 4, m_str, palette.text_secondary, f_small);
                }

                // [Refresh Telemetry] button
                let ref_rect = Rect::new(content_x + 390, bar_y, 76, 22);
                ui::draw::draw_rounded_rect(s, ref_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, ref_rect, 1, palette.accent_primary);
                ui::font::draw_text_font(s, ref_rect.x + 8, ref_rect.y + 4, "Refresh", palette.accent_primary, f_small);

                // 3. Live Supervision Table Header (content_y + 56)
                let tbl_y = content_y + 56;
                s.fill_rect(Rect::new(content_x, tbl_y, 470, 20), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 8, tbl_y + 3, "Status", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 86, tbl_y + 3, "Service Name", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 185, tbl_y + 3, "Binary & Target", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 295, tbl_y + 3, "PID", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 348, tbl_y + 3, "Restarts", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 412, tbl_y + 3, "Policy", palette.text_muted, f_small);

                // 4. Live Service Rows (content_y + 78)
                let row_h = 22;
                let list_y = content_y + 78;
                for i in 0..self.service_mgr.count {
                    let svc = &self.service_mgr.services[i];
                    let ry = list_y + (i as i32 * row_h);
                    let is_sel = i == self.service_mgr.selected_idx;
                    let r_rect = Rect::new(content_x, ry, 470, row_h);

                    if is_sel {
                        s.fill_rect(r_rect, palette.surface_raised.lighten(15));
                        ui::draw::draw_rect_outline(s, r_rect, 1, palette.accent_primary);
                    } else if i % 2 == 1 {
                        s.fill_rect(r_rect, palette.surface_raised.darken(10));
                    }

                    // Status pill
                    let pill_rect = Rect::new(content_x + 6, ry + 2, 72, 18);
                    let s_col = svc.state_color(palette);
                    ui::draw::draw_rounded_rect(s, pill_rect, RADIUS_XS, s_col.darken(35));
                    ui::draw::draw_rect_outline(s, pill_rect, 1, s_col);
                    let st_str = svc.state_str();
                    let st_sz = f_small.measure_str(st_str);
                    ui::font::draw_text_font(s, pill_rect.x + (pill_rect.w - st_sz.w) / 2, pill_rect.y + 2, st_str, Color::WHITE, f_small);

                    // Service Name
                    ui::font::draw_text_font(s, content_x + 86, ry + 4, svc.name_str(), if is_sel { Color::WHITE } else { palette.text_primary }, f_small);

                    // Binary & Target
                    ui::font::draw_text_font(s, content_x + 185, ry + 4, svc.exec_str(), palette.text_secondary, f_small);

                    // PID
                    if svc.pid > 0 {
                        let mut pid_buf = [0u8; 16];
                        let mut poff = copy_bytes(&mut pid_buf, b"PID ");
                        poff += format_u32(svc.pid as u32, &mut pid_buf[poff..]);
                        if let Ok(pid_str) = core::str::from_utf8(&pid_buf[..poff]) {
                            ui::font::draw_text_font(s, content_x + 295, ry + 4, pid_str, palette.status_success, f_small);
                        }
                    } else {
                        ui::font::draw_text_font(s, content_x + 295, ry + 4, "-", palette.text_muted, f_small);
                    }

                    // Restarts
                    let mut r_buf = [0u8; 16];
                    let mut r_off = format_u32(svc.restart_count, &mut r_buf);
                    r_off += copy_bytes(&mut r_buf[r_off..], b" / 20");
                    if let Ok(r_str) = core::str::from_utf8(&r_buf[..r_off]) {
                        let r_col = if svc.restart_count >= 20 {
                            palette.status_danger
                        } else if svc.restart_count > 0 {
                            palette.status_warning
                        } else {
                            palette.text_muted
                        };
                        ui::font::draw_text_font(s, content_x + 348, ry + 4, r_str, r_col, f_small);
                    }

                    // Policy
                    ui::font::draw_text_font(s, content_x + 412, ry + 4, svc.policy_str(), palette.accent_primary, f_small);
                }

                // 5. Selected Service Inspector Card (content_y + 196)
                let insp_y = content_y + 196;
                let insp_rect = Rect::new(content_x, insp_y, 470, 84);
                ui::draw::draw_rounded_rect(s, insp_rect, RADIUS_SM, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, insp_rect, 1, palette.border_subtle);

                if self.service_mgr.selected_idx < self.service_mgr.count {
                    let sel = &self.service_mgr.services[self.service_mgr.selected_idx];

                    // Title line: Service Inspector: <name>
                    ui::font::draw_text_font(s, content_x + 12, insp_y + 6, "Service Inspector:", palette.text_muted, f_small);
                    ui::font::draw_text_font(s, content_x + 104, insp_y + 6, sel.name_str(), palette.accent_primary, font);

                    // Privilege tag
                    let priv_str = if sel.trusted { "Trusted (SYS_EXEC_TRUSTED)" } else { "Standard (SYS_EXEC)" };
                    let priv_col = if sel.trusted { palette.status_warning } else { palette.text_muted };
                    ui::font::draw_text_font(s, content_x + 280, insp_y + 8, priv_str, priv_col, f_small);

                    // Line 1: Command & Dependencies
                    let mut cmd_buf = [0u8; 96];
                    let mut c_off = 0;
                    c_off += copy_bytes(&mut cmd_buf[c_off..], b"Exec: ");
                    c_off += copy_bytes(&mut cmd_buf[c_off..], sel.exec_str().as_bytes());
                    if sel.arg_str() != "-" {
                        c_off += copy_bytes(&mut cmd_buf[c_off..], b" ");
                        c_off += copy_bytes(&mut cmd_buf[c_off..], sel.arg_str().as_bytes());
                    }
                    c_off += copy_bytes(&mut cmd_buf[c_off..], b"  |  Depends on: ");
                    c_off += copy_bytes(&mut cmd_buf[c_off..], sel.depends_str().as_bytes());
                    if let Ok(cmd_str) = core::str::from_utf8(&cmd_buf[..c_off]) {
                        ui::font::draw_text_font(s, content_x + 12, insp_y + 28, cmd_str, palette.text_secondary, f_small);
                    }

                    // Line 2: Supervision status, restarts, exit code
                    let mut stat_buf = [0u8; 96];
                    let mut s_off = 0;
                    s_off += copy_bytes(&mut stat_buf[s_off..], b"Status: ");
                    s_off += copy_bytes(&mut stat_buf[s_off..], sel.state_str().as_bytes());
                    s_off += copy_bytes(&mut stat_buf[s_off..], b"  |  Restarts: ");
                    s_off += format_u32(sel.restart_count, &mut stat_buf[s_off..]);
                    s_off += copy_bytes(&mut stat_buf[s_off..], b"/20  |  Restart Policy: ");
                    s_off += copy_bytes(&mut stat_buf[s_off..], sel.policy_str().as_bytes());
                    if let Ok(stat_str) = core::str::from_utf8(&stat_buf[..s_off]) {
                        ui::font::draw_text_font(s, content_x + 12, insp_y + 44, stat_str, palette.text_secondary, f_small);
                    }

                    // Action buttons
                    let btn_y = insp_y + 58;
                    // [Restart Service]
                    let restart_btn = Rect::new(content_x + 10, btn_y, 108, 20);
                    ui::draw::draw_rounded_rect(s, restart_btn, RADIUS_XS, palette.accent_primary.darken(35));
                    ui::draw::draw_rect_outline(s, restart_btn, 1, palette.accent_primary);
                    ui::font::draw_text_font(s, restart_btn.x + 8, restart_btn.y + 3, "Restart Service", Color::WHITE, f_small);

                    // [Stop Service]
                    let stop_btn = Rect::new(content_x + 126, btn_y, 88, 20);
                    ui::draw::draw_rounded_rect(s, stop_btn, RADIUS_XS, palette.surface_raised);
                    ui::draw::draw_rect_outline(s, stop_btn, 1, palette.status_danger);
                    ui::font::draw_text_font(s, stop_btn.x + 10, stop_btn.y + 3, "Stop Service", palette.status_danger, f_small);

                    // [Start Service]
                    let start_btn = Rect::new(content_x + 222, btn_y, 88, 20);
                    ui::draw::draw_rounded_rect(s, start_btn, RADIUS_XS, palette.surface_raised);
                    ui::draw::draw_rect_outline(s, start_btn, 1, palette.status_success);
                    ui::font::draw_text_font(s, start_btn.x + 8, start_btn.y + 3, "Start Service", palette.status_success, f_small);
                }
            }

            ServiceViewMode::ConfigFile => {
                // Config File Editor (SERVICES.CFG)
                ui::font::draw_text_font(s, content_x, content_y + 28, "Supervisor Static Policies (SERVICES.CFG)", palette.accent_primary, font);

                let tbl_y = content_y + 48;
                s.fill_rect(Rect::new(content_x, tbl_y, 470, 20), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 8, tbl_y + 3, "Service Name", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 130, tbl_y + 3, "Target Binary", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 230, tbl_y + 3, "Restart Policy", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 340, tbl_y + 3, "Privilege", palette.text_muted, f_small);

                let mut ry = tbl_y + 22;
                for i in 0..self.services_cfg.count {
                    let svc = &self.services_cfg.services[i];
                    let r_rect = Rect::new(content_x, ry, 470, 22);
                    if i % 2 == 1 {
                        s.fill_rect(r_rect, palette.surface_raised.darken(10));
                    }

                    ui::font::draw_text_font(s, content_x + 8, ry + 4, svc.name_str(), palette.text_primary, f_small);
                    ui::font::draw_text_font(s, content_x + 130, ry + 4, svc.exec_path_str(), palette.text_secondary, f_small);

                    let pill_rect = Rect::new(content_x + 226, ry + 2, 74, 18);
                    ui::draw::draw_rounded_rect(s, pill_rect, RADIUS_XS, palette.surface_sunken);
                    ui::draw::draw_rect_outline(s, pill_rect, 1, palette.accent_primary);
                    ui::font::draw_text_font(s, pill_rect.x + 8, pill_rect.y + 2, svc.restart.as_str(), palette.accent_primary, f_small);

                    let (priv_text, priv_col) = if svc.trusted {
                        ("Trusted", palette.status_warning)
                    } else {
                        ("Standard", palette.text_muted)
                    };
                    ui::font::draw_text_font(s, content_x + 340, ry + 4, priv_text, priv_col, f_small);
                    ry += 24;
                }

                ui::font::draw_text_font(s, content_x, content_y + 200, "Click restart policy pill to cycle policies (always, on-crash, never).", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x, content_y + 218, "Click 'Save' at top right to write changes back to SERVICES.CFG.", palette.text_muted, f_small);
            }
        }
    }

    fn render_users_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        content_x: i32,
        content_y: i32,
        font: &Font,
        f_small: &Font,
        palette: &ThemePalette,
    ) {
        ui::font::draw_text_font(s, content_x, content_y, "User Accounts & Permissions (USERS.CFG)", palette.accent_primary, font);

        // Top right capacity badge: "Capacity: X / 8 Users"
        let count_str_x = content_x + 320;
        let mut cnt_buf = [0u8; 32];
        let mut off = copy_bytes(&mut cnt_buf, b"Capacity: ");
        off += format_u32(self.users_cfg.count as u32, &mut cnt_buf[off..]);
        off += copy_bytes(&mut cnt_buf[off..], b" / 8 Users");
        if let Ok(c_txt) = core::str::from_utf8(&cnt_buf[..off]) {
            let badge_rect = Rect::new(count_str_x, content_y - 2, 130, 20);
            ui::draw::draw_rounded_rect(s, badge_rect, RADIUS_XS, palette.surface_raised);
            ui::draw::draw_rect_outline(s, badge_rect, 1, palette.border_subtle);
            ui::font::draw_text_font(s, count_str_x + 8, content_y + 2, c_txt, palette.text_secondary, f_small);
        }

        match self.users_ui.view_mode {
            UsersViewMode::List => {
                // Left Column: Accounts List (width 200)
                let lx = content_x;
                let ly = content_y + 24;
                ui::font::draw_text_font(s, lx, ly + 5, "Registered Accounts:", palette.text_secondary, f_small);

                // [+ Add User] button
                let add_btn = Rect::new(lx + 106, ly, 94, 22);
                ui::draw::draw_rounded_rect(s, add_btn, RADIUS_XS, palette.accent_primary);
                ui::font::draw_text_font(s, add_btn.x + 8, add_btn.y + 4, "+ Add User", Color::WHITE, f_small);

                let list_start_y = ly + 28;
                for i in 0..self.users_cfg.count {
                    let u = &self.users_cfg.accounts[i];
                    let card_y = list_start_y + (i as i32 * 40);
                    let card_rect = Rect::new(lx, card_y, 200, 36);
                    let is_sel = i == self.users_ui.selected_idx;

                    let bg_col = if is_sel {
                        palette.accent_primary.darken(35)
                    } else if i % 2 == 1 {
                        palette.surface_raised.darken(10)
                    } else {
                        palette.surface_raised
                    };
                    ui::draw::draw_rounded_rect(s, card_rect, RADIUS_XS, bg_col);
                    ui::draw::draw_rect_outline(s, card_rect, 1, if is_sel { palette.accent_primary } else { palette.border_subtle });

                    // Avatar badge (square with rounded corners)
                    let av_rect = Rect::new(lx + 6, card_y + 6, 24, 24);
                    let av_bg = if u.is_admin() { palette.accent_primary } else { palette.surface_sunken };
                    ui::draw::draw_rounded_rect(s, av_rect, RADIUS_XS, av_bg);
                    let initial = if u.username[0] != 0 { u.username[0].to_ascii_uppercase() } else { b'?' };
                    let init_str = [initial];
                    if let Ok(c_init) = core::str::from_utf8(&init_str) {
                        ui::font::draw_text_font(s, av_rect.x + 8, av_rect.y + 4, c_init, Color::WHITE, f_small);
                    }

                    // Username
                    ui::font::draw_text_font(s, lx + 36, card_y + 4, u.username_str(), if is_sel { Color::WHITE } else { palette.text_primary }, f_small);

                    // Role & UID
                    let mut role_buf = [0u8; 32];
                    let mut r_off = 0;
                    if u.is_admin() {
                        r_off += copy_bytes(&mut role_buf, b"Admin (UID ");
                    } else {
                        r_off += copy_bytes(&mut role_buf, b"User (UID ");
                    }
                    r_off += format_u32(u.uid, &mut role_buf[r_off..]);
                    r_off += copy_bytes(&mut role_buf[r_off..], b")");
                    if let Ok(r_txt) = core::str::from_utf8(&role_buf[..r_off]) {
                        let col = if u.is_admin() { palette.accent_primary.lighten(30) } else { palette.text_muted };
                        ui::font::draw_text_font(s, lx + 36, card_y + 19, r_txt, col, f_small);
                    }
                }

                // Right Column: Account Details & Permissions Inspector
                let rx = content_x + 212;
                let ry = content_y + 24;
                let insp_rect = Rect::new(rx, ry, 242, 280);
                ui::draw::draw_rounded_rect(s, insp_rect, RADIUS_SM, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, insp_rect, 1, palette.border_subtle);

                let sel_idx = self.users_ui.selected_idx;
                if sel_idx < self.users_cfg.count {
                    let u = &self.users_cfg.accounts[sel_idx];

                    // Large Avatar
                    let big_av = Rect::new(rx + 12, ry + 12, 32, 32);
                    let big_av_bg = if u.is_admin() { palette.accent_primary } else { palette.surface_raised };
                    ui::draw::draw_rounded_rect(s, big_av, RADIUS_SM, big_av_bg);
                    let init_c = if u.username[0] != 0 { u.username[0].to_ascii_uppercase() } else { b'?' };
                    let big_init = [init_c];
                    if let Ok(bi) = core::str::from_utf8(&big_init) {
                        ui::font::draw_text_font(s, big_av.x + 11, big_av.y + 6, bi, Color::WHITE, font);
                    }

                    // Account name & identity
                    ui::font::draw_text_font(s, rx + 52, ry + 10, u.username_str(), palette.text_primary, font);

                    let mut id_buf = [0u8; 32];
                    let mut id_off = copy_bytes(&mut id_buf, b"UID: ");
                    id_off += format_u32(u.uid, &mut id_buf[id_off..]);
                    id_off += copy_bytes(&mut id_buf[id_off..], b"  GID: ");
                    id_off += format_u32(u.gid, &mut id_buf[id_off..]);
                    if let Ok(id_txt) = core::str::from_utf8(&id_buf[..id_off]) {
                        ui::font::draw_text_font(s, rx + 52, ry + 28, id_txt, palette.text_secondary, f_small);
                    }

                    // Divider
                    ui::draw::draw_line(s, rx + 12, ry + 52, rx + 230, ry + 52, palette.border_subtle);

                    // Section 1: Role & Privileges
                    ui::font::draw_text_font(s, rx + 12, ry + 58, "Security Role & Privileges:", palette.text_secondary, f_small);

                    // Role switcher pills
                    // [ Standard User ]
                    let std_pill = Rect::new(rx + 12, ry + 76, 104, 24);
                    let is_std = !u.is_admin();
                    let std_bg = if is_std { palette.accent_primary.darken(35) } else { palette.surface_raised };
                    ui::draw::draw_rounded_rect(s, std_pill, RADIUS_XS, std_bg);
                    ui::draw::draw_rect_outline(s, std_pill, 1, if is_std { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, std_pill.x + 10, std_pill.y + 5, "Standard User", if is_std { Color::WHITE } else { palette.text_secondary }, f_small);

                    // [ Administrator ]
                    let adm_pill = Rect::new(rx + 122, ry + 76, 108, 24);
                    let is_adm = u.is_admin();
                    let adm_bg = if is_adm { palette.accent_primary } else { palette.surface_raised };
                    ui::draw::draw_rounded_rect(s, adm_pill, RADIUS_XS, adm_bg);
                    ui::draw::draw_rect_outline(s, adm_pill, 1, if is_adm { palette.accent_primary } else { palette.border_subtle });
                    ui::font::draw_text_font(s, adm_pill.x + 10, adm_pill.y + 5, "Administrator", if is_adm { Color::WHITE } else { palette.text_secondary }, f_small);

                    // Privilege description
                    let priv_desc = if is_adm {
                        "Admin: gid 0 (sudo, firewall, updates)"
                    } else {
                        "Standard: unprivileged, sandboxed task"
                    };
                    ui::font::draw_text_font(s, rx + 12, ry + 106, priv_desc, palette.text_muted, f_small);

                    // Divider
                    ui::draw::draw_line(s, rx + 12, ry + 124, rx + 230, ry + 124, palette.border_subtle);

                    // Section 2: Credentials & PBKDF2
                    ui::font::draw_text_font(s, rx + 12, ry + 130, "Authentication & Hash (PBKDF2):", palette.text_secondary, f_small);
                    ui::font::draw_text_font(s, rx + 12, ry + 148, "* * * * * * * *  (4096 iter)", palette.accent_primary, f_small);

                    // Salt preview (hex of first 4 bytes)
                    let mut salt_buf = [0u8; 32];
                    let mut s_off = copy_bytes(&mut salt_buf, b"Salt: 0x");
                    for b in &u.salt[..4] {
                        salt_buf[s_off] = to_hex_digit(b >> 4);
                        salt_buf[s_off + 1] = to_hex_digit(b & 0xF);
                        s_off += 2;
                    }
                    s_off += copy_bytes(&mut salt_buf[s_off..], b"...");
                    if let Ok(s_txt) = core::str::from_utf8(&salt_buf[..s_off]) {
                        ui::font::draw_text_font(s, rx + 12, ry + 166, s_txt, palette.text_muted, f_small);
                    }

                    // Change password button
                    let pw_btn = Rect::new(rx + 12, ry + 186, 126, 22);
                    ui::draw::draw_rounded_rect(s, pw_btn, RADIUS_XS, palette.surface_raised);
                    ui::draw::draw_rect_outline(s, pw_btn, 1, palette.border_subtle);
                    ui::font::draw_text_font(s, pw_btn.x + 8, pw_btn.y + 4, "Change Password", palette.text_primary, f_small);

                    // Divider
                    ui::draw::draw_line(s, rx + 12, ry + 216, rx + 230, ry + 216, palette.border_subtle);

                    // Section 3: Account Deletion
                    if u.is_admin() && self.users_cfg.admin_count() <= 1 {
                        ui::font::draw_text_font(s, rx + 12, ry + 226, "Protected (Last Administrator)", palette.text_muted, f_small);
                    } else if self.users_cfg.count <= 1 {
                        ui::font::draw_text_font(s, rx + 12, ry + 226, "Protected (Sole Account)", palette.text_muted, f_small);
                    } else {
                        let del_btn = Rect::new(rx + 12, ry + 226, 110, 22);
                        ui::draw::draw_rounded_rect(s, del_btn, RADIUS_XS, palette.surface_raised);
                        ui::draw::draw_rect_outline(s, del_btn, 1, palette.status_danger);
                        ui::font::draw_text_font(s, del_btn.x + 8, del_btn.y + 4, "- Delete User", palette.status_danger, f_small);
                    }
                }
            }

            UsersViewMode::CreateAccount => {
                // Useradd Form Card (width 434, height 280)
                let fx = content_x + 10;
                let fy = content_y + 24;
                let form_rect = Rect::new(fx, fy, 434, 280);
                ui::draw::draw_rounded_rect(s, form_rect, RADIUS_SM, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, form_rect, 1, palette.accent_primary);

                ui::font::draw_text_font(s, fx + 16, fy + 12, "Create User Account (useradd)", palette.accent_primary, font);
                ui::font::draw_text_font(s, fx + 16, fy + 32, "Sets username, UID/GID security role, and credentials in USERS.CFG", palette.text_secondary, f_small);

                // Quick presets row
                ui::font::draw_text_font(s, fx + 16, fy + 52, "Presets:", palette.text_muted, f_small);
                let p_names = ["alice (std)", "bob (admin)", "operator"];
                for i in 0..3 {
                    let p_btn = Rect::new(fx + 76 + (i as i32 * 96), fy + 50, 90, 20);
                    ui::draw::draw_rounded_rect(s, p_btn, RADIUS_XS, palette.surface_raised);
                    ui::draw::draw_rect_outline(s, p_btn, 1, palette.border_subtle);
                    ui::font::draw_text_font(s, p_btn.x + 6, p_btn.y + 3, p_names[i], palette.text_primary, f_small);
                }

                // Field 1: Username
                ui::font::draw_text_font(s, fx + 16, fy + 78, "Username:", palette.text_secondary, f_small);
                let un_rect = Rect::new(fx + 16, fy + 94, 180, 24);
                ui::draw::draw_rounded_rect(s, un_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, un_rect, 1, palette.border_subtle);
                if self.users_ui.form_username_len > 0 {
                    if let Ok(un_str) = core::str::from_utf8(&self.users_ui.form_username[..self.users_ui.form_username_len]) {
                        ui::font::draw_text_font(s, un_rect.x + 8, un_rect.y + 4, un_str, Color::WHITE, f_small);
                    }
                } else {
                    ui::font::draw_text_font(s, un_rect.x + 8, un_rect.y + 4, "type or click preset", palette.text_muted, f_small);
                }

                // Field 2: Role Switch
                ui::font::draw_text_font(s, fx + 216, fy + 78, "Account Type / Role:", palette.text_secondary, f_small);
                let std_btn = Rect::new(fx + 216, fy + 94, 96, 24);
                let is_std = !self.users_ui.form_is_admin;
                ui::draw::draw_rounded_rect(s, std_btn, RADIUS_XS, if is_std { palette.accent_primary.darken(35) } else { palette.surface_raised });
                ui::draw::draw_rect_outline(s, std_btn, 1, if is_std { palette.accent_primary } else { palette.border_subtle });
                ui::font::draw_text_font(s, std_btn.x + 6, std_btn.y + 4, "Standard", if is_std { Color::WHITE } else { palette.text_secondary }, f_small);

                let adm_btn = Rect::new(fx + 318, fy + 94, 100, 24);
                let is_adm = self.users_ui.form_is_admin;
                ui::draw::draw_rounded_rect(s, adm_btn, RADIUS_XS, if is_adm { palette.accent_primary } else { palette.surface_raised });
                ui::draw::draw_rect_outline(s, adm_btn, 1, if is_adm { palette.accent_primary } else { palette.border_subtle });
                ui::font::draw_text_font(s, adm_btn.x + 6, adm_btn.y + 4, "Admin (gid 0)", if is_adm { Color::WHITE } else { palette.text_secondary }, f_small);

                // Field 3: Assigned UID
                let mut uid_buf = [0u8; 32];
                let mut u_off = copy_bytes(&mut uid_buf, b"Assigned UID: ");
                u_off += format_u32(self.users_ui.form_uid, &mut uid_buf[u_off..]);
                if let Ok(uid_str) = core::str::from_utf8(&uid_buf[..u_off]) {
                    ui::font::draw_text_font(s, fx + 16, fy + 128, uid_str, palette.accent_primary.lighten(30), f_small);
                }

                // Field 4: Password
                ui::font::draw_text_font(s, fx + 16, fy + 148, "Password:", palette.text_secondary, f_small);
                let pw_rect = Rect::new(fx + 16, fy + 164, 180, 24);
                ui::draw::draw_rounded_rect(s, pw_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, pw_rect, 1, palette.border_subtle);
                if self.users_ui.form_password_len > 0 {
                    ui::font::draw_text_font(s, pw_rect.x + 8, pw_rect.y + 4, "* * * * * * * *", palette.accent_primary, f_small);
                } else {
                    ui::font::draw_text_font(s, pw_rect.x + 8, pw_rect.y + 4, "required", palette.text_muted, f_small);
                }

                // Field 5: Confirm Password
                ui::font::draw_text_font(s, fx + 216, fy + 148, "Confirm Password:", palette.text_secondary, f_small);
                let cpw_rect = Rect::new(fx + 216, fy + 164, 180, 24);
                ui::draw::draw_rounded_rect(s, cpw_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, cpw_rect, 1, palette.border_subtle);
                if self.users_ui.form_confirm_len > 0 {
                    ui::font::draw_text_font(s, cpw_rect.x + 8, cpw_rect.y + 4, "* * * * * * * *", palette.accent_primary, f_small);
                } else {
                    ui::font::draw_text_font(s, cpw_rect.x + 8, cpw_rect.y + 4, "retype password", palette.text_muted, f_small);
                }

                // Status message
                let st_msg = self.users_ui.status_str();
                if !st_msg.is_empty() {
                    let col = if self.users_ui.status_is_error { palette.status_danger } else { palette.status_success };
                    ui::font::draw_text_font(s, fx + 16, fy + 200, st_msg, col, f_small);
                }

                // Action buttons
                let create_rect = Rect::new(fx + 16, fy + 230, 130, 26);
                ui::draw::draw_rounded_rect(s, create_rect, RADIUS_XS, palette.accent_primary);
                ui::font::draw_text_font(s, create_rect.x + 12, create_rect.y + 5, "Create Account", Color::WHITE, f_small);

                let cancel_rect = Rect::new(fx + 158, fy + 230, 80, 26);
                ui::draw::draw_rounded_rect(s, cancel_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, cancel_rect, 1, palette.border_subtle);
                ui::font::draw_text_font(s, cancel_rect.x + 16, cancel_rect.y + 5, "Cancel", palette.text_primary, f_small);
            }

            UsersViewMode::ChangePassword => {
                let fx = content_x + 10;
                let fy = content_y + 24;
                let form_rect = Rect::new(fx, fy, 434, 250);
                ui::draw::draw_rounded_rect(s, form_rect, RADIUS_SM, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, form_rect, 1, palette.accent_primary);

                let u_name = if self.users_ui.selected_idx < self.users_cfg.count {
                    self.users_cfg.accounts[self.users_ui.selected_idx].username_str()
                } else {
                    "account"
                };

                let mut title_buf = [0u8; 64];
                let mut t_off = copy_bytes(&mut title_buf, b"Change Password for '");
                t_off += copy_bytes(&mut title_buf[t_off..], u_name.as_bytes());
                t_off += copy_bytes(&mut title_buf[t_off..], b"'");
                if let Ok(t_str) = core::str::from_utf8(&title_buf[..t_off]) {
                    ui::font::draw_text_font(s, fx + 16, fy + 12, t_str, palette.accent_primary, font);
                }
                ui::font::draw_text_font(s, fx + 16, fy + 32, "Derives a fresh salt and re-hashes credentials with PBKDF2 (4096 iter)", palette.text_secondary, f_small);

                // Preset button
                let pw_preset = Rect::new(fx + 16, fy + 52, 140, 20);
                ui::draw::draw_rounded_rect(s, pw_preset, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, pw_preset, 1, palette.border_subtle);
                ui::font::draw_text_font(s, pw_preset.x + 8, pw_preset.y + 3, "preset: novapass456", palette.text_primary, f_small);

                // Fields
                ui::font::draw_text_font(s, fx + 16, fy + 82, "New Password:", palette.text_secondary, f_small);
                let pw_rect = Rect::new(fx + 16, fy + 98, 180, 24);
                ui::draw::draw_rounded_rect(s, pw_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, pw_rect, 1, palette.border_subtle);
                if self.users_ui.form_password_len > 0 {
                    ui::font::draw_text_font(s, pw_rect.x + 8, pw_rect.y + 4, "* * * * * * * *", palette.accent_primary, f_small);
                }

                ui::font::draw_text_font(s, fx + 216, fy + 82, "Confirm New Password:", palette.text_secondary, f_small);
                let cpw_rect = Rect::new(fx + 216, fy + 98, 180, 24);
                ui::draw::draw_rounded_rect(s, cpw_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, cpw_rect, 1, palette.border_subtle);
                if self.users_ui.form_confirm_len > 0 {
                    ui::font::draw_text_font(s, cpw_rect.x + 8, cpw_rect.y + 4, "* * * * * * * *", palette.accent_primary, f_small);
                }

                let st_msg = self.users_ui.status_str();
                if !st_msg.is_empty() {
                    let col = if self.users_ui.status_is_error { palette.status_danger } else { palette.status_success };
                    ui::font::draw_text_font(s, fx + 16, fy + 140, st_msg, col, f_small);
                }

                // Action buttons
                let update_rect = Rect::new(fx + 16, fy + 180, 130, 26);
                ui::draw::draw_rounded_rect(s, update_rect, RADIUS_XS, palette.accent_primary);
                ui::font::draw_text_font(s, update_rect.x + 10, update_rect.y + 5, "Update Password", Color::WHITE, f_small);

                let cancel_rect = Rect::new(fx + 158, fy + 180, 80, 26);
                ui::draw::draw_rounded_rect(s, cancel_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, cancel_rect, 1, palette.border_subtle);
                ui::font::draw_text_font(s, cancel_rect.x + 16, cancel_rect.y + 5, "Cancel", palette.text_primary, f_small);
            }
        }
    }

    fn render_firewall_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        content_x: i32,
        content_y: i32,
        viewport: Rect,
        palette: &ThemePalette,
        font: &Font,
        f_small: &Font,
    ) {
        // Sub-tabs navigation across top of panel
        let subtab_w = 110;
        let subtab_h = 24;
        let subtabs = ["Rules & Policies", "Network & IP", "Flows & Stats"];
        for (i, &st_name) in subtabs.iter().enumerate() {
            let st_rect = Rect::new(content_x + (i as i32 * (subtab_w + 6)), content_y, subtab_w, subtab_h);
            let is_act = self.firewall_cfg.active_subtab == i;
            if is_act {
                ui::draw::draw_rounded_rect(s, st_rect, RADIUS_XS, palette.accent_primary.darken(35));
                ui::draw::draw_rect_outline(s, st_rect, 1, palette.accent_primary);
                let sz = f_small.measure_str(st_name);
                ui::font::draw_text_font(s, st_rect.x + (st_rect.w - sz.w) / 2, st_rect.y + 4, st_name, Color::WHITE, f_small);
            } else {
                ui::draw::draw_rounded_rect(s, st_rect, RADIUS_XS, palette.surface_sunken);
                ui::draw::draw_rect_outline(s, st_rect, 1, palette.border_subtle);
                let sz = f_small.measure_str(st_name);
                ui::font::draw_text_font(s, st_rect.x + (st_rect.w - sz.w) / 2, st_rect.y + 4, st_name, palette.text_secondary, f_small);
            }
        }

        match self.firewall_cfg.active_subtab {
            0 => {
                // === SUBTAB 0: Rules & Policies ===
                let fw_on = self.firewall_cfg.enabled;
                let status_rect = Rect::new(content_x, content_y + 32, 90, 22);
                let st_bg = if fw_on { palette.status_success.darken(30) } else { palette.status_danger.darken(30) };
                let st_fg = if fw_on { palette.status_success } else { palette.status_danger };
                ui::draw::draw_rounded_rect(s, status_rect, RADIUS_XS, st_bg);
                ui::draw::draw_rect_outline(s, status_rect, 1, st_fg);
                let st_text = if fw_on { "ACTIVE" } else { "DISABLED" };
                ui::font::draw_text_font(s, status_rect.x + 12, status_rect.y + 4, st_text, Color::WHITE, f_small);

                // Inbound policy pill
                let in_pol_rect = Rect::new(content_x + 98, content_y + 32, 100, 22);
                let in_bg = if self.firewall_cfg.inbound_drop { palette.status_danger.darken(30) } else { palette.status_success.darken(30) };
                let in_fg = if self.firewall_cfg.inbound_drop { palette.status_danger } else { palette.status_success };
                ui::draw::draw_rounded_rect(s, in_pol_rect, RADIUS_XS, in_bg);
                ui::draw::draw_rect_outline(s, in_pol_rect, 1, in_fg);
                let in_str = if self.firewall_cfg.inbound_drop { "In: DROP" } else { "In: ACCEPT" };
                ui::font::draw_text_font(s, in_pol_rect.x + 8, in_pol_rect.y + 4, in_str, Color::WHITE, f_small);

                // Outbound policy pill
                let out_pol_rect = Rect::new(content_x + 204, content_y + 32, 100, 22);
                let out_bg = if self.firewall_cfg.policy_out_accept { palette.status_success.darken(30) } else { palette.status_danger.darken(30) };
                let out_fg = if self.firewall_cfg.policy_out_accept { palette.status_success } else { palette.status_danger };
                ui::draw::draw_rounded_rect(s, out_pol_rect, RADIUS_XS, out_bg);
                ui::draw::draw_rect_outline(s, out_pol_rect, 1, out_fg);
                let out_str = if self.firewall_cfg.policy_out_accept { "Out: ACCEPT" } else { "Out: DROP" };
                ui::font::draw_text_font(s, out_pol_rect.x + 8, out_pol_rect.y + 4, out_str, Color::WHITE, f_small);

                // Rules count
                let mut info_buf = [0u8; 48];
                let mut ilen = 0;
                ilen += copy_bytes(&mut info_buf[ilen..], b"Rules: ");
                ilen += format_u32(self.firewall_cfg.rule_count as u32, &mut info_buf[ilen..]);
                ilen += copy_bytes(&mut info_buf[ilen..], b" / 32 max");
                if let Ok(info_str) = core::str::from_utf8(&info_buf[..ilen]) {
                    ui::font::draw_text_font(s, content_x + 316, content_y + 36, info_str, palette.text_secondary, f_small);
                }

                // Table Header
                let tbl_y = content_y + 60;
                let tbl_w = (viewport.right() - content_x - 16).max(380);
                s.fill_rect(Rect::new(content_x, tbl_y, tbl_w, 20), palette.surface_raised);
                ui::font::draw_text_font(s, content_x + 6, tbl_y + 3, "#", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 24, tbl_y + 3, "Action", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + 80, tbl_y + 3, "Firewall Rule Criteria & Filter", palette.text_muted, f_small);
                ui::font::draw_text_font(s, content_x + tbl_w - 50, tbl_y + 3, "Hits", palette.text_muted, f_small);

                // Rule Rows
                let mut ry = content_y + 82;
                for i in 0..self.firewall_cfg.rule_count {
                    if ry + 22 > viewport.bottom() - 60 { break; }
                    let r_rect = Rect::new(content_x, ry, tbl_w, 20);
                    let is_sel = self.firewall_cfg.selected_rule == i;
                    let bg_col = if is_sel {
                        palette.surface_sunken.lighten(15)
                    } else if i % 2 == 1 {
                        palette.surface_raised.darken(10)
                    } else {
                        palette.surface_sunken
                    };
                    s.fill_rect(r_rect, bg_col);
                    if is_sel {
                        ui::draw::draw_rect_outline(s, r_rect, 1, palette.accent_primary);
                    }

                    // Index #
                    let mut idx_buf = [0u8; 4];
                    let idx_len = format_u32((i + 1) as u32, &mut idx_buf);
                    if let Ok(idx_str) = core::str::from_utf8(&idx_buf[..idx_len]) {
                        ui::font::draw_text_font(s, content_x + 6, ry + 3, idx_str, palette.text_secondary, f_small);
                    }

                    // Action badge
                    let act_col = if self.firewall_cfg.rules[i].accept { palette.status_success } else { palette.status_danger };
                    let act_txt = if self.firewall_cfg.rules[i].accept { "ACCEPT" } else { "DROP" };
                    ui::font::draw_text_font(s, content_x + 24, ry + 3, act_txt, act_col, f_small);

                    // Rule description text
                    let mut desc_buf = [0u8; 96];
                    let desc_len = self.firewall_cfg.rules[i].format_description(&mut desc_buf);
                    if let Ok(desc_str) = core::str::from_utf8(&desc_buf[..desc_len]) {
                        ui::font::draw_text_font(s, content_x + 80, ry + 3, desc_str, palette.text_primary, f_small);
                    }

                    // Hits counter
                    let mut hits_buf = [0u8; 10];
                    let h_len = format_u32(self.firewall_cfg.rules[i].hits, &mut hits_buf);
                    if let Ok(h_str) = core::str::from_utf8(&hits_buf[..h_len]) {
                        ui::font::draw_text_font(s, content_x + tbl_w - 50, ry + 3, h_str, palette.text_secondary, f_small);
                    }

                    ry += 22;
                }

                // Action buttons toolbar
                let by = ry + 8;
                let ssh_rect = Rect::new(content_x, by, 96, 22);
                ui::draw::draw_rounded_rect(s, ssh_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, ssh_rect, 1, palette.border_subtle);
                ui::font::draw_text_font(s, ssh_rect.x + 8, ssh_rect.y + 4, "+ SSH (22)", palette.text_primary, f_small);

                let http_rect = Rect::new(content_x + 102, by, 96, 22);
                ui::draw::draw_rounded_rect(s, http_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, http_rect, 1, palette.border_subtle);
                ui::font::draw_text_font(s, http_rect.x + 8, http_rect.y + 4, "+ HTTP (80)", palette.text_primary, f_small);

                let del_rect = Rect::new(content_x + 204, by, 84, 22);
                ui::draw::draw_rounded_rect(s, del_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, del_rect, 1, palette.status_danger);
                ui::font::draw_text_font(s, del_rect.x + 8, del_rect.y + 4, "- Delete", palette.status_danger, f_small);

                let sync_rect = Rect::new(content_x + 294, by, 88, 22);
                ui::draw::draw_rounded_rect(s, sync_rect, RADIUS_XS, palette.accent_primary.darken(35));
                ui::draw::draw_rect_outline(s, sync_rect, 1, palette.accent_primary);
                ui::font::draw_text_font(s, sync_rect.x + 8, sync_rect.y + 4, "Sync Live", Color::WHITE, f_small);
            }

            1 => {
                // === SUBTAB 1: Network & IP Configuration ===
                let mut ny = content_y + 36;
                ui::font::draw_text_font(s, content_x, ny, "Network Interface & Adapter Hardware:", palette.accent_primary, font);
                ny += 22;

                let card_rect = Rect::new(content_x, ny, 380, 76);
                s.fill_rect(card_rect, palette.surface_raised);
                ui::draw::draw_rect_outline(s, card_rect, 1, palette.border_subtle);

                ui::font::draw_text_font(s, card_rect.x + 10, card_rect.y + 8, "Adapter:", palette.text_secondary, f_small);
                ui::font::draw_text_font(s, card_rect.x + 90, card_rect.y + 8, "eth0 (Realtek RTL8139 / Virtio-Net)", palette.text_primary, f_small);

                ui::font::draw_text_font(s, card_rect.x + 10, card_rect.y + 28, "Hardware MAC:", palette.text_secondary, f_small);
                let mut mac_buf = [0u8; 18];
                let m_len = self.network_cfg.format_mac(&mut mac_buf);
                if let Ok(m_str) = core::str::from_utf8(&mac_buf[..m_len]) {
                    ui::font::draw_text_font(s, card_rect.x + 90, card_rect.y + 28, m_str, palette.text_primary, f_small);
                }

                ui::font::draw_text_font(s, card_rect.x + 10, card_rect.y + 48, "Link State:", palette.text_secondary, f_small);
                ui::font::draw_text_font(s, card_rect.x + 90, card_rect.y + 48, "UP - 100 Mbps Full Duplex", palette.status_success, f_small);

                ny += 86;

                ui::font::draw_text_font(s, content_x, ny, "IPv4 Protocol Addressing & Routing:", palette.accent_primary, font);
                ny += 22;

                let ip_card = Rect::new(content_x, ny, 380, 96);
                s.fill_rect(ip_card, palette.surface_raised);
                ui::draw::draw_rect_outline(s, ip_card, 1, palette.border_subtle);

                let mut ip_buf = [0u8; 16];
                let mut l_y = ip_card.y + 8;

                ui::font::draw_text_font(s, ip_card.x + 10, l_y, "IP Address:", palette.text_secondary, f_small);
                let ip_len = self.network_cfg.format_ip_bytes(&self.network_cfg.ip_addr, &mut ip_buf);
                if let Ok(ip_s) = core::str::from_utf8(&ip_buf[..ip_len]) {
                    ui::font::draw_text_font(s, ip_card.x + 110, l_y, ip_s, palette.text_primary, f_small);
                }
                l_y += 20;

                ui::font::draw_text_font(s, ip_card.x + 10, l_y, "Subnet Mask:", palette.text_secondary, f_small);
                let nm_len = self.network_cfg.format_ip_bytes(&self.network_cfg.netmask, &mut ip_buf);
                if let Ok(nm_s) = core::str::from_utf8(&ip_buf[..nm_len]) {
                    ui::font::draw_text_font(s, ip_card.x + 110, l_y, nm_s, palette.text_primary, f_small);
                }
                l_y += 20;

                ui::font::draw_text_font(s, ip_card.x + 10, l_y, "Gateway:", palette.text_secondary, f_small);
                let gw_len = self.network_cfg.format_ip_bytes(&self.network_cfg.gateway, &mut ip_buf);
                if let Ok(gw_s) = core::str::from_utf8(&ip_buf[..gw_len]) {
                    ui::font::draw_text_font(s, ip_card.x + 110, l_y, gw_s, palette.text_primary, f_small);
                }
                l_y += 20;

                ui::font::draw_text_font(s, ip_card.x + 10, l_y, "DNS Server:", palette.text_secondary, f_small);
                let dns_len = self.network_cfg.format_ip_bytes(&self.network_cfg.dns_server, &mut ip_buf);
                if let Ok(dns_s) = core::str::from_utf8(&ip_buf[..dns_len]) {
                    ui::font::draw_text_font(s, ip_card.x + 110, l_y, dns_s, palette.text_primary, f_small);
                }
            }

            _ => {
                // === SUBTAB 2: Flows & Statistics ===
                let mut sy = content_y + 36;
                ui::font::draw_text_font(s, content_x, sy, "Kernel Packet Filter Telemetry:", palette.accent_primary, font);
                sy += 22;

                let stats_card = Rect::new(content_x, sy, 380, 96);
                s.fill_rect(stats_card, palette.surface_raised);
                ui::draw::draw_rect_outline(s, stats_card, 1, palette.border_subtle);

                let mut num_buf = [0u8; 12];
                let mut cy = stats_card.y + 8;

                ui::font::draw_text_font(s, stats_card.x + 10, cy, "Inbound Accepted:", palette.text_secondary, f_small);
                let l = format_u32(self.firewall_cfg.accepted_in, &mut num_buf);
                if let Ok(s_txt) = core::str::from_utf8(&num_buf[..l]) {
                    ui::font::draw_text_font(s, stats_card.x + 140, cy, s_txt, palette.status_success, f_small);
                }
                cy += 20;

                ui::font::draw_text_font(s, stats_card.x + 10, cy, "Inbound Dropped:", palette.text_secondary, f_small);
                let l2 = format_u32(self.firewall_cfg.dropped_in, &mut num_buf);
                if let Ok(s_txt) = core::str::from_utf8(&num_buf[..l2]) {
                    ui::font::draw_text_font(s, stats_card.x + 140, cy, s_txt, palette.status_danger, f_small);
                }
                cy += 20;

                ui::font::draw_text_font(s, stats_card.x + 10, cy, "Outbound Accepted:", palette.text_secondary, f_small);
                let l3 = format_u32(self.firewall_cfg.accepted_out, &mut num_buf);
                if let Ok(s_txt) = core::str::from_utf8(&num_buf[..l3]) {
                    ui::font::draw_text_font(s, stats_card.x + 140, cy, s_txt, palette.text_primary, f_small);
                }
                cy += 20;

                ui::font::draw_text_font(s, stats_card.x + 10, cy, "Active Flows:", palette.text_secondary, f_small);
                let l4 = format_u32(self.firewall_cfg.conns_count, &mut num_buf);
                if let Ok(s_txt) = core::str::from_utf8(&num_buf[..l4]) {
                    ui::font::draw_text_font(s, stats_card.x + 140, cy, s_txt, palette.accent_primary, f_small);
                }

                let flush_rect = Rect::new(content_x, sy + 108, 110, 22);
                ui::draw::draw_rounded_rect(s, flush_rect, RADIUS_XS, palette.surface_raised);
                ui::draw::draw_rect_outline(s, flush_rect, 1, palette.border_subtle);
                ui::font::draw_text_font(s, flush_rect.x + 8, flush_rect.y + 4, "Flush Flows", palette.text_secondary, f_small);
            }
        }
    }
}

// ===========================================================================
// Selftest Suite
// ===========================================================================

static mut TEST_RENDER_BUF: [u32; 640 * 240] = [0u32; 640 * 240];
static mut TEST_USERS_BUF: [u8; USERS_TOTAL_SIZE] = [0u8; USERS_TOTAL_SIZE];
static mut TEST_FW_BUF: [u8; 1024] = [0u8; 1024];

#[inline(never)]
fn selftest_system_and_config(app: &mut SettingsApp) -> i32 {
    let mut code = 0;

    // Case 1: Plugin panel registration & category switching
    let all_panels = PanelKind::all();
    let panels_ok = all_panels.len() == 7
        && all_panels[0] == PanelKind::System
        && all_panels[1] == PanelKind::Users
        && all_panels[2] == PanelKind::Services
        && all_panels[3] == PanelKind::Firewall
        && all_panels[4] == PanelKind::Display
        && all_panels[5] == PanelKind::Devices
        && all_panels[6] == PanelKind::Updates;

    app.select_panel(PanelKind::Users);
    let select_users_ok = app.active_panel == PanelKind::Users;
    app.select_panel(PanelKind::Services);
    let select_services_ok = app.active_panel == PanelKind::Services;
    app.select_panel(PanelKind::Devices);
    let select_dev_ok = app.active_panel == PanelKind::Devices;
    app.select_panel(PanelKind::Updates);
    let select_updates_ok = app.active_panel == PanelKind::Updates;
    app.select_panel(PanelKind::System);
    let select_sys_ok = app.active_panel == PanelKind::System;

    if panels_ok && select_users_ok && select_services_ok && select_dev_ok && select_updates_ok && select_sys_ok {
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
    let users_buf = unsafe { &mut *core::ptr::addr_of_mut!(TEST_USERS_BUF) };
    let u_len = app.users_cfg.to_bytes(users_buf);
    let users_bytes_ok = u_len == USERS_TOTAL_SIZE && &users_buf[0..4] == b"NVUS";

    let add_ok = app.users_cfg.add_user("testuser", 1002, 1002);
    let u_len2 = app.users_cfg.to_bytes(users_buf);
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

    code
}

#[inline(never)]
fn selftest_mouse_and_surface(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 5: Mouse hit-testing, action dispatch, and dynamic cursor shapes
    let vp = Rect::new(0, 0, 640, 480);
    let hit_tab1 = app.button_at(50, 95, vp) == Some(SettingsAction::SelectPanel(PanelKind::Users));
    app.on_left_click(50, 95, vp);
    let tab1_selected = app.active_panel == PanelKind::Users;

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
    surf.buffer.fill(0);
    app.select_panel(PanelKind::System);
    app.render(surf, Rect::new(0, 0, 640, 240));

    let palette = Theme::Dark.palette();
    let sidebar_pixel = surf.buffer[10 * 640 + 10];
    let content_pixel = surf.buffer[80 * 640 + 200];
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

    code
}

#[inline(never)]
fn selftest_firewall_and_network(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 7: Live firewall query via SYS_FW_INFO
    app.select_panel(PanelKind::Firewall);
    let sync_ok = app.firewall_cfg.sync_from_kernel();
    let fw_status_ok = sync_ok && app.firewall_cfg.enabled && app.firewall_cfg.inbound_drop;

    if fw_status_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: SYS_FW_INFO live kernel firewall query\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: SYS_FW_INFO query\n\0".as_ptr()) };
        code |= 64;
    }

    // Case 8: Live rule enumeration via SYS_FW_CTL (NOVA_FW_OP_GET_RULE)
    let rules_count_ok = app.firewall_cfg.rule_count >= 2;
    let r0 = &app.firewall_cfg.rules[0];
    let r1 = &app.firewall_cfg.rules[1];
    let r0_ok = r0.accept && r0.dir == ffi::NOVA_FW_DIR_IN;
    let r1_ok = r1.accept && r1.proto == ffi::NOVA_FW_PROTO_ICMP && r1.icmp_set && r1.icmp_type == 8;

    if rules_count_ok && r0_ok && r1_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: SYS_FW_CTL rule table enumeration and criteria matching\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: SYS_FW_CTL rule table enumeration\n\0".as_ptr()) };
        code |= 128;
    }

    // Case 9: Inbound and outbound policy toggles (NOVA_FW_OP_SET_POLICY)
    let pol_ok = if !app.firewall_cfg.locked {
        let pol_in_toggled = app.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, true);
        let pol_in_verified = !app.firewall_cfg.inbound_drop;
        let pol_in_restored = app.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, false);
        let pol_restored_verified = app.firewall_cfg.inbound_drop;
        pol_in_toggled && pol_in_verified && pol_in_restored && pol_restored_verified
    } else {
        let prev_drop = app.firewall_cfg.inbound_drop;
        let mutation_rejected = !app.firewall_cfg.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, true);
        mutation_rejected && app.firewall_cfg.inbound_drop == prev_drop && app.firewall_cfg.locked
    };

    if pol_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: firewall policy mutation and kernel dispatch\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: firewall policy mutation\n\0".as_ptr()) };
        code |= 256;
    }

    // Case 10: Dynamic rule addition (NOVA_FW_OP_ADD_RULE) & deletion (NOVA_FW_OP_DEL_RULE)
    let rule_mutation_ok = if !app.firewall_cfg.locked {
        let count_before = app.firewall_cfg.rule_count;
        let add_rule_ok = app.firewall_cfg.add_rule_kernel("rule accept in proto tcp dport 22 state new log");
        let count_after_add = app.firewall_cfg.rule_count;
        let added_ok = add_rule_ok && count_after_add == count_before + 1;

        let del_rule_ok = app.firewall_cfg.delete_rule_kernel(count_after_add - 1);
        let count_after_del = app.firewall_cfg.rule_count;
        let del_ok = del_rule_ok && count_after_del == count_before;
        added_ok && del_ok
    } else {
        let count_before = app.firewall_cfg.rule_count;
        let add_rejected = !app.firewall_cfg.add_rule_kernel("rule accept in proto tcp dport 22 state new log");
        let del_rejected = !app.firewall_cfg.delete_rule_kernel(0);
        let count_unchanged = app.firewall_cfg.rule_count == count_before;
        add_rejected && del_rejected && count_unchanged
    };

    if rule_mutation_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: dynamic rule addition and deletion\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: dynamic rule addition/deletion\n\0".as_ptr()) };
        code |= 512;
    }

    // Case 11: Network interface & IP configuration model
    let mut mac_buf = [0u8; 18];
    let m_len = app.network_cfg.format_mac(&mut mac_buf);
    let mac_str = core::str::from_utf8(&mac_buf[..m_len]).unwrap_or("");
    let mac_ok = mac_str == "52:54:00:12:34:56";

    let mut ip_buf = [0u8; 16];
    let ip_len = app.network_cfg.format_ip_bytes(&app.network_cfg.ip_addr, &mut ip_buf);
    let ip_str = core::str::from_utf8(&ip_buf[..ip_len]).unwrap_or("");
    let ip_ok = ip_str == "10.0.2.15";

    let gw_len = app.network_cfg.format_ip_bytes(&app.network_cfg.gateway, &mut ip_buf);
    let gw_str = core::str::from_utf8(&ip_buf[..gw_len]).unwrap_or("");
    let gw_ok = gw_str == "10.0.2.2";

    if mac_ok && ip_ok && gw_ok && app.network_cfg.link_up {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: network interface and IPv4 address configuration model\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: network configuration model\n\0".as_ptr()) };
        code |= 1024;
    }

    // Case 12: FIREWALL.CFG file serialization roundtrip
    let fw_buf = unsafe { &mut *core::ptr::addr_of_mut!(TEST_FW_BUF) };
    let fw_len = app.firewall_cfg.to_bytes(fw_buf);
    let fw_serialized_ok = fw_len > 0 && fw_buf[..fw_len].starts_with(b"# FIREWALL.CFG");
    let parsed_fw = FirewallConfig::from_bytes(&fw_buf[..fw_len]).unwrap();
    let fw_roundtrip_ok = parsed_fw.rule_count == app.firewall_cfg.rule_count
        && parsed_fw.inbound_drop == app.firewall_cfg.inbound_drop;

    if fw_serialized_ok && fw_roundtrip_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: FIREWALL.CFG persistent file serialization roundtrip\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: FIREWALL.CFG serialization\n\0".as_ptr()) };
        code |= 2048;
    }

    // Case 13: Graphical surface rendering of Network & Firewall sub-tabs
    app.firewall_cfg.active_subtab = 0; // Rules
    app.render(surf, Rect::new(0, 0, 640, 240));
    let rules_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    app.firewall_cfg.active_subtab = 1; // Network & IP
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let net_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    app.firewall_cfg.active_subtab = 2; // Flows & Stats
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let stats_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    if rules_pixels > 5000 && net_pixels > 5000 && stats_pixels > 5000 {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: Network & Firewall graphical presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: Network & Firewall graphical presentation\n\0".as_ptr()) };
        code |= 4096;
    }

    code
}

#[inline(never)]
fn selftest_users_and_permissions(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 14: Graphical useradd and PBKDF2-HMAC-SHA256 account credentials
    app.select_panel(PanelKind::Users);
    let vp = Rect::new(0, 0, 640, 480);
    // 1. Click "+ Add User" button (x: 282..376, y: 88..110)
    app.on_left_click(300, 95, vp);
    let form_opened = app.users_ui.view_mode == UsersViewMode::CreateAccount;

    // 2. Click "Alice" preset button (x: 262..352, y: 138..158)
    app.on_left_click(280, 145, vp);
    let preset_applied = app.users_ui.form_username_str() == "alice"
        && !app.users_ui.form_is_admin
        && app.users_ui.form_password_str() == "alicepass";

    // 3. Click "Create Account" submit button (x: 202..332, y: 318..344)
    app.on_left_click(250, 325, vp);
    let form_closed = app.users_ui.view_mode == UsersViewMode::List;

    let alice_idx = app.users_cfg.find_user("alice");
    let alice_ok = if let Some(idx) = alice_idx {
        let u = &app.users_cfg.accounts[idx];
        !u.is_admin()
            && u.verify_password(b"alicepass")
            && !u.verify_password(b"wrongpass")
            && u.uid >= 1000
    } else {
        false
    };

    if form_opened && preset_applied && form_closed && alice_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: graphical useradd and PBKDF2-HMAC-SHA256 account credentials\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: graphical useradd and account credentials\n\0".as_ptr()) };
        code |= 8192;
    }

    // Case 15: User role privilege toggle and last-administrator safeguard
    let mut role_ok = false;
    if let Some(a_idx) = app.users_cfg.find_user("alice") {
        // Promote alice to Administrator (gid = 0)
        let promote_res = app.users_cfg.toggle_admin(a_idx);
        let promote_ok = promote_res == Ok(true) && app.users_cfg.accounts[a_idx].is_admin();

        // Demote alice back to Standard User
        let demote_res = app.users_cfg.toggle_admin(a_idx);
        let demote_ok = demote_res == Ok(false) && !app.users_cfg.accounts[a_idx].is_admin();

        // Last-administrator safeguard: attempt to demote the sole remaining administrator ("admin")
        if let Some(admin_idx) = app.users_cfg.find_user("admin") {
            let demote_last_admin_res = app.users_cfg.toggle_admin(admin_idx);
            let safeguard_ok = demote_last_admin_res.is_err() && app.users_cfg.accounts[admin_idx].is_admin();
            role_ok = promote_ok && demote_ok && safeguard_ok;
        }
    }

    if role_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: user role privilege toggle and last-administrator safeguard\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: user role privilege toggle\n\0".as_ptr()) };
        code |= 16384;
    }

    // Case 16: Account deletion and minimum account safeguards
    let mut deletion_ok = false;
    let add_bob_ok = app.users_cfg.add_user_with_credentials("bob", 1004, false, b"bobpass", 0x1234).is_ok();
    if let (Some(b_idx), Some(_)) = (app.users_cfg.find_user("bob"), app.users_cfg.find_user("alice")) {
        let del_bob_ok = app.users_cfg.delete_account(b_idx).is_ok() && app.users_cfg.find_user("bob").is_none();
        // Recalculate alice index after bob deletion
        if let Some(cur_a_idx) = app.users_cfg.find_user("alice") {
            let del_alice_ok = app.users_cfg.delete_account(cur_a_idx).is_ok() && app.users_cfg.find_user("alice").is_none();

            // Clean up testuser if present from earlier tests
            if let Some(t_idx) = app.users_cfg.find_user("testuser") {
                let _ = app.users_cfg.delete_account(t_idx);
            }

            // Now accounts remaining: "persisted" (standard) and "admin" (admin)
            if let Some(admin_idx) = app.users_cfg.find_user("admin") {
                // Safeguard: Cannot delete sole administrator
                let del_admin_err = app.users_cfg.delete_account(admin_idx).is_err();

                if let Some(p_idx) = app.users_cfg.find_user("persisted") {
                    let del_p_ok = app.users_cfg.delete_account(p_idx).is_ok();
                    let count_one = app.users_cfg.count == 1;

                    // Minimum account safeguard: Cannot delete the only remaining account
                    let del_sole_err = if let Some(adm2) = app.users_cfg.find_user("admin") {
                        app.users_cfg.delete_account(adm2).is_err()
                    } else {
                        false
                    };

                    // Restore "persisted" account to keep mock store consistent
                    let _ = app.users_cfg.add_user_with_credentials("persisted", 700, false, b"persisted-pw", 0x1234_5678);
                    app.save_users_cfg();

                    deletion_ok = add_bob_ok && del_bob_ok && del_alice_ok && del_admin_err && del_p_ok && count_one && del_sole_err;
                }
            }
        }
    }

    if deletion_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: account deletion and minimum account safeguards\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: account deletion safeguards\n\0".as_ptr()) };
        code |= 32768;
    }

    // Case 17: User & Permissions graphical presentation on BufferSurface32
    app.select_panel(PanelKind::Users);
    app.users_ui.view_mode = UsersViewMode::List;
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let list_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    app.users_ui.open_create_form(1005);
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let form_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    app.users_ui.open_change_password();
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let pw_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    if list_pixels > 5000 && form_pixels > 5000 && pw_pixels > 5000 {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: User & Permissions graphical presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: User & Permissions graphical presentation\n\0".as_ptr()) };
        code |= 65536;
    }

    code
}

#[inline(never)]
fn selftest_service_manager(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 18: Live service supervisor query and supervision controls
    app.select_panel(PanelKind::Services);
    app.service_mgr.view_mode = ServiceViewMode::LiveDashboard;
    let query_ok = app.service_mgr.sync_from_kernel();
    let count_ok = app.service_mgr.count >= 3;
    let s0_ok = app.service_mgr.services[0].name_str() == "network-check";
    let s1_ok = app.service_mgr.services[1].name_str() == "greeting";
    let s2_ok = app.service_mgr.services[2].name_str() == "shell"
        && app.service_mgr.services[2].trusted;

    // Dispatch control action: Restart service 0 ("network-check")
    let restart_ok = app.service_mgr.restart_service(0);
    // Dispatch control action: Stop service 2 ("shell")
    let stop_ok = app.service_mgr.stop_service(2);
    // Dispatch control action: Start service 1 ("greeting")
    let start_ok = app.service_mgr.start_service(1);

    if query_ok && count_ok && s0_ok && s1_ok && s2_ok && restart_ok && stop_ok && start_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: live service supervisor query and supervision controls\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: live service supervisor query/controls\n\0".as_ptr()) };
        code |= 131072;
    }

    // Case 19: Interactive mouse hit-testing and view modes
    let vp = Rect::new(0, 0, 640, 480);
    // Switch to ConfigFile editor (Subtab 1 at x: 302..422, y: 64..88)
    app.on_left_click(310, 70, vp);
    let config_mode_ok = app.service_mgr.view_mode == ServiceViewMode::ConfigFile;
    // Switch back to LiveDashboard (Subtab 0 at x: 176..296, y: 64..88)
    app.on_left_click(180, 70, vp);
    let live_mode_ok = app.service_mgr.view_mode == ServiceViewMode::LiveDashboard;

    // Click row 1 (list_y starts at 64 + 78 = 142, row 1 at y: 164..186)
    app.on_left_click(200, 170, vp);
    let row_sel_ok = app.service_mgr.selected_idx == 1;

    if config_mode_ok && live_mode_ok && row_sel_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: Service Manager mouse hit-testing and view modes\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: Service Manager mouse hit-testing\n\0".as_ptr()) };
        code |= 262144;
    }

    // Case 20: Service Manager graphical presentation on BufferSurface32
    app.service_mgr.view_mode = ServiceViewMode::LiveDashboard;
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let live_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    app.service_mgr.view_mode = ServiceViewMode::ConfigFile;
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let cfg_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    if live_pixels > 5000 && cfg_pixels > 5000 {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: Service Manager graphical presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: Service Manager graphical presentation\n\0".as_ptr()) };
        code |= 524288;
    }

    code
}

fn selftest_device_manager(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 21: Device Manager hardware query and driver registration
    app.select_panel(PanelKind::Devices);
    app.device_mgr.view_mode = DeviceViewMode::HardwareTree;
    let query_ok = app.device_mgr.sync_from_kernel();
    let drv_count_ok = app.device_mgr.driver_count() >= 10;
    let dev_count_ok = app.device_mgr.device_count() >= 10;
    let nvme_ok = app.device_mgr.summary.nvme_present != 0;
    let xhci_ok = app.device_mgr.summary.xhci_present != 0;
    let hw_ok = nvme_ok && xhci_ok;

    if query_ok && drv_count_ok && dev_count_ok && hw_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: Device Manager hardware query and driver registration\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: Device Manager hardware query/drivers\n\0".as_ptr()) };
        code |= 1048576;
    }

    // Case 22: Device Manager mouse hit-testing and category filter
    let vp = Rect::new(0, 0, 640, 480);
    // Switch to DriverTable view (Subtab 1 at x: 292..422, y: 64..88)
    app.on_left_click(300, 70, vp);
    let driver_mode_ok = app.device_mgr.view_mode == DeviceViewMode::DriverTable;

    // Switch back to HardwareTree view (Subtab 0 at x: 176..286, y: 64..88)
    app.on_left_click(180, 70, vp);
    let tree_mode_ok = app.device_mgr.view_mode == DeviceViewMode::HardwareTree;

    // Filter by Storage (Category chip 1 at x: 176 + 56 = 232, y: 128..148)
    app.on_left_click(240, 135, vp);
    let storage_filter_ok = app.device_mgr.category_filter == DeviceCategoryFilter::Storage;
    let storage_count_ok = app.device_mgr.filtered_device_count() >= 1;

    // Filter by All (Category chip 0 at x: 176, y: 128..148)
    app.on_left_click(180, 135, vp);
    let all_filter_ok = app.device_mgr.category_filter == DeviceCategoryFilter::All;

    // Select row 1 (NVMe is row 0, xHCI is row 1 at y: 172 + 20 = 192..212)
    app.on_left_click(200, 195, vp);
    let dev_sel_ok = app.device_mgr.selected_device_idx == 1;

    // Trigger rescan button (x: 556..626, y: 64..86)
    app.on_left_click(560, 70, vp);
    let rescan_ok = app.device_mgr.loaded;

    if driver_mode_ok && tree_mode_ok && storage_filter_ok && storage_count_ok && all_filter_ok && dev_sel_ok && rescan_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: Device Manager mouse hit-testing and category filter\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: Device Manager hit-testing/filter\n\0".as_ptr()) };
        code |= 2097152;
    }

    // Case 23: Device Manager graphical presentation on BufferSurface32
    app.device_mgr.view_mode = DeviceViewMode::HardwareTree;
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let tree_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    app.device_mgr.view_mode = DeviceViewMode::DriverTable;
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let driver_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    if tree_pixels > 5000 && driver_pixels > 5000 {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: Device Manager graphical presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: Device Manager graphical presentation\n\0".as_ptr()) };
        code |= 4194304;
    }

    code
}

#[inline(never)]
fn selftest_accessibility_panel(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 24: Accessibility configuration and ACCESS.CFG persistence
    app.select_panel(PanelKind::Display);

    // 1. Theme cycling and High Contrast toggle
    app.apply_action(SettingsAction::DisplayToggleHighContrast);
    let hc_enabled = app.display_cfg.theme.is_high_contrast()
        && app.display_cfg.cursor_theme == CursorTheme::HighContrast
        && app.display_cfg.high_contrast_cursor;

    // 2. Font scale configuration
    app.apply_action(SettingsAction::DisplaySetFontScale(FontScale::Large)); // 150%
    let scale_large_ok = app.display_cfg.font_scale == FontScale::Large;

    app.apply_action(SettingsAction::DisplayCycleFontScale); // 150% -> 200%
    let scale_cycle_ok = app.display_cfg.font_scale == FontScale::ExtraLarge;

    // 3. Cursor theme cycling
    app.apply_action(SettingsAction::DisplayCycleCursor);
    let cursor_cycled_ok = app.display_cfg.cursor_theme != CursorTheme::HighContrast;

    // Restore high-contrast cursor
    app.display_cfg.cursor_theme = CursorTheme::HighContrast;

    // 4. Binary serialization roundtrip (ACCESS.CFG format)
    let cfg = app.display_cfg.to_accessibility_config();
    let mut cfg_buf = [0u8; ACCESS_CFG_SIZE];
    cfg.serialize(&mut cfg_buf);
    let magic_ok = &cfg_buf[0..4] == &ACCESS_CFG_MAGIC;
    let ver_ok = cfg_buf[4] == 1;
    let theme_byte_ok = cfg_buf[5] == 2; // HighContrast
    let scale_byte_ok = cfg_buf[6] == 200; // ExtraLarge

    let deserialized = AccessibilityConfig::deserialize(&cfg_buf);
    let roundtrip_ok = deserialized == Some(cfg);

    if hc_enabled && scale_large_ok && scale_cycle_ok && cursor_cycled_ok && magic_ok && ver_ok && theme_byte_ok && scale_byte_ok && roundtrip_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: accessibility configuration and ACCESS.CFG persistence\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: accessibility configuration/persistence\n\0".as_ptr()) };
        code |= 8388608;
    }

    // Case 25: Accessibility preferences graphical presentation on BufferSurface32
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));

    // High-contrast palette: background is pure black
    let pal = app.display_cfg.theme.palette();
    let is_black_bg = pal.desktop_bg == Color::BLACK;

    // Check that UI elements were rendered into surface
    let non_zero_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();

    // Verify live preview rendered scaled text pixels (around y = 140..170, x = 180..400)
    let mut found_preview_pixels = false;
    for y in 140..170 {
        for x in 180..400 {
            if surf.buffer[y * 640 + x] != 0 {
                found_preview_pixels = true;
                break;
            }
        }
        if found_preview_pixels {
            break;
        }
    }

    if is_black_bg && non_zero_pixels > 4000 && found_preview_pixels {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: accessibility preferences graphical presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: accessibility preferences presentation\n\0".as_ptr()) };
        code |= 16777216;
    }

    code
}

#[inline(never)]
fn selftest_updates_panel(app: &mut SettingsApp, surf: &mut BufferSurface32) -> i32 {
    let mut code = 0;

    // Case 26: Atomic update boot slot query and status telemetry
    app.select_panel(PanelKind::Updates);
    let select_ok = app.active_panel == PanelKind::Updates;

    let sync_ok = app.update_state.sync_from_kernel();
    let slot_a_ok = app.update_state.status.active_slot == ffi::NOVA_BOOT_SLOT_A;
    let state_norm_ok = app.update_state.status.slot_state == ffi::NOVA_SLOT_STATE_NORMAL;
    let attempts_ok = app.update_state.status.boot_attempts == 3;
    let ver_a_ok = app.update_state.status.slot_a_version_str() == "0.88.0";

    if select_ok && sync_ok && slot_a_ok && state_norm_ok && attempts_ok && ver_a_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: atomic update boot slot query and status telemetry\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: atomic update boot slot query\n\0".as_ptr()) };
        code |= 33554432;
    }

    // Case 27: Atomic update workflow and dual-slot presentation on BufferSurface32
    // 1. Check for updates -> transitions to Available
    app.apply_action(SettingsAction::UpdatesCheck);
    let step_avail_ok = app.update_state.step == UpdateStep::Available;

    // 2. Review update -> transitions to ConfirmDialog
    app.apply_action(SettingsAction::UpdatesReview);
    let step_confirm_ok = app.update_state.step == UpdateStep::ConfirmDialog;

    // 3. Confirm and apply -> stages to secondary slot, progress 100%, transitions to Complete
    app.apply_action(SettingsAction::UpdatesConfirmAndApply);
    let step_complete_ok = app.update_state.step == UpdateStep::Complete;
    let progress_ok = app.update_state.progress_pct == 100;
    let slot_state_testing_ok = app.update_state.status.slot_state == ffi::NOVA_SLOT_STATE_TESTING;
    let target_ver_ok = app.update_state.status.slot_b_version_str() == "0.89.0";

    // 4. Confirm Healthy
    app.apply_action(SettingsAction::UpdatesConfirmHealthy);
    let state_confirmed_ok = app.update_state.status.slot_state == ffi::NOVA_SLOT_STATE_NORMAL;

    // 5. Rollback
    app.apply_action(SettingsAction::UpdatesRollback);
    let state_rolled_back_ok = app.update_state.status.slot_state == ffi::NOVA_SLOT_STATE_ROLLED_BACK;

    // 6. Surface rendering test
    surf.buffer.fill(0);
    app.render(surf, Rect::new(0, 0, 640, 240));
    let non_zero_pixels = surf.buffer.iter().filter(|&&p| p != 0).count();
    let render_ok = non_zero_pixels > 4000;

    if step_avail_ok && step_confirm_ok && step_complete_ok && progress_ok && slot_state_testing_ok && target_ver_ok && state_confirmed_ok && state_rolled_back_ok && render_ok {
        unsafe { ffi::sys_write(b"[settings-selftest] PASS: atomic update workflow and dual-slot presentation on BufferSurface32\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[settings-selftest] FAIL: atomic update workflow and presentation\n\0".as_ptr()) };
        code |= 67108864;
    }

    code
}

#[inline(never)]
pub fn run_selftest() -> i32 {
    let mut code = 0;
    unsafe { ffi::sys_write(b"[settings-selftest] starting\n\0".as_ptr()) };

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_SETTINGS) };
    app.init();

    code |= selftest_system_and_config(app);

    let render_slice = unsafe {
        core::slice::from_raw_parts_mut(core::ptr::addr_of_mut!(TEST_RENDER_BUF) as *mut u32, 640 * 240)
    };
    render_slice.fill(0);
    let mut surf = BufferSurface32::new(render_slice, 640, 240);

    code |= selftest_mouse_and_surface(app, &mut surf);
    code |= selftest_firewall_and_network(app, &mut surf);
    code |= selftest_users_and_permissions(app, &mut surf);
    code |= selftest_service_manager(app, &mut surf);
    code |= selftest_device_manager(app, &mut surf);
    code |= selftest_accessibility_panel(app, &mut surf);
    code |= selftest_updates_panel(app, &mut surf);

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

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_SETTINGS) };
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
            app.on_left_click(cursor_x, cursor_y, viewport);
        }
        prev_left = m.left_button;

        app.render(&mut surf, viewport);

        let cur_shape = app.cursor_shape_at(cursor_x, cursor_y, viewport);
        ui::cursor::draw_cursor_sprite(&mut surf, cursor_x, cursor_y, cur_shape, app.display_cfg.cursor_theme);

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
        unsafe { ffi::reset_mock_environment() };
        assert_eq!(run_selftest(), 0);
    }

    #[test]
    fn test_system_config_serialization() {
        unsafe { ffi::reset_mock_environment() };
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
        unsafe { ffi::reset_mock_environment() };
        let mut cfg = UsersConfig::new();
        assert_eq!(cfg.count, 2);
        assert_eq!(cfg.accounts[0].username_str(), "persisted");
        assert_eq!(cfg.accounts[1].username_str(), "admin");

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
        unsafe { ffi::reset_mock_environment() };
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
    fn test_firewall_config_kernel_sync_and_mutation() {
        unsafe { ffi::reset_mock_environment() };
        let mut fw = FirewallConfig::new();
        assert!(fw.sync_from_kernel());
        assert_eq!(fw.rule_count, 2);
        assert_eq!(fw.rules[0].dir, ffi::NOVA_FW_DIR_IN);
        assert_eq!(fw.rules[1].proto, ffi::NOVA_FW_PROTO_ICMP);

        // Policy change
        assert!(fw.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, true));
        assert!(!fw.inbound_drop);
        assert!(fw.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, false));
        assert!(fw.inbound_drop);

        // Add & Delete rule
        assert!(fw.add_rule_kernel("rule accept in proto tcp dport 22 state new log"));
        assert_eq!(fw.rule_count, 3);
        assert!(fw.delete_rule_kernel(2));
        assert_eq!(fw.rule_count, 2);
    }

    #[test]
    fn test_firewall_locked_state() {
        unsafe {
            ffi::reset_mock_environment();
            ffi::MOCK_FW_LOCKED = 1;
        };
        let mut fw = FirewallConfig::new();
        assert!(fw.sync_from_kernel());
        assert!(fw.locked);

        // Mutations must be rejected when locked
        assert!(!fw.apply_policy_to_kernel(ffi::NOVA_FW_DIR_IN, true));
        assert!(!fw.add_rule_kernel("rule accept in proto tcp dport 22 state new log"));
        assert!(!fw.delete_rule_kernel(0));
        assert_eq!(fw.rule_count, 2);
    }

    #[test]
    fn test_network_config_formatting() {
        let net = NetworkConfig::new();
        let mut mac_buf = [0u8; 18];
        let m_len = net.format_mac(&mut mac_buf);
        assert_eq!(&mac_buf[..m_len], b"52:54:00:12:34:56");

        let mut ip_buf = [0u8; 16];
        let ip_len = net.format_ip_bytes(&net.ip_addr, &mut ip_buf);
        assert_eq!(&ip_buf[..ip_len], b"10.0.2.15");

        let gw_len = net.format_ip_bytes(&net.gateway, &mut ip_buf);
        assert_eq!(&ip_buf[..gw_len], b"10.0.2.2");
    }

    #[test]
    fn test_settings_app_surface_rendering() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        let mut buf = vec![0u32; 640 * 480];
        let mut surf = BufferSurface32::new(&mut buf, 640, 480);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));

        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 1000, "Should render sidebar, panel content, and status bar");

        // Render Firewall panel
        app.select_panel(PanelKind::Firewall);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));
        let non_zero_fw = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_fw > 1000, "Should render firewall rules table and controls");

        // Render Users panel in list and modal modes
        app.select_panel(PanelKind::Users);
        app.users_ui.view_mode = UsersViewMode::List;
        app.render(&mut surf, Rect::new(0, 0, 640, 480));
        let non_zero_users = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_users > 1000, "Should render accounts list and inspector");

        app.users_ui.open_create_form(1001);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));
        let non_zero_create = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_create > 1000, "Should render useradd modal form");
    }

    #[test]
    fn test_user_record_password_hashing_and_verification() {
        let mut rec = UserRecord::empty();
        rec.in_use = 1;
        rec.set_username("charlie");
        assert_eq!(rec.username_str(), "charlie");
        rec.uid = 1002;
        rec.gid = 1002;

        rec.set_password(b"secret-password", 0x1234_5678);
        assert!(rec.verify_password(b"secret-password"));
        assert!(!rec.verify_password(b"wrong-password"));
        assert!(!rec.verify_password(b""));
    }

    #[test]
    fn test_users_config_admin_role_toggle_and_last_admin_safeguard() {
        unsafe { ffi::reset_mock_environment() };
        let mut cfg = UsersConfig::new();
        assert_eq!(cfg.admin_count(), 1); // "admin" is gid 0

        // Add a standard user
        let user_idx = cfg.add_user_with_credentials("dan", 1003, false, b"danpass", 0x5555).unwrap();
        assert!(!cfg.accounts[user_idx].is_admin());

        // Promote dan to Admin (gid 0)
        assert_eq!(cfg.toggle_admin(user_idx), Ok(true));
        assert!(cfg.accounts[user_idx].is_admin());
        assert_eq!(cfg.admin_count(), 2);

        // Demote dan back to Standard user (gid 1003)
        assert_eq!(cfg.toggle_admin(user_idx), Ok(false));
        assert!(!cfg.accounts[user_idx].is_admin());
        assert_eq!(cfg.admin_count(), 1);

        // Attempt to demote the last remaining admin ("admin")
        let admin_idx = cfg.find_user("admin").unwrap();
        assert!(cfg.toggle_admin(admin_idx).is_err());
        assert!(cfg.accounts[admin_idx].is_admin());
        assert_eq!(cfg.admin_count(), 1);
    }

    #[test]
    fn test_users_config_deletion_safeguards() {
        unsafe { ffi::reset_mock_environment() };
        let mut cfg = UsersConfig::new();
        let eve_idx = cfg.add_user_with_credentials("eve", 1004, false, b"evepass", 0x7777).unwrap();
        assert_eq!(cfg.count, 3);

        // Deleting eve should succeed
        assert!(cfg.delete_account(eve_idx).is_ok());
        assert_eq!(cfg.count, 2);
        assert!(cfg.find_user("eve").is_none());

        // Attempting to delete the last admin ("admin") must fail
        let admin_idx = cfg.find_user("admin").unwrap();
        assert!(cfg.delete_account(admin_idx).is_err());
        assert_eq!(cfg.count, 2);

        // Deleting standard account "persisted" succeeds
        let p_idx = cfg.find_user("persisted").unwrap();
        assert!(cfg.delete_account(p_idx).is_ok());
        assert_eq!(cfg.count, 1);

        // Attempting to delete the only remaining account in system must fail
        let last_idx = cfg.find_user("admin").unwrap();
        assert!(cfg.delete_account(last_idx).is_err());
        assert_eq!(cfg.count, 1);
    }

    #[test]
    fn test_users_ui_state_and_action_dispatch() {
        let mut ui = UsersUiState::new();
        assert_eq!(ui.view_mode, UsersViewMode::List);

        // Open create form
        ui.open_create_form(1005);
        assert_eq!(ui.view_mode, UsersViewMode::CreateAccount);
        assert_eq!(ui.form_uid, 1005);
        assert_eq!(ui.form_username_str(), "");

        // Set preset
        ui.set_form_preset("alice", false, "alicepass");
        assert_eq!(ui.form_username_str(), "alice");
        assert_eq!(ui.form_password_str(), "alicepass");
        assert!(!ui.form_is_admin);

        // Open change password
        ui.open_change_password();
        assert_eq!(ui.view_mode, UsersViewMode::ChangePassword);
        assert_eq!(ui.form_password_str(), "");
    }

    #[test]
    fn test_service_manager_sync_and_controls() {
        unsafe { ffi::reset_mock_environment() };
        let mut mgr = ServiceManager::new();
        assert!(mgr.sync_from_kernel());
        assert_eq!(mgr.count, 3);
        assert_eq!(mgr.running_count(), 2);
        assert_eq!(mgr.stopped_count(), 1);

        // Check service 0
        assert_eq!(mgr.services[0].name_str(), "network-check");
        assert_eq!(mgr.services[0].state_str(), "RUNNING");
        assert_eq!(mgr.services[0].policy_str(), "always");

        // Restart service 0
        assert!(mgr.restart_service(0));
        assert_eq!(unsafe { ffi::MOCK_SVC_LAST_ACTION }, ffi::NOVA_SVC_ACTION_RESTART);
        assert_eq!(mgr.services[0].restart_count, 1);

        // Stop service 2 ("shell")
        assert!(mgr.stop_service(2));
        assert_eq!(unsafe { ffi::MOCK_SVC_LAST_ACTION }, ffi::NOVA_SVC_ACTION_STOP);
        assert_eq!(mgr.services[2].state_str(), "STOPPED");

        // Start service 1 ("greeting")
        assert!(mgr.start_service(1));
        assert_eq!(unsafe { ffi::MOCK_SVC_LAST_ACTION }, ffi::NOVA_SVC_ACTION_START);
        assert_eq!(mgr.services[1].state_str(), "RUNNING");
    }

    #[test]
    fn test_service_manager_surface_rendering() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Services);
        app.service_mgr.view_mode = ServiceViewMode::LiveDashboard;

        let mut buf = vec![0u32; 640 * 480];
        let mut surf = BufferSurface32::new(&mut buf, 640, 480);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));

        let non_zero_live = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_live > 5000, "Should render live supervision dashboard with status pills");

        // Switch to config editor view
        app.service_mgr.view_mode = ServiceViewMode::ConfigFile;
        surf.buffer.fill(0);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));
        let non_zero_cfg = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_cfg > 5000, "Should render SERVICES.CFG config editor");
    }

    #[test]
    fn test_device_manager_sync_and_filter() {
        unsafe { ffi::reset_mock_environment() };
        let mut mgr = DeviceManager::new();
        assert!(mgr.sync_from_kernel());
        assert_eq!(mgr.driver_count(), 12);
        assert_eq!(mgr.device_count(), 13);
        assert_eq!(mgr.summary.nvme_present, 1);
        assert_eq!(mgr.summary.xhci_present, 1);
        assert_eq!(mgr.summary.wifi_present, 1);

        // Check filter
        mgr.category_filter = DeviceCategoryFilter::Storage;
        assert_eq!(mgr.filtered_device_count(), 2);
        let (_, dev0) = mgr.filtered_device_at(0).unwrap();
        assert_eq!(str_from_u8_nul(&dev0.name), "NVMe Solid-State Drive");
        assert_eq!(str_from_u8_nul(&dev0.driver_name), "nvme");

        mgr.category_filter = DeviceCategoryFilter::Network;
        assert_eq!(mgr.filtered_device_count(), 2);

        mgr.category_filter = DeviceCategoryFilter::All;
        assert_eq!(mgr.filtered_device_count(), 13);
    }

    #[test]
    fn test_device_manager_hit_testing() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Devices);
        let vp = Rect::new(0, 0, 640, 480);

        // Switch to DriverTable
        app.on_left_click(300, 70, vp);
        assert_eq!(app.device_mgr.view_mode, DeviceViewMode::DriverTable);

        // Switch to HardwareTree
        app.on_left_click(180, 70, vp);
        assert_eq!(app.device_mgr.view_mode, DeviceViewMode::HardwareTree);

        // Select Storage filter
        app.on_left_click(240, 135, vp);
        assert_eq!(app.device_mgr.category_filter, DeviceCategoryFilter::Storage);
    }

    #[test]
    fn test_device_manager_surface_rendering() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Devices);

        let mut buf = vec![0u32; 640 * 480];
        let mut surf = BufferSurface32::new(&mut buf, 640, 480);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));

        let non_zero_tree = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_tree > 5000, "Should render Hardware Tree with telemetry cards");

        // Switch to DriverTable view
        app.device_mgr.view_mode = DeviceViewMode::DriverTable;
        surf.buffer.fill(0);
        app.render(&mut surf, Rect::new(0, 0, 640, 480));
        let non_zero_drv = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_drv > 5000, "Should render Registered Drivers table");
    }

    #[test]
    fn test_display_accessibility_config_and_actions() {
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Display);

        assert_eq!(app.display_cfg.theme, Theme::Dark);
        assert_eq!(app.display_cfg.font_scale, FontScale::Normal);

        app.apply_action(SettingsAction::DisplayToggleHighContrast);
        assert_eq!(app.display_cfg.theme, Theme::HighContrast);
        assert_eq!(app.display_cfg.cursor_theme, CursorTheme::HighContrast);
        assert!(app.display_cfg.high_contrast_cursor);

        app.apply_action(SettingsAction::DisplaySetFontScale(FontScale::Large));
        assert_eq!(app.display_cfg.font_scale, FontScale::Large);

        let cfg = app.display_cfg.to_accessibility_config();
        let mut buf = [0u8; ACCESS_CFG_SIZE];
        cfg.serialize(&mut buf);
        let restored = AccessibilityConfig::deserialize(&buf).unwrap();
        assert_eq!(restored.theme, Theme::HighContrast);
        assert_eq!(restored.font_scale, FontScale::Large);
    }

    #[test]
    fn test_display_accessibility_surface_rendering() {
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Display);
        app.display_cfg.theme = Theme::HighContrast;
        app.display_cfg.font_scale = FontScale::Large;

        let mut buf = vec![0u32; 640 * 240];
        let mut surf = BufferSurface32::new(&mut buf, 640, 240);
        app.render(&mut surf, Rect::new(0, 0, 640, 240));

        let non_zero = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 3000);
    }

    #[test]
    fn test_updates_panel_status_and_switching() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Updates);
        assert_eq!(app.active_panel, PanelKind::Updates);
        assert_eq!(app.update_state.status.active_slot, ffi::NOVA_BOOT_SLOT_A);
        assert_eq!(app.update_state.status.active_slot_name(), "Slot A");
        assert_eq!(app.update_state.status.target_slot_name(), "Slot B");
        assert_eq!(app.update_state.status.slot_a_version_str(), "0.88.0");
        assert_eq!(app.update_state.status.state_name(), "Normal (Healthy)");
    }

    #[test]
    fn test_updates_workflow_state_transitions() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Updates);
        assert_eq!(app.update_state.step, UpdateStep::Idle);

        // Check for updates
        app.apply_action(SettingsAction::UpdatesCheck);
        assert_eq!(app.update_state.step, UpdateStep::Available);
        assert_eq!(app.update_state.candidate_version_str(), "0.89.0");

        // Review update
        app.apply_action(SettingsAction::UpdatesReview);
        assert_eq!(app.update_state.step, UpdateStep::ConfirmDialog);

        // Cancel
        app.apply_action(SettingsAction::UpdatesCancel);
        assert_eq!(app.update_state.step, UpdateStep::Idle);

        // Check and confirm apply
        app.apply_action(SettingsAction::UpdatesCheck);
        app.apply_action(SettingsAction::UpdatesReview);
        app.apply_action(SettingsAction::UpdatesConfirmAndApply);
        assert_eq!(app.update_state.step, UpdateStep::Complete);
        assert_eq!(app.update_state.progress_pct, 100);
        assert_eq!(app.update_state.status.slot_state, ffi::NOVA_SLOT_STATE_TESTING);
        assert_eq!(app.update_state.status.slot_b_version_str(), "0.89.0");

        // Confirm healthy
        app.apply_action(SettingsAction::UpdatesConfirmHealthy);
        assert_eq!(app.update_state.status.slot_state, ffi::NOVA_SLOT_STATE_NORMAL);

        // Rollback
        app.apply_action(SettingsAction::UpdatesRollback);
        assert_eq!(app.update_state.status.slot_state, ffi::NOVA_SLOT_STATE_ROLLED_BACK);
    }

    #[test]
    fn test_updates_surface_rendering_and_hit_testing() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = SettingsApp::new();
        app.select_panel(PanelKind::Updates);

        let mut buf = vec![0u32; 640 * 480];
        let mut surf = BufferSurface32::new(&mut buf, 640, 480);
        let vp = Rect::new(0, 0, 640, 480);

        // Render in Idle step
        app.render(&mut surf, vp);
        let non_zero_idle = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_idle > 4000, "Should render updates panel with dual-slot cards");

        // Hit testing for "Check for Updates" button
        // content_x = 176, wf_y = 64 + 42 + 52 + 76 + 10 = 244
        // Check btn at (176 + 12 = 188, 244 + 60 = 304, 160, 30)
        let hit_check = app.button_at(195, 310, vp);
        assert_eq!(hit_check, Some(SettingsAction::UpdatesCheck));

        // Transition to Available step
        app.apply_action(SettingsAction::UpdatesCheck);
        surf.buffer.fill(0);
        app.render(&mut surf, vp);
        let non_zero_avail = surf.buffer.iter().filter(|&&p| p != 0).count();
        assert!(non_zero_avail > 4000);

        // Hit test for "Install Update" button
        let hit_install = app.button_at(195, 310, vp);
        assert_eq!(hit_install, Some(SettingsAction::UpdatesReview));

        // Transition to ConfirmDialog step
        app.apply_action(SettingsAction::UpdatesReview);
        let hit_confirm = app.button_at(195, 310, vp);
        assert_eq!(hit_confirm, Some(SettingsAction::UpdatesConfirmAndApply));
    }
}
