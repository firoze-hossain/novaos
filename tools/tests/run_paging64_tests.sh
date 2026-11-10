#!/bin/sh
# Month 1 - Foundation: host-side tests for Pure Rust x86_64 Long Mode & PML4 4-Level Paging Engine
# 1. Canonical 48-bit sign-extended address validation and filtering.
# 2. PML4, PDPT, PD, PT 9-bit index and 12-bit offset extraction.
# 3. Hardware NX (No-Execute bit 63), Present, Writable, User/Supervisor, Huge page flags.
# 4. Zero-unsafe PageTableWalker translation across 4KB standard, 2MB huge, and 1GB huge pages.
# 5. Page Fault Error Code (PFEC) bitfield decoding (P, W/R, U/S, RSVD, I/D, PK, SS).
# 6. Long Mode register configuration (CR3, CR4.PAE/PSE/PGE, EFER.LME/NXE/SCE, CR0.PG/WP/PE).
# 7. Boot PML4 environment: 0-4GB identity map with 2MB pages and higher-half base (0xFFFF_8000_0000_0000).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_paging64_tests.sh      (or: make paging64-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/arch/x86_64/paging.rs host tests (Pure Rust x86_64 Long Mode Paging) ---"
    rustc --edition 2021 --test -o "$OUT/paging64_test" kernel/src/arch/x86_64/paging.rs
    "$OUT/paging64_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/paging64_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/arch/x86_64/paging.rs host tests succeeded."
else
    echo "paging64-test: rustc not found - skipping the Rust suite"
fi
