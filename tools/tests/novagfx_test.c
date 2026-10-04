/*
 * novagfx_test.c - Phase 81: host-side tests for the drawing helpers in
 * userland/libc/include/novagfx.h (put_pixel / fill_rect / copy_rect).
 * Compiles the REAL header. Each operation is checked against a
 * per-pixel oracle that contains no clipping arithmetic at all: for
 * every pixel of the surface, ask "is this pixel inside the requested
 * rectangle?" with plain comparisons, and "what would it have been
 * copied from?" - the same technique as fb_geom_test.c's pixel-set
 * oracle, and for the same reason (two unrelated algorithms agreeing
 * on 100k+ random cases is strong evidence; one algorithm agreeing
 * with itself is none).
 *
 * Out-of-bounds writes are the failure that matters most for a drawing
 * routine and the one a "does the picture look right" check misses, so
 * every surface is allocated inside guard words (and uses a stride
 * wider than width*4, with the padding bytes guarded too): any store
 * outside the visible pixels corrupts a guard and fails the test.
 */
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include <novagfx.h>

static long checks, failures;
#define CHECK(cond, ...) do { checks++; if (!(cond)) { failures++; \
    if (failures <= 15) { printf("FAIL %s:%d: %s\n   ", __FILE__, __LINE__, #cond); \
    printf(__VA_ARGS__); printf("\n"); } } } while (0)

static unsigned int rng_state = 0xC0FFEEu;
static unsigned int rnd(void) {
    rng_state ^= rng_state << 13; rng_state ^= rng_state >> 17; rng_state ^= rng_state << 5;
    return rng_state;
}
static int rr(int lo, int hi) { return lo + (int)(rnd() % (unsigned)(hi - lo + 1)); }

#define GUARD_WORDS 16
#define GUARD 0xDEADBEEFu
#define PAD_WORDS 3 /* stride = (width + PAD_WORDS) * 4 */

typedef struct {
    nova_surface_t s;
    unsigned int* block;  /* allocation including guards */
    unsigned int words;
} tsurf_t;

static void tsurf_make(tsurf_t* t, unsigned w, unsigned h, unsigned fill_seed) {
    unsigned stride_words = w + PAD_WORDS;
    t->words = GUARD_WORDS + stride_words * h + GUARD_WORDS;
    t->block = malloc(t->words * 4);
    for (unsigned i = 0; i < t->words; i++) t->block[i] = GUARD;
    t->s.handle = 1;
    t->s.width = w;
    t->s.height = h;
    t->s.stride = stride_words * 4;
    t->s.pixels = t->block + GUARD_WORDS;
    for (unsigned y = 0; y < h; y++)
        for (unsigned x = 0; x < w; x++)
            t->s.pixels[y * stride_words + x] = (fill_seed + y * 1000u + x) & 0x00FFFFFFu;
}
static unsigned tsurf_at(const tsurf_t* t, unsigned x, unsigned y) {
    return t->s.pixels[y * (t->s.stride / 4) + x];
}
static int tsurf_guards_ok(const tsurf_t* t) {
    unsigned stride_words = t->s.stride / 4;
    for (unsigned i = 0; i < GUARD_WORDS; i++)
        if (t->block[i] != GUARD || t->block[t->words - 1 - i] != GUARD) return 0;
    for (unsigned y = 0; y < t->s.height; y++)
        for (unsigned p = 0; p < PAD_WORDS; p++)
            if (t->s.pixels[y * stride_words + t->s.width + p] != GUARD) return 0;
    return 1;
}
static void tsurf_free(tsurf_t* t) { free(t->block); }

static void test_put_pixel(void) {
    for (int i = 0; i < 20000; i++) {
        unsigned w = (unsigned)rr(1, 9), h = (unsigned)rr(1, 9);
        tsurf_t a, ref; tsurf_make(&a, w, h, 7); tsurf_make(&ref, w, h, 7);
        int x = (rnd() % 4 == 0) ? (int)rnd() : rr(-3, 12);
        int y = (rnd() % 4 == 0) ? (int)rnd() : rr(-3, 12);
        nova_surface_put_pixel(&a.s, x, y, 0x123456);
        if (x >= 0 && y >= 0 && (unsigned)x < w && (unsigned)y < h)
            ref.s.pixels[(unsigned)y * (ref.s.stride / 4) + (unsigned)x] = 0x123456;
        int same = 1;
        for (unsigned py = 0; py < h; py++) for (unsigned px = 0; px < w; px++)
            if (tsurf_at(&a, px, py) != tsurf_at(&ref, px, py)) same = 0;
        CHECK(same && tsurf_guards_ok(&a), "put_pixel (%d,%d) on %ux%u", x, y, w, h);
        tsurf_free(&a); tsurf_free(&ref);
    }
}

static int pick_coord(void) {
    switch (rnd() % 6) {
        case 0: return INT_MIN; case 1: return INT_MAX; case 2: return INT_MIN + 1;
        case 3: return INT_MAX - 1; default: return rr(-8, 25);
    }
}
static int pick_size(void) {
    switch (rnd() % 8) {
        case 0: return INT_MAX; case 1: return INT_MIN; case 2: return 0; case 3: return -1;
        default: return rr(-2, 22);
    }
}

static void test_fill(void) {
    for (int i = 0; i < 150000; i++) {
        unsigned w = (unsigned)rr(1, 14), h = (unsigned)rr(1, 14);
        tsurf_t a; tsurf_make(&a, w, h, 99);
        unsigned orig[14 * 14];
        for (unsigned py = 0; py < h; py++) for (unsigned px = 0; px < w; px++)
            orig[py * 14 + px] = tsurf_at(&a, px, py);
        int x = (i & 1) ? pick_coord() : rr(-8, 20), y = (i & 1) ? pick_coord() : rr(-8, 20);
        int fw = (i & 2) ? pick_size() : rr(-2, 22), fh = (i & 2) ? pick_size() : rr(-2, 22);
        nova_surface_fill_rect(&a.s, x, y, fw, fh, 0xABCDEF);

        int ok = 1;
        for (unsigned py = 0; py < h; py++) for (unsigned px = 0; px < w; px++) {
            /* oracle: plain wide-integer containment, no clipping math */
            long long lx = x, ly = y, lw = fw, lh = fh;
            int inside = lw > 0 && lh > 0 && (long long)px >= lx && (long long)px < lx + lw &&
                         (long long)py >= ly && (long long)py < ly + lh;
            unsigned want = inside ? 0xABCDEFu : orig[py * 14 + px];
            if (tsurf_at(&a, px, py) != want) ok = 0;
        }
        CHECK(ok, "fill (%d,%d %dx%d) on %ux%u", x, y, fw, fh, w, h);
        CHECK(tsurf_guards_ok(&a), "fill wrote out of bounds: (%d,%d %dx%d) on %ux%u", x, y, fw, fh, w, h);
        tsurf_free(&a);
    }
}

static void test_copy(int same_surface) {
    for (int i = 0; i < 150000; i++) {
        unsigned sw = (unsigned)rr(1, 12), sh = (unsigned)rr(1, 12);
        unsigned dw = same_surface ? sw : (unsigned)rr(1, 12);
        unsigned dh = same_surface ? sh : (unsigned)rr(1, 12);
        tsurf_t src, dst;
        tsurf_make(&src, sw, sh, 5000);
        if (!same_surface) tsurf_make(&dst, dw, dh, 900000);
        tsurf_t* D = same_surface ? &src : &dst;

        /* snapshot of everything BEFORE the copy: the oracle's "source" */
        unsigned ssnap[12 * 12], dsnap[12 * 12];
        for (unsigned py = 0; py < sh; py++) for (unsigned px = 0; px < sw; px++)
            ssnap[py * 12 + px] = tsurf_at(&src, px, py);
        for (unsigned py = 0; py < dh; py++) for (unsigned px = 0; px < dw; px++)
            dsnap[py * 12 + px] = tsurf_at(D, px, py);

        int sx = (i & 1) ? pick_coord() : rr(-8, 16), sy = (i & 1) ? pick_coord() : rr(-8, 16);
        int dx = (i & 1) ? pick_coord() : rr(-8, 16), dy = (i & 1) ? pick_coord() : rr(-8, 16);
        int w = (i & 2) ? pick_size() : rr(-2, 16), h = (i & 2) ? pick_size() : rr(-2, 16);
        nova_surface_copy_rect(&D->s, dx, dy, &src.s, sx, sy, w, h);

        int ok = 1;
        for (unsigned py = 0; py < dh; py++) for (unsigned px = 0; px < dw; px++) {
            /* pixel (px,py) is written iff it lies in the dst block AND
               the pixel it maps from lies in the src surface */
            long long rx = (long long)px - dx, ry = (long long)py - dy;
            int in_block = w > 0 && h > 0 && rx >= 0 && rx < w && ry >= 0 && ry < h;
            long long fx = (long long)sx + rx, fy = (long long)sy + ry;
            int src_ok = fx >= 0 && fx < (long long)sw && fy >= 0 && fy < (long long)sh;
            unsigned want = (in_block && src_ok) ? ssnap[fy * 12 + fx] : dsnap[py * 12 + px];
            if (tsurf_at(D, px, py) != want) ok = 0;
        }
        CHECK(ok, "%s copy src(%d,%d) dst(%d,%d) %dx%d  s=%ux%u d=%ux%u",
              same_surface ? "overlap" : "cross", sx, sy, dx, dy, w, h, sw, sh, dw, dh);
        CHECK(tsurf_guards_ok(&src) && (same_surface || tsurf_guards_ok(&dst)),
              "copy wrote out of bounds");
        tsurf_free(&src);
        if (!same_surface) tsurf_free(&dst);
    }
}

int main(void) {
    test_put_pixel();
    test_fill();
    test_copy(0);
    test_copy(1);
    printf("novagfx host test: %ld checks, %ld failures\n", checks, failures);
    return failures ? 1 : 0;
}
