#!/bin/sh
# Boot-tests NovaOS's custom bootloader (Phase 28c) end-to-end: real
# stage1/stage2 boot code -> the actual kernel binary -> the full
# self-test suite, including the FAT32+ext2 data disk, exactly like
# `make test` does via GRUB. Confirms the custom bootloader is a
# genuine, complete alternative, not just "boots to a shell."
#
# Disk ordering note: disk.img must be attached as the primary IDE
# master (index=0) - that's the fixed position this kernel's ATA
# driver reads from, unrelated to which disk the BIOS actually boots
# from. custom-boot.img is attached at a different IDE position with
# bootindex=0, so SeaBIOS boots from it while the kernel still finds
# disk.img exactly where it always has. Getting this backwards doesn't
# break the bootloader itself - it was verified working correctly
# first - but does make the kernel unable to find its data disk, which
# looks confusingly similar to a real failure. See PROGRESS.md.
set -e
cd "$(dirname "$0")/../.."   # repo root

make clean >/dev/null 2>&1 || true
make >/dev/null
make disk.img >/dev/null
./tools/custom-boot/build-image.sh custom-boot.img

LOG=$(mktemp)
timeout 25 qemu-system-x86_64 -M pc,smm=off \
    -drive file=disk.img,format=raw,if=ide,index=0,media=disk \
    -drive file=custom-boot.img,format=raw,if=none,id=cbdrive \
    -device ide-hd,drive=cbdrive,bus=ide.1,unit=0,bootindex=0 \
    -netdev user,id=net0,tftp=tools/fixtures/tftproot \
    -device rtl8139,netdev=net0,mac=52:54:00:12:34:56 \
    -device piix3-usb-uhci -device usb-kbd \
    -audiodev none,id=noaudio -device AC97,audiodev=noaudio \
    -m 512M -display none -serial file:"$LOG" -no-reboot || true

if grep -q "FAT32 mounted" "$LOG" && \
   grep -q "process_exec: loaded 'HELLOC.ELF'" "$LOG" && \
   grep -q "process_fork: pid .* forked" "$LOG" && \
   grep -q "UHCI controller at PCI" "$LOG" && \
   grep -q "EXT2 WRITE.READBACK OK" "$LOG" && \
   ! grep -q "PANIC" "$LOG" && \
   ! grep -q "\[sandbox\] FAIL" "$LOG"; then
    echo "✅ Custom bootloader test PASSED - full self-test suite ran"
    echo "   correctly via stage1/stage2, not GRUB."
    rm -f "$LOG"
    exit 0
else
    echo "❌ Custom bootloader test FAILED - see log:"
    cat "$LOG"
    rm -f "$LOG"
    exit 1
fi
