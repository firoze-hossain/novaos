#!/bin/sh
# NovaOS Calculator Application & Math Studio (nova-calc) Test Suite
#
# Synthesizes Apple Calculator (macOS), Windows 11 Fluent Calculator, and Ubuntu GNOME Calculator:
# 1. Multi-mode computation: Standard, Scientific, Programmer (Hex/Dec/Oct/Bin), and Calculation History Tape.
# 2. Mathematical precision: recursive-descent expression parser, trigonometric, logarithmic, power, factorial functions.
# 3. Programmer integer mode: 64/32/16/8-bit word sizing, live 4-base conversion, bitwise AND/OR/XOR/NOT/shifts.
# 4. Memory management: MC, MR, M+, M-, MS with interactive screen indicator badge.
# 5. Host-side UI rendering with BufferSurface: acrylic title bar, digital high-contrast LCD screen, warm amber operator buttons.
# 6. Full CLI execution: --selftest, direct arithmetic evaluation, and help manual.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nova_calc_tests.sh
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- userland/desktop/apps/nova-calc host tests (Unified Calculator Studio) ---"
    (cd userland/desktop/apps/nova-calc && cargo test --quiet)
    (cd userland/desktop/apps/nova-calc && cargo run --quiet -- --selftest)
    (cd userland/desktop/apps/nova-calc && cargo run --quiet -- "125 * (4 + 8)" > /dev/null)
    (cd userland/desktop/apps/nova-calc && cargo run --quiet -- "sqrt(144) + 2 ^ 8" > /dev/null)
    (cd userland/desktop/apps/nova-calc && cargo run --quiet -- "sin(pi / 2) + cos(0)" > /dev/null)
    echo "[PASS] All NovaOS Calculator (nova-calc) tests passed successfully."
else
    echo "nova-calc-test: cargo not found - skipping the suite"
fi
