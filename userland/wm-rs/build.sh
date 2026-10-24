#!/bin/sh
# Builds userland/wm-rs/wm.rs (+ ffi.rs) against NovaOS's custom bare-
# metal Rust sysroot (tools/rust-sysroot/) into a real ELF32
# executable - the same "reuse the existing, unmodified C crt0.asm/
# syscall.c for process entry and the actual `int 0x80` syscall
# mechanics; only this program's own logic is Rust" shape userland/
# ping-rs/build.sh (Phase 33) and userland/coreutils-rs/build.sh
# (Phase 59) already established.
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

echo "Compiling wm.rs (Phase 70 - a real, interactive ring-3 window manager)..."
# Use active rustc (stable 1.99.1 by default) with RUSTC_BOOTSTRAP=1 to match
# the sysroot built by build-sysroot.sh.
RUSTC_CMD="${RUSTC:-rustc}"
BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"

UI_DIR="../ui-rs"
if [ ! -f "$UI_DIR/libui.rlib" ]; then
    echo "Building userland/ui-rs (NovaOS Design System)..."
    "$UI_DIR/build.sh"
fi

env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
    --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type bin -C panic=abort -C opt-level=2 \
    --emit obj=wm_main.o \
    -L "$LIBRUST" \
    -L "$UI_DIR" \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    --extern ui="$UI_DIR/libui.rlib" \
    -C link-dead-code=no \
    wm.rs

ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
    -o wm.elf crt0.o wm_main.o syscall.o string.o stdlib.o \
    "$UI_DIR/libui.rlib" \
    "$LIBRUST/libcore.rlib" "$LIBRUST/libcompiler_builtins.rlib"

cp wm.elf ../../tools/fixtures/WM.ELF
echo "Built tools/fixtures/WM.ELF (Rust)"
