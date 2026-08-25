#!/bin/sh
# Builds disk.img as a genuinely partitioned disk (Phase 25): an MBR
# partition table with partition 1 = FAT32 (all the existing fixtures,
# unchanged) and partition 2 = a real ext2 filesystem. Requires
# parted, mtools (mformat/mcopy), and e2fsprogs (mkfs.ext2, debugfs) -
# installed by `make setup` alongside every other build dependency.
#
# Partition images are built as separate small files and then dd'd
# into the combined image at the exact byte offsets parted assigned -
# this avoids needing loop devices (losetup) or actually mounting
# anything, which may not be available/permitted in every build
# environment (e.g. some sandboxed CI containers).
set -e
cd "$(dirname "$0")/.."   # repo root

DISK_IMG="${1:-disk.img}"
FIXTURES=tools/fixtures
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

TOTAL_MB=98
PART1_MB=64   # FAT32 - matches the pre-Phase-25 whole-disk size
PART2_MB=32   # ext2

# --- Partition 1: FAT32, exactly the same fixtures as before ---
dd if=/dev/zero of="$TMP/part1.img" bs=1M count=$PART1_MB status=none
mformat -i "$TMP/part1.img" -F ::
for f in HELLO.TXT EDITOR.PKG GAME.PKG SYSTEM.CFG HELLO.ELF HELLOC.ELF; do
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

# --- Combine: blank image, MBR partition table, then dd both
# partition images in at parted's chosen offsets ---
dd if=/dev/zero of="$DISK_IMG" bs=1M count=$TOTAL_MB status=none
parted -s "$DISK_IMG" mklabel msdos >/dev/null 2>&1
parted -s "$DISK_IMG" mkpart primary fat32 1MiB $((PART1_MB + 1))MiB >/dev/null 2>&1
parted -s "$DISK_IMG" mkpart primary ext2 $((PART1_MB + 1))MiB $((PART1_MB + PART2_MB + 1))MiB >/dev/null 2>&1

# 2048 sectors (1MiB) is parted's standard alignment for the first
# partition; the second partition starts wherever the first one ends,
# which parted also aligns to a clean sector boundary. Read the real
# offsets back rather than assuming, in case parted's alignment choice
# ever changes.
PART1_START=$(parted -s "$DISK_IMG" unit s print | awk '/^ 1/ {gsub("s","",$2); print $2}')
PART2_START=$(parted -s "$DISK_IMG" unit s print | awk '/^ 2/ {gsub("s","",$2); print $2}')

dd if="$TMP/part1.img" of="$DISK_IMG" bs=512 seek="$PART1_START" conv=notrunc status=none
dd if="$TMP/part2.img" of="$DISK_IMG" bs=512 seek="$PART2_START" conv=notrunc status=none

echo "Built $DISK_IMG: MBR, partition 1 (FAT32) at sector $PART1_START, partition 2 (ext2) at sector $PART2_START"
