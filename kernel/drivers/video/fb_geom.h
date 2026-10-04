#ifndef DRIVERS_VIDEO_FB_GEOM_H
#define DRIVERS_VIDEO_FB_GEOM_H

/*
 * fb_geom.h - Phase 81: the framebuffer API's rectangle and size
 * arithmetic, as pure functions with no kernel dependencies.
 *
 * Why this is its own header: every number in here arrives from
 * ring 3 as a plain `int`, and arithmetic on attacker-chosen ints is
 * where the classic bugs live - `x + width` wrapping negative so a
 * bounds check passes, `(height - 1) * stride` wrapping to a small
 * positive number so a "big enough buffer" check passes for a buffer
 * that is nothing of the kind. A bug there is not a wrong picture, it
 * is the kernel copying past the end of a user buffer or a framebuffer.
 * Keeping the math free of kernel types means tools/tests/
 * fb_geom_test.c (`make fb-test`) can compile this exact file on the
 * host and throw edge cases and a few hundred thousand random
 * rectangles at it, checked against an independent, obviously-correct
 * (slow, wide-integer) reference - the same "test the real code
 * against an oracle" approach userland/libc/tests/ already uses for
 * the libc.
 *
 * Constraint worth knowing about: this builds for 32-bit freestanding
 * x86 with no libgcc, so 64-bit MULTIPLY and DIVIDE are off limits
 * (gcc would emit calls to __muldi3/__divdi3, which this kernel does
 * not link). 64-bit add, subtract and compare compile inline and are
 * used freely; the one place a product could overflow 32 bits is
 * checked with a 32-bit division instead (fb_buffer_span()).
 */

#ifdef FB_GEOM_HOST
#include <stdint.h>
#include <stdbool.h>
#else
#include "../../include/types.h"
#endif

#define FB_GEOM_BPP 4u
#define FB_GEOM_MAX_DIM 2048u

/* Surface dimensions must each be 1..FB_GEOM_MAX_DIM. */
static inline bool fb_surface_dims_ok(uint32_t w, uint32_t h) {
    return w >= 1u && h >= 1u && w <= FB_GEOM_MAX_DIM && h <= FB_GEOM_MAX_DIM;
}

/* Bytes for a surface whose dims already passed fb_surface_dims_ok():
 * at most 2048*2048*4 = 16MB, so this cannot overflow 32 bits (and is
 * therefore only ever called after that check - garbage in, garbage
 * out otherwise, by design). */
static inline uint32_t fb_surface_bytes(uint32_t w, uint32_t h) {
    return w * h * FB_GEOM_BPP;
}

/* Whole 4KB pages needed for `bytes` (<= 16MB, so no overflow). */
static inline uint32_t fb_pages_for_bytes(uint32_t bytes) {
    return (bytes + 4095u) / 4096u;
}

typedef struct {
    int32_t src_x, src_y; /* where to start reading in the surface */
    int32_t dst_x, dst_y; /* where to start writing on the display */
    int32_t w, h;         /* the (possibly shrunk) size to copy */
} fb_blit_t;

#define FB_CLIP_INVALID (-1) /* caller bug: reject with -EINVAL */
#define FB_CLIP_EMPTY    0   /* nothing visible: succeed, do nothing */
#define FB_CLIP_OK       1   /* *out describes the copy to perform */

/* Clips a present: copy the (sx,sy,w,h) rectangle of a sw x sh surface
 * to (dx,dy) on a dw x dh display.
 *
 * Source out of the surface is INVALID, not clipped: reading outside
 * your own surface means the caller's idea of its own picture is
 * wrong, and silently shrinking it would hide the bug. Destination
 * off the display is CLIPPED, not an error: a window partly dragged
 * off-screen is ordinary, and when the left/top edge is clipped the
 * source start moves with it (otherwise the picture would shift). All
 * comparisons are done in 64-bit so no int32 sum can wrap. */
static inline int fb_clip_blit(uint32_t sw, uint32_t sh,
                               uint32_t dw, uint32_t dh,
                               int32_t sx, int32_t sy,
                               int32_t w, int32_t h,
                               int32_t dx, int32_t dy,
                               fb_blit_t* out) {
    if (w <= 0 || h <= 0) {
        return FB_CLIP_INVALID;
    }
    if (sx < 0 || sy < 0) {
        return FB_CLIP_INVALID;
    }
    if ((int64_t)sx + (int64_t)w > (int64_t)sw ||
        (int64_t)sy + (int64_t)h > (int64_t)sh) {
        return FB_CLIP_INVALID;
    }

    int64_t x0 = dx;
    int64_t y0 = dy;
    int64_t x1 = (int64_t)dx + (int64_t)w;
    int64_t y1 = (int64_t)dy + (int64_t)h;
    int64_t cx0 = x0 < 0 ? 0 : x0;
    int64_t cy0 = y0 < 0 ? 0 : y0;
    int64_t cx1 = x1 > (int64_t)dw ? (int64_t)dw : x1;
    int64_t cy1 = y1 > (int64_t)dh ? (int64_t)dh : y1;
    if (cx0 >= cx1 || cy0 >= cy1) {
        return FB_CLIP_EMPTY;
    }

    out->src_x = (int32_t)((int64_t)sx + (cx0 - x0));
    out->src_y = (int32_t)((int64_t)sy + (cy0 - y0));
    out->dst_x = (int32_t)cx0;
    out->dst_y = (int32_t)cy0;
    out->w = (int32_t)(cx1 - cx0);
    out->h = (int32_t)(cy1 - cy0);
    return FB_CLIP_OK;
}

/* A rectangle that must lie ENTIRELY inside a dw x dh display (no
 * clipping) - used by readback, where "read the part that happens to
 * be on screen" would silently return less than was asked for. */
static inline bool fb_rect_in_bounds(uint32_t dw, uint32_t dh,
                                     int32_t x, int32_t y,
                                     int32_t w, int32_t h) {
    if (w <= 0 || h <= 0 || x < 0 || y < 0) {
        return false;
    }
    return (int64_t)x + (int64_t)w <= (int64_t)dw &&
           (int64_t)y + (int64_t)h <= (int64_t)dh;
}

/* How many bytes a caller's buffer must span to hold a w x h XRGB
 * rectangle laid out with `stride` bytes per row: (h-1)*stride +
 * w*4 - the last row only needs its pixels, not the padding after
 * them. Returns false if the rectangle is empty/negative, the stride
 * is smaller than one row, or the span does not fit in 32 bits.
 *
 * The overflow test for (h-1)*stride + row deliberately avoids a
 * multiply (see this file's top comment): rows*stride + row <=
 * 0xFFFFFFFF  <=>  stride <= (0xFFFFFFFF - row) / rows, exact in
 * integer arithmetic. */
static inline bool fb_buffer_span(uint32_t stride, int32_t w, int32_t h,
                                  uint32_t* out_span) {
    if (w <= 0 || h <= 0) {
        return false;
    }
    if ((uint32_t)w > 0x3FFFFFFFu) {
        return false; /* w*4 would not fit in 32 bits */
    }
    uint32_t row = (uint32_t)w * FB_GEOM_BPP;
    if (stride < row) {
        return false;
    }
    uint32_t rows_before_last = (uint32_t)h - 1u;
    if (rows_before_last != 0u &&
        stride > (0xFFFFFFFFu - row) / rows_before_last) {
        return false;
    }
    *out_span = rows_before_last * stride + row;
    return true;
}

#endif
