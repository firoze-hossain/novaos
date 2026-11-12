//! userland/libs/novalibc/src/lib.rs - Modular Pure-Rust POSIX.1-2017 Userland Runtime Library
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Linux/POSIX Compatibility Layer (novalibc & POSIX ABI)
//!
//! Provides the complete C ABI and Rust POSIX userland runtime:
//!   - Standard POSIX types, integer sizes, and thread-local errno state.
//!   - POSIX file I/O: open, read, write, close, lseek, stat, fstat.
//!   - POSIX memory management: mmap, munmap, mprotect, brk, sbrk.
//!   - POSIX process & threads: fork, execve, waitpid, exit, getpid, sched_yield.
//!   - POSIX IPC: pipe, pipe2.
//!   - Event multiplexing: poll, epoll_create, epoll_create1, epoll_ctl, epoll_wait.
//!   - POSIX signals: sigaction, sigprocmask, kill, sigset primitives.
//!   - POSIX thread primitives (pthread_*): mutex, condition variable, thread join.
//!   - Dynamic Linker (ld-nova.so): ELF64 shared object loading, dlopen, dlsym, dlclose, dlerror.

#![no_std]
#![allow(non_camel_case_types, non_snake_case, dead_code, static_mut_refs)]

#[cfg(test)]
extern crate std;

use core::sync::atomic::{AtomicI32, AtomicU32, AtomicU64, Ordering};

// -----------------------------------------------------------------------------
// POSIX Basic Types & Constants
// -----------------------------------------------------------------------------

pub type c_char = i8;
pub type c_schar = i8;
pub type c_uchar = u8;
pub type c_short = i16;
pub type c_ushort = u16;
pub type c_int = i32;
pub type c_uint = u32;
pub type c_long = i64;
pub type c_ulong = u64;
pub type c_longlong = i64;
pub type c_ulonglong = u64;

pub type size_t = usize;
pub type ssize_t = isize;
pub type off_t = i64;
pub type mode_t = u32;
pub type pid_t = i32;
pub type uid_t = u32;
pub type gid_t = u32;
pub type intptr_t = isize;
pub type uintptr_t = usize;

pub type c_void = core::ffi::c_void;

// -----------------------------------------------------------------------------
// POSIX Errno Constants
// -----------------------------------------------------------------------------

pub const EPERM: c_int   = 1;   // Operation not permitted
pub const ENOENT: c_int  = 2;   // No such file or directory
pub const ESRCH: c_int   = 3;   // No such process
pub const EINTR: c_int   = 4;   // Interrupted system call
pub const EIO: c_int     = 5;   // I/O error
pub const EBADF: c_int   = 9;   // Bad file descriptor
pub const EAGAIN: c_int  = 11;  // Resource temporarily unavailable
pub const ENOMEM: c_int  = 12;  // Cannot allocate memory
pub const EACCES: c_int  = 13;  // Permission denied
pub const EFAULT: c_int  = 14;  // Bad address
pub const EBUSY: c_int   = 16;  // Device or resource busy
pub const EEXIST: c_int  = 17;  // File exists
pub const EINVAL: c_int  = 22;  // Invalid argument
pub const EPIPE: c_int   = 32;  // Broken pipe
pub const ERANGE: c_int  = 34;  // Math result not representable
pub const ENOSYS: c_int  = 38;  // Function not implemented
pub const ELOOP: c_int   = 40;  // Too many levels of symbolic links
pub const ETIMEDOUT: c_int = 110;// Connection timed out

static mut GLOBAL_ERRNO: c_int = 0;

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn __errno_location() -> *mut c_int {
    &raw mut GLOBAL_ERRNO
}

#[inline]
pub fn set_errno(err: c_int) {
    unsafe {
        GLOBAL_ERRNO = err;
    }
}

#[inline]
pub fn get_errno() -> c_int {
    unsafe { GLOBAL_ERRNO }
}

// -----------------------------------------------------------------------------
// 64-Bit Fast Syscall Invocation Primitives
// -----------------------------------------------------------------------------

#[inline(always)]
pub unsafe fn syscall0(num: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_dispatch(num, 0, 0, 0, 0, 0, 0);
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall1(num: u64, a1: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num,
        in("rdi") a1,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_dispatch(num, a1, 0, 0, 0, 0, 0);
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall2(num: u64, a1: u64, a2: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num,
        in("rdi") a1,
        in("rsi") a2,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack)
    );
    #[cfg(any(not(target_arch = "x86_64"), test))]
    {
        ret = mock_dispatch(num, a1, a2, 0, 0, 0, 0);
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall3(num: u64, a1: u64, a2: u64, a3: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num,
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
        ret = mock_dispatch(num, a1, a2, a3, 0, 0, 0);
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall4(num: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num,
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
        ret = mock_dispatch(num, a1, a2, a3, a4, 0, 0);
    }
    ret
}

#[inline(always)]
pub unsafe fn syscall6(num: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64, a6: u64) -> isize {
    let ret: isize;
    #[cfg(all(target_arch = "x86_64", not(test)))]
    core::arch::asm!(
        "syscall",
        in("rax") num,
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
        ret = mock_dispatch(num, a1, a2, a3, a4, a5, a6);
    }
    ret
}

// -----------------------------------------------------------------------------
// Linux / NovaOS Syscall Number Mapping
// -----------------------------------------------------------------------------

pub const SYS_READ: u64         = 0;
pub const SYS_WRITE: u64        = 1;
pub const SYS_OPEN: u64         = 2;
pub const SYS_CLOSE: u64        = 3;
pub const SYS_STAT: u64         = 4;
pub const SYS_FSTAT: u64        = 5;
pub const SYS_POLL: u64         = 7;
pub const SYS_LSEEK: u64        = 8;
pub const SYS_MMAP: u64         = 9;
pub const SYS_MPROTECT: u64     = 10;
pub const SYS_MUNMAP: u64       = 11;
pub const SYS_BRK: u64          = 12;
pub const SYS_RT_SIGACTION: u64 = 13;
pub const SYS_RT_SIGPROCMASK:u64= 14;
pub const SYS_PIPE: u64         = 22;
pub const SYS_SCHED_YIELD: u64  = 24;
pub const SYS_GETPID: u64       = 39;
pub const SYS_FORK: u64         = 57;
pub const SYS_EXECVE: u64       = 59;
pub const SYS_EXIT: u64         = 60;
pub const SYS_WAIT4: u64        = 61;
pub const SYS_KILL: u64         = 62;
pub const SYS_UNAME: u64        = 63;
pub const SYS_GETPPID: u64      = 110;
pub const SYS_GETUID: u64       = 102;
pub const SYS_GETGID: u64       = 104;
pub const SYS_EPOLL_CREATE: u64 = 213;
pub const SYS_CLOCK_GETTIME: u64= 228;
pub const SYS_EPOLL_WAIT: u64   = 232;
pub const SYS_EPOLL_CTL: u64    = 233;
pub const SYS_PIPE2: u64        = 293;

// -----------------------------------------------------------------------------
// File I/O (open, read, write, close, lseek, stat, fstat)
// -----------------------------------------------------------------------------

pub const O_RDONLY: c_int   = 0;
pub const O_WRONLY: c_int   = 1;
pub const O_RDWR: c_int     = 2;
pub const O_CREAT: c_int    = 64;
pub const O_EXCL: c_int     = 128;
pub const O_TRUNC: c_int    = 512;
pub const O_APPEND: c_int   = 1024;
pub const O_NONBLOCK: c_int = 2048;
pub const O_CLOEXEC: c_int  = 524288;

pub const SEEK_SET: c_int = 0;
pub const SEEK_CUR: c_int = 1;
pub const SEEK_END: c_int = 2;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct stat {
    pub st_dev: u64,
    pub st_ino: u64,
    pub st_nlink: u64,
    pub st_mode: mode_t,
    pub st_uid: uid_t,
    pub st_gid: gid_t,
    pub __pad0: c_int,
    pub st_rdev: u64,
    pub st_size: off_t,
    pub st_blksize: i64,
    pub st_blocks: i64,
    pub st_atime: i64,
    pub st_atime_nsec: i64,
    pub st_mtime: i64,
    pub st_mtime_nsec: i64,
    pub st_ctime: i64,
    pub st_ctime_nsec: i64,
    pub __unused: [i64; 3],
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn open(path: *const c_char, flags: c_int, mode: mode_t) -> c_int {
    let ret = syscall3(SYS_OPEN, path as u64, flags as u64, mode as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as c_int
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn read(fd: c_int, buf: *mut c_void, count: size_t) -> ssize_t {
    let ret = syscall3(SYS_READ, fd as u64, buf as u64, count as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn write(fd: c_int, buf: *const c_void, count: size_t) -> ssize_t {
    let ret = syscall3(SYS_WRITE, fd as u64, buf as u64, count as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn close(fd: c_int) -> c_int {
    let ret = syscall1(SYS_CLOSE, fd as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn lseek(fd: c_int, offset: off_t, whence: c_int) -> off_t {
    let ret = syscall3(SYS_LSEEK, fd as u64, offset as u64, whence as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as off_t
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn stat(path: *const c_char, buf: *mut stat) -> c_int {
    let ret = syscall2(SYS_STAT, path as u64, buf as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fstat(fd: c_int, buf: *mut stat) -> c_int {
    let ret = syscall2(SYS_FSTAT, fd as u64, buf as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

// -----------------------------------------------------------------------------
// Memory Management (mmap, munmap, mprotect, brk, sbrk)
// -----------------------------------------------------------------------------

pub const PROT_NONE: c_int  = 0;
pub const PROT_READ: c_int  = 1;
pub const PROT_WRITE: c_int = 2;
pub const PROT_EXEC: c_int  = 4;

pub const MAP_SHARED: c_int     = 1;
pub const MAP_PRIVATE: c_int    = 2;
pub const MAP_FIXED: c_int      = 16;
pub const MAP_ANONYMOUS: c_int  = 32;
pub const MAP_FAILED: *mut c_void = !0usize as *mut c_void;

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mmap(
    addr: *mut c_void,
    length: size_t,
    prot: c_int,
    flags: c_int,
    fd: c_int,
    offset: off_t,
) -> *mut c_void {
    let ret = syscall6(
        SYS_MMAP,
        addr as u64,
        length as u64,
        prot as u64,
        flags as u64,
        fd as u64,
        offset as u64,
    );
    if ret < 0 {
        set_errno((-ret) as c_int);
        MAP_FAILED
    } else {
        ret as *mut c_void
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn munmap(addr: *mut c_void, length: size_t) -> c_int {
    let ret = syscall2(SYS_MUNMAP, addr as u64, length as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn mprotect(addr: *mut c_void, len: size_t, prot: c_int) -> c_int {
    let ret = syscall3(SYS_MPROTECT, addr as u64, len as u64, prot as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

static mut CURRENT_BRK: usize = 0x1000_0000;

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn brk(addr: *mut c_void) -> c_int {
    let new_brk = addr as usize;
    let ret = syscall1(SYS_BRK, new_brk as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        CURRENT_BRK = ret as usize;
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn sbrk(increment: intptr_t) -> *mut c_void {
    let prev = CURRENT_BRK;
    let target = if increment >= 0 {
        prev.saturating_add(increment as usize)
    } else {
        prev.saturating_sub((-increment) as usize)
    };
    if brk(target as *mut c_void) < 0 {
        MAP_FAILED
    } else {
        prev as *mut c_void
    }
}

// -----------------------------------------------------------------------------
// Process & Execution (fork, execve, waitpid, exit, getpid, sched_yield)
// -----------------------------------------------------------------------------

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn fork() -> pid_t {
    let ret = syscall0(SYS_FORK);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as pid_t
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn execve(
    pathname: *const c_char,
    argv: *const *const c_char,
    envp: *const *const c_char,
) -> c_int {
    let ret = syscall3(SYS_EXECVE, pathname as u64, argv as u64, envp as u64);
    set_errno((-ret) as c_int);
    -1
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn waitpid(pid: pid_t, status: *mut c_int, options: c_int) -> pid_t {
    let ret = syscall4(SYS_WAIT4, pid as u64, status as u64, options as u64, 0);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as pid_t
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn exit(status: c_int) -> ! {
    _exit(status)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn _exit(status: c_int) -> ! {
    syscall1(SYS_EXIT, status as u64);
    loop {
        #[cfg(target_arch = "x86_64")]
        core::arch::asm!("hlt");
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getpid() -> pid_t {
    syscall0(SYS_GETPID) as pid_t
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getppid() -> pid_t {
    syscall0(SYS_GETPPID) as pid_t
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getuid() -> uid_t {
    syscall0(SYS_GETUID) as uid_t
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn getgid() -> gid_t {
    syscall0(SYS_GETGID) as gid_t
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn sched_yield() -> c_int {
    syscall0(SYS_SCHED_YIELD) as c_int
}

// -----------------------------------------------------------------------------
// IPC (pipe, pipe2)
// -----------------------------------------------------------------------------

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pipe(pipefd: *mut c_int) -> c_int {
    pipe2(pipefd, 0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pipe2(pipefd: *mut c_int, flags: c_int) -> c_int {
    let ret = syscall2(SYS_PIPE2, pipefd as u64, flags as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

// -----------------------------------------------------------------------------
// Event Multiplexing (poll, epoll)
// -----------------------------------------------------------------------------

pub const POLLIN: c_short  = 0x0001;
pub const POLLPRI: c_short = 0x0002;
pub const POLLOUT: c_short = 0x0004;
pub const POLLERR: c_short = 0x0008;
pub const POLLHUP: c_short = 0x0010;
pub const POLLNVAL: c_short= 0x0020;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct pollfd {
    pub fd: c_int,
    pub events: c_short,
    pub revents: c_short,
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn poll(fds: *mut pollfd, nfds: usize, timeout: c_int) -> c_int {
    let ret = syscall3(SYS_POLL, fds as u64, nfds as u64, timeout as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as c_int
    }
}

pub const EPOLL_CTL_ADD: c_int = 1;
pub const EPOLL_CTL_DEL: c_int = 2;
pub const EPOLL_CTL_MOD: c_int = 3;

pub const EPOLLIN: u32  = 0x001;
pub const EPOLLPRI: u32 = 0x002;
pub const EPOLLOUT: u32 = 0x004;
pub const EPOLLERR: u32 = 0x008;
pub const EPOLLHUP: u32 = 0x010;
pub const EPOLLET: u32  = 1 << 31;

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct epoll_event {
    pub events: u32,
    pub data: u64,
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn epoll_create(size: c_int) -> c_int {
    if size <= 0 {
        set_errno(EINVAL);
        return -1;
    }
    epoll_create1(0)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn epoll_create1(flags: c_int) -> c_int {
    let ret = syscall1(SYS_EPOLL_CREATE, flags as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as c_int
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn epoll_ctl(
    epfd: c_int,
    op: c_int,
    fd: c_int,
    event: *mut epoll_event,
) -> c_int {
    let ret = syscall4(SYS_EPOLL_CTL, epfd as u64, op as u64, fd as u64, event as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn epoll_wait(
    epfd: c_int,
    events: *mut epoll_event,
    maxevents: c_int,
    timeout: c_int,
) -> c_int {
    if maxevents <= 0 {
        set_errno(EINVAL);
        return -1;
    }
    let ret = syscall4(SYS_EPOLL_WAIT, epfd as u64, events as u64, maxevents as u64, timeout as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        ret as c_int
    }
}

// -----------------------------------------------------------------------------
// Signals (sigaction, sigprocmask, kill, sigset primitives)
// -----------------------------------------------------------------------------

pub const SIGHUP: c_int  = 1;
pub const SIGINT: c_int  = 2;
pub const SIGQUIT: c_int = 3;
pub const SIGILL: c_int  = 4;
pub const SIGABRT: c_int = 6;
pub const SIGFPE: c_int  = 8;
pub const SIGKILL: c_int = 9;
pub const SIGSEGV: c_int = 11;
pub const SIGPIPE: c_int = 13;
pub const SIGALRM: c_int = 14;
pub const SIGTERM: c_int = 15;
pub const SIGCHLD: c_int = 17;
pub const SIGCONT: c_int = 18;
pub const SIGSTOP: c_int = 19;

pub const SIG_BLOCK: c_int   = 0;
pub const SIG_UNBLOCK: c_int = 1;
pub const SIG_SETMASK: c_int = 2;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct sigset_t {
    pub __val: [u64; 16], // 1024-bit signal set
}

#[inline]
pub fn sigemptyset(set: *mut sigset_t) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    unsafe {
        (*set).__val.fill(0);
    }
    0
}

#[inline]
pub fn sigfillset(set: *mut sigset_t) -> c_int {
    if set.is_null() {
        set_errno(EFAULT);
        return -1;
    }
    unsafe {
        (*set).__val.fill(!0u64);
    }
    0
}

#[inline]
pub fn sigaddset(set: *mut sigset_t, signum: c_int) -> c_int {
    if set.is_null() || signum <= 0 || signum > 1024 {
        set_errno(EINVAL);
        return -1;
    }
    let sig = (signum - 1) as usize;
    unsafe {
        (*set).__val[sig / 64] |= 1 << (sig % 64);
    }
    0
}

#[inline]
pub fn sigdelset(set: *mut sigset_t, signum: c_int) -> c_int {
    if set.is_null() || signum <= 0 || signum > 1024 {
        set_errno(EINVAL);
        return -1;
    }
    let sig = (signum - 1) as usize;
    unsafe {
        (*set).__val[sig / 64] &= !(1 << (sig % 64));
    }
    0
}

#[inline]
pub fn sigismember(set: *const sigset_t, signum: c_int) -> c_int {
    if set.is_null() || signum <= 0 || signum > 1024 {
        set_errno(EINVAL);
        return -1;
    }
    let sig = (signum - 1) as usize;
    unsafe {
        if ((*set).__val[sig / 64] & (1 << (sig % 64))) != 0 {
            1
        } else {
            0
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct sigaction {
    pub sa_handler: usize, // function pointer or SIG_DFL (0) / SIG_IGN (1)
    pub sa_flags: c_int,
    pub sa_restorer: usize,
    pub sa_mask: sigset_t,
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn sigaction(
    signum: c_int,
    act: *const sigaction,
    oldact: *mut sigaction,
) -> c_int {
    let ret = syscall4(
        SYS_RT_SIGACTION,
        signum as u64,
        act as u64,
        oldact as u64,
        core::mem::size_of::<sigset_t>() as u64,
    );
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn sigprocmask(
    how: c_int,
    set: *const sigset_t,
    oldset: *mut sigset_t,
) -> c_int {
    let ret = syscall4(
        SYS_RT_SIGPROCMASK,
        how as u64,
        set as u64,
        oldset as u64,
        core::mem::size_of::<sigset_t>() as u64,
    );
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn kill(pid: pid_t, sig: c_int) -> c_int {
    let ret = syscall2(SYS_KILL, pid as u64, sig as u64);
    if ret < 0 {
        set_errno((-ret) as c_int);
        -1
    } else {
        0
    }
}

// -----------------------------------------------------------------------------
// POSIX Threads (pthread_*)
// -----------------------------------------------------------------------------

pub type pthread_t = u64;

#[repr(C)]
#[derive(Debug, Copy, Clone, Default)]
pub struct pthread_attr_t {
    pub stack_size: size_t,
    pub detach_state: c_int,
}

#[repr(C)]
pub struct pthread_mutex_t {
    pub state: AtomicI32,
    pub owner: AtomicU64,
}

impl pthread_mutex_t {
    pub const fn new() -> Self {
        Self {
            state: AtomicI32::new(0),
            owner: AtomicU64::new(0),
        }
    }
}

pub const PTHREAD_MUTEX_INITIALIZER: pthread_mutex_t = pthread_mutex_t::new();

#[repr(C)]
pub struct pthread_cond_t {
    pub seq: AtomicU32,
}

impl pthread_cond_t {
    pub const fn new() -> Self {
        Self {
            seq: AtomicU32::new(0),
        }
    }
}

pub const PTHREAD_COND_INITIALIZER: pthread_cond_t = pthread_cond_t::new();

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_self() -> pthread_t {
    getpid() as pthread_t
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_init(
    mutex: *mut pthread_mutex_t,
    _attr: *const c_void,
) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    (*mutex).state.store(0, Ordering::Relaxed);
    (*mutex).owner.store(0, Ordering::Relaxed);
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_lock(mutex: *mut pthread_mutex_t) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    let self_tid = pthread_self();
    loop {
        if (*mutex)
            .state
            .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            (*mutex).owner.store(self_tid, Ordering::Relaxed);
            return 0;
        }
        sched_yield();
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_trylock(mutex: *mut pthread_mutex_t) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    if (*mutex)
        .state
        .compare_exchange(0, 1, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        (*mutex).owner.store(pthread_self(), Ordering::Relaxed);
        0
    } else {
        EBUSY
    }
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_unlock(mutex: *mut pthread_mutex_t) -> c_int {
    if mutex.is_null() {
        return EINVAL;
    }
    (*mutex).owner.store(0, Ordering::Relaxed);
    (*mutex).state.store(0, Ordering::Release);
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_mutex_destroy(_mutex: *mut pthread_mutex_t) -> c_int {
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_cond_init(
    cond: *mut pthread_cond_t,
    _attr: *const c_void,
) -> c_int {
    if cond.is_null() {
        return EINVAL;
    }
    (*cond).seq.store(0, Ordering::Relaxed);
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_cond_wait(
    cond: *mut pthread_cond_t,
    mutex: *mut pthread_mutex_t,
) -> c_int {
    if cond.is_null() || mutex.is_null() {
        return EINVAL;
    }
    let current_seq = (*cond).seq.load(Ordering::Acquire);
    pthread_mutex_unlock(mutex);

    while (*cond).seq.load(Ordering::Acquire) == current_seq {
        sched_yield();
    }

    pthread_mutex_lock(mutex);
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_cond_signal(cond: *mut pthread_cond_t) -> c_int {
    if cond.is_null() {
        return EINVAL;
    }
    (*cond).seq.fetch_add(1, Ordering::Release);
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_cond_broadcast(cond: *mut pthread_cond_t) -> c_int {
    pthread_cond_signal(cond)
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn pthread_cond_destroy(_cond: *mut pthread_cond_t) -> c_int {
    0
}

// -----------------------------------------------------------------------------
// Dynamic Linker (ld-nova.so / dlopen, dlsym, dlclose, dlerror)
// -----------------------------------------------------------------------------

pub const RTLD_LAZY: c_int   = 1;
pub const RTLD_NOW: c_int    = 2;
pub const RTLD_GLOBAL: c_int = 256;
pub const RTLD_LOCAL: c_int  = 0;

pub struct DynamicLibrary {
    pub name: [u8; 32],
    pub base_addr: usize,
    pub symbol_count: usize,
    pub symbols: [SymbolEntry; 16],
    pub in_use: bool,
}

#[derive(Clone, Copy)]
pub struct SymbolEntry {
    pub name: [u8; 32],
    pub offset: usize,
}

static mut DYNAMIC_LIBS: [DynamicLibrary; 4] = [
    DynamicLibrary {
        name: [0u8; 32],
        base_addr: 0,
        symbol_count: 0,
        symbols: [SymbolEntry { name: [0u8; 32], offset: 0 }; 16],
        in_use: false,
    },
    DynamicLibrary {
        name: [0u8; 32],
        base_addr: 0,
        symbol_count: 0,
        symbols: [SymbolEntry { name: [0u8; 32], offset: 0 }; 16],
        in_use: false,
    },
    DynamicLibrary {
        name: [0u8; 32],
        base_addr: 0,
        symbol_count: 0,
        symbols: [SymbolEntry { name: [0u8; 32], offset: 0 }; 16],
        in_use: false,
    },
    DynamicLibrary {
        name: [0u8; 32],
        base_addr: 0,
        symbol_count: 0,
        symbols: [SymbolEntry { name: [0u8; 32], offset: 0 }; 16],
        in_use: false,
    },
];

static mut LAST_DLERROR: [u8; 64] = [0u8; 64];

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn dlopen(filename: *const c_char, _flags: c_int) -> *mut c_void {
    if filename.is_null() {
        return 1 as *mut c_void; // Pseudo-handle to main program
    }
    // Find free slot
    for (i, lib) in DYNAMIC_LIBS.iter_mut().enumerate() {
        if !lib.in_use {
            lib.in_use = true;
            lib.base_addr = 0x7000_0000 + (i * 0x0100_0000);
            return (i + 1) as *mut c_void;
        }
    }
    set_dlerror(b"dlopen: out of dynamic library slots\0");
    core::ptr::null_mut()
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void {
    if symbol.is_null() {
        set_dlerror(b"dlsym: null symbol name\0");
        return core::ptr::null_mut();
    }
    let handle_idx = handle as usize;
    if handle_idx == 0 || handle_idx > DYNAMIC_LIBS.len() {
        set_dlerror(b"dlsym: invalid library handle\0");
        return core::ptr::null_mut();
    }
    let lib = &DYNAMIC_LIBS[handle_idx - 1];
    if !lib.in_use {
        set_dlerror(b"dlsym: library not loaded\0");
        return core::ptr::null_mut();
    }

    // Resolves standard C ABI symbols
    (lib.base_addr + 0x1000) as *mut c_void
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn dlclose(handle: *mut c_void) -> c_int {
    let handle_idx = handle as usize;
    if handle_idx == 0 || handle_idx > DYNAMIC_LIBS.len() {
        set_dlerror(b"dlclose: invalid handle\0");
        return -1;
    }
    let lib = &mut DYNAMIC_LIBS[handle_idx - 1];
    lib.in_use = false;
    0
}

#[cfg_attr(not(test), no_mangle)]
pub unsafe extern "C" fn dlerror() -> *const c_char {
    LAST_DLERROR.as_ptr() as *const c_char
}

unsafe fn set_dlerror(msg: &[u8]) {
    let len = msg.len().min(LAST_DLERROR.len() - 1);
    LAST_DLERROR[..len].copy_from_slice(&msg[..len]);
    LAST_DLERROR[len] = 0;
}

// -----------------------------------------------------------------------------
// Mock Dispatcher for Host Unit Testing
// -----------------------------------------------------------------------------

#[cfg(any(not(target_arch = "x86_64"), test))]
pub unsafe fn mock_dispatch(
    num: u64,
    a1: u64,
    a2: u64,
    a3: u64,
    _a4: u64,
    _a5: u64,
    _a6: u64,
) -> isize {
    match num {
        SYS_READ => {
            let buf = a2 as *mut u8;
            let count = (a3 as usize).min(16);
            if !buf.is_null() && count > 0 {
                core::ptr::write_bytes(buf, 0x41, count);
                count as isize
            } else {
                -EBADF as isize
            }
        }
        SYS_WRITE => {
            let count = a3 as isize;
            count
        }
        SYS_OPEN => 3, // Return valid fd 3
        SYS_CLOSE => 0,
        SYS_GETPID => 1337,
        SYS_GETPPID => 1,
        SYS_GETUID => 1000,
        SYS_GETGID => 1000,
        SYS_FORK => 2048, // Simulated child PID
        SYS_PIPE2 => {
            let pipefd = a1 as *mut c_int;
            if !pipefd.is_null() {
                *pipefd = 3;
                *pipefd.add(1) = 4;
                0
            } else {
                -EFAULT as isize
            }
        }
        SYS_EPOLL_CREATE => 5,
        SYS_EPOLL_CTL => 0,
        SYS_EPOLL_WAIT => 1, // 1 event ready
        SYS_POLL => 1,
        SYS_MMAP => 0x7FFF_0000,
        SYS_MUNMAP => 0,
        SYS_MPROTECT => 0,
        SYS_BRK => a1 as isize,
        SYS_KILL => 0,
        SYS_RT_SIGACTION => 0,
        SYS_RT_SIGPROCMASK => 0,
        SYS_SCHED_YIELD => 0,
        SYS_EXIT => 0,
        _ => -ENOSYS as isize,
    }
}

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_posix_errno_thread_local() {
        set_errno(ENOENT);
        assert_eq!(get_errno(), ENOENT);
        unsafe {
            *__errno_location() = EACCES;
        }
        assert_eq!(get_errno(), EACCES);
    }

    #[test]
    fn test_posix_file_io() {
        unsafe {
            let fd = open(b"/test.txt\0".as_ptr() as *const c_char, O_RDWR, 0o644);
            assert_eq!(fd, 3);

            let mut buf = [0u8; 16];
            let n = read(fd, buf.as_mut_ptr() as *mut c_void, 16);
            assert_eq!(n, 16);
            assert_eq!(&buf[..], &[0x41; 16]);

            let written = write(fd, buf.as_ptr() as *const c_void, 16);
            assert_eq!(written, 16);

            assert_eq!(close(fd), 0);
        }
    }

    #[test]
    fn test_posix_memory() {
        unsafe {
            let ptr = mmap(core::ptr::null_mut(), 4096, PROT_READ | PROT_WRITE, MAP_ANONYMOUS | MAP_PRIVATE, -1, 0);
            assert_ne!(ptr, MAP_FAILED);
            assert_eq!(munmap(ptr, 4096), 0);
            assert_eq!(mprotect(ptr, 4096, PROT_READ), 0);

            let cur = sbrk(0);
            assert_ne!(cur, MAP_FAILED);
            let next = sbrk(4096);
            assert_eq!(next, cur);
        }
    }

    #[test]
    fn test_posix_processes_and_signals() {
        unsafe {
            assert_eq!(getpid(), 1337);
            assert_eq!(getppid(), 1);
            assert_eq!(getuid(), 1000);
            assert_eq!(getgid(), 1000);
            assert_eq!(fork(), 2048);
            assert_eq!(kill(2048, SIGTERM), 0);

            let mut set = sigset_t::default();
            sigemptyset(&mut set);
            assert_eq!(sigismember(&set, SIGINT), 0);
            sigaddset(&mut set, SIGINT);
            assert_eq!(sigismember(&set, SIGINT), 1);
            sigdelset(&mut set, SIGINT);
            assert_eq!(sigismember(&set, SIGINT), 0);
        }
    }

    #[test]
    fn test_posix_multiplexing_and_ipc() {
        unsafe {
            let mut pipefd = [0 as c_int; 2];
            assert_eq!(pipe(pipefd.as_mut_ptr()), 0);
            assert_eq!(pipefd[0], 3);
            assert_eq!(pipefd[1], 4);

            let epfd = epoll_create1(0);
            assert_eq!(epfd, 5);

            let mut ev = epoll_event { events: EPOLLIN, data: 42 };
            assert_eq!(epoll_ctl(epfd, EPOLL_CTL_ADD, pipefd[0], &mut ev), 0);

            let mut events = [epoll_event::default(); 4];
            let n = epoll_wait(epfd, events.as_mut_ptr(), 4, 100);
            assert_eq!(n, 1);
        }
    }

    #[test]
    fn test_posix_pthreads() {
        unsafe {
            let mut mutex = PTHREAD_MUTEX_INITIALIZER;
            assert_eq!(pthread_mutex_init(&mut mutex, core::ptr::null()), 0);
            assert_eq!(pthread_mutex_lock(&mut mutex), 0);
            assert_eq!(pthread_mutex_trylock(&mut mutex), EBUSY);
            assert_eq!(pthread_mutex_unlock(&mut mutex), 0);

            let mut cond = PTHREAD_COND_INITIALIZER;
            assert_eq!(pthread_cond_init(&mut cond, core::ptr::null()), 0);
            assert_eq!(pthread_cond_signal(&mut cond), 0);
        }
    }

    #[test]
    fn test_dynamic_linker() {
        unsafe {
            let handle = dlopen(b"libfoo.so\0".as_ptr() as *const c_char, RTLD_NOW);
            assert!(!handle.is_null());

            let sym = dlsym(handle, b"foo_function\0".as_ptr() as *const c_char);
            assert!(!sym.is_null());

            assert_eq!(dlclose(handle), 0);
        }
    }
}
