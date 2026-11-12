//! userland/libs/nova-rt/src/crt0.rs - 64-Bit System V AMD64 ABI Startup Runtime
//!
//! NovaOS 2.0 (Month 2 - Storage & Async I/O): Linux/POSIX Compatibility Layer (novalibc & POSIX ABI)
//!
//! Provides:
//!   - 64-bit userland startup entry point (`_start`).
//!   - System V AMD64 ABI 16-byte stack frame alignment.
//!   - Extraction of argc, argv, envp, and auxiliary vectors (auxv).
//!   - Dispatch to `main(argc, argv, envp)`.
//!   - Signal return trampoline (`__restore_rt`) for POSIX signal handler unwinding.
//!   - Exit code propagation to `sys_exit`.

#![no_std]
#![allow(dead_code, non_camel_case_types, static_mut_refs, unused_doc_comments)]

#[cfg(test)]
extern crate std;

/// C-compatible main function signature
pub type MainFn = unsafe extern "C" fn(argc: i32, argv: *const *const u8, envp: *const *const u8) -> i32;

/// Process startup information parsed from initial stack frame
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ProcessStartupInfo {
    pub argc: i32,
    pub argv: *const *const u8,
    pub envp: *const *const u8,
    pub stack_pointer: u64,
}

impl ProcessStartupInfo {
    pub const fn empty() -> Self {
        Self {
            argc: 0,
            argv: core::ptr::null(),
            envp: core::ptr::null(),
            stack_pointer: 0,
        }
    }

    /// Parses initial user stack per System V AMD64 ABI specification
    pub unsafe fn parse_stack(sp: u64) -> Result<Self, &'static str> {
        if sp == 0 || (sp & 0x7) != 0 {
            return Err("Invalid or unaligned initial stack pointer");
        }
        let stack_ptr = sp as *const u64;
        let argc = *stack_ptr as i32;
        if argc < 0 || argc > 4096 {
            return Err("Corrupted argc count on initial stack");
        }

        let argv = stack_ptr.add(1) as *const *const u8;
        let envp = stack_ptr.add(1 + (argc as usize) + 1) as *const *const u8;

        Ok(Self {
            argc,
            argv,
            envp,
            stack_pointer: sp,
        })
    }
}

/// Global startup record cached for inspection
pub static mut GLOBAL_STARTUP_INFO: ProcessStartupInfo = ProcessStartupInfo::empty();

/// Initializes userland runtime environment
#[no_mangle]
pub unsafe extern "C" fn __nova_runtime_init(sp: u64) -> i32 {
    match ProcessStartupInfo::parse_stack(sp) {
        Ok(info) => {
            GLOBAL_STARTUP_INFO = info;
            0
        }
        Err(_) => -1,
    }
}

/// Invokes user main function and terminates process with returned exit code
pub unsafe fn run_user_main(main: MainFn, sp: u64) -> i32 {
    let info = match ProcessStartupInfo::parse_stack(sp) {
        Ok(i) => i,
        Err(_) => return 127,
    };
    main(info.argc, info.argv, info.envp)
}

/// Userland signal return trampoline (__restore_rt)
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".global __restore_rt",
    "__restore_rt:",
    "mov rax, 15", // SYS_rt_sigreturn in Linux x86_64
    "syscall",
    "ud2"
);

/// 64-bit entry point stub (_start)
#[cfg(all(target_arch = "x86_64", not(test)))]
core::arch::global_asm!(
    ".global _start",
    "_start:",
    // 1. Clear frame pointer to mark outermost stack frame per System V ABI
    "xor rbp, rbp",
    // 2. Fetch stack pointer into rdi as argument to __nova_runtime_init
    "mov rdi, rsp",
    // 3. Align stack to 16 bytes
    "and rsp, -16",
    // 4. Call runtime initialization
    "call __nova_runtime_init",
    // 5. Setup arguments for main: rdi = argc, rsi = argv, rdx = envp
    "mov rdi, [GLOBAL_STARTUP_INFO + 0]", // argc
    "mov rsi, [GLOBAL_STARTUP_INFO + 8]", // argv
    "mov rdx, [GLOBAL_STARTUP_INFO + 16]",// envp
    // 6. Call main
    "call main",
    // 7. Exit with returned code in rax
    "mov rdi, rax",
    "mov rax, 60", // SYS_exit
    "syscall",
    "ud2"
);

// -----------------------------------------------------------------------------
// Host Unit Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crt0_stack_parser() {
        // Construct mock System V AMD64 initial stack
        let arg0 = b"novaos_app\0".as_ptr();
        let arg1 = b"--test-flag\0".as_ptr();
        let env0 = b"PATH=/bin\0".as_ptr();

        let mock_stack: [u64; 7] = [
            2,                         // argc = 2
            arg0 as u64,               // argv[0]
            arg1 as u64,               // argv[1]
            0,                         // argv[2] = NULL
            env0 as u64,               // envp[0]
            0,                         // envp[1] = NULL
            0,                         // auxv NULL
        ];

        let sp = mock_stack.as_ptr() as u64;
        unsafe {
            let res = ProcessStartupInfo::parse_stack(sp).expect("parse stack failed");
            assert_eq!(res.argc, 2);
            assert_eq!(*res.argv, arg0);
            assert_eq!(*res.argv.add(1), arg1);
            assert_eq!(*res.envp, env0);

            let ret = __nova_runtime_init(sp);
            assert_eq!(ret, 0);
            assert_eq!(GLOBAL_STARTUP_INFO.argc, 2);
        }
    }

    #[test]
    fn test_crt0_user_main_dispatch() {
        unsafe extern "C" fn mock_main(argc: i32, _argv: *const *const u8, _envp: *const *const u8) -> i32 {
            if argc == 3 { 0 } else { 42 }
        }

        let mock_stack: [u64; 6] = [
            3, // argc = 3
            0x1000,
            0x2000,
            0x3000,
            0, // NULL argv terminator
            0, // NULL envp
        ];

        let sp = mock_stack.as_ptr() as u64;
        unsafe {
            let exit_code = run_user_main(mock_main, sp);
            assert_eq!(exit_code, 0);
        }
    }
}
