#!/bin/bash
echo "🐧 Setting up NovaOS for Linux..."

# Check if apt is available
if command -v apt &> /dev/null; then
    sudo apt update
    sudo apt install -y \
        nasm gcc g++ make qemu-system-x86 \
        xorriso grub-pc-bin gdb build-essential \
        mtools
else
    echo "Please install dependencies manually:"
    echo "  - nasm, gcc, make, qemu, xorriso, grub, gdb"
fi

echo "✅ Linux setup complete!"
echo "Run 'make' to build NovaOS"