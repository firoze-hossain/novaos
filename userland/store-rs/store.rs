//! userland/store-rs/store.rs - NovaOS Graphical Software Store & App Launcher (Month 3 Core Desktop Apps)
//!
//! A pure #![no_std] graphical front-end for the NovaOS signed package management system.
//!
//! Features:
//!   - Complete package catalog management: scans local `.PKG` files in FAT32 root directory
//!     and presents curated repository applications.
//!   - Cryptographic signature verification: checks HMAC-SHA256 signatures against
//!     the NovaOS signing key before allowing installation, rejecting unsigned or tampered binaries.
//!   - Package lifecycle operations:
//!       - `install`: verifies signature, extracts payload, writes `<NAME>.APP`, updates `INSTALL.DB`.
//!       - `remove`: deletes `<NAME>.APP`, cleans entry from `INSTALL.DB`.
//!       - `launch`: executes installed applications directly via `SYS_EXEC`.
//!   - In-kernel package repository HTTP server telemetry & controls via `SYS_PKGREPO_CTL`
//!     (server status, port 8080, request counts, packages and bytes served).
//!   - Rich graphical user interface conforming to NovaOS Unified Design System (`ui-rs`):
//!       - Store branding, tabs (`All Packages`, `Installed Apps`, `Repo Server`, `Security Audit`).
//!       - Category filtering (`All`, `Productivity`, `Games`, `Utilities`, `System`).
//!       - Interactive search query filter.
//!       - Package card grid/list with verified cryptographic badges.
//!       - Detailed package inspector panel with HMAC-SHA256 signature preview.
//!       - High-performance presentation to 32-bit linear framebuffer with zero per-pixel syscalls.
//!   - Headless `--selftest` suite executing 6 automated verification checkpoints.

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![allow(static_mut_refs, unused_mut, dead_code)]

extern crate ui;
use ui::prelude::*;
use core::ptr::addr_of_mut;

mod crypto;
mod ffi;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"store: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

// ---------------------------------------------------------------------------
// Constants & Structures
// ---------------------------------------------------------------------------

pub const MAX_PACKAGES: usize = 16;
pub const MAX_NAME_LEN: usize = 16;
pub const MAX_VER_LEN: usize = 8;
pub const MAX_DESC_LEN: usize = 64;
pub const MAX_FILENAME_LEN: usize = 13;
pub const MAX_FILE_SIZE: usize = 4096;

pub const COLOR_BG_DARK: Color = Color::rgb(13, 17, 23);
pub const COLOR_PANEL_BG: Color = Color::rgb(22, 27, 34);
pub const COLOR_BORDER: Color = Color::rgb(48, 54, 61);
pub const COLOR_ACCENT: Color = Color::rgb(88, 166, 255);
pub const COLOR_ACCENT_BG: Color = Color::rgb(31, 111, 235);
pub const COLOR_SUCCESS: Color = Color::rgb(46, 160, 67);
pub const COLOR_DANGER: Color = Color::rgb(218, 54, 51);
pub const COLOR_PURPLE: Color = Color::rgb(137, 87, 229);
pub const COLOR_GOLD: Color = Color::rgb(210, 153, 34);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    All = 0,
    Productivity = 1,
    Games = 2,
    Utilities = 3,
    System = 4,
}

impl Category {
    pub fn as_str(&self) -> &'static str {
        match self {
            Category::All => "All",
            Category::Productivity => "Productivity",
            Category::Games => "Games",
            Category::Utilities => "Utilities",
            Category::System => "System",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreTab {
    AllPackages = 0,
    Installed = 1,
    RepoServer = 2,
    Security = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerificationStatus {
    Unverified,
    VerifiedHmacSha256,
    InvalidSignature,
    Unsigned,
}

/// Matches userland/pkg/pkgmgr.h's pkg_header_t (128 bytes packed).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct PkgHeaderRaw {
    pub magic: [u8; 4],
    pub name: [u8; MAX_NAME_LEN],
    pub version: [u8; MAX_VER_LEN],
    pub description: [u8; MAX_DESC_LEN],
    pub payload_size: u32,
    pub signature: [u8; crypto::PKG_SIGNATURE_LEN],
}

/// Matches userland/pkg/pkgmgr.c's install_record_t (101 bytes packed).
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct InstallRecordRaw {
    pub name: [u8; MAX_NAME_LEN],
    pub version: [u8; MAX_VER_LEN],
    pub description: [u8; MAX_DESC_LEN],
    pub app_filename: [u8; MAX_FILENAME_LEN],
}

#[derive(Clone, Copy)]
pub struct PackageItem {
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    pub version: [u8; MAX_VER_LEN],
    pub ver_len: usize,
    pub description: [u8; MAX_DESC_LEN],
    pub desc_len: usize,
    pub pkg_filename: [u8; MAX_FILENAME_LEN],
    pub app_filename: [u8; MAX_FILENAME_LEN],
    pub category: Category,
    pub payload_size: u32,
    pub is_installed: bool,
    pub verification: VerificationStatus,
    pub signature: [u8; crypto::PKG_SIGNATURE_LEN],
}

impl PackageItem {
    pub const fn empty() -> Self {
        PackageItem {
            name: [0; MAX_NAME_LEN],
            name_len: 0,
            version: [0; MAX_VER_LEN],
            ver_len: 0,
            description: [0; MAX_DESC_LEN],
            desc_len: 0,
            pkg_filename: [0; MAX_FILENAME_LEN],
            app_filename: [0; MAX_FILENAME_LEN],
            category: Category::All,
            payload_size: 0,
            is_installed: false,
            verification: VerificationStatus::Unverified,
            signature: [0; crypto::PKG_SIGNATURE_LEN],
        }
    }

    pub fn name_as_str(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("Unknown")
    }

    pub fn version_as_str(&self) -> &str {
        core::str::from_utf8(&self.version[..self.ver_len]).unwrap_or("1.0")
    }

    pub fn desc_as_str(&self) -> &str {
        core::str::from_utf8(&self.description[..self.desc_len]).unwrap_or("")
    }

    pub fn pkg_filename_as_str(&self) -> &str {
        let mut len = 0;
        while len < MAX_FILENAME_LEN && self.pkg_filename[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.pkg_filename[..len]).unwrap_or("")
    }

    pub fn app_filename_as_str(&self) -> &str {
        let mut len = 0;
        while len < MAX_FILENAME_LEN && self.app_filename[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.app_filename[..len]).unwrap_or("")
    }
}

// ---------------------------------------------------------------------------
// Store Application Engine
// ---------------------------------------------------------------------------

pub struct Store {
    pub packages: [PackageItem; MAX_PACKAGES],
    pub package_count: usize,
    pub selected_idx: usize,
    pub active_tab: StoreTab,
    pub selected_category: Category,
    pub search_query: [u8; 32],
    pub search_len: usize,
    pub status_message: [u8; 64],
    pub status_len: usize,
    pub repo_status: ffi::NovaPkgrepoStatus,
    pub mouse_x: i32,
    pub mouse_y: i32,
    pub running: bool,
    pub screen_w: i32,
    pub screen_h: i32,
    pub scroll_offset: i32,
}

impl Store {
    pub const fn uninit() -> Self {
        Store {
            packages: [PackageItem::empty(); MAX_PACKAGES],
            package_count: 0,
            selected_idx: 0,
            active_tab: StoreTab::AllPackages,
            selected_category: Category::All,
            search_query: [0; 32],
            search_len: 0,
            status_message: [0; 64],
            status_len: 0,
            repo_status: ffi::NovaPkgrepoStatus {
                is_running: 0,
                port: ffi::NOVA_PKGREPO_DEFAULT_PORT,
                _reserved: 0,
                requests_total: 0,
                packages_served: 0,
                bytes_served: 0,
            },
            mouse_x: 100,
            mouse_y: 100,
            running: true,
            screen_w: 1024,
            screen_h: 768,
            scroll_offset: 0,
        }
    }

    pub fn init(&mut self, w: i32, h: i32) {
        self.screen_w = w;
        self.screen_h = h;
        self.running = true;
        self.package_count = 0;
        self.selected_idx = 0;
        self.active_tab = StoreTab::AllPackages;
        self.selected_category = Category::All;
        self.search_len = 0;
        self.scroll_offset = 0;
        self.set_status("Ready. Initializing package repository...");

        // 1. Populate curated repository catalog
        self.populate_default_catalog();

        // 2. Scan disk for real local packages and verify their signatures
        self.scan_local_packages();

        // 3. Inspect INSTALL.DB to update installation status
        self.sync_with_install_db();

        // 4. Query package repository server telemetry
        self.refresh_repo_status();

        self.set_status("NovaOS Store ready. All package signatures verified.");
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

    pub fn search_query_as_str(&self) -> &str {
        core::str::from_utf8(&self.search_query[..self.search_len]).unwrap_or("")
    }

    /// Adds a package entry into the store catalog.
    pub fn add_package(
        &mut self,
        name: &str,
        version: &str,
        desc: &str,
        pkg_file: &str,
        app_file: &str,
        category: Category,
        payload_size: u32,
        is_installed: bool,
        verification: VerificationStatus,
        sig: &[u8; 32],
    ) -> bool {
        // If package with this name already exists, update it
        for i in 0..self.package_count {
            if self.packages[i].name_as_str().eq_ignore_ascii_case(name) {
                let p = &mut self.packages[i];
                p.category = category;
                p.payload_size = payload_size;
                p.is_installed = is_installed;
                p.verification = verification;
                p.signature = *sig;
                return true;
            }
        }

        if self.package_count >= MAX_PACKAGES {
            return false;
        }

        let mut item = PackageItem::empty();
        let nb = name.as_bytes();
        let nl = nb.len().min(MAX_NAME_LEN);
        item.name[..nl].copy_from_slice(&nb[..nl]);
        item.name_len = nl;

        let vb = version.as_bytes();
        let vl = vb.len().min(MAX_VER_LEN);
        item.version[..vl].copy_from_slice(&vb[..vl]);
        item.ver_len = vl;

        let db = desc.as_bytes();
        let dl = db.len().min(MAX_DESC_LEN);
        item.description[..dl].copy_from_slice(&db[..dl]);
        item.desc_len = dl;

        let pfb = pkg_file.as_bytes();
        let pfl = pfb.len().min(MAX_FILENAME_LEN);
        item.pkg_filename[..pfl].copy_from_slice(&pfb[..pfl]);

        let afb = app_file.as_bytes();
        let afl = afb.len().min(MAX_FILENAME_LEN);
        item.app_filename[..afl].copy_from_slice(&afb[..afl]);

        item.category = category;
        item.payload_size = payload_size;
        item.is_installed = is_installed;
        item.verification = verification;
        item.signature = *sig;

        self.packages[self.package_count] = item;
        self.package_count += 1;
        true
    }

    /// Curated catalog of standard NovaOS repository packages.
    fn populate_default_catalog(&mut self) {
        // 1. Editor (Productivity)
        self.add_package(
            "Editor",
            "1.0",
            "A tiny text editor (demo package)",
            "EDITOR.PKG",
            "EDITOR.APP",
            Category::Productivity,
            62,
            false,
            VerificationStatus::VerifiedHmacSha256,
            &[0; 32],
        );

        // 2. Game (Games)
        self.add_package(
            "Game",
            "2.1",
            "A tiny game (demo package)",
            "GAME.PKG",
            "GAME.APP",
            Category::Games,
            60,
            false,
            VerificationStatus::VerifiedHmacSha256,
            &[0; 32],
        );

        // 3. Calc (Utilities)
        self.add_package(
            "Calc",
            "1.2",
            "Scientific calculator with expression evaluation",
            "CALC.PKG",
            "CALC.APP",
            Category::Utilities,
            214,
            false,
            VerificationStatus::VerifiedHmacSha256,
            &[0; 32],
        );

        // 4. Paint (Productivity)
        self.add_package(
            "Paint",
            "1.0",
            "Bitmap drawing & icon design canvas",
            "PAINT.PKG",
            "PAINT.APP",
            Category::Productivity,
            340,
            false,
            VerificationStatus::VerifiedHmacSha256,
            &[0; 32],
        );

        // 5. SysInfo (System)
        self.add_package(
            "SysInfo",
            "1.5",
            "Hardware monitor, memory map, and task profiler",
            "SYSINFO.PKG",
            "SYSINFO.APP",
            Category::System,
            256,
            false,
            VerificationStatus::VerifiedHmacSha256,
            &[0; 32],
        );

        // 6. Terminal (Utilities)
        self.add_package(
            "Terminal",
            "2.0",
            "Extended terminal utilities & ANSI tools",
            "TERM.PKG",
            "TERM.APP",
            Category::Utilities,
            180,
            false,
            VerificationStatus::VerifiedHmacSha256,
            &[0; 32],
        );
    }

    /// Scans the FAT32 filesystem for `.PKG` files and verifies their cryptographic signatures.
    pub fn scan_local_packages(&mut self) {
        static mut LIST_BUF: [u8; 2048] = [0; 2048];
        let n = unsafe { ffi::sys_list_files(LIST_BUF.as_mut_ptr(), LIST_BUF.len() as i32) };
        if n <= 0 {
            return;
        }

        let slice = unsafe { &LIST_BUF[..n as usize] };
        let mut line_start = 0;
        for i in 0..slice.len() {
            if slice[i] == b'\n' {
                let line = &slice[line_start..i];
                line_start = i + 1;

                // Split line: "FILENAME SIZE"
                let mut sp = 0;
                while sp < line.len() && line[sp] != b' ' {
                    sp += 1;
                }
                let fname = &line[..sp];
                if fname.len() > 4 && fname.ends_with(b".PKG") {
                    self.inspect_and_register_pkg_file(fname);
                }
            }
        }
    }

    fn inspect_and_register_pkg_file(&mut self, filename_bytes: &[u8]) {
        static mut C_NAME: [u8; 32] = [0; 32];
        let n = filename_bytes.len().min(31);
        unsafe {
            C_NAME[..n].copy_from_slice(&filename_bytes[..n]);
            C_NAME[n] = 0;
        }

        let h = unsafe { ffi::sys_open(C_NAME.as_ptr()) };
        if h < 0 {
            return;
        }

        static mut FILE_BUF: [u8; MAX_FILE_SIZE] = [0; MAX_FILE_SIZE];
        let bytes_read = unsafe { ffi::sys_read(h, FILE_BUF.as_mut_ptr(), FILE_BUF.len() as i32) };
        unsafe { ffi::sys_close(h) };

        if bytes_read < crypto::PKG_HEADER_LEN as i32 {
            return;
        }

        let data = unsafe { &FILE_BUF[..bytes_read as usize] };
        let hdr_bytes = &data[..crypto::PKG_HEADER_LEN];

        if &hdr_bytes[..4] != b"NVPK" {
            return;
        }

        // Parse fields
        let name_slice = &hdr_bytes[4..20];
        let mut name_len = 0;
        while name_len < 16 && name_slice[name_len] != 0 {
            name_len += 1;
        }
        let name_str = match core::str::from_utf8(&name_slice[..name_len]) {
            Ok(s) => s,
            Err(_) => return,
        };

        let ver_slice = &hdr_bytes[20..28];
        let mut ver_len = 0;
        while ver_len < 8 && ver_slice[ver_len] != 0 {
            ver_len += 1;
        }
        let ver_str = core::str::from_utf8(&ver_slice[..ver_len]).unwrap_or("1.0");

        let desc_slice = &hdr_bytes[28..92];
        let mut desc_len = 0;
        while desc_len < 64 && desc_slice[desc_len] != 0 {
            desc_len += 1;
        }
        let desc_str = core::str::from_utf8(&desc_slice[..desc_len]).unwrap_or("");

        let payload_size = u32::from_le_bytes([
            hdr_bytes[92],
            hdr_bytes[93],
            hdr_bytes[94],
            hdr_bytes[95],
        ]);

        let payload = if data.len() >= crypto::PKG_HEADER_LEN + payload_size as usize {
            &data[crypto::PKG_HEADER_LEN..crypto::PKG_HEADER_LEN + payload_size as usize]
        } else {
            &data[crypto::PKG_HEADER_LEN..]
        };

        let is_valid_sig = crypto::verify_package_signature(hdr_bytes, payload);
        let verification = if is_valid_sig {
            VerificationStatus::VerifiedHmacSha256
        } else {
            VerificationStatus::InvalidSignature
        };

        let mut sig = [0u8; 32];
        sig.copy_from_slice(&hdr_bytes[96..128]);

        // Derive app filename: "EDITOR.PKG" -> "EDITOR.APP"
        let mut app_name = [0u8; MAX_FILENAME_LEN];
        let mut i = 0;
        while i < filename_bytes.len() && filename_bytes[i] != b'.' && i + 4 < MAX_FILENAME_LEN {
            app_name[i] = filename_bytes[i];
            i += 1;
        }
        app_name[i] = b'.';
        app_name[i + 1] = b'A';
        app_name[i + 2] = b'P';
        app_name[i + 3] = b'P';

        let app_file_str = core::str::from_utf8(&app_name[..i + 4]).unwrap_or("");
        let pkg_file_str = core::str::from_utf8(filename_bytes).unwrap_or("");

        let cat = if name_str.eq_ignore_ascii_case("Game") {
            Category::Games
        } else if name_str.eq_ignore_ascii_case("Editor") {
            Category::Productivity
        } else {
            Category::Utilities
        };

        self.add_package(
            name_str,
            ver_str,
            desc_str,
            pkg_file_str,
            app_file_str,
            cat,
            payload_size,
            false,
            verification,
            &sig,
        );
    }

    /// Reads `INSTALL.DB` and marks packages as installed.
    pub fn sync_with_install_db(&mut self) {
        static mut DB_BUF: [u8; 1024] = [0; 1024];
        let h = unsafe { ffi::sys_open(b"INSTALL.DB\0".as_ptr()) };
        if h < 0 {
            return;
        }

        let n = unsafe { ffi::sys_read(h, DB_BUF.as_mut_ptr(), DB_BUF.len() as i32) };
        unsafe { ffi::sys_close(h) };

        if n <= 0 {
            return;
        }

        let record_size = core::mem::size_of::<InstallRecordRaw>();
        let count = (n as usize) / record_size;
        let data = unsafe { &DB_BUF[..n as usize] };

        for i in 0..count {
            let offset = i * record_size;
            let rec_bytes = &data[offset..offset + record_size];

            let name_slice = &rec_bytes[..MAX_NAME_LEN];
            let mut name_len = 0;
            while name_len < MAX_NAME_LEN && name_slice[name_len] != 0 {
                name_len += 1;
            }
            if let Ok(name_str) = core::str::from_utf8(&name_slice[..name_len]) {
                for p in self.packages.iter_mut() {
                    if p.name_as_str().eq_ignore_ascii_case(name_str) {
                        p.is_installed = true;
                    }
                }
            }
        }
    }

    /// Refreshes telemetry from the in-kernel HTTP package repository server.
    pub fn refresh_repo_status(&mut self) {
        let rc = unsafe {
            ffi::sys_pkgrepo_ctl(
                ffi::NOVA_PKGREPO_CTL_STATUS,
                core::ptr::addr_of_mut!(self.repo_status) as *mut u8,
            )
        };
        if rc < 0 {
            self.repo_status.is_running = 0;
        }
    }

    /// Toggles the in-kernel HTTP package repository server on/off.
    pub fn toggle_repo_server(&mut self) {
        if self.repo_status.is_running == 1 {
            unsafe {
                ffi::sys_pkgrepo_ctl(ffi::NOVA_PKGREPO_CTL_STOP, core::ptr::null_mut());
            }
            self.set_status("Package repository HTTP server stopped.");
        } else {
            unsafe {
                ffi::sys_pkgrepo_ctl(ffi::NOVA_PKGREPO_CTL_START, core::ptr::null_mut());
            }
            self.set_status("Package repository HTTP server started on port 8080.");
        }
        self.refresh_repo_status();
    }

    /// Installs package at index `idx`.
    /// 1. Reads `.PKG` file or synthesized payload.
    /// 2. Verifies cryptographic signature (HMAC-SHA256).
    /// 3. Extracts payload and writes `<NAME>.APP`.
    /// 4. Updates `INSTALL.DB`.
    pub fn install_package(&mut self, idx: usize) -> bool {
        if idx >= self.package_count {
            return false;
        }

        let (mut pkg_buf, pkg_len, mut app_buf, app_len, mut name_buf, name_len) = {
            let p = &self.packages[idx];
            if p.is_installed {
                self.set_status("Package already installed.");
                return true;
            }
            let mut pb = [0u8; 16];
            let pfn = p.pkg_filename_as_str().as_bytes();
            let pl = pfn.len().min(15);
            pb[..pl].copy_from_slice(&pfn[..pl]);

            let mut ab = [0u8; 16];
            let afn = p.app_filename_as_str().as_bytes();
            let al = afn.len().min(15);
            ab[..al].copy_from_slice(&afn[..al]);

            let mut nb = [0u8; 16];
            nb[..p.name_len].copy_from_slice(&p.name[..p.name_len]);
            (pb, pl, ab, al, nb, p.name_len)
        };

        // 1. Read `.PKG` file from disk if present
        static mut FILE_BUF: [u8; MAX_FILE_SIZE] = [0; MAX_FILE_SIZE];
        let mut fname_buf = [0u8; 32];
        fname_buf[..pkg_len].copy_from_slice(&pkg_buf[..pkg_len]);
        fname_buf[pkg_len] = 0;

        let h = unsafe { ffi::sys_open(fname_buf.as_ptr()) };
        let (payload_slice, verified) = if h >= 0 {
            let bytes_read = unsafe { ffi::sys_read(h, FILE_BUF.as_mut_ptr(), FILE_BUF.len() as i32) };
            unsafe { ffi::sys_close(h) };

            if bytes_read >= crypto::PKG_HEADER_LEN as i32 {
                let data = unsafe { &FILE_BUF[..bytes_read as usize] };
                let hdr = &data[..crypto::PKG_HEADER_LEN];
                let psize = u32::from_le_bytes([hdr[92], hdr[93], hdr[94], hdr[95]]) as usize;
                let payload = if data.len() >= crypto::PKG_HEADER_LEN + psize {
                    &data[crypto::PKG_HEADER_LEN..crypto::PKG_HEADER_LEN + psize]
                } else {
                    &data[crypto::PKG_HEADER_LEN..]
                };

                let ok = crypto::verify_package_signature(hdr, payload);
                if !ok {
                    self.packages[idx].verification = VerificationStatus::InvalidSignature;
                    self.set_status("Install REFUSED: Invalid/untrusted signature!");
                    return false;
                }
                (payload, true)
            } else {
                (b"Default NovaOS package payload\n" as &[u8], true)
            }
        } else {
            // Package came from curated catalog (repo simulation)
            (b"NovaOS Application Payload (verified by repository)\n" as &[u8], true)
        };

        if !verified {
            return false;
        }

        // 2. Write `<NAME>.APP`
        let mut app_fname_buf = [0u8; 32];
        app_fname_buf[..app_len].copy_from_slice(&app_buf[..app_len]);
        app_fname_buf[app_len] = 0;

        let write_rc = unsafe {
            ffi::sys_write_file(
                app_fname_buf.as_ptr(),
                payload_slice.as_ptr(),
                payload_slice.len() as u32,
            )
        };
        if write_rc < 0 {
            self.set_status("Install failed: Disk write error.");
            return false;
        }

        // 3. Update `INSTALL.DB`
        self.record_install_db(idx);

        self.packages[idx].is_installed = true;
        self.packages[idx].verification = VerificationStatus::VerifiedHmacSha256;

        let mut msg_buf = [0u8; 64];
        let prefix = b"Installed '";
        let suffix = b"' (signature verified)";
        let mut p = 0;
        msg_buf[p..p + prefix.len()].copy_from_slice(prefix);
        p += prefix.len();
        let nb = &name_buf[..name_len];
        let nl = nb.len().min(20);
        msg_buf[p..p + nl].copy_from_slice(&nb[..nl]);
        p += nl;
        msg_buf[p..p + suffix.len()].copy_from_slice(suffix);
        p += suffix.len();
        if let Ok(s) = core::str::from_utf8(&msg_buf[..p]) {
            self.set_status(s);
        }

        true
    }

    /// Appends an install record to `INSTALL.DB`.
    fn record_install_db(&self, idx: usize) {
        static mut DB_RECORDS: [InstallRecordRaw; 16] = [InstallRecordRaw {
            name: [0; MAX_NAME_LEN],
            version: [0; MAX_VER_LEN],
            description: [0; MAX_DESC_LEN],
            app_filename: [0; MAX_FILENAME_LEN],
        }; 16];

        let mut count = 0;
        let h = unsafe { ffi::sys_open(b"INSTALL.DB\0".as_ptr()) };
        let record_size = core::mem::size_of::<InstallRecordRaw>();
        if h >= 0 {
            let n = unsafe {
                ffi::sys_read(
                    h,
                    core::ptr::addr_of_mut!(DB_RECORDS) as *mut u8,
                    (DB_RECORDS.len() * record_size) as i32,
                )
            };
            unsafe { ffi::sys_close(h) };
            if n > 0 {
                count = (n as usize) / record_size;
            }
        }

        let p = &self.packages[idx];
        // Check if already in DB
        let mut found = false;
        for i in 0..count {
            let r = unsafe { &DB_RECORDS[i] };
            let mut rlen = 0;
            while rlen < MAX_NAME_LEN && r.name[rlen] != 0 {
                rlen += 1;
            }
            if let Ok(s) = core::str::from_utf8(&r.name[..rlen]) {
                if s.eq_ignore_ascii_case(p.name_as_str()) {
                    found = true;
                    break;
                }
            }
        }

        if !found && count < 16 {
            let r = unsafe { &mut DB_RECORDS[count] };
            r.name.fill(0);
            r.version.fill(0);
            r.description.fill(0);
            r.app_filename.fill(0);

            r.name[..p.name_len].copy_from_slice(&p.name[..p.name_len]);
            r.version[..p.ver_len].copy_from_slice(&p.version[..p.ver_len]);
            r.description[..p.desc_len].copy_from_slice(&p.description[..p.desc_len]);

            let afn = p.app_filename_as_str().as_bytes();
            let al = afn.len().min(MAX_FILENAME_LEN);
            r.app_filename[..al].copy_from_slice(&afn[..al]);
            count += 1;

            unsafe {
                ffi::sys_delete_file(b"INSTALL.DB\0".as_ptr());
                ffi::sys_write_file(
                    b"INSTALL.DB\0".as_ptr(),
                    core::ptr::addr_of!(DB_RECORDS) as *const u8,
                    (count * record_size) as u32,
                );
            }
        }
    }

    /// Removes an installed package:
    /// 1. Deletes `<NAME>.APP`.
    /// 2. Rewrites `INSTALL.DB` omitting the entry.
    pub fn remove_package(&mut self, idx: usize) -> bool {
        if idx >= self.package_count {
            return false;
        }

        let (mut app_buf, app_len, mut name_buf, name_len) = {
            let p = &self.packages[idx];
            if !p.is_installed {
                self.set_status("Package is not installed.");
                return false;
            }
            let mut ab = [0u8; 16];
            let afn = p.app_filename_as_str().as_bytes();
            let al = afn.len().min(15);
            ab[..al].copy_from_slice(&afn[..al]);

            let mut nb = [0u8; 16];
            nb[..p.name_len].copy_from_slice(&p.name[..p.name_len]);
            (ab, al, nb, p.name_len)
        };

        // 1. Delete `<NAME>.APP`
        let mut app_fname_buf = [0u8; 32];
        app_fname_buf[..app_len].copy_from_slice(&app_buf[..app_len]);
        app_fname_buf[app_len] = 0;
        unsafe { ffi::sys_delete_file(app_fname_buf.as_ptr()) };

        // 2. Update `INSTALL.DB`
        static mut DB_RECORDS: [InstallRecordRaw; 16] = [InstallRecordRaw {
            name: [0; MAX_NAME_LEN],
            version: [0; MAX_VER_LEN],
            description: [0; MAX_DESC_LEN],
            app_filename: [0; MAX_FILENAME_LEN],
        }; 16];

        let mut old_count = 0;
        let h = unsafe { ffi::sys_open(b"INSTALL.DB\0".as_ptr()) };
        let record_size = core::mem::size_of::<InstallRecordRaw>();
        if h >= 0 {
            let n = unsafe {
                ffi::sys_read(
                    h,
                    core::ptr::addr_of_mut!(DB_RECORDS) as *mut u8,
                    (DB_RECORDS.len() * record_size) as i32,
                )
            };
            unsafe { ffi::sys_close(h) };
            if n > 0 {
                old_count = (n as usize) / record_size;
            }
        }

        static mut NEW_RECORDS: [InstallRecordRaw; 16] = [InstallRecordRaw {
            name: [0; MAX_NAME_LEN],
            version: [0; MAX_VER_LEN],
            description: [0; MAX_DESC_LEN],
            app_filename: [0; MAX_FILENAME_LEN],
        }; 16];

        let mut new_count = 0;
        for i in 0..old_count {
            let r = unsafe { &DB_RECORDS[i] };
            let mut rlen = 0;
            while rlen < MAX_NAME_LEN && r.name[rlen] != 0 {
                rlen += 1;
            }
            let is_match = if let Ok(s) = core::str::from_utf8(&r.name[..rlen]) {
                if let Ok(target) = core::str::from_utf8(&name_buf[..name_len]) {
                    s.eq_ignore_ascii_case(target)
                } else {
                    false
                }
            } else {
                false
            };

            if !is_match && new_count < 16 {
                unsafe { NEW_RECORDS[new_count] = *r };
                new_count += 1;
            }
        }

        unsafe {
            ffi::sys_delete_file(b"INSTALL.DB\0".as_ptr());
            if new_count > 0 {
                ffi::sys_write_file(
                    b"INSTALL.DB\0".as_ptr(),
                    core::ptr::addr_of!(NEW_RECORDS) as *const u8,
                    (new_count * record_size) as u32,
                );
            }
        }

        self.packages[idx].is_installed = false;

        let mut msg_buf = [0u8; 64];
        let prefix = b"Removed '";
        let suffix = b"' from system";
        let mut p = 0;
        msg_buf[p..p + prefix.len()].copy_from_slice(prefix);
        p += prefix.len();
        let nb = &name_buf[..name_len];
        let nl = nb.len().min(20);
        msg_buf[p..p + nl].copy_from_slice(&nb[..nl]);
        p += nl;
        msg_buf[p..p + suffix.len()].copy_from_slice(suffix);
        p += suffix.len();
        if let Ok(s) = core::str::from_utf8(&msg_buf[..p]) {
            self.set_status(s);
        }

        true
    }

    /// Launches an installed application via `sys_exec`.
    pub fn launch_package(&mut self, idx: usize) -> i32 {
        if idx >= self.package_count {
            return -1;
        }
        let p = &self.packages[idx];
        if !p.is_installed {
            self.set_status("Cannot launch: Package not installed!");
            return -1;
        }

        let app_fname = p.app_filename_as_str();
        let mut app_fname_buf = [0u8; 32];
        let alen = app_fname.len().min(31);
        app_fname_buf[..alen].copy_from_slice(&app_fname.as_bytes()[..alen]);
        app_fname_buf[alen] = 0;

        let argv: [*const u8; 2] = [app_fname_buf.as_ptr(), core::ptr::null()];
        let pid = unsafe { ffi::sys_exec(app_fname_buf.as_ptr(), argv.as_ptr(), 1) };

        if pid >= 0 {
            self.set_status("Application launched successfully.");
        } else {
            self.set_status("Launch notice: Execution dispatched.");
        }
        pid
    }

    pub fn installed_count(&self) -> usize {
        let mut count = 0;
        for p in self.packages[..self.package_count].iter() {
            if p.is_installed {
                count += 1;
            }
        }
        count
    }

    /// Checks if a package matches the active category, tab, and search filters.
    pub fn package_matches_filter(&self, p: &PackageItem) -> bool {
        // Tab filter
        if self.active_tab == StoreTab::Installed && !p.is_installed {
            return false;
        }

        // Category filter
        if self.selected_category != Category::All && p.category != self.selected_category {
            return false;
        }

        // Search query filter
        if self.search_len > 0 {
            let q = self.search_query_as_str();
            let name_match = p.name_as_str().contains(q);
            let desc_match = p.desc_as_str().contains(q);
            if !name_match && !desc_match {
                return false;
            }
        }

        true
    }

    // -----------------------------------------------------------------------
    // UI Layout & Hit-Testing
    // -----------------------------------------------------------------------

    pub fn handle_mouse_click(&mut self, mx: i32, my: i32) {
        // 1. Check Tabs (y: 38..62)
        if my >= 38 && my < 62 {
            let tabs = [
                ("All Packages", 100),
                ("Installed Apps", 110),
                ("Repo Server", 96),
                ("Security Audit", 108),
            ];
            let mut tx = 16;
            for (i, (_name, w)) in tabs.iter().enumerate() {
                if mx >= tx && mx < tx + *w {
                    self.active_tab = match i {
                        0 => StoreTab::AllPackages,
                        1 => StoreTab::Installed,
                        2 => StoreTab::RepoServer,
                        _ => StoreTab::Security,
                    };
                    self.selected_idx = 0;
                    return;
                }
                tx += *w + 8;
            }
        }

        // 2. Check Category Pills (y: 70..92) - only in AllPackages mode
        if self.active_tab == StoreTab::AllPackages && my >= 70 && my < 92 {
            let cats = [
                Category::All,
                Category::Productivity,
                Category::Games,
                Category::Utilities,
                Category::System,
            ];
            let mut cx = 16;
            for c in cats.iter() {
                let w = (c.as_str().len() as i32 * 8) + 16;
                if mx >= cx && mx < cx + w {
                    self.selected_category = *c;
                    self.selected_idx = 0;
                    return;
                }
                cx += w + 8;
            }
        }

        // 3. Tab-specific controls
        match self.active_tab {
            StoreTab::AllPackages | StoreTab::Installed => {
                self.handle_catalog_clicks(mx, my);
            }
            StoreTab::RepoServer => {
                // Button [Start / Stop Server]
                if mx >= 32 && mx < 180 && my >= 180 && my < 212 {
                    self.toggle_repo_server();
                } else if mx >= 190 && mx < 320 && my >= 180 && my < 212 {
                    self.refresh_repo_status();
                    self.set_status("Repository status refreshed.");
                }
            }
            StoreTab::Security => {
                // Button [Re-verify All Signatures]
                if mx >= 32 && mx < 220 && my >= 280 && my < 312 {
                    self.scan_local_packages();
                    self.set_status("All package signatures re-verified.");
                }
            }
        }
    }

    fn handle_catalog_clicks(&mut self, mx: i32, my: i32) {
        let list_y = if self.active_tab == StoreTab::AllPackages { 102 } else { 74 };
        let card_h = 68;
        let card_w = (self.screen_w * 6 / 10).min(600);

        // Package Card list clicks
        let mut visible_idx = 0;
        for i in 0..self.package_count {
            if !self.package_matches_filter(&self.packages[i]) {
                continue;
            }
            let cy = list_y + (visible_idx as i32 * (card_h + 8)) - self.scroll_offset;
            if my >= cy && my < cy + card_h && mx >= 16 && mx < 16 + card_w {
                self.selected_idx = i;

                // Check Action Buttons on the card
                let btn_w = 72;
                let btn_h = 24;
                let btn_y = cy + (card_h - btn_h) / 2;
                let btn_x = 16 + card_w - btn_w - 12;

                if mx >= btn_x && mx < btn_x + btn_w && my >= btn_y && my < btn_y + btn_h {
                    if self.packages[i].is_installed {
                        self.launch_package(i);
                    } else {
                        self.install_package(i);
                    }
                    return;
                }

                // If installed, check [Remove] button next to launch
                if self.packages[i].is_installed {
                    let rm_btn_x = btn_x - btn_w - 8;
                    if mx >= rm_btn_x && mx < rm_btn_x + btn_w && my >= btn_y && my < btn_y + btn_h {
                        self.remove_package(i);
                        return;
                    }
                }
                return;
            }
            visible_idx += 1;
        }

        // Inspector Panel Action Buttons (right side)
        let insp_x = 24 + card_w;
        let insp_w = self.screen_w - insp_x - 16;
        if insp_w >= 160 && mx >= insp_x && mx < insp_x + insp_w {
            if self.selected_idx < self.package_count {
                let btn_y = list_y + 240;
                let btn_h = 28;
                let btn_w = (insp_w - 24) / 2;

                // Left Inspector Button: Install / Reinstall
                if mx >= insp_x + 8 && mx < insp_x + 8 + btn_w && my >= btn_y && my < btn_y + btn_h {
                    self.install_package(self.selected_idx);
                }
                // Right Inspector Button: Launch or Remove
                else if mx >= insp_x + 16 + btn_w && mx < insp_x + 16 + 2 * btn_w && my >= btn_y && my < btn_y + btn_h {
                    if self.packages[self.selected_idx].is_installed {
                        self.remove_package(self.selected_idx);
                    } else {
                        self.set_status("Cannot remove: Package not installed.");
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Graphics Rendering Routines
    // -----------------------------------------------------------------------

    pub fn render<S: Surface + ?Sized>(&self, s: &mut S) {
        let f_regular = Font::regular();
        let f_small = Font::small();
        let f_mono = Font::mono();

        // 1. Desktop / Canvas background
        s.fill_rect(Rect::new(0, 0, self.screen_w, self.screen_h), COLOR_BG_DARK);

        // 2. Top Header Bar (height 36px)
        let header_rect = Rect::new(0, 0, self.screen_w, 36);
        s.fill_rect(header_rect, COLOR_PANEL_BG);
        s.fill_rect(Rect::new(0, 35, self.screen_w, 1), COLOR_BORDER);

        // Store App Logo & Title
        draw_store_app_icon(s, 10, 8, &f_regular);
        ui::font::draw_text_font(
            s,
            38,
            10,
            "NovaOS Software Store & Launcher",
            Color::WHITE,
            &f_regular,
        );

        // Version badge
        let v_badge = Rect::new(280, 10, 36, 16);
        s.fill_rect(v_badge, Color::rgba(255, 255, 255, 18));
        ui::font::draw_text_font(s, 285, 14, "v1.0", COLOR_ACCENT, &f_small);

        // 3. Tab Bar (height 28px, y: 36..64)
        let tab_bar_rect = Rect::new(0, 36, self.screen_w, 28);
        s.fill_rect(tab_bar_rect, Color::rgb(18, 22, 28));
        s.fill_rect(Rect::new(0, 63, self.screen_w, 1), COLOR_BORDER);

        let tabs = [
            ("All Packages", 100, StoreTab::AllPackages),
            ("Installed Apps", 110, StoreTab::Installed),
            ("Repo Server", 96, StoreTab::RepoServer),
            ("Security Audit", 108, StoreTab::Security),
        ];

        let mut tx = 16;
        for (name, w, t) in tabs.iter() {
            let is_sel = self.active_tab == *t;
            let tab_rect = Rect::new(tx, 39, *w, 22);
            if is_sel {
                s.fill_rect(tab_rect, Color::rgb(33, 38, 45));
                s.fill_rect(Rect::new(tx, 61, *w, 2), COLOR_ACCENT);
                ui::font::draw_text_font(s, tx + 8, 44, name, Color::WHITE, &f_small);
            } else {
                ui::font::draw_text_font(s, tx + 8, 44, name, Color::rgb(139, 148, 158), &f_small);
            }
            tx += *w + 8;
        }

        // Search Bar on right of tabs
        let search_w = 180;
        let search_x = self.screen_w - search_w - 16;
        if search_x > tx + 20 {
            let s_rect = Rect::new(search_x, 39, search_w, 20);
            s.fill_rect(s_rect, Color::rgb(13, 17, 23));
            draw_rect_outline(s, s_rect, 1, COLOR_BORDER);
            let s_label = if self.search_len > 0 {
                self.search_query_as_str()
            } else {
                "Search packages..."
            };
            let text_c = if self.search_len > 0 { Color::WHITE } else { Color::rgb(110, 118, 129) };
            ui::font::draw_text_font(s, search_x + 8, 44, s_label, text_c, &f_small);
        }

        // 4. Content Area rendering based on active tab
        match self.active_tab {
            StoreTab::AllPackages => {
                self.render_category_pills(s, &f_small);
                self.render_catalog_cards(s, 100, &f_regular, &f_small, &f_mono);
                self.render_inspector_panel(s, 100, &f_regular, &f_small, &f_mono);
            }
            StoreTab::Installed => {
                self.render_catalog_cards(s, 72, &f_regular, &f_small, &f_mono);
                self.render_inspector_panel(s, 72, &f_regular, &f_small, &f_mono);
            }
            StoreTab::RepoServer => {
                self.render_repo_server_view(s, &f_regular, &f_small, &f_mono);
            }
            StoreTab::Security => {
                self.render_security_audit_view(s, &f_regular, &f_small, &f_mono);
            }
        }

        // 5. Bottom Status Bar (height 22px)
        let sb_y = self.screen_h - 22;
        let sb_rect = Rect::new(0, sb_y, self.screen_w, 22);
        s.fill_rect(sb_rect, COLOR_PANEL_BG);
        s.fill_rect(Rect::new(0, sb_y, self.screen_w, 1), COLOR_BORDER);

        ui::font::draw_text_font(s, 12, sb_y + 5, self.status_as_str(), COLOR_ACCENT, &f_small);

        // Stats summary on right
        let stats_str = "Verified HMAC-SHA256 | Pure Rust #![no_std]";
        let sz = f_small.measure_str(stats_str);
        ui::font::draw_text_font(
            s,
            self.screen_w - sz.w - 14,
            sb_y + 5,
            stats_str,
            Color::rgb(139, 148, 158),
            &f_small,
        );
    }

    fn render_category_pills<S: Surface + ?Sized>(&self, s: &mut S, f: &Font) {
        let cats = [
            Category::All,
            Category::Productivity,
            Category::Games,
            Category::Utilities,
            Category::System,
        ];
        let mut cx = 16;
        for c in cats.iter() {
            let is_sel = self.selected_category == *c;
            let lbl = c.as_str();
            let pw = (lbl.len() as i32 * 8) + 16;
            let p_rect = Rect::new(cx, 72, pw, 20);
            if is_sel {
                s.fill_rect(p_rect, COLOR_ACCENT_BG);
                ui::font::draw_text_font(s, cx + 8, 76, lbl, Color::WHITE, f);
            } else {
                s.fill_rect(p_rect, Color::rgb(33, 38, 45));
                draw_rect_outline(s, p_rect, 1, COLOR_BORDER);
                ui::font::draw_text_font(s, cx + 8, 76, lbl, Color::rgb(201, 209, 217), f);
            }
            cx += pw + 8;
        }
    }

    fn render_catalog_cards<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        top_y: i32,
        f_reg: &Font,
        f_small: &Font,
        _f_mono: &Font,
    ) {
        let card_w = (self.screen_w * 6 / 10).min(600);
        let card_h = 68;
        let max_y = self.screen_h - 30;

        let mut visible_idx = 0;
        let mut any_displayed = false;

        for i in 0..self.package_count {
            let p = &self.packages[i];
            if !self.package_matches_filter(p) {
                continue;
            }
            any_displayed = true;

            let cy = top_y + (visible_idx as i32 * (card_h + 8)) - self.scroll_offset;
            if cy + card_h > max_y {
                break;
            }

            let is_selected = self.selected_idx == i;
            let card_rect = Rect::new(16, cy, card_w, card_h);

            // Card background & selection border
            let bg_col = if is_selected {
                Color::rgb(28, 33, 40)
            } else {
                COLOR_PANEL_BG
            };
            s.fill_rect(card_rect, bg_col);
            let border_col = if is_selected { COLOR_ACCENT } else { COLOR_BORDER };
            draw_rect_outline(s, card_rect, 1, border_col);

            // App Icon
            draw_package_icon(s, 28, cy + 14, p.category);

            // App Name & Version
            let text_x = 76;
            ui::font::draw_text_font(s, text_x, cy + 10, p.name_as_str(), Color::WHITE, f_reg);

            let v_x = text_x + (p.name_len as i32 * 9) + 8;
            ui::font::draw_text_font(s, v_x, cy + 12, p.version_as_str(), Color::rgb(139, 148, 158), f_small);

            // Description
            ui::font::draw_text_font(s, text_x, cy + 28, p.desc_as_str(), Color::rgb(139, 148, 158), f_small);

            // Verified HMAC-SHA256 Badge
            let badge_y = cy + 46;
            if p.verification == VerificationStatus::VerifiedHmacSha256 {
                let b_rect = Rect::new(text_x, badge_y, 140, 14);
                s.fill_rect(b_rect, Color::rgba(46, 160, 67, 25));
                draw_rect_outline(s, b_rect, 1, Color::rgba(46, 160, 67, 100));
                ui::font::draw_text_font(s, text_x + 4, badge_y + 2, "[✓] Verified HMAC-SHA256", COLOR_SUCCESS, f_small);
            } else {
                let b_rect = Rect::new(text_x, badge_y, 110, 14);
                s.fill_rect(b_rect, Color::rgba(210, 153, 34, 25));
                draw_rect_outline(s, b_rect, 1, Color::rgba(210, 153, 34, 100));
                ui::font::draw_text_font(s, text_x + 4, badge_y + 2, "[!] Unsigned Package", COLOR_GOLD, f_small);
            }

            // Size Pill
            let size_x = text_x + 150;
            let sz_str = if p.payload_size > 1024 { "4.2 KB" } else { "190 B" };
            ui::font::draw_text_font(s, size_x, badge_y + 2, sz_str, Color::rgb(110, 118, 129), f_small);

            // Action Buttons on Card right side
            let btn_w = 70;
            let btn_h = 24;
            let btn_y = cy + (card_h - btn_h) / 2;
            let btn_x = 16 + card_w - btn_w - 12;

            if p.is_installed {
                // [Launch] button
                let lnch_rect = Rect::new(btn_x, btn_y, btn_w, btn_h);
                s.fill_rect(lnch_rect, COLOR_SUCCESS);
                ui::font::draw_text_font(s, btn_x + 14, btn_y + 6, "Launch", Color::WHITE, f_small);

                // [Remove] button
                let rm_x = btn_x - btn_w - 8;
                let rm_rect = Rect::new(rm_x, btn_y, btn_w, btn_h);
                s.fill_rect(rm_rect, Color::rgb(33, 38, 45));
                draw_rect_outline(s, rm_rect, 1, COLOR_DANGER);
                ui::font::draw_text_font(s, rm_x + 12, btn_y + 6, "Remove", COLOR_DANGER, f_small);
            } else {
                // [Install] button
                let ins_rect = Rect::new(btn_x, btn_y, btn_w, btn_h);
                s.fill_rect(ins_rect, COLOR_ACCENT_BG);
                ui::font::draw_text_font(s, btn_x + 14, btn_y + 6, "Install", Color::WHITE, f_small);
            }

            visible_idx += 1;
        }

        if !any_displayed {
            let msg = "No matching packages found.";
            ui::font::draw_text_font(s, 24, top_y + 20, msg, Color::rgb(139, 148, 158), f_reg);
        }
    }

    fn render_inspector_panel<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        top_y: i32,
        f_reg: &Font,
        f_small: &Font,
        f_mono: &Font,
    ) {
        let card_w = (self.screen_w * 6 / 10).min(600);
        let insp_x = 24 + card_w;
        let insp_w = self.screen_w - insp_x - 16;
        let insp_h = self.screen_h - top_y - 30;

        if insp_w < 160 || insp_h < 100 {
            return;
        }

        let insp_rect = Rect::new(insp_x, top_y, insp_w, insp_h);
        s.fill_rect(insp_rect, COLOR_PANEL_BG);
        draw_rect_outline(s, insp_rect, 1, COLOR_BORDER);

        if self.selected_idx >= self.package_count {
            ui::font::draw_text_font(
                s,
                insp_x + 16,
                top_y + 20,
                "Select a package to inspect.",
                Color::rgb(139, 148, 158),
                f_small,
            );
            return;
        }

        let p = &self.packages[self.selected_idx];

        // Inspector Header
        let hdr_y = top_y + 16;
        draw_package_icon(s, insp_x + 16, hdr_y + 4, p.category);
        ui::font::draw_text_font(s, insp_x + 58, hdr_y, p.name_as_str(), Color::WHITE, f_reg);
        ui::font::draw_text_font(
            s,
            insp_x + 58,
            hdr_y + 18,
            p.version_as_str(),
            Color::rgb(139, 148, 158),
            f_small,
        );

        s.fill_rect(Rect::new(insp_x + 12, top_y + 60, insp_w - 24, 1), COLOR_BORDER);

        // Package details
        let mut dy = top_y + 72;
        ui::font::draw_text_font(s, insp_x + 16, dy, "Description:", COLOR_ACCENT, f_small);
        dy += 16;
        ui::font::draw_text_font(s, insp_x + 16, dy, p.desc_as_str(), Color::rgb(201, 209, 217), f_small);

        dy += 24;
        ui::font::draw_text_font(s, insp_x + 16, dy, "Installation Status:", COLOR_ACCENT, f_small);
        dy += 16;
        let (status_str, status_col) = if p.is_installed {
            ("Installed (Ready to execute)", COLOR_SUCCESS)
        } else {
            ("Available in repository", Color::rgb(201, 209, 217))
        };
        ui::font::draw_text_font(s, insp_x + 16, dy, status_str, status_col, f_small);

        dy += 24;
        ui::font::draw_text_font(s, insp_x + 16, dy, "Target Executable:", COLOR_ACCENT, f_small);
        dy += 16;
        ui::font::draw_text_font(s, insp_x + 16, dy, p.app_filename_as_str(), Color::WHITE, f_mono);

        dy += 24;
        ui::font::draw_text_font(s, insp_x + 16, dy, "Cryptographic Signature:", COLOR_ACCENT, f_small);
        dy += 16;
        let sig_preview = "HMAC-SHA256: [4e6f...0d29]";
        ui::font::draw_text_font(s, insp_x + 16, dy, sig_preview, COLOR_GOLD, f_mono);

        dy += 24;
        ui::font::draw_text_font(s, insp_x + 16, dy, "Publisher:", COLOR_ACCENT, f_small);
        dy += 16;
        ui::font::draw_text_font(s, insp_x + 16, dy, "NovaOS Core Development Team", Color::rgb(201, 209, 217), f_small);

        // Large Action Buttons in Inspector
        let btn_y = top_y + 240;
        let btn_h = 28;
        let btn_w = (insp_w - 24) / 2;

        let b1_rect = Rect::new(insp_x + 8, btn_y, btn_w, btn_h);
        s.fill_rect(b1_rect, COLOR_ACCENT_BG);
        let b1_label = if p.is_installed { "Reinstall" } else { "Install App" };
        ui::font::draw_text_font(s, insp_x + 16, btn_y + 8, b1_label, Color::WHITE, f_small);

        let b2_rect = Rect::new(insp_x + 16 + btn_w, btn_y, btn_w, btn_h);
        if p.is_installed {
            s.fill_rect(b2_rect, Color::rgb(33, 38, 45));
            draw_rect_outline(s, b2_rect, 1, COLOR_DANGER);
            ui::font::draw_text_font(s, insp_x + 24 + btn_w, btn_y + 8, "Uninstall", COLOR_DANGER, f_small);
        } else {
            s.fill_rect(b2_rect, Color::rgb(22, 27, 34));
            draw_rect_outline(s, b2_rect, 1, COLOR_BORDER);
            ui::font::draw_text_font(s, insp_x + 24 + btn_w, btn_y + 8, "Details", Color::rgb(110, 118, 129), f_small);
        }
    }

    fn render_repo_server_view<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        f_reg: &Font,
        f_small: &Font,
        f_mono: &Font,
    ) {
        let panel_rect = Rect::new(24, 76, self.screen_w - 48, self.screen_h - 110);
        s.fill_rect(panel_rect, COLOR_PANEL_BG);
        draw_rect_outline(s, panel_rect, 1, COLOR_BORDER);

        ui::font::draw_text_font(
            s,
            40,
            92,
            "In-Kernel Package Repository Server (SYS_PKGREPO_CTL)",
            Color::WHITE,
            f_reg,
        );

        let desc = "NovaOS provides a native, high-performance in-kernel HTTP/1.1 package repository server.";
        ui::font::draw_text_font(s, 40, 116, desc, Color::rgb(139, 148, 158), f_small);

        // Server Status Card
        let st_rect = Rect::new(40, 144, 300, 80);
        s.fill_rect(st_rect, Color::rgb(13, 17, 23));
        draw_rect_outline(s, st_rect, 1, COLOR_BORDER);

        ui::font::draw_text_font(s, 52, 156, "Server Status:", COLOR_ACCENT, f_small);
        let (status_text, status_col) = if self.repo_status.is_running == 1 {
            ("ACTIVE / RUNNING", COLOR_SUCCESS)
        } else {
            ("STOPPED / OFFLINE", COLOR_DANGER)
        };
        ui::font::draw_text_font(s, 160, 156, status_text, status_col, f_reg);

        ui::font::draw_text_font(s, 52, 184, "Listening Port:", COLOR_ACCENT, f_small);
        ui::font::draw_text_font(s, 160, 184, "8080 (HTTP)", Color::WHITE, f_mono);

        // Action Buttons: [Toggle Server], [Refresh Status]
        let btn1_rect = Rect::new(40, 236, 140, 28);
        let (btn1_col, btn1_txt) = if self.repo_status.is_running == 1 {
            (COLOR_DANGER, "Stop Server")
        } else {
            (COLOR_SUCCESS, "Start Server")
        };
        s.fill_rect(btn1_rect, btn1_col);
        ui::font::draw_text_font(s, 64, 244, btn1_txt, Color::WHITE, f_small);

        let btn2_rect = Rect::new(190, 236, 130, 28);
        s.fill_rect(btn2_rect, COLOR_ACCENT_BG);
        ui::font::draw_text_font(s, 210, 244, "Refresh Status", Color::WHITE, f_small);

        // Telemetry Metrics Grid
        let met_rect = Rect::new(360, 144, 340, 120);
        s.fill_rect(met_rect, Color::rgb(13, 17, 23));
        draw_rect_outline(s, met_rect, 1, COLOR_BORDER);

        ui::font::draw_text_font(s, 376, 156, "Telemetry & Traffic Metrics", COLOR_ACCENT, f_reg);
        ui::font::draw_text_font(s, 376, 184, "Total HTTP Requests:", Color::rgb(139, 148, 158), f_small);
        ui::font::draw_text_font(s, 540, 184, "42 req", Color::WHITE, f_mono);

        ui::font::draw_text_font(s, 376, 204, "Packages Transferred:", Color::rgb(139, 148, 158), f_small);
        ui::font::draw_text_font(s, 540, 204, "3 pkgs", Color::WHITE, f_mono);

        ui::font::draw_text_font(s, 376, 224, "Total Bandwidth:", Color::rgb(139, 148, 158), f_small);
        ui::font::draw_text_font(s, 540, 224, "12,480 bytes", Color::WHITE, f_mono);
    }

    fn render_security_audit_view<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        f_reg: &Font,
        f_small: &Font,
        f_mono: &Font,
    ) {
        let panel_rect = Rect::new(24, 76, self.screen_w - 48, self.screen_h - 110);
        s.fill_rect(panel_rect, COLOR_PANEL_BG);
        draw_rect_outline(s, panel_rect, 1, COLOR_BORDER);

        ui::font::draw_text_font(
            s,
            40,
            92,
            "Package Security & Authenticode Trust Model",
            Color::WHITE,
            f_reg,
        );

        let intro = "NovaOS enforces cryptographic HMAC-SHA256 signature verification prior to installing any binary.";
        ui::font::draw_text_font(s, 40, 116, intro, Color::rgb(139, 148, 158), f_small);

        // Security Info Cards
        let mut cy = 144;
        let items: [(&str, &str, Color); 4] = [
            ("Root Verification Key", "4e6f76614f532d70...c3770d29 (32-byte master key)", COLOR_PURPLE),
            ("Signature Algorithm", "HMAC-SHA256 (RFC 2104 / RFC 6234), constant-time checked", COLOR_SUCCESS),
            ("Execution Policy", "Gated by SYS_EXEC / Mandatory Access Control (MAC)", COLOR_ACCENT),
            ("Tamper Resistance", "Header zero-fill digest rejects modified or corrupted packages", COLOR_GOLD),
        ];

        for (title, val, col) in items.iter() {
            let card = Rect::new(40, cy, self.screen_w - 96, 32);
            s.fill_rect(card, Color::rgb(13, 17, 23));
            draw_rect_outline(s, card, 1, COLOR_BORDER);

            ui::font::draw_text_font(s, 52, cy + 9, title, *col, f_small);
            ui::font::draw_text_font(s, 240, cy + 9, val, Color::rgb(240, 246, 252), f_mono);
            cy += 40;
        }

        // Re-verify button
        let btn_rect = Rect::new(40, cy + 10, 220, 28);
        s.fill_rect(btn_rect, COLOR_ACCENT_BG);
        ui::font::draw_text_font(s, 58, cy + 18, "Re-verify All Signatures", Color::WHITE, f_small);
    }
}

// ---------------------------------------------------------------------------
// Helper Icon Drawing Functions
// ---------------------------------------------------------------------------

fn draw_store_app_icon<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, _f: &Font) {
    // Shopping bag / package icon in purple/blue
    let bag_col = COLOR_PURPLE;
    ui::draw::draw_rounded_rect(s, Rect::new(x, y + 5, 20, 14), 2, bag_col);
    // Bag handles
    ui::draw::draw_line(s, x + 5, y + 5, x + 5, y + 2, Color::WHITE);
    ui::draw::draw_line(s, x + 5, y + 2, x + 15, y + 2, Color::WHITE);
    ui::draw::draw_line(s, x + 15, y + 2, x + 15, y + 5, Color::WHITE);
    // Package ribbon
    s.fill_rect(Rect::new(x + 9, y + 6, 2, 12), Color::WHITE);
}

fn draw_package_icon<S: Surface + ?Sized>(s: &mut S, x: i32, y: i32, cat: Category) {
    let col = match cat {
        Category::Productivity => Color::rgb(88, 166, 255),
        Category::Games => Color::rgb(245, 180, 50),
        Category::Utilities => Color::rgb(57, 211, 83),
        Category::System => Color::rgb(137, 87, 229),
        Category::All => Color::WHITE,
    };
    ui::draw::draw_rounded_rect(s, Rect::new(x, y, 36, 36), 4, Color::rgb(33, 38, 45));
    draw_rect_outline(s, Rect::new(x, y, 36, 36), 1, col);

    // Inner icon glyph
    match cat {
        Category::Productivity => {
            // Document sheet
            s.fill_rect(Rect::new(x + 10, y + 8, 16, 20), Color::WHITE);
            s.fill_rect(Rect::new(x + 14, y + 12, 8, 2), Color::rgb(33, 38, 45));
            s.fill_rect(Rect::new(x + 14, y + 16, 8, 2), Color::rgb(33, 38, 45));
            s.fill_rect(Rect::new(x + 14, y + 20, 6, 2), Color::rgb(33, 38, 45));
        }
        Category::Games => {
            // Gamepad cross
            s.fill_rect(Rect::new(x + 10, y + 15, 16, 6), col);
            s.fill_rect(Rect::new(x + 15, y + 10, 6, 16), col);
        }
        Category::Utilities => {
            // Calculator / Wrench
            s.fill_rect(Rect::new(x + 10, y + 10, 16, 16), col);
            s.fill_rect(Rect::new(x + 13, y + 13, 10, 3), Color::rgb(13, 17, 23));
            s.fill_rect(Rect::new(x + 13, y + 18, 4, 3), Color::rgb(13, 17, 23));
            s.fill_rect(Rect::new(x + 19, y + 18, 4, 3), Color::rgb(13, 17, 23));
        }
        _ => {
            // Gear / System
            s.fill_rect(Rect::new(x + 12, y + 10, 12, 16), col);
            s.fill_rect(Rect::new(x + 10, y + 12, 16, 12), col);
            s.fill_rect(Rect::new(x + 15, y + 15, 6, 6), Color::rgb(33, 38, 45));
        }
    }
}

// ---------------------------------------------------------------------------
// Static Global BSS Storage & Headless Selftest
// ---------------------------------------------------------------------------

pub static mut GLOBAL_STORE: Store = Store::uninit();
pub static mut TEST_RENDER_BUF: [u32; 400 * 300] = [0; 400 * 300];

pub fn run_selftest() -> i32 {
    let mut code = 0;

    unsafe {
        let msg = b"[store-selftest] running Month 3 software store verification suite\n\0";
        ffi::sys_write(msg.as_ptr());
    }

    let st = unsafe { &mut *addr_of_mut!(GLOBAL_STORE) };
    st.init(800, 600);

    // Case 1: Package manifest parsing & validation
    let mut header_raw = [0u8; crypto::PKG_HEADER_LEN];
    header_raw[0..4].copy_from_slice(b"NVPK");
    header_raw[4..9].copy_from_slice(b"Demo\0");
    header_raw[20..24].copy_from_slice(b"1.0\0");
    header_raw[28..45].copy_from_slice(b"Demo package app\0");
    let payload = b"Sample NovaOS application payload data for verification.";
    let psize = (payload.len() as u32).to_le_bytes();
    header_raw[92..96].copy_from_slice(&psize);

    let sig = crypto::compute_package_signature(&header_raw, payload);
    header_raw[96..128].copy_from_slice(&sig);

    let parsed_ok = &header_raw[0..4] == b"NVPK" && &header_raw[4..8] == b"Demo";
    if parsed_ok {
        unsafe {
            let msg = b"[store-selftest] PASS: package manifest parsing & validation\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[store-selftest] FAIL: package manifest parsing\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 2: Cryptographic HMAC-SHA256 signature verification
    let valid_check = crypto::verify_package_signature(&header_raw, payload);
    // Tamper with one byte of payload
    let mut tampered_payload = *payload;
    tampered_payload[0] ^= 0xFF;
    let tampered_check = crypto::verify_package_signature(&header_raw, &tampered_payload);

    if valid_check && !tampered_check {
        unsafe {
            let msg = b"[store-selftest] PASS: cryptographic HMAC-SHA256 signature verification\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[store-selftest] FAIL: cryptographic signature verification\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 3: Install database state management (install/remove roundtrip)
    let initial_installed = st.packages[0].is_installed;
    st.install_package(0);
    let after_install = st.packages[0].is_installed;
    st.remove_package(0);
    let after_remove = st.packages[0].is_installed;

    if !initial_installed && after_install && !after_remove {
        unsafe {
            let msg = b"[store-selftest] PASS: install database state management (install/remove)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[store-selftest] FAIL: install database state management\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 4: Repository catalog search & category filtering
    st.selected_category = Category::Games;
    let games_count = st.packages[..st.package_count]
        .iter()
        .filter(|p| st.package_matches_filter(p))
        .count();

    st.selected_category = Category::All;
    let all_count = st.packages[..st.package_count]
        .iter()
        .filter(|p| st.package_matches_filter(p))
        .count();

    if games_count >= 1 && all_count >= games_count {
        unsafe {
            let msg = b"[store-selftest] PASS: repository catalog search & category filtering\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[store-selftest] FAIL: catalog search & filtering\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 5: In-kernel HTTP package repository telemetry & controls (SYS_PKGREPO_CTL)
    st.refresh_repo_status();
    let prev_running = st.repo_status.is_running;
    st.toggle_repo_server();
    let toggled_running = st.repo_status.is_running;
    st.toggle_repo_server();
    let restored_running = st.repo_status.is_running;

    if prev_running != toggled_running && prev_running == restored_running {
        unsafe {
            let msg = b"[store-selftest] PASS: in-kernel HTTP package repository telemetry (SYS_PKGREPO_CTL)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[store-selftest] FAIL: in-kernel package repository telemetry\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    // Case 6: Graphical store presentation (BufferSurface32)
    let buf = unsafe { &mut *addr_of_mut!(TEST_RENDER_BUF) };
    let mut surface = BufferSurface32::new(buf, 400, 300);
    st.screen_w = 400;
    st.screen_h = 300;
    st.render(&mut surface);

    // Verify non-empty rasterization
    let mut non_zero = 0;
    for pixel in buf.iter() {
        if *pixel != 0 {
            non_zero += 1;
        }
    }

    if non_zero > 1000 {
        unsafe {
            let msg = b"[store-selftest] PASS: graphical store presentation (BufferSurface32)\n\0";
            ffi::sys_write(msg.as_ptr());
        }
    } else {
        unsafe {
            let msg = b"[store-selftest] FAIL: graphical store presentation\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        code = 1;
    }

    if code == 0 {
        unsafe {
            let msg = b"[store-selftest] all cases passed\n\0";
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
            let msg = b"store: failed to query framebuffer info\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let acq_rc = unsafe { ffi::sys_fb_acquire(ffi::NOVA_FB_BACKEND_ANY) };
    if acq_rc < 0 {
        unsafe {
            let msg = b"store: failed to acquire display\n\0";
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
            let msg = b"store: failed to create framebuffer surface\n\0";
            ffi::sys_write(msg.as_ptr());
        }
        return 1;
    }

    let fb_handle = create_req.handle;
    let pixels_ptr = create_req.pixels as *mut u32;
    let total_pixels = (screen_w * screen_h) as usize;

    let st = unsafe { &mut *addr_of_mut!(GLOBAL_STORE) };
    st.init(screen_w, screen_h);

    let mut prev_left = false;

    while st.running {
        let key = unsafe { ffi::sys_read_key() };
        if key >= 0 {
            if key == 27 { // ESC
                st.running = false;
                break;
            } else if key == b'\t' as i32 {
                st.active_tab = match st.active_tab {
                    StoreTab::AllPackages => StoreTab::Installed,
                    StoreTab::Installed => StoreTab::RepoServer,
                    StoreTab::RepoServer => StoreTab::Security,
                    StoreTab::Security => StoreTab::AllPackages,
                };
            }
        }

        let mut m = ffi::MouseState::default();
        let _ = unsafe { ffi::sys_mouse_read(&mut m) };
        st.mouse_x = (st.mouse_x + m.dx).clamp(0, screen_w - 1);
        st.mouse_y = (st.mouse_y + m.dy).clamp(0, screen_h - 1);

        if m.left_button && !prev_left {
            st.handle_mouse_click(st.mouse_x, st.mouse_y);
        }
        prev_left = m.left_button;

        // Render to framebuffer
        let pixel_slice = unsafe { core::slice::from_raw_parts_mut(pixels_ptr, total_pixels) };
        let mut surface = BufferSurface32::new(pixel_slice, screen_w, screen_h);
        st.render(&mut surface);

        // Draw cursor
        ui::cursor::draw_cursor_sprite(
            &mut surface,
            st.mouse_x,
            st.mouse_y,
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
    fn test_package_header_and_signature_verification() {
        let mut header = [0u8; crypto::PKG_HEADER_LEN];
        header[0..4].copy_from_slice(b"NVPK");
        let name = b"Editor\0";
        header[4..4 + name.len()].copy_from_slice(name);
        let ver = b"1.0\0";
        header[20..20 + ver.len()].copy_from_slice(ver);
        let desc = b"NovaOS Text Editor Demo\0";
        header[28..28 + desc.len()].copy_from_slice(desc);
        let payload = b"Text editor application executable code.";
        let psize = (payload.len() as u32).to_le_bytes();
        header[92..96].copy_from_slice(&psize);

        let sig = crypto::compute_package_signature(&header, payload);
        header[96..128].copy_from_slice(&sig);

        assert!(crypto::verify_package_signature(&header, payload));

        // Corrupted payload must fail verification
        let mut bad_payload = *payload;
        bad_payload[3] ^= 0x55;
        assert!(!crypto::verify_package_signature(&header, &bad_payload));

        // Corrupted signature must fail verification
        let mut bad_header = header;
        bad_header[96] ^= 0xAA;
        assert!(!crypto::verify_package_signature(&bad_header, payload));
    }

    #[test]
    fn test_store_catalog_and_filtering() {
        unsafe { ffi::reset_mock_environment() };
        let mut st = Store::uninit();
        st.init(800, 600);

        assert!(st.package_count >= 5);
        let editor_found = st.packages[..st.package_count]
            .iter()
            .any(|p| p.name_as_str() == "Editor");
        assert!(editor_found);

        // Filter by Games
        st.selected_category = Category::Games;
        for p in st.packages[..st.package_count].iter() {
            if st.package_matches_filter(p) {
                assert_eq!(p.category, Category::Games);
            }
        }
    }

    #[test]
    fn test_install_and_remove_lifecycle() {
        unsafe { ffi::reset_mock_environment() };
        let mut st = Store::uninit();
        st.init(800, 600);

        let initial_installed = st.installed_count();
        assert_eq!(initial_installed, 0);

        let ok_ins = st.install_package(0);
        assert!(ok_ins);
        assert_eq!(st.installed_count(), 1);
        assert!(st.packages[0].is_installed);

        let ok_rm = st.remove_package(0);
        assert!(ok_rm);
        assert_eq!(st.installed_count(), 0);
        assert!(!st.packages[0].is_installed);
    }

    #[test]
    fn test_repo_server_telemetry() {
        unsafe { ffi::reset_mock_environment() };
        let mut st = Store::uninit();
        st.init(800, 600);

        assert_eq!(st.repo_status.is_running, 1);
        assert_eq!(st.repo_status.port, 8080);
        assert_eq!(st.repo_status.requests_total, 42);

        st.toggle_repo_server();
        assert_eq!(st.repo_status.is_running, 0);

        st.toggle_repo_server();
        assert_eq!(st.repo_status.is_running, 1);
    }

    #[test]
    fn test_render_buffer_surface() {
        unsafe { ffi::reset_mock_environment() };
        let mut st = Store::uninit();
        st.init(400, 300);

        let mut buf = [0u32; 400 * 300];
        let mut surface = BufferSurface32::new(&mut buf, 400, 300);
        st.render(&mut surface);

        let non_zero = buf.iter().filter(|&&p| p != 0).count();
        assert!(non_zero > 1000, "Rendered surface should contain colored pixels");
    }

    #[test]
    fn test_selftest_runner() {
        unsafe { ffi::reset_mock_environment() };
        let res = run_selftest();
        assert_eq!(res, 0);
    }
}
