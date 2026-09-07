#!/bin/sh
# Builds a minimal Rust sysroot (libcore.rlib + libcompiler_builtins.rlib)
# for NovaOS's custom bare-metal i686 target (i686-novaos.json), usable
# by both kernel and userland Rust code.
#
# Why this exists instead of `rustup target add` + `cargo build
# -Z build-std`: this environment has no rustup (its usual download
# domain isn't network-reachable here) and only a stable rustc from
# Ubuntu's own package archive - no nightly toolchain. `-Z build-std`
# itself requires nightly regardless of network access. The
# workaround: compile core's and compiler_builtins' own source
# directly with `RUSTC_BOOTSTRAP=1`, a long-standing (if unofficial)
# escape hatch that lets a stable rustc accept the same
# `#![feature(...)]` gates core's source itself uses internally -
# originally meant for Cargo's own bootstrapping, not a NovaOS-specific
# trick. If a real nightly toolchain (or `rustup target add` support)
# is available instead, prefer that; this script is what makes Rust
# work at all without it. Verified end-to-end: a trivial Rust function
# compiled this way, linked with a plain nasm _start stub, executes
# and returns the exact expected result when run directly.
#
# --crate-name core (not inferred from the lib.rs filename) matters:
# without it, rustc's automatic prelude injection for no_std crates
# doesn't recognize the compiled rlib as "the" core crate, and
# anything built against it (including compiler_builtins) fails with
# confusing "cannot find Some/None in this scope" errors despite
# otherwise-correct code.
set -e
cd "$(dirname "$0")"

TARGET=i686-novaos
SYSROOT_LIB="sysroot/lib/rustlib/$TARGET/lib"
mkdir -p "$SYSROOT_LIB"

CORE_SRC=$(rustc --print sysroot)/lib/rustlib/src/rust/library/core/src/lib.rs
if [ ! -f "$CORE_SRC" ]; then
    # Debian/Ubuntu's rust-src package location differs from the
    # upstream rustup layout.
    CORE_SRC=$(find /usr/src -maxdepth 4 -path "*/library/core/src/lib.rs" 2>/dev/null | head -1)
fi
if [ ! -f "$CORE_SRC" ]; then
    echo "ERROR: core's source not found - install the 'rust-src' package" >&2
    exit 1
fi

echo "Building libcore.rlib for $TARGET..."
RUSTC_BOOTSTRAP=1 rustc --edition 2021 --target i686-novaos.json \
    --crate-type lib --crate-name core -C panic=abort -C opt-level=2 \
    "$CORE_SRC" -o "$SYSROOT_LIB/libcore.rlib"

CB_VERSION=0.1.101
CB_SRC_DIR=$(find "$HOME/.cargo/registry/src" -maxdepth 2 \
    -iname "compiler_builtins-$CB_VERSION" 2>/dev/null | head -1)
if [ -z "$CB_SRC_DIR" ]; then
    echo "Fetching compiler_builtins $CB_VERSION from crates.io..."
    TMP_FETCH=$(mktemp -d)
    (cd "$TMP_FETCH" && cargo init --lib --name cb_fetch --vcs none >/dev/null 2>&1
     cat > Cargo.toml << EOF
[package]
name = "cb_fetch"
version = "0.1.0"
edition = "2021"
[dependencies]
compiler_builtins = { version = "=$CB_VERSION", features = ["mem"] }
EOF
     cargo fetch >/dev/null 2>&1)
    rm -rf "$TMP_FETCH"
    CB_SRC_DIR=$(find "$HOME/.cargo/registry/src" -maxdepth 2 \
        -iname "compiler_builtins-$CB_VERSION" 2>/dev/null | head -1)
fi
if [ -z "$CB_SRC_DIR" ]; then
    echo "ERROR: could not fetch compiler_builtins from crates.io" >&2
    exit 1
fi

echo "Building libcompiler_builtins.rlib for $TARGET..."
RUSTC_BOOTSTRAP=1 rustc --edition 2021 --target i686-novaos.json \
    --crate-type rlib --crate-name compiler_builtins -C panic=abort \
    -C opt-level=2 \
    --extern core="$SYSROOT_LIB/libcore.rlib" \
    --cfg 'feature="mem"' --cfg 'feature="compiler-builtins"' \
    "$CB_SRC_DIR/src/lib.rs" -o "$SYSROOT_LIB/libcompiler_builtins.rlib"

echo "Sysroot ready: $SYSROOT_LIB"
