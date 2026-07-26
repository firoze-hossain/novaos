/*
 * compositor.c - fixed-count draggable rectangle windows over a
 * double-buffered VGA Mode 13h framebuffer
 */
#include "compositor.h"
#include "font5x7.h"
#include "../drivers/video/vga_graphics.h"
#include "../lib/string.h"

#define TITLEBAR_HEIGHT 10
#define CURSOR_SIZE 7

typedef struct {
    int x, y, w, h;
    uint8_t body_color;
    uint8_t titlebar_color;
    int label; /* 1-9, rendered via font5x7_digits[label] */
} window_t;

static window_t windows[COMPOSITOR_MAX_WINDOWS];

static int cursor_x = VGA_GFX_WIDTH / 2;
static int cursor_y = VGA_GFX_HEIGHT / 2;

static int dragging_window = -1; /* index into windows[], or -1 */
static int drag_offset_x = 0;
static int drag_offset_y = 0;

/* Back buffer: rendered into off-screen, then blitted to 0xA0000 in
 * one memcpy per frame - avoids the visible tearing/flicker that
 * drawing each shape directly to the live framebuffer would cause
 * while the CRT is mid-scan. */
static uint8_t backbuffer[VGA_GFX_WIDTH * VGA_GFX_HEIGHT];

static void bb_put_pixel(int x, int y, uint8_t color) {
    if (x < 0 || y < 0 || x >= VGA_GFX_WIDTH || y >= VGA_GFX_HEIGHT) {
        return;
    }
    backbuffer[y * VGA_GFX_WIDTH + x] = color;
}

static void bb_fill_rect(int x, int y, int w, int h, uint8_t color) {
    for (int row = y; row < y + h; row++) {
        for (int col = x; col < x + w; col++) {
            bb_put_pixel(col, row, color);
        }
    }
}

static void bb_draw_rect(int x, int y, int w, int h, uint8_t color) {
    for (int col = x; col < x + w; col++) {
        bb_put_pixel(col, y, color);
        bb_put_pixel(col, y + h - 1, color);
    }
    for (int row = y; row < y + h; row++) {
        bb_put_pixel(x, row, color);
        bb_put_pixel(x + w - 1, row, color);
    }
}

static void bb_draw_digit(int x, int y, int digit, uint8_t color) {
    if (digit < 0 || digit > 9) {
        return;
    }
    for (int row = 0; row < 7; row++) {
        uint8_t bits = font5x7_digits[digit][row];
        for (int col = 0; col < 5; col++) {
            if (bits & (uint8_t)(0x10 >> col)) {
                bb_put_pixel(x + col, y + row, color);
            }
        }
    }
}

static void bb_draw_cursor(int x, int y) {
    /* A simple filled arrow, drawn one row at a time getting narrower
     * - a small, self-contained shape, not a transcribed bitmap. */
    const uint8_t CURSOR_COLOR = 15; /* white */
    for (int row = 0; row < CURSOR_SIZE; row++) {
        for (int col = 0; col <= row; col++) {
            bb_put_pixel(x + col, y + row, CURSOR_COLOR);
        }
    }
}

void compositor_init(void) {
    windows[0] = (window_t){.x = 20,  .y = 20, .w = 100, .h = 70,
                             .body_color = 3,  .titlebar_color = 1,
                             .label = 1};
    windows[1] = (window_t){.x = 140, .y = 40, .w = 100, .h = 70,
                             .body_color = 10, .titlebar_color = 2,
                             .label = 2};
    windows[2] = (window_t){.x = 80,  .y = 110, .w = 100, .h = 70,
                             .body_color = 14, .titlebar_color = 6,
                             .label = 3};

    cursor_x = VGA_GFX_WIDTH / 2;
    cursor_y = VGA_GFX_HEIGHT / 2;
    dragging_window = -1;
}

static bool point_in_rect(int px, int py, int x, int y, int w, int h) {
    return px >= x && px < x + w && py >= y && py < y + h;
}

void compositor_handle_mouse(mouse_state_t state) {
    cursor_x += state.dx;
    cursor_y += state.dy;
    if (cursor_x < 0) cursor_x = 0;
    if (cursor_y < 0) cursor_y = 0;
    if (cursor_x >= VGA_GFX_WIDTH) cursor_x = VGA_GFX_WIDTH - 1;
    if (cursor_y >= VGA_GFX_HEIGHT) cursor_y = VGA_GFX_HEIGHT - 1;

    if (state.left_button) {
        if (dragging_window == -1) {
            /* Topmost-drawn (highest index) window under the cursor's
             * titlebar wins, matching the draw order in render(). */
            for (int i = COMPOSITOR_MAX_WINDOWS - 1; i >= 0; i--) {
                if (point_in_rect(cursor_x, cursor_y, windows[i].x,
                                   windows[i].y, windows[i].w,
                                   TITLEBAR_HEIGHT)) {
                    dragging_window = i;
                    drag_offset_x = cursor_x - windows[i].x;
                    drag_offset_y = cursor_y - windows[i].y;
                    break;
                }
            }
        }
        if (dragging_window != -1) {
            windows[dragging_window].x = cursor_x - drag_offset_x;
            windows[dragging_window].y = cursor_y - drag_offset_y;
        }
    } else {
        dragging_window = -1;
    }
}

void compositor_render(void) {
    memset(backbuffer, 1, sizeof(backbuffer)); /* desktop: blue */

    for (int i = 0; i < COMPOSITOR_MAX_WINDOWS; i++) {
        window_t* w = &windows[i];
        bb_fill_rect(w->x, w->y + TITLEBAR_HEIGHT, w->w,
                     w->h - TITLEBAR_HEIGHT, w->body_color);
        bb_fill_rect(w->x, w->y, w->w, TITLEBAR_HEIGHT, w->titlebar_color);
        bb_draw_rect(w->x, w->y, w->w, w->h, 15 /* white border */);
        bb_draw_digit(w->x + 3, w->y + 1, w->label, 15);
    }

    bb_draw_cursor(cursor_x, cursor_y);

    /* One shot per frame, straight into VGA memory - see the file
     * header comment for why this beats drawing shapes directly to
     * 0xA0000 above. */
    for (int y = 0; y < VGA_GFX_HEIGHT; y++) {
        for (int x = 0; x < VGA_GFX_WIDTH; x++) {
            vga_put_pixel(x, y, backbuffer[y * VGA_GFX_WIDTH + x]);
        }
    }
}
