#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Headless Server Kernel Profile & NUMA Tuning
# 1. Headless Hardware Profile:
#    - Strips all display, graphics, DRM/KMS, and audio modules.
#    - Enables serial console and high-throughput network-first I/O.
# 2. NUMA-Aware Physical Memory Allocation:
#    - Multi-node topology modeling with distance matrices (ACPI SRAT/SLIT).
#    - Policies: LocalPreferred, LocalStrict, Interleave, and Bind.
#    - Proximity-based fallback allocation penalty minimization.
# 3. 2MB and 1GB Huge Pages Support:
#    - Dedicated physical huge frame allocator.
#    - Strict 2MB (0x200000) and 1GB (0x40000000) boundary alignment validation.
#    - TLB miss reduction for container workloads.
# 4. Aggressive TCP Buffer Auto-Tuning:
#    - Socket backlog tuned to 4096 (eliminates SYN drops).
#    - Dynamic TCP receive/transmit buffer auto-tuning up to 16MB via Bandwidth-Delay Product (BDP).
#    - RFC 7323 TCP window scaling factor computation.
# 5. Full Tickless Idle Scheduling (NO_HZ_FULL):
#    - Suppresses timer ticks during CPU idle and uninterrupted single-task execution.
#    - Replaces fixed ticks with APIC one-shot deadlines.
# 6. Memory Efficiency & Ultra-Fast Boot:
#    - Dedicates >= 99.5% of RAM to application workloads.
#    - Boot budget verification (< 800ms).
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_server_profile_tests.sh      (or: make server-profile-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/config/server.rs host tests (Headless Server Profile & NUMA Tuning) ---"
    rustc --edition 2021 --test -o "$OUT/server_profile_test" kernel/src/config/server.rs
    "$OUT/server_profile_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/server_profile_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/config/server.rs tests succeeded."

    echo "--- Verifying Makefile kernel-server build target ---"
    make kernel-server --quiet
    echo "[PASS] Dedicated headless server kernel binary compiled successfully."

    echo "[PASS] All Headless Server Kernel Profile & NUMA Tuning tests passed successfully."
else
    echo "server-profile-test: rustc not found - skipping the Rust suite"
fi
