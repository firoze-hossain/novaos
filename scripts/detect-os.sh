#!/bin/bash
# Detect OS and set variables

if [[ "$OSTYPE" == "darwin"* ]]; then
    echo "mac"
elif [[ "$OSTYPE" == "linux-gnu"* ]]; then
    echo "linux"
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then
    # Git Bash / MSYS2 / native Windows shell - NovaOS's toolchain
    # (gcc -m32 targeting bare-metal ELF, nasm, grub-mkrescue) isn't
    # available natively on Windows. See TESTING.md: the supported path
    # is WSL2 (reports as "linux"), where this script just re-runs
    # setup-linux.sh automatically.
    echo "windows-native-unsupported"
else
    echo "unknown"
fi