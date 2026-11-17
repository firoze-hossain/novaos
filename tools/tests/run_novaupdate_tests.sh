#!/bin/sh
# Month 5 - Enterprise Storage: Automated Unattended Security Updates with Atomic A/B Rollback
#
# Validates pure Rust automated update engine (novaupdate) and A/B dual-slot recovery:
# 1. Dual Boot Slot Metadata & A/B Partitioning (Slot A / Slot B):
#    - Preserves BOOTSLOT.DAT metadata with SHA-256 digest validation and atomic file writes.
# 2. Cryptographically Signed OS Delta Packages:
#    - HMAC-SHA256 signature verification rejects untrusted/tampered update payloads.
# 3. Hardware Watchdog Supervision:
#    - Arms 60-second hardware watchdog timer (/dev/watchdog) during probationary boots.
# 4. Atomic Staging & Inactive Slot Upgrades:
#    - Writes update payload to inactive slot without modifying active running system.
# 5. Probationary Test Boot (Testing Mode):
#    - Decrements remaining boot attempts; automatically reverts upon boot failure.
# 6. Post-Boot Integrity Diagnostics & Atomic Commit:
#    - Self-tests pass -> permanently commits slot to NORMAL state and disarms watchdog.
# 7. Automated Watchdog Rollback on Crash/Panic:
#    - Simulates boot failure and watchdog timeout -> triggers automatic fallback to known-good slot.
# 8. Full CLI Management & Daemon Orchestration:
#    - novaupdate status, check, stage, arm-test, boot-next, confirm, rollback, build-pkg, daemon.
#
# Usage:
#   ./tools/tests/run_novaupdate_tests.sh      (or: make novaupdate-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- Month 5: Automated Unattended Security Updates with Atomic A/B Rollback (novaupdate) ---"
    
    # 1. Run Cargo unit test suite
    echo "[1/4] Running Cargo unit test suite..."
    (cd userland/server/novaupdate && cargo test --quiet)

    # 2. Run automated self-test suite
    echo "[2/4] Running novaupdate --selftest verification..."
    (cd userland/server/novaupdate && cargo run --quiet -- --selftest)

    # 3. Run full CLI A/B update lifecycle in isolated sandbox
    echo "[3/4] Running CLI atomic A/B update & rollback lifecycle workflow..."
    SANDBOX_DIR=$(mktemp -d)
    trap 'rm -rf "$SANDBOX_DIR"' EXIT
    export NOVAUPDATE_DATA_DIR="$SANDBOX_DIR/boot"
    export NOVAUPDATE_WATCHDOG_DEV="$SANDBOX_DIR/watchdog.state"
    mkdir -p "$NOVAUPDATE_DATA_DIR"

    BIN="userland/server/novaupdate/target/debug/novaupdate"
    if [ ! -f "$BIN" ]; then
        (cd userland/server/novaupdate && cargo build --quiet)
    fi

    # Query initial boot status
    echo "  Inspecting initial A/B boot slot status..."
    $BIN status | grep -q "Active Boot Slot:    Slot A"

    # Create mock new kernel payload
    echo "  Generating new kernel delta payload binary..."
    printf "NOVAOS_KERNEL_DELTA_V2.1.0_ENTERPRISE_PATCH" > "$SANDBOX_DIR/new_kernel.bin"

    # Build cryptographically signed delta package
    echo "  Building cryptographically signed OS delta package..."
    $BIN build-pkg --name novaos-kernel --ver 2.1.0 --min 2.0.0 --payload "$SANDBOX_DIR/new_kernel.bin" --out "$SANDBOX_DIR/patch_2.1.0.pkg"
    [ -f "$SANDBOX_DIR/patch_2.1.0.pkg" ]

    # Stage update into inactive slot B
    echo "  Staging signed package into inactive slot B..."
    $BIN stage "$SANDBOX_DIR/patch_2.1.0.pkg" | grep -q "staged version '2.1.0' into Slot B"

    # Arm slot B in probationary testing mode with watchdog
    echo "  Arming slot B for probationary test boot..."
    $BIN arm-test B | grep -q "Armed Slot B (Secondary) in TESTING mode"
    $BIN status | grep -q "Slot State:          TESTING"

    # Confirm healthy post-boot self-test and atomic commit
    echo "  Executing post-boot health confirmation & permanent commit..."
    $BIN confirm | grep -q "Successfully confirmed Slot B (Secondary) healthy!"
    $BIN status | grep -q "Slot State:          NORMAL"
    $BIN status | grep -q "Active Boot Slot:    Slot B"

    # Trigger manual administrator rollback
    echo "  Executing rollback to fallback slot..."
    $BIN rollback --reason "Testing operator rollback" | grep -q "Restored active slot to: Slot A"
    $BIN status | grep -q "Active Boot Slot:    Slot A"

    # Test boot-next switch
    echo "  Testing explicit boot-next slot selection..."
    $BIN boot-next B | grep -q "Next boot slot set to: Slot B"

    # Test daemon initialization
    echo "  Testing update daemon initialization..."
    $BIN daemon | grep -q "Starting NovaOS Automated Unattended Update Daemon"

    # 4. Verify systemd unit fixture exists and is valid
    echo "[4/4] Validating novad systemd-style unit configuration..."
    grep -q "novaupdate daemon" tools/fixtures/novad/system/novaupdate.service
    grep -q "novaupdate.service" tools/fixtures/novad/system/multi-user.target

    echo "--- All Automated Unattended Update & Atomic A/B Rollback tests PASSED! ---"
else
    echo "cargo not found; skipping novaupdate tests"
fi
