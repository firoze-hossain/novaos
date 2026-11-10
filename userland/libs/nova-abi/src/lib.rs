//! userland/libs/nova-abi/src/lib.rs - NovaOS 64-Bit Userland ABI Library
//!
//! Provides the canonical userland interface for NovaOS 64-bit Long Mode system calls.

#![cfg_attr(not(test), no_std)]

pub mod syscall;

pub use syscall::{
    sys_close, sys_exit, sys_fork, sys_getpid, sys_mmap, sys_munmap, sys_open, sys_read,
    sys_write, sys_yield, syscall0, syscall1, syscall2, syscall3, syscall4, syscall6,
    SyscallError, SyscallNumber,
};
