#!/bin/sh
# Builds userland/net-rs/{nslookup,tftp}.rs (+ shared ffi.rs) against
# NovaOS's custom bare-metal Rust sysroot (tools/rust-sysroot/) into
# real ELF32 executables - Phase 60's nslookup/tftp, the exact same
# "reuse the existing, unmodified C crt0.asm/syscall.c for process
# entry and the actual `int 0x80` syscall mechanics; only this
# program's own logic is Rust" shape userland/ping-rs/build.sh
# established (Phase 33) and userland/coreutils-rs/build.sh already
# repeats (Phase 59), looped over two programs in their own directory
# rather than added to either of those (ping-rs is one program;
# coreutils-rs is specifically Linux/BSD coreutils - nslookup/tftp are
# network utilities, the same category as ping, so their own directory
# keeps that grouping honest rather than overloading either existing
# one).
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

# Same rustc-flavor detection as ping-rs/build.sh and coreutils-rs/
# build.sh - see either script's own comment for why this matters
# (must match whichever toolchain build-sysroot.sh actually used for
# the rlibs being linked against).
if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    RUSTC_CMD="rustc +nightly"
    BOOTSTRAP_ENV=""
else
    RUSTC_CMD="rustc"
    BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"
fi

# prog -> fixture name mapping, since "nslookup" -> "NSLOOKUP" (8
# characters) is already at FAT32 8.3's exact 8-character name limit -
# no truncation needed here, but spelled out explicitly rather than
# relying on coreutils-rs/build.sh's shorter `tr a-z A-Z` one-liner,
# so a future program in this directory with a longer name fails loud
# at the mapping instead of silently truncating.
for prog in nslookup tftp; do
    case "$prog" in
        nslookup) fixture=NSLOOKUP.ELF ;;
        tftp)     fixture=TFTP.ELF ;;
    esac

    echo "Compiling $prog.rs (Phase 60 - a real ring-3 network utility, in Rust)..."
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

    cp "$prog.elf" "../../tools/fixtures/$fixture"
    echo "Built tools/fixtures/$fixture"
done
