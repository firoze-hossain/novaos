#!/bin/sh
# Builds userland/ui-rs (NovaOS Unified Design System and Widget Library)
# against NovaOS's custom bare-metal Rust sysroot (tools/rust-sysroot/)
# into a reusable library (libui.rlib).
set -e
cd "$(dirname "$0")"

RUST_SYSROOT=../../tools/rust-sysroot
LIBRUST="$RUST_SYSROOT/sysroot/lib/rustlib/i686-novaos/lib"
if [ ! -f "$LIBRUST/libcore.rlib" ]; then
    echo "Rust sysroot not built yet - building it now..."
    "$RUST_SYSROOT/build-sysroot.sh"
fi

RUSTC_CMD="${RUSTC:-rustc}"
BOOTSTRAP_ENV="RUSTC_BOOTSTRAP=1"

echo "Compiling userland/ui-rs (NovaOS Design System & Widget Framework)..."
env $BOOTSTRAP_ENV $RUSTC_CMD --edition 2021 -Z unstable-options \
    --target "$RUST_SYSROOT/i686-novaos.json" \
    --crate-type rlib -C panic=abort -C opt-level=2 \
    -L "$LIBRUST" \
    --extern core="$LIBRUST/libcore.rlib" \
    --extern compiler_builtins="$LIBRUST/libcompiler_builtins.rlib" \
    -o libui.rlib \
    src/lib.rs

echo "✅ Built userland/ui-rs/libui.rlib"
