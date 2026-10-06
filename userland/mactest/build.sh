#!/bin/sh
# Builds userland/mactest/mactest.c against the NovaOS libc into
# tools/fixtures/MACTEST.ELF - the unconfined-root DRIVER of the mandatory-access-control conformance test
# (see mactest.c). Same toolchain and
# technique as userland/shmtest/build.sh and userland/gfxtest/build.sh.
#
# Run from the repo root:
#   ./userland/mactest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c mactest.c -o mactest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o mactest.elf \
    crt0.o mactest_main.o syscall.o string.o stdio.o stdlib.o

cp mactest.elf ../../tools/fixtures/MACTEST.ELF
rm -f *.o mactest.elf
echo "Built tools/fixtures/MACTEST.ELF"
