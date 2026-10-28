#!/bin/sh
# NovaOS Interface Track - Full UI QA Pass
#
# Dedicated test pass executing every single user-facing application and UI subsystem
# developed across Months 1-6 of the NovaOS Interface Track:
#   1. Shared UI Design System (userland/ui-rs)
#   2. Window Manager & Compositor (userland/wm-rs)
#   3. File Manager (userland/files-rs)
#   4. Terminal Emulator (userland/term-rs)
#   5. Text Editor (userland/edit-rs)
#   6. Software Store (userland/store-rs)
#   7. Settings Shell & Admin Panels (userland/settings-rs)
#   8. Login & Lockscreen (userland/login-rs)
#   9. Structured Kernel Log Viewer (userland/klog-rs)
#  10. Activity Monitor & Process Control (userland/actmon-rs)
#  11. Graphical Installer Wizard (userland/installer-rs)
#  12. Graphical Crash Reporter (userland/crashrep-rs)
#  13. First-Run Setup Wizard (userland/firstrun-rs)
#
# Usage:
#   ./tools/tests/run_ui_qa_pass.sh    (or: make ui-qa)
set -e
cd "$(dirname "$0")/../.."

echo "================================================================================"
echo "           NovaOS Interface Track - Full Comprehensive UI QA Pass              "
echo "================================================================================"
echo "Target: 13 UI Subsystems & Applications (Months 1 - 6 Interface Track)"
echo "Date:   $(date)"
echo "--------------------------------------------------------------------------------"

TOTAL_SUITES=0
PASSED_SUITES=0

run_qa_step() {
    SUITE_NAME="$1"
    SCRIPT_PATH="$2"
    TOTAL_SUITES=$((TOTAL_SUITES + 1))
    echo ""
    echo ">>> Running QA Suite [$TOTAL_SUITES/13]: $SUITE_NAME"
    echo "--------------------------------------------------------------------------------"
    if [ -f "$SCRIPT_PATH" ]; then
        sh "$SCRIPT_PATH"
        PASSED_SUITES=$((PASSED_SUITES + 1))
        echo ">>> [OK] $SUITE_NAME passed QA verification."
    else
        echo ">>> [ERROR] Test script not found: $SCRIPT_PATH"
        exit 1
    fi
}

run_qa_step "1. UI Design System (userland/ui-rs)" "tools/tests/run_ui_tests.sh"
run_qa_step "2. Window Manager & Tiling Engine (userland/wm-rs)" "tools/tests/run_wm_tests.sh"
run_qa_step "3. File Browser (userland/files-rs)" "tools/tests/run_files_tests.sh"
run_qa_step "4. In-Window Terminal (userland/term-rs)" "tools/tests/run_term_tests.sh"
run_qa_step "5. Monospace Text Editor (userland/edit-rs)" "tools/tests/run_edit_tests.sh"
run_qa_step "6. Package Store (userland/store-rs)" "tools/tests/run_store_tests.sh"
run_qa_step "7. Settings & Panels (userland/settings-rs)" "tools/tests/run_settings_tests.sh"
run_qa_step "8. Login & Authentication (userland/login-rs)" "tools/tests/run_login_tests.sh"
run_qa_step "9. Kernel Log Viewer (userland/klog-rs)" "tools/tests/run_klog_tests.sh"
run_qa_step "10. Activity Monitor (userland/actmon-rs)" "tools/tests/run_actmon_tests.sh"
run_qa_step "11. Graphical Installer Wizard (userland/installer-rs)" "tools/tests/run_installer_tests.sh"
run_qa_step "12. Graphical Crash Reporter (userland/crashrep-rs)" "tools/tests/run_crashrep_tests.sh"
run_qa_step "13. First-Run Setup Wizard (userland/firstrun-rs)" "tools/tests/run_firstrun_tests.sh"

echo ""
echo "================================================================================"
echo "                         Full UI QA Pass Complete!                              "
echo "================================================================================"
echo "Results: $PASSED_SUITES / $TOTAL_SUITES UI test suites passed successfully."
echo "Status:  100% PASS - All 13 ring-3 applications & UI subsystems verified!"
echo "================================================================================"
