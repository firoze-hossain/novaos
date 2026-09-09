#!/bin/sh
# FALLBACK sysroot builder, used only when `rustup` isn't available at
# all (see build-sysroot.sh, which tries a real nightly toolchain via
# `cargo -Z build-std` first and only calls into this script if that
# isn't possible). This project's own sandboxed testing environment is
# exactly that case: a plain apt-installed stable rustc, no rustup, no
# network access to rustup's download servers.
#
# Compiles core's and compiler_builtins' own source directly with
# `RUSTC_BOOTSTRAP=1`, a long-standing (if unofficial) escape hatch
# that lets a stable rustc accept the same `#![feature(...)]` gates
# core's source itself uses internally - originally meant for Cargo's
# own bootstrapping, not a NovaOS-specific trick.
#
# -Z unstable-options is required alongside RUSTC_BOOTSTRAP=1 on at
# least some rustc versions - custom target JSON files are themselves
# gated as an unstable feature, separately from the #![feature(...)]
# gates inside core/compiler_builtins' own source that
# RUSTC_BOOTSTRAP=1 alone unlocks.
#
# "features": no +soft-float - a real rustc rejected it outright
# ("target feature soft-float is incompatible with the ABI"), a
# stricter validation older rustc versions don't perform. Confirmed
# safe to drop: nothing in this project's own .rs files anywhere uses
# f32/f64. -mmx,-sse is kept - still worth disabling so the compiler
# has no reason to touch XMM/MMX registers for anything else.
#
# Edition for core specifically: a real rustc (1.97.0) ships a bundled
# core source that uses "let chains" and relies on Rust 2024's changed
# impl-Trait lifetime-capture defaults - both genuinely require
# --edition 2024 to compile. This environment's own rustc doesn't
# recognize 2024 as a valid edition at all, so which edition is
# correct can't be hardcoded - detected by trying 2021 first, retrying
# with 2024 only on the specific "let chains" failure.
#
# compiler_builtins version and known limitation: this script pins a
# specific crates.io release (see CB_VERSION below) and, where needed,
# patches missing feature-gate declarations into its fetched source.
# This has repeatedly proven fragile against a rustc meaningfully
# newer than this environment's own 1.75.0 - real, different
# incompatibilities (edition-gated lints, internal-intrinsic API
# gaps, language features assumed-stable-but-not-declared) have shown
# up in successive rounds of actually testing this against real,
# newer machines, not hypothetically. This is exactly why
# build-sysroot.sh tries the nightly + -Z build-std path first
# whenever rustup is available: that path doesn't have this problem at
# all, since core and compiler_builtins are compiled from the exact
# source bundled with whatever nightly is doing the compiling, always
# mutually version-matched. Treat this script as a fallback of last
# resort, not a fully-solved path - if it still fails on a given
# rustc, installing a nightly toolchain via rustup and letting
# build-sysroot.sh use that instead is the recommended fix, not
# another patch to this file.
set -e
cd "$(dirname "$0")"

TARGET=i686-novaos
TARGET_JSON="$TARGET.json"
SYSROOT_LIB="sysroot/lib/rustlib/$TARGET/lib"
mkdir -p "$SYSROOT_LIB"

CANDIDATE_WIDTH_FIELDS="target-pointer-width target-c-int-width"

field_is_flipped() {
    field="$1"
    for f in $FLIPPED_FIELDS; do
        [ "$f" = "$field" ] && return 0
    done
    return 1
}

json_value_for() {
    field="$1"
    default_value="$2"
    if field_is_flipped "$field"; then
        echo "$default_value"
    else
        echo "\"$default_value\""
    fi
}

write_target_json() {
    pw=$(json_value_for target-pointer-width 32)
    ciw=$(json_value_for target-c-int-width 32)
    cat > "$TARGET_JSON" << EOF
{
  "llvm-target": "i686-unknown-none",
  "data-layout": "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128",
  "arch": "x86",
  "target-endian": "little",
  "target-pointer-width": $pw,
  "target-c-int-width": $ciw,
  "os": "none",
  "linker-flavor": "ld.lld",
  "linker": "rust-lld",
  "panic-strategy": "abort",
  "disable-redzone": true,
  "features": "-mmx,-sse",
  "max-atomic-width": 32
}
EOF
}

run_probe() {
    probe_src="rust_target_probe.rs"
    echo 'pub fn x() {}' > "$probe_src"
    RUSTC_BOOTSTRAP=1 rustc -Z unstable-options --edition 2021 \
        --target "$TARGET_JSON" --crate-type lib --emit metadata \
        -o /dev/null "$probe_src" 2>&1 || true
    rm -f "$probe_src"
}

echo "Detecting this rustc's expected target JSON field types..."
FLIPPED_FIELDS=""
attempt=0
max_attempts=$(($(echo "$CANDIDATE_WIDTH_FIELDS" | wc -w) + 1))
while [ "$attempt" -lt "$max_attempts" ]; do
    write_target_json
    probe_output=$(run_probe)

    if ! echo "$probe_output" | grep -q "error loading target specification"; then
        echo "  -> accepted (flipped to number: ${FLIPPED_FIELDS:-none})"
        break
    fi

    bad_field=$(echo "$probe_output" | sed -n \
        's/.*error loading target specification: \([a-zA-Z_-]*\): invalid type: string.*/\1/p')

    if [ -z "$bad_field" ]; then
        echo "ERROR: rustc rejected the target JSON for a reason this" >&2
        echo "script doesn't know how to auto-correct. Full rustc output:" >&2
        echo "$probe_output" >&2
        exit 1
    fi

    already_flipped=false
    field_is_flipped "$bad_field" && already_flipped=true
    if [ "$already_flipped" = true ]; then
        echo "ERROR: rustc still rejects '$bad_field' after already" >&2
        echo "flipping it once. Full rustc output:" >&2
        echo "$probe_output" >&2
        exit 1
    fi

    case " $CANDIDATE_WIDTH_FIELDS " in
        *" $bad_field "*)
            echo "  '$bad_field' needs a JSON number, not a string - retrying..."
            FLIPPED_FIELDS="$FLIPPED_FIELDS $bad_field"
            ;;
        *)
            echo "ERROR: rustc says '$bad_field' needs a different JSON" >&2
            echo "type, but that field isn't in this script's known" >&2
            echo "candidate list ($CANDIDATE_WIDTH_FIELDS). Full rustc" >&2
            echo "output:" >&2
            echo "$probe_output" >&2
            exit 1
            ;;
    esac
    attempt=$((attempt + 1))
done

if [ "$attempt" -ge "$max_attempts" ]; then
    echo "ERROR: exhausted $max_attempts detection attempts. Last output:" >&2
    echo "$probe_output" >&2
    exit 1
fi

CORE_SRC=$(rustc --print sysroot)/lib/rustlib/src/rust/library/core/src/lib.rs
if [ ! -f "$CORE_SRC" ]; then
    CORE_SRC=$(find /usr/src -maxdepth 4 -path "*/library/core/src/lib.rs" 2>/dev/null | head -1)
fi
if [ ! -f "$CORE_SRC" ]; then
    echo "ERROR: core's source not found - install the 'rust-src' package" >&2
    exit 1
fi

build_rlib_with_edition_fallback() {
    crate_name="$1"
    src_file="$2"
    out_file="$3"
    shift 3
    extra_args="$@"

    edition=2021
    build_output=$(RUSTC_BOOTSTRAP=1 rustc --edition "$edition" -Z unstable-options \
        --target "$TARGET_JSON" --crate-name "$crate_name" \
        -C panic=abort -C opt-level=2 $extra_args \
        "$src_file" -o "$out_file" 2>&1) && { BUILD_EDITION=$edition; return 0; }

    if echo "$build_output" | grep -q "let chains are only allowed in Rust 2024"; then
        echo "  $crate_name's source needs edition 2024 - retrying..."
        edition=2024
        build_output=$(RUSTC_BOOTSTRAP=1 rustc --edition "$edition" -Z unstable-options \
            --target "$TARGET_JSON" --crate-name "$crate_name" \
            -C panic=abort -C opt-level=2 $extra_args \
            "$src_file" -o "$out_file" 2>&1) && { BUILD_EDITION=$edition; return 0; }
    fi

    echo "ERROR: failed to build $crate_name (tried edition 2021$(
        [ "$edition" = 2024 ] && echo ', then 2024')). Full rustc output:" >&2
    echo "$build_output" >&2
    exit 1
}

echo "Building libcore.rlib for $TARGET..."
build_rlib_with_edition_fallback core "$CORE_SRC" "$SYSROOT_LIB/libcore.rlib" \
    --crate-type lib
CORE_EDITION=$BUILD_EDITION

CB_VERSION=0.1.152
CB_SRC_DIR="$(pwd)/compiler_builtins-$CB_VERSION-src/compiler_builtins-$CB_VERSION"
if [ ! -d "$CB_SRC_DIR" ]; then
    echo "Fetching compiler_builtins $CB_VERSION from crates.io..."
    mkdir -p "$(dirname "$CB_SRC_DIR")"
    curl -sL "https://static.crates.io/crates/compiler_builtins/compiler_builtins-$CB_VERSION.crate" \
        -o /tmp/compiler_builtins-$CB_VERSION.crate
    tar -xzf /tmp/compiler_builtins-$CB_VERSION.crate -C "$(dirname "$CB_SRC_DIR")"
    rm -f /tmp/compiler_builtins-$CB_VERSION.crate
fi
if [ ! -f "$CB_SRC_DIR/src/lib.rs" ]; then
    echo "ERROR: could not fetch compiler_builtins from crates.io" >&2
    exit 1
fi

if ! grep -q "feature(inline_const)" "$CB_SRC_DIR/src/lib.rs"; then
    sed -i '1i #![feature(inline_const)]\n#![feature(associated_type_bounds)]' \
        "$CB_SRC_DIR/src/lib.rs"
fi

echo "Building libcompiler_builtins.rlib for $TARGET..."
build_rlib_with_edition_fallback compiler_builtins "$CB_SRC_DIR/src/lib.rs" \
    "$SYSROOT_LIB/libcompiler_builtins.rlib" \
    --crate-type rlib \
    --extern core="$SYSROOT_LIB/libcore.rlib" \
    --cfg 'feature="mem"' --cfg 'feature="compiler-builtins"'

echo "Sysroot ready: $SYSROOT_LIB (core edition $CORE_EDITION, compiler_builtins edition $BUILD_EDITION)"
echo ""
echo "NOTE: this was built via the RUSTC_BOOTSTRAP fallback path, not"
echo "nightly + -Z build-std. If this rustc changes again in a way that"
echo "breaks this path, installing a nightly toolchain via rustup"
echo "(rustup toolchain install nightly) is the recommended fix, not"
echo "another patch to this script."
