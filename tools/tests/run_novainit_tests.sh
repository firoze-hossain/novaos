#!/bin/sh
# Interface Track / Month 1 - Prep: host-side tests for the NovaOS Service
# Supervisor (userland/novainit-rs) and SERVICES.CFG declarative config:
# verifying restart policies, crash recovery rules, dependency ordering,
# and the supervised, trusted, always-restarted shell service wiring.
#
# No QEMU needed; executes in under a second.
#
#   ./tools/tests/run_novainit_tests.sh      (or: make novainit-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

SERVICES_CFG="tools/fixtures/SERVICES.CFG"
if [ ! -f "$SERVICES_CFG" ]; then
    echo "FAIL: $SERVICES_CFG not found" >&2
    exit 1
fi

echo "--- Checking tools/fixtures/SERVICES.CFG declarative structure ---"
# Check header
HEADER=$(head -n 1 "$SERVICES_CFG")
if [ "$HEADER" != "name|exec_path|arg|depends_on|restart|trusted" ]; then
    echo "FAIL: invalid SERVICES.CFG header: '$HEADER'" >&2
    exit 1
fi

# Verify shell service line exists exactly
if ! grep -q "^shell|SHELL\.ELF|-|.*|always|yes$" "$SERVICES_CFG"; then
    echo "FAIL: shell entry missing or misconfigured in $SERVICES_CFG" >&2
    exit 1
fi
echo "[PASS] tools/fixtures/SERVICES.CFG defines supervised shell service (restart=always, trusted=yes)"

if command -v rustc >/dev/null 2>&1; then
    echo "--- userland/novainit-rs host tests (Service Supervisor Logic) ---"
    rustc --edition 2021 --test -o "$OUT/novainit_test" userland/novainit-rs/novainit.rs
    "$OUT/novainit_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/novainit_test" >/dev/null 2>&1
    echo "[PASS] All userland/novainit-rs host tests succeeded."
else
    echo "novainit-test: rustc not found - skipping the Rust suite"
fi
