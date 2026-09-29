#!/bin/sh
# Builds userland/dynlib/dynlib.c into DYNLIB.SO, a real ELF32 shared
# library (ET_DYN, PIC), and copies it to tools/fixtures/ for the disk
# image. See dynlib.c's own comment and kernel/rust/dynlink.rs for what
# NovaOS's dynamic linker actually supports.
#
# --hash-style=sysv is not a style preference: kernel/rust/dynlink.rs
# finds a library's own symbol COUNT via the classic SysV DT_HASH
# table's nchain field (see that file's own comment on why) - a
# library built with the more common modern default
# (--hash-style=gnu, or "both") would load, but every symbol lookup
# against it would silently fail, since --hash-style=gnu never emits
# DT_HASH at all. --no-dynamic-linker suppresses PT_INTERP/.interp -
# there is no userspace ld.so on NovaOS for the kernel to hand off to
# (the kernel itself is the loader; see dynlink.rs), so requesting one
# would be requesting something that will never exist.
#
# Run from the repo root:
#   ./userland/dynlib/build.sh
set -e
cd "$(dirname "$0")"

gcc -m32 -fPIC -ffreestanding -nostdlib -shared \
    -Wl,--hash-style=sysv -Wl,--no-dynamic-linker \
    -Wl,-soname,DYNLIB.SO \
    -o dynlib.so dynlib.c

cp dynlib.so ../../tools/fixtures/DYNLIB.SO
rm -f dynlib.so
echo "Built tools/fixtures/DYNLIB.SO"
