#!/bin/sh
# Builds userland/dyntest/dyntest.c against the NovaOS libc AND
# DYNLIB.SO (userland/dynlib/) into DYNTEST.ELF - a real, DYNAMICALLY
# LINKED ELF32 executable. Unlike every other userland/*/build.sh in
# this tree, this one drops `-static` and adds DYNLIB.SO directly as
# a link input: the linker sees its ELF type is DYN, records its
# SONAME as a DT_NEEDED entry instead of copying its code in, and
# resolves dyn_fib/dyn_crc32/dyn_apply/dyn_version against its
# .dynsym - producing a PLT/GOT and a .rel.plt exactly like any real
# dynamically-linked Linux binary would have. See kernel/task/
# process.c's load_and_link_shared_libraries() and kernel/rust/
# dynlink.rs for what actually resolves that at exec time - NOT a
# userspace ld.so, since none exists on NovaOS.
#
# --no-dynamic-linker: no PT_INTERP/.interp - there's nothing on
# NovaOS to hand off to (see dynlib.c's build.sh for the same note).
# No explicit -Ttext: letting the linker choose its own default base
# for a PT_DYNAMIC-bearing ET_EXEC avoids a section-layout overlap an
# explicit -Ttext=0x08048000 causes in that specific combination with
# this linker version - and its default happens to BE 0x08048000
# anyway (confirmed with readelf), so nothing actually changes versus
# every static build in this tree.
#
# Run from the repo root:
#   ./userland/dynlib/build.sh   (DYNLIB.SO must exist first)
#   ./userland/dyntest/build.sh
set -e
cd "$(dirname "$0")"

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"
DYNLIB=../../tools/fixtures/DYNLIB.SO

if [ ! -f "$DYNLIB" ]; then
    echo "DYNLIB.SO not found - run userland/dynlib/build.sh first" >&2
    exit 1
fi

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/string.c" -o string.o
$CC -c "$LIBC/stdio.c" -o stdio.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c dyntest.c -o dyntest_main.o

ld -m elf_i386 --entry=_start --hash-style=sysv --no-dynamic-linker \
    -o dyntest.elf \
    crt0.o dyntest_main.o syscall.o string.o stdio.o stdlib.o "$DYNLIB"

cp dyntest.elf ../../tools/fixtures/DYNTEST.ELF
rm -f *.o dyntest.elf
echo "Built tools/fixtures/DYNTEST.ELF"
