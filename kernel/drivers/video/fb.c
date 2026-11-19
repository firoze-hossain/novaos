/*
 * fb.c - Phase 81: the framebuffer graphics API. See fb.h for the
 * layering and userland/libc/include/nova_fb_abi.h for the contract
 * and the reasoning behind its shape.
 */
#include "fb.h"
#include "fb_geom.h"
#include "vbe.h"
#include "../virtiogpu/virtiogpu.h"
#include "../vga/vga.h"
#include "../../arch/x86/mm/paging.h"
#include "../../arch/x86/mm/pmm.h"
#include "../../lib/string.h"
#include "../../lib/spinlock.h"
#include "../../include/kernel.h"

#define ERR(name) (-(NOVA_FB_ERR_##name))

/* ------------------------------------------------------------------ *
 * Surface memory layout.
 *
 * Each surface lives at a fixed, per-process virtual address chosen
 * from slots of FB_SURFACE_SLOT_BYTES (16MB - exactly one maximum-size
 * surface, so slots can never overlap however the sizes are mixed)
 * starting at FB_SURFACE_VIRT_BASE. 0x60000000 sits clear of every
 * other region this kernel carves out of a user address space: the
 * executable (0x08048000), shared libraries (0x10000000-0x12000000),
 * the heap (0x20000000 up) and the stack (0x40000000) - see
 * kernel/task/process.h.
 * ------------------------------------------------------------------ */
#define FB_SURFACE_VIRT_BASE  0x60000000u
#define FB_SURFACE_SLOT_BYTES 0x01000000u
#define FB_MAX_SURFACES       16u

typedef char fb_slot_covers_max_surface
    [(FB_SURFACE_SLOT_BYTES >= NOVA_FB_MAX_SURFACE_DIM *
                               NOVA_FB_MAX_SURFACE_DIM *
                               NOVA_FB_BYTES_PER_PIXEL) ? 1 : -1];

typedef struct {
    bool in_use;     /* slot reserved (possibly still being built) */
    bool ready;      /* fully created: lookup() may return it */
    int owner_pid;
    uint32_t gen;    /* bumped on every creation, so a stale handle to
                        a reused slot no longer matches */
    uint32_t width, height, stride;
    uint32_t vaddr;  /* in the owner's address space */
    uint32_t pages;
    uint32_t slot;   /* which of the owner's FB_SURFACE_SLOT_BYTES slots */
} fb_surface_t;

static fb_surface_t g_surfaces[FB_MAX_SURFACES];

/* Zero-initialised == unlocked (see spinlock.h), so this is usable
 * even by a process-exit hook that somehow ran before fb_init(). */
static spinlock_t g_lock;

/* ------------------------------------------------------------------ *
 * Backends.
 * ------------------------------------------------------------------ */
typedef struct {
    const char* name;
    uint32_t id;
    uint32_t caps;
    /* True (and the display size filled in) if this backend can drive a
     * display right now. */
    bool (*probe)(uint32_t* width, uint32_t* height);
    /* Take over the screen and clear it to black. */
    bool (*acquire)(void);
    void (*release)(void);
    /* Show w*h XRGB pixels (rows src_stride bytes apart) at (dx, dy).
     * The rectangle is already validated and clipped to the display. */
    bool (*present)(const uint8_t* src, uint32_t src_stride,
                    int dx, int dy, int w, int h);
    /* The inverse: XRGB pixels (X always 0) from the backend's store. */
    bool (*readback)(uint8_t* dst, uint32_t dst_stride,
                     int x, int y, int w, int h);
} fb_backend_t;

static inline void copy_dwords(void* dst, const void* src, uint32_t n) {
    __asm__ volatile ("rep movsl"
                      : "+D"(dst), "+S"(src), "+c"(n)
                      :
                      : "memory");
}

static inline void fill_dwords(void* dst, uint32_t value, uint32_t n) {
    __asm__ volatile ("rep stosl"
                      : "+D"(dst), "+c"(n)
                      : "a"(value)
                      : "memory");
}

/* ---- VBE: a plain CPU copy into the linear framebuffer ----------- */

static bool vbe_probe(uint32_t* w, uint32_t* h) {
    volatile uint8_t* base;
    uint32_t pitch;
    return vbe_get_xrgb8888_surface(&base, w, h, &pitch);
}

static bool vbe_acquire(void) {
    volatile uint8_t* base;
    uint32_t w, h, pitch;
    if (!vbe_get_xrgb8888_surface(&base, &w, &h, &pitch)) {
        return false;
    }
    /* Clear BEFORE switching the display over, so the first thing
     * ever shown is black, not whatever the last owner left behind. */
    fill_dwords((void*)base, 0, pitch / 4u * h);
    vbe_enter_graphics();
    return true;
}

static void vbe_release(void) {
    vbe_exit_graphics();
    vga_clear();
}

static bool vbe_present(const uint8_t* src, uint32_t src_stride,
                        int dx, int dy, int w, int h) {
    volatile uint8_t* base;
    uint32_t dw, dh, pitch;
    if (!vbe_get_xrgb8888_surface(&base, &dw, &dh, &pitch)) {
        return false;
    }
    for (int row = 0; row < h; row++) {
        copy_dwords((void*)(base + (uint32_t)(dy + row) * pitch +
                            (uint32_t)dx * 4u),
                    src + (uint32_t)row * src_stride, (uint32_t)w);
    }
    return true;
}

static bool vbe_readback(uint8_t* dst, uint32_t dst_stride,
                         int x, int y, int w, int h) {
    volatile uint8_t* base;
    uint32_t dw, dh, pitch;
    if (!vbe_get_xrgb8888_surface(&base, &dw, &dh, &pitch)) {
        return false;
    }
    for (int row = 0; row < h; row++) {
        const volatile uint32_t* s = (const volatile uint32_t*)
            (base + (uint32_t)(y + row) * pitch + (uint32_t)x * 4u);
        uint32_t* d = (uint32_t*)(dst + (uint32_t)row * dst_stride);
        for (int i = 0; i < w; i++) {
            d[i] = s[i] & 0x00FFFFFFu; /* X is always 0 to the app */
        }
    }
    return true;
}

/* ---- virtio-gpu: copy into the resource's backing, then DMA ------ */

static bool gpu_probe(uint32_t* w, uint32_t* h) {
    uint8_t* backing;
    return virtiogpu_get_surface(&backing, w, h);
}

static bool gpu_acquire(void) {
    uint8_t* backing;
    uint32_t w, h;
    if (!virtiogpu_get_surface(&backing, &w, &h)) {
        return false;
    }
    /* Opaque black: B8G8R8A8 has a live alpha byte, see gpu_present(). */
    fill_dwords(backing, 0xFF000000u, w * h);
    return virtiogpu_flush_rect(0, 0, w, h);
}

static void gpu_release(void) {
    /* Nothing to hand back: this backend never took the text console
     * away (it is a separate display head), so there is nothing to
     * restore. The resource keeps its last frame until the next
     * acquire clears it. */
}

static bool gpu_present(const uint8_t* src, uint32_t src_stride,
                        int dx, int dy, int w, int h) {
    uint8_t* backing;
    uint32_t dw, dh;
    if (!virtiogpu_get_surface(&backing, &dw, &dh)) {
        return false;
    }
    for (int row = 0; row < h; row++) {
        const uint32_t* s = (const uint32_t*)(src + (uint32_t)row * src_stride);
        uint32_t* d = (uint32_t*)(backing + ((uint32_t)(dy + row) * dw +
                                             (uint32_t)dx) * 4u);
        /* Force alpha opaque. The ABI's format is XRGB (the X byte is
         * "don't care", and apps routinely leave it 0), but the
         * resource is B8G8R8A8 and a host display stack is entitled to
         * honour that alpha - an app's zero X byte must not turn into
         * a transparent screen. */
        for (int i = 0; i < w; i++) {
            d[i] = s[i] | 0xFF000000u;
        }
    }
    return virtiogpu_flush_rect((uint32_t)dx, (uint32_t)dy, (uint32_t)w,
                                (uint32_t)h);
}

static bool gpu_readback(uint8_t* dst, uint32_t dst_stride,
                         int x, int y, int w, int h) {
    uint8_t* backing;
    uint32_t dw, dh;
    if (!virtiogpu_get_surface(&backing, &dw, &dh)) {
        return false;
    }
    for (int row = 0; row < h; row++) {
        const uint32_t* s = (const uint32_t*)
            (backing + ((uint32_t)(y + row) * dw + (uint32_t)x) * 4u);
        uint32_t* d = (uint32_t*)(dst + (uint32_t)row * dst_stride);
        for (int i = 0; i < w; i++) {
            d[i] = s[i] & 0x00FFFFFFu; /* strip the backend's alpha */
        }
    }
    return true;
}

/* Table order IS the automatic-selection order. The VBE backend is
 * first because it drives the same display head the text console and
 * every existing program already use: a program that asks for "any
 * backend" gets the screen the user is already looking at, and a
 * machine that has a virtio-gpu AND a VGA adapter behaves exactly as
 * it did before this API existed. virtio-gpu is chosen automatically
 * when VBE is absent (a pure-virtio machine) or when asked for by id.
 * Apps that want the GPU path regardless say so explicitly. */
static const fb_backend_t g_backends[] = {
    { "vbe", NOVA_FB_BACKEND_VBE,
      NOVA_FB_CAP_DAMAGE_PRESENT | NOVA_FB_CAP_READBACK,
      vbe_probe, vbe_acquire, vbe_release, vbe_present, vbe_readback },
    { "virtio-gpu", NOVA_FB_BACKEND_VIRTIOGPU,
      NOVA_FB_CAP_DAMAGE_PRESENT | NOVA_FB_CAP_GPU_TRANSFER |
          NOVA_FB_CAP_READBACK,
      gpu_probe, gpu_acquire, gpu_release, gpu_present, gpu_readback },
};
#define FB_BACKEND_COUNT (sizeof(g_backends) / sizeof(g_backends[0]))

/* hint 0 = automatic, otherwise exactly that backend. A hinted backend
 * that is absent is NULL, never silently substituted: asking for the
 * GPU path and quietly getting a CPU framebuffer would make a test (or
 * an app's performance assumptions) lie. */
static const fb_backend_t* pick_backend(uint32_t hint, uint32_t* w, uint32_t* h) {
    for (unsigned i = 0; i < FB_BACKEND_COUNT; i++) {
        const fb_backend_t* be = &g_backends[i];
        if (hint != NOVA_FB_BACKEND_ANY && hint != be->id) {
            continue;
        }
        if (be->probe(w, h)) {
            return be;
        }
    }
    return NULL;
}

/* ------------------------------------------------------------------ *
 * Display ownership.
 *
 * TRANSITION is a real state, not decoration: acquire/release call into
 * a backend (port I/O, a device command) and that must not happen with
 * g_lock held (the project's no-nested-locks rule - virtio-gpu has its
 * own lock). Without it there would be a window after the lock is
 * dropped in which a second process could win an acquire and then have
 * the FIRST process's release tear down its freshly taken display.
 * ------------------------------------------------------------------ */
typedef enum { DISPLAY_FREE = 0, DISPLAY_TRANSITION, DISPLAY_OWNED } display_state_t;

static display_state_t g_state;
static int g_owner_pid = -1;
static const fb_backend_t* g_active;
static uint32_t g_disp_w, g_disp_h;

bool fb_display_is_owned(void) {
    return g_state != DISPLAY_FREE;
}

void fb_init(void) {
    uint32_t w, h;
    char line[96];
    unsigned n = 0;
    line[0] = '\0';
    for (unsigned i = 0; i < FB_BACKEND_COUNT; i++) {
        if (g_backends[i].probe(&w, &h)) {
            if (n > 0) {
                strcpy(line + strlen(line), ", ");
            }
            strcpy(line + strlen(line), g_backends[i].name);
            n++;
        }
    }
    const fb_backend_t* def = pick_backend(NOVA_FB_BACKEND_ANY, &w, &h);
    if (def == NULL) {
        kernel_log("[ .. ] Framebuffer API: no usable backend (needs a "
                   "32bpp XRGB8888 VBE framebuffer or a virtio-gpu) - "
                   "SYS_FB_* will return -ENODEV\n");
        return;
    }
    kernel_log("[ OK ] Framebuffer API: %d backend(s) available (%s), "
               "default '%s' %dx%d XRGB8888\n",
               (int)n, line, def->name, (int)w, (int)h);
}

int fb_sys_info(uint32_t user_ptr) {
    /* The struct_size handshake (see the ABI header): read the
     * caller's own idea of the struct's size first, then write at most
     * that much. */
    if (!paging_user_range_ok(user_ptr, 4, false)) {
        return ERR(FAULT);
    }
    uint32_t caller_size;
    memcpy(&caller_size, (const void*)user_ptr, 4);
    if (caller_size < 4) {
        return ERR(INVAL);
    }
    uint32_t n = caller_size < sizeof(nova_fb_info_t)
                     ? caller_size : (uint32_t)sizeof(nova_fb_info_t);
    if (!paging_user_range_ok(user_ptr, n, true)) {
        return ERR(FAULT);
    }

    uint32_t w = 0, h = 0, pw, ph;
    const fb_backend_t* def = pick_backend(NOVA_FB_BACKEND_ANY, &w, &h);
    if (def == NULL) {
        return ERR(NODEV);
    }
    uint32_t mask = 0;
    for (unsigned i = 0; i < FB_BACKEND_COUNT; i++) {
        if (g_backends[i].probe(&pw, &ph)) {
            mask |= 1u << g_backends[i].id;
        }
    }

    nova_fb_info_t info;
    memset(&info, 0, sizeof info);
    info.struct_size = n;
    info.width = w;
    info.height = h;
    info.format = NOVA_FB_FORMAT_XRGB8888;
    info.backend = def->id;
    info.backend_mask = mask;
    info.caps = def->caps;
    info.max_surface_dim = NOVA_FB_MAX_SURFACE_DIM;
    info.max_surfaces = NOVA_FB_MAX_SURFACES_PER_PROC;
    memcpy((void*)user_ptr, &info, n);
    return 0;
}

int fb_sys_acquire(int pid, uint32_t backend) {
    if (backend != NOVA_FB_BACKEND_ANY && backend != NOVA_FB_BACKEND_VBE &&
        backend != NOVA_FB_BACKEND_VIRTIOGPU) {
        return ERR(INVAL);
    }

    uint32_t flags = spinlock_acquire(&g_lock);
    if (g_state != DISPLAY_FREE) {
        spinlock_release(&g_lock, flags);
        return ERR(BUSY);
    }
    uint32_t w = 0, h = 0;
    const fb_backend_t* be = pick_backend(backend, &w, &h);
    if (be == NULL) {
        spinlock_release(&g_lock, flags);
        return ERR(NODEV);
    }
    g_state = DISPLAY_TRANSITION;
    g_owner_pid = pid;
    spinlock_release(&g_lock, flags);

    bool ok = be->acquire();

    flags = spinlock_acquire(&g_lock);
    if (!ok) {
        g_state = DISPLAY_FREE;
        g_owner_pid = -1;
        spinlock_release(&g_lock, flags);
        kernel_log("[FAULT] fb: backend '%s' failed to take over the "
                   "display for pid %d\n", be->name, pid);
        return ERR(IO);
    }
    g_active = be;
    g_disp_w = w;
    g_disp_h = h;
    g_state = DISPLAY_OWNED;
    spinlock_release(&g_lock, flags);

    kernel_log("[ OK ] fb: pid %d acquired the display (backend %s, %dx%d)\n",
               pid, be->name, (int)w, (int)h);
    return 0;
}

/* Gives the display back: shared by SYS_FB_RELEASE and the exit hook.
 * Caller has already moved g_state to TRANSITION under the lock. */
static void finish_release(const fb_backend_t* be, int pid, const char* why) {
    be->release();
    uint32_t flags = spinlock_acquire(&g_lock);
    g_active = NULL;
    g_owner_pid = -1;
    g_state = DISPLAY_FREE;
    spinlock_release(&g_lock, flags);
    kernel_log("[ OK ] fb: display released (%s, pid %d)\n", why, pid);
}

int fb_sys_release(int pid) {
    uint32_t flags = spinlock_acquire(&g_lock);
    if (g_state != DISPLAY_OWNED || g_owner_pid != pid) {
        spinlock_release(&g_lock, flags);
        return ERR(PERM);
    }
    const fb_backend_t* be = g_active;
    g_state = DISPLAY_TRANSITION;
    spinlock_release(&g_lock, flags);
    finish_release(be, pid, "released");
    return 0;
}

void fb_process_exit(int pid) {
    uint32_t flags = spinlock_acquire(&g_lock);
    for (unsigned i = 0; i < FB_MAX_SURFACES; i++) {
        if (g_surfaces[i].in_use && g_surfaces[i].owner_pid == pid) {
            g_surfaces[i].in_use = false;
            g_surfaces[i].ready = false;
        }
    }
    bool owned = (g_state == DISPLAY_OWNED && g_owner_pid == pid);
    const fb_backend_t* be = g_active;
    if (owned) {
        g_state = DISPLAY_TRANSITION;
    }
    spinlock_release(&g_lock, flags);
    if (owned) {
        finish_release(be, pid, "owner exited without releasing");
    }
}

/* ------------------------------------------------------------------ *
 * Surfaces.
 * ------------------------------------------------------------------ */
static uint32_t make_handle(unsigned index, uint32_t gen) {
    return (gen << 8) | (uint32_t)(index + 1u);
}

/* Caller holds g_lock. NULL if the handle is not (or no longer) a
 * ready surface owned by `pid`. */
static fb_surface_t* lookup_locked(int pid, uint32_t handle) {
    uint32_t idx1 = handle & 0xFFu;
    if (idx1 == 0 || idx1 > FB_MAX_SURFACES) {
        return NULL;
    }
    fb_surface_t* s = &g_surfaces[idx1 - 1u];
    if (!s->in_use || !s->ready || s->owner_pid != pid ||
        s->gen != (handle >> 8)) {
        return NULL;
    }
    return s;
}

int fb_sys_create(int pid, uint32_t* pd, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_fb_create_t), true)) {
        return ERR(FAULT);
    }
    nova_fb_create_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);

    if (req.flags != 0 || !fb_surface_dims_ok(req.width, req.height)) {
        return ERR(INVAL);
    }
    uint32_t bytes = fb_surface_bytes(req.width, req.height);
    uint32_t pages = fb_pages_for_bytes(bytes);

    /* Reserve a table entry and a virtual slot under the lock; do the
     * (slow) allocation and mapping outside it. The entry is in_use
     * but not ready, so nobody else can look it up half-built. */
    uint32_t flags = spinlock_acquire(&g_lock);
    unsigned owned_count = 0;
    uint32_t owned_bytes = 0;
    uint32_t used_slots = 0; /* bitmask over this process's 4 slots */
    int free_index = -1;
    for (unsigned i = 0; i < FB_MAX_SURFACES; i++) {
        fb_surface_t* s = &g_surfaces[i];
        if (!s->in_use) {
            if (free_index < 0) {
                free_index = (int)i;
            }
            continue;
        }
        if (s->owner_pid == pid) {
            owned_count++;
            owned_bytes += s->pages * 4096u;
            used_slots |= 1u << s->slot;
        }
    }
    if (free_index < 0 || owned_count >= NOVA_FB_MAX_SURFACES_PER_PROC ||
        owned_bytes + pages * 4096u > NOVA_FB_MAX_BYTES_PER_PROC) {
        spinlock_release(&g_lock, flags);
        return ERR(NOSPC);
    }
    uint32_t slot = 0;
    while (used_slots & (1u << slot)) {
        slot++;
    }
    fb_surface_t* s = &g_surfaces[free_index];
    s->in_use = true;
    s->ready = false;
    s->owner_pid = pid;
    s->gen = (s->gen + 1u) & 0x00FFFFFFu;
    if (s->gen == 0) {
        s->gen = 1; /* a zero generation would let handle 0x...01 alias
                       "no generation yet" in a fresh table */
    }
    s->width = req.width;
    s->height = req.height;
    s->stride = req.width * NOVA_FB_BYTES_PER_PIXEL;
    s->vaddr = FB_SURFACE_VIRT_BASE + slot * FB_SURFACE_SLOT_BYTES;
    s->pages = pages;
    s->slot = slot;
    uint32_t vaddr = s->vaddr;
    uint32_t gen = s->gen;
    spinlock_release(&g_lock, flags);

    /* Allocate, ZERO, and map. Zeroing is not optional: pmm_alloc_frame()
     * hands back frames with whatever the previous user left in them,
     * and these are about to become readable by an unprivileged
     * process - mapping them as-is would leak other processes' (or the
     * kernel's) old memory. (process_sbrk() has this same hole for
     * heap pages; that is outside this phase's scope and is called out
     * in PROGRESS.md rather than quietly widened.) Zeroing goes through
     * the frame's physical address, which is only mapped for the first
     * PAGING_IDENTITY_MAP_BYTES of RAM. The PMM no longer hands out
     * frames beyond that line (pmm.c's allocatable_frames()), so the
     * check below should never fire - it stays as defense in depth
     * because the failure it guards against is a kernel panic, and this
     * is the one place a frame is written to before it is mapped for
     * user space: if the allocator's ceiling is ever raised without
     * the identity map, this refuses (ENOMEM) instead of faulting. */
    uint32_t done = 0;
    int err = 0;
    for (; done < pages; done++) {
        uint32_t frame = pmm_alloc_frame();
        if (frame == 0) {
            err = ERR(NOMEM);
            break;
        }
        if (frame >= PAGING_IDENTITY_MAP_BYTES) {
            pmm_free_frame(frame);
            err = ERR(NOMEM);
            break;
        }
        memset((void*)frame, 0, 4096);
        if (!paging_map_page(pd, vaddr + done * 4096u, frame,
                             PAGE_PRESENT | PAGE_WRITE | PAGE_USER)) {
            pmm_free_frame(frame);
            err = ERR(NOMEM);
            break;
        }
    }
    if (err != 0) {
        for (uint32_t j = 0; j < done; j++) {
            uint32_t old = paging_unmap_page(pd, vaddr + j * 4096u);
            if (old != 0) {
                pmm_free_frame(old & 0xFFFFF000u);
            }
        }
        flags = spinlock_acquire(&g_lock);
        s->in_use = false;
        spinlock_release(&g_lock, flags);
        return err;
    }

    flags = spinlock_acquire(&g_lock);
    s->ready = true;
    spinlock_release(&g_lock, flags);

    /* paging_user_range_ok() above already made this range writable
     * (resolving COW if it had to); nothing since could have changed
     * that, so write the results straight back. */
    req.handle = make_handle((unsigned)free_index, gen);
    req.pixels = vaddr;
    req.stride = req.width * NOVA_FB_BYTES_PER_PIXEL;
    req.size = pages * 4096u;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

int fb_sys_destroy(int pid, uint32_t* pd, uint32_t handle) {
    uint32_t flags = spinlock_acquire(&g_lock);
    fb_surface_t* s = lookup_locked(pid, handle);
    if (s == NULL) {
        spinlock_release(&g_lock, flags);
        return ERR(BADF);
    }
    s->ready = false; /* from here on, lookup() no longer finds it */
    uint32_t vaddr = s->vaddr;
    uint32_t pages = s->pages;
    spinlock_release(&g_lock, flags);

    for (uint32_t i = 0; i < pages; i++) {
        uint32_t old = paging_unmap_page(pd, vaddr + i * 4096u);
        /* A copy-on-write page may still be mapped by a fork()'d
         * relative (this process's own pages are marked COW too when it
         * forks); freeing the frame would hand memory a sibling still
         * uses back to the allocator. The same conservative rule
         * free_user_address_space() applies - leak a bounded amount
         * rather than risk that - and for the same documented reason. */
        if (old != 0 && !(old & PAGE_COW)) {
            pmm_free_frame(old & 0xFFFFF000u);
        }
    }

    flags = spinlock_acquire(&g_lock);
    s->in_use = false;
    spinlock_release(&g_lock, flags);
    return 0;
}

int fb_sys_present(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_fb_present_t), false)) {
        return ERR(FAULT);
    }
    nova_fb_present_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    if (req.flags != 0) {
        return ERR(INVAL);
    }

    uint32_t flags = spinlock_acquire(&g_lock);
    if (g_state != DISPLAY_OWNED || g_owner_pid != pid) {
        spinlock_release(&g_lock, flags);
        return ERR(PERM);
    }
    fb_surface_t* s = lookup_locked(pid, req.handle);
    if (s == NULL) {
        spinlock_release(&g_lock, flags);
        return ERR(BADF);
    }
    uint32_t sw = s->width, sh = s->height, stride = s->stride;
    uint32_t vaddr = s->vaddr;
    uint32_t dw = g_disp_w, dh = g_disp_h;
    const fb_backend_t* be = g_active;
    spinlock_release(&g_lock, flags);

    fb_blit_t b;
    int rc = fb_clip_blit(sw, sh, dw, dh, req.src_x, req.src_y, req.width,
                          req.height, req.dst_x, req.dst_y, &b);
    if (rc == FB_CLIP_INVALID) {
        return ERR(INVAL);
    }
    if (rc == FB_CLIP_EMPTY) {
        return 0;
    }

    /* The copy runs without g_lock held (it can be a multi-megabyte
     * memcpy plus a device round trip). That is safe because only the
     * owner can present, the owner is this very thread, and neither
     * the surface (only this process can destroy it) nor the display
     * (only this process can release it, and the exit hook cannot run
     * under a syscall in progress) can change underneath it. */
    const uint8_t* src = (const uint8_t*)(vaddr + (uint32_t)b.src_y * stride +
                                          (uint32_t)b.src_x * 4u);
    return be->present(src, stride, b.dst_x, b.dst_y, b.w, b.h)
               ? 0 : ERR(IO);
}

int fb_sys_readback(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_fb_readback_t), false)) {
        return ERR(FAULT);
    }
    nova_fb_readback_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    if (req.flags != 0) {
        return ERR(INVAL);
    }

    uint32_t flags = spinlock_acquire(&g_lock);
    if (g_state != DISPLAY_OWNED || g_owner_pid != pid) {
        spinlock_release(&g_lock, flags);
        return ERR(PERM);
    }
    uint32_t dw = g_disp_w, dh = g_disp_h;
    const fb_backend_t* be = g_active;
    spinlock_release(&g_lock, flags);

    if (!fb_rect_in_bounds(dw, dh, req.x, req.y, req.width, req.height)) {
        return ERR(INVAL);
    }
    uint32_t span;
    if (!fb_buffer_span(req.dst_stride, req.width, req.height, &span)) {
        return ERR(INVAL);
    }
    if (!paging_user_range_ok(req.dst, span, true)) {
        return ERR(FAULT);
    }
    return be->readback((uint8_t*)req.dst, req.dst_stride, req.x, req.y,
                        req.width, req.height)
               ? 0 : ERR(IO);
}
