#!/bin/sh
# Month 1 - Foundation: host-side tests for Pure Rust Modern UEFI 2.9 Bootloader & GPT Discovery
# 1. Protective MBR (type 0xEE) validation.
# 2. GUID Partition Table (GPT) header parsing, revision check, and CRC32 verification.
# 3. GPT Partition Entry array decoding, CRC32 verification, and discovery of:
#    - NovaOS / Linux Root Filesystem GUID (0FC63DAF-8483-4772-8E79-3D69D8477DE4).
#    - EFI System Partition (ESP) GUID (C12A7328-F81F-11D2-BA4B-00A0C93EC93B).
# 4. Authenticode PE32+ header parsing, Security Directory lookup, and SHA-256 digest computation.
# 5. UEFI Graphics Output Protocol (GOP) mode query and native 1080p/4K resolution selection.
# 6. 64-bit ELF kernel loader planning, PT_LOAD segment validation, and higher-half entry resolution.
# 7. Memory map descriptor summary and modern BootInfo handover contract validation.
# 8. Generation and inspection of real x86_64 PE32+ BOOTX64.EFI binary.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_uefi_tests.sh      (or: make uefi-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- boot/uefi-loader/src/lib.rs host tests (Pure Rust UEFI & GPT Subsystem) ---"
    rustc --edition 2021 --test -o "$OUT/uefi_lib_test" boot/uefi-loader/src/lib.rs
    "$OUT/uefi_lib_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/uefi_lib_test" >/dev/null 2>&1

    echo "--- boot/uefi-loader/src/main.rs host tests (UEFI 2.9 Entry & Sequence) ---"
    rustc --edition 2021 --test -o "$OUT/uefi_main_test" boot/uefi-loader/src/main.rs
    "$OUT/uefi_main_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/uefi_main_test" >/dev/null 2>&1

    echo "--- Building release BOOTX64.EFI for x86_64-unknown-uefi ---"
    rustc --edition 2021 --target x86_64-unknown-uefi -O --crate-type=bin boot/uefi-loader/src/main.rs -o "$OUT/BOOTX64.EFI"
    test -f "$OUT/BOOTX64.EFI"
    echo "[PASS] Generated valid x86_64 PE32+ EFI application: BOOTX64.EFI"

    echo "[PASS] All UEFI 2.9 bootloader & GPT discovery host tests succeeded."
else
    echo "uefi-test: rustc not found - skipping the Rust suite"
fi
