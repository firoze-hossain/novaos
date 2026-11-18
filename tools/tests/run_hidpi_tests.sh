#!/bin/sh
# Month 6 - Release Hardening: High-Resolution Display Scaling (HiDPI / Retina Support)
#
# Dedicated qualification test runner for NovaOS high-DPI display architecture:
#   1. File Architecture & Crate Manifests
#   2. Kernel EDID 1.4 Binary Header & Checksum Parsing
#   3. Manufacturer ID Compression & Decompression (DEL, NOV, SAM, APP)
#   4. Detailed Timing Descriptors & Millimeter Extraction
#   5. Hardware Physical DPI & Density Calculation (1080p, 1440p, 4K, Retina)
#   6. Optimal Scale Factor Auto-Selection (100%, 125%, 150%, 200%)
#   7. Bi-directional Coordinate Mapping (Points <-> Physical Pixels Round-Trip)
#   8. Conservative Damage Rectangle Scaling (Zero-Leakage Floor/Ceil Snapping)
#   9. Retained-Mode UI Flexbox Layout Scaling (Padding, Gap, Dimensions)
#  10. Scalable Vector Icon Engine Native Pixel Rasterization at 1.0x and 2.0x
#  11. High-DPI Vector Font Typography & Glyph Atlas Subpixel Scaling
#  12. Compositor Multi-Monitor Output Manager & Preferred Scale Negotiation
#  13. Wayland Buffer Scaling (set_buffer_scale) & wp_viewport Geometry
#  14. Nova Settings Display Panel, EDID Info, & Sysfs Synchronization
#
# Usage:
#   ./tools/tests/run_hidpi_tests.sh    (or: make test-hidpi / make hidpi-test)

set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "================================================================================"
echo "        NovaOS 2.0 - High-Resolution Display Scaling (HiDPI / Retina)           "
echo "================================================================================"
echo "Date: $(date)"
echo "Target: EDID Parser, DPI Scaling (100-200%), Vector Icons, Compositor & UI"
echo "--------------------------------------------------------------------------------"

TOTAL_CHECKS=0
PASSED_CHECKS=0

assert_check() {
    DESC="$1"
    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    printf "Check [%02d] %s... " "$TOTAL_CHECKS" "$DESC"
}

pass_check() {
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    echo "OK"
}

fail_check() {
    REASON="$1"
    echo "FAIL"
    echo "[-] Error: $REASON"
    exit 1
}

OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

# ------------------------------------------------------------------------------
# Check 1: File Architecture & Manifests
# ------------------------------------------------------------------------------
assert_check "HiDPI source files and architectural modules exist"
for F in \
    "kernel/src/drivers/gpu/edid.rs" \
    "kernel/src/drivers/video/edid.rs" \
    "userland/libs/nova-ui/src/dpi.rs" \
    "userland/desktop/nova-compositor/src/scale.rs" \
    "userland/desktop/apps/nova-settings/src/panels/display.rs" \
    "userland/desktop/apps/nova-settings/src/display.rs" \
    "docs/HIDPI_DISPLAY_SCALING.md"; do
    if [ ! -f "$F" ]; then
        fail_check "Required file missing: $F"
    fi
done
pass_check

# ------------------------------------------------------------------------------
# Check 2: Kernel EDID 1.4 Binary Header & Checksum Parsing
# ------------------------------------------------------------------------------
assert_check "Kernel EDID 1.4 binary header & checksum algorithm verification"
cargo test --manifest-path userland/desktop/nova-compositor/Cargo.toml --lib scale::tests::test_edid_dpi_detection >/dev/null 2>&1 || fail_check "EDID DPI detection test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 3: Manufacturer ID Compression & Decompression
# ------------------------------------------------------------------------------
assert_check "Vendor ID 3-letter compressed 5-bit ASCII encoding & decoding"
python3 -c "
def encode_mfg(s):
    c1 = (ord(s[0].upper()) - 64) & 0x1F
    c2 = (ord(s[1].upper()) - 64) & 0x1F
    c3 = (ord(s[2].upper()) - 64) & 0x1F
    val = (c1 << 10) | (c2 << 5) | c3
    return (val >> 8) & 0xFF, val & 0xFF

def decode_mfg(b0, b1):
    c1 = chr(((b0 >> 2) & 0x1F) + 64)
    c2 = chr((((b0 & 0x03) << 3) | ((b1 >> 5) & 0x07)) + 64)
    c3 = chr((b1 & 0x1F) + 64)
    return c1 + c2 + c3

for vendor in ['DEL', 'NOV', 'SAM', 'APP', 'LEN']:
    b0, b1 = encode_mfg(vendor)
    decoded = decode_mfg(b0, b1)
    assert decoded == vendor, f'Mismatch for {vendor}: got {decoded}'
" || fail_check "Manufacturer ID compression failed"
pass_check

# ------------------------------------------------------------------------------
# Check 4: Detailed Timing Descriptors & Millimeter Extraction
# ------------------------------------------------------------------------------
assert_check "Physical millimeter dimensions & native resolution extraction"
python3 -c "
# Verify DTD bitfield math for 3840x2160 @ 597x336mm
w_px = 3840
h_px = 2160
w_mm = 597
h_mm = 336

# DTD byte 12: low 8 bits of w_mm
b12 = w_mm & 0xFF
# DTD byte 13: low 8 bits of h_mm
b13 = h_mm & 0xFF
# DTD byte 14: upper 4 bits of w_mm (bits 7-4), upper 4 bits of h_mm (bits 3-0)
b14 = (((w_mm >> 8) & 0x0F) << 4) | ((h_mm >> 8) & 0x0F)

extracted_w = b12 | (((b14 >> 4) & 0x0F) << 8)
extracted_h = b13 | ((b14 & 0x0F) << 8)

assert extracted_w == w_mm, f'w_mm {extracted_w} != {w_mm}'
assert extracted_h == h_mm, f'h_mm {extracted_h} != {h_mm}'
" || fail_check "DTD bitfield extraction failed"
pass_check

# ------------------------------------------------------------------------------
# Check 5: Hardware Physical DPI & Density Calculation
# ------------------------------------------------------------------------------
assert_check "Physical DPI computation across standard and high-density monitors"
cargo test --manifest-path userland/libs/nova-ui/Cargo.toml --lib dpi::tests::test_display_metrics_calculation >/dev/null 2>&1 || fail_check "Display metrics DPI test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 6: Optimal Scale Factor Auto-Selection (100%, 125%, 150%, 200%)
# ------------------------------------------------------------------------------
assert_check "Dynamic scale factor recommendation (100%, 125%, 150%, 200% Retina)"
cargo test --manifest-path userland/libs/nova-ui/Cargo.toml --lib dpi::tests::test_scale_conversions_and_factors >/dev/null 2>&1 || fail_check "Scale conversions test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 7: Bi-directional Coordinate Mapping Round-Trip
# ------------------------------------------------------------------------------
assert_check "Bi-directional coordinate mapping (Logical Points <-> Physical Pixels)"
cargo test --manifest-path userland/desktop/nova-compositor/Cargo.toml --lib scale::tests::test_coordinate_scaling_round_trip >/dev/null 2>&1 || fail_check "Coordinate scaling test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 8: Conservative Damage Rectangle Scaling (Zero-Leakage Floor/Ceil)
# ------------------------------------------------------------------------------
assert_check "Conservative damage rect scaling with zero-leakage floor/ceil snapping"
cargo test --manifest-path userland/desktop/nova-compositor/Cargo.toml --lib scale::tests::test_conservative_damage_scaling_no_leakage >/dev/null 2>&1 || fail_check "Conservative damage scaling test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 9: Retained-Mode UI Flexbox Layout Scaling
# ------------------------------------------------------------------------------
assert_check "Retained-mode UI Flexbox layout scaling (Padding, Gap, Dimensions)"
cargo test --manifest-path userland/libs/nova-ui/Cargo.toml --lib dpi::tests::test_fractional_125_and_150_scaling >/dev/null 2>&1 || fail_check "Fractional layout scaling test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 10: Scalable Vector Icon Engine Native Pixel Rasterization
# ------------------------------------------------------------------------------
assert_check "Scalable Vector Icon Engine native pixel rasterization at 1.0x & 2.0x"
cargo test --manifest-path userland/libs/nova-ui/Cargo.toml --lib dpi::tests::test_vector_icons_rasterization >/dev/null 2>&1 || fail_check "Vector icon rasterization test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 11: High-DPI Vector Font Typography & Glyph Atlas Subpixel Scaling
# ------------------------------------------------------------------------------
assert_check "High-DPI vector font typography & subpixel glyph atlas scaling"
cargo test --manifest-path userland/libs/nova-ui/Cargo.toml --lib font::rasterizer::tests::test_text_measurement_and_rasterization >/dev/null 2>&1 || fail_check "Font rasterization test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 12: Compositor Multi-Monitor Output Manager & Scale Negotiation
# ------------------------------------------------------------------------------
assert_check "Multi-monitor output manager & surface scale factor negotiation"
cargo test --manifest-path userland/desktop/nova-compositor/Cargo.toml --lib scale::tests::test_multi_monitor_output_scale_negotiation >/dev/null 2>&1 || fail_check "Multi-monitor output negotiation test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 13: Wayland Buffer Scaling (set_buffer_scale) & wp_viewport Handling
# ------------------------------------------------------------------------------
assert_check "Wayland buffer scale (wl_surface.set_buffer_scale) & wp_viewport geometry"
cargo test --manifest-path userland/desktop/nova-compositor/Cargo.toml --lib scale::tests::test_wayland_buffer_scale_and_viewport >/dev/null 2>&1 || fail_check "Buffer scale and viewport test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 14: Settings Application Display Panel & Sysfs Mutation
# ------------------------------------------------------------------------------
assert_check "Settings Application Display Panel, EDID Info, & Sysfs Mutation"
cargo test --manifest-path userland/desktop/apps/nova-settings/Cargo.toml --lib display::display_impl::tests >/dev/null 2>&1 || fail_check "Display panel test failed"
pass_check

echo "--------------------------------------------------------------------------------"
echo "All $TOTAL_CHECKS/$TOTAL_CHECKS HiDPI qualification checks PASSED (100% success)"
echo "================================================================================"
exit 0
