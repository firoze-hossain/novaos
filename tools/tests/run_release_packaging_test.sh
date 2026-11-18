#!/bin/sh
# ==============================================================================
#  NovaOS Month 6: Dual-Edition Release Packaging Qualification Suite
# ==============================================================================
#
# Validates production release media pipeline:
# 1. tools/release/build_release_images.sh execution
# 2. novaos-desktop-2.0.iso (Hybrid El Torito BIOS + UEFI bootability)
# 3. novaos-server-2.0.iso  (Hybrid El Torito BIOS + UEFI bootability)
# 4. novaos-desktop-2.0.img (GPT Hybrid Partition Table: ESP, SYS, DATA, JNL, CRASH)
# 5. novaos-server-2.0.img  (GPT Hybrid Partition Table: ESP, SYS, DATA, JNL, CRASH)
# 6. Standalone Kernel Binaries (novaos.bin, novaos-server.bin)
# 7. Cryptographic SHA-256 Checksum Integrity (SHA256SUMS)
# 8. Cryptographic GPG Detached Signatures (SHA256SUMS.asc, *.sig)
# 9. JSON Release Manifest (release_manifest.json)
#
# Usage:
#   ./tools/tests/run_release_packaging_test.sh
# ==============================================================================

set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

TESTS_RUN=0
TESTS_PASSED=0
TESTS_FAILED=0

pass() {
    TESTS_RUN=$((TESTS_RUN + 1))
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo "  [PASS] $1"
}

fail() {
    TESTS_RUN=$((TESTS_RUN + 1))
    TESTS_FAILED=$((TESTS_FAILED + 1))
    echo "  [FAIL] $1"
}

echo "========================================================================"
echo "    NovaOS Release Track: Dual-Edition Release Packaging Test Suite     "
echo "========================================================================"
echo "Date:   $(date)"
echo "Target: release/v2.0 (Desktop ISO, Server ISO, GPT Images & GPG Signing)"
echo "------------------------------------------------------------------------"

# ------------------------------------------------------------------------------
# Stage 1: Pipeline Scripts & Rust Engine Validation
# ------------------------------------------------------------------------------
echo ""
echo "[1/8] Validating Packaging Scripts & Rust novarelease Tool..."

if [ -x tools/release/build_release_images.sh ]; then
    pass "build_release_images.sh is present and executable"
else
    fail "build_release_images.sh is not executable"
fi

if [ -f tools/release/Cargo.toml ]; then
    pass "tools/release/Cargo.toml exists"
else
    fail "tools/release/Cargo.toml missing"
fi

# Run cargo test on novarelease crate
if cargo test --manifest-path tools/release/Cargo.toml >/dev/null 2>&1; then
    pass "novarelease Rust crate unit tests pass (format_guid, u32_le)"
else
    fail "novarelease Rust crate unit tests failed"
fi

# ------------------------------------------------------------------------------
# Stage 2: Pipeline Execution Test
# ------------------------------------------------------------------------------
echo ""
echo "[2/8] Executing Automated Release Packaging Pipeline..."

TEST_REL_DIR="build/test-release-v2.0"
rm -rf "$TEST_REL_DIR"

if ./tools/release/build_release_images.sh --release-dir "$TEST_REL_DIR" --skip-build --version "2.0.0-test" >/dev/null 2>&1; then
    pass "build_release_images.sh executed cleanly (exit code 0)"
else
    fail "build_release_images.sh failed during execution"
fi

# ------------------------------------------------------------------------------
# Stage 3: Artifact Presence & Sizing Verification
# ------------------------------------------------------------------------------
echo ""
echo "[3/8] Verifying Required Release Deliverables in $TEST_REL_DIR..."

check_deliverable() {
    fname="$1"
    min_size="$2"
    desc="$3"
    fpath="$TEST_REL_DIR/$fname"
    if [ -f "$fpath" ]; then
        sz=$(wc -c < "$fpath")
        if [ "$sz" -ge "$min_size" ]; then
            pass "$fname ($desc) present with valid size ($sz bytes >= $min_size)"
        else
            fail "$fname size too small: $sz bytes < $min_size"
        fi
    else
        fail "$fname ($desc) is missing"
    fi
}

check_deliverable "novaos-desktop-2.0.iso" 15000000 "Desktop Hybrid ISO"
check_deliverable "novaos-server-2.0.iso"  15000000 "Server Hybrid ISO"
check_deliverable "novaos-desktop-2.0.img" 100000000 "Desktop GPT Disk Image"
check_deliverable "novaos-server-2.0.img"  100000000 "Server GPT Disk Image"
check_deliverable "novaos.bin"             1000000 "Desktop Multiboot Kernel Binary"
check_deliverable "novaos-server.bin"      1000000 "Server Multiboot Kernel Binary"
check_deliverable "release_manifest.json"  500 "Release Manifest JSON"
check_deliverable "SHA256SUMS"             200 "Cryptographic Checksum Matrix"
check_deliverable "SHA256SUMS.asc"         100 "GPG ASCII-Armored Detached Signature"
check_deliverable "novaos-release-2.0-key.asc" 100 "Exported GPG Public Key"

# ------------------------------------------------------------------------------
# Stage 4: El Torito Dual-Boot Catalog Verification
# ------------------------------------------------------------------------------
echo ""
echo "[4/8] Validating El Torito Hybrid Boot (BIOS + UEFI) on Both ISOs..."

NOVARELEASE_BIN="tools/release/target/release/novarelease"

# Desktop ISO El Torito check
if "$NOVARELEASE_BIN" inspect-eltorito "$TEST_REL_DIR/novaos-desktop-2.0.iso" | grep -q "BIOS Boot Media:    BOOTABLE" && \
   "$NOVARELEASE_BIN" inspect-eltorito "$TEST_REL_DIR/novaos-desktop-2.0.iso" | grep -q "UEFI Boot Media:    BOOTABLE"; then
    pass "novaos-desktop-2.0.iso has verified dual BIOS + UEFI bootable El Torito catalog"
else
    fail "novaos-desktop-2.0.iso El Torito verification failed"
fi

# Server ISO El Torito check
if "$NOVARELEASE_BIN" inspect-eltorito "$TEST_REL_DIR/novaos-server-2.0.iso" | grep -q "BIOS Boot Media:    BOOTABLE" && \
   "$NOVARELEASE_BIN" inspect-eltorito "$TEST_REL_DIR/novaos-server-2.0.iso" | grep -q "UEFI Boot Media:    BOOTABLE"; then
    pass "novaos-server-2.0.iso has verified dual BIOS + UEFI bootable El Torito catalog"
else
    fail "novaos-server-2.0.iso El Torito verification failed"
fi

# ------------------------------------------------------------------------------
# Stage 5: GPT Partition Table Verification
# ------------------------------------------------------------------------------
echo ""
echo "[5/8] Validating GPT Partition Table Structure & ESP Alignment..."

# Desktop GPT disk image
if "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "Primary Header:   VALID" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "Backup Header:    VALID" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "ESP" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "NOVAOS_SYS" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "NOVAOS_DATA" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "NOVAOS_JNL" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-desktop-2.0.img" | grep -q "NOVAOS_CRASH"; then
    pass "novaos-desktop-2.0.img has valid GPT table with 5 verified partitions (ESP, SYS, DATA, JNL, CRASH)"
else
    fail "novaos-desktop-2.0.img GPT verification failed"
fi

# Server GPT disk image
if "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "Primary Header:   VALID" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "Backup Header:    VALID" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "ESP" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "NOVAOS_SYS" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "NOVAOS_DATA" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "NOVAOS_JNL" && \
   "$NOVARELEASE_BIN" inspect-gpt "$TEST_REL_DIR/novaos-server-2.0.img" | grep -q "NOVAOS_CRASH"; then
    pass "novaos-server-2.0.img has valid GPT table with 5 verified partitions (ESP, SYS, DATA, JNL, CRASH)"
else
    fail "novaos-server-2.0.img GPT verification failed"
fi

# ------------------------------------------------------------------------------
# Stage 6: SHA-256 Checksum Matrix Integrity
# ------------------------------------------------------------------------------
echo ""
echo "[6/8] Verifying Cryptographic Checksums (SHA256SUMS)..."

cd "$TEST_REL_DIR"
if sha256sum -c SHA256SUMS >/dev/null 2>&1; then
    pass "SHA256SUMS verification passed: 100% of files match their recorded SHA-256 digests"
else
    fail "SHA256SUMS verification failed"
fi
cd "$REPO_ROOT"

# ------------------------------------------------------------------------------
# Stage 7: GPG Release Signature Verification
# ------------------------------------------------------------------------------
echo ""
echo "[7/8] Verifying Enterprise GPG Detached Signatures..."

GPG_HOMEDIR="$TEST_REL_DIR/keys/gnupg"
if gpg --homedir "$GPG_HOMEDIR" --verify "$TEST_REL_DIR/SHA256SUMS.asc" "$TEST_REL_DIR/SHA256SUMS" >/dev/null 2>&1; then
    pass "GPG detached signature verification on SHA256SUMS.asc passed"
else
    fail "GPG detached signature verification on SHA256SUMS.asc failed"
fi

if gpg --homedir "$GPG_HOMEDIR" --verify "$TEST_REL_DIR/novaos-desktop-2.0.iso.sig" "$TEST_REL_DIR/novaos-desktop-2.0.iso" >/dev/null 2>&1; then
    pass "GPG detached signature on novaos-desktop-2.0.iso verified"
else
    fail "GPG detached signature on novaos-desktop-2.0.iso failed"
fi

if gpg --homedir "$GPG_HOMEDIR" --verify "$TEST_REL_DIR/novaos-server-2.0.iso.sig" "$TEST_REL_DIR/novaos-server-2.0.iso" >/dev/null 2>&1; then
    pass "GPG detached signature on novaos-server-2.0.iso verified"
else
    fail "GPG detached signature on novaos-server-2.0.iso failed"
fi

# ------------------------------------------------------------------------------
# Stage 8: Release Manifest Schema Verification
# ------------------------------------------------------------------------------
echo ""
echo "[8/8] Validating JSON Release Manifest Schema..."

if python3 -c "
import json, sys
with open('$TEST_REL_DIR/release_manifest.json') as f:
    data = json.load(f)
assert data['project'] == 'NovaOS', 'Invalid project name'
assert data['version'] == '2.0.0-test', 'Invalid version string'
assert len(data['artifacts']) == 6, f'Expected 6 artifacts, got {len(data[\"artifacts\"])}'
filenames = {a['filename'] for a in data['artifacts']}
expected = {'novaos-desktop-2.0.iso', 'novaos-server-2.0.iso', 'novaos-desktop-2.0.img', 'novaos-server-2.0.img', 'novaos.bin', 'novaos-server.bin'}
assert filenames == expected, f'Artifact set mismatch: {filenames}'
for a in data['artifacts']:
    assert len(a['sha256']) == 64, f'Invalid sha256 hash length for {a[\"filename\"]}'
    assert a['size_bytes'] > 0, f'Zero size for {a[\"filename\"]}'
" >/dev/null 2>&1; then
    pass "release_manifest.json schema, versioning, and artifact records 100% valid"
else
    fail "release_manifest.json schema validation failed"
fi

# Clean up test output directory
rm -rf "$TEST_REL_DIR"

# ------------------------------------------------------------------------------
# Summary
# ------------------------------------------------------------------------------
echo ""
echo "========================================================================"
echo " Dual-Edition Release Packaging Qualification: $TESTS_PASSED / $TESTS_RUN Checks Passed"
echo "========================================================================"

if [ "$TESTS_FAILED" -eq 0 ]; then
    echo " ✅ ALL $TESTS_PASSED CHECKS PASSED: Dual-Edition Packaging Pipeline 100% Verified!"
    echo "========================================================================"
    exit 0
else
    echo " ❌ $TESTS_FAILED CHECKS FAILED: Dual-Edition Packaging Pipeline qualification failed."
    echo "========================================================================"
    exit 1
fi
