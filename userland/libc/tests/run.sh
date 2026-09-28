#!/bin/sh
# Builds and runs the host-side libc tests. See libc_host_test.c for what
# they cover and why they exist next to the in-OS test.
#
# Requires gcc with -m32 support (gcc-multilib - already required to build
# NovaOS itself) and a kernel that can run 32-bit static binaries. If the
# machine cannot run 32-bit binaries this says so and exits 0 rather than
# failing: the tests are unavailable there, not broken.
#
#   ./userland/libc/tests/run.sh
set -e
cd "$(dirname "$0")"
OUT="${TMPDIR:-/tmp}/novaos-libc-tests.$$"
mkdir -p "$OUT"
trap 'rm -rf "$OUT"' EXIT

# Can this machine run a 32-bit static executable at all?
cat > "$OUT/probe.c" <<'PROBE'
void _start(void) {
    __asm__ volatile ("int $0x80" : : "a"(1), "b"(0));
}
PROBE
if ! gcc -m32 -ffreestanding -nostdlib -static -fno-pie -no-pie \
        -o "$OUT/probe" "$OUT/probe.c" 2>/dev/null || ! "$OUT/probe" 2>/dev/null; then
    echo "libc host tests: SKIPPED - this machine cannot build/run 32-bit static binaries"
    exit 0
fi

# Step 1: the printf expectation table must agree with a real C library.
gcc -w -o "$OUT/printf_oracle" printf_oracle.c
"$OUT/printf_oracle"

# Step 2: the libc itself, freestanding, against the fake kernel - built
# exactly like the real userland (-O0) and again optimised (-O2), since
# code that passes only unoptimised is usually relying on luck.
for opt in -O0 -O2; do
    echo "--- libc host tests ($opt) ---"
    gcc -m32 $opt -ffreestanding -fno-stack-protector -fno-pie -no-pie \
        -nostdlib -nostdinc -isystem "$(gcc -print-file-name=include)" \
        -static -Wall -Wextra -Wno-unused-function -I../include \
        -o "$OUT/libc_host_test" libc_host_test.c
    "$OUT/libc_host_test"
done
