#!/bin/sh
# Builds disk.img as a genuinely partitioned disk (Phase 25): an MBR
# partition table with partition 1 = FAT32 (all the existing fixtures,
# unchanged), partition 2 = a real ext2 filesystem, and (Phase 53)
# partition 3 = FAT32's dedicated write-ahead journal region - see
# kernel/rust/journal.rs's own header comment for what lives there and
# why it needs to be its own partition rather than sharing space
# inside partition 1 (FAT32's own "reserved sectors" area is far too
# small - typically 32 sectors/16KB - to hold this module's
# descriptor+data layout, which alone needs 130 sectors, before even
# leaving room for the self-test's own scratch sectors). Requires
# parted, mtools (mformat/mcopy), and e2fsprogs (mkfs.ext2, debugfs) -
# installed by `make setup` alongside every other build dependency.
#
# Partition images are built as separate small files and then dd'd
# into the combined image at the exact byte offsets parted assigned -
# this avoids needing loop devices (losetup) or actually mounting
# anything, which may not be available/permitted in every build
# environment (e.g. some sandboxed CI containers). The journal
# partition needs no filesystem of its own - kernel/rust/journal.rs
# reads/writes it directly as raw sectors - so, unlike partitions 1
# and 2, no image is built for it at all; it's left as the zeroed
# space dd'ing the rest of the disk already produces (a freshly-zeroed
# journal region is exactly what "no transaction has ever run here"
# looks like to journal.rs's own recovery logic - see that file's
# header comment on its MAGIC sentinel).
set -e
cd "$(dirname "$0")/.."   # repo root

DISK_IMG="${1:-disk.img}"
FIXTURES=tools/fixtures
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

PART1_MB=64   # FAT32 - matches the pre-Phase-25 whole-disk size
PART2_MB=32   # ext2
PART3_MB=4    # Phase 53: journal - 4MB is far more than the ~130
              # sectors (65KB) this module's fixed descriptor+data
              # layout actually needs, leaving generous headroom for
              # both a possible future increase to journal.rs's own
              # MAX_TXN_BLOCKS and the self-test's own scratch sectors
              # (see rust_journal_selftest()'s doc comment) without
              # ever touching the real descriptor+data area.
TOTAL_MB=$((1 + PART1_MB + PART2_MB + PART3_MB + 1))  # +1MiB leading
                                                        # alignment gap,
                                                        # +1MiB trailing
                                                        # slack

# --- Partition 1: FAT32, exactly the same fixtures as before ---
dd if=/dev/zero of="$TMP/part1.img" bs=1M count=$PART1_MB status=none
mformat -i "$TMP/part1.img" -F ::
for f in HELLO.TXT EDITOR.PKG GAME.PKG SYSTEM.CFG USERS.CFG HELLO.ELF HELLOC.ELF CAT.ELF SHELL.ELF GUI.ELF PING.ELF; do
    mcopy -i "$TMP/part1.img" "$FIXTURES/$f" "::$f"
done

# --- Partition 2: a real ext2 filesystem, populated via debugfs (no
# mount/loop device needed) ---
dd if=/dev/zero of="$TMP/part2.img" bs=1M count=$PART2_MB status=none
mkfs.ext2 -q -F "$TMP/part2.img"
for f in "$FIXTURES"/ext2root/*; do
    name=$(basename "$f")
    debugfs -w -R "write $f $name" "$TMP/part2.img" >/dev/null 2>&1
done

# --- Combine: blank image, MBR partition table, then dd the two real
# partition images in at parted's chosen offsets. Partition 3 (the
# journal) gets a table entry but no dd - see this script's own header
# comment on why the pre-zeroed space is already the correct initial
# state. ---
dd if=/dev/zero of="$DISK_IMG" bs=1M count=$TOTAL_MB status=none
parted -s "$DISK_IMG" mklabel msdos >/dev/null 2>&1
parted -s "$DISK_IMG" mkpart primary fat32 1MiB $((PART1_MB + 1))MiB >/dev/null 2>&1
parted -s "$DISK_IMG" mkpart primary ext2 $((PART1_MB + 1))MiB $((PART1_MB + PART2_MB + 1))MiB >/dev/null 2>&1
parted -s "$DISK_IMG" mkpart primary ext2 $((PART1_MB + PART2_MB + 1))MiB $((PART1_MB + PART2_MB + PART3_MB + 1))MiB >/dev/null 2>&1

# 2048 sectors (1MiB) is parted's standard alignment for the first
# partition; each subsequent partition starts wherever the previous
# one ends, which parted also aligns to a clean sector boundary. Read
# the real offsets back rather than assuming, in case parted's
# alignment choice ever changes.
PART1_START=$(parted -s "$DISK_IMG" unit s print | awk '/^ 1/ {gsub("s","",$2); print $2}')
PART2_START=$(parted -s "$DISK_IMG" unit s print | awk '/^ 2/ {gsub("s","",$2); print $2}')
PART3_START=$(parted -s "$DISK_IMG" unit s print | awk '/^ 3/ {gsub("s","",$2); print $2}')

dd if="$TMP/part1.img" of="$DISK_IMG" bs=512 seek="$PART1_START" conv=notrunc status=none
dd if="$TMP/part2.img" of="$DISK_IMG" bs=512 seek="$PART2_START" conv=notrunc status=none

echo "Built $DISK_IMG: MBR, partition 1 (FAT32) at sector $PART1_START, partition 2 (ext2) at sector $PART2_START, partition 3 (journal) at sector $PART3_START"
