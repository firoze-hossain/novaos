#!/bin/sh
# Builds a standalone test disk image for NovaOS's custom bootloader
# (Phase 28c): stage1 (LBA 0) + stage2 (LBA 1-32) + the real kernel
# ELF (LBA 33 onward). Purely additive/parallel to novaos.iso and
# disk.img - neither is touched by this.
set -e
cd "$(dirname "$0")/../.."   # repo root (up from tools/custom-boot/)

OUT="${1:-custom-boot.img}"

nasm -f bin tools/custom-boot/stage1.asm -o /tmp/nb-stage1.bin
nasm -f bin tools/custom-boot/stage2.asm -o /tmp/nb-stage2.bin

STAGE2_SIZE=$(stat -c%s /tmp/nb-stage2.bin)
if [ "$STAGE2_SIZE" -gt 17408 ]; then
    echo "ERROR: stage2.bin ($STAGE2_SIZE bytes) exceeds the 34-sector" >&2
    echo "(17408-byte) budget stage1.asm's STAGE2_SECTOR_COUNT and" >&2
    echo "stage2.asm's KERNEL_LOAD_LBA assume - raise both constants" >&2
    echo "(see their comments) and rebuild." >&2
    exit 1
fi

dd if=/dev/zero of="$OUT" bs=1M count=16 status=none
dd if=/tmp/nb-stage1.bin of="$OUT" conv=notrunc status=none
dd if=/tmp/nb-stage2.bin of="$OUT" bs=512 seek=1 conv=notrunc status=none
dd if=build/novaos.bin of="$OUT" bs=512 seek=35 conv=notrunc status=none

echo "Built $OUT"
