#!/bin/sh
# Builds userland/fwtest/fwtest.c against the NovaOS libc into
# tools/fixtures/FWTEST.ELF - the in-OS conformance test for the stateful
# firewall (SYS_FW_INFO / SYS_FW_CTL, kernel/rust/firewall.rs): connection
# tracking, rules, policy, the one-way lock, and real traffic through the
# filter. Same toolchain and technique as userland/shmtest/build.sh.
#
# Run from the repo root:
#   ./userland/fwtest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c fwtest.c -o fwtest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o fwtest.elf \
    crt0.o fwtest_main.o syscall.o string.o stdio.o stdlib.o

cp fwtest.elf ../../tools/fixtures/FWTEST.ELF
rm -f *.o fwtest.elf
echo "Built tools/fixtures/FWTEST.ELF"
