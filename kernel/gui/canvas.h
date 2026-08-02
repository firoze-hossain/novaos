#ifndef GUI_CANVAS_H
#define GUI_CANVAS_H

#include "../include/types.h"

/* Generic pixel-buffer drawing primitives, parameterized by buffer/
 * dimensions rather than assuming a single global screen the way
 * kernel/gui/compositor.c's private helpers do - written fresh for
 * the Phase 12 Software Center rather than refactoring compositor.c
 * to share them, specifically to avoid touching already-verified
 * Phase 7 code for a phase that doesn't need to change it. */

void canvas_put_pixel(uint8_t* buf, int width, int height, int x, int y,
                       uint8_t color);
void canvas_fill_rect(uint8_t* buf, int width, int height, int x, int y,
                       int w, int h, uint8_t color);
void canvas_draw_rect(uint8_t* buf, int width, int height, int x, int y,
                       int w, int h, uint8_t color);

/* Draws one glyph (see font5x7.h) at (x, y). Unrecognized characters
 * (anything font5x7_lookup() doesn't cover) draw nothing - a gap, not
 * a crash or a wrong-looking placeholder. */
void canvas_draw_char(uint8_t* buf, int width, int height, int x, int y,
                       char c, uint8_t color);

/* Draws a NUL-terminated string left-to-right starting at (x, y), 6px
 * per character (5px glyph + 1px spacing). Does not wrap - text
 * running past the buffer's width is silently clipped by
 * canvas_put_pixel()'s own bounds check. */
void canvas_draw_text(uint8_t* buf, int width, int height, int x, int y,
                       const char* text, uint8_t color);

#endif
