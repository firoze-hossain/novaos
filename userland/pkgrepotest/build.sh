#!/bin/sh
# Builds userland/pkgrepotest/pkgrepotest.c against the NovaOS libc into
# tools/fixtures/PKGREPOTEST.ELF - the in-OS conformance test for the package
# repository server subsystem (SYS_PKGREPO_CTL, kernel/rust/pkgrepo.rs).
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c pkgrepotest.c -o pkgrepotest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o pkgrepotest.elf \
    crt0.o pkgrepotest_main.o syscall.o string.o stdio.o stdlib.o

cp pkgrepotest.elf ../../tools/fixtures/PKGRTEST.ELF
rm -f *.o pkgrepotest.elf
echo "Built tools/fixtures/PKGRTEST.ELF"
