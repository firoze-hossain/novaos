#!/bin/sh
# Builds userland/klogtest/klogtest.c against the NovaOS libc into
# tools/fixtures/KLOGTEST.ELF - the in-OS conformance test for the structured
# kernel logging subsystem (SYS_KLOG_READ / SYS_KLOG_WRITE / SYS_KLOG_INFO,
# kernel/rust/klog.rs).
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c klogtest.c -o klogtest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o klogtest.elf \
    crt0.o klogtest_main.o syscall.o string.o stdio.o stdlib.o

cp klogtest.elf ../../tools/fixtures/KLOGTEST.ELF
rm -f *.o klogtest.elf
echo "Built tools/fixtures/KLOGTEST.ELF"
