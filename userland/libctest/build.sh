#!/bin/sh
# Builds userland/libctest/libctest.c against the NovaOS libc into
# tools/fixtures/LIBCTEST.ELF - the in-OS libc test (see libctest.c).
# Same toolchain and technique as userland/coreutils/build.sh.
#
# Run from the repo root:
#   ./userland/libctest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c libctest.c -o libctest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o libctest.elf \
    crt0.o libctest_main.o syscall.o string.o stdio.o stdlib.o

cp libctest.elf ../../tools/fixtures/LIBCTEST.ELF
rm -f *.o libctest.elf
echo "Built tools/fixtures/LIBCTEST.ELF"
