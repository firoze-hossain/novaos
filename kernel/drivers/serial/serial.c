/*
 * serial.c - Minimal 16550 UART driver (COM1 @ 0x3F8)
 *
 * Every hosted OS needs a logging channel that works before (and
 * independently of) the video driver: it is how kernel_panic() output
 * survives even if VGA state is corrupted, and it is what our headless
 * CI boot-test (QEMU with `-serial stdio`, no display) reads to verify
 * that each subsystem initialized correctly on Windows/Linux/macOS
 * runners alike.
 */
#include "serial.h"
#include "../../arch/x86/io.h"
#include "../../lib/stdio.h"
#include <stdarg.h>

#define COM1 0x3F8

void serial_init(void) {
    outb(COM1 + 1, 0x00);    /* Disable interrupts             */
    outb(COM1 + 3, 0x80);    /* Enable DLAB (set baud divisor)  */
    outb(COM1 + 0, 0x03);    /* Divisor low byte  -> 38400 baud */
    outb(COM1 + 1, 0x00);    /* Divisor high byte               */
    outb(COM1 + 3, 0x03);    /* 8 bits, no parity, one stop bit */
    outb(COM1 + 2, 0xC7);    /* Enable FIFO, clear, 14-byte threshold */
    outb(COM1 + 4, 0x0B);    /* IRQs disabled, RTS/DSR set       */
}

static int transmit_empty(void) {
    return inb(COM1 + 5) & 0x20;
}

void serial_putchar(char c) {
    if (c == '\n') {
        serial_putchar('\r');
    }
    while (!transmit_empty()) { }
    outb(COM1, (uint8_t)c);
}

void serial_puts(const char* str) {
    while (*str) {
        serial_putchar(*str++);
    }
}

void serial_printf(const char* format, ...) {
    va_list args;
    va_start(args, format);

    char buffer[256];
    vsnprintf(buffer, sizeof(buffer), format, args);
    serial_puts(buffer);

    va_end(args);
}
