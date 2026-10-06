#!/bin/sh
# Builds userland/macjail/macjail.c against the NovaOS libc into
# tools/fixtures/MACJAIL.ELF - the CONFINED program of the mandatory-access-control conformance test (one binary, installed under several names - see tools/build-disk-image.sh)
# (see macjail.c). Same toolchain and
# technique as userland/shmtest/build.sh and userland/gfxtest/build.sh.
#
# Run from the repo root:
#   ./userland/macjail/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c macjail.c -o macjail_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o macjail.elf \
    crt0.o macjail_main.o syscall.o string.o stdio.o stdlib.o

cp macjail.elf ../../tools/fixtures/MACJAIL.ELF
rm -f *.o macjail.elf
echo "Built tools/fixtures/MACJAIL.ELF"
