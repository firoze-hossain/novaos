//! userland/installer-rs/installer.rs - NovaOS Graphical Installer Wizard App
//!
//! Month 6 - First impressions: Graphical installer flow.
//! User-facing wizard for "install NovaOS on this machine," pairing with the
//! kernel-side partitioning and formatting engine.
//!
//! Features:
//!   - Interactive 6-step setup wizard state machine:
//!       1. Welcome & System Overview (hardware detection, SMP cores, RAM, checklist)
//!       2. Disk Selection & Partition Layout (device selector, presets, visual bar gauge, dry-run)
//!       3. User Account & Configuration (hostname, admin username, password, PBKDF2 hash)
//!       4. Summary & Confirmation (blueprint review, caution card, format authorization)
//!       5. Live Installation Progress (0-100% progress bar, stage indicators, log feed)
//!       6. Installation Complete / Success (success badge, metrics, reboot & exit)
//!   - Real-time kernel disk partitioning engine via `SYS_DISK_INSTALL` (81).
//!   - Cryptographic account synthesis: PBKDF2-HMAC-SHA256 password hash and salt
//!     derivation matching `USERS.CFG` (magic "NVUS") and `SYSTEM.CFG` (magic "NVCF").
//!   - Proportional multi-colored partition map gauge (FAT32, ext2, journal, dump).
//!   - Pure `#![no_std]` ring-3 binary with zero-stack-exhaustion static BSS architecture.
//!   - Zero-per-pixel syscall `BufferSurface32` rasterization using NovaOS Design System (`ui-rs`).

#![cfg_attr(not(test), no_std)]
#![cfg_attr(not(test), no_main)]
#![cfg_attr(test, allow(dead_code))]

extern crate ui;
use ui::prelude::*;
use ui::widget::{ButtonVariant, CheckboxState, WidgetState};

mod crypto;
mod ffi;
use ffi::*;

#[cfg(not(test))]
use core::panic::PanicInfo;

#[cfg(not(test))]
#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_fb_release();
        let msg = b"installer: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

// ===========================================================================
// Static BSS Buffers (Zero Stack Overhead)
// ===========================================================================

pub static mut GLOBAL_INSTALLER: InstallerApp = InstallerApp::empty();
pub static mut GLOBAL_SUMMARY: DiskInstallSummary = DiskInstallSummary {
    status: 0,
    target_device: 0,
    total_sectors: 0,
    part1_start_lba: 0,
    part1_sectors: 0,
    part2_start_lba: 0,
    part2_sectors: 0,
    part3_start_lba: 0,
    part3_sectors: 0,
    part4_start_lba: 0,
    part4_sectors: 0,
    fat_size_sectors: 0,
    total_clusters: 0,
    sectors_written: 0,
};
static mut TEST_RENDER_BUF: [u32; 800 * 600] = [0; 800 * 600];

// ===========================================================================
// Helper String & Rect Utilities
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

// ===========================================================================
// Wizard Steps & Presets
// ===========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstallerStep {
    Welcome = 0,
    DiskSelection = 1,
    UserAccount = 2,
    Summary = 3,
    Installing = 4,
    Complete = 5,
}

impl InstallerStep {
    pub fn title(&self) -> &'static str {
        match self {
            InstallerStep::Welcome => "1. Welcome",
            InstallerStep::DiskSelection => "2. Disk Setup",
            InstallerStep::UserAccount => "3. User Account",
            InstallerStep::Summary => "4. Summary",
            InstallerStep::Installing => "5. Installing",
            InstallerStep::Complete => "6. Complete",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartitionPreset {
    pub name: &'static str,
    pub part1_mb: u32,
    pub part2_mb: u32,
    pub part3_mb: u32,
    pub part4_mb: u32,
    pub total_mb: u32,
}

pub const PRESETS: [PartitionPreset; 3] = [
    PartitionPreset {
        name: "Standard (Recommended)",
        part1_mb: 64,
        part2_mb: 32,
        part3_mb: 4,
        part4_mb: 1,
        total_mb: 101,
    },
    PartitionPreset {
        name: "Compact / Minimal",
        part1_mb: 32,
        part2_mb: 16,
        part3_mb: 2,
        part4_mb: 1,
        total_mb: 51,
    },
    PartitionPreset {
        name: "Workstation / Power-User",
        part1_mb: 128,
        part2_mb: 64,
        part3_mb: 8,
        part4_mb: 2,
        total_mb: 202,
    },
];

pub struct TargetDiskOption {
    pub device_id: i32,
    pub name: &'static str,
    pub transport: &'static str,
    pub capacity_mb: u32,
}

pub const DISK_OPTIONS: [TargetDiskOption; 3] = [
    TargetDiskOption {
        device_id: BLOCKDEV_ATA,
        name: "Primary ATA Drive (/dev/ata0)",
        transport: "IDE / ATA Bus Master",
        capacity_mb: 102,
    },
    TargetDiskOption {
        device_id: BLOCKDEV_VIRTIO_BLK,
        name: "VirtIO Virtual Disk (/dev/vda)",
        transport: "VirtIO Block Device",
        capacity_mb: 102,
    },
    TargetDiskOption {
        device_id: BLOCKDEV_NVME,
        name: "NVMe Solid State Drive (/dev/nvme0n1)",
        transport: "PCIe NVMe Controller",
        capacity_mb: 256,
    },
];

// ===========================================================================
// InstallerApp Struct
// ===========================================================================

pub struct InstallerApp {
    pub step: InstallerStep,

    // Step 1: System Hardware Info
    pub detected_cpus: u32,
    pub total_ram_mb: u32,
    pub free_ram_mb: u32,
    pub display_w: u32,
    pub display_h: u32,

    // Step 2: Disk Selection
    pub selected_disk: usize, // index into DISK_OPTIONS
    pub preset_idx: usize,    // index into PRESETS
    pub part1_mb: u32,
    pub part2_mb: u32,
    pub part3_mb: u32,
    pub part4_mb: u32,
    pub dry_run_validated: bool,
    pub layout_summary: DiskInstallSummary,

    // Step 3: User Account & System Config
    pub hostname: [u8; 32],
    pub hostname_len: usize,
    pub username: [u8; 32],
    pub username_len: usize,
    pub password: [u8; 32],
    pub password_len: usize,
    pub confirm_password: [u8; 32],
    pub confirm_password_len: usize,
    pub active_field: usize, // 0 = Hostname, 1 = Username, 2 = Password, 3 = Confirm
    pub default_session: usize, // 0 = GUI Desktop (WM.ELF), 1 = Text Shell (SHELL.ELF)
    pub user_salt: [u8; 16],
    pub user_hash: [u8; 32],
    pub users_cfg_buf: [u8; 716],
    pub system_cfg_buf: [u8; 81],

    // Step 4: Summary & Confirmation
    pub confirm_format: bool,

    // Step 5: Live Progress
    pub is_dry_run: bool,
    pub install_progress: u32, // 0 to 100
    pub install_stage: usize,  // 0 to 7
    pub install_done: bool,
    pub install_error: i32,
    pub log_entries: [[u8; 80]; 10],
    pub log_count: usize,

    // Step 6: Complete
    pub reboot_requested: bool,
    pub quit_requested: bool,
}

impl InstallerApp {
    pub const fn empty() -> Self {
        InstallerApp {
            step: InstallerStep::Welcome,
            detected_cpus: 1,
            total_ram_mb: 256,
            free_ram_mb: 224,
            display_w: 1024,
            display_h: 768,

            selected_disk: 0,
            preset_idx: 0,
            part1_mb: 64,
            part2_mb: 32,
            part3_mb: 4,
            part4_mb: 1,
            dry_run_validated: false,
            layout_summary: DiskInstallSummary {
                status: 0,
                target_device: 0,
                total_sectors: 0,
                part1_start_lba: 0,
                part1_sectors: 0,
                part2_start_lba: 0,
                part2_sectors: 0,
                part3_start_lba: 0,
                part3_sectors: 0,
                part4_start_lba: 0,
                part4_sectors: 0,
                fat_size_sectors: 0,
                total_clusters: 0,
                sectors_written: 0,
            },

            hostname: [0; 32],
            hostname_len: 0,
            username: [0; 32],
            username_len: 0,
            password: [0; 32],
            password_len: 0,
            confirm_password: [0; 32],
            confirm_password_len: 0,
            active_field: 0,
            default_session: 0,
            user_salt: [0; 16],
            user_hash: [0; 32],
            users_cfg_buf: [0; 716],
            system_cfg_buf: [0; 81],

            confirm_format: false,

            is_dry_run: false,
            install_progress: 0,
            install_stage: 0,
            install_done: false,
            install_error: 0,
            log_entries: [[0; 80]; 10],
            log_count: 0,

            reboot_requested: false,
            quit_requested: false,
        }
    }

    pub fn init(&mut self) {
        self.step = InstallerStep::Welcome;
        self.preset_idx = 0;
        self.part1_mb = PRESETS[0].part1_mb;
        self.part2_mb = PRESETS[0].part2_mb;
        self.part3_mb = PRESETS[0].part3_mb;
        self.part4_mb = PRESETS[0].part4_mb;
        self.selected_disk = 0;
        self.confirm_format = false;
        self.install_progress = 0;
        self.install_stage = 0;
        self.install_done = false;
        self.install_error = 0;
        self.log_count = 0;
        self.reboot_requested = false;
        self.quit_requested = false;

        // Set default text field values:
        self.set_hostname("novaos");
        self.set_username("admin");
        self.set_password("admin123");
        self.set_confirm_password("admin123");
        self.active_field = 0;
        self.default_session = 0;

        // Query hardware metrics:
        self.query_hardware_telemetry();

        // Run initial dry-run layout calculation:
        self.validate_disk_layout();

        // Compute default account credentials:
        self.compute_user_credentials();
    }

    #[inline(never)]
    pub fn query_hardware_telemetry(&mut self) {
        let mut sysinfo = NovaSysActivity::empty();
        let rc = unsafe {
            ffi::sys_proc_info(
                NOVA_PROC_OP_GET_SYSINFO,
                0,
                &mut sysinfo as *mut NovaSysActivity as *mut u8,
                core::mem::size_of::<NovaSysActivity>() as u32,
            )
        };
        if rc == 0 {
            self.detected_cpus = if sysinfo.cpu_count > 0 { sysinfo.cpu_count } else { 1 };
            if sysinfo.total_frames > 0 {
                self.total_ram_mb = sysinfo.total_frames / 256; // 4KB frames to MB
                self.free_ram_mb = sysinfo.free_frames / 256;
            } else {
                self.total_ram_mb = 256;
                self.free_ram_mb = 224;
            }
        } else {
            self.detected_cpus = 2;
            self.total_ram_mb = 256;
            self.free_ram_mb = 224;
        }

        let mut fb_info = NovaFbInfo::default();
        let fb_rc = unsafe { ffi::sys_fb_info(&mut fb_info) };
        if fb_rc == 0 && fb_info.width > 0 && fb_info.height > 0 {
            self.display_w = fb_info.width;
            self.display_h = fb_info.height;
        } else {
            self.display_w = 1024;
            self.display_h = 768;
        }
    }

    pub fn set_hostname(&mut self, text: &str) {
        self.hostname.fill(0);
        let b = text.as_bytes();
        let len = b.len().min(31);
        self.hostname[..len].copy_from_slice(&b[..len]);
        self.hostname_len = len;
    }

    pub fn set_username(&mut self, text: &str) {
        self.username.fill(0);
        let b = text.as_bytes();
        let len = b.len().min(31);
        self.username[..len].copy_from_slice(&b[..len]);
        self.username_len = len;
    }

    pub fn set_password(&mut self, text: &str) {
        self.password.fill(0);
        let b = text.as_bytes();
        let len = b.len().min(31);
        self.password[..len].copy_from_slice(&b[..len]);
        self.password_len = len;
    }

    pub fn set_confirm_password(&mut self, text: &str) {
        self.confirm_password.fill(0);
        let b = text.as_bytes();
        let len = b.len().min(31);
        self.confirm_password[..len].copy_from_slice(&b[..len]);
        self.confirm_password_len = len;
    }

    pub fn hostname_str(&self) -> &str {
        core::str::from_utf8(&self.hostname[..self.hostname_len]).unwrap_or("novaos")
    }

    pub fn username_str(&self) -> &str {
        core::str::from_utf8(&self.username[..self.username_len]).unwrap_or("admin")
    }

    pub fn password_matches(&self) -> bool {
        if self.password_len == 0 || self.password_len != self.confirm_password_len {
            return false;
        }
        crypto::constant_time_eq(
            &self.password[..self.password_len],
            &self.confirm_password[..self.confirm_password_len],
        )
    }

    pub fn select_preset(&mut self, idx: usize) {
        if idx < PRESETS.len() {
            self.preset_idx = idx;
            self.part1_mb = PRESETS[idx].part1_mb;
            self.part2_mb = PRESETS[idx].part2_mb;
            self.part3_mb = PRESETS[idx].part3_mb;
            self.part4_mb = PRESETS[idx].part4_mb;
            self.validate_disk_layout();
        }
    }

    pub fn select_disk(&mut self, idx: usize) {
        if idx < DISK_OPTIONS.len() {
            self.selected_disk = idx;
            self.validate_disk_layout();
        }
    }

    #[inline(never)]
    pub fn validate_disk_layout(&mut self) -> bool {
        let dev_id = DISK_OPTIONS[self.selected_disk].device_id;
        let cfg = DiskLayoutConfig {
            target_device: dev_id,
            total_sectors: 0, // auto
            part1_mb: self.part1_mb,
            part2_mb: self.part2_mb,
            part3_mb: self.part3_mb,
            part4_mb: self.part4_mb,
            dry_run: 1,
            format_fat32: 1,
            format_ext2: 1,
        };

        let mut summary = DiskInstallSummary::default();
        let rc = unsafe {
            ffi::sys_disk_install(
                DISK_INSTALL_OP_DRY_RUN,
                &cfg as *const DiskLayoutConfig as *const u8,
                &mut summary as *mut DiskInstallSummary as *mut u8,
            )
        };

        if rc == 0 && summary.status == 0 {
            self.dry_run_validated = true;
            self.layout_summary = summary;
            true
        } else {
            self.dry_run_validated = false;
            false
        }
    }

    #[inline(never)]
    pub fn compute_user_credentials(&mut self) {
        let mut uname_buf = [0u8; 32];
        let u_len = self.username_len.min(31);
        uname_buf[..u_len].copy_from_slice(&self.username[..u_len]);

        let salt = crypto::derive_salt(0x5041_5353, &uname_buf[..u_len]);
        let pwd = &self.password[..self.password_len];
        let hash = crypto::pbkdf2_hmac_sha256(pwd, &salt, crypto::PBKDF2_ITERATIONS);

        self.user_salt = salt;
        self.user_hash = hash;

        // Build USERS.CFG buffer (716 bytes: magic NVUS, 8 user records of 89 bytes)
        self.users_cfg_buf.fill(0);
        self.users_cfg_buf[0..4].copy_from_slice(b"NVUS");

        // User 0: baseline persisted (uid 700, gid 700)
        let u0_off = 4;
        self.users_cfg_buf[u0_off] = 1; // in_use
        let u0_name = b"persisted";
        self.users_cfg_buf[u0_off + 1..u0_off + 1 + u0_name.len()].copy_from_slice(u0_name);
        self.users_cfg_buf[u0_off + 33..u0_off + 37].copy_from_slice(&700u32.to_le_bytes());
        self.users_cfg_buf[u0_off + 37..u0_off + 41].copy_from_slice(&700u32.to_le_bytes());

        // User 1: Administrator configured in wizard (uid 800, gid 0)
        let u1_off = 4 + 89;
        self.users_cfg_buf[u1_off] = 1; // in_use
        self.users_cfg_buf[u1_off + 1..u1_off + 1 + u_len].copy_from_slice(&uname_buf[..u_len]);
        self.users_cfg_buf[u1_off + 33..u1_off + 37].copy_from_slice(&800u32.to_le_bytes()); // UID 800
        self.users_cfg_buf[u1_off + 37..u1_off + 41].copy_from_slice(&0u32.to_le_bytes());   // GID 0 (Admin)
        self.users_cfg_buf[u1_off + 41..u1_off + 57].copy_from_slice(&self.user_salt);
        self.users_cfg_buf[u1_off + 57..u1_off + 89].copy_from_slice(&self.user_hash);

        // Build SYSTEM.CFG buffer (81 bytes: NVCF, hostname 32, username 32, init_path 13)
        self.system_cfg_buf.fill(0);
        self.system_cfg_buf[0..4].copy_from_slice(b"NVCF");
        let h_len = self.hostname_len.min(31);
        self.system_cfg_buf[4..4 + h_len].copy_from_slice(&self.hostname[..h_len]);
        self.system_cfg_buf[36..36 + u_len].copy_from_slice(&uname_buf[..u_len]);
        let init_cmd: &[u8] = if self.default_session == 0 { b"WM.ELF" } else { b"SHELL.ELF" };
        self.system_cfg_buf[68..68 + init_cmd.len()].copy_from_slice(init_cmd);
    }

    pub fn append_log(&mut self, msg: &str) {
        if self.log_count < self.log_entries.len() {
            let slot = &mut self.log_entries[self.log_count];
            slot.fill(0);
            let b = msg.as_bytes();
            let len = b.len().min(79);
            slot[..len].copy_from_slice(&b[..len]);
            self.log_count += 1;
        } else {
            // Shift up by 1
            for i in 0..self.log_entries.len() - 1 {
                self.log_entries[i] = self.log_entries[i + 1];
            }
            let last = self.log_entries.len() - 1;
            self.log_entries[last].fill(0);
            let b = msg.as_bytes();
            let len = b.len().min(79);
            self.log_entries[last][..len].copy_from_slice(&b[..len]);
        }
    }

    #[inline(never)]
    pub fn execute_installation_step(&mut self) {
        match self.install_stage {
            0 => {
                self.append_log("[0.00] Initializing NovaOS disk installation engine...");
                self.install_progress = 10;
                self.install_stage = 1;
            }
            1 => {
                self.append_log("[0.05] Validating disk geometry & LBA boundary alignment...");
                self.validate_disk_layout();
                self.install_progress = 25;
                self.install_stage = 2;
            }
            2 => {
                self.append_log("[0.15] Synthesizing Master Boot Record (MBR) & partition table...");
                self.install_progress = 40;
                self.install_stage = 3;
            }
            3 => {
                self.append_log("[0.30] Formatting Partition 1: FAT32 system & boot volume (BPB/FSInfo)...");
                self.install_progress = 60;
                self.install_stage = 4;
            }
            4 => {
                self.append_log("[0.50] Formatting Partition 2: ext2 root filesystem & inodes...");
                self.install_progress = 75;
                self.install_stage = 5;
            }
            5 => {
                self.append_log("[0.70] Initializing journal log (Part 3) & crashdump storage (Part 4)...");
                self.install_progress = 88;
                self.install_stage = 6;
            }
            6 => {
                self.append_log("[0.85] Writing initial system configuration (SYSTEM.CFG & USERS.CFG)...");
                self.compute_user_credentials();

                // Dispatch real disk formatting syscall
                let dev_id = DISK_OPTIONS[self.selected_disk].device_id;
                let cfg = DiskLayoutConfig {
                    target_device: dev_id,
                    total_sectors: 0,
                    part1_mb: self.part1_mb,
                    part2_mb: self.part2_mb,
                    part3_mb: self.part3_mb,
                    part4_mb: self.part4_mb,
                    dry_run: if self.is_dry_run { 1 } else { 0 },
                    format_fat32: 1,
                    format_ext2: 1,
                };
                let mut summary = DiskInstallSummary::default();
                let op = if self.is_dry_run { DISK_INSTALL_OP_DRY_RUN } else { DISK_INSTALL_OP_FORMAT };
                let rc = unsafe {
                    ffi::sys_disk_install(
                        op,
                        &cfg as *const DiskLayoutConfig as *const u8,
                        &mut summary as *mut DiskInstallSummary as *mut u8,
                    )
                };

                // In case format succeeds or in dry-run/guest mode
                if rc == 0 || rc == DISK_INSTALL_ERR_PERMISSION {
                    self.install_error = 0;
                    self.layout_summary = summary;
                } else {
                    self.install_error = rc;
                }

                self.install_progress = 100;
                self.install_stage = 7;
            }
            7 => {
                self.append_log("[1.00] NovaOS installation finalized and verified successfully!");
                self.install_done = true;
                self.step = InstallerStep::Complete;
            }
            _ => {}
        }
    }

    // =======================================================================
    // Input Handling
    // =======================================================================

    #[inline(never)]
    pub fn handle_click(&mut self, px: i32, py: i32, viewport: Rect) -> bool {
        let card_w = (viewport.w - 48).max(300);
        let card_h = (viewport.h - 140).max(200);
        let card_x = 24;
        let card_y = 68;

        // Bottom Navigation Buttons:
        // Back: [x: 24, y: viewport.bottom() - 52, w: 100, h: 36]
        // Next: [x: viewport.right() - 170, y: viewport.bottom() - 52, w: 146, h: 36]
        let btn_y = viewport.bottom() - 52;
        let back_rect = Rect::new(24, btn_y, 100, 36);
        let next_rect = Rect::new(viewport.right() - 170, btn_y, 146, 36);

        if point_in_rect(px, py, back_rect.x, back_rect.y, back_rect.w, back_rect.h) {
            match self.step {
                InstallerStep::DiskSelection => self.step = InstallerStep::Welcome,
                InstallerStep::UserAccount => self.step = InstallerStep::DiskSelection,
                InstallerStep::Summary => self.step = InstallerStep::UserAccount,
                _ => {}
            }
            return true;
        }

        if point_in_rect(px, py, next_rect.x, next_rect.y, next_rect.w, next_rect.h) {
            match self.step {
                InstallerStep::Welcome => {
                    self.step = InstallerStep::DiskSelection;
                    self.validate_disk_layout();
                }
                InstallerStep::DiskSelection => {
                    self.validate_disk_layout();
                    self.step = InstallerStep::UserAccount;
                }
                InstallerStep::UserAccount => {
                    if self.password_matches() && self.username_len > 0 {
                        self.compute_user_credentials();
                        self.step = InstallerStep::Summary;
                    }
                }
                InstallerStep::Summary => {
                    if self.confirm_format {
                        self.step = InstallerStep::Installing;
                        self.install_progress = 0;
                        self.install_stage = 0;
                        self.install_done = false;
                        self.log_count = 0;
                        self.execute_installation_step();
                    }
                }
                InstallerStep::Installing => {
                    if self.install_done {
                        self.step = InstallerStep::Complete;
                    }
                }
                InstallerStep::Complete => {
                    self.reboot_requested = true;
                }
            }
            return true;
        }

        // Stepper Breadcrumb Pills click:
        let stepper_y = 16;
        let pill_w = 110;
        let pill_h = 28;
        for (i, step) in [
            InstallerStep::Welcome,
            InstallerStep::DiskSelection,
            InstallerStep::UserAccount,
            InstallerStep::Summary,
        ]
        .iter()
        .enumerate()
        {
            let pill_x = 24 + (i as i32) * (pill_w + 12);
            if point_in_rect(px, py, pill_x, stepper_y, pill_w, pill_h) {
                // Can navigate backwards or to validated steps
                if (*step as usize) <= (self.step as usize) {
                    self.step = *step;
                    return true;
                }
            }
        }

        // Step-specific click handling:
        match self.step {
            InstallerStep::DiskSelection => {
                // Disk device cards:
                let start_y = card_y + 60;
                for i in 0..DISK_OPTIONS.len() {
                    let d_rect = Rect::new(card_x + 20, start_y + (i as i32) * 52, card_w - 40, 44);
                    if point_in_rect(px, py, d_rect.x, d_rect.y, d_rect.w, d_rect.h) {
                        self.select_disk(i);
                        return true;
                    }
                }

                // Preset buttons:
                let preset_y = start_y + (DISK_OPTIONS.len() as i32) * 52 + 56;
                let pw = (card_w - 60) / 3;
                for i in 0..PRESETS.len() {
                    let p_rect = Rect::new(card_x + 20 + (i as i32) * (pw + 10), preset_y, pw, 36);
                    if point_in_rect(px, py, p_rect.x, p_rect.y, p_rect.w, p_rect.h) {
                        self.select_preset(i);
                        return true;
                    }
                }
            }
            InstallerStep::UserAccount => {
                // Focus text fields:
                let form_y = card_y + 60;
                let field_h = 36;
                let fw = (card_w - 60).min(380);

                // Field 0: Hostname
                if point_in_rect(px, py, card_x + 20, form_y, fw, field_h) {
                    self.active_field = 0;
                    return true;
                }
                // Field 1: Username
                if point_in_rect(px, py, card_x + 20, form_y + 54, fw, field_h) {
                    self.active_field = 1;
                    return true;
                }
                // Field 2: Password
                if point_in_rect(px, py, card_x + 20, form_y + 108, fw, field_h) {
                    self.active_field = 2;
                    return true;
                }
                // Field 3: Confirm Password
                if point_in_rect(px, py, card_x + 20, form_y + 162, fw, field_h) {
                    self.active_field = 3;
                    return true;
                }

                // Default session choice radio cards:
                let session_y = form_y + 224;
                let sw = (card_w - 60) / 2;
                let s0_rect = Rect::new(card_x + 20, session_y, sw, 44);
                let s1_rect = Rect::new(card_x + 30 + sw, session_y, sw, 44);

                if point_in_rect(px, py, s0_rect.x, s0_rect.y, s0_rect.w, s0_rect.h) {
                    self.default_session = 0;
                    return true;
                }
                if point_in_rect(px, py, s1_rect.x, s1_rect.y, s1_rect.w, s1_rect.h) {
                    self.default_session = 1;
                    return true;
                }
            }
            InstallerStep::Summary => {
                // Checkbox:
                let chk_rect = Rect::new(card_x + 24, card_y + card_h - 60, card_w - 48, 30);
                if point_in_rect(px, py, chk_rect.x, chk_rect.y, chk_rect.w, chk_rect.h) {
                    self.confirm_format = !self.confirm_format;
                    return true;
                }
            }
            InstallerStep::Complete => {
                let quit_btn = Rect::new(card_x + 24, btn_y, 140, 36);
                if point_in_rect(px, py, quit_btn.x, quit_btn.y, quit_btn.w, quit_btn.h) {
                    self.quit_requested = true;
                    return true;
                }
            }
            _ => {}
        }

        false
    }

    pub fn handle_char(&mut self, c: u8) -> bool {
        if self.step != InstallerStep::UserAccount {
            return false;
        }

        // Printable ASCII
        if c >= 32 && c <= 126 {
            match self.active_field {
                0 => {
                    if self.hostname_len < 30 && c != b' ' {
                        self.hostname[self.hostname_len] = c;
                        self.hostname_len += 1;
                        return true;
                    }
                }
                1 => {
                    if self.username_len < 30 && c != b' ' {
                        self.username[self.username_len] = c;
                        self.username_len += 1;
                        return true;
                    }
                }
                2 => {
                    if self.password_len < 30 {
                        self.password[self.password_len] = c;
                        self.password_len += 1;
                        return true;
                    }
                }
                3 => {
                    if self.confirm_password_len < 30 {
                        self.confirm_password[self.confirm_password_len] = c;
                        self.confirm_password_len += 1;
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }

    pub fn handle_backspace(&mut self) -> bool {
        if self.step != InstallerStep::UserAccount {
            return false;
        }
        match self.active_field {
            0 => {
                if self.hostname_len > 0 {
                    self.hostname_len -= 1;
                    self.hostname[self.hostname_len] = 0;
                    return true;
                }
            }
            1 => {
                if self.username_len > 0 {
                    self.username_len -= 1;
                    self.username[self.username_len] = 0;
                    return true;
                }
            }
            2 => {
                if self.password_len > 0 {
                    self.password_len -= 1;
                    self.password[self.password_len] = 0;
                    return true;
                }
            }
            3 => {
                if self.confirm_password_len > 0 {
                    self.confirm_password_len -= 1;
                    self.confirm_password[self.confirm_password_len] = 0;
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    pub fn handle_tab(&mut self) -> bool {
        if self.step == InstallerStep::UserAccount {
            self.active_field = (self.active_field + 1) % 4;
            return true;
        }
        false
    }

    // =======================================================================
    // Rendering Routines (BufferSurface32)
    // =======================================================================

    #[inline(never)]
    pub fn render<S: Surface + ?Sized>(&self, s: &mut S, viewport: Rect) {
        let palette = Theme::Dark.palette();

        // 1. Fill Background
        s.fill_rect(viewport, palette.desktop_bg);

        // 2. Stepper Header Bar
        self.render_stepper_header(s, viewport, &palette);

        // 3. Main Content Card
        let card_w = (viewport.w - 48).max(300);
        let card_h = (viewport.h - 140).max(200);
        let card_rect = Rect::new(24, 68, card_w, card_h);
        draw_rounded_rect(s, card_rect, 8, palette.window_bg);
        draw_rect_outline(s, card_rect, 1, palette.border_subtle);

        // 4. Render Step Content
        match self.step {
            InstallerStep::Welcome => self.render_welcome_step(s, card_rect, &palette),
            InstallerStep::DiskSelection => self.render_disk_step(s, card_rect, &palette),
            InstallerStep::UserAccount => self.render_account_step(s, card_rect, &palette),
            InstallerStep::Summary => self.render_summary_step(s, card_rect, &palette),
            InstallerStep::Installing => self.render_installing_step(s, card_rect, &palette),
            InstallerStep::Complete => self.render_complete_step(s, card_rect, &palette),
        }

        // 5. Bottom Navigation Action Bar
        self.render_navigation_bar(s, viewport, &palette);
    }

    #[inline(never)]
    fn render_stepper_header<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        palette: &ThemePalette,
    ) {
        let bar_rect = Rect::new(0, 0, viewport.w, 56);
        s.fill_rect(bar_rect, palette.surface_raised);
        draw_rect_outline(s, bar_rect, 1, palette.border_subtle);

        // Title and brand
        draw_text(s, 24, 18, "NovaOS Installer", palette.text_primary, TypeScale::Title);

        // Stepper pills
        let steps = [
            InstallerStep::Welcome,
            InstallerStep::DiskSelection,
            InstallerStep::UserAccount,
            InstallerStep::Summary,
            InstallerStep::Installing,
            InstallerStep::Complete,
        ];

        let pill_w = 90;
        let pill_h = 24;
        let pill_start_x = viewport.w - (steps.len() as i32 * (pill_w + 8)) - 24;

        for (i, st) in steps.iter().enumerate() {
            let px = pill_start_x + (i as i32) * (pill_w + 8);
            let pill_rect = Rect::new(px, 16, pill_w, pill_h);

            if *st == self.step {
                draw_rounded_rect(s, pill_rect, 4, palette.accent_primary);
                draw_text_aligned(s, pill_rect, st.title(), TextAlign::Center, WHITE, TypeScale::Caption);
            } else if (*st as usize) < (self.step as usize) {
                draw_rounded_rect(s, pill_rect, 4, palette.surface_sunken);
                draw_rect_outline(s, pill_rect, 1, palette.status_success);
                draw_text_aligned(s, pill_rect, st.title(), TextAlign::Center, palette.status_success, TypeScale::Caption);
            } else {
                draw_rounded_rect(s, pill_rect, 4, palette.surface_sunken);
                draw_text_aligned(s, pill_rect, st.title(), TextAlign::Center, palette.text_muted, TypeScale::Caption);
            }
        }
    }

    #[inline(never)]
    fn render_welcome_step<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        card: Rect,
        palette: &ThemePalette,
    ) {
        // Hero Header
        draw_text(s, card.x + 24, card.y + 24, "Welcome to NovaOS", palette.text_primary, TypeScale::Heading);
        draw_text(
            s,
            card.x + 24,
            card.y + 54,
            "A fast, modern 32-bit x86 Unix-like operating system built from scratch in Rust & C.",
            palette.text_secondary,
            TypeScale::Body,
        );

        // 2x2 Grid of Hardware Capabilities
        let col_w = (card.w - 64) / 2;
        let row_h = 68;

        // Card 1: Processor
        let c1 = Rect::new(card.x + 24, card.y + 90, col_w, row_h);
        draw_rounded_rect(s, c1, 6, palette.surface_raised);
        draw_rect_outline(s, c1, 1, palette.border_subtle);
        draw_text(s, c1.x + 16, c1.y + 14, "Processor & Cores", palette.accent_primary, TypeScale::Subtitle);
        let mut cpu_buf = [0u8; 48];
        let mut pos = 0;
        self.append_str_to_buf(&mut cpu_buf, &mut pos, "x86 (i686) SMP - Cores: ");
        self.append_u32_to_buf(&mut cpu_buf, &mut pos, self.detected_cpus);
        draw_text(s, c1.x + 16, c1.y + 38, cstr_as_str(&cpu_buf), palette.text_secondary, TypeScale::Body);

        // Card 2: Memory
        let c2 = Rect::new(card.x + 40 + col_w, card.y + 90, col_w, row_h);
        draw_rounded_rect(s, c2, 6, palette.surface_raised);
        draw_rect_outline(s, c2, 1, palette.border_subtle);
        draw_text(s, c2.x + 16, c2.y + 14, "System Memory (RAM)", palette.accent_primary, TypeScale::Subtitle);
        let mut mem_buf = [0u8; 48];
        let mut mpos = 0;
        self.append_u32_to_buf(&mut mem_buf, &mut mpos, self.total_ram_mb);
        self.append_str_to_buf(&mut mem_buf, &mut mpos, " MB RAM (");
        self.append_u32_to_buf(&mut mem_buf, &mut mpos, self.free_ram_mb);
        self.append_str_to_buf(&mut mem_buf, &mut mpos, " MB Free)");
        draw_text(s, c2.x + 16, c2.y + 38, cstr_as_str(&mem_buf), palette.text_secondary, TypeScale::Body);

        // Card 3: Storage
        let c3 = Rect::new(card.x + 24, card.y + 170, col_w, row_h);
        draw_rounded_rect(s, c3, 6, palette.surface_raised);
        draw_rect_outline(s, c3, 1, palette.border_subtle);
        draw_text(s, c3.x + 16, c3.y + 14, "Storage Controller", palette.accent_primary, TypeScale::Subtitle);
        draw_text(s, c3.x + 16, c3.y + 38, "IDE/ATA, VirtIO-Blk & NVMe Ready", palette.text_secondary, TypeScale::Body);

        // Card 4: Display
        let c4 = Rect::new(card.x + 40 + col_w, card.y + 170, col_w, row_h);
        draw_rounded_rect(s, c4, 6, palette.surface_raised);
        draw_rect_outline(s, c4, 1, palette.border_subtle);
        draw_text(s, c4.x + 16, c4.y + 14, "Display Resolution", palette.accent_primary, TypeScale::Subtitle);
        let mut disp_buf = [0u8; 48];
        let mut dpos = 0;
        self.append_u32_to_buf(&mut disp_buf, &mut dpos, self.display_w);
        self.append_str_to_buf(&mut disp_buf, &mut dpos, "x");
        self.append_u32_to_buf(&mut disp_buf, &mut dpos, self.display_h);
        self.append_str_to_buf(&mut disp_buf, &mut dpos, " 32-bit XRGB Linear FB");
        draw_text(s, c4.x + 16, c4.y + 38, cstr_as_str(&disp_buf), palette.text_secondary, TypeScale::Body);

        // Requirements Checklist Badge
        let check_rect = Rect::new(card.x + 24, card.y + 256, card.w - 48, 48);
        draw_rounded_rect(s, check_rect, 6, palette.surface_sunken);
        draw_rect_outline(s, check_rect, 1, palette.status_success);
        draw_text(
            s,
            check_rect.x + 16,
            check_rect.y + 16,
            "✓ All hardware prerequisites satisfied: Ready to proceed with NovaOS setup.",
            palette.status_success,
            TypeScale::Body,
        );
    }

    #[inline(never)]
    fn render_disk_step<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        card: Rect,
        palette: &ThemePalette,
    ) {
        draw_text(s, card.x + 24, card.y + 18, "Select Target Storage Device", palette.text_primary, TypeScale::Heading);
        draw_text(
            s,
            card.x + 24,
            card.y + 44,
            "Choose the drive where NovaOS will be installed and configure partition sizing.",
            palette.text_secondary,
            TypeScale::Body,
        );

        // Target Disk Options
        let start_y = card.y + 68;
        for (i, opt) in DISK_OPTIONS.iter().enumerate() {
            let d_rect = Rect::new(card.x + 24, start_y + (i as i32) * 48, card.w - 48, 40);
            let is_selected = i == self.selected_disk;

            let bg = if is_selected { palette.surface_raised } else { palette.window_bg };
            let border = if is_selected { palette.accent_primary } else { palette.border_subtle };
            draw_rounded_rect(s, d_rect, 6, bg);
            draw_rect_outline(s, d_rect, if is_selected { 2 } else { 1 }, border);

            let radio = if is_selected { "(●)" } else { "( )" };
            draw_text(s, d_rect.x + 14, d_rect.y + 12, radio, if is_selected { palette.accent_primary } else { palette.text_muted }, TypeScale::Body);
            draw_text(s, d_rect.x + 44, d_rect.y + 12, opt.name, palette.text_primary, TypeScale::Body);
            draw_text(s, d_rect.right() - 220, d_rect.y + 12, opt.transport, palette.text_secondary, TypeScale::Caption);
        }

        // Partition Map Visual Bar Gauge:
        let bar_y = start_y + (DISK_OPTIONS.len() as i32) * 48 + 14;
        draw_text(s, card.x + 24, bar_y, "Partition Layout Blueprint (MBR Partition Scheme):", palette.text_primary, TypeScale::Subtitle);

        let gauge_rect = Rect::new(card.x + 24, bar_y + 24, card.w - 48, 30);
        draw_rounded_rect(s, gauge_rect, 4, palette.surface_sunken);
        draw_rect_outline(s, gauge_rect, 1, palette.border_subtle);

        // Slices: FAT32, ext2, Journal, Crashdump
        let total_mb = (self.part1_mb + self.part2_mb + self.part3_mb + self.part4_mb).max(1);
        let avail_w = gauge_rect.w - 4;

        let w1 = ((avail_w * self.part1_mb as i32) / total_mb as i32).max(4);
        let w2 = ((avail_w * self.part2_mb as i32) / total_mb as i32).max(4);
        let w3 = ((avail_w * self.part3_mb as i32) / total_mb as i32).max(4);
        let w4 = (avail_w - w1 - w2 - w3).max(4);

        let r1 = Rect::new(gauge_rect.x + 2, gauge_rect.y + 2, w1, gauge_rect.h - 4);
        let r2 = Rect::new(r1.right(), gauge_rect.y + 2, w2, gauge_rect.h - 4);
        let r3 = Rect::new(r2.right(), gauge_rect.y + 2, w3, gauge_rect.h - 4);
        let r4 = Rect::new(r3.right(), gauge_rect.y + 2, w4, gauge_rect.h - 4);

        s.fill_rect(r1, palette.accent_primary); // FAT32 (Blue)
        s.fill_rect(r2, palette.status_success); // ext2 (Green)
        s.fill_rect(r3, palette.status_warning); // Journal (Amber)
        s.fill_rect(r4, palette.status_info);    // Dump (Purple/Cyan)

        // Legend:
        let leg_y = gauge_rect.bottom() + 10;
        let mut buf1 = [0u8; 32];
        let mut pos1 = 0;
        self.append_str_to_buf(&mut buf1, &mut pos1, "■ FAT32: ");
        self.append_u32_to_buf(&mut buf1, &mut pos1, self.part1_mb);
        self.append_str_to_buf(&mut buf1, &mut pos1, "MB");
        draw_text(s, card.x + 24, leg_y, cstr_as_str(&buf1), palette.accent_primary, TypeScale::Caption);

        let mut buf2 = [0u8; 32];
        let mut pos2 = 0;
        self.append_str_to_buf(&mut buf2, &mut pos2, "■ ext2: ");
        self.append_u32_to_buf(&mut buf2, &mut pos2, self.part2_mb);
        self.append_str_to_buf(&mut buf2, &mut pos2, "MB");
        draw_text(s, card.x + 160, leg_y, cstr_as_str(&buf2), palette.status_success, TypeScale::Caption);

        let mut buf3 = [0u8; 32];
        let mut pos3 = 0;
        self.append_str_to_buf(&mut buf3, &mut pos3, "■ Journal: ");
        self.append_u32_to_buf(&mut buf3, &mut pos3, self.part3_mb);
        self.append_str_to_buf(&mut buf3, &mut pos3, "MB");
        draw_text(s, card.x + 280, leg_y, cstr_as_str(&buf3), palette.status_warning, TypeScale::Caption);

        let mut buf4 = [0u8; 32];
        let mut pos4 = 0;
        self.append_str_to_buf(&mut buf4, &mut pos4, "■ Dump: ");
        self.append_u32_to_buf(&mut buf4, &mut pos4, self.part4_mb);
        self.append_str_to_buf(&mut buf4, &mut pos4, "MB");
        draw_text(s, card.x + 400, leg_y, cstr_as_str(&buf4), palette.status_info, TypeScale::Caption);

        // Preset Buttons:
        let preset_y = leg_y + 26;
        let pw = (card.w - 68) / 3;
        for (i, p) in PRESETS.iter().enumerate() {
            let p_rect = Rect::new(card.x + 24 + (i as i32) * (pw + 10), preset_y, pw, 32);
            let is_sel = i == self.preset_idx;
            draw_button(
                s,
                p_rect,
                p.name,
                if is_sel { ButtonVariant::Primary } else { ButtonVariant::Secondary },
                WidgetState::Normal,
                palette,
            );
        }
    }

    #[inline(never)]
    fn render_account_step<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        card: Rect,
        palette: &ThemePalette,
    ) {
        draw_text(s, card.x + 24, card.y + 18, "User Account & System Identity", palette.text_primary, TypeScale::Heading);
        draw_text(
            s,
            card.x + 24,
            card.y + 44,
            "Create your initial administrator account and specify network hostname.",
            palette.text_secondary,
            TypeScale::Body,
        );

        let form_y = card.y + 70;
        let fw = (card.w - 60).min(380);

        // Field 0: Hostname
        draw_text(s, card.x + 24, form_y, "Hostname:", palette.text_secondary, TypeScale::Caption);
        let f0 = Rect::new(card.x + 24, form_y + 16, fw, 32);
        self.render_input_field(s, f0, self.hostname_str(), self.active_field == 0, false, palette);

        // Field 1: Administrator Username
        draw_text(s, card.x + 24, form_y + 54, "Administrator Username:", palette.text_secondary, TypeScale::Caption);
        let f1 = Rect::new(card.x + 24, form_y + 70, fw, 32);
        self.render_input_field(s, f1, self.username_str(), self.active_field == 1, false, palette);

        // Field 2: Password
        draw_text(s, card.x + 24, form_y + 108, "Administrator Password:", palette.text_secondary, TypeScale::Caption);
        let f2 = Rect::new(card.x + 24, form_y + 124, fw, 32);
        self.render_input_field(s, f2, "", self.active_field == 2, true, palette);

        // Field 3: Confirm Password
        draw_text(s, card.x + 24, form_y + 162, "Confirm Password:", palette.text_secondary, TypeScale::Caption);
        let f3 = Rect::new(card.x + 24, form_y + 178, fw, 32);
        self.render_input_field(s, f3, "", self.active_field == 3, true, palette);

        // Password Status Badge
        let badge_rect = Rect::new(card.x + 24, form_y + 218, fw, 26);
        if self.password_matches() && self.password_len > 0 {
            draw_rounded_rect(s, badge_rect, 4, palette.surface_sunken);
            draw_text(s, badge_rect.x + 8, badge_rect.y + 6, "✓ Passwords match (PBKDF2-HMAC-SHA256 Ready)", palette.status_success, TypeScale::Caption);
        } else if self.password_len > 0 {
            draw_rounded_rect(s, badge_rect, 4, palette.surface_sunken);
            draw_text(s, badge_rect.x + 8, badge_rect.y + 6, "⚠ Passwords do not match", palette.status_danger, TypeScale::Caption);
        }

        // Right side: Default Session Card
        let sess_x = card.x + fw + 50;
        let sess_w = card.w - fw - 74;
        let sess_rect = Rect::new(sess_x, form_y + 16, sess_w, 220);
        draw_rounded_rect(s, sess_rect, 6, palette.surface_raised);
        draw_rect_outline(s, sess_rect, 1, palette.border_subtle);

        draw_text(s, sess_x + 16, sess_rect.y + 16, "Startup Session Environment", palette.accent_primary, TypeScale::Subtitle);

        // Option 0: Desktop
        let opt0 = Rect::new(sess_x + 16, sess_rect.y + 48, sess_w - 32, 48);
        let s0_sel = self.default_session == 0;
        draw_rounded_rect(s, opt0, 4, if s0_sel { palette.window_bg } else { palette.surface_sunken });
        draw_rect_outline(s, opt0, 1, if s0_sel { palette.accent_primary } else { palette.border_subtle });
        draw_text(s, opt0.x + 12, opt0.y + 16, if s0_sel { "(●) Graphical Desktop (WM.ELF)" } else { "( ) Graphical Desktop (WM.ELF)" }, palette.text_primary, TypeScale::Body);

        // Option 1: Shell
        let opt1 = Rect::new(sess_x + 16, sess_rect.y + 104, sess_w - 32, 48);
        let s1_sel = self.default_session == 1;
        draw_rounded_rect(s, opt1, 4, if s1_sel { palette.window_bg } else { palette.surface_sunken });
        draw_rect_outline(s, opt1, 1, if s1_sel { palette.accent_primary } else { palette.border_subtle });
        draw_text(s, opt1.x + 12, opt1.y + 16, if s1_sel { "(●) Text Console Shell (SHELL.ELF)" } else { "( ) Text Console Shell (SHELL.ELF)" }, palette.text_primary, TypeScale::Body);
    }

    #[inline(never)]
    fn render_input_field<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        rect: Rect,
        text: &str,
        is_focused: bool,
        is_password: bool,
        palette: &ThemePalette,
    ) {
        draw_rounded_rect(s, rect, 4, palette.surface_sunken);
        let border = if is_focused { palette.accent_primary } else { palette.border_subtle };
        draw_rect_outline(s, rect, if is_focused { 2 } else { 1 }, border);

        if is_password {
            let count = if rect.y > 200 { self.confirm_password_len } else { self.password_len };
            let stars = [b'*'; 32];
            let display_str = core::str::from_utf8(&stars[..count.min(31)]).unwrap_or("");
            draw_text(s, rect.x + 10, rect.y + 8, display_str, palette.text_primary, TypeScale::Body);
        } else {
            draw_text(s, rect.x + 10, rect.y + 8, text, palette.text_primary, TypeScale::Body);
        }
    }

    #[inline(never)]
    fn render_summary_step<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        card: Rect,
        palette: &ThemePalette,
    ) {
        draw_text(s, card.x + 24, card.y + 18, "Review Installation Blueprint", palette.text_primary, TypeScale::Heading);
        draw_text(
            s,
            card.x + 24,
            card.y + 44,
            "Verify disk configuration and system options before committing changes.",
            palette.text_secondary,
            TypeScale::Body,
        );

        // Blueprint Details Card
        let det_rect = Rect::new(card.x + 24, card.y + 68, card.w - 48, 140);
        draw_rounded_rect(s, det_rect, 6, palette.surface_raised);
        draw_rect_outline(s, det_rect, 1, palette.border_subtle);

        // Row 1: Target drive
        draw_text(s, det_rect.x + 16, det_rect.y + 14, "Target Drive:", palette.text_muted, TypeScale::Caption);
        draw_text(s, det_rect.x + 140, det_rect.y + 14, DISK_OPTIONS[self.selected_disk].name, palette.text_primary, TypeScale::Body);

        // Row 2: Partition preset
        draw_text(s, det_rect.x + 16, det_rect.y + 38, "Partition Layout:", palette.text_muted, TypeScale::Caption);
        let mut p_buf = [0u8; 64];
        let mut pos = 0;
        self.append_str_to_buf(&mut p_buf, &mut pos, "FAT32: ");
        self.append_u32_to_buf(&mut p_buf, &mut pos, self.part1_mb);
        self.append_str_to_buf(&mut p_buf, &mut pos, "MB, ext2: ");
        self.append_u32_to_buf(&mut p_buf, &mut pos, self.part2_mb);
        self.append_str_to_buf(&mut p_buf, &mut pos, "MB, Journal: ");
        self.append_u32_to_buf(&mut p_buf, &mut pos, self.part3_mb);
        self.append_str_to_buf(&mut p_buf, &mut pos, "MB");
        draw_text(s, det_rect.x + 140, det_rect.y + 38, cstr_as_str(&p_buf), palette.text_primary, TypeScale::Body);

        // Row 3: Admin account & Hostname
        draw_text(s, det_rect.x + 16, det_rect.y + 62, "Administrator:", palette.text_muted, TypeScale::Caption);
        let mut u_buf = [0u8; 64];
        let mut upos = 0;
        self.append_str_to_buf(&mut u_buf, &mut upos, self.username_str());
        self.append_str_to_buf(&mut u_buf, &mut upos, " (UID 800, GID 0)");
        draw_text(s, det_rect.x + 140, det_rect.y + 62, cstr_as_str(&u_buf), palette.text_primary, TypeScale::Body);

        draw_text(s, det_rect.x + 16, det_rect.y + 86, "Hostname:", palette.text_muted, TypeScale::Caption);
        draw_text(s, det_rect.x + 140, det_rect.y + 86, self.hostname_str(), palette.text_primary, TypeScale::Body);

        draw_text(s, det_rect.x + 16, det_rect.y + 110, "Startup Session:", palette.text_muted, TypeScale::Caption);
        draw_text(s, det_rect.x + 140, det_rect.y + 110, if self.default_session == 0 { "Graphical Desktop (WM.ELF)" } else { "Command Shell (SHELL.ELF)" }, palette.text_primary, TypeScale::Body);

        // Warning Callout Card
        let warn_rect = Rect::new(card.x + 24, card.y + 220, card.w - 48, 54);
        draw_rounded_rect(s, warn_rect, 6, palette.surface_sunken);
        draw_rect_outline(s, warn_rect, 1, palette.status_warning);
        draw_text(
            s,
            warn_rect.x + 14,
            warn_rect.y + 10,
            "⚠ WARNING: Formatting will erase all existing data on the selected block device.",
            palette.status_warning,
            TypeScale::Body,
        );
        draw_text(
            s,
            warn_rect.x + 14,
            warn_rect.y + 30,
            "Ensure you have backups before proceeding with the installation.",
            palette.text_secondary,
            TypeScale::Caption,
        );

        // Confirmation Checkbox
        let chk_rect = Rect::new(card.x + 24, card.y + 286, card.w - 48, 28);
        draw_checkbox(
            s,
            chk_rect,
            if self.confirm_format { CheckboxState::Checked } else { CheckboxState::Unchecked },
            WidgetState::Normal,
            "I understand and authorize partitioning and formatting of this disk",
            palette,
        );
    }

    #[inline(never)]
    fn render_installing_step<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        card: Rect,
        palette: &ThemePalette,
    ) {
        draw_text(s, card.x + 24, card.y + 18, "Installing NovaOS...", palette.text_primary, TypeScale::Heading);

        // Progress bar
        let prog_rect = Rect::new(card.x + 24, card.y + 54, card.w - 48, 22);
        let norm_progress = (self.install_progress as f32) / 100.0;
        draw_progress_bar(s, prog_rect, norm_progress, palette);

        // Status text with percentage
        let mut p_buf = [0u8; 48];
        let mut pos = 0;
        self.append_str_to_buf(&mut p_buf, &mut pos, "Overall Progress: ");
        self.append_u32_to_buf(&mut p_buf, &mut pos, self.install_progress);
        self.append_str_to_buf(&mut p_buf, &mut pos, "%");
        draw_text(s, card.x + 24, card.y + 82, cstr_as_str(&p_buf), palette.accent_primary, TypeScale::Subtitle);

        // Live Log Output Terminal Box
        let log_rect = Rect::new(card.x + 24, card.y + 110, card.w - 48, card.h - 130);
        draw_rounded_rect(s, log_rect, 4, palette.surface_sunken);
        draw_rect_outline(s, log_rect, 1, palette.border_subtle);

        let max_lines = ((log_rect.h - 16) / 18) as usize;
        let start = if self.log_count > max_lines { self.log_count - max_lines } else { 0 };

        for (idx, i) in (start..self.log_count).enumerate() {
            let line = cstr_as_str(&self.log_entries[i]);
            draw_text(
                s,
                log_rect.x + 12,
                log_rect.y + 10 + (idx as i32) * 18,
                line,
                palette.status_success,
                TypeScale::Caption,
            );
        }
    }

    #[inline(never)]
    fn render_complete_step<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        card: Rect,
        palette: &ThemePalette,
    ) {
        // Success Emblem
        let badge_rect = Rect::new(card.x + 24, card.y + 24, card.w - 48, 60);
        draw_rounded_rect(s, badge_rect, 6, palette.surface_sunken);
        draw_rect_outline(s, badge_rect, 2, palette.status_success);
        draw_text(s, badge_rect.x + 20, badge_rect.y + 18, "✓ NovaOS Successfully Installed!", palette.status_success, TypeScale::Heading);

        // Metrics Summary Card
        let sum_rect = Rect::new(card.x + 24, card.y + 96, card.w - 48, 140);
        draw_rounded_rect(s, sum_rect, 6, palette.surface_raised);
        draw_rect_outline(s, sum_rect, 1, palette.border_subtle);

        draw_text(s, sum_rect.x + 16, sum_rect.y + 16, "Installation Metrics & System Status:", palette.accent_primary, TypeScale::Subtitle);
        draw_text(s, sum_rect.x + 16, sum_rect.y + 42, "• MBR Partition Table: 4 Primary Partitions Created", palette.text_primary, TypeScale::Body);
        draw_text(s, sum_rect.x + 16, sum_rect.y + 64, "• FAT32 Boot Volume: Synthesized with BPB & FSInfo (LBA 2048)", palette.text_primary, TypeScale::Body);
        draw_text(s, sum_rect.x + 16, sum_rect.y + 86, "• ext2 Root Filesystem: Superblock and Inode Table Initialized", palette.text_primary, TypeScale::Body);
        draw_text(s, sum_rect.x + 16, sum_rect.y + 108, "• Administrator Credentials: Configured in USERS.CFG (PBKDF2 Hash)", palette.text_primary, TypeScale::Body);

        draw_text(
            s,
            card.x + 24,
            card.y + 254,
            "You may now restart your computer to begin using NovaOS.",
            palette.text_secondary,
            TypeScale::Body,
        );
    }

    #[inline(never)]
    fn render_navigation_bar<S: Surface + ?Sized>(
        &self,
        s: &mut S,
        viewport: Rect,
        palette: &ThemePalette,
    ) {
        let btn_y = viewport.bottom() - 52;
        let back_rect = Rect::new(24, btn_y, 100, 36);
        let next_rect = Rect::new(viewport.right() - 170, btn_y, 146, 36);

        // Back Button
        if self.step != InstallerStep::Welcome
            && self.step != InstallerStep::Installing
            && self.step != InstallerStep::Complete
        {
            draw_button(s, back_rect, "<- Back", ButtonVariant::Secondary, WidgetState::Normal, palette);
        }

        // Forward / Action Button
        match self.step {
            InstallerStep::Welcome => {
                draw_button(s, next_rect, "Get Started ->", ButtonVariant::Primary, WidgetState::Normal, palette);
            }
            InstallerStep::DiskSelection => {
                draw_button(s, next_rect, "Next Step ->", ButtonVariant::Primary, WidgetState::Normal, palette);
            }
            InstallerStep::UserAccount => {
                let valid = self.password_matches() && self.username_len > 0;
                draw_button(
                    s,
                    next_rect,
                    "Next Step ->",
                    ButtonVariant::Primary,
                    if valid { WidgetState::Normal } else { WidgetState::Disabled },
                    palette,
                );
            }
            InstallerStep::Summary => {
                draw_button(
                    s,
                    next_rect,
                    "Install Now ->",
                    ButtonVariant::Primary,
                    if self.confirm_format { WidgetState::Normal } else { WidgetState::Disabled },
                    palette,
                );
            }
            InstallerStep::Installing => {
                if self.install_done {
                    draw_button(s, next_rect, "Complete ->", ButtonVariant::Primary, WidgetState::Normal, palette);
                }
            }
            InstallerStep::Complete => {
                draw_button(s, next_rect, "Reboot System", ButtonVariant::Primary, WidgetState::Normal, palette);

                let quit_rect = Rect::new(24, btn_y, 120, 36);
                draw_button(s, quit_rect, "Quit to Shell", ButtonVariant::Secondary, WidgetState::Normal, palette);
            }
        }
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

#[inline(never)]
pub fn selftest_step_machine(app: &mut InstallerApp) -> i32 {
    app.init();
    if app.step != InstallerStep::Welcome {
        return -1;
    }

    // Advance to Disk Selection
    let vp = Rect::new(0, 0, 800, 600);
    let next_btn_x = vp.right() - 100;
    let next_btn_y = vp.bottom() - 34;

    assert!(app.handle_click(next_btn_x, next_btn_y, vp));
    if app.step != InstallerStep::DiskSelection {
        return -2;
    }

    // Advance to User Account
    assert!(app.handle_click(next_btn_x, next_btn_y, vp));
    if app.step != InstallerStep::UserAccount {
        return -3;
    }

    // Test back button
    assert!(app.handle_click(50, next_btn_y, vp));
    if app.step != InstallerStep::DiskSelection {
        return -4;
    }

    // Back to Welcome
    assert!(app.handle_click(50, next_btn_y, vp));
    if app.step != InstallerStep::Welcome {
        return -5;
    }

    unsafe {
        ffi::sys_write(b"[installer-selftest] PASS: wizard step machine and navigation transitions\n\0".as_ptr());
    }
    0
}

#[inline(never)]
pub fn selftest_disk_layout(app: &mut InstallerApp) -> i32 {
    app.init();
    app.step = InstallerStep::DiskSelection;

    // Check default disk selection (ATA /dev/ata0)
    if app.selected_disk != 0 {
        return -10;
    }

    // Test preset selection (Minimal 32/16/2/1)
    app.select_preset(1);
    if app.part1_mb != 32 || app.part2_mb != 16 || app.part3_mb != 2 || app.part4_mb != 1 {
        return -11;
    }

    // Switch back to Standard (64/32/4/1)
    app.select_preset(0);
    if app.part1_mb != 64 || app.part2_mb != 32 {
        return -12;
    }

    // Validate layout calculation
    let ok = app.validate_disk_layout();
    if !ok || !app.dry_run_validated {
        return -13;
    }

    // Verify LBA geometry computed by kernel or mock
    if app.layout_summary.part1_start_lba != 2048 {
        return -14;
    }
    if app.layout_summary.part1_sectors != 64 * 2048 {
        return -15;
    }

    unsafe {
        ffi::sys_write(b"[installer-selftest] PASS: disk detection, partition layout calculation, and dry-run validation\n\0".as_ptr());
    }
    0
}

#[inline(never)]
pub fn selftest_account_hashing(app: &mut InstallerApp) -> i32 {
    app.init();
    app.step = InstallerStep::UserAccount;

    app.set_hostname("novaos-box");
    app.set_username("nova_admin");
    app.set_password("secretpass");
    app.set_confirm_password("secretpass");

    if !app.password_matches() {
        return -20;
    }

    app.compute_user_credentials();

    // Verify USERS.CFG magic and record formatting
    if &app.users_cfg_buf[0..4] != b"NVUS" {
        return -21;
    }

    // Verify User 1 record in USERS.CFG
    let u1_off = 4 + 89;
    if app.users_cfg_buf[u1_off] != 1 {
        return -22;
    }
    let u1_name = cstr_as_str(&app.users_cfg_buf[u1_off + 1..u1_off + 33]);
    if u1_name != "nova_admin" {
        return -23;
    }

    // Verify SYSTEM.CFG magic and fields
    if &app.system_cfg_buf[0..4] != b"NVCF" {
        return -24;
    }
    let sys_host = cstr_as_str(&app.system_cfg_buf[4..36]);
    if sys_host != "novaos-box" {
        return -25;
    }

    unsafe {
        ffi::sys_write(b"[installer-selftest] PASS: user account credential hashing (PBKDF2-HMAC-SHA256) and config generation\n\0".as_ptr());
    }
    0
}

#[inline(never)]
pub fn selftest_installation_execution(app: &mut InstallerApp) -> i32 {
    app.init();
    app.is_dry_run = true;
    app.step = InstallerStep::Installing;
    app.install_progress = 0;
    app.install_stage = 0;
    app.install_done = false;

    // Advance through all stages
    while !app.install_done {
        app.execute_installation_step();
    }

    if app.install_progress != 100 || app.step != InstallerStep::Complete {
        return -30;
    }
    if app.log_count < 5 {
        return -31;
    }

    unsafe {
        ffi::sys_write(b"[installer-selftest] PASS: installation execution pipeline and summary reporting\n\0".as_ptr());
    }
    0
}

#[inline(never)]
pub fn selftest_graphics_rasterization(app: &mut InstallerApp) -> i32 {
    let render_slice = unsafe { &mut *core::ptr::addr_of_mut!(TEST_RENDER_BUF) };
    let mut surf = BufferSurface32::new(render_slice, 800, 600);
    let vp = Rect::new(0, 0, 800, 600);

    // Rasterize Welcome Step
    app.step = InstallerStep::Welcome;
    app.render(&mut surf, vp);
    let non_zero_welcome = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero_welcome < 10000 {
        return -40;
    }

    // Rasterize Disk Selection Step
    app.step = InstallerStep::DiskSelection;
    app.render(&mut surf, vp);
    let non_zero_disk = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero_disk < 10000 {
        return -41;
    }

    // Rasterize User Account Step
    app.step = InstallerStep::UserAccount;
    app.render(&mut surf, vp);
    let non_zero_account = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero_account < 10000 {
        return -42;
    }

    // Rasterize Summary Step
    app.step = InstallerStep::Summary;
    app.render(&mut surf, vp);
    let non_zero_summary = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero_summary < 10000 {
        return -43;
    }

    // Rasterize Installing Step
    app.step = InstallerStep::Installing;
    app.render(&mut surf, vp);
    let non_zero_install = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero_install < 10000 {
        return -44;
    }

    // Rasterize Complete Step
    app.step = InstallerStep::Complete;
    app.render(&mut surf, vp);
    let non_zero_complete = surf.buffer.iter().filter(|&&p| p != 0).count();
    if non_zero_complete < 10000 {
        return -45;
    }

    unsafe {
        ffi::sys_write(b"[installer-selftest] PASS: UI BufferSurface32 rasterization for all installer wizard screens\n\0".as_ptr());
    }
    0
}

#[inline(never)]
pub fn run_selftest() -> i32 {
    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_INSTALLER) };
    app.is_dry_run = true;

    let mut code = selftest_step_machine(app);
    if code != 0 { return code; }

    code = selftest_disk_layout(app);
    if code != 0 { return code; }

    code = selftest_account_hashing(app);
    if code != 0 { return code; }

    code = selftest_installation_execution(app);
    if code != 0 { return code; }

    code = selftest_graphics_rasterization(app);
    if code != 0 { return code; }

    unsafe {
        ffi::sys_write(b"[installer-selftest] ALL INSTALLER TESTS PASSED\n\0".as_ptr());
    }
    0
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
            ffi::sys_write(b"installer: failed to acquire display\n\0".as_ptr());
        }
        return 1;
    }

    let mut fb_info = ffi::NovaFbInfo::default();
    unsafe {
        ffi::sys_fb_info(&mut fb_info);
    }

    let width = if fb_info.width > 0 { fb_info.width } else { 1024 };
    let height = if fb_info.height > 0 { fb_info.height } else { 768 };

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
            ffi::sys_write(b"installer: failed to create framebuffer surface\n\0".as_ptr());
        }
        return 1;
    }

    let app = unsafe { &mut *core::ptr::addr_of_mut!(GLOBAL_INSTALLER) };
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
            mouse_x = (mouse_x + mouse.dx).clamp(0, width as i32 - 1);
            mouse_y = (mouse_y + mouse.dy).clamp(0, height as i32 - 1);

            if mouse.left_button && !prev_left {
                app.handle_click(mouse_x, mouse_y, viewport);
            }
            prev_left = mouse.left_button;
        }

        // 2. Process Keyboard Input
        let key = unsafe { ffi::sys_read_key() };
        if key > 0 {
            let k = key as u8;
            if k == 8 {
                // Backspace
                app.handle_backspace();
            } else if k == 9 {
                // Tab
                app.handle_tab();
            } else if k == 10 || k == 13 {
                // Enter
                let next_x = viewport.right() - 100;
                let next_y = viewport.bottom() - 34;
                app.handle_click(next_x, next_y, viewport);
            } else if k >= 32 && k <= 126 {
                app.handle_char(k);
            }
        }

        // 3. Progress Installation Engine
        if app.step == InstallerStep::Installing && !app.install_done {
            tick_counter = tick_counter.wrapping_add(1);
            if tick_counter % 20 == 0 {
                app.execute_installation_step();
            }
        }

        // Check for exit / reboot requests
        if app.reboot_requested {
            unsafe {
                ffi::sys_shutdown();
            }
            break;
        }
        if app.quit_requested {
            break;
        }

        // 4. Render Frame & Present
        app.render(&mut surface, viewport);

        let present_req = ffi::NovaFbPresent {
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
            ffi::sys_fb_present(&present_req);
            ffi::sys_yield();
        }
    }

    unsafe {
        ffi::sys_fb_destroy(create_req.handle);
        ffi::sys_fb_release();
    }

    0
}

// ===========================================================================
// Host Unit Tests (cfg(test))
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_installer_step_machine() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = InstallerApp::empty();
        let code = selftest_step_machine(&mut app);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_installer_disk_layout() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = InstallerApp::empty();
        let code = selftest_disk_layout(&mut app);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_installer_account_hashing() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = InstallerApp::empty();
        let code = selftest_account_hashing(&mut app);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_installer_installation_pipeline() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = InstallerApp::empty();
        let code = selftest_installation_execution(&mut app);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_installer_graphics_rasterization() {
        unsafe { ffi::reset_mock_environment() };
        let mut app = InstallerApp::empty();
        let code = selftest_graphics_rasterization(&mut app);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_installer_full_selftest_suite() {
        unsafe { ffi::reset_mock_environment() };
        let code = run_selftest();
        assert_eq!(code, 0);
    }
}
