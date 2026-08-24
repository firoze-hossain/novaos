#!/bin/sh
# Rebuilds tools/fixtures/HELLO.ELF from tools/elf-fixtures/hello.asm.
# Requires nasm and GNU ld (both already required to build NovaOS
# itself). Run from the repo root:
#   ./tools/elf-fixtures/build.sh
set -e
cd "$(dirname "$0")"
nasm -f elf32 hello.asm -o hello.o
ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o hello.elf hello.o
cp hello.elf ../fixtures/HELLO.ELF
echo "Built tools/fixtures/HELLO.ELF"
