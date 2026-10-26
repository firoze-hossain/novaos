#!/bin/sh
# Interface Track / Month 4 - Server & admin UI: host-side tests for the NovaOS
# Graphical Settings App (userland/settings-rs): verifying plugin panel registration,
# SYSTEM.CFG/USERS.CFG/SERVICES.CFG serialization, live kernel firewall telemetry
# and rule management (SYS_FW_INFO, SYS_FW_CTL), network interface & IPv4 addressing
# model, FIREWALL.CFG roundtrip, and in-memory 32-bit BufferSurface32 rendering.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_settings_tests.sh      (or: make settings-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/settings-rs host tests (NovaOS Graphical Settings App) ---"
    rustc --edition 2021 --crate-type=rlib -o "$OUT/libui.rlib" userland/ui-rs/src/lib.rs
    rustc --edition 2021 --test -L "$OUT" --extern ui="$OUT/libui.rlib" -o "$OUT/settings_test" userland/settings-rs/settings.rs
    "$OUT/settings_test" --test-threads=1 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/settings_test" --test-threads=1 >/dev/null
    echo "[PASS] All userland/settings-rs host tests succeeded."
else
    echo "settings-test: rustc not found - skipping the Rust suite"
fi
