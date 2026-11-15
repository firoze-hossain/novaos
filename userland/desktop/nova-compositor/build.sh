#!/bin/sh
# Builds userland/desktop/nova-compositor (Hardware-Accelerated Wayland-Style Display Server)
# against NovaOS's custom bare-metal Rust sysroot (tools/rust-sysroot/) into a real ELF32
# executable - reusing the C crt0.asm/syscall.c for process entry and `int 0x80` syscalls.
set -e
cd "$(dirname "$0")"

REPO_ROOT=../../..
RUST_SYSROOT="$REPO_ROOT/tools/rust-sysroot"
LIBRUST="$RUST_SYSROOT/sysroot/lib/rustlib/i686-novaos/lib"

if [ ! -f "$LIBRUST/libcore.rlib" ]; then
    echo "Rust sysroot not built yet - building it now..."
    "$RUST_SYSROOT/build-sysroot.sh"
fi

LIBC="$REPO_ROOT/userland/libc"
CC="gcc -m32 -ffreestanding -fno-stack-protector -fno-pie -Wall -Wextra -I$LIBC/include"

nasm -f elf32 "$LIBC/crt0.asm" -o crt0.o
$CC -c "$LIBC/syscall.c" -o syscall.o
$CC -c "$LIBC/stdlib.c" -o stdlib.o
$CC -c "$LIBC/string.c" -o string.o

echo "Compiling nova-compositor (Month 4 - Hardware-Accelerated Wayland Display Server)..."
RUSTC_CMD="${RUSTC:-rustc}"
BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"

# 1. Compile libnova_compositor.rlib
env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
    --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type rlib -C panic=abort -C opt-level=2 \
    -L "$LIBRUST" \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    -o libnova_compositor.rlib \
    src/lib.rs

# 2. Compile main.rs
env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
    --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type bin -C panic=abort -C opt-level=2 \
    --emit obj=compositor_main.o \
    -L "$LIBRUST" \
    -L . \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    --extern nova_compositor=libnova_compositor.rlib \
    -C link-dead-code=no \
    src/main.rs

# 3. Link bare-metal static ELF
ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
    -o compositor.elf crt0.o compositor_main.o syscall.o string.o stdlib.o \
    libnova_compositor.rlib \
    "$LIBRUST/libcore.rlib" "$LIBRUST/libcompiler_builtins.rlib"

cp compositor.elf "$REPO_ROOT/tools/fixtures/COMPOSITOR.ELF"
cp compositor.elf "$REPO_ROOT/tools/fixtures/COMPOSIT.ELF"
echo "✅ Built tools/fixtures/COMPOSITOR.ELF and COMPOSIT.ELF (Wayland Display Server)"
