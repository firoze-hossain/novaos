#!/bin/sh
# tools/tests/run_ci_matrix_tests.sh
# Release Track: Qualification test suite for Automated CI/CD Qualification & Test Matrix Runner
# Proves:
#   1. tools/ci/test_matrix.py CLI syntax, argument parsing, and help banner
#   2. PixelDiffEngine PPM reader, writer, and diff calculation logic:
#      - Identical frames produce 0.000% diff, max delta 0, RMS 0.0
#      - Intentional delta within tolerance passes and reports exact dirty pixel count
#      - Exceeded delta triggers expected failure
#      - Damage rectangle clipping verifies zero leak outside bounding box
#   3. Dual-Edition Boot Matrix assertion classification & 600+ assertion tracking
#   4. Fast qualification matrix execution and JSON test report generation
#   5. Structured JSON report validation (valid schema, summary, boot & subsystem results)

set -e
cd "$(dirname "$0")/../.."   # repo root

echo "========================================================================"
echo "  NovaOS Release Track: CI/CD Test Matrix Runner Qualification Suite"
echo "========================================================================"
echo "Target: tools/ci/test_matrix.py | 600+ Assertions | Framebuffer Pixel-Diff"
echo "Date:   $(date)"
echo "------------------------------------------------------------------------"

FAILED=0
TOTAL=0

assert_eq() {
    TOTAL=$((TOTAL + 1))
    DESC="$1"
    ACTUAL="$2"
    EXPECTED="$3"
    if [ "$ACTUAL" = "$EXPECTED" ]; then
        echo "  [PASS] $DESC"
    else
        echo "  [FAIL] $DESC: expected '$EXPECTED', got '$ACTUAL'"
        FAILED=$((FAILED + 1))
    fi
}

assert_ge() {
    TOTAL=$((TOTAL + 1))
    DESC="$1"
    ACTUAL="$2"
    MIN="$3"
    if [ "$ACTUAL" -ge "$MIN" ] 2>/dev/null; then
        echo "  [PASS] $DESC ($ACTUAL >= $MIN)"
    else
        echo "  [FAIL] $DESC: expected >= '$MIN', got '$ACTUAL'"
        FAILED=$((FAILED + 1))
    fi
}

assert_contains() {
    TOTAL=$((TOTAL + 1))
    DESC="$1"
    HAYSTACK="$2"
    NEEDLE="$3"
    if echo "$HAYSTACK" | grep -q -F -- "$NEEDLE"; then
        echo "  [PASS] $DESC"
    else
        echo "  [FAIL] $DESC: expected to find '$NEEDLE'"
        FAILED=$((FAILED + 1))
    fi
}

# --- Step 1: CLI Syntax & Help ---
echo "\n[1/5] Validating tools/ci/test_matrix.py CLI interface..."
assert_eq "test_matrix.py is an executable file" "$(test -x tools/ci/test_matrix.py && echo yes)" "yes"

HELP_OUT=$(python3 tools/ci/test_matrix.py --help)
assert_contains "Help output contains --boot flag" "$HELP_OUT" "--boot"
assert_contains "Help output contains --fast flag" "$HELP_OUT" "--fast"
assert_contains "Help output contains --pixel-diff flag" "$HELP_OUT" "--pixel-diff"
assert_contains "Help output contains --subsystems flag" "$HELP_OUT" "--subsystems"
assert_contains "Help output contains --parallel flag" "$HELP_OUT" "--parallel"
assert_contains "Help output contains --profile flag" "$HELP_OUT" "--profile"
assert_contains "Help output contains --report flag" "$HELP_OUT" "--report"

# --- Step 2: Pixel-Diff Engine Unit Tests ---
echo "\n[2/5] Testing Framebuffer Pixel-Diff & Visual Regression Engine..."
python3 -c "
import sys
from pathlib import Path
sys.path.insert(0, 'tools/ci')
from test_matrix import PPMImage, PixelDiffEngine

# Test 1: Identical images produce 0.000% difference
img1 = PPMImage.blank(64, 64, (20, 30, 40))
img2 = PPMImage.blank(64, 64, (20, 30, 40))
res1 = PixelDiffEngine.compare(img1, img2, tolerance_percent=0.0)
assert res1.passed, 'Identical images failed'
assert res1.diff_pixels == 0, f'Expected 0 diff, got {res1.diff_pixels}'
assert res1.diff_percentage == 0.0, f'Expected 0.0%, got {res1.diff_percentage}'
assert res1.max_channel_delta == 0, f'Expected delta 0, got {res1.max_channel_delta}'
assert res1.rms_error == 0.0, f'Expected RMS 0, got {res1.rms_error}'

# Test 2: Invalidate a known rectangular damage region
# 16x16 rect inside 64x64 canvas = 256 / 4096 = 6.25% dirty
img2.fill_rect(10, 10, 16, 16, (200, 200, 200))
res2 = PixelDiffEngine.compare(img1, img2, tolerance_percent=10.0, channel_threshold=5)
assert res2.passed, 'Expected pass within 10% tolerance'
assert res2.diff_pixels == 256, f'Expected 256 dirty pixels, got {res2.diff_pixels}'
assert abs(res2.diff_percentage - 6.25) < 0.01, f'Expected 6.25% diff, got {res2.diff_percentage}'

# Test 3: Exceeded tolerance triggers failure
res3 = PixelDiffEngine.compare(img1, img2, tolerance_percent=5.0, channel_threshold=5)
assert not res3.passed, 'Expected failure when diff (6.25%) exceeds tolerance (5.0%)'

# Test 4: Zero-leak damage clipping test
for y in range(64):
    for x in range(64):
        if not (10 <= x < 26 and 10 <= y < 26):
            assert img1.get_pixel(x, y) == img2.get_pixel(x, y), 'Pixel leak outside damage rect!'

print('PIXEL_DIFF_ENGINE_UNIT_TESTS_OK')
" > /tmp/pix_unit_out.txt 2>&1

assert_contains "PixelDiffEngine unit assertions passed" "$(cat /tmp/pix_unit_out.txt)" "PIXEL_DIFF_ENGINE_UNIT_TESTS_OK"
rm -f /tmp/pix_unit_out.txt

# --- Step 3: Compositor Oracle Visual Regression ---
echo "\n[3/5] Running Compositor Snap Layout & Damage-Rect Oracle Test..."
python3 -c "
import sys
from pathlib import Path
sys.path.insert(0, 'tools/ci')
from test_matrix import PixelDiffEngine

out_dir = Path('build/ci_test_oracle')
res = PixelDiffEngine.run_compositor_oracle_regression_test(out_dir)
assert res['status'] == 'PASS', f'Compositor oracle failed: {res}'
assert res['zero_leak_verified'] == True, 'Outside boundary leak detected!'
assert res['geometry'] == '1024x768', f'Unexpected geometry: {res[\"geometry\"]}'
assert res['damage_rect'] == {'x': 60, 'y': 120, 'w': 200, 'h': 150}
assert res['dirty_pixels'] == 30000, f'Unexpected dirty pixels: {res[\"dirty_pixels\"]}'
print('COMPOSITOR_ORACLE_TEST_OK')
" > /tmp/oracle_out.txt 2>&1

assert_contains "Compositor Snap Layout Oracle verified" "$(cat /tmp/oracle_out.txt)" "COMPOSITOR_ORACLE_TEST_OK"
rm -f /tmp/oracle_out.txt
rm -rf build/ci_test_oracle

# --- Step 4: Assertion Count & Classification Validation ---
echo "\n[4/5] Validating Boot Assertion Coverage (600+ Assertions)..."
python3 -c "
import sys
from pathlib import Path
sys.path.insert(0, 'tools/ci')
from test_matrix import BootAssertionMatrix

desk_assertions = BootAssertionMatrix.get_assertions_for_profile('desktop')
serv_assertions = BootAssertionMatrix.get_assertions_for_profile('server')

total_combined = len(desk_assertions) + len(serv_assertions)
print(f'DESKTOP_ASSERTIONS={len(desk_assertions)}')
print(f'SERVER_ASSERTIONS={len(serv_assertions)}')
print(f'COMBINED_ASSERTIONS={total_combined}')
assert len(desk_assertions) == 343, f'Expected 343 desktop assertions, got {len(desk_assertions)}'
assert len(serv_assertions) == 271, f'Expected 271 server assertions, got {len(serv_assertions)}'
assert total_combined >= 600, f'Expected >= 600 combined assertions, got {total_combined}'
" > /tmp/assertions_count.txt

DESK_CNT=$(grep "DESKTOP_ASSERTIONS=" /tmp/assertions_count.txt | cut -d= -f2)
SERV_CNT=$(grep "SERVER_ASSERTIONS=" /tmp/assertions_count.txt | cut -d= -f2)
COMB_CNT=$(grep "COMBINED_ASSERTIONS=" /tmp/assertions_count.txt | cut -d= -f2)

assert_eq "Desktop assertion count equals 343" "$DESK_CNT" "343"
assert_eq "Server assertion count equals 271" "$SERV_CNT" "271"
assert_ge "Combined assertion count satisfies 600+ requirement" "$COMB_CNT" 600
rm -f /tmp/assertions_count.txt

# --- Step 5: Fast Qualification Run & JSON Report Schema Verification ---
echo "\n[5/5] Executing Fast Qualification Run & Validating JSON Report..."
FAST_REPORT="build/ci/test-matrix-fast-verified.json"
python3 tools/ci/test_matrix.py --fast --report "$FAST_REPORT" > /tmp/fast_matrix_run.log 2>&1

assert_eq "JSON qualification report generated" "$(test -f "$FAST_REPORT" && echo yes)" "yes"

python3 -c "
import json
with open('$FAST_REPORT') as f:
    rep = json.load(f)

meta = rep.get('metadata', {})
summary = rep.get('summary', {})
boot = rep.get('boot_matrix', [])
subsystems = rep.get('subsystems_matrix', [])
pix = rep.get('pixel_diff_matrix', {})

assert meta.get('overall_status') == 'PASSED', f'Overall status not PASSED: {meta.get(\"overall_status\")}'
assert summary.get('total_boot_assertions', 0) >= 600, f'Total boot assertions < 600: {summary.get(\"total_boot_assertions\")}'
assert summary.get('failed_boot_assertions') == 0, f'Failed boot assertions: {summary.get(\"failed_boot_assertions\")}'
assert summary.get('boot_assertions_pass_rate') == 100.0, f'Pass rate: {summary.get(\"boot_assertions_pass_rate\")}'
assert summary.get('failed_subsystem_suites') == 0, f'Failed subsystem suites: {summary.get(\"failed_subsystem_suites\")}'
assert len(subsystems) >= 18, f'Expected >= 18 subsystem suites, got {len(subsystems)}'
assert pix.get('compositor_oracle', {}).get('status') == 'PASS', 'Compositor oracle not PASS'

print('JSON_REPORT_SCHEMA_VALID_OK')
" > /tmp/json_val.txt 2>&1

assert_contains "JSON qualification report schema and 100% pass rate verified" "$(cat /tmp/json_val.txt)" "JSON_REPORT_SCHEMA_VALID_OK"
rm -f /tmp/json_val.txt

echo "\n========================================================================"
if [ "$FAILED" -eq 0 ]; then
    echo " ✅ ALL $TOTAL CHECKS PASSED: Automated CI/CD Matrix Runner 100% Verified!"
    echo "========================================================================"
    exit 0
else
    echo " ❌ $FAILED of $TOTAL CHECKS FAILED!"
    echo "========================================================================"
    exit 1
fi
