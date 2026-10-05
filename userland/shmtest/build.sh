#!/bin/sh
# Builds userland/shmtest/shmtest.c against the NovaOS libc into
# tools/fixtures/SHMTEST.ELF - the in-OS conformance test for the SYS_SHM_*
# shared-memory IPC syscalls (see shmtest.c). Same toolchain and technique
# as userland/gfxtest/build.sh.
#
# Run from the repo root:
#   ./userland/shmtest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c shmtest.c -o shmtest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o shmtest.elf \
    crt0.o shmtest_main.o syscall.o string.o stdio.o stdlib.o

cp shmtest.elf ../../tools/fixtures/SHMTEST.ELF
rm -f *.o shmtest.elf
echo "Built tools/fixtures/SHMTEST.ELF"
