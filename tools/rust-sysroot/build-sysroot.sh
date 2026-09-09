#!/bin/sh
# Builds a minimal Rust sysroot (libcore.rlib + libcompiler_builtins.rlib)
# for NovaOS's custom bare-metal i686 target, usable by both kernel and
# userland Rust code.
#
# This is the PREFERRED path, used automatically whenever `rustup` is
# available: a real nightly toolchain (installed via `rustup toolchain
# install nightly` if not already present) plus Cargo's own official
# `-Z build-std=core,compiler_builtins` feature. This sidesteps every
# version-compatibility problem the alternative
# (build-sysroot-bootstrap.sh: RUSTC_BOOTSTRAP=1 on a stable compiler,
# manually fetching and patching a specific compiler_builtins release
# from crates.io) has repeatedly run into across real testing against
# real, newer machines: edition-gated lints, missing internal-intrinsic
# APIs, language features assumed-stable-but-undeclared. `-Z build-std`
# builds core/compiler_builtins from the *exact* source bundled with
# that specific nightly's own rust-src component - the same nightly's
# compiler is what's compiling them, so they're guaranteed to already
# be mutually compatible, the same way any ordinary rustc release's own
# standard library already is. No crates.io fetch, no manual
# feature-gate patching, no version pin to keep re-checking as rustc
# moves forward. Confirmed genuinely working end-to-end on a real
# machine (macOS/Apple Silicon, rustc 1.98.1 via a rustup nightly): the
# full kernel-side Rust build compiled and linked successfully.
#
# -Z json-target-spec: Cargo itself (not just rustc) gates accepting a
# custom .json file as --target behind its own separate unstable flag
# - distinct from rustc's own -Z unstable-options (used in the
# bootstrap fallback script and this script's own probe calls below),
# which only unlocks the target JSON for rustc itself.
#
# -Z embed-metadata=yes: Cargo began defaulting to -Zembed-metadata=no
# on the nightly channel (an active, ongoing experiment - see Rust's
# "Inside Rust" blog, 2026-08-18, "Experiment in reducing target
# directory size on nightly") shortly before this was hit on a real
# machine, which makes -Z build-std produce core/compiler_builtins as
# small metadata-only stub rlibs (~440 bytes, no compiled code) rather
# than full rlibs. -Z embed-metadata=yes is that post's own documented
# opt-out, passed to Cargo directly (not via RUSTFLAGS, since Cargo
# itself needs to know the flag's value).
set -e
cd "$(dirname "$0")"

TARGET=i686-novaos
TARGET_JSON="$TARGET.json"
SYSROOT_LIB="sysroot/lib/rustlib/$TARGET/lib"
mkdir -p "$SYSROOT_LIB"

if ! command -v rustup >/dev/null 2>&1; then
    echo "rustup not found - falling back to the RUSTC_BOOTSTRAP-based build..."
    exec ./build-sysroot-bootstrap.sh
fi

echo "rustup found - using a nightly toolchain with cargo -Z build-std"
echo "(this project's preferred path when available)..."

if ! rustup toolchain list 2>/dev/null | grep -q '^nightly'; then
    echo "Installing a nightly toolchain via rustup..."
    if ! rustup toolchain install nightly >/dev/null 2>&1; then
        echo "  Could not install nightly (offline?) - falling back to" >&2
        echo "  the RUSTC_BOOTSTRAP-based build..." >&2
        exec ./build-sysroot-bootstrap.sh
    fi
fi

if ! rustup component add rust-src --toolchain nightly >/dev/null 2>&1; then
    echo "  Could not add the rust-src component to nightly - falling" >&2
    echo "  back to the RUSTC_BOOTSTRAP-based build..." >&2
    exec ./build-sysroot-bootstrap.sh
fi

# Reuses this project's own field-type detection logic (the correct
# JSON type - string vs number - for target-pointer-width/
# target-c-int-width has genuinely differed between real rustc
# versions this project has been tested against) rather than
# duplicating it - the nightly toolchain's own target-spec parser
# needs exactly the same detection a stable one does, since this is
# about the target JSON's schema, not about stable-vs-nightly.
write_target_json_variant() {
    pw="$1"
    cat > "$TARGET_JSON" << EOF
{
  "llvm-target": "i686-unknown-none",
  "data-layout": "e-m:e-p:32:32-p270:32:32-p271:32:32-p272:64:64-i128:128-f64:32:64-f80:32-n8:16:32-S128",
  "arch": "x86",
  "target-endian": "little",
  "target-pointer-width": $pw,
  "target-c-int-width": $pw,
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

echo "Detecting nightly's expected target JSON field types..."
target_json_ok=false
for pw in '"32"' '32'; do
    write_target_json_variant "$pw"
    probe_src="rust_target_probe.rs"
    echo 'pub fn x() {}' > "$probe_src"
    probe_output=$(rustc +nightly -Z unstable-options --edition 2021 \
        --target "$TARGET_JSON" --crate-type lib --emit metadata \
        -o /dev/null "$probe_src" 2>&1) || true
    rm -f "$probe_src"
    if ! echo "$probe_output" | grep -q "error loading target specification"; then
        target_json_ok=true
        break
    fi
done

if [ "$target_json_ok" != true ]; then
    echo "  Could not find a working target JSON with nightly - falling" >&2
    echo "  back to the RUSTC_BOOTSTRAP-based build..." >&2
    exec ./build-sysroot-bootstrap.sh
fi

NIGHTLY_PROJECT_DIR="nightly-build-std-project"
mkdir -p "$NIGHTLY_PROJECT_DIR/src"
cat > "$NIGHTLY_PROJECT_DIR/Cargo.toml" << EOF
[package]
name = "novaos_sysroot_builder"
version = "0.1.0"
edition = "2021"
EOF
cat > "$NIGHTLY_PROJECT_DIR/src/lib.rs" << 'EOF'
#![no_std]
// Empty on purpose - this crate exists only to give cargo something to
// build, so that -Z build-std is triggered and produces
// libcore.rlib/libcompiler_builtins.rlib as a side effect. Nothing in
// this project links against this crate itself.
EOF

echo "Building core + compiler_builtins via cargo -Z build-std (this can"
echo "take a minute the first time)..."
(cd "$NIGHTLY_PROJECT_DIR" && \
    cargo +nightly build -Z build-std=core,compiler_builtins -Z json-target-spec \
    -Z embed-metadata=yes \
    --target "../$TARGET_JSON" --release 2>&1)

# Found on a real machine: the exact subdirectory cargo -Z build-std
# places these artifacts in isn't reliably target/$TARGET/release/deps/
# as might be assumed from cargo's general (non---target-json) build-
# cache documentation. Rather than assume one specific path, this
# searches the entire target/ tree recursively for the expected
# filenames - correct regardless of exactly which subdirectory cargo
# used for a JSON-file target on a given cargo version/platform.
DEPS_SEARCH_ROOT="$NIGHTLY_PROJECT_DIR/target"
CORE_RLIB_BUILT=$(find "$DEPS_SEARCH_ROOT" -name "libcore-*.rlib" 2>/dev/null | head -1)
CB_RLIB_BUILT=$(find "$DEPS_SEARCH_ROOT" -name "libcompiler_builtins-*.rlib" 2>/dev/null | head -1)

if [ -z "$CORE_RLIB_BUILT" ] || [ -z "$CB_RLIB_BUILT" ]; then
    echo "ERROR: cargo -Z build-std reported success but the expected" >&2
    echo "output rlibs weren't found anywhere under $DEPS_SEARCH_ROOT." >&2
    echo "Full directory listing, for diagnosis:" >&2
    find "$DEPS_SEARCH_ROOT" -type f -name "*.rlib" >&2 2>/dev/null
    echo "(if nothing is listed above, no .rlib files exist anywhere" >&2
    echo "under $DEPS_SEARCH_ROOT at all - the build may have silently" >&2
    echo "skipped producing them despite reporting success)" >&2
    exit 1
fi

# Validates that -Z embed-metadata=yes actually took effect, rather
# than silently trusting it and only finding out much later: a genuine
# libcore.rlib with real compiled code is several MB; the "metadata
# stub" failure mode this project has directly hit produces a file
# only a few hundred bytes in size. 100KB is a deliberately generous
# floor - nowhere near either real number - so this only ever fires on
# the specific, already-seen failure mode.
MIN_REAL_RLIB_BYTES=102400
core_size=$(wc -c < "$CORE_RLIB_BUILT" 2>/dev/null || echo 0)
cb_size=$(wc -c < "$CB_RLIB_BUILT" 2>/dev/null || echo 0)
if [ "$core_size" -lt "$MIN_REAL_RLIB_BYTES" ] || [ "$cb_size" -lt "$MIN_REAL_RLIB_BYTES" ]; then
    echo "ERROR: the built rlib(s) are suspiciously small - likely" >&2
    echo "metadata-only stubs, not full rlibs with real compiled code" >&2
    echo "(this project has seen this exact failure mode before, caused" >&2
    echo "by Cargo's -Zembed-metadata=no nightly experiment):" >&2
    echo "  $CORE_RLIB_BUILT: $core_size bytes" >&2
    echo "  $CB_RLIB_BUILT: $cb_size bytes" >&2
    echo "-Z embed-metadata=yes was already passed to this build; if" >&2
    echo "you still see this, that flag's name or behavior may have" >&2
    echo "changed again - check https://blog.rust-lang.org/inside-rust/" >&2
    echo "for the current state of this experiment." >&2
    exit 1
fi

cp "$CORE_RLIB_BUILT" "$SYSROOT_LIB/libcore.rlib"
cp "$CB_RLIB_BUILT" "$SYSROOT_LIB/libcompiler_builtins.rlib"

echo "Sysroot ready: $SYSROOT_LIB (built via nightly + -Z build-std)"
