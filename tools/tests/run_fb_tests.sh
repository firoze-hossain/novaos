#!/bin/sh
# Phase 81: builds and runs the host-side framebuffer tests:
#   * tools/tests/fb_geom_test.c - the kernel's rectangle/size
#     arithmetic (kernel/drivers/video/fb_geom.h), against pixel-set
#     and wide-integer oracles;
#   * tools/tests/novagfx_test.c - the userland drawing helpers
#     (userland/libc/include/novagfx.h), against a per-pixel oracle,
#     with guard words to catch any out-of-bounds store.
# Both compile the REAL headers, not copies. Also checks fb_geom.h stays
# free of libgcc 64-bit arithmetic helpers (the kernel does not link
# libgcc).
#
#   ./tools/tests/run_fb_tests.sh      (or: make fb-test)
set -e
cd "$(dirname "$0")"
OUT="${TMPDIR:-/tmp}/novaos-fb-geom.$$"
mkdir -p "$OUT"
trap 'rm -rf "$OUT"' EXIT

echo "--- fb_geom host test (native, -O2) ---"
gcc -O2 -Wall -Wextra -o "$OUT/t64" fb_geom_test.c
"$OUT/t64"

echo "--- fb_geom host test (native, -O0, UB sanitizers) ---"
gcc -O0 -g -Wall -Wextra -fsanitize=undefined -fno-sanitize-recover=undefined \
    -o "$OUT/t64s" fb_geom_test.c
"$OUT/t64s"

# -idirafter (not -I): the libc headers share names with the host's
# (errno.h, stdio.h, ...), and the host's must win for everything except
# the NovaOS-only ones (novasys.h, nova_fb_abi.h) this test needs.
echo "--- novagfx host test (native, -O2) ---"
gcc -O2 -Wall -Wextra -idirafter ../../userland/libc/include \
    -o "$OUT/g64" novagfx_test.c
"$OUT/g64"

echo "--- novagfx host test (native, -O0, UB + address sanitizers) ---"
gcc -O0 -g -Wall -Wextra -idirafter ../../userland/libc/include \
    -fsanitize=undefined,address -fno-sanitize-recover=undefined \
    -o "$OUT/g64s" novagfx_test.c
"$OUT/g64s"

# The same code compiled the way the kernel compiles it (32-bit,
# freestanding, -O2): every function inlined into a probe, then check
# the object for undefined libgcc helpers. A __muldi3/__divdi3 here
# would be a LINK error in the kernel, caught now instead of there.
cat > "$OUT/probe.c" <<'PROBE'
#include "../../kernel/drivers/video/fb_geom.h"
int probe(uint32_t a, uint32_t b, int32_t c, int32_t d, int32_t e, int32_t f,
          int32_t g, int32_t h, uint32_t stride) {
    fb_blit_t blit;
    uint32_t span = 0;
    int r = fb_clip_blit(a, b, a, b, c, d, e, f, g, h, &blit);
    r += fb_rect_in_bounds(a, b, c, d, e, f);
    r += fb_buffer_span(stride, e, f, &span);
    r += (int)fb_surface_bytes(a, b) + (int)fb_pages_for_bytes(span);
    return r + blit.w + (int)span;
}
PROBE
sed -i 's#"../../kernel#"'"$(pwd)"'/../../kernel#' "$OUT/probe.c"
# FB_GEOM_HOST selects <stdint.h> (present on any host), with -m32 and
# the freestanding flags the kernel uses.
if gcc -m32 -std=c99 -ffreestanding -O2 -fno-builtin -DFB_GEOM_HOST \
       -c "$OUT/probe.c" -o "$OUT/probe.o" 2>/dev/null; then
    if nm "$OUT/probe.o" | grep -E ' U __(mul|div|udiv|mod|umod)di3|__ashldi3|__lshrdi3|__ashrdi3'; then
        echo "FAIL: fb_geom.h pulls in libgcc 64-bit helpers the kernel does not link"
        exit 1
    fi
    echo "--- fb_geom.h (-m32 freestanding): no libgcc 64-bit helper references ---"
else
    echo "--- fb_geom.h -m32 libgcc-helper check SKIPPED (no 32-bit gcc support here) ---"
fi
