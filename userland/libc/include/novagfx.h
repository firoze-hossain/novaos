#ifndef NOVAGFX_H
#define NOVAGFX_H

/*
 * novagfx.h - Phase 81: the app-side half of the framebuffer API. A
 * thin, header-only library over the SYS_FB_* syscalls
 * (nova_fb_abi.h has the model and the full contract): create a
 * surface, draw into it with ordinary memory writes, present the part
 * that changed.
 *
 * The drawing helpers here (put_pixel, fill_rect, copy_rect) are plain
 * software loops over the surface's memory, with NO syscall in them -
 * which is exactly the property that lets a future GPU backend slot in
 * underneath without anyone rewriting their drawing code: an app that
 * calls these today keeps working unchanged when presenting starts
 * going through a GPU, and a smarter drawing library (or a GPU-
 * accelerated fill/copy) can replace these three functions without the
 * app noticing. They clip silently to the surface, like every 2D
 * library: drawing partly outside your own surface is normal (a
 * sprite sliding off an edge), not an error. Contrast SYS_FB_PRESENT,
 * where an out-of-surface SOURCE rectangle is an error - there it
 * means the caller's idea of its own surface is wrong.
 *
 * Colors are 0x00RRGGBB (NOVA_RGB). All clipping arithmetic uses
 * `long long` add/subtract/compare only (no multiply/divide), so a
 * rectangle with INT_MAX-sized coordinates cannot wrap around into
 * bounds, and the code still links without libgcc.
 */

#include <errno.h>
#include <novasys.h>

/* The kernel can't include errno.h, so nova_fb_abi.h repeats the errno
 * numbers it returns as NOVA_FB_ERR_*. This is the other half of that
 * arrangement: if either side is ever edited out of step, this stops
 * the build instead of letting a program mis-handle an error. */
typedef char novagfx_errno_perm [(NOVA_FB_ERR_PERM  == EPERM)  ? 1 : -1];
typedef char novagfx_errno_io   [(NOVA_FB_ERR_IO    == EIO)    ? 1 : -1];
typedef char novagfx_errno_badf [(NOVA_FB_ERR_BADF  == EBADF)  ? 1 : -1];
typedef char novagfx_errno_nomem[(NOVA_FB_ERR_NOMEM == ENOMEM) ? 1 : -1];
typedef char novagfx_errno_fault[(NOVA_FB_ERR_FAULT == EFAULT) ? 1 : -1];
typedef char novagfx_errno_busy [(NOVA_FB_ERR_BUSY  == EBUSY)  ? 1 : -1];
typedef char novagfx_errno_nodev[(NOVA_FB_ERR_NODEV == ENODEV) ? 1 : -1];
typedef char novagfx_errno_inval[(NOVA_FB_ERR_INVAL == EINVAL) ? 1 : -1];
typedef char novagfx_errno_nospc[(NOVA_FB_ERR_NOSPC == ENOSPC) ? 1 : -1];

#define NOVA_RGB(r, g, b) \
    ((((unsigned int)(r) & 0xFFu) << 16) | \
     (((unsigned int)(g) & 0xFFu) << 8)  | \
      ((unsigned int)(b) & 0xFFu))

typedef struct {
    unsigned int handle;
    unsigned int* pixels; /* row 0, column 0 */
    unsigned int width;
    unsigned int height;
    unsigned int stride;  /* BYTES between rows */
} nova_surface_t;

/* Returns the address of pixel (x, y). No bounds check: for callers
 * that have already clipped. */
static inline unsigned int* nova_surface_pixel(const nova_surface_t* s,
                                               unsigned int x,
                                               unsigned int y) {
    return (unsigned int*)((char*)s->pixels + (unsigned long)y * s->stride) + x;
}

/* ---- surface lifetime --------------------------------------------- */

/* Creates a w x h surface (cleared to 0 = black by the kernel).
 * Returns 0, or a negative errno (-EINVAL bad size, -ENOSPC limit,
 * -ENOMEM). */
static inline int nova_surface_create(nova_surface_t* s, unsigned int w,
                                      unsigned int h) {
    nova_fb_create_t req;
    req.width = w;
    req.height = h;
    req.flags = 0;
    req.handle = 0;
    req.pixels = 0;
    req.stride = 0;
    req.size = 0;
    int rc = sys_fb_create(&req);
    if (rc < 0) {
        return rc;
    }
    s->handle = req.handle;
    s->pixels = (unsigned int*)(unsigned long)req.pixels;
    s->width = w;
    s->height = h;
    s->stride = req.stride;
    return 0;
}

static inline int nova_surface_destroy(nova_surface_t* s) {
    int rc = sys_fb_destroy(s->handle);
    if (rc == 0) {
        s->handle = 0;
        s->pixels = 0;
    }
    return rc;
}

/* ---- drawing (clipped to the surface, no syscalls) ----------------- */

static inline void nova_surface_put_pixel(nova_surface_t* s, int x, int y,
                                          unsigned int color) {
    if (x < 0 || y < 0 || (unsigned int)x >= s->width ||
        (unsigned int)y >= s->height) {
        return;
    }
    *nova_surface_pixel(s, (unsigned int)x, (unsigned int)y) = color;
}

static inline void nova_surface_fill_rect(nova_surface_t* s, int x, int y,
                                          int w, int h, unsigned int color) {
    if (w <= 0 || h <= 0) {
        return;
    }
    long long x0 = x, y0 = y;
    long long x1 = x0 + w, y1 = y0 + h;
    if (x0 < 0) x0 = 0;
    if (y0 < 0) y0 = 0;
    if (x1 > (long long)s->width) x1 = s->width;
    if (y1 > (long long)s->height) y1 = s->height;
    if (x0 >= x1 || y0 >= y1) {
        return;
    }
    for (long long py = y0; py < y1; py++) {
        unsigned int* row = nova_surface_pixel(s, (unsigned int)x0,
                                               (unsigned int)py);
        for (long long px = x0; px < x1; px++) {
            *row++ = color;
        }
    }
}

/* Copies a w x h block from src at (sx, sy) to dst at (dx, dy),
 * clipped to BOTH surfaces (a part of the block that falls outside
 * either is simply not copied, and the rest keeps its alignment). Safe
 * when src and dst are the same surface and the blocks overlap: the
 * result is as if the source block had been copied aside first (like
 * memmove) - what scrolling a window's contents needs. */
static inline void nova_surface_copy_rect(nova_surface_t* dst, int dx, int dy,
                                          const nova_surface_t* src,
                                          int sx, int sy, int w, int h) {
    if (w <= 0 || h <= 0) {
        return;
    }
    long long ww = w, hh = h;
    long long sx0 = sx, sy0 = sy, dx0 = dx, dy0 = dy;

    /* Clip the left/top edges: whichever side is negative drags the
     * other side's start (and shrinks the block) by the same amount. */
    if (sx0 < 0) { dx0 -= sx0; ww += sx0; sx0 = 0; }
    if (dx0 < 0) { sx0 -= dx0; ww += dx0; dx0 = 0; }
    if (sy0 < 0) { dy0 -= sy0; hh += sy0; sy0 = 0; }
    if (dy0 < 0) { sy0 -= dy0; hh += dy0; dy0 = 0; }
    if (ww <= 0 || hh <= 0) {
        return;
    }
    /* Clip the right/bottom edges against both surfaces. */
    if (sx0 >= (long long)src->width || dx0 >= (long long)dst->width ||
        sy0 >= (long long)src->height || dy0 >= (long long)dst->height) {
        return;
    }
    if (ww > (long long)src->width - sx0) ww = (long long)src->width - sx0;
    if (ww > (long long)dst->width - dx0) ww = (long long)dst->width - dx0;
    if (hh > (long long)src->height - sy0) hh = (long long)src->height - sy0;
    if (hh > (long long)dst->height - dy0) hh = (long long)dst->height - dy0;
    if (ww <= 0 || hh <= 0) {
        return;
    }

    int w2 = (int)ww, h2 = (int)hh;
    /* Overlap-safe order: when copying within one surface, walk rows
     * bottom-up if the block moves down, and pixels right-to-left if it
     * moves right - the same rule memmove uses, applied per axis. */
    int row_down = (dst == src && dy0 > sy0);
    int col_back = (dst == src && dx0 > sx0);
    for (int r = 0; r < h2; r++) {
        int row = row_down ? (h2 - 1 - r) : r;
        const unsigned int* s = nova_surface_pixel(src, (unsigned int)sx0,
                                                   (unsigned int)(sy0 + row));
        unsigned int* d = nova_surface_pixel(dst, (unsigned int)dx0,
                                             (unsigned int)(dy0 + row));
        if (col_back) {
            for (int c = w2 - 1; c >= 0; c--) {
                d[c] = s[c];
            }
        } else {
            for (int c = 0; c < w2; c++) {
                d[c] = s[c];
            }
        }
    }
}

/* ---- display ------------------------------------------------------- */

/* Shows the (sx, sy, w, h) part of `s` at (dx, dy) on the display.
 * Requires owning the display (sys_fb_acquire). Source must be inside
 * the surface (-EINVAL); the destination may hang off the display's
 * edges (clipped). One syscall - present your whole frame's damage,
 * not a pixel at a time. */
static inline int nova_surface_present(const nova_surface_t* s, int sx, int sy,
                                       int w, int h, int dx, int dy) {
    nova_fb_present_t req;
    req.handle = s->handle;
    req.src_x = sx;
    req.src_y = sy;
    req.width = w;
    req.height = h;
    req.dst_x = dx;
    req.dst_y = dy;
    req.flags = 0;
    return sys_fb_present(&req);
}

static inline int nova_surface_present_full(const nova_surface_t* s, int dx,
                                            int dy) {
    return nova_surface_present(s, 0, 0, (int)s->width, (int)s->height, dx, dy);
}

/* Reads (x, y, w, h) of what is currently shown on the display into
 * `dst` (rows dst_stride bytes apart, XRGB8888). The rectangle must be
 * entirely on the display. */
static inline int nova_display_readback(int x, int y, int w, int h,
                                        unsigned int* dst,
                                        unsigned int dst_stride) {
    nova_fb_readback_t req;
    req.x = x;
    req.y = y;
    req.width = w;
    req.height = h;
    req.dst = (unsigned int)(unsigned long)dst;
    req.dst_stride = dst_stride;
    req.flags = 0;
    return sys_fb_readback(&req);
}

#endif
