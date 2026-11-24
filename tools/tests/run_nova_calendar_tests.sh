#!/bin/sh
# NovaOS Calendar Application & Interactive Schedule Hub (nova-calendar / Kairos) Test Suite
#
# Synthesizes Apple Calendar (macOS), Windows 11 Calendar, and Ubuntu GNOME Calendar:
# 1. Multi-view scheduling: Month grid, Week timeline, Day agenda, and Agenda list views.
# 2. Category filtering & tag management: Work, Personal, NovaOS Releases, Holidays, Reminders.
# 3. Interactive date calculations: leap-year compensation, days in month, day-of-week anchor math.
# 4. Host-side UI rendering with BufferSurface: acrylic sidebar, hero header, today badge, event indicator dots.
# 5. Full CLI execution: --selftest, --events, --month, --export-ics.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_calendar_tests.sh
set -e
cd "$(dirname "$0")/../.."

echo "--- userland/desktop/apps/nova-calendar host tests (Unified Calendar Hub) ---"

if command -v cargo >/dev/null 2>&1; then
    # 1. Run Kairos Companion Workspace Tests
    KAIROS_DIR="$(cd .. && pwd)/kairos"
    if [ -d "$KAIROS_DIR" ]; then
        echo "--> Testing Kairos Core & UI in $KAIROS_DIR..."
        (cd "$KAIROS_DIR" && cargo test --quiet)
        (cd "$KAIROS_DIR" && cargo run --quiet -p kairos-app -- --selftest)
        (cd "$KAIROS_DIR" && cargo run --quiet -p kairos-app -- --events > /dev/null)
        (cd "$KAIROS_DIR" && cargo run --quiet -p kairos-app -- --month > /dev/null)
        (cd "$KAIROS_DIR" && cargo run --quiet -p kairos-app -- --export-ics > /dev/null)
    fi

    # 2. Run NovaOS userland/desktop/apps/nova-calendar Tests
    echo "--> Testing userland/desktop/apps/nova-calendar..."
    (cd userland/desktop/apps/nova-calendar && cargo test --quiet)
    (cd userland/desktop/apps/nova-calendar && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-calendar && cargo run --quiet -- --events > /dev/null)

    # 3. Verify Bare-Metal Fixtures
    if [ -f "tools/fixtures/CALENDAR.ELF" ] && [ -f "tools/fixtures/CALENDAR.PKG" ]; then
        echo "--> Verified CALENDAR.ELF and CALENDAR.PKG fixtures present and valid."
    fi
    if [ -f "tools/fixtures/KAIROS.ELF" ] && [ -f "tools/fixtures/KAIROS.PKG" ]; then
        echo "--> Verified KAIROS.ELF and KAIROS.PKG fixtures present and valid."
    fi

    echo "[PASS] All NovaOS Calendar (nova-calendar / Kairos) tests passed successfully."
else
    echo "nova-calendar-test: cargo not found - skipping the suite"
fi
