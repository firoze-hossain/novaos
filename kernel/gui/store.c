/*
 * store.c - a GUI front-end for nova-pkg
 */
#include "store.h"
#include "canvas.h"
#include "../drivers/video/vga_graphics.h"
#include "../drivers/keyboard/keyboard.h"
#include "../drivers/mouse/ps2mouse.h"
#include "../drivers/timer/timer.h"
#include "../drivers/vga/vga.h"
#include "../pkg/pkgmgr.h"
#include "../fs/vfs.h"
#include "../lib/string.h"

#define MAX_STORE_ROWS 8
#define ROW_HEIGHT 20
#define LIST_TOP 26
#define LIST_LEFT 8
#define LIST_WIDTH 300
#define BUTTON_WIDTH 56
#define BUTTON_HEIGHT 12

#define COLOR_BACKGROUND 1
#define COLOR_TITLEBAR   9
#define COLOR_TEXT       15
#define COLOR_ROW_BORDER 15
#define COLOR_INSTALL_BTN 2
#define COLOR_REMOVE_BTN  4

typedef struct {
    char name[PKG_NAME_MAX];
    char description[PKG_DESC_MAX];
    bool installed;
} store_row_t;

static store_row_t rows[MAX_STORE_ROWS];
static int row_count;

static uint8_t backbuffer[VGA_GFX_WIDTH * VGA_GFX_HEIGHT];

static int cursor_x, cursor_y;
static bool prev_left_button;

/* strncpy isn't in this freestanding libc subset (same libc-gaps note
 * as pkgmgr.c and shell.c's own local copies of this exact helper). */
static void strncpy_local(char* dest, const char* src, size_t size) {
    size_t i = 0;
    while (i < size - 1 && src[i]) {
        dest[i] = src[i];
        i++;
    }
    dest[i] = '\0';
}

static void collect_row(const char* filename, const char* name,
                         const char* version, const char* description,
                         bool installed) {
    (void)filename;
    (void)version;
    if (row_count >= MAX_STORE_ROWS) {
        return;
    }
    store_row_t* r = &rows[row_count];
    strncpy_local(r->name, name, sizeof(r->name));
    strncpy_local(r->description, description, sizeof(r->description));
    r->installed = installed;
    row_count++;
}

static void refresh_rows(void) {
    row_count = 0;
    pkg_list_available(collect_row);
}

static bool point_in_rect(int px, int py, int x, int y, int w, int h) {
    return px >= x && px < x + w && py >= y && py < y + h;
}

static void row_button_rect(int row_index, int* out_x, int* out_y) {
    *out_x = LIST_LEFT + LIST_WIDTH - BUTTON_WIDTH - 6;
    *out_y = LIST_TOP + row_index * ROW_HEIGHT +
             (ROW_HEIGHT - 2 - BUTTON_HEIGHT) / 2;
}

static void render(void) {
    memset(backbuffer, COLOR_BACKGROUND, sizeof(backbuffer));

    canvas_fill_rect(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT, 0, 0,
                      VGA_GFX_WIDTH, 16, COLOR_TITLEBAR);
    canvas_draw_text(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT, 6, 5,
                      "NOVAOS SOFTWARE CENTER", COLOR_TEXT);

    if (row_count == 0) {
        canvas_draw_text(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT, LIST_LEFT,
                          LIST_TOP, "NO PACKAGES FOUND", COLOR_TEXT);
    }

    for (int i = 0; i < row_count; i++) {
        int y = LIST_TOP + i * ROW_HEIGHT;
        canvas_draw_rect(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT, LIST_LEFT,
                          y, LIST_WIDTH, ROW_HEIGHT - 2, COLOR_ROW_BORDER);
        canvas_draw_text(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT,
                          LIST_LEFT + 4, y + 3, rows[i].name, COLOR_TEXT);

        int bx, by;
        row_button_rect(i, &bx, &by);
        uint8_t btn_color =
            rows[i].installed ? COLOR_REMOVE_BTN : COLOR_INSTALL_BTN;
        canvas_fill_rect(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT, bx, by,
                          BUTTON_WIDTH, BUTTON_HEIGHT, btn_color);
        canvas_draw_text(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT, bx + 2,
                          by + 3, rows[i].installed ? "REMOVE" : "INSTALL",
                          COLOR_TEXT);
    }

    /* Cursor: a small filled triangle, same shape convention as
     * kernel/gui/compositor.c's (a fresh copy rather than a shared
     * helper - see canvas.h's header comment on why this phase
     * doesn't refactor that file). */
    for (int row = 0; row < 7; row++) {
        for (int col = 0; col <= row; col++) {
            canvas_put_pixel(backbuffer, VGA_GFX_WIDTH, VGA_GFX_HEIGHT,
                              cursor_x + col, cursor_y + row, COLOR_TEXT);
        }
    }

    for (int y = 0; y < VGA_GFX_HEIGHT; y++) {
        for (int x = 0; x < VGA_GFX_WIDTH; x++) {
            vga_put_pixel(x, y, backbuffer[y * VGA_GFX_WIDTH + x]);
        }
    }
}

static void handle_click(void) {
    for (int i = 0; i < row_count; i++) {
        int bx, by;
        row_button_rect(i, &bx, &by);
        if (point_in_rect(cursor_x, cursor_y, bx, by, BUTTON_WIDTH,
                           BUTTON_HEIGHT)) {
            if (rows[i].installed) {
                pkg_remove(rows[i].name);
            } else {
                pkg_install(rows[i].name);
            }
            refresh_rows();
            return;
        }
    }
}

void store_run(void) {
    if (!ps2mouse_is_present()) {
        vga_puts("store: no PS/2 mouse detected - nothing to interact "
                  "with\n");
        return;
    }
    if (!vfs_is_mounted()) {
        vga_puts("store: no filesystem mounted - no packages to show\n");
        return;
    }

    vga_puts("Entering the Software Center. Click INSTALL/REMOVE; ESC to "
              "return to the shell.\n");

    vga_graphics_enter();
    cursor_x = VGA_GFX_WIDTH / 2;
    cursor_y = VGA_GFX_HEIGHT / 2;
    prev_left_button = false;
    refresh_rows();

    /* Discard mouse movement accumulated before this command ran - see
     * kernel/shell/shell.c's cmd_gui() for why this settling window
     * exists (a real bug found and fixed in Phase 7: a large stale
     * delta on entry threw off the cursor's starting position). */
    for (int i = 0; i < 20; i++) {
        (void)ps2mouse_read();
        timer_sleep_ms(10);
    }

    for (;;) {
        if (keyboard_has_char()) {
            char c = keyboard_get_char();
            if (c == 27) { /* ESC */
                break;
            }
        }

        mouse_state_t state = ps2mouse_read();
        cursor_x += state.dx;
        cursor_y += state.dy;
        if (cursor_x < 0) cursor_x = 0;
        if (cursor_y < 0) cursor_y = 0;
        if (cursor_x >= VGA_GFX_WIDTH) cursor_x = VGA_GFX_WIDTH - 1;
        if (cursor_y >= VGA_GFX_HEIGHT) cursor_y = VGA_GFX_HEIGHT - 1;

        if (state.left_button && !prev_left_button) {
            handle_click(); /* rising edge only - one click, one action */
        }
        prev_left_button = state.left_button;

        render();
        timer_sleep_ms(16);
    }

    vga_graphics_exit();
    vga_clear();
}
