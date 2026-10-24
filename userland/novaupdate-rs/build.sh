#!/bin/sh
# Builds userland/novaupdate-rs/novaupdate.rs against NovaOS's custom bare-metal
# Rust sysroot (tools/rust-sysroot/) into a real ELF32 executable.
set -e
cd "$(dirname "$0")"

RUST_SYSROOT=../../tools/rust-sysroot
LIBRUST="$RUST_SYSROOT/sysroot/lib/rustlib/i686-novaos/lib"
if [ ! -f "$LIBRUST/libcore.rlib" ]; then
    echo "Rust sysroot not built yet - building it now..."
    "$RUST_SYSROOT/build-sysroot.sh"
fi

LIBC=../libc
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c "$LIBC/string.c" -o string.o

# Use active rustc (stable 1.99.1 by default) with RUSTC_BOOTSTRAP=1 to match
# the sysroot built by build-sysroot.sh.
RUSTC_CMD="${RUSTC:-rustc}"
BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"

echo "Compiling novaupdate.rs (NovaOS Dual-Source Autonomous Update Service in Rust)..."
env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
    --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type bin -C panic=abort -C opt-level=2 \
    --emit obj=novaupdate_main.o \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    -C link-dead-code=no \
    novaupdate.rs

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
    -o update.elf crt0.o novaupdate_main.o syscall.o string.o stdlib.o \
    "$LIBRUST/libcore.rlib" "$LIBRUST/libcompiler_builtins.rlib"

cp update.elf ../../tools/fixtures/UPDATE.ELF
echo "✅ Successfully built tools/fixtures/UPDATE.ELF (Rust)"
