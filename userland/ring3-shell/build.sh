#!/bin/sh
# Builds userland/ring3-shell/shell.c against the minimal NovaOS libc
# into a real ELF32 executable - the genuine ring-3 interactive shell
# (Phase 30) NovaOS boots into via process_exec_as_shell(), replacing
# the ring-0 shell kernel task. Same toolchain and technique as
# userland/examples/build.sh and userland/coreutils/build.sh.
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c shell.c -o shell_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
    -o shell.elf crt0.o shell_main.o syscall.o string.o stdio.o stdlib.o

cp shell.elf ../../tools/fixtures/SHELL.ELF
echo "Built tools/fixtures/SHELL.ELF"
