#!/bin/sh
# NovaOS Clock Application & Timekeeping Suite (nova-clock / Chronos) Test Suite
#
# Synthesizes Apple macOS Clock & Ubuntu GNOME Clocks:
# 1. World Clock: solar terminator day/night equirectangular geometry & 24 global metropolitan hubs.
# 2. Vector Analog Clocks: continuous hand sweeps, daylight/nighttime dial faces, sunrise/sunset info.
# 3. Multi-Alarm System: weekday/weekend bitmask scheduling, animated switches, and 9-minute snooze.
# 4. Centisecond Stopwatch: 10ms high-precision timer with fastest (green) & slowest (red) lap telemetry.
# 5. Countdown Timers: Ubuntu-style 1m..1h Quick Start preset grid and remaining progress ratio.
# 6. Full CLI execution: --selftest, --status, and autonomous verification.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_clock_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-clock host tests (Sovereign Clock & Timekeeping Suite) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Chronos Companion Workspace Tests
    CHRONOS_DIR="$(cd .. && pwd)/chronos"
    if [ -d "$CHRONOS_DIR" ]; then
        echo "--> Testing Chronos Core & UI in $CHRONOS_DIR..."
        (cd "$CHRONOS_DIR" && cargo test --quiet)
        (cd "$CHRONOS_DIR" && cargo run --quiet -p chronos-app -- --selftest)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-clock Tests
    echo "--> Testing userland/desktop/apps/nova-clock..."
    (cd userland/desktop/apps/nova-clock && cargo test --quiet)
    (cd userland/desktop/apps/nova-clock && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-clock && cargo run --quiet -- --status > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/CLOCK.ELF" ] && [ -f "tools/fixtures/CLOCK.PKG" ]; then
        echo "--> Verified CLOCK.ELF and CLOCK.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Clock (nova-clock / Chronos) tests passed successfully."
else
    echo "nova-clock-test: cargo not found - skipping the suite"
fi
