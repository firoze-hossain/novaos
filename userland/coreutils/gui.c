/*
 * gui.c - a genuine ring-3 graphics demo (Phase 32b).
 *
 * Proof-of-concept for the graphics/mouse syscalls this phase added
 * (SYS_GFX_ENTER/EXIT/PUT_PIXEL/FILL_RECT, SYS_MOUSE_READ) - not a
 * port of the existing compositor's multi-window management or the
 * Store's package-browsing UI, which remain ring-0
 * (userland/gui/compositor.c, userland/gui/store.c) and are a
 * substantially larger undertaking left as honest follow-up work
 * (see PROGRESS.md). This draws a small, static scene (a few colored
 * "window"-like rectangles) to prove real ring-3 code can drive VGA
 * Mode 13h through syscalls alone, holds it on screen until a key is
 * pressed, and polls the mouse to prove SYS_MOUSE_READ works, before
 * cleanly returning to text mode - no kernel-side GUI-specific logic
 * is used at all.
 */
#include <novasys.h>
#include <stdio.h>

int main(int argc, char** argv, char** envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    printf("Entering graphics mode (VGA Mode 13h, 320x200)...\n");
    sys_gfx_enter();

    /* Background */
    sys_gfx_fill_rect(0, 0, 320, 200, 1); /* blue */

    /* Two "window"-like rectangles, proving fill_rect works at
     * different positions/sizes/colors. */
    sys_gfx_fill_rect(20, 20, 120, 80, 15);  /* white */
    sys_gfx_fill_rect(30, 30, 100, 20, 9);   /* light blue titlebar */
    sys_gfx_fill_rect(180, 60, 100, 90, 14); /* yellow */
    sys_gfx_fill_rect(190, 70, 80, 20, 6);   /* brown titlebar */

    /* A handful of individual pixels along a diagonal, proving
     * put_pixel works independently of fill_rect. */
    for (int i = 0; i < 60; i++) {
        sys_gfx_put_pixel(20 + i, 140 + i / 3, 4); /* red */
    }

    /* Poll the mouse a few times - proves SYS_MOUSE_READ works, even
     * without a full event loop reacting to it (a real interactive
     * compositor is out of scope for this proof-of-concept). */
    nova_mouse_state_t m;
    int mouse_present = sys_mouse_read(&m);

    /* Hold the scene until a key is pressed, using the same
     * SYS_READ_KEY+SYS_YIELD loop the shell's own input uses -
     * deliberately not a raw instruction-count busy-wait (the exact
     * timing anti-pattern Phase 31's USB fix corrected: duration
     * varies unpredictably with host CPU speed). */
    int key;
    while ((key = sys_read_key()) < 0) {
        sys_yield();
    }
    (void)key;

    sys_gfx_exit();

    if (mouse_present) {
        printf("Graphics demo complete. Mouse detected (last delta: "
               "dx=%d dy=%d).\n",
               m.dx, m.dy);
    } else {
        printf("Graphics demo complete. No PS/2 mouse detected.\n");
    }

    return 0;
}
