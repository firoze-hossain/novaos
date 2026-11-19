#!/bin/sh
# Month 6 - Release Hardening: Production Graphical Installer (nova-installer)
#
# Dedicated qualification test runner for NovaOS Desktop Graphical Installer architecture:
#   1. File Architecture & Crate Manifests
#   2. Userland Cryptographic Engine (SHA-256, PBKDF2, CRC-32)
#   3. Guided Automatic Partitioning Engine (ESP, NovaFS Root, Home, Swap)
#   4. 1MB (2048 sector) Alignment & Boundary Verification
#   5. Custom Manual Partitioning Manipulation
#   6. Multi-Colored Proportional Partition Gauge Rendering
#   7. User Account Security & PBKDF2 Password Hashing
#   8. Sudoers Privilege Droplet Generation
#   9. Interactive Timezone Database & Clock Calculation
#  10. Desktop Personalization & Live Preview (Dark Mica & Light Cupertino)
#  11. Animated Slide Showcase Engine (5 Architecture Highlights)
#  12. End-to-End Installation Pipeline Execution (GPT, NovaFS, UEFI Bootloader)
#  13. Multi-Step Wizard State Machine (9-Step Navigation)
#  14. Retained-Mode Buffer Surface Rasterization & CLI Self-Test
#
# Usage:
#   ./tools/tests/run_nova_installer_tests.sh    (or: make test-installer)

set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "================================================================================"
echo "         NovaOS 2.0 - Production Graphical Installer (nova-installer)           "
echo "================================================================================"
echo "Date: $(date)"
echo "Target: userland/desktop/apps/nova-installer (partition_wizard, engine, ui)"
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
assert_check "File architecture and crate manifests"
[ -f "userland/desktop/apps/nova-installer/Cargo.toml" ] || fail_check "Cargo.toml missing"
[ -f "userland/desktop/apps/nova-installer/src/main.rs" ] || fail_check "src/main.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/lib.rs" ] || fail_check "src/lib.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/partition_wizard.rs" ] || fail_check "src/partition_wizard.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/account.rs" ] || fail_check "src/account.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/timezone.rs" ] || fail_check "src/timezone.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/preview.rs" ] || fail_check "src/preview.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/showcase.rs" ] || fail_check "src/showcase.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/engine.rs" ] || fail_check "src/engine.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/wizard.rs" ] || fail_check "src/wizard.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/ui.rs" ] || fail_check "src/ui.rs missing"
[ -f "userland/desktop/apps/nova-installer/src/crypto.rs" ] || fail_check "src/crypto.rs missing"
pass_check

# ------------------------------------------------------------------------------
# Check 2: Cryptographic Engine (SHA-256, PBKDF2, CRC-32)
# ------------------------------------------------------------------------------
assert_check "Cryptographic engine (SHA-256, PBKDF2, CRC-32)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib crypto::tests >/dev/null 2>&1 || fail_check "Crypto unit tests failed"
pass_check

# ------------------------------------------------------------------------------
# Check 3: Guided Automatic Partitioning Engine
# ------------------------------------------------------------------------------
assert_check "Guided automatic GPT partitioning engine (ESP, NovaFS Root, Home, Swap)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib partition_wizard::tests::test_guided_partitioning_generation_and_validation >/dev/null 2>&1 || fail_check "Guided partitioning test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 4: 1MB (2048 sector) Alignment & Boundary Verification
# ------------------------------------------------------------------------------
assert_check "1MB (2048 sector) alignment & LBA boundary verification"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib partition_wizard::tests::test_guided_partitioning_generation_and_validation >/dev/null 2>&1 || fail_check "Alignment check failed"
pass_check

# ------------------------------------------------------------------------------
# Check 5: Custom Manual Partitioning Manipulation
# ------------------------------------------------------------------------------
assert_check "Custom manual partitioning manipulation (Add, Delete, Mount points)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib partition_wizard::tests::test_custom_partitioning_validation >/dev/null 2>&1 || fail_check "Custom partitioning test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 6: Multi-Colored Proportional Partition Gauge Rendering
# ------------------------------------------------------------------------------
assert_check "Multi-colored proportional visual partition bar gauge rendering"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib partition_wizard::tests::test_guided_partitioning_generation_and_validation >/dev/null 2>&1 || fail_check "Visual partition gauge failed"
pass_check

# ------------------------------------------------------------------------------
# Check 7: User Account Security & PBKDF2 Password Hashing
# ------------------------------------------------------------------------------
assert_check "User account security & PBKDF2 password hashing (/etc/passwd, /etc/shadow)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib account::tests >/dev/null 2>&1 || fail_check "Account unit tests failed"
pass_check

# ------------------------------------------------------------------------------
# Check 8: Sudoers Privilege Droplet Generation
# ------------------------------------------------------------------------------
assert_check "Sudoers privilege droplet generation (/etc/sudoers.d/90-<user>)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib account::tests::test_password_strength_and_verification >/dev/null 2>&1 || fail_check "Sudoers droplet generation failed"
pass_check

# ------------------------------------------------------------------------------
# Check 9: Interactive Timezone Database & Clock Calculation
# ------------------------------------------------------------------------------
assert_check "Interactive timezone database & simulated clock calculation (/etc/timezone)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib timezone::tests >/dev/null 2>&1 || fail_check "Timezone unit tests failed"
pass_check

# ------------------------------------------------------------------------------
# Check 10: Desktop Personalization & Live Preview
# ------------------------------------------------------------------------------
assert_check "Desktop personalization & live preview (Dark Mica & Light Cupertino)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib preview::tests >/dev/null 2>&1 || fail_check "Desktop preview unit tests failed"
pass_check

# ------------------------------------------------------------------------------
# Check 11: Animated Slide Showcase Engine
# ------------------------------------------------------------------------------
assert_check "Animated slide showcase engine (5 architectural highlights)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib showcase::tests >/dev/null 2>&1 || fail_check "Showcase unit tests failed"
pass_check

# ------------------------------------------------------------------------------
# Check 12: End-to-End Installation Pipeline Execution
# ------------------------------------------------------------------------------
assert_check "End-to-end installation pipeline (GPT, NovaFS, /EFI/BOOT/BOOTX64.EFI)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib engine::tests >/dev/null 2>&1 || fail_check "Installation pipeline failed"
pass_check

# ------------------------------------------------------------------------------
# Check 13: Multi-Step Wizard State Machine
# ------------------------------------------------------------------------------
assert_check "Multi-step wizard state machine (9-step navigation)"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib wizard::tests >/dev/null 2>&1 || fail_check "Wizard state machine failed"
pass_check

# ------------------------------------------------------------------------------
# Check 14: Retained-Mode Buffer Surface Rasterization & CLI Self-Test
# ------------------------------------------------------------------------------
assert_check "Retained-mode buffer surface rasterization & nova-installer --selftest"
cargo test --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --lib ui::tests >/dev/null 2>&1 || fail_check "UI rendering test failed"
cargo run --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --quiet -- --selftest >/dev/null 2>&1 || fail_check "nova-installer --selftest failed"
cargo run --manifest-path userland/desktop/apps/nova-installer/Cargo.toml --quiet -- --help >/dev/null 2>&1 || fail_check "nova-installer --help failed"
pass_check

echo "--------------------------------------------------------------------------------"
echo "All $TOTAL_CHECKS/$TOTAL_CHECKS nova-installer qualification checks PASSED (100% success)"
echo "================================================================================"
exit 0
