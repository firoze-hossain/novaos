#!/bin/sh
# Rebuilds tools/fixtures/HELLO.ELF (ELF32) and tools/fixtures/HELLO64.ELF (ELF64).
# Requires nasm and GNU ld (both already required to build NovaOS itself).
# Run from the repo root:
#   ./tools/elf-fixtures/build.sh
set -e
cd "$(dirname "$0")"

# 1. 32-bit ELF fixture (i686)
nasm -f elf32 hello.asm -o hello.o
ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o hello.elf hello.o
cp hello.elf ../fixtures/HELLO.ELF
echo "Built tools/fixtures/HELLO.ELF (ELF32)"

# 2. 64-bit ELF fixture (x86_64 Long Mode)
nasm -f elf64 hello64.asm -o hello64.o
ld -m elf_x86_64 -Ttext=0x00400000 --entry=_start -static -o hello64.elf hello64.o
cp hello64.elf ../fixtures/HELLO64.ELF
echo "Built tools/fixtures/HELLO64.ELF (ELF64)"
