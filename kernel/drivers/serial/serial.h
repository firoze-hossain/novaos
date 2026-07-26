#ifndef DRIVERS_SERIAL_H
#define DRIVERS_SERIAL_H

#include "../../include/types.h"

/* Initialize COM1 at 38400 baud, 8N1. Safe to call even when no serial
 * port is present (QEMU always provides one; on real hardware output is
 * simply discarded if nothing is attached). */
void serial_init(void);

void serial_putchar(char c);
void serial_puts(const char* str);

/* printf-style logging that always goes to the serial port. This is the
 * backbone of kernel_log() and is what the test harness (see
 * TESTING.md / scripts/test.sh) greps for boot markers such as
 * "[ OK ] GDT initialized". */
void serial_printf(const char* format, ...);

#endif
