#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Systemd-Style Service & Daemon Orchestrator (novad)
#
# Validates modern service supervision and init system features:
# 1. Declarative Unit Parsing:
#    - .service, .socket, and .target INI unit configuration syntax
#    - Directives: Requires=, Wants=, BindsTo=, Conflicts=, Before=, After=
#    - Service types: simple, forking, oneshot, notify
#    - Restart policies: no, always, on-success, on-failure, on-abnormal, on-watchdog
# 2. Dependency Graph Resolution:
#    - Directed Acyclic Graph (DAG) compilation
#    - Ordering cycle detection and graceful cycle breaking
#    - Deterministic parallel startup waves (topological concurrency)
#    - Strict requirement failure and conflict propagation
# 3. On-Demand Socket Activation:
#    - Pre-binding network listening sockets (ListenStream=)
#    - Dynamic activation of dormant services upon arrival of network/IPC connections
# 4. Health Check Watchdogs:
#    - Watchdog timer tracking and heartbeat processing (sd_notify WATCHDOG=1)
#    - Automatic termination and restart on unresponsive watchdog timeout
# 5. Automatic Crash Recovery & Exponential Backoff:
#    - Exponential backoff scaling with max cap
#    - StartLimitBurst and StartLimitIntervalSec fork-bomb and thrash prevention
# 6. Graceful Reverse Topological Shutdown:
#    - Clean shutdown ordering in exact reverse waves of startup
# 7. Complete CLI Integration (novad and novactl service):
#    - novad daemon, start, stop, restart, reload, status, list, boot, shutdown, --selftest
#    - novactl service start, stop, restart, status, list, boot, shutdown
#
# Usage:
#   ./tools/tests/run_novad_tests.sh      (or: make novad-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/server/novad host tests (Systemd-Style Service Orchestrator) ---"

    # Set isolated test state file
    TEST_STATE="/tmp/novad_ci_test_state_$$.json"
    export NOVAD_STATE_FILE="$TEST_STATE"
    trap 'rm -f "$TEST_STATE"' EXIT

    # 1. Run novad Cargo unit test suite
    echo "[1/5] Running novad Cargo unit test suite..."
    (cd userland/server/novad && cargo test --quiet)

    # 2. Run novad automated self-test suite
    echo "[2/5] Running novad --selftest verification..."
    (cd userland/server/novad && cargo run --quiet -- --selftest)

    # 3. Test novad CLI lifecycle operations
    echo "[3/5] Testing novad CLI lifecycle operations..."
    NOVAD_BIN="userland/server/novad/target/debug/novad"

    # Boot default.target
    BOOT_OUT=$("$NOVAD_BIN" -u tools/fixtures/novad/system boot default.target)
    echo "$BOOT_OUT" | grep -q "Boot complete" || { echo "Error: novad boot failed"; exit 1; }

    # Check status of novasshd.service
    STATUS_OUT=$("$NOVAD_BIN" status novasshd.service)
    echo "$STATUS_OUT" | grep -q "active (running)" || { echo "Error: novasshd.service not active after boot"; exit 1; }

    # Restart novasshd.service
    RESTART_OUT=$("$NOVAD_BIN" restart novasshd.service)
    echo "$RESTART_OUT" | grep -q "Successfully restarted unit" || { echo "Error: novad restart failed"; exit 1; }

    # Stop novasshd.service
    STOP_OUT=$("$NOVAD_BIN" stop novasshd.service)
    echo "$STOP_OUT" | grep -q "Successfully stopped unit" || { echo "Error: novad stop failed"; exit 1; }

    STATUS_STOPPED=$("$NOVAD_BIN" status novasshd.service)
    echo "$STATUS_STOPPED" | grep -q "inactive (dead)" || { echo "Error: novasshd.service still active after stop"; exit 1; }

    # List units
    LIST_OUT=$("$NOVAD_BIN" list-units)
    echo "$LIST_OUT" | grep -q "novafw.service" || { echo "Error: novafw.service missing from list"; exit 1; }
    echo "$LIST_OUT" | grep -q "novactl.service" || { echo "Error: novactl.service missing from list"; exit 1; }
    echo "$LIST_OUT" | grep -q "novamon.service" || { echo "Error: novamon.service missing from list"; exit 1; }

    # Graceful shutdown
    SHUTDOWN_OUT=$("$NOVAD_BIN" shutdown)
    echo "$SHUTDOWN_OUT" | grep -q "Shutdown complete" || { echo "Error: novad shutdown failed"; exit 1; }

    # 4. Test novactl service CLI integration
    echo "[4/5] Testing novactl service CLI suite..."
    NOVACTL_BIN="userland/server/novactl/target/debug/novactl"

    # novactl service boot
    CTL_BOOT=$("$NOVACTL_BIN" service boot default.target)
    echo "$CTL_BOOT" | grep -q "Boot complete" || { echo "Error: novactl service boot failed"; exit 1; }

    # novactl service status
    CTL_STATUS=$("$NOVACTL_BIN" service status novasshd.service)
    echo "$CTL_STATUS" | grep -q "active (running)" || { echo "Error: novasshd not running in novactl service status"; exit 1; }

    # novactl service restart
    CTL_RESTART=$("$NOVACTL_BIN" service restart novasshd.service)
    echo "$CTL_RESTART" | grep -q "Successfully restarted service" || { echo "Error: novactl service restart failed"; exit 1; }

    # novactl service stop
    CTL_STOP=$("$NOVACTL_BIN" service stop novasshd.service)
    echo "$CTL_STOP" | grep -q "Successfully stopped service" || { echo "Error: novactl service stop failed"; exit 1; }

    # novactl service list
    CTL_LIST=$("$NOVACTL_BIN" service list)
    echo "$CTL_LIST" | grep -q "multi-user.target" || { echo "Error: multi-user.target missing from novactl service list"; exit 1; }

    # novactl service shutdown
    CTL_SHUTDOWN=$("$NOVACTL_BIN" service shutdown)
    echo "$CTL_SHUTDOWN" | grep -q "Shutdown complete" || { echo "Error: novactl service shutdown failed"; exit 1; }

    # 5. Run novactl full self-test including service integration
    echo "[5/5] Running novactl full test suite with novad integration..."
    (cd userland/server/novactl && cargo test --quiet)

    echo "[PASS] All Systemd-Style Service & Daemon Orchestrator (novad) tests passed successfully."
else
    echo "novad-test: cargo not found - skipping the suite"
fi
