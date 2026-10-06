#!/bin/sh
# Builds userland/audiotest/audiotest.c against the NovaOS libc into
# tools/fixtures/AUDTEST.ELF - the in-OS conformance test for the SYS_AUDIO_*
# audio-mixing syscalls (see audiotest.c). Same toolchain and
# technique as userland/shmtest/build.sh and userland/gfxtest/build.sh.
#
# Run from the repo root:
#   ./userland/audiotest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c audiotest.c -o audiotest_main.o

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static -o audiotest.elf \
    crt0.o audiotest_main.o syscall.o string.o stdio.o stdlib.o

cp audiotest.elf ../../tools/fixtures/AUDTEST.ELF
rm -f *.o audiotest.elf
echo "Built tools/fixtures/AUDTEST.ELF"
