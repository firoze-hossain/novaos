#!/bin/sh
# Month 1 - Foundation: host-side tests for Demand Paging & Copy-on-Write (CoW) Virtual Memory Engine
# 1. 48-bit VirtualAddress and PhysicalAddress validation, canonical checks, 4KB page alignment.
# 2. Virtual Memory Area (VmaRegion) layout, kinds (Anonymous, Stack, FileBacked, GuardPage, Shared), and ProtFlags.
# 3. Pure #![no_std] BTreeMap<VirtualAddress, VmaRegion> lookup, insertion, overlap, and removal.
# 4. Physical frame reference counting table (FrameRefTable) for zero-leak CoW tracking.
# 5. ASLR engine randomized virtual addresses (stack, heap, mmap).
# 6. Page Fault (#PF Exception 14) handler: Demand-zero paging, guard page violation, SIGSEGV,
#    CoW duplication, and CoW zero-copy in-place promotion.
# 7. Memory footprint reduction verification (>= 75% savings vs eager allocation).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_vmm_tests.sh      (or: make vmm-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/mm/vmm.rs host tests (VMA BTreeMap & Page Refcounting) ---"
    rustc --edition 2021 --test -o "$OUT/vmm_test" kernel/src/mm/vmm.rs
    "$OUT/vmm_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/vmm_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/mm/vmm.rs tests succeeded."

    echo "--- kernel/src/mm/fault.rs host tests (Demand Paging & CoW Dispatcher) ---"
    rustc --edition 2021 --test -o "$OUT/fault_test" kernel/src/mm/fault.rs
    "$OUT/fault_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/fault_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/mm/fault.rs tests succeeded."

    echo "[PASS] All Demand Paging & CoW Virtual Memory Engine host tests succeeded."
else
    echo "vmm-test: rustc not found - skipping the Rust suite"
fi
