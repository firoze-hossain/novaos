#!/bin/sh
# Month 2 - Storage & Async I/O: host-side tests for Linux/POSIX Compatibility Layer (novalibc & POSIX ABI)
# 1. novalibc C ABI system call wrappers (open, read, write, close, lseek, stat, fstat).
# 2. novalibc thread-local / reentrant errno implementation (__errno_location).
# 3. novalibc memory management (mmap, munmap, mprotect, brk, sbrk).
# 4. novalibc process & execution primitives (fork, execve, waitpid, exit, getpid, sched_yield).
# 5. novalibc IPC and event multiplexing (pipe, pipe2, poll, epoll_create1, epoll_ctl, epoll_wait).
# 6. novalibc POSIX signals (sigaction, sigprocmask, kill, sigset operations).
# 7. novalibc pthread primitives (pthread_mutex, pthread_cond, pthread_self).
# 8. novalibc dynamic linker (ld-nova.so: dlopen, dlsym, dlclose, dlerror).
# 9. nova-rt crt0 startup runtime (System V AMD64 16-byte stack frame alignment, argc/argv/envp extraction).
# 10. kernel-side POSIX translation table (mapping Linux x86_64 syscalls to NovaOS kernel handlers).
# 11. POSIX open and protection flags conversion.
# 12. ELF64 dynamic relocation type validation (R_X86_64_RELATIVE, GLOB_DAT, JUMP_SLOT).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_posix_compat_tests.sh      (or: make posix-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/libs/novalibc/src/lib.rs host tests (POSIX.1-2017 novalibc Runtime) ---"
    rustc --edition 2021 --test -o "$OUT/novalibc_test" userland/libs/novalibc/src/lib.rs
    "$OUT/novalibc_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/novalibc_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All userland/libs/novalibc/src/lib.rs tests succeeded."

    echo "--- userland/libs/nova-rt/src/crt0.rs host tests (System V AMD64 crt0 Runtime) ---"
    rustc --edition 2021 --test -o "$OUT/nova_rt_test" userland/libs/nova-rt/src/crt0.rs
    "$OUT/nova_rt_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/nova_rt_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All userland/libs/nova-rt/src/crt0.rs tests succeeded."

    echo "--- kernel/src/compat/posix.rs host tests (POSIX Translation Table & Relocations) ---"
    rustc --edition 2021 --test -o "$OUT/posix_compat_test" kernel/src/compat/posix.rs
    "$OUT/posix_compat_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/posix_compat_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/compat/posix.rs tests succeeded."

    echo "[PASS] All Linux/POSIX Compatibility Layer (novalibc & POSIX ABI) host tests succeeded."
else
    echo "posix-test: rustc not found - skipping the Rust suite"
fi
