#!/bin/sh
# Builds userland/novainit-rs/novainit.rs (+ ffi.rs) against NovaOS's
# custom bare-metal Rust sysroot (tools/rust-sysroot/) into a real
# ELF32 executable - the same "reuse the existing, unmodified C
# crt0.asm/syscall.c for process entry and the actual `int 0x80`
# syscall mechanics; only this program's own logic is Rust" shape
# userland/ping-rs/build.sh (Phase 33), userland/coreutils-rs/build.sh
# (Phase 59), and userland/wm-rs/build.sh (Phase 70) already
# established.
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

echo "Compiling novainit.rs (Phase 71 - a real service supervisor)..."
# Same rustc-flavor detection as every other userland/*-rs/build.sh -
# must match whichever toolchain build-sysroot.sh actually used for
# the rlibs being linked against.
if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    RUSTC_CMD="rustc +nightly"
    BOOTSTRAP_ENV=""
else
    RUSTC_CMD="rustc"
    BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"
fi

env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
    --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type bin -C panic=abort -C opt-level=2 \
    --emit obj=novainit_main.o \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    -C link-dead-code=no \
    novainit.rs

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
    -o novainit.elf crt0.o novainit_main.o syscall.o stdlib.o \
    "$LIBRUST/libcore.rlib" "$LIBRUST/libcompiler_builtins.rlib"

cp novainit.elf ../../tools/fixtures/NOVAINIT.ELF
echo "Built tools/fixtures/NOVAINIT.ELF (Rust)"
