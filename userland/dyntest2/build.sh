#!/bin/sh
# Builds userland/dyntest2/dyntest2.c against the NovaOS libc AND
# DYNLIB.SO into DYNTEST2.ELF - a SECOND, independently built,
# dynamically-linked consumer of the same shared library DYNTEST.ELF
# uses. See dyntest2.c's own comment for why this one exists at all
# (it's the actual proof of on-disk sharing, not just a second test),
# and userland/dyntest/build.sh for the flags used here.
#
# Run from the repo root:
#   ./userland/dynlib/build.sh    (DYNLIB.SO must exist first)
#   ./userland/dyntest2/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"
DYNLIB=../../tools/fixtures/DYNLIB.SO

if [ ! -f "$DYNLIB" ]; then
    echo "DYNLIB.SO not found - run userland/dynlib/build.sh first" >&2
    exit 1
fi

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c dyntest2.c -o dyntest2_main.o

ld -m elf_i386 --entry=_start --hash-style=sysv --no-dynamic-linker \
    -o dyntest2.elf \
    crt0.o dyntest2_main.o syscall.o string.o stdio.o stdlib.o "$DYNLIB"

cp dyntest2.elf ../../tools/fixtures/DYNTEST2.ELF
rm -f *.o dyntest2.elf
echo "Built tools/fixtures/DYNTEST2.ELF"
