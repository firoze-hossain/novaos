#!/bin/sh
# Month 1 - Foundation: host-side tests for Fast Syscall/Sysret MSR Dispatcher & 64-Bit ABI
# 1. Kernel MSR configuration (IA32_STAR, IA32_LSTAR, IA32_FMASK, IA32_KERNEL_GS_BASE).
# 2. Strict 64-bit canonical userland boundary validation (rejecting non-canonical & kernel space).
# 3. 64-bit Syscall Dispatcher with callee-saved register (RBX, RBP, R12-R15) preservation.
# 4. Latency advantage verification (~70 cycles vs ~450 cycles).
# 5. Userland 64-bit ABI library (userland/libs/nova-abi): syscall0..6 wrappers, safe abstractions,
#    and POSIX/NovaOS error conversion.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_syscall64_tests.sh      (or: make syscall64-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/arch/x86_64/syscall.rs host tests (Kernel MSR Dispatcher) ---"
    rustc --edition 2021 --test -o "$OUT/syscall64_test" kernel/src/arch/x86_64/syscall.rs
    "$OUT/syscall64_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/syscall64_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/arch/x86_64/syscall.rs tests succeeded."

    echo "--- userland/libs/nova-abi host tests (Userland 64-Bit ABI) ---"
    rustc --edition 2021 --test -o "$OUT/abi_test" userland/libs/nova-abi/src/syscall.rs
    "$OUT/abi_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/abi_test" >/dev/null 2>&1
    echo "[PASS] All userland/libs/nova-abi tests succeeded."

    echo "--- Building userland/libs/nova-abi rlib ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libnova_abi.rlib" userland/libs/nova-abi/src/lib.rs
    test -f "$OUT/libnova_abi.rlib"
    echo "[PASS] Generated valid libnova_abi.rlib"

    echo "[PASS] All Fast Syscall/Sysret MSR & 64-Bit ABI host tests succeeded."
else
    echo "syscall64-test: rustc not found - skipping the Rust suite"
fi
