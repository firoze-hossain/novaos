/*
 * canvas.c - generic pixel-buffer drawing primitives (see canvas.h)
 */
#include "canvas.h"
#include "font5x7.h"

void canvas_put_pixel(uint8_t* buf, int width, int height, int x, int y,
                       uint8_t color) {
    if (x < 0 || y < 0 || x >= width || y >= height) {
        return;
    }
    buf[y * width + x] = color;
}

void canvas_fill_rect(uint8_t* buf, int width, int height, int x, int y,
                       int w, int h, uint8_t color) {
    for (int row = y; row < y + h; row++) {
        for (int col = x; col < x + w; col++) {
            canvas_put_pixel(buf, width, height, col, row, color);
        }
    }
}

void canvas_draw_rect(uint8_t* buf, int width, int height, int x, int y,
                       int w, int h, uint8_t color) {
    for (int col = x; col < x + w; col++) {
        canvas_put_pixel(buf, width, height, col, y, color);
        canvas_put_pixel(buf, width, height, col, y + h - 1, color);
    }
    for (int row = y; row < y + h; row++) {
        canvas_put_pixel(buf, width, height, x, row, color);
        canvas_put_pixel(buf, width, height, x + w - 1, row, color);
    }
}

void canvas_draw_char(uint8_t* buf, int width, int height, int x, int y,
                       char c, uint8_t color) {
    const uint8_t* glyph;
    if (!font5x7_lookup(c, &glyph)) {
        return; /* space, or anything unrecognized - just a gap */
    }
    for (int row = 0; row < 7; row++) {
        uint8_t bits = glyph[row];
        for (int col = 0; col < 5; col++) {
            if (bits & (uint8_t)(0x10 >> col)) {
                canvas_put_pixel(buf, width, height, x + col, y + row, color);
            }
        }
    }
}

void canvas_draw_text(uint8_t* buf, int width, int height, int x, int y,
                       const char* text, uint8_t color) {
    int cursor_x = x;
    for (const char* p = text; *p; p++) {
        canvas_draw_char(buf, width, height, cursor_x, y, *p, color);
        cursor_x += 6; /* 5px glyph + 1px spacing */
    }
}
