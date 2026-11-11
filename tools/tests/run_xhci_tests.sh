#!/bin/sh
# Month 1 - Foundation: host-side tests for Pure Rust xHCI USB 3.2 Host Controller & HID Stack
# 1. PCI class enumeration matching 0x0C0330 (Serial Bus 0x0C, USB 0x03, xHCI 0x30).
# 2. 64-bit BAR0 MMIO register space mapping (Capability, Operational, Runtime, Doorbells).
# 3. Doorbell offset calculations across varying doorbell strides and target endpoints.
# 4. 16-byte packed TRB layout, type encoding, cycle bit, and command completion codes.
# 5. Circular TRB Ring management with Link TRB cycle-bit toggling and wrap.
# 6. Event Ring Segment Table (ERST) and consumer cycle state tracking.
# 7. Device Context Base Address Array (DCBAA), Slot Context, and Endpoint Context.
# 8. Root hub port speed decoding (FullSpeed, LowSpeed, HighSpeed, SuperSpeed, SuperSpeedPlus).
# 9. USB HID Keyboard 6KRO report decoder, modifier key bitmasks, and ASCII/ScanCode translation.
# 10. USB HID Optical Mouse 3/4-byte report decoder, relative deltas (dx, dy), and wheel deltas.
# 11. USB HID Multi-Touch Digitizer coordinate scaling (0..65535), contact tracking, and pressure.
# 12. USB HID Gamepad digital buttons and analog axes.
# 13. Circular Input Event Queue FIFO ordering and kernel injection hooks.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_xhci_tests.sh      (or: make xhci-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/drivers/usb/xhci.rs host tests (xHCI Host Controller) ---"
    rustc --edition 2021 --test -o "$OUT/xhci_test" kernel/src/drivers/usb/xhci.rs
    "$OUT/xhci_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/xhci_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/usb/xhci.rs tests succeeded."

    echo "--- kernel/src/drivers/usb/hid.rs host tests (USB HID Stack) ---"
    rustc --edition 2021 --test -o "$OUT/hid_test" kernel/src/drivers/usb/hid.rs
    "$OUT/hid_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/hid_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/usb/hid.rs tests succeeded."

    echo "[PASS] All xHCI USB 3.2 Host Controller & HID Stack host tests succeeded."
else
    echo "xhci-test: rustc not found - skipping the Rust suite"
fi
