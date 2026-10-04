#ifndef NOVA_FB_ABI_H
#define NOVA_FB_ABI_H

/*
 * nova_fb_abi.h - Phase 81: the ABI of NovaOS's framebuffer graphics
 * syscalls (SYS_FB_*), shared verbatim by the kernel
 * (kernel/drivers/video/fb.c, kernel/arch/x86/cpu/syscall.c) and
 * userland (novagfx.h, novasys.h) - ONE definition, included by both
 * sides, rather than two hand-maintained copies that can drift apart
 * (this project's earlier nova_udp_addr_t is defined twice, and the
 * only thing keeping the copies in sync is care; for an ABI with six
 * structs that's the wrong trade). Deliberately dependency-free: plain
 * `unsigned int` / `int` (32 bits on this i686 target - checked by the
 * size assertions at the bottom, so a port to a different data model
 * breaks the build instead of silently changing the ABI).
 *
 * THE MODEL (why this shape, and what "hand off to a GPU later without
 * a rewrite" means concretely)
 *
 * The old API (SYS_GFX_PUT_PIXEL / SYS_GFX_FILL_RECT) makes the kernel
 * draw: every pixel is a syscall, the app has no memory of its own
 * picture, and the only thing it can ever be backed by is the one VGA
 * memory window those calls poke. There is nothing a GPU could take
 * over, because nothing the app owns is describable to one.
 *
 * This API inverts that, and it is the same model virtio-gpu, KMS/DRM
 * dumb buffers, and Wayland's wl_shm all converge on:
 *
 *   1. An app CREATES a surface: a block of ordinary memory the kernel
 *      maps into the app's own address space (32-bit XRGB pixels,
 *      app-owned, readable and writable with plain loads and stores).
 *      The app draws into it however it likes - this is where any
 *      future GPU-accelerated drawing library plugs in, with no
 *      syscall in the inner loop.
 *   2. The app PRESENTS a rectangle of the surface to the display, as
 *      a "damage" region: "these pixels changed, show them." One
 *      syscall per frame (or per damaged region), not per pixel.
 *   3. The kernel's backend decides HOW the pixels reach the screen.
 *      Today: a CPU copy into a linear framebuffer (VESA/VBE, Phase
 *      79) or into a virtio-gpu resource followed by TRANSFER_TO_HOST_
 *      2D + RESOURCE_FLUSH of exactly the damaged rectangle (Phase 80).
 *      Later: texture upload and a composited quad on a 3D-capable
 *      backend. The syscall ABI below does not change for any of
 *      them - that is the "without a rewrite".
 *
 * Capabilities (NOVA_FB_CAP_*) are reported honestly per backend, so an
 * app can adapt (batch damage rects when presents are device round
 * trips) instead of assuming the best backend.
 *
 * ERRORS: every call returns 0 (or a non-negative handle/count) on
 * success and a NEGATIVE errno value (-EINVAL, -EFAULT, ... using the
 * numbers in errno.h) on failure. No call ever returns -1 for "some
 * reason", and none can panic the kernel on a bad pointer - every
 * user pointer is validated (mapped, user-accessible, writable where
 * the kernel writes) before it is touched.
 */

/* Pixel formats. Only one exists today; the field is here so a second
 * format (say, RGB565 for a tiny display) can be added without
 * changing any struct below. XRGB8888: a 32-bit little-endian word
 * 0x00RRGGBB - blue in byte 0, green in byte 1, red in byte 2, byte 3
 * unused ("X": ignored on present, always 0 on readback - the kernel
 * never lets a backend's alpha convention leak to the app). */
#define NOVA_FB_FORMAT_XRGB8888 1u

#define NOVA_FB_BYTES_PER_PIXEL 4u

/* Backend ids (SYS_FB_INFO .backend, SYS_FB_ACQUIRE's argument). */
#define NOVA_FB_BACKEND_ANY       0u /* acquire only: kernel's choice */
#define NOVA_FB_BACKEND_VBE       1u /* VESA/VBE linear framebuffer */
#define NOVA_FB_BACKEND_VIRTIOGPU 2u /* virtio-gpu 2D resource */

/* Capability bits (SYS_FB_INFO .caps - describes the backend named by
 * .backend). Only properties that are TRUE of the backend today. */
/* present() cost scales with the damaged area: a sub-rectangle present
 * moves only those pixels. */
#define NOVA_FB_CAP_DAMAGE_PRESENT (1u << 0)
/* Presents are DMA transfers to a GPU-side resource followed by a
 * flush - each present is a device round trip, so prefer a few larger
 * damage rectangles over many tiny ones. Not set for a plain CPU
 * framebuffer, where a present is just a memory copy. */
#define NOVA_FB_CAP_GPU_TRANSFER   (1u << 1)
/* SYS_FB_READBACK works. */
#define NOVA_FB_CAP_READBACK       (1u << 2)

/* Limits, so an app can size things without guessing. */
#define NOVA_FB_MAX_SURFACE_DIM       2048u /* width and height each */
#define NOVA_FB_MAX_SURFACES_PER_PROC 4u
#define NOVA_FB_MAX_BYTES_PER_PROC    (16u * 1024u * 1024u)

/* SYS_FB_INFO (47): EBX = nova_fb_info_t*, out. The caller sets
 * struct_size to sizeof its own struct; the kernel fills at most that
 * many bytes (and reports in struct_size how many it wrote), so fields
 * can be appended in a later phase without breaking an older binary
 * that was built against a shorter struct. */
typedef struct {
    unsigned int struct_size;
    unsigned int width;           /* display size, in pixels, of .backend */
    unsigned int height;
    unsigned int format;          /* NOVA_FB_FORMAT_* */
    unsigned int backend;         /* what ACQUIRE(ANY) would pick */
    unsigned int backend_mask;    /* bit (1u << id) for each one present */
    unsigned int caps;            /* NOVA_FB_CAP_* of .backend */
    unsigned int max_surface_dim;
    unsigned int max_surfaces;    /* per process */
} nova_fb_info_t;

/* SYS_FB_ACQUIRE (48): EBX = backend id (NOVA_FB_BACKEND_*). Takes
 * exclusive ownership of the display: switches to graphics output
 * (a no-op where there is nothing to switch) and clears it to black.
 * -ENODEV: no such backend (or none at all); -EBUSY: the display is
 * already owned, including by the caller; -EINVAL: unknown id. */
/* SYS_FB_RELEASE (49): no arguments. Gives the display back (text
 * console restored where the backend shares it). -EPERM if the caller
 * does not own it. A process that exits without releasing is released
 * automatically - a crashed graphics app never strands the machine in
 * graphics mode. */

/* SYS_FB_CREATE (50): EBX = nova_fb_create_t*, in/out. Creating does
 * not require owning the display. -EINVAL: bad size or nonzero flags;
 * -ENOSPC: per-process/system surface or byte limit reached;
 * -ENOMEM: out of memory. */
typedef struct {
    unsigned int width;   /* in: 1..NOVA_FB_MAX_SURFACE_DIM */
    unsigned int height;  /* in: 1..NOVA_FB_MAX_SURFACE_DIM */
    unsigned int flags;   /* in: reserved, must be 0 */
    unsigned int handle;  /* out: opaque, never 0 */
    unsigned int pixels;  /* out: address of the surface in YOUR address
                             space (a 32-bit pointer) */
    unsigned int stride;  /* out: bytes per row (width * 4) */
    unsigned int size;    /* out: bytes mapped (rounded up to pages) */
} nova_fb_create_t;

/* SYS_FB_DESTROY (51): EBX = handle. The memory is unmapped and
 * returned. A destroyed handle stays invalid even if its slot is
 * reused (handles carry a generation) -> -EBADF. */

/* SYS_FB_PRESENT (52): EBX = nova_fb_present_t*, in. Copies the
 * source rectangle of the surface to the display at (dst_x, dst_y).
 * Requires owning the display (-EPERM otherwise). The SOURCE
 * rectangle must lie entirely inside the surface (-EINVAL - that is a
 * bug in the caller). The DESTINATION may hang off any edge of the
 * display: it is clipped (a window dragged partly off-screen is
 * normal), and a rectangle entirely off-screen succeeds having done
 * nothing. */
typedef struct {
    unsigned int handle;
    int src_x, src_y;
    int width, height;    /* > 0 */
    int dst_x, dst_y;
    unsigned int flags;   /* reserved, must be 0 */
} nova_fb_present_t;

/* SYS_FB_READBACK (53): EBX = nova_fb_readback_t*, in. Copies a
 * rectangle of the DISPLAY (what has been presented) into the
 * caller's buffer as XRGB8888. Requires owning the display. The
 * rectangle must lie entirely inside the display (-EINVAL). The
 * display is cleared to black when acquired, so a client can only
 * ever read back what it (or an earlier presenter in the same
 * ownership) put there - never a previous owner's screen. */
typedef struct {
    int x, y;
    int width, height;    /* > 0 */
    unsigned int dst;     /* address of the destination buffer in YOUR
                             address space */
    unsigned int dst_stride; /* bytes per row, >= width * 4 */
    unsigned int flags;   /* reserved, must be 0 */
} nova_fb_readback_t;

/* The error numbers this API returns (negated). They are the standard
 * errno values - identical to errno.h's EPERM, EBADF, ... - repeated
 * here under NOVA_FB_ERR_* only because the kernel includes this header
 * and has no errno.h of its own. novagfx.h asserts at compile time
 * that every one still equals its errno.h twin, so the two cannot
 * drift apart unnoticed. */
#define NOVA_FB_ERR_PERM    1  /* EPERM:  you do not own the display */
#define NOVA_FB_ERR_IO      5  /* EIO:    the device did not do it */
#define NOVA_FB_ERR_BADF    9  /* EBADF:  no such surface handle */
#define NOVA_FB_ERR_NOMEM  12  /* ENOMEM: out of memory */
#define NOVA_FB_ERR_FAULT  14  /* EFAULT: bad pointer */
#define NOVA_FB_ERR_BUSY   16  /* EBUSY:  display already owned */
#define NOVA_FB_ERR_NODEV  19  /* ENODEV: no such backend */
#define NOVA_FB_ERR_INVAL  22  /* EINVAL: bad argument */
#define NOVA_FB_ERR_NOSPC  28  /* ENOSPC: surface/byte limit reached */

/* Size assertions: this is an ABI. If any of these fire, a struct
 * changed shape - which breaks every compiled binary. */
typedef char nova_fb_abi_check_info[(sizeof(nova_fb_info_t) == 36) ? 1 : -1];
typedef char nova_fb_abi_check_create[(sizeof(nova_fb_create_t) == 28) ? 1 : -1];
typedef char nova_fb_abi_check_present[(sizeof(nova_fb_present_t) == 32) ? 1 : -1];
typedef char nova_fb_abi_check_readback[(sizeof(nova_fb_readback_t) == 28) ? 1 : -1];

#endif
