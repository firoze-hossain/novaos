#!/bin/sh
# Builds userland/ping-rs/main.rs (+ ffi.rs) against NovaOS's custom
# bare-metal Rust sysroot (tools/rust-sysroot/) into a real ELF32
# executable - Phase 33's first genuine Rust userland program. Reuses
# the existing, unmodified C crt0.asm/syscall.c (built exactly like
# every other userland program's - see userland/coreutils/build.sh)
# for process entry and the actual `int 0x80` syscall mechanics; only
# ping's own program logic is Rust.
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

echo "Compiling main.rs (Phase 33 - NovaOS's first Rust userland program)..."
# Detected at execution time, not hardcoded - must match whichever
# toolchain build-sysroot.sh actually used for the rlibs this links
# against (nightly + -Z build-std if rustup/nightly are available,
# RUSTC_BOOTSTRAP=1 on stable otherwise - see that script's own
# comments). Linking rlibs built by one rustc against object code
# compiled by a different one risks a metadata version mismatch, the
# same way any two mismatched rustc versions would.
if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    RUSTC_CMD="rustc +nightly"
    BOOTSTRAP_ENV=""
else
    RUSTC_CMD="rustc"
    BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"
fi
env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type bin -C panic=abort -C opt-level=2 \
    --emit obj=ping_main.o \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    -C link-dead-code=no \
    main.rs

# core.rlib/compiler_builtins.rlib are ar archives ld can pull object
# files from directly, same as any other .a - the earlier --extern
# flags only told rustc these symbols exist for type-checking; the
# actual implementations still need to be linked in here.
ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
    -o ping.elf crt0.o ping_main.o syscall.o stdlib.o \
    "$LIBRUST/libcore.rlib" "$LIBRUST/libcompiler_builtins.rlib"

cp ping.elf ../../tools/fixtures/PING.ELF
echo "Built tools/fixtures/PING.ELF (Rust)"
