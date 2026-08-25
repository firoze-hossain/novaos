#!/bin/sh
# Builds userland/examples/hello.c against the minimal NovaOS libc
# (userland/libc/) into a real ELF32 executable. Requires nasm, gcc
# (with -m32/multilib support), and GNU ld - the same toolchain
# already required to build NovaOS itself.
#
# Run from the repo root:
#   ./userland/examples/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c hello.c -o hello_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o hello_c.elf \
    crt0.o hello_main.o syscall.o string.o stdio.o stdlib.o

cp hello_c.elf ../../tools/fixtures/HELLOC.ELF
echo "Built tools/fixtures/HELLOC.ELF"
