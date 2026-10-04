/*
 * gfxtest.c - Phase 81: the in-OS conformance test for the SYS_FB_*
 * framebuffer API. A genuine ring-3 program making real int 0x80
 * calls against the real kernel (kernel/task/exec_trust_demo.c runs it
 * once per display backend and tools/python/test_runner.py checks the
 * "[gfxtest] PASS" lines), the in-OS counterpart of the host tests in
 * tools/tests/ - those prove the arithmetic against oracles, this
 * proves the whole stack: syscall -> validation -> surface memory ->
 * backend -> device -> and back out through readback.
 *
 *   GFXTEST.ELF [auto|vbe|gpu] [leak] | show [seconds]
 *
 *   (default)  run the full conformance suite on the chosen backend
 *   leak       acquire the display, draw, and exit WITHOUT releasing -
 *              the process-exit hook must hand the display back
 *   show       paint a test card, hold it for N seconds (default 20),
 *              then release - for looking at the real output (e.g. via
 *              the QEMU monitor's screendump); runs no tests
 *
 * HOW THE DISPLAY CONTENT IS CHECKED. The program keeps a MODEL of what
 * the display must contain. Every operation is applied to the model
 * with deliberately dumb per-pixel loops (no clipping formulas shared
 * with the kernel or with novagfx.h), and after each step the ENTIRE
 * display is read back through SYS_FB_READBACK and compared to the
 * model. That one comparison checks "the right pixels changed" and
 * "nothing else did" at the same time, for every operation - clipping,
 * damage rectangles, off-screen presents, unpresented changes - without
 * a hand-written expectation per case.
 *
 * Every failure prints "[gfxtest] FAIL: ..." (the uppercase word also
 * trips test_runner.py's global no-FAIL assertion, so a regression can
 * never slip through). Exit status 0 iff every check passed.
 */
#include <errno.h>
#include <novagfx.h>
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DW 1024 /* the display size this test expects (checked, below) */
#define DH 768

static int checks, failures;
static const char* backend_name = "auto";
static unsigned int backend_id = NOVA_FB_BACKEND_ANY;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 25) { \
            printf("[gfxtest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

/* A syscall result that must be exactly `want` (0, or a specific
 * negative errno). Prints the actual value on mismatch - "returned -22
 * not -14" is the whole diagnosis. */
#define EXPECT(call, want) do { \
    int rc_ = (call); \
    checks++; \
    if (rc_ != (want)) { \
        failures++; \
        if (failures <= 25) \
            printf("[gfxtest] FAIL: %s returned %d, expected %d (line %d)\n", \
                   #call, rc_, (int)(want), __LINE__); \
    } \
} while (0)

static int raw_syscall1(int number, unsigned int arg) {
    int r = number;
    __asm__ volatile ("int $0x80" : "+a"(r) : "b"(arg) : "memory", "cc");
    return r;
}

/* ---- patterns ------------------------------------------------------ *
 * The X byte is deliberately NONZERO: the ABI says it is ignored on
 * present and always 0 on readback, and a backend that lets its own
 * alpha convention (virtio-gpu forces 0xFF) leak back to the app would
 * fail the model comparison. */
static unsigned int pat_a(int x, int y) {
    return 0x5A000000u | NOVA_RGB((x * 7 + y) & 255, (y * 5 + 3) & 255,
                                  ((x ^ y) | 1) & 255);
}
static unsigned int pat_b(int x, int y) {
    return 0x5A000000u | NOVA_RGB(255 - ((x * 3) & 255), ((x + y * 2) | 1) & 255, 200);
}

static void paint(nova_surface_t* s, unsigned int (*fn)(int, int)) {
    for (unsigned int y = 0; y < s->height; y++)
        for (unsigned int x = 0; x < s->width; x++)
            *nova_surface_pixel(s, x, y) = fn((int)x, (int)y);
}

/* ---- the model ------------------------------------------------------ */
static unsigned int* model;   /* DW*DH, XRGB with X = 0, row stride DW*4 */
static unsigned int* readbuf; /* scratch the display is read back into */

static void model_clear(void) {
    memset(model, 0, (size_t)DW * DH * 4);
}

/* What presenting rectangle (sx,sy,w,h) of `s` at (dx,dy) must do to
 * the display: for each source pixel, find where it lands; keep it if
 * that spot is on the display. No clipping formulas - containment only. */
static void model_present(const nova_surface_t* s, int sx, int sy, int w, int h,
                          int dx, int dy) {
    for (int yy = 0; yy < h; yy++) {
        for (int xx = 0; xx < w; xx++) {
            int X = dx + xx, Y = dy + yy;
            if (X >= 0 && X < DW && Y >= 0 && Y < DH) {
                model[(unsigned)Y * DW + (unsigned)X] =
                    *nova_surface_pixel(s, (unsigned)(sx + xx), (unsigned)(sy + yy))
                    & 0x00FFFFFFu;
            }
        }
    }
}

static int display_matches_model(const char* step) {
    int rc = nova_display_readback(0, 0, DW, DH, readbuf, DW * 4);
    CHECK(rc == 0, "[%s] full-display readback returned %d", step, rc);
    if (rc != 0) {
        return 0;
    }
    long bad = 0;
    int fx = -1, fy = -1;
    unsigned int got = 0, want = 0;
    for (int y = 0; y < DH; y++) {
        for (int x = 0; x < DW; x++) {
            unsigned int g = readbuf[(unsigned)y * DW + (unsigned)x];
            unsigned int m = model[(unsigned)y * DW + (unsigned)x];
            if (g != m) {
                if (bad == 0) { fx = x; fy = y; got = g; want = m; }
                bad++;
            }
        }
    }
    CHECK(bad == 0, "[%s] display differs from model in %d pixels; first at "
          "(%d,%d): got 0x%x want 0x%x", step, (int)bad, fx, fy, got, want);
    return bad == 0;
}

/* ---- groups --------------------------------------------------------- */

static void group_done(const char* name, int failures_before) {
    if (failures == failures_before) {
        printf("[gfxtest] ok: %s\n", name);
    }
}

static void test_info(void) {
    int f0 = failures;
    nova_fb_info_t info;
    memset(&info, 0xAA, sizeof info);
    info.struct_size = sizeof info;
    EXPECT(sys_fb_info(&info), 0);
    CHECK(info.struct_size == sizeof info, "struct_size %u", info.struct_size);
    CHECK(info.format == NOVA_FB_FORMAT_XRGB8888, "format %u", info.format);
    CHECK(info.width == DW && info.height == DH, "display %ux%u, expected %dx%d",
          info.width, info.height, DW, DH);
    CHECK(info.backend == NOVA_FB_BACKEND_VBE || info.backend == NOVA_FB_BACKEND_VIRTIOGPU,
          "backend id %u", info.backend);
    CHECK(info.backend_mask & (1u << info.backend), "mask 0x%x lacks default backend %u",
          info.backend_mask, info.backend);
    if (backend_id != NOVA_FB_BACKEND_ANY) {
        CHECK(info.backend_mask & (1u << backend_id),
              "requested backend '%s' not in mask 0x%x", backend_name, info.backend_mask);
    }
    CHECK((info.caps & NOVA_FB_CAP_DAMAGE_PRESENT) && (info.caps & NOVA_FB_CAP_READBACK),
          "caps 0x%x", info.caps);
    CHECK(((info.caps & NOVA_FB_CAP_GPU_TRANSFER) != 0) ==
              (info.backend == NOVA_FB_BACKEND_VIRTIOGPU),
          "GPU_TRANSFER cap (0x%x) disagrees with backend %u", info.caps, info.backend);
    CHECK(info.max_surface_dim == NOVA_FB_MAX_SURFACE_DIM &&
          info.max_surfaces == NOVA_FB_MAX_SURFACES_PER_PROC, "limits %u/%u",
          info.max_surface_dim, info.max_surfaces);

    /* The struct_size handshake: a caller built against a SHORTER struct
     * gets exactly that much written... */
    unsigned int buf[16];
    for (int i = 0; i < 16; i++) buf[i] = 0xAAAAAAAAu;
    buf[0] = 4;
    EXPECT(sys_fb_info((nova_fb_info_t*)buf), 0);
    CHECK(buf[0] == 4 && buf[1] == 0xAAAAAAAAu, "short struct: wrote past 4 bytes");
    /* ...and one built against a LONGER (future) struct gets the fields
     * this kernel knows, with struct_size reporting how many that was. */
    for (int i = 0; i < 16; i++) buf[i] = 0xAAAAAAAAu;
    buf[0] = 64;
    EXPECT(sys_fb_info((nova_fb_info_t*)buf), 0);
    CHECK(buf[0] == sizeof(nova_fb_info_t) && buf[9] == 0xAAAAAAAAu && buf[15] == 0xAAAAAAAAu,
          "long struct: size %u, tail touched", buf[0]);
    buf[0] = 0;
    EXPECT(sys_fb_info((nova_fb_info_t*)buf), -EINVAL);
    buf[0] = 3;
    EXPECT(sys_fb_info((nova_fb_info_t*)buf), -EINVAL);
    group_done("info (incl. struct_size handshake)", f0);
}

static void test_bad_pointers(void) {
    int f0 = failures;
    /* Every one of these used to be a way to panic the whole machine
     * (a kernel-mode fault is fatal here). Now each must come back as
     * -EFAULT and the system must carry on. */
    void* nowhere[] = {
        (void*)0,           /* NULL */
        (void*)0x00100000,  /* kernel image: mapped, but not user-accessible */
        (void*)0x70000000,  /* user range, never mapped */
        (void*)0xFFFFFFFE,  /* a 4-byte read here wraps past 4GB */
    };
    for (unsigned i = 0; i < sizeof nowhere / sizeof nowhere[0]; i++) {
        void* p = nowhere[i];
        EXPECT(sys_fb_info((nova_fb_info_t*)p), -EFAULT);
        EXPECT(sys_fb_create((nova_fb_create_t*)p), -EFAULT);
        EXPECT(sys_fb_present((const nova_fb_present_t*)p), -EFAULT);
        EXPECT(sys_fb_readback((const nova_fb_readback_t*)p), -EFAULT);
    }
    /* A struct that STARTS in mapped memory but runs off the end of it:
     * the check has to cover every page, not just the first. The heap's
     * mapped end is page-aligned, so a pointer 8 bytes short of it
     * straddles into an unmapped page. */
    unsigned int brk = (unsigned int)(unsigned long)sys_sbrk(0);
    unsigned int edge = (brk + 4095u) & ~4095u;
    EXPECT(sys_fb_create((nova_fb_create_t*)(unsigned long)(edge - 8)), -EFAULT);
    EXPECT(sys_fb_info((nova_fb_info_t*)(unsigned long)(edge - 2)), -EFAULT);

    /* The two legacy graphics calls that used to dereference user
     * pointers unchecked. Neither may fault; mouse_read reports "no
     * data" (0). (fill_rect returns void, so surviving it IS the test.) */
    raw_syscall1(SYS_GFX_FILL_RECT, 0x00100000u);
    raw_syscall1(SYS_GFX_FILL_RECT, 0);
    raw_syscall1(SYS_GFX_FILL_RECT, 0x70000000u);
    CHECK(raw_syscall1(SYS_MOUSE_READ, 0x00100000u) == 0, "mouse_read(kernel ptr)");
    CHECK(raw_syscall1(SYS_MOUSE_READ, 0) == 0, "mouse_read(NULL)");
    CHECK(raw_syscall1(SYS_MOUSE_READ, 0x70000000u) == 0, "mouse_read(unmapped)");
    /* (Worded to avoid the uppercase words test_runner.py's global
     * no-panic assertion greps for: the errno is literally spelled
     * EFAULT, and a PASS line must not trip a check meant to catch
     * real failures. That check is deliberately blunt; this is the
     * cheaper side to bend.) */
    group_done("hostile pointers rejected as bad-address errors, kernel unharmed", f0);
}

static void test_not_owned_and_bad_args(void) {
    int f0 = failures;
    nova_fb_present_t pr = { 1, 0, 0, 8, 8, 0, 0, 0 };
    nova_fb_readback_t rb = { 0, 0, 8, 8, (unsigned int)(unsigned long)readbuf, 32, 0 };
    EXPECT(sys_fb_present(&pr), -EPERM);
    EXPECT(sys_fb_readback(&rb), -EPERM);
    EXPECT(sys_fb_release(), -EPERM);
    EXPECT(sys_fb_destroy(0), -EBADF);
    EXPECT(sys_fb_destroy(0xFFFFFFFFu), -EBADF);
    EXPECT(sys_fb_destroy(0x00000101u), -EBADF);
    EXPECT(sys_fb_acquire(3), -EINVAL);
    EXPECT(sys_fb_acquire(99), -EINVAL);

    struct { unsigned w, h, flags; } bad[] = {
        {0, 5, 0}, {5, 0, 0}, {2049, 5, 0}, {5, 2049, 0},
        {0xFFFFFFFFu, 0xFFFFFFFFu, 0}, {0xFFFFFFFFu, 1, 0},
        {16, 16, 1}, {16, 16, 0x80000000u},
    };
    for (unsigned i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        nova_fb_create_t req = { bad[i].w, bad[i].h, bad[i].flags, 0, 0, 0, 0 };
        CHECK(sys_fb_create(&req) == -EINVAL, "create %ux%u flags %u should be EINVAL",
              bad[i].w, bad[i].h, bad[i].flags);
    }
    group_done("ownership + argument errors", f0);
}

static void test_limits_zeroing_generations(void) {
    int f0 = failures;
    nova_surface_t tiny[NOVA_FB_MAX_SURFACES_PER_PROC + 1];
    unsigned int made = 0;
    for (; made < NOVA_FB_MAX_SURFACES_PER_PROC; made++) {
        int rc = nova_surface_create(&tiny[made], 16, 16);
        CHECK(rc == 0, "tiny surface %u: %d", made, rc);
        if (rc != 0) break;
    }
    CHECK(nova_surface_create(&tiny[made], 16, 16) == -ENOSPC,
          "the %uth surface should hit the per-process count limit",
          NOVA_FB_MAX_SURFACES_PER_PROC + 1);
    /* Distinct virtual slots, page-aligned, never overlapping. */
    for (unsigned int i = 0; i < made; i++) {
        CHECK(((unsigned long)tiny[i].pixels & 0xFFF) == 0, "surface %u not page aligned", i);
        CHECK(tiny[i].stride == 64, "stride %u", tiny[i].stride);
        for (unsigned int j = i + 1; j < made; j++) {
            CHECK(tiny[i].pixels != tiny[j].pixels && tiny[i].handle != tiny[j].handle,
                  "surfaces %u and %u collide", i, j);
        }
    }
    for (unsigned int i = 0; i < made; i++) {
        EXPECT(nova_surface_destroy(&tiny[i]), 0);
    }

    /* New surfaces are ZEROED. The test that makes this mean something:
     * fill a surface with a pattern, destroy it (its frames go back to
     * the allocator, which hands the lowest free frame out first, so
     * the next create gets those very frames), create again - and the
     * old contents must be gone. Without the kernel zeroing them they
     * would be the previous owner's pixels. */
    nova_surface_t a, b;
    EXPECT(nova_surface_create(&a, 256, 256), 0);
    for (unsigned int y = 0; y < 256; y++)
        for (unsigned int x = 0; x < 256; x++)
            *nova_surface_pixel(&a, x, y) = 0xDEADBEEFu;
    EXPECT(nova_surface_destroy(&a), 0);
    EXPECT(nova_surface_create(&b, 256, 256), 0);
    long nonzero = 0;
    for (unsigned int y = 0; y < 256; y++)
        for (unsigned int x = 0; x < 256; x++)
            if (*nova_surface_pixel(&b, x, y) != 0) nonzero++;
    CHECK(nonzero == 0, "a fresh surface held %d non-zero pixels of a previous "
          "surface's data", (int)nonzero);

    /* A handle outlives nothing: destroy it, reuse its slot, and the
     * old handle must NOT alias the new surface. */
    unsigned int old_handle = b.handle;
    EXPECT(nova_surface_destroy(&b), 0);
    EXPECT(sys_fb_destroy(old_handle), -EBADF); /* double destroy */
    nova_surface_t c;
    EXPECT(nova_surface_create(&c, 256, 256), 0);
    CHECK(c.handle != old_handle, "reused slot reissued the identical handle 0x%x", c.handle);
    EXPECT(sys_fb_destroy(old_handle), -EBADF);
    EXPECT(nova_surface_destroy(&c), 0);

    /* The per-process BYTE limit (16MB), if memory allows the test:
     * two 8MB surfaces fit exactly, a third of any size does not. */
    nova_surface_t big1, big2, extra;
    int r1 = nova_surface_create(&big1, 2048, 1024);
    int r2 = (r1 == 0) ? nova_surface_create(&big2, 2048, 1024) : r1;
    if (r1 == 0 && r2 == 0) {
        CHECK(nova_surface_create(&extra, 16, 16) == -ENOSPC,
              "a surface past the 16MB per-process byte limit");
        EXPECT(nova_surface_destroy(&big2), 0);
    } else {
        /* Not enough free low memory right now - report, don't guess. */
        printf("[gfxtest] note: byte-limit check skipped (create returned %d/%d)\n", r1, r2);
    }
    if (r1 == 0) {
        EXPECT(nova_surface_destroy(&big1), 0);
    }
    group_done("limits, zeroed memory, handle generations", f0);
}

/* Page-aligned, in .bss, untouched by this program until fork(): the
 * child's syscalls write results into these while the page is still
 * shared copy-on-write with the parent. (A stack variable would not do:
 * the child privatises its stack pages the moment it calls anything.) */
static nova_fb_info_t cow_info __attribute__((aligned(4096)));
static nova_fb_create_t cow_create __attribute__((aligned(4096)));

static void test_cow_and_exit_hook(void) {
    int f0 = failures;

    /* The kernel runs with CR0.WP clear, so a ring-0 store into a
     * read-only COW page does NOT fault - it silently writes the frame
     * every sharer sees. If a syscall's output went through that, a
     * child calling sys_fb_info() would rewrite its PARENT's memory.
     * The parent plants sentinels, forks, lets the child make the
     * syscalls, and checks its own copy is untouched. */
    cow_info.struct_size = sizeof cow_info;
    cow_info.width = 0xDEADBEEFu;
    cow_create.width = 8; cow_create.height = 8; cow_create.flags = 0;
    cow_create.handle = 0xDEADBEEFu;
    int pid = sys_fork();
    if (pid == 0) {
        int ok = sys_fb_info(&cow_info) == 0 && cow_info.width == DW &&
                 sys_fb_create(&cow_create) == 0 && cow_create.handle != 0 &&
                 cow_create.handle != 0xDEADBEEFu;
        sys_exit(ok ? 0 : 7);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    int child_rc = sys_wait(pid);
    CHECK(child_rc == 0, "child's syscalls into COW pages failed (exit %d)", child_rc);
    CHECK(cow_info.width == 0xDEADBEEFu,
          "child's sys_fb_info() wrote through to the PARENT's page (width=0x%x)",
          cow_info.width);
    CHECK(cow_create.handle == 0xDEADBEEFu,
          "child's sys_fb_create() wrote through to the PARENT's page (handle=0x%x)",
          cow_create.handle);

    /* A process that exits holding the display must not strand it. */
    pid = sys_fork();
    if (pid == 0) {
        nova_surface_t s;
        int ok = sys_fb_acquire(backend_id) == 0 && nova_surface_create(&s, 64, 64) == 0;
        if (ok) {
            nova_surface_fill_rect(&s, 0, 0, 64, 64, NOVA_RGB(255, 0, 255));
            ok = nova_surface_present_full(&s, 10, 10) == 0;
        }
        sys_exit(ok ? 0 : 8); /* deliberately NO sys_fb_release() */
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    child_rc = sys_wait(pid);
    CHECK(child_rc == 0, "child could not acquire/draw (exit %d)", child_rc);
    /* If the exit hook were missing this would be -EBUSY forever. */
    int rc = sys_fb_acquire(backend_id);
    CHECK(rc == 0, "display still owned after its owner exited: acquire -> %d", rc);
    if (rc == 0) {
        EXPECT(sys_fb_release(), 0);
    }
    group_done("fork + COW-safe syscall output; exit hook releases the display", f0);

    /* The ordering guarantee behind that hook, stress-tested: "wait()
     * returned" must imply "the child's display is already free". The
     * parent and child run on different CPUs, so a single round only
     * loses the race some of the time (it did, 1 run in 3, when the
     * hook ran AFTER the process was marked terminated); a tight loop
     * of rounds makes a regression fail essentially every time. Each
     * round: child acquires and exits without releasing; the parent
     * waits, then must be able to acquire at once. */
    int f1 = failures;
    int busy = 0, rounds_done = 0;
    for (int round = 0; round < 200; round++) {
        int cpid = sys_fork();
        if (cpid == 0) {
            sys_exit(sys_fb_acquire(backend_id) == 0 ? 0 : 9);
        }
        if (cpid < 0) {
            CHECK(0, "fork failed in stress round %d: %d", round, cpid);
            break;
        }
        int crc = sys_wait(cpid);
        int arc = sys_fb_acquire(backend_id);
        if (crc != 0 || arc != 0) {
            busy++;
        }
        if (arc == 0) {
            sys_fb_release();
        }
        rounds_done++;
    }
    CHECK(busy == 0, "%d of %d rounds: the display was still owned after wait() "
          "returned (exit hook ordering)", busy, rounds_done);
    group_done("exit-hook ordering under stress (200 rounds)", f1);
}

static void test_display(void) {
    int f0 = failures;
    nova_surface_t s;

    EXPECT(sys_fb_acquire(backend_id), 0);
    EXPECT(sys_fb_acquire(backend_id), -EBUSY);
    EXPECT(sys_fb_acquire(NOVA_FB_BACKEND_VBE), -EBUSY);

    EXPECT(nova_surface_create(&s, 512, 384), 0);
    CHECK(s.pixels == (unsigned int*)0x60000000u, "first surface at %x", (unsigned)(unsigned long)s.pixels);

    /* Fork before drawing: the surface's pages become copy-on-write in
     * the parent. Drawing then privatises them page by page, and the
     * kernel must present what the parent's mapping holds NOW, not the
     * stale shared frame. (All the forks are up here, while the process
     * is still small - see main().) */
    int pid = sys_fork();
    if (pid == 0) { sys_exit(0); }
    CHECK(pid > 0, "fork failed: %d", pid);
    CHECK(sys_wait(pid) == 0, "child");

    /* Destroying a surface that is still copy-on-write shared with a
     * fork()'d child must not free frames the child may still map. */
    nova_surface_t cow;
    EXPECT(nova_surface_create(&cow, 64, 64), 0);
    pid = sys_fork();
    if (pid == 0) { sys_exit(0); }
    CHECK(pid > 0, "fork failed: %d", pid);
    CHECK(sys_wait(pid) == 0, "child");
    EXPECT(nova_surface_destroy(&cow), 0);
    nova_surface_t after;
    EXPECT(nova_surface_create(&after, 64, 64), 0);
    EXPECT(nova_surface_destroy(&after), 0);

    /* No more forks from here on: now the big buffers. */
    model = malloc((size_t)DW * DH * 4);
    readbuf = malloc((size_t)DW * DH * 4);
    if (model == NULL || readbuf == NULL) {
        CHECK(0, "out of memory for the %d-byte model buffers", DW * DH * 4);
        return;
    }
    memset(readbuf, 0, (size_t)DW * DH * 4);

    /* Acquire clears the display: whatever the previous owner (the
     * leaking child above drew magenta) left must not be visible. */
    model_clear();
    display_matches_model("fresh acquire is black");

    paint(&s, pat_a);
    EXPECT(nova_surface_present_full(&s, 100, 50), 0);
    model_present(&s, 0, 0, 512, 384, 100, 50);
    display_matches_model("full present at (100,50) after fork+draw");

    /* Damage rectangle: only the changed part is presented. */
    for (int y = 30; y < 90; y++)
        for (int x = 20; x < 120; x++) *nova_surface_pixel(&s, (unsigned)x, (unsigned)y) = pat_b(x, y);
    EXPECT(nova_surface_present(&s, 20, 30, 100, 60, 120, 80), 0);
    model_present(&s, 20, 30, 100, 60, 120, 80);
    display_matches_model("damage rect only");

    /* The display shows what was PRESENTED, not what the surface holds
     * now: change pixels and do not present - the display must not move. */
    nova_surface_fill_rect(&s, 300, 200, 50, 50, NOVA_RGB(0, 255, 0));
    display_matches_model("unpresented change must stay invisible");

    /* Source offset and destination offset are independent. */
    EXPECT(nova_surface_present(&s, 5, 7, 64, 64, 700, 600), 0);
    model_present(&s, 5, 7, 64, 64, 700, 600);
    display_matches_model("sub-rect moved to (700,600)");

    /* Hanging off each edge: clipped, picture does not shift. */
    EXPECT(nova_surface_present_full(&s, -100, -50), 0);
    model_present(&s, 0, 0, 512, 384, -100, -50);
    display_matches_model("clipped left/top");
    EXPECT(nova_surface_present_full(&s, 900, 700), 0);
    model_present(&s, 0, 0, 512, 384, 900, 700);
    display_matches_model("clipped right/bottom");
    EXPECT(nova_surface_present(&s, 0, 0, 1, 1, 1023, 767), 0);
    model_present(&s, 0, 0, 1, 1, 1023, 767);
    display_matches_model("last pixel of the display");
    EXPECT(nova_surface_present_full(&s, -511, 0), 0); /* one column visible */
    model_present(&s, 0, 0, 512, 384, -511, 0);
    display_matches_model("one column visible at the left edge");

    /* Entirely off-screen: succeeds, does nothing. */
    EXPECT(nova_surface_present_full(&s, 5000, 5000), 0);
    EXPECT(nova_surface_present_full(&s, -512, 0), 0);
    EXPECT(nova_surface_present_full(&s, 1024, 0), 0);
    EXPECT(nova_surface_present_full(&s, 0, 768), 0);
    EXPECT(nova_surface_present_full(&s, 0, -384), 0);
    display_matches_model("off-screen presents change nothing");

    /* Caller bugs are rejected, and rejected without drawing anything. */
    nova_fb_present_t p;
    #define PRESENT_BAD(sx_, sy_, w_, h_, flags_, handle_, want_) do { \
        p.handle = (handle_); p.src_x = (sx_); p.src_y = (sy_); p.width = (w_); \
        p.height = (h_); p.dst_x = 0; p.dst_y = 0; p.flags = (flags_); \
        EXPECT(sys_fb_present(&p), (want_)); } while (0)
    PRESENT_BAD(0, 0, 513, 384, 0, s.handle, -EINVAL);   /* too wide */
    PRESENT_BAD(1, 0, 512, 384, 0, s.handle, -EINVAL);   /* runs 1px past the edge */
    PRESENT_BAD(-1, 0, 8, 8, 0, s.handle, -EINVAL);      /* negative origin */
    PRESENT_BAD(0, 0, 0, 8, 0, s.handle, -EINVAL);       /* zero size */
    PRESENT_BAD(0, 0, 8, -5, 0, s.handle, -EINVAL);      /* negative size */
    PRESENT_BAD(0, 0, 2147483647, 8, 0, s.handle, -EINVAL); /* x+w would wrap */
    PRESENT_BAD(2147483647, 0, 8, 8, 0, s.handle, -EINVAL);
    PRESENT_BAD(0, 0, 8, 8, 1, s.handle, -EINVAL);       /* reserved flags */
    PRESENT_BAD(0, 0, 8, 8, 0, 0, -EBADF);               /* no such handle */
    PRESENT_BAD(0, 0, 8, 8, 0, s.handle ^ 0x100, -EBADF);/* wrong generation */
    PRESENT_BAD(0, 0, 8, 8, 0, s.handle + 1, -EBADF);    /* wrong slot */
    #undef PRESENT_BAD
    display_matches_model("rejected presents draw nothing");

    /* Readback argument checking. */
    nova_fb_readback_t rb;
    #define READBACK_BAD(x_, y_, w_, h_, dst_, stride_, want_) do { \
        rb.x = (x_); rb.y = (y_); rb.width = (w_); rb.height = (h_); \
        rb.dst = (unsigned int)(unsigned long)(dst_); rb.dst_stride = (stride_); rb.flags = 0; \
        EXPECT(sys_fb_readback(&rb), (want_)); } while (0)
    READBACK_BAD(0, 0, DW + 1, 8, readbuf, DW * 4 + 4, -EINVAL);   /* off the right */
    READBACK_BAD(1, 0, DW, 8, readbuf, DW * 4, -EINVAL);
    READBACK_BAD(0, 0, 8, DH + 1, readbuf, 64, -EINVAL);
    READBACK_BAD(-1, 0, 8, 8, readbuf, 64, -EINVAL);
    READBACK_BAD(0, 0, 0, 8, readbuf, 64, -EINVAL);
    READBACK_BAD(0, 0, 16, 4, readbuf, 63, -EINVAL);               /* stride < row */
    READBACK_BAD(0, 0, 16, 4, readbuf, 0xFFFFFFFFu, -EINVAL);      /* span overflows */
    READBACK_BAD(0, 0, 1, 3, readbuf, 0x80000000u, -EINVAL);       /* (h-1)*stride wraps */
    READBACK_BAD(0, 0, 8, 8, 0x00100000u, 32, -EFAULT);            /* dst in the kernel */
    READBACK_BAD(0, 0, 8, 8, 0, 32, -EFAULT);                      /* dst NULL */
    READBACK_BAD(0, 0, 8, 8, 0x70000000u, 32, -EFAULT);            /* dst unmapped */
    {
        /* Runs off the end of MAPPED memory (not just off the end of a
         * malloc block, which is usually followed by more mapped heap):
         * 16 bytes short of the heap's page-aligned mapped end, with a
         * 256-byte span, so the last pages of the buffer are unmapped. */
        unsigned int brk = (unsigned int)(unsigned long)sys_sbrk(0);
        unsigned int edge = (brk + 4095u) & ~4095u;
        READBACK_BAD(0, 0, 8, 8, (unsigned long)(edge - 16), 32, -EFAULT);
    }
    rb.flags = 1;
    rb.x = 0; rb.y = 0; rb.width = 8; rb.height = 8;
    rb.dst = (unsigned int)(unsigned long)readbuf; rb.dst_stride = 32;
    EXPECT(sys_fb_readback(&rb), -EINVAL);
    #undef READBACK_BAD
    display_matches_model("rejected readbacks");

    /* The old SYS_GFX_* calls are ignored while this API owns the
     * display - one of them would otherwise scribble (0,0) white. */
    sys_gfx_put_pixel(0, 0, 15);
    sys_gfx_fill_rect(0, 0, 50, 50, 15);
    sys_gfx_enter();
    sys_gfx_exit();
    display_matches_model("legacy SYS_GFX_* ignored while the display is owned");

    /* A full-size surface: every pixel of a 1024x768 display. */
    nova_surface_t full;
    EXPECT(nova_surface_create(&full, DW, DH), 0);
    paint(&full, pat_b);
    EXPECT(nova_surface_present_full(&full, 0, 0), 0);
    model_present(&full, 0, 0, DW, DH, 0, 0);
    display_matches_model("full-screen present of a 1024x768 surface");

    EXPECT(nova_surface_destroy(&full), 0);
    EXPECT(nova_surface_destroy(&s), 0);

    /* Release, then re-acquire: presents are refused in between, and the
     * new ownership starts from black - the previous frame must not
     * survive into it. */
    EXPECT(sys_fb_release(), 0);
    EXPECT(sys_fb_release(), -EPERM);
    nova_fb_present_t pr = { 1, 0, 0, 1, 1, 0, 0, 0 };
    EXPECT(sys_fb_present(&pr), -EPERM);
    EXPECT(sys_fb_acquire(backend_id), 0);
    model_clear();
    display_matches_model("re-acquire starts from black (previous frame gone)");
    EXPECT(sys_fb_release(), 0);

    group_done("display: present/damage/clip/readback/legacy-guard/acquire-clear", f0);
}

/* "show": a test card, held for a while, for looking at with a real
 * eye (or the QEMU monitor's screendump). Not a test. */
static int run_show(int seconds) {
    nova_surface_t s;
    if (nova_surface_create(&s, DW, DH) != 0 || sys_fb_acquire(backend_id) != 0) {
        printf("[gfxtest] show: could not set up the display\n");
        return 1;
    }
    /* Vertical colour bars on top, a horizontal gradient below, a white
     * border, and two diagonals - enough to see colour order, pixel
     * format, orientation and that the whole 1024x768 is addressed. */
    static const unsigned int bars[8] = {
        0xFFFFFF, 0xFFFF00, 0x00FFFF, 0x00FF00, 0xFF00FF, 0xFF0000, 0x0000FF, 0x000000 };
    for (int y = 0; y < DH; y++) {
        for (int x = 0; x < DW; x++) {
            unsigned int c;
            if (y < DH * 2 / 3) c = bars[x * 8 / DW];
            else { unsigned int g = (unsigned)(x * 255 / (DW - 1)); c = NOVA_RGB(g, g, g); }
            *nova_surface_pixel(&s, (unsigned)x, (unsigned)y) = c;
        }
    }
    for (int i = 0; i < DW; i++) {
        nova_surface_put_pixel(&s, i, i * DH / DW, NOVA_RGB(255, 128, 0));
        nova_surface_put_pixel(&s, i, DH - 1 - i * DH / DW, NOVA_RGB(0, 128, 255));
    }
    nova_surface_fill_rect(&s, 0, 0, DW, 4, 0xFFFFFF);
    nova_surface_fill_rect(&s, 0, DH - 4, DW, 4, 0xFFFFFF);
    nova_surface_fill_rect(&s, 0, 0, 4, DH, 0xFFFFFF);
    nova_surface_fill_rect(&s, DW - 4, 0, 4, DH, 0xFFFFFF);
    int rc = nova_surface_present_full(&s, 0, 0);
    printf("[gfxtest] show: test card presented (rc=%d), holding %d s\n", rc, seconds);
    nova_rtc_time_t t0, t;
    sys_rtc_read(&t0);
    int start = t0.minute * 60 + t0.second;
    for (;;) {
        sys_yield();
        sys_rtc_read(&t);
        int now = t.minute * 60 + t.second;
        int elapsed = now - start;
        if (elapsed < 0) elapsed += 3600;
        if (elapsed >= seconds) break;
    }
    sys_fb_release();
    nova_surface_destroy(&s);
    return rc == 0 ? 0 : 1;
}

int main(int argc, char** argv) {
    int leak = 0, show = 0, show_seconds = 20;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "auto") == 0) { backend_name = "auto"; backend_id = NOVA_FB_BACKEND_ANY; }
        else if (strcmp(argv[i], "vbe") == 0) { backend_name = "vbe"; backend_id = NOVA_FB_BACKEND_VBE; }
        else if (strcmp(argv[i], "gpu") == 0) { backend_name = "gpu"; backend_id = NOVA_FB_BACKEND_VIRTIOGPU; }
        else if (strcmp(argv[i], "leak") == 0) { leak = 1; }
        else if (strcmp(argv[i], "show") == 0) { show = 1; if (i + 1 < argc) show_seconds = atoi(argv[++i]); }
        else { printf("usage: GFXTEST.ELF [auto|vbe|gpu] [leak] | show [seconds]\n"); return 2; }
    }
    if (show) {
        return run_show(show_seconds);
    }

    if (leak) {
        nova_surface_t s;
        int ok = sys_fb_acquire(backend_id) == 0 && nova_surface_create(&s, 32, 32) == 0;
        if (ok) {
            nova_surface_fill_rect(&s, 0, 0, 32, 32, NOVA_RGB(255, 128, 0));
            ok = nova_surface_present_full(&s, 0, 0) == 0;
        }
        printf("[gfxtest] leak: %s - exiting WITHOUT releasing the display\n", ok ? "drew" : "FAIL: setup failed");
        return ok ? 0 : 1;
    }

    /* The model/readback buffers (6MB) are allocated INSIDE test_display(),
     * after every fork() the suite does. A fork()'d parent leaks the pages
     * it later rewrites (copy-on-write frames are never freed - a
     * documented limitation, see PROGRESS.md), so forking while holding
     * megabytes of constantly-rewritten buffers would leak megabytes per
     * fork and exhaust memory over three back-to-back suite runs; forking
     * while small leaks almost nothing. */
    test_info();
    test_bad_pointers();
    test_not_owned_and_bad_args();
    test_limits_zeroing_generations();
    test_cow_and_exit_hook();
    test_display();

    if (failures == 0) {
        printf("[gfxtest] PASS: backend=%s - the full SYS_FB_* contract holds (%d checks)\n",
               backend_name, checks);
        return 0;
    }
    printf("[gfxtest] FAIL: backend=%s - %d of %d checks failed\n", backend_name, failures, checks);
    return 1;
}
