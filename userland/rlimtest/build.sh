#!/bin/sh
# Builds userland/rlimtest/rlimtest.c against the NovaOS libc into
# tools/fixtures/RLIMTEST.ELF - the in-OS conformance test for the SYS_RLIMIT
# per-process resource limits (see rlimtest.c). Same toolchain and
# technique as userland/shmtest/build.sh and userland/gfxtest/build.sh.
#
# Run from the repo root:
#   ./userland/rlimtest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c rlimtest.c -o rlimtest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o rlimtest.elf \
    crt0.o rlimtest_main.o syscall.o string.o stdio.o stdlib.o

cp rlimtest.elf ../../tools/fixtures/RLIMTEST.ELF
rm -f *.o rlimtest.elf
echo "Built tools/fixtures/RLIMTEST.ELF"
