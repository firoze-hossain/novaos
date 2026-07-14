#ifndef VGA_H
#define VGA_H

#include "../../include/types.h"

// VGA color codes
typedef enum {
    VGA_COLOR_BLACK         = 0,
    VGA_COLOR_BLUE          = 1,
    VGA_COLOR_GREEN         = 2,
    VGA_COLOR_CYAN          = 3,
    VGA_COLOR_RED           = 4,
    VGA_COLOR_MAGENTA       = 5,
    VGA_COLOR_BROWN         = 6,
    VGA_COLOR_LIGHT_GREY    = 7,
    VGA_COLOR_DARK_GREY     = 8,
    VGA_COLOR_LIGHT_BLUE    = 9,
    VGA_COLOR_LIGHT_GREEN   = 10,
    VGA_COLOR_LIGHT_CYAN    = 11,
    VGA_COLOR_LIGHT_RED     = 12,
    VGA_COLOR_LIGHT_MAGENTA = 13,
    VGA_COLOR_LIGHT_BROWN   = 14,
    VGA_COLOR_WHITE         = 15,
} vga_color_t;

// Initialize VGA
void vga_init(void);

// Clear screen
void vga_clear(void);

// Set text color
void vga_set_color(vga_color_t foreground, vga_color_t background);

// Print functions
void vga_putchar(char c);
void vga_puts(const char* str);
void vga_printf(const char* format, ...);

// Cursor control
void vga_cursor_enable(void);
void vga_cursor_disable(void);
void vga_cursor_set_pos(int x, int y);
void vga_cursor_get_pos(int* x, int* y);

// I/O Port functions
void vga_outb(uint16_t port, uint8_t value);
uint8_t vga_inb(uint16_t port);

#endif