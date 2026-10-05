#!/bin/sh
# Builds userland/msgtest/msgtest.c against the NovaOS libc into
# tools/fixtures/MSGTEST.ELF - the in-OS conformance test for the SYS_MSG_*
# app-to-app messaging syscalls (see msgtest.c). Same toolchain and
# technique as userland/shmtest/build.sh and userland/gfxtest/build.sh.
#
# Run from the repo root:
#   ./userland/msgtest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c msgtest.c -o msgtest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o msgtest.elf \
    crt0.o msgtest_main.o syscall.o string.o stdio.o stdlib.o

cp msgtest.elf ../../tools/fixtures/MSGTEST.ELF
rm -f *.o msgtest.elf
echo "Built tools/fixtures/MSGTEST.ELF"
