//! userland/libs/nova-abi/src/syscall.rs - 64-Bit Fast Syscall Userland ABI
//!
//! Provides the official NovaOS 64-bit userland system call interface using hardware
//! `syscall`/`sysret` instructions:
//!   - Low-level primitives: `syscall0` through `syscall6` passing arguments in
//!     RAX, RDI, RSI, RDX, R10, R8, R9.
//!   - High-level ergonomic and type-safe abstractions (`write`, `read`, `open`, `mmap`, etc.).
//!   - Conversion of negative return values to strongly-typed `SyscallError`.

// Standalone / module compatibility: `#![no_std]` is set at crate root (userland/libs/nova-abi/src/lib.rs).

// -----------------------------------------------------------------------------
// Syscall Numbers (Identical to Kernel ABI)
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(u64)]
pub enum SyscallNumber {
    SysWrite = 1,
    SysExit = 2,
    SysYield = 3,
    SysOpen = 4,
    SysRead = 5,
    SysClose = 6,
    SysNetSend = 7,
    SysSpawn = 8,
    SysExec = 9,
    SysWait = 10,
    SysSbrk = 11,
    SysFork = 12,
    SysGetpid = 13,
    SysStat = 14,
    SysMmap = 15,
    SysMunmap = 16,
    SysPipe = 17,
    SysIoctl = 18,
    SysUname = 19,
    SysClockGetTime = 20,
    SysFbAcquire = 21,
    SysFbRelease = 22,
    SysFbPresent = 23,
    SysShmCreate = 24,
    SysShmAttach = 25,
    SysMsgSend = 26,
    SysMsgRecv = 27,
    SysAuditLog = 28,
    SysUpdateCheck = 29,
    SysUpdateApply = 30,
}

// -----------------------------------------------------------------------------
// Strongly-Typed Errors
// -----------------------------------------------------------------------------

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(isize)]
pub enum SyscallError {
    PermissionDenied = -1,    // EPERM
    NotFound = -2,            // ENOENT
    NoSuchProcess = -3,       // ESRCH
    Interrupted = -4,         // EINTR
    IoError = -5,             // EIO
    BadFileDescriptor = -9,   // EBADF
    WouldBlock = -11,         // EAGAIN
    OutOfMemory = -12,        // ENOMEM
    AccessDenied = -13,       // EACCES
    BadAddress = -14,         // EFAULT
    ResourceBusy = -16,       // EBUSY
    AlreadyExists = -17,      // EEXIST
    InvalidArgument = -22,    // EINVAL
    NotImplemented = -38,     // ENOSYS
    Unknown = -999,
}

impl SyscallError {
    pub fn from_isize(code: isize) -> Self {
        match code {
            -1 => Self::PermissionDenied,
            -2 => Self::NotFound,
            -3 => Self::NoSuchProcess,
            -4 => Self::Interrupted,
            -5 => Self::IoError,
            -9 => Self::BadFileDescriptor,
            -11 => Self::WouldBlock,
            -12 => Self::OutOfMemory,
            -13 => Self::AccessDenied,
            -14 => Self::BadAddress,
            -16 => Self::ResourceBusy,
            -17 => Self::AlreadyExists,
            -22 => Self::InvalidArgument,
            -38 => Self::NotImplemented,
            _ => Self::Unknown,
        }
    }
}

// -----------------------------------------------------------------------------
// Low-Level Syscall Invocations (Hardware `syscall` instruction)
// -----------------------------------------------------------------------------

/// Invokes a 64-bit system call with 0 arguments.
#[inline(always)]
pub unsafe fn syscall0(num: SyscallNumber) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num as u64,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_syscall_dispatch(num as u64, 0, 0, 0, 0, 0, 0);
    }
    ret
}

/// Invokes a 64-bit system call with 1 argument.
#[inline(always)]
pub unsafe fn syscall1(num: SyscallNumber, a1: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num as u64,
        in("rdi") a1,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_syscall_dispatch(num as u64, a1, 0, 0, 0, 0, 0);
    }
    ret
}

/// Invokes a 64-bit system call with 2 arguments.
#[inline(always)]
pub unsafe fn syscall2(num: SyscallNumber, a1: u64, a2: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num as u64,
        in("rdi") a1,
        in("rsi") a2,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_syscall_dispatch(num as u64, a1, a2, 0, 0, 0, 0);
    }
    ret
}

/// Invokes a 64-bit system call with 3 arguments.
#[inline(always)]
pub unsafe fn syscall3(num: SyscallNumber, a1: u64, a2: u64, a3: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num as u64,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_syscall_dispatch(num as u64, a1, a2, a3, 0, 0, 0);
    }
    ret
}

/// Invokes a 64-bit system call with 4 arguments.
#[inline(always)]
pub unsafe fn syscall4(num: SyscallNumber, a1: u64, a2: u64, a3: u64, a4: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num as u64,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_syscall_dispatch(num as u64, a1, a2, a3, a4, 0, 0);
    }
    ret
}

/// Invokes a 64-bit system call with 6 arguments.
#[inline(always)]
pub unsafe fn syscall6(
    num: SyscallNumber,
    a1: u64,
    a2: u64,
    a3: u64,
    a4: u64,
    a5: u64,
    a6: u64,
) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num as u64,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        in("r10") a4,
        in("r8") a5,
        in("r9") a6,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_syscall_dispatch(num as u64, a1, a2, a3, a4, a5, a6);
    }
    ret
}

// -----------------------------------------------------------------------------
// High-Level Ergonomic Userland APIs
// -----------------------------------------------------------------------------

/// Writes buffer to file descriptor (stdout = 1, stderr = 2).
pub fn sys_write(fd: i32, data: &[u8]) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall3(
            SyscallNumber::SysWrite,
            fd as u64,
            data.as_ptr() as u64,
            data.len() as u64,
        )
    };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Reads from file descriptor into destination buffer.
pub fn sys_read(fd: i32, buf: &mut [u8]) -> Result<usize, SyscallError> {
    let ret = unsafe {
        syscall3(
            SyscallNumber::SysRead,
            fd as u64,
            buf.as_mut_ptr() as u64,
            buf.len() as u64,
        )
    };
    if ret >= 0 {
        Ok(ret as usize)
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Opens a file path.
pub fn sys_open(path: &str, flags: u32) -> Result<i32, SyscallError> {
    let ret = unsafe {
        syscall2(
            SyscallNumber::SysOpen,
            path.as_ptr() as u64,
            flags as u64,
        )
    };
    if ret >= 0 {
        Ok(ret as i32)
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Closes a file descriptor.
pub fn sys_close(fd: i32) -> Result<(), SyscallError> {
    let ret = unsafe { syscall1(SyscallNumber::SysClose, fd as u64) };
    if ret >= 0 {
        Ok(())
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Returns current process PID.
pub fn sys_getpid() -> u32 {
    let ret = unsafe { syscall0(SyscallNumber::SysGetpid) };
    ret as u32
}

/// Surrenders CPU quantum voluntarily.
pub fn sys_yield() {
    unsafe {
        syscall0(SyscallNumber::SysYield);
    }
}

/// Forks the current process.
pub fn sys_fork() -> Result<u32, SyscallError> {
    let ret = unsafe { syscall0(SyscallNumber::SysFork) };
    if ret >= 0 {
        Ok(ret as u32)
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Maps virtual pages into userland memory space.
pub fn sys_mmap(
    hint: u64,
    len: usize,
    prot: u32,
    flags: u32,
    fd: i32,
    offset: u64,
) -> Result<u64, SyscallError> {
    let ret = unsafe {
        syscall6(
            SyscallNumber::SysMmap,
            hint,
            len as u64,
            prot as u64,
            flags as u64,
            fd as u64,
            offset,
        )
    };
    if ret >= 0 {
        Ok(ret as u64)
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Unmaps virtual pages from userland memory space.
pub fn sys_munmap(addr: u64, len: usize) -> Result<(), SyscallError> {
    let ret = unsafe { syscall2(SyscallNumber::SysMunmap, addr, len as u64) };
    if ret >= 0 {
        Ok(())
    } else {
        Err(SyscallError::from_isize(ret))
    }
}

/// Terminates current process with status code.
pub fn sys_exit(code: i32) -> ! {
    unsafe {
        syscall1(SyscallNumber::SysExit, code as u64);
    }
    loop {
        #[cfg(all(target_arch = "x86_64", not(test)))]
        unsafe {
            core::arch::asm!("ud2", options(noreturn));
        }
        #[cfg(any(not(target_arch = "x86_64"), test))]
        panic!("process exited with code {}", code);
    }
}

// -----------------------------------------------------------------------------
// Host & Test Mock Dispatcher
// -----------------------------------------------------------------------------

#[cfg(any(not(target_arch = "x86_64"), test))]
fn mock_syscall_dispatch(
    num: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    _a4: u64,
    _a5: u64,
    _a6: u64,
) -> isize {
    match num {
        1 => {
            // SysWrite: fd = a1, buf = a2, len = a3
            if a1 == 1 || a1 == 2 {
                a3 as isize
            } else {
                -9 // EBADF
            }
        }
        2 => 0,  // SysExit
        3 => 0,  // SysYield
        4 => 3,  // SysOpen -> handle 3
        5 => 0,  // SysRead
        6 => 0,  // SysClose
        12 => 2, // SysFork -> child PID 2
        13 => 1, // SysGetpid -> PID 1
        15 => {
            // SysMmap
            if a2 == 0 { -22 } else { 0x0000_7000_0000_0000 as isize }
        }
        16 => 0, // SysMunmap
        _ => -38, // ENOSYS
    }
}

// -----------------------------------------------------------------------------
// Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_userland_syscall_write() {
        let msg = b"Hello NovaOS 64-bit ABI!";
        let res = sys_write(1, msg);
        assert_eq!(res, Ok(msg.len()));

        let err_res = sys_write(99, msg);
        assert_eq!(err_res, Err(SyscallError::BadFileDescriptor));
    }

    #[test]
    fn test_userland_getpid_and_yield() {
        assert_eq!(sys_getpid(), 1);
        sys_yield();
    }

    #[test]
    fn test_userland_mmap_and_munmap() {
        let mapped = sys_mmap(0, 4096, 3, 0x22, -1, 0);
        assert_eq!(mapped, Ok(0x0000_7000_0000_0000));
        assert_eq!(sys_munmap(0x0000_7000_0000_0000, 4096), Ok(()));

        let invalid = sys_mmap(0, 0, 3, 0x22, -1, 0);
        assert_eq!(invalid, Err(SyscallError::InvalidArgument));
    }

    #[test]
    fn test_error_conversion() {
        assert_eq!(SyscallError::from_isize(-14), SyscallError::BadAddress);
        assert_eq!(SyscallError::from_isize(-38), SyscallError::NotImplemented);
        assert_eq!(SyscallError::from_isize(-1), SyscallError::PermissionDenied);
    }
}
