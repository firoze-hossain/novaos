#!/bin/sh
# Builds userland/gfxtest/gfxtest.c against the NovaOS libc into
# tools/fixtures/GFXTEST.ELF - the in-OS conformance test for the
# SYS_FB_* framebuffer API (see gfxtest.c). Same toolchain and
# technique as userland/libctest/build.sh.
#
# Run from the repo root:
#   ./userland/gfxtest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c gfxtest.c -o gfxtest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o gfxtest.elf \
    crt0.o gfxtest_main.o syscall.o string.o stdio.o stdlib.o

cp gfxtest.elf ../../tools/fixtures/GFXTEST.ELF
rm -f *.o gfxtest.elf
echo "Built tools/fixtures/GFXTEST.ELF"
