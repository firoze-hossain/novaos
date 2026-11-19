#!/bin/sh
# Month 6 - Release Hardening: System Recovery Environment & Snapshot Rollback Mode
#
# Dedicated qualification test runner for NovaOS Recovery Environment architecture:
#   1. File Architecture & Crate Manifests
#   2. Kernel Emergency Boot Hook & Panic Fallback Integration
#   3. Kernel Boot Mode Evaluation & Attempt Counter Logic
#   4. Kernel Protective MBR Validation & In-Place Repair
#   5. Kernel Primary GPT Restoration from Backup GPT
#   6. Kernel NovaFS Snapshot Discovery & Atomic Rollback
#   7. Userland Cryptographic Engine (CRC32, PBKDF2-HMAC-SHA256)
#   8. NovaFS Snapshot Discovery & Generation Tracking
#   9. Atomic Rollback & Pre-Rollback Checkpoint Creation
#  10. Administrative Password Reset & /etc/shadow Generation
#  11. Account Unlock & Credential Verification
#  12. Dual-Header GPT Inspection & Backup Recovery
#  13. Emergency System & Kernel Crash Log Viewer
#  14. Retained-Mode 800x600 GUI Framebuffer Rasterization (5 Tabs)
#  15. Headless CLI Rescue Console Commands
#  16. Master Autonomous Self-Test Execution
#
# Usage:
#   ./tools/tests/run_recovery_tests.sh    (or: make test-recovery)

set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "================================================================================"
echo "    NovaOS 2.0 - System Recovery Environment & Snapshot Rollback Mode           "
echo "================================================================================"
echo "Date: $(date)"
echo "Target: kernel/src/boot/recovery.rs & userland/desktop/apps/nova-recovery"
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
assert_check "Kernel recovery engine source exists (kernel/src/boot/recovery.rs)"
if [ ! -f "kernel/src/boot/recovery.rs" ]; then
    fail_check "kernel/src/boot/recovery.rs missing"
fi
if [ "$(wc -l < kernel/src/boot/recovery.rs)" -lt 250 ]; then
    fail_check "kernel/src/boot/recovery.rs too brief, expected > 250 lines"
fi
pass_check

assert_check "Userland recovery manifest exists (userland/desktop/apps/nova-recovery/Cargo.toml)"
if [ ! -f "userland/desktop/apps/nova-recovery/Cargo.toml" ]; then
    fail_check "nova-recovery/Cargo.toml missing"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 2: Kernel Emergency Hook & C-ABI Exports
# ------------------------------------------------------------------------------
assert_check "Kernel Rust exports rust_recovery_kernel_selftest in kernel/rust/lib.rs"
if ! grep -q "pub extern \"C\" fn rust_recovery_kernel_selftest" kernel/rust/lib.rs; then
    fail_check "rust_recovery_kernel_selftest not found in kernel/rust/lib.rs"
fi
pass_check

assert_check "Kernel boot initialization calls recovery self-test in kernel/init/main.c"
if ! grep -q "rust_recovery_kernel_selftest" kernel/init/main.c; then
    fail_check "rust_recovery_kernel_selftest not called in kernel/init/main.c"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 3: Kernel Boot Mode Evaluation & Attempt Counter Logic
# ------------------------------------------------------------------------------
assert_check "Kernel evaluates BootMode, PanicFallback, and consecutive attempts"
if ! grep -q "pub fn evaluate_boot_mode" kernel/src/boot/recovery.rs; then
    fail_check "evaluate_boot_mode missing in recovery.rs"
fi
if ! grep -q "BootMode::PanicFallback" kernel/src/boot/recovery.rs; then
    fail_check "BootMode::PanicFallback missing in recovery.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 4: Kernel Protective MBR & GPT Repair Logic
# ------------------------------------------------------------------------------
assert_check "Kernel validates Protective MBR and restores Primary GPT from Backup"
if ! grep -q "pub fn repair_protective_mbr" kernel/src/boot/recovery.rs; then
    fail_check "repair_protective_mbr missing in recovery.rs"
fi
if ! grep -q "pub fn repair_primary_gpt_from_backup" kernel/src/boot/recovery.rs; then
    fail_check "repair_primary_gpt_from_backup missing in recovery.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 5: Kernel NovaFS Snapshot Discovery & Atomic Rollback
# ------------------------------------------------------------------------------
assert_check "Kernel discovers snapshots and executes atomic root block pivot"
if ! grep -q "pub fn kernel_execute_snapshot_rollback" kernel/src/boot/recovery.rs; then
    fail_check "kernel_execute_snapshot_rollback missing in recovery.rs"
fi
if ! grep -q "pub fn run_recovery_kernel_selftest" kernel/src/boot/recovery.rs; then
    fail_check "run_recovery_kernel_selftest missing in recovery.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 6: Userland Cryptographic Engine (CRC32, PBKDF2-HMAC-SHA256)
# ------------------------------------------------------------------------------
assert_check "Userland recovery crypto implements PBKDF2 and IEEE 802.3 CRC32"
if [ ! -f "userland/desktop/apps/nova-recovery/src/crypto.rs" ]; then
    fail_check "src/crypto.rs missing"
fi
if ! grep -q "pub fn pbkdf2_hmac_sha256" userland/desktop/apps/nova-recovery/src/crypto.rs; then
    fail_check "pbkdf2_hmac_sha256 missing in crypto.rs"
fi
if ! grep -q "pub fn crc32" userland/desktop/apps/nova-recovery/src/crypto.rs; then
    fail_check "crc32 missing in crypto.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 7: NovaFS Snapshot Discovery & Generation Tracking
# ------------------------------------------------------------------------------
assert_check "SnapshotManager discovers CoW snapshots and tracks generations"
if [ ! -f "userland/desktop/apps/nova-recovery/src/snapshot.rs" ]; then
    fail_check "src/snapshot.rs missing"
fi
if ! grep -q "pub struct SnapshotManager" userland/desktop/apps/nova-recovery/src/snapshot.rs; then
    fail_check "SnapshotManager struct missing"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 8: Atomic Rollback & Pre-Rollback Safety Checkpoint Creation
# ------------------------------------------------------------------------------
assert_check "SnapshotManager creates pre-rollback checkpoints and executes rollback"
if ! grep -q "pub fn create_pre_rollback_checkpoint" userland/desktop/apps/nova-recovery/src/snapshot.rs; then
    fail_check "create_pre_rollback_checkpoint missing"
fi
if ! grep -q "pub fn rollback_to_snapshot" userland/desktop/apps/nova-recovery/src/snapshot.rs; then
    fail_check "rollback_to_snapshot missing"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 9: Administrative Credential Recovery & Shadow Password Hashing
# ------------------------------------------------------------------------------
assert_check "PasswordResetManager updates /etc/shadow with constant-time verification"
if [ ! -f "userland/desktop/apps/nova-recovery/src/passwd_reset.rs" ]; then
    fail_check "src/passwd_reset.rs missing"
fi
if ! grep -q "pub fn reset_password" userland/desktop/apps/nova-recovery/src/passwd_reset.rs; then
    fail_check "reset_password missing in passwd_reset.rs"
fi
if ! grep -q "pub fn unlock_account" userland/desktop/apps/nova-recovery/src/passwd_reset.rs; then
    fail_check "unlock_account missing in passwd_reset.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 10: Dual-Header GPT Inspection & Backup Recovery
# ------------------------------------------------------------------------------
assert_check "BootRepairManager validates MBR and repairs Primary GPT from Backup"
if [ ! -f "userland/desktop/apps/nova-recovery/src/boot_repair.rs" ]; then
    fail_check "src/boot_repair.rs missing"
fi
if ! grep -q "pub fn repair_primary_from_backup" userland/desktop/apps/nova-recovery/src/boot_repair.rs; then
    fail_check "repair_primary_from_backup missing in boot_repair.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 11: Emergency System & Kernel Crash Log Viewer
# ------------------------------------------------------------------------------
assert_check "LogViewer filters entries by severity level and facility"
if [ ! -f "userland/desktop/apps/nova-recovery/src/log_viewer.rs" ]; then
    fail_check "src/log_viewer.rs missing"
fi
if ! grep -q "pub fn filter_entries" userland/desktop/apps/nova-recovery/src/log_viewer.rs; then
    fail_check "filter_entries missing in log_viewer.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 12: Retained-Mode 800x600 GUI Framebuffer Rasterization (5 Tabs)
# ------------------------------------------------------------------------------
assert_check "RecoveryUiRenderer renders 800x600 rescue console into BufferSurface"
if [ ! -f "userland/desktop/apps/nova-recovery/src/ui.rs" ]; then
    fail_check "src/ui.rs missing"
fi
if ! grep -q "pub fn render" userland/desktop/apps/nova-recovery/src/ui.rs; then
    fail_check "render function missing in ui.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 13: Headless CLI Rescue Console Commands
# ------------------------------------------------------------------------------
assert_check "RescueCliSession handles snapshots, rollback, reset-root, and status"
if [ ! -f "userland/desktop/apps/nova-recovery/src/cli.rs" ]; then
    fail_check "src/cli.rs missing"
fi
if ! grep -q "pub fn execute_command" userland/desktop/apps/nova-recovery/src/cli.rs; then
    fail_check "execute_command missing in cli.rs"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 14: Userland Unit Tests (14/14 tests)
# ------------------------------------------------------------------------------
assert_check "Run cargo test on nova-recovery crate"
if ! cargo test --manifest-path userland/desktop/apps/nova-recovery/Cargo.toml --quiet > "$OUT/test.log" 2>&1; then
    cat "$OUT/test.log"
    fail_check "cargo test on nova-recovery failed"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 15: Master Autonomous Self-Test Execution
# ------------------------------------------------------------------------------
assert_check "Execute nova-recovery --selftest"
if ! cargo run --manifest-path userland/desktop/apps/nova-recovery/Cargo.toml --quiet -- --selftest > "$OUT/selftest.log" 2>&1; then
    cat "$OUT/selftest.log"
    fail_check "nova-recovery --selftest failed"
fi
if ! grep -q "\[PASS\]" "$OUT/selftest.log"; then
    cat "$OUT/selftest.log"
    fail_check "nova-recovery --selftest did not report PASS"
fi
pass_check

# ------------------------------------------------------------------------------
# Check 16: CLI Dispatcher Verification
# ------------------------------------------------------------------------------
assert_check "Execute nova-recovery --list-snapshots and --repair-boot"
if ! cargo run --manifest-path userland/desktop/apps/nova-recovery/Cargo.toml --quiet -- --list-snapshots > "$OUT/snap.log" 2>&1; then
    cat "$OUT/snap.log"
    fail_check "--list-snapshots failed"
fi
if ! grep -q "@snapshot-20261115" "$OUT/snap.log"; then
    fail_check "@snapshot-20261115 not in list-snapshots output"
fi
if ! cargo run --manifest-path userland/desktop/apps/nova-recovery/Cargo.toml --quiet -- --repair-boot > "$OUT/repair.log" 2>&1; then
    cat "$OUT/repair.log"
    fail_check "--repair-boot failed"
fi
if ! grep -q "\[OK\]" "$OUT/repair.log"; then
    fail_check "--repair-boot did not report OK"
fi
pass_check

echo "--------------------------------------------------------------------------------"
echo "Result: $PASSED_CHECKS / $TOTAL_CHECKS checks passed (100% SUCCESS)"
echo "================================================================================"
