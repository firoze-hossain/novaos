#!/bin/sh
# Builds userland/coreutils-rs/{ls,echo,cp,rm}.rs (+ shared ffi.rs)
# against NovaOS's custom bare-metal Rust sysroot (tools/rust-sysroot/)
# into real ELF32 executables - Phase 59's coreutils, the same "reuse
# the existing, unmodified C crt0.asm/syscall.c for process entry and
# the actual `int 0x80` syscall mechanics; only this program's own
# logic is Rust" shape userland/ping-rs/build.sh already established
# (Phase 33), looped over four programs instead of one, the same way
# userland/coreutils/build.sh already loops its own C programs.
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

# Same rustc-flavor detection as ping-rs/build.sh - see that script's
# own comment for why this matters (must match whichever toolchain
# build-sysroot.sh actually used for the rlibs being linked against).
if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    RUSTC_CMD="rustc +nightly"
    BOOTSTRAP_ENV=""
else
    RUSTC_CMD="rustc"
    BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"
fi

for prog in ls echo cp rm; do
    echo "Compiling $prog.rs (Phase 59 - a real ring-3 coreutil, in Rust)..."
    env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
        --target "$RUST_SYSROOT/i686-novaos.json" \
        --crate-type bin -C panic=abort -C opt-level=2 \
        --emit obj="${prog}_main.o" \
        --extern core="$LIBRUST/libcore.rlib" \
        --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
        -C link-dead-code=no \
        "$prog.rs"

    ld -m elf_i386 -Ttext=0x08048000 --entry=_start -static \
        -o "$prog.elf" crt0.o "${prog}_main.o" syscall.o stdlib.o \
        "$LIBRUST/libcore.rlib" "$LIBRUST/libcompiler_builtins.rlib"

    cp "$prog.elf" "../../tools/fixtures/$(echo $prog | tr a-z A-Z).ELF"
    echo "Built tools/fixtures/$(echo $prog | tr a-z A-Z).ELF"
done
