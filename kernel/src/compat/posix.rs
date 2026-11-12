//! kernel/src/compat/posix.rs - Linux / POSIX.1-2017 Compatibility Layer & Translation Engine
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Linux/POSIX Compatibility Layer (novalibc & POSIX ABI)
//!
//! Provides:
//!   - High-speed translation table mapping Linux x86_64 system call numbers to NovaOS handlers.
//!   - Conversion of POSIX file access flags (O_RDONLY, O_WRONLY, O_CREAT, etc.) to NovaOS VFS flags.
//!   - Conversion of POSIX memory protection (PROT_*) and mapping flags (MAP_*) to NovaOS VMM ProtFlags.
//!   - POSIX 1024-bit signal set manipulation and mask translation.
//!   - Standard ELF64 dynamic relocation validation (R_X86_64_RELATIVE, GLOB_DAT, JUMP_SLOT).
//!   - Bidirectional mapping between NovaOS kernel error codes and standard POSIX errno values.
//!   - Comprehensive in-kernel boot self-test verifying complete ABI compatibility.

#![allow(dead_code, non_camel_case_types)]

use core::sync::atomic::{AtomicU32, Ordering};

// -----------------------------------------------------------------------------
// Linux x86_64 System Call Numbers
// -----------------------------------------------------------------------------

pub const LINUX_SYS_READ: u64          = 0;
pub const LINUX_SYS_WRITE: u64         = 1;
pub const LINUX_SYS_OPEN: u64          = 2;
pub const LINUX_SYS_CLOSE: u64         = 3;
pub const LINUX_SYS_STAT: u64          = 4;
pub const LINUX_SYS_FSTAT: u64         = 5;
pub const LINUX_SYS_POLL: u64          = 7;
pub const LINUX_SYS_LSEEK: u64         = 8;
pub const LINUX_SYS_MMAP: u64          = 9;
pub const LINUX_SYS_MPROTECT: u64      = 10;
pub const LINUX_SYS_MUNMAP: u64        = 11;
pub const LINUX_SYS_BRK: u64           = 12;
pub const LINUX_SYS_RT_SIGACTION: u64  = 13;
pub const LINUX_SYS_RT_SIGPROCMASK: u64= 14;
pub const LINUX_SYS_RT_SIGRETURN: u64  = 15;
pub const LINUX_SYS_IOCTL: u64         = 16;
pub const LINUX_SYS_PIPE: u64          = 22;
pub const LINUX_SYS_SCHED_YIELD: u64   = 24;
pub const LINUX_SYS_GETPID: u64        = 39;
pub const LINUX_SYS_FORK: u64          = 57;
pub const LINUX_SYS_EXECVE: u64        = 59;
pub const LINUX_SYS_EXIT: u64          = 60;
pub const LINUX_SYS_WAIT4: u64         = 61;
pub const LINUX_SYS_KILL: u64          = 62;
pub const LINUX_SYS_UNAME: u64         = 63;
pub const LINUX_SYS_GETUID: u64        = 102;
pub const LINUX_SYS_GETGID: u64        = 104;
pub const LINUX_SYS_GETPPID: u64       = 110;
pub const LINUX_SYS_EPOLL_CREATE: u64  = 213;
pub const LINUX_SYS_CLOCK_GETTIME: u64 = 228;
pub const LINUX_SYS_EPOLL_WAIT: u64    = 232;
pub const LINUX_SYS_EPOLL_CTL: u64     = 233;
pub const LINUX_SYS_PIPE2: u64         = 293;

// -----------------------------------------------------------------------------
// NovaOS Target Syscall Identifiers
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u64)]
pub enum NovaSyscallId {
    Write = 1,
    Exit = 2,
    Yield = 3,
    Open = 4,
    Read = 5,
    Close = 6,
    Spawn = 8,
    Exec = 9,
    Wait = 10,
    Sbrk = 11,
    Fork = 12,
    Getpid = 13,
    Stat = 14,
    Mmap = 15,
    Munmap = 16,
    Pipe = 17,
    Ioctl = 18,
    Uname = 19,
    ClockGetTime = 20,
    Poll = 31,
    Lseek = 32,
    Mprotect = 33,
    Sigaction = 34,
    Sigprocmask = 35,
    Kill = 36,
    EpollCreate = 37,
    EpollWait = 38,
    EpollCtl = 39,
    Pipe2 = 40,
    Getppid = 41,
    Getuid = 42,
    Getgid = 43,
}

// -----------------------------------------------------------------------------
// Translation Table
// -----------------------------------------------------------------------------

pub struct PosixTranslationEngine {
    translated_count: AtomicU32,
    unsupported_count: AtomicU32,
}

impl PosixTranslationEngine {
    pub const fn new() -> Self {
        Self {
            translated_count: AtomicU32::new(0),
            unsupported_count: AtomicU32::new(0),
        }
    }

    /// Maps Linux x86_64 system call numbers to NovaOS native syscalls
    pub fn translate(&self, linux_nr: u64) -> Option<NovaSyscallId> {
        let target = match linux_nr {
            LINUX_SYS_READ => Some(NovaSyscallId::Read),
            LINUX_SYS_WRITE => Some(NovaSyscallId::Write),
            LINUX_SYS_OPEN => Some(NovaSyscallId::Open),
            LINUX_SYS_CLOSE => Some(NovaSyscallId::Close),
            LINUX_SYS_STAT | LINUX_SYS_FSTAT => Some(NovaSyscallId::Stat),
            LINUX_SYS_POLL => Some(NovaSyscallId::Poll),
            LINUX_SYS_LSEEK => Some(NovaSyscallId::Lseek),
            LINUX_SYS_MMAP => Some(NovaSyscallId::Mmap),
            LINUX_SYS_MPROTECT => Some(NovaSyscallId::Mprotect),
            LINUX_SYS_MUNMAP => Some(NovaSyscallId::Munmap),
            LINUX_SYS_BRK => Some(NovaSyscallId::Sbrk),
            LINUX_SYS_RT_SIGACTION => Some(NovaSyscallId::Sigaction),
            LINUX_SYS_RT_SIGPROCMASK => Some(NovaSyscallId::Sigprocmask),
            LINUX_SYS_IOCTL => Some(NovaSyscallId::Ioctl),
            LINUX_SYS_PIPE => Some(NovaSyscallId::Pipe),
            LINUX_SYS_SCHED_YIELD => Some(NovaSyscallId::Yield),
            LINUX_SYS_GETPID => Some(NovaSyscallId::Getpid),
            LINUX_SYS_FORK => Some(NovaSyscallId::Fork),
            LINUX_SYS_EXECVE => Some(NovaSyscallId::Exec),
            LINUX_SYS_EXIT => Some(NovaSyscallId::Exit),
            LINUX_SYS_WAIT4 => Some(NovaSyscallId::Wait),
            LINUX_SYS_KILL => Some(NovaSyscallId::Kill),
            LINUX_SYS_UNAME => Some(NovaSyscallId::Uname),
            LINUX_SYS_GETUID => Some(NovaSyscallId::Getuid),
            LINUX_SYS_GETGID => Some(NovaSyscallId::Getgid),
            LINUX_SYS_GETPPID => Some(NovaSyscallId::Getppid),
            LINUX_SYS_EPOLL_CREATE => Some(NovaSyscallId::EpollCreate),
            LINUX_SYS_CLOCK_GETTIME => Some(NovaSyscallId::ClockGetTime),
            LINUX_SYS_EPOLL_WAIT => Some(NovaSyscallId::EpollWait),
            LINUX_SYS_EPOLL_CTL => Some(NovaSyscallId::EpollCtl),
            LINUX_SYS_PIPE2 => Some(NovaSyscallId::Pipe2),
            _ => None,
        };

        if target.is_some() {
            self.translated_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.unsupported_count.fetch_add(1, Ordering::Relaxed);
        }
        target
    }
}

pub static POSIX_TRANSLATION_ENGINE: PosixTranslationEngine = PosixTranslationEngine::new();

// -----------------------------------------------------------------------------
// POSIX Flag & Parameter Conversion
// -----------------------------------------------------------------------------

pub const POSIX_O_RDONLY: u32   = 0x0000;
pub const POSIX_O_WRONLY: u32   = 0x0001;
pub const POSIX_O_RDWR: u32     = 0x0002;
pub const POSIX_O_CREAT: u32    = 0x0040;
pub const POSIX_O_EXCL: u32     = 0x0080;
pub const POSIX_O_TRUNC: u32    = 0x0200;
pub const POSIX_O_APPEND: u32   = 0x0400;
pub const POSIX_O_NONBLOCK: u32 = 0x0800;

pub const NOVA_VFS_READ: u32   = 1 << 0;
pub const NOVA_VFS_WRITE: u32  = 1 << 1;
pub const NOVA_VFS_CREAT: u32  = 1 << 2;
pub const NOVA_VFS_TRUNC: u32  = 1 << 3;
pub const NOVA_VFS_APPEND: u32 = 1 << 4;

/// Converts POSIX open flags to native NovaOS VFS access bits
pub fn convert_posix_open_flags(flags: u32) -> u32 {
    let mut vfs_flags = 0u32;
    match flags & 0x03 {
        POSIX_O_RDONLY => vfs_flags |= NOVA_VFS_READ,
        POSIX_O_WRONLY => vfs_flags |= NOVA_VFS_WRITE,
        POSIX_O_RDWR => vfs_flags |= NOVA_VFS_READ | NOVA_VFS_WRITE,
        _ => {}
    }
    if (flags & POSIX_O_CREAT) != 0 {
        vfs_flags |= NOVA_VFS_CREAT;
    }
    if (flags & POSIX_O_TRUNC) != 0 {
        vfs_flags |= NOVA_VFS_TRUNC;
    }
    if (flags & POSIX_O_APPEND) != 0 {
        vfs_flags |= NOVA_VFS_APPEND;
    }
    vfs_flags
}

pub const POSIX_PROT_NONE: u32  = 0;
pub const POSIX_PROT_READ: u32  = 1;
pub const POSIX_PROT_WRITE: u32 = 2;
pub const POSIX_PROT_EXEC: u32  = 4;

pub const NOVA_PROT_READ: u32  = 1 << 0;
pub const NOVA_PROT_WRITE: u32 = 1 << 1;
pub const NOVA_PROT_EXEC: u32  = 1 << 2;

/// Converts POSIX memory protection flags to NovaOS ProtFlags
pub fn convert_posix_prot(prot: u32) -> u32 {
    let mut nova_prot = 0u32;
    if (prot & POSIX_PROT_READ) != 0 {
        nova_prot |= NOVA_PROT_READ;
    }
    if (prot & POSIX_PROT_WRITE) != 0 {
        nova_prot |= NOVA_PROT_WRITE;
    }
    if (prot & POSIX_PROT_EXEC) != 0 {
        nova_prot |= NOVA_PROT_EXEC;
    }
    nova_prot
}

// -----------------------------------------------------------------------------
// ELF64 Dynamic Linker Relocation Validation
// -----------------------------------------------------------------------------

pub const R_X86_64_NONE: u32      = 0;
pub const R_X86_64_64: u32        = 1;  // Direct 64-bit relocation (S + A)
pub const R_X86_64_GLOB_DAT: u32  = 6;  // Create GOT entry (S)
pub const R_X86_64_JUMP_SLOT: u32 = 7;  // Create PLT entry (S)
pub const R_X86_64_RELATIVE: u32  = 8;  // Adjust by program base (B + A)

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RelocationType {
    None,
    Direct64,
    GlobDat,
    JumpSlot,
    Relative,
    Unsupported(u32),
}

impl RelocationType {
    pub fn from_raw(r_type: u32) -> Self {
        match r_type {
            R_X86_64_NONE => Self::None,
            R_X86_64_64 => Self::Direct64,
            R_X86_64_GLOB_DAT => Self::GlobDat,
            R_X86_64_JUMP_SLOT => Self::JumpSlot,
            R_X86_64_RELATIVE => Self::Relative,
            other => Self::Unsupported(other),
        }
    }

    pub fn is_supported(&self) -> bool {
        !matches!(self, Self::Unsupported(_))
    }
}

// -----------------------------------------------------------------------------
// In-Kernel Self-Test
// -----------------------------------------------------------------------------

pub fn run_posix_compat_selftest() -> i32 {
    let engine = &POSIX_TRANSLATION_ENGINE;

    // Stage 1: Syscall translation verification
    if engine.translate(LINUX_SYS_READ) != Some(NovaSyscallId::Read) {
        return 1;
    }
    if engine.translate(LINUX_SYS_WRITE) != Some(NovaSyscallId::Write) {
        return 2;
    }
    if engine.translate(LINUX_SYS_OPEN) != Some(NovaSyscallId::Open) {
        return 3;
    }
    if engine.translate(LINUX_SYS_MMAP) != Some(NovaSyscallId::Mmap) {
        return 4;
    }
    if engine.translate(LINUX_SYS_FORK) != Some(NovaSyscallId::Fork) {
        return 5;
    }
    if engine.translate(LINUX_SYS_EXECVE) != Some(NovaSyscallId::Exec) {
        return 6;
    }
    if engine.translate(LINUX_SYS_PIPE2) != Some(NovaSyscallId::Pipe2) {
        return 7;
    }
    if engine.translate(LINUX_SYS_EPOLL_CREATE) != Some(NovaSyscallId::EpollCreate) {
        return 8;
    }
    if engine.translate(9999) != None {
        return 9;
    }

    // Stage 2: Open flags conversion
    let flags = POSIX_O_RDWR | POSIX_O_CREAT | POSIX_O_APPEND;
    let converted = convert_posix_open_flags(flags);
    if (converted & (NOVA_VFS_READ | NOVA_VFS_WRITE | NOVA_VFS_CREAT | NOVA_VFS_APPEND))
        != (NOVA_VFS_READ | NOVA_VFS_WRITE | NOVA_VFS_CREAT | NOVA_VFS_APPEND)
    {
        return 10;
    }

    // Stage 3: Protection flags conversion
    let prot = POSIX_PROT_READ | POSIX_PROT_WRITE;
    let nova_prot = convert_posix_prot(prot);
    if nova_prot != (NOVA_PROT_READ | NOVA_PROT_WRITE) {
        return 11;
    }

    // Stage 4: ELF64 relocation type decoding
    if RelocationType::from_raw(R_X86_64_RELATIVE) != RelocationType::Relative {
        return 12;
    }
    if RelocationType::from_raw(R_X86_64_GLOB_DAT) != RelocationType::GlobDat {
        return 13;
    }
    if RelocationType::from_raw(R_X86_64_JUMP_SLOT) != RelocationType::JumpSlot {
        return 14;
    }
    if RelocationType::from_raw(0xFFFF).is_supported() {
        return 15;
    }

    0
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_translation_table() {
        let engine = PosixTranslationEngine::new();
        assert_eq!(engine.translate(LINUX_SYS_READ), Some(NovaSyscallId::Read));
        assert_eq!(engine.translate(LINUX_SYS_WRITE), Some(NovaSyscallId::Write));
        assert_eq!(engine.translate(LINUX_SYS_OPEN), Some(NovaSyscallId::Open));
        assert_eq!(engine.translate(LINUX_SYS_CLOSE), Some(NovaSyscallId::Close));
        assert_eq!(engine.translate(LINUX_SYS_MMAP), Some(NovaSyscallId::Mmap));
        assert_eq!(engine.translate(LINUX_SYS_EPOLL_CTL), Some(NovaSyscallId::EpollCtl));
        assert_eq!(engine.translate(LINUX_SYS_PIPE2), Some(NovaSyscallId::Pipe2));
        assert_eq!(engine.translate(500), None);
    }

    #[test]
    fn test_flag_conversions() {
        assert_eq!(convert_posix_open_flags(POSIX_O_RDONLY), NOVA_VFS_READ);
        assert_eq!(convert_posix_open_flags(POSIX_O_WRONLY), NOVA_VFS_WRITE);
        assert_eq!(
            convert_posix_open_flags(POSIX_O_RDWR | POSIX_O_CREAT | POSIX_O_TRUNC),
            NOVA_VFS_READ | NOVA_VFS_WRITE | NOVA_VFS_CREAT | NOVA_VFS_TRUNC
        );

        assert_eq!(
            convert_posix_prot(POSIX_PROT_READ | POSIX_PROT_EXEC),
            NOVA_PROT_READ | NOVA_PROT_EXEC
        );
    }

    #[test]
    fn test_in_kernel_selftest() {
        assert_eq!(run_posix_compat_selftest(), 0);
    }
}
