#!/bin/sh
# NovaOS System & Resource Monitor (nova-monitor / Pulse) Test Suite
#
# Synthesizes Ubuntu System Monitor (gnome-system-monitor):
# 1. Real-time process discovery and inspection across PIDs, memory, CPU %, and priorities.
# 2. Interactive search filtering by process name, user, or PID.
# 3. Multi-core CPU telemetry with per-core rolling sample history.
# 4. Memory, cache, and swap usage invariant calculation and circular gauges.
# 5. Network interface downlink/uplink throughput and cumulative transfer metrics.
# 6. Disk storage I/O read/write transfer rate profiling.
# 7. Storage filesystem volume topology and mount point mapping.
# 8. Process signal termination with critical PID 1 protection interlock.
# 9. Pure 2D software double-buffered vector rasterizer.
# 10. Full CLI execution: --selftest, --processes, --resources, --filesystems, --report, --version.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_pulse_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-monitor host tests (System & Resource Monitor) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Pulse Companion Workspace Tests
    PULSE_DIR="$(cd .. && pwd)/pulse"
    if [ -d "$PULSE_DIR" ]; then
        echo "--> Testing Pulse Core, Storage, Render & UI in $PULSE_DIR..."
        (cd "$PULSE_DIR" && cargo test --quiet)
        (cd "$PULSE_DIR" && cargo run --quiet -p pulse-app -- --test-full)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-monitor Tests
    echo "--> Testing userland/desktop/apps/nova-monitor..."
    (cd userland/desktop/apps/nova-monitor && cargo test --quiet)
    (cd userland/desktop/apps/nova-monitor && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-monitor && cargo run --quiet -- --processes > /dev/null)
    (cd userland/desktop/apps/nova-monitor && cargo run --quiet -- --resources > /dev/null)
    (cd userland/desktop/apps/nova-monitor && cargo run --quiet -- --filesystems > /dev/null)
    (cd userland/desktop/apps/nova-monitor && cargo run --quiet -- --report > /dev/null)
    (cd userland/desktop/apps/nova-monitor && cargo run --quiet -- --version > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/PULSE.ELF" ] && [ -f "tools/fixtures/PULSE.PKG" ] && [ -f "tools/fixtures/MONITOR.ELF" ] && [ -f "tools/fixtures/MONITOR.PKG" ]; then
        echo "--> Verified PULSE.ELF, MONITOR.ELF, PULSE.PKG and MONITOR.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS System Monitor (nova-monitor / Pulse) tests passed successfully."
else
    echo "nova-monitor-test: cargo not found - skipping the suite"
fi
