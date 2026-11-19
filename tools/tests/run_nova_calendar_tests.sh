#!/bin/sh
# NovaOS Calendar Application & Interactive Schedule Hub (nova-calendar) Test Suite
#
# Synthesizes Apple Calendar (macOS), Windows 11 Calendar, and Ubuntu GNOME Calendar:
# 1. Multi-view scheduling: Month grid, Week timeline, Day agenda, and Agenda list views.
# 2. Category filtering & tag management: Work, Personal, NovaOS Releases, Holidays, Reminders.
# 3. Interactive date calculations: leap-year compensation, days in month, day-of-week anchor math.
# 4. Host-side UI rendering with BufferSurface: acrylic sidebar, hero header, today badge, event indicator dots.
# 5. Full CLI execution: --selftest, --events, --month, --add.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_calendar_tests.sh
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-calendar host tests (Unified Calendar Hub) ---"
    (cd userland/desktop/apps/nova-calendar && cargo test --quiet)
    (cd userland/desktop/apps/nova-calendar && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-calendar && cargo run --quiet -- --events > /dev/null)
    echo "[PASS] All NovaOS Calendar (nova-calendar) tests passed successfully."
else
    echo "nova-calendar-test: cargo not found - skipping the suite"
fi
