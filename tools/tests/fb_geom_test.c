/*
 * fb_geom_test.c - Phase 81: host-side tests for kernel/drivers/video/
 * fb_geom.h, the framebuffer API's rectangle/size arithmetic. Compiles
 * the REAL header (FB_GEOM_HOST selects <stdint.h> instead of the
 * kernel's types.h) - not a copy - and checks it against references
 * that share no code with it:
 *
 *  1. A brute-force, pixel-set oracle for small rectangles: enumerate
 *     every source pixel, map it to the display, keep the ones that
 *     land on screen, and derive the expected clipped rectangle from
 *     that set. No clipping formulas anywhere - if fb_clip_blit() and
 *     this agree on hundreds of thousands of random cases, a bug would
 *     have to be the same bug twice in two unrelated algorithms.
 *  2. A wide-integer (__int128) reference for the extremes, where
 *     brute force is impossible and the whole point is that int32
 *     arithmetic would wrap: INT32_MIN/INT32_MAX coordinates and
 *     sizes, strides near 2^32.
 *  3. Hand-written edge cases with known answers.
 *
 * Exit status 0 = every check passed.
 */
#define FB_GEOM_HOST 1
#include "../../kernel/drivers/video/fb_geom.h"

#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static long checks, failures;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 20) { \
            printf("FAIL %s:%d: %s\n   ", __FILE__, __LINE__, #cond); \
            printf(__VA_ARGS__); printf("\n"); \
        } \
    } \
} while (0)

/* xorshift32: deterministic, so a failure reproduces. */
static uint32_t rng_state = 0x1234567u;
static uint32_t rnd(void) {
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 17;
    rng_state ^= rng_state << 5;
    return rng_state;
}
static int rnd_range(int lo, int hi) { /* inclusive */
    return lo + (int)(rnd() % (uint32_t)(hi - lo + 1));
}

/* ---- oracle 1: pixel-set -------------------------------------- */
static void check_clip_against_pixel_oracle(uint32_t sw, uint32_t sh,
                                            uint32_t dw, uint32_t dh,
                                            int sx, int sy, int w, int h,
                                            int dx, int dy) {
    fb_blit_t got;
    memset(&got, 0xAA, sizeof got);
    int rc = fb_clip_blit(sw, sh, dw, dh, sx, sy, w, h, dx, dy, &got);

    /* INVALID is defined by the source rectangle alone. */
    int expect_invalid = (w <= 0 || h <= 0 || sx < 0 || sy < 0 ||
                          sx + w > (int)sw || sy + h > (int)sh);
    if (expect_invalid) {
        CHECK(rc == FB_CLIP_INVALID,
              "s=%ux%u d=%ux%u src=(%d,%d %dx%d) dst=(%d,%d) rc=%d",
              sw, sh, dw, dh, sx, sy, w, h, dx, dy, rc);
        return;
    }
    CHECK(rc != FB_CLIP_INVALID, "valid source rejected: rc=%d", rc);

    /* Enumerate source pixels that land on the display. */
    int min_x = INT_MAX, min_y = INT_MAX, max_x = INT_MIN, max_y = INT_MIN;
    long visible = 0;
    for (int y = 0; y < h; y++) {
        for (int x = 0; x < w; x++) {
            int px = dx + x, py = dy + y;
            if (px >= 0 && py >= 0 && px < (int)dw && py < (int)dh) {
                visible++;
                if (px < min_x) min_x = px;
                if (py < min_y) min_y = py;
                if (px > max_x) max_x = px;
                if (py > max_y) max_y = py;
            }
        }
    }
    if (visible == 0) {
        CHECK(rc == FB_CLIP_EMPTY,
              "expected EMPTY: src=(%d,%d %dx%d) dst=(%d,%d) d=%ux%u rc=%d",
              sx, sy, w, h, dx, dy, dw, dh, rc);
        return;
    }
    CHECK(rc == FB_CLIP_OK, "expected OK, rc=%d", rc);
    if (rc != FB_CLIP_OK) return;

    CHECK(got.dst_x == min_x && got.dst_y == min_y &&
          got.w == max_x - min_x + 1 && got.h == max_y - min_y + 1,
          "dst rect (%d,%d %dx%d) != oracle (%d,%d %dx%d)",
          got.dst_x, got.dst_y, got.w, got.h,
          min_x, min_y, max_x - min_x + 1, max_y - min_y + 1);
    /* The picture must not shift: source offset == destination offset. */
    CHECK(got.src_x - sx == got.dst_x - dx && got.src_y - sy == got.dst_y - dy,
          "picture shifted: src off (%d,%d) dst off (%d,%d)",
          got.src_x - sx, got.src_y - sy, got.dst_x - dx, got.dst_y - dy);
    CHECK(got.w == (int)visible / (got.h ? got.h : 1) || visible == (long)got.w * got.h,
          "visible pixel count %ld != %d*%d", visible, got.w, got.h);
    /* And the clipped read stays inside the source rectangle. */
    CHECK(got.src_x >= sx && got.src_y >= sy &&
          got.src_x + got.w <= sx + w && got.src_y + got.h <= sy + h,
          "clipped source escapes the requested source rect");
}

/* ---- oracle 2: wide integers ---------------------------------- */
#ifdef __SIZEOF_INT128__
typedef __int128 wide_t;
static void check_clip_wide(uint32_t sw, uint32_t sh, uint32_t dw, uint32_t dh,
                            int32_t sx, int32_t sy, int32_t w, int32_t h,
                            int32_t dx, int32_t dy) {
    fb_blit_t got;
    int rc = fb_clip_blit(sw, sh, dw, dh, sx, sy, w, h, dx, dy, &got);
    wide_t W = w, H = h, SX = sx, SY = sy, DX = dx, DY = dy;
    int invalid = (W <= 0 || H <= 0 || SX < 0 || SY < 0 ||
                   SX + W > (wide_t)sw || SY + H > (wide_t)sh);
    if (invalid) {
        CHECK(rc == FB_CLIP_INVALID, "wide: expected INVALID rc=%d "
              "(sx=%d sy=%d w=%d h=%d)", rc, sx, sy, w, h);
        return;
    }
    wide_t cx0 = DX < 0 ? 0 : DX, cy0 = DY < 0 ? 0 : DY;
    wide_t cx1 = DX + W > (wide_t)dw ? (wide_t)dw : DX + W;
    wide_t cy1 = DY + H > (wide_t)dh ? (wide_t)dh : DY + H;
    if (cx0 >= cx1 || cy0 >= cy1) {
        CHECK(rc == FB_CLIP_EMPTY, "wide: expected EMPTY rc=%d "
              "(dx=%d dy=%d w=%d h=%d)", rc, dx, dy, w, h);
        return;
    }
    CHECK(rc == FB_CLIP_OK, "wide: expected OK rc=%d", rc);
    if (rc != FB_CLIP_OK) return;
    CHECK((wide_t)got.dst_x == cx0 && (wide_t)got.dst_y == cy0 &&
          (wide_t)got.w == cx1 - cx0 && (wide_t)got.h == cy1 - cy0 &&
          (wide_t)got.src_x == SX + (cx0 - DX) &&
          (wide_t)got.src_y == SY + (cy0 - DY),
          "wide: result mismatch (dx=%d dy=%d w=%d h=%d)", dx, dy, w, h);
}
#endif

static void test_clip(void) {
    /* Hand-written cases with known answers. */
    fb_blit_t b;
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, 10, 20, &b) == FB_CLIP_OK
          && b.dst_x == 10 && b.dst_y == 20 && b.w == 100 && b.h == 100
          && b.src_x == 0 && b.src_y == 0, "plain full present");
    /* Hanging off the left/top: source start must advance. */
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, -30, -40, &b) == FB_CLIP_OK
          && b.dst_x == 0 && b.dst_y == 0 && b.src_x == 30 && b.src_y == 40
          && b.w == 70 && b.h == 60, "clipped left/top");
    /* Hanging off the right/bottom: size shrinks, source start doesn't. */
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, 1000, 750, &b) == FB_CLIP_OK
          && b.dst_x == 1000 && b.src_x == 0 && b.w == 24 && b.h == 18,
          "clipped right/bottom");
    /* Entirely off-screen on each side: EMPTY, not an error. */
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, -100, 0, &b) == FB_CLIP_EMPTY, "just off left");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, 1024, 0, &b) == FB_CLIP_EMPTY, "just off right");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, 0, -100, &b) == FB_CLIP_EMPTY, "just off top");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, 0, 768, &b) == FB_CLIP_EMPTY, "just off bottom");
    /* One pixel of overlap is visible. */
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 100, 100, -99, 0, &b) == FB_CLIP_OK
          && b.w == 1 && b.src_x == 99, "one pixel column visible");
    /* Source bugs are INVALID, never clipped. */
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 101, 100, 0, 0, &b) == FB_CLIP_INVALID, "too wide");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 50, 0, 51, 10, 0, 0, &b) == FB_CLIP_INVALID, "src overruns by 1");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 50, 0, 50, 10, 0, 0, &b) == FB_CLIP_OK, "src exactly fits");
    CHECK(fb_clip_blit(100, 100, 1024, 768, -1, 0, 10, 10, 0, 0, &b) == FB_CLIP_INVALID, "negative src x");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 0, 10, 0, 0, &b) == FB_CLIP_INVALID, "zero width");
    CHECK(fb_clip_blit(100, 100, 1024, 768, 0, 0, 10, -5, 0, 0, &b) == FB_CLIP_INVALID, "negative height");

    /* Exhaustive-ish small sweep against the pixel oracle. */
    for (int i = 0; i < 400000; i++) {
        uint32_t sw = (uint32_t)rnd_range(1, 24), sh = (uint32_t)rnd_range(1, 24);
        uint32_t dw = (uint32_t)rnd_range(1, 30), dh = (uint32_t)rnd_range(1, 30);
        int sx = rnd_range(-4, 26), sy = rnd_range(-4, 26);
        int w = rnd_range(-3, 28), h = rnd_range(-3, 28);
        int dx = rnd_range(-40, 45), dy = rnd_range(-40, 45);
        check_clip_against_pixel_oracle(sw, sh, dw, dh, sx, sy, w, h, dx, dy);
    }

#ifdef __SIZEOF_INT128__
    /* The extremes: wrap-around bait. */
    const int32_t ext[] = { INT_MIN, INT_MIN + 1, -1, 0, 1, 2047, 2048,
                            1023, 1024, 1025, INT_MAX - 1, INT_MAX,
                            -2048, -1024, 1 << 30, -(1 << 30) };
    const int n = (int)(sizeof ext / sizeof ext[0]);
    for (int a = 0; a < n; a++) for (int b2 = 0; b2 < n; b2++)
    for (int c = 0; c < n; c++) for (int d = 0; d < n; d++) {
        check_clip_wide(2048, 2048, 1024, 768, 0, 0, ext[a], ext[b2], ext[c], ext[d]);
        check_clip_wide(2048, 2048, 1024, 768, ext[a], ext[b2], 1, 1, ext[c], ext[d]);
        check_clip_wide(1024, 768, 1024, 768, ext[a], ext[b2], ext[c], ext[d], 0, 0);
    }
    for (int i = 0; i < 300000; i++) {
        check_clip_wide((uint32_t)rnd_range(1, 2048), (uint32_t)rnd_range(1, 2048),
                        (uint32_t)rnd_range(1, 4096), (uint32_t)rnd_range(1, 4096),
                        (int32_t)rnd(), (int32_t)rnd(), (int32_t)rnd(), (int32_t)rnd(),
                        (int32_t)rnd(), (int32_t)rnd());
        /* Biased toward VALID sources so the dst arithmetic gets tested
           with huge dx/dy rather than always bailing out early. */
        check_clip_wide(2048, 2048, 1024, 768,
                        rnd_range(0, 100), rnd_range(0, 100),
                        rnd_range(1, 100), rnd_range(1, 100),
                        (int32_t)rnd(), (int32_t)rnd());
    }
#endif
}

static void test_bounds(void) {
    CHECK(fb_rect_in_bounds(1024, 768, 0, 0, 1024, 768), "whole display");
    CHECK(!fb_rect_in_bounds(1024, 768, 0, 0, 1025, 768), "one too wide");
    CHECK(!fb_rect_in_bounds(1024, 768, 1, 0, 1024, 768), "shifted by one");
    CHECK(fb_rect_in_bounds(1024, 768, 1023, 767, 1, 1), "last pixel");
    CHECK(!fb_rect_in_bounds(1024, 768, 1024, 0, 1, 1), "first pixel past the edge");
    CHECK(!fb_rect_in_bounds(1024, 768, -1, 0, 5, 5), "negative x");
    CHECK(!fb_rect_in_bounds(1024, 768, 0, 0, 0, 5), "zero width");
    CHECK(!fb_rect_in_bounds(1024, 768, INT_MAX, 0, INT_MAX, 1), "x+w wraps");
    CHECK(!fb_rect_in_bounds(1024, 768, 5, INT_MAX, 1, INT_MAX), "y+h wraps");
    CHECK(!fb_rect_in_bounds(1024, 768, 0, 0, INT_MIN, 1), "INT_MIN width");
    for (int i = 0; i < 200000; i++) {
        uint32_t dw = (uint32_t)rnd_range(1, 20), dh = (uint32_t)rnd_range(1, 20);
        int x = rnd_range(-5, 25), y = rnd_range(-5, 25);
        int w = rnd_range(-3, 25), h = rnd_range(-3, 25);
        int expect = (w > 0 && h > 0 && x >= 0 && y >= 0 &&
                      x + w <= (int)dw && y + h <= (int)dh);
        CHECK(fb_rect_in_bounds(dw, dh, x, y, w, h) == (expect != 0),
              "bounds d=%ux%u r=(%d,%d %dx%d)", dw, dh, x, y, w, h);
    }
}

static void test_span(void) {
    uint32_t span = 0;
    CHECK(fb_buffer_span(4096, 1024, 1, &span) && span == 4096, "single row = w*4");
    CHECK(fb_buffer_span(4100, 1024, 3, &span) && span == 2 * 4100u + 4096u,
          "last row has no trailing padding");
    CHECK(!fb_buffer_span(4095, 1024, 2, &span), "stride smaller than a row");
    CHECK(fb_buffer_span(4096, 1024, 768, &span) && span == 767u * 4096u + 4096u,
          "full 1024x768");
    CHECK(!fb_buffer_span(0xFFFFFFFFu, 1, 2, &span), "stride near 2^32, 2 rows overflows");
    CHECK(fb_buffer_span(0xFFFFFFFFu, 1, 1, &span) && span == 4, "stride near 2^32 but 1 row is fine");
    CHECK(!fb_buffer_span(0x80000000u, 1, 3, &span), "(h-1)*stride wraps to 0");
    CHECK(!fb_buffer_span(0x10000000u, 1, 17, &span), "16*2^28 = 2^32 exactly wraps");
    CHECK(!fb_buffer_span(4096, 0x40000000, 1, &span), "w*4 overflows 32 bits");
    CHECK(!fb_buffer_span(4096, 0, 5, &span), "zero width");
    CHECK(!fb_buffer_span(4096, 5, 0, &span), "zero height");
    CHECK(!fb_buffer_span(4096, -1, 5, &span), "negative width");
    CHECK(!fb_buffer_span(4096, 1024, INT_MIN, &span), "negative height");
    CHECK(!fb_buffer_span(4096, 1024, INT_MAX, &span), "huge height overflows");

#ifdef __SIZEOF_INT128__
    const uint32_t strides[] = { 0, 1, 4, 4095, 4096, 4097, 0x7FFFFFFFu, 0x80000000u,
                                 0xFFFFFFFEu, 0xFFFFFFFFu, 0x10000000u, 0x20000000u };
    const int32_t dims[] = { INT_MIN, -1, 0, 1, 2, 3, 16, 17, 768, 1024, 4096,
                             0x3FFFFFFF, 0x40000000, INT_MAX };
    for (unsigned a = 0; a < sizeof strides / sizeof strides[0]; a++)
    for (unsigned b = 0; b < sizeof dims / sizeof dims[0]; b++)
    for (unsigned c = 0; c < sizeof dims / sizeof dims[0]; c++) {
        uint32_t st = strides[a]; int32_t w = dims[b], h = dims[c];
        wide_t W = w, H = h;
        int ok = 0; wide_t need = 0;
        if (W > 0 && H > 0 && (wide_t)st >= W * 4) {
            need = (H - 1) * (wide_t)st + W * 4;
            ok = need <= (wide_t)0xFFFFFFFFu;
        }
        uint32_t got = 0xDEADBEEF;
        int rc = fb_buffer_span(st, w, h, &got);
        CHECK(rc == ok, "span stride=%u w=%d h=%d rc=%d want=%d", st, w, h, rc, ok);
        if (rc && ok) CHECK((wide_t)got == need, "span value %u != %lld", got, (long long)need);
    }
    for (int i = 0; i < 300000; i++) {
        uint32_t st = rnd(); int32_t w = (int32_t)(rnd() % 5000u) - 100;
        int32_t h = (int32_t)(rnd() >> (rnd() % 32u));
        wide_t W = w, H = h; int ok = 0; wide_t need = 0;
        if (W > 0 && H > 0 && (wide_t)st >= W * 4) {
            need = (H - 1) * (wide_t)st + W * 4;
            ok = need <= (wide_t)0xFFFFFFFFu;
        }
        uint32_t got = 0;
        int rc = fb_buffer_span(st, w, h, &got);
        CHECK(rc == ok, "span rand stride=%u w=%d h=%d", st, w, h);
        if (rc && ok) CHECK((wide_t)got == need, "span rand value");
    }
#endif
}

static void test_surface_sizes(void) {
    CHECK(fb_surface_dims_ok(1, 1), "1x1");
    CHECK(fb_surface_dims_ok(2048, 2048), "max");
    CHECK(!fb_surface_dims_ok(0, 5), "zero width");
    CHECK(!fb_surface_dims_ok(5, 0), "zero height");
    CHECK(!fb_surface_dims_ok(2049, 5), "too wide");
    CHECK(!fb_surface_dims_ok(5, 2049), "too tall");
    CHECK(!fb_surface_dims_ok(0xFFFFFFFFu, 0xFFFFFFFFu), "huge");
    CHECK(fb_surface_bytes(2048, 2048) == 16u * 1024u * 1024u, "max surface is exactly 16MB");
    CHECK(fb_surface_bytes(1024, 768) == 3145728u, "1024x768");
    CHECK(fb_pages_for_bytes(1) == 1 && fb_pages_for_bytes(4096) == 1 &&
          fb_pages_for_bytes(4097) == 2 && fb_pages_for_bytes(16u * 1024u * 1024u) == 4096,
          "page rounding");
    /* 3x1 px = 12 bytes -> 1 page; every dims pair's bytes fit in 32 bits */
    for (uint32_t w = 1; w <= 2048; w += 37)
        for (uint32_t h = 1; h <= 2048; h += 41) {
            uint64_t exact = (uint64_t)w * h * 4;
            CHECK(fb_surface_bytes(w, h) == exact && exact <= 0xFFFFFFFFu, "bytes %ux%u", w, h);
            CHECK((uint64_t)fb_pages_for_bytes(fb_surface_bytes(w, h)) * 4096 >= exact &&
                  (uint64_t)fb_pages_for_bytes(fb_surface_bytes(w, h)) * 4096 < exact + 4096,
                  "pages %ux%u", w, h);
        }
}

int main(void) {
    test_clip();
    test_bounds();
    test_span();
    test_surface_sizes();
    printf("fb_geom host test: %ld checks, %ld failures\n", checks, failures);
    return failures ? 1 : 0;
}
