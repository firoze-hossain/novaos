#!/bin/bash
echo "🪟 Native Windows shell detected (Git Bash / MSYS2 / cmd)."
echo ""
echo "NovaOS's toolchain (a 32-bit bare-metal GCC target, NASM, and"
echo "grub-mkrescue) is not available natively on Windows. You have two"
echo "supported options - both covered in detail in TESTING.md:"
echo ""
echo "  1) WSL2 (recommended): wsl --install, then open the Ubuntu shell"
echo "     and run 'make setup && make && make run' from there."
echo ""
echo "  2) Docker Desktop: docker build -t novaos-build ."
echo "     docker run --rm -v \"\$PWD\":/novaos -w /novaos novaos-build make"
echo ""
exit 1
