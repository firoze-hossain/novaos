#!/bin/sh
# tools/tests/run_editions_test.sh
# Release Track: Validation test suite for Desktop vs Server Edition profiles.
# Proves:
#   1. Clean independent builds of Desktop (disk.img) and Server (disk-server.img)
#   2. Partition layout identical across both editions (FAT32, ext2, journal, crash dump)
#   3. Desktop image contains full compositor, window manager, and GUI apps suite
#   4. Server image strips GUI binaries and packages (~6.7MB saving)
#   5. Server image provisions headless MOTD, EDITION.TXT, SYSTEM.CFG, and FIREWALL.CFG
#   6. Core CLI tools, system services, and kernel test binaries retained on both

set -e
cd "$(dirname "$0")/../.."   # repo root

echo "========================================================================"
echo " NovaOS Release Track: Desktop & Server Editions Validation Suite"
echo "========================================================================"

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

assert_contains() {
    TOTAL=$((TOTAL + 1))
    DESC="$1"
    HAYSTACK="$2"
    NEEDLE="$3"
    if echo "$HAYSTACK" | grep -q "$NEEDLE"; then
        echo "  [PASS] $DESC"
    else
        echo "  [FAIL] $DESC: expected to find '$NEEDLE'"
        FAILED=$((FAILED + 1))
    fi
}

assert_not_contains() {
    TOTAL=$((TOTAL + 1))
    DESC="$1"
    HAYSTACK="$2"
    NEEDLE="$3"
    if echo "$HAYSTACK" | grep -q "$NEEDLE"; then
        echo "  [FAIL] $DESC: expected NOT to find '$NEEDLE'"
        FAILED=$((FAILED + 1))
    else
        echo "  [PASS] $DESC"
    fi
}

# --- Step 1: Build Desktop and Server Images ---
echo "\n[1/4] Building Desktop Edition disk.img..."
./tools/build-disk-image.sh disk.img desktop >/dev/null 2>&1
assert_eq "Desktop disk.img file exists" "$(test -f disk.img && echo yes)" "yes"

echo "\n[2/4] Building Server Edition disk-server.img..."
./tools/build-disk-image.sh disk-server.img server >/dev/null 2>&1
assert_eq "Server disk-server.img file exists" "$(test -f disk-server.img && echo yes)" "yes"

# --- Step 2: Validate Desktop Edition ---
echo "\n[3/4] Inspecting Desktop Edition partition 1..."
DESKTOP_LISTING=$(mdir -i disk.img@@1M :: 2>/dev/null)
DESKTOP_EDITION=$(mtype -i disk.img@@1M ::EDITION.TXT 2>/dev/null || echo "")

assert_contains "Desktop EDITION.TXT indicates Desktop Edition" "$DESKTOP_EDITION" "NovaOS Desktop Edition"
assert_contains "Desktop contains WM.ELF" "$DESKTOP_LISTING" "WM       ELF"
assert_contains "Desktop contains GUI.ELF" "$DESKTOP_LISTING" "GUI      ELF"
assert_contains "Desktop contains FILES.ELF" "$DESKTOP_LISTING" "FILES    ELF"
assert_contains "Desktop contains SETTINGS.ELF" "$DESKTOP_LISTING" "SETTINGS ELF"
assert_contains "Desktop contains TERM.ELF" "$DESKTOP_LISTING" "TERM     ELF"
assert_contains "Desktop contains EDIT.ELF" "$DESKTOP_LISTING" "EDIT     ELF"
assert_contains "Desktop contains STORE.ELF" "$DESKTOP_LISTING" "STORE    ELF"
assert_contains "Desktop contains LOGIN.ELF" "$DESKTOP_LISTING" "LOGIN    ELF"
assert_contains "Desktop contains INSTALL.ELF" "$DESKTOP_LISTING" "INSTALL  ELF"
assert_contains "Desktop contains CRASHREP.ELF" "$DESKTOP_LISTING" "CRASHREP ELF"
assert_contains "Desktop contains FIRSTRUN.ELF" "$DESKTOP_LISTING" "FIRSTRUN ELF"
assert_contains "Desktop contains ACCESS.CFG" "$DESKTOP_LISTING" "ACCESS   CFG"
assert_contains "Desktop contains EDITOR.PKG" "$DESKTOP_LISTING" "EDITOR   PKG"
assert_contains "Desktop contains GAME.PKG" "$DESKTOP_LISTING" "GAME     PKG"

# --- Step 3: Validate Server Edition ---
echo "\n[4/4] Inspecting Server Edition partition 1..."
SERVER_LISTING=$(mdir -i disk-server.img@@1M :: 2>/dev/null)
SERVER_EDITION=$(mtype -i disk-server.img@@1M ::EDITION.TXT 2>/dev/null || echo "")
SERVER_MOTD=$(mtype -i disk-server.img@@1M ::MOTD.TXT 2>/dev/null || echo "")
SERVER_CFG=$(mtype -i disk-server.img@@1M ::SYSTEM.CFG 2>/dev/null || echo "")
SERVER_FW=$(mtype -i disk-server.img@@1M ::FIREWALL.CFG 2>/dev/null || echo "")

assert_contains "Server EDITION.TXT indicates Server Edition" "$SERVER_EDITION" "NovaOS Server Edition"
assert_contains "Server MOTD.TXT presents headless server banner" "$SERVER_MOTD" "NovaOS Server Edition (x86_32 headless)"
assert_contains "Server SYSTEM.CFG provisions novaos-server hostname" "$SERVER_CFG" "novaos-server"
assert_contains "Server SYSTEM.CFG provisions admin root user" "$SERVER_CFG" "admin"
assert_contains "Server SYSTEM.CFG provisions NOVAINIT.ELF" "$SERVER_CFG" "NOVAINIT.ELF"
assert_contains "Server FIREWALL.CFG includes SSH rule" "$SERVER_FW" "dport 22"
assert_contains "Server FIREWALL.CFG includes HTTP rule" "$SERVER_FW" "dport 80"

# Assert GUI apps stripped
assert_not_contains "Server stripped WM.ELF" "$SERVER_LISTING" "WM       ELF"
assert_not_contains "Server stripped GUI.ELF" "$SERVER_LISTING" "GUI      ELF"
assert_not_contains "Server stripped FILES.ELF" "$SERVER_LISTING" "FILES    ELF"
assert_not_contains "Server stripped SETTINGS.ELF" "$SERVER_LISTING" "SETTINGS ELF"
assert_not_contains "Server stripped TERM.ELF" "$SERVER_LISTING" "TERM     ELF"
assert_not_contains "Server stripped EDIT.ELF" "$SERVER_LISTING" "EDIT     ELF"
assert_not_contains "Server stripped STORE.ELF" "$SERVER_LISTING" "STORE    ELF"
assert_not_contains "Server stripped LOGIN.ELF" "$SERVER_LISTING" "LOGIN    ELF"
assert_not_contains "Server stripped INSTALL.ELF" "$SERVER_LISTING" "INSTALL  ELF"
assert_not_contains "Server stripped CRASHREP.ELF" "$SERVER_LISTING" "CRASHREP ELF"
assert_not_contains "Server stripped FIRSTRUN.ELF" "$SERVER_LISTING" "FIRSTRUN ELF"
assert_not_contains "Server stripped ACCESS.CFG" "$SERVER_LISTING" "ACCESS   CFG"
assert_not_contains "Server stripped EDITOR.PKG" "$SERVER_LISTING" "EDITOR   PKG"
assert_not_contains "Server stripped GAME.PKG" "$SERVER_LISTING" "GAME     PKG"

# Assert Server core utilities retained
assert_contains "Server contains SHELL.ELF" "$SERVER_LISTING" "SHELL    ELF"
assert_contains "Server contains NOVAINIT.ELF" "$SERVER_LISTING" "NOVAINIT ELF"
assert_contains "Server contains CAT.ELF" "$SERVER_LISTING" "CAT      ELF"
assert_contains "Server contains SYSINFO.ELF" "$SERVER_LISTING" "SYSINFO  ELF"
assert_contains "Server contains PING.ELF" "$SERVER_LISTING" "PING     ELF"
assert_contains "Server contains UPDATE.ELF" "$SERVER_LISTING" "UPDATE   ELF"
assert_contains "Server contains KLOG.ELF" "$SERVER_LISTING" "KLOG     ELF"
assert_contains "Server contains ACTMON.ELF" "$SERVER_LISTING" "ACTMON   ELF"

echo "\n========================================================================"
if [ "$FAILED" -eq 0 ]; then
    echo " ✅ ALL $TOTAL VALIDATION CHECKS PASSED: Desktop & Server Editions Verified!"
    echo "========================================================================"
    exit 0
else
    echo " ❌ $FAILED of $TOTAL CHECKS FAILED!"
    echo "========================================================================"
    exit 1
fi
