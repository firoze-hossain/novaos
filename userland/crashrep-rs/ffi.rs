//! userland/crashrep-rs/ffi.rs - FFI bindings and mock test environment for Crash Reporter.
//! Exposes binary ABI matching `crashdump.h`, `novasys.h`, and framebuffer syscalls.

#![allow(dead_code, static_mut_refs)]

pub const SYS_EXIT: u32 = 1;
pub const SYS_READ: u32 = 3;
pub const SYS_WRITE: u32 = 4;
pub const SYS_OPEN: u32 = 5;
pub const SYS_CLOSE: u32 = 6;
pub const SYS_YIELD: u32 = 24;
pub const SYS_SHUTDOWN: u32 = 52;
pub const SYS_FB_INFO: u32 = 68;
pub const SYS_FB_ACQUIRE: u32 = 69;
pub const SYS_FB_RELEASE: u32 = 70;
pub const SYS_FB_CREATE: u32 = 71;
pub const SYS_FB_DESTROY: u32 = 72;
pub const SYS_FB_PRESENT: u32 = 73;
pub const SYS_READ_KEY: u32 = 77;
pub const SYS_MOUSE_READ: u32 = 78;
pub const SYS_CRASHDUMP_CTL: u32 = 80;

pub const CRASHDUMP_OP_STATUS: i32 = 1;
pub const CRASHDUMP_OP_GET: i32 = 2;
pub const CRASHDUMP_OP_EXPORT: i32 = 3;
pub const CRASHDUMP_OP_CLEAR: i32 = 4;

pub const O_RDONLY: i32 = 0;

#[repr(C, packed)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NovaCrashdumpStatus {
    pub has_crash: u32,
    pub exported: u32,
    pub ticks: u32,
    pub file_size: u32,
    pub filename: [u8; 16],
}

impl NovaCrashdumpStatus {
    pub fn filename_str(&self) -> &str {
        let mut len = 0;
        while len < self.filename.len() && self.filename[len] != 0 {
            len += 1;
        }
        core::str::from_utf8(&self.filename[..len]).unwrap_or("CRASHDMP.TXT")
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct NovaCrashReport {
    pub ticks: u32,
    pub reason: [u8; 64],
    pub has_regs: u32,
    pub regs_ds: u32,
    pub regs_edi: u32,
    pub regs_esi: u32,
    pub regs_ebp: u32,
    pub regs_esp_dummy: u32,
    pub regs_ebx: u32,
    pub regs_edx: u32,
    pub regs_ecx: u32,
    pub regs_eax: u32,
    pub regs_int_no: u32,
    pub regs_err_code: u32,
    pub regs_eip: u32,
    pub regs_cs: u32,
    pub regs_eflags: u32,
    pub regs_useresp: u32,
    pub regs_ss: u32,
    pub has_fault_addr: u32,
    pub fault_addr: u32,
    pub stack: [u32; 8],
    pub stack_count: u32,
}

impl Default for NovaCrashReport {
    fn default() -> Self {
        NovaCrashReport {
            ticks: 0,
            reason: [0u8; 64],
            has_regs: 0,
            regs_ds: 0,
            regs_edi: 0,
            regs_esi: 0,
            regs_ebp: 0,
            regs_esp_dummy: 0,
            regs_ebx: 0,
            regs_edx: 0,
            regs_ecx: 0,
            regs_eax: 0,
            regs_int_no: 0,
            regs_err_code: 0,
            regs_eip: 0,
            regs_cs: 0,
            regs_eflags: 0,
            regs_useresp: 0,
            regs_ss: 0,
            has_fault_addr: 0,
            fault_addr: 0,
            stack: [0u32; 8],
            stack_count: 0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct MouseState {
    pub dx: i32,
    pub dy: i32,
    pub left_button: bool,
    pub right_button: bool,
    pub middle_button: bool,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbInfo {
    pub struct_size: u32,
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub backend: u32,
    pub backend_mask: u32,
    pub caps: u32,
    pub max_surface_dim: u32,
    pub max_surfaces: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbCreate {
    pub width: u32,
    pub height: u32,
    pub flags: u32,
    pub handle: u32,
    pub pixels: u32,
    pub stride: u32,
    pub size: u32,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct NovaFbPresent {
    pub handle: u32,
    pub src_x: i32,
    pub src_y: i32,
    pub width: i32,
    pub height: i32,
    pub dst_x: i32,
    pub dst_y: i32,
    pub flags: u32,
}

pub const NOVA_FB_BACKEND_ANY: u32 = 0;

// ===========================================================================
// Bare-Metal Syscalls (not(test))
// ===========================================================================

#[cfg(not(test))]
extern "C" {
    pub fn sys_crashdump_ctl(op: i32, arg: *mut u8) -> i32;
    pub fn sys_open(filename: *const u8) -> i32;
    pub fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32;
    pub fn sys_close(handle: i32);
    pub fn sys_write(s: *const u8) -> i32;
    pub fn sys_exit(code: i32) -> !;
    pub fn sys_yield();
    pub fn sys_shutdown() -> i32;

    pub fn sys_fb_info(out: *mut NovaFbInfo) -> i32;
    pub fn sys_fb_acquire(backend: u32) -> i32;
    pub fn sys_fb_release() -> i32;
    pub fn sys_fb_create(req: *mut NovaFbCreate) -> i32;
    pub fn sys_fb_destroy(handle: u32) -> i32;
    pub fn sys_fb_present(req: *const NovaFbPresent) -> i32;

    pub fn sys_read_key() -> i32;
    pub fn sys_mouse_read(out: *mut MouseState) -> i32;
}

// ===========================================================================
// Mock Test Environment (test)
// ===========================================================================

#[cfg(test)]
pub static mut MOCK_CRASH_STATUS: NovaCrashdumpStatus = NovaCrashdumpStatus {
    has_crash: 1,
    exported: 1,
    ticks: 344865,
    file_size: 1174,
    filename: [b'C', b'R', b'A', b'S', b'H', b'D', b'M', b'P', b'.', b'T', b'X', b'T', 0, 0, 0, 0],
};

#[cfg(test)]
pub static mut MOCK_CRASH_REPORT: NovaCrashReport = NovaCrashReport {
    ticks: 344865,
    reason: [
        b's', b'e', b'l', b'f', b't', b'e', b's', b't', b':', b' ', b's', b'y', b'n', b't', b'h',
        b'e', b't', b'i', b'c', b' ', b'p', b'a', b'g', b'e', b' ', b'f', b'a', b'u', b'l', b't',
        b' ', b'c', b'r', b'a', b's', b'h', b' ', b'e', b'x', b'p', b'o', b'r', b't', 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ],
    has_regs: 1,
    regs_ds: 0x10,
    regs_edi: 0x01000000,
    regs_esi: 0x02000000,
    regs_ebp: 0x03000000,
    regs_esp_dummy: 0,
    regs_ebx: 0x04000000,
    regs_edx: 0x05000000,
    regs_ecx: 0x06000000,
    regs_eax: 0x11223344,
    regs_int_no: 14,
    regs_err_code: 0x2,
    regs_eip: 0x80481234,
    regs_cs: 0x08,
    regs_eflags: 0x0202,
    regs_useresp: 0x07000000,
    regs_ss: 0x10,
    has_fault_addr: 1,
    fault_addr: 0x014AB000,
    stack: [0x80481000, 0x80482000, 0x80483000, 0, 0, 0, 0, 0],
    stack_count: 3,
};

#[cfg(test)]
pub static MOCK_SAMPLE_CRASHDMP_TXT: &[u8] = b"================================================================================
NovaOS Crash Dump Report
================================================================================

Status: Unhandled Kernel Exception / Panic
Timestamp: 344865 ticks

Panic Reason:
  selftest: synthetic page fault crash export

Fault Information:
  Exception Vector: 14 (Page Fault)
  Error Code:       0x00000002
  Faulting EIP:     0x80481234  CS: 0x00000008
  EFLAGS:           0x00000202

CPU Registers:
  EAX: 0x11223344  EBX: 0x04000000  ECX: 0x06000000  EDX: 0x05000000
  ESI: 0x02000000  EDI: 0x01000000  EBP: 0x03000000  ESP: 0x07000000
  DS:  0x00000010  SS:  0x00000010

Faulting Address (CR2):
  0x014ab000

Call Stack Trace (3 frames):
  [0] 0x80481000
  [1] 0x80482000
  [2] 0x80483000

[CRASH_METADATA]
MAGIC=NOVACRASH
VERSION=1
TICKS=344865
REASON=selftest: synthetic page fault crash export
HAS_REGS=1
INT_NO=14
ERR_CODE=0x00000002
EIP=0x80481234
EAX=0x11223344
EBX=0x04000000
ECX=0x06000000
EDX=0x05000000
ESI=0x02000000
EDI=0x01000000
EBP=0x03000000
ESP=0x07000000
CR2=0x014ab000
STACK_COUNT=3
STACK_0=0x80481000
STACK_1=0x80482000
STACK_2=0x80483000
END_METADATA=1
";

#[cfg(test)]
pub static mut MOCK_FILE_OPEN: bool = false;
#[cfg(test)]
pub static mut MOCK_FILE_READ_OFFSET: usize = 0;
#[cfg(test)]
pub static mut MOCK_CLEARED: bool = false;

#[cfg(test)]
pub unsafe fn sys_crashdump_ctl(op: i32, arg: *mut u8) -> i32 {
    match op {
        CRASHDUMP_OP_STATUS => {
            if arg.is_null() {
                return -1;
            }
            let out = &mut *(arg as *mut NovaCrashdumpStatus);
            *out = MOCK_CRASH_STATUS;
            0
        }
        CRASHDUMP_OP_GET => {
            if arg.is_null() {
                return -1;
            }
            let out = &mut *(arg as *mut NovaCrashReport);
            *out = MOCK_CRASH_REPORT;
            0
        }
        CRASHDUMP_OP_CLEAR => {
            MOCK_CLEARED = true;
            MOCK_CRASH_STATUS.has_crash = 0;
            MOCK_CRASH_STATUS.exported = 0;
            MOCK_CRASH_STATUS.file_size = 0;
            0
        }
        CRASHDUMP_OP_EXPORT => 0,
        _ => -1,
    }
}

#[cfg(test)]
pub unsafe fn sys_open(_filename: *const u8) -> i32 {
    if MOCK_CRASH_STATUS.has_crash == 0 {
        return -1;
    }
    MOCK_FILE_OPEN = true;
    MOCK_FILE_READ_OFFSET = 0;
    42 // valid fd
}

#[cfg(test)]
pub unsafe fn sys_read(handle: i32, buf: *mut u8, max_len: i32) -> i32 {
    if handle != 42 || !MOCK_FILE_OPEN || buf.is_null() || max_len <= 0 {
        return -1;
    }
    let data = MOCK_SAMPLE_CRASHDMP_TXT;
    if MOCK_FILE_READ_OFFSET >= data.len() {
        return 0; // EOF
    }
    let remaining = data.len() - MOCK_FILE_READ_OFFSET;
    let to_copy = remaining.min(max_len as usize);
    let slice = core::slice::from_raw_parts_mut(buf, to_copy);
    slice.copy_from_slice(&data[MOCK_FILE_READ_OFFSET..MOCK_FILE_READ_OFFSET + to_copy]);
    MOCK_FILE_READ_OFFSET += to_copy;
    to_copy as i32
}

#[cfg(test)]
pub unsafe fn sys_close(handle: i32) {
    if handle == 42 {
        MOCK_FILE_OPEN = false;
    }
}

#[cfg(test)]
pub unsafe fn sys_write(_s: *const u8) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_exit(_code: i32) -> ! {
    panic!("sys_exit called in test");
}

#[cfg(test)]
pub unsafe fn sys_yield() {}

#[cfg(test)]
pub unsafe fn sys_shutdown() -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_info(out: *mut NovaFbInfo) -> i32 {
    if !out.is_null() {
        (*out).width = 1024;
        (*out).height = 768;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_acquire(_backend: u32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_release() -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_create(req: *mut NovaFbCreate) -> i32 {
    if !req.is_null() {
        (*req).handle = 1;
    }
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_destroy(_handle: u32) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_fb_present(_req: *const NovaFbPresent) -> i32 {
    0
}

#[cfg(test)]
pub unsafe fn sys_read_key() -> i32 {
    -1
}

#[cfg(test)]
pub unsafe fn sys_mouse_read(_out: *mut MouseState) -> i32 {
    -1
}
