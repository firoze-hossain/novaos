#!/bin/sh
# Builds userland/coreutils/*.c against the minimal NovaOS libc into
# real ELF32 executables - genuine ring-3 userland programs, the first
# concrete proof (Phase 29) that this project's kernel/userland split
# supports real Linux-kernel-vs-Ubuntu-userland-style separation, not
# just reorganized source directories. Same toolchain and technique as
# userland/examples/build.sh.
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o

for prog in cat gui; do
    $CC -c "$prog.c" -o "${prog}_main.o"
    ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
        -o "$prog.elf" crt0.o "${prog}_main.o" syscall.o string.o stdio.o stdlib.o
    cp "$prog.elf" "../../tools/fixtures/$(echo $prog | tr a-z A-Z).ELF"
    echo "Built tools/fixtures/$(echo $prog | tr a-z A-Z).ELF"
done
