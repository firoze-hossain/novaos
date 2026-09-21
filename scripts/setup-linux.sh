#!/bin/bash
echo "🐧 Setting up NovaOS for Linux..."

# Check if apt is available
if command -v apt &> /dev/null; then
    sudo apt update
    # parted and e2fsprogs (mkfs.ext2, debugfs): tools/build-disk-
    # image.sh needs both for the ext2/journal/crash-dump partitions
    # it builds alongside the original FAT32 one - confirmed missing
    # here (and in .github/workflows/ci.yml, fixed the same way) as
    # the actual root cause of `make disk.img` failing outright, not
    # a defensive addition.
    sudo apt install -y \
        nasm gcc g++ make qemu-system-x86 \
        xorriso grub-pc-bin gdb build-essential \
        mtools parted e2fsprogs
else
    echo "Please install dependencies manually:"
    echo "  - nasm, gcc, make, qemu, xorriso, grub, gdb, parted, e2fsprogs"
fi

echo "✅ Linux setup complete!"
echo "Run 'make' to build NovaOS"