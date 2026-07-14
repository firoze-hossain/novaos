#include "vga.h"
#include "../../lib/string.h"
#include "../../lib/stdio.h"
#include <stdarg.h>

// VGA hardware constants
#define VGA_MEMORY      0xB8000
#define VGA_WIDTH       80
#define VGA_HEIGHT      25
#define VGA_COMMAND_PORT 0x3D4
#define VGA_DATA_PORT    0x3D5

// VGA buffer pointer
static volatile uint16_t* vga_buffer = (volatile uint16_t*)VGA_MEMORY;
static uint8_t vga_row = 0;
static uint8_t vga_col = 0;
static uint8_t vga_color = 0x0F; // White on black

// I/O Port functions
void vga_outb(uint16_t port, uint8_t value) {
    __asm__ volatile ("outb %0, %1" : : "a"(value), "Nd"(port));
}

uint8_t vga_inb(uint16_t port) {
    uint8_t ret;
    __asm__ volatile ("inb %1, %0" : "=a"(ret) : "Nd"(port));
    return ret;
}

// Helper: Make video memory entry
static inline uint16_t vga_entry(char c, uint8_t color) {
    return (uint16_t)c | ((uint16_t)color << 8);
}

void vga_init(void) {
    vga_clear();
    vga_cursor_enable();
}

void vga_clear(void) {
    for (int y = 0; y < VGA_HEIGHT; y++) {
        for (int x = 0; x < VGA_WIDTH; x++) {
            const size_t index = y * VGA_WIDTH + x;
            vga_buffer[index] = vga_entry(' ', vga_color);
        }
    }
    vga_row = 0;
    vga_col = 0;
}

void vga_set_color(vga_color_t foreground, vga_color_t background) {
    vga_color = (background << 4) | (foreground & 0x0F);
}

void vga_putchar(char c) {
    if (c == '\n') {
        vga_col = 0;
        vga_row++;
    } else if (c == '\r') {
        vga_col = 0;
    } else if (c == '\t') {
        vga_col = (vga_col + 4) & ~3;
    } else if (c == '\b') {
        if (vga_col > 0) {
            vga_col--;
            const size_t index = vga_row * VGA_WIDTH + vga_col;
            vga_buffer[index] = vga_entry(' ', vga_color);
        }
    } else {
        const size_t index = vga_row * VGA_WIDTH + vga_col;
        vga_buffer[index] = vga_entry(c, vga_color);
        vga_col++;
    }

    // Handle scrolling
    if (vga_col >= VGA_WIDTH) {
        vga_col = 0;
        vga_row++;
    }

    if (vga_row >= VGA_HEIGHT) {
        // Scroll up
        for (int y = 1; y < VGA_HEIGHT; y++) {
            for (int x = 0; x < VGA_WIDTH; x++) {
                const size_t src = y * VGA_WIDTH + x;
                const size_t dst = (y - 1) * VGA_WIDTH + x;
                vga_buffer[dst] = vga_buffer[src];
            }
        }
        // Clear last line
        for (int x = 0; x < VGA_WIDTH; x++) {
            const size_t index = (VGA_HEIGHT - 1) * VGA_WIDTH + x;
            vga_buffer[index] = vga_entry(' ', vga_color);
        }
        vga_row = VGA_HEIGHT - 1;
    }

    // Update cursor
    vga_cursor_set_pos(vga_col, vga_row);
}

void vga_puts(const char* str) {
    while (*str) {
        vga_putchar(*str++);
    }
}

void vga_printf(const char* format, ...) {
    va_list args;
    va_start(args, format);

    char buffer[256];
    vsnprintf(buffer, sizeof(buffer), format, args);
    vga_puts(buffer);

    va_end(args);
}

// Cursor functions
void vga_cursor_enable(void) {
    vga_cursor_set_pos(vga_col, vga_row);
}

void vga_cursor_disable(void) {
    vga_outb(VGA_COMMAND_PORT, 0x0A);
    vga_outb(VGA_DATA_PORT, 0x20);
}

void vga_cursor_set_pos(int x, int y) {
    const uint16_t pos = y * VGA_WIDTH + x;
    vga_outb(VGA_COMMAND_PORT, 0x0F);
    vga_outb(VGA_DATA_PORT, (uint8_t)(pos & 0xFF));
    vga_outb(VGA_COMMAND_PORT, 0x0E);
    vga_outb(VGA_DATA_PORT, (uint8_t)((pos >> 8) & 0xFF));
}

void vga_cursor_get_pos(int* x, int* y) {
    *x = vga_col;
    *y = vga_row;
}