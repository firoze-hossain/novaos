/*
 * stdio.c - putchar/puts/printf (see stdio.h for exact scope)
 */
#include "stdio.h"
#include "novasys.h"
#include <stdarg.h>

int putchar(int c) {
    char buf[2] = {(char)c, '\0'};
    sys_write(buf);
    return c;
}

int puts(const char* s) {
    sys_write(s);
    sys_write("\n");
    return 0;
}

/* Converts `value` into `out` in the given base (10 or 16), writing
 * least-significant digit first then reversing in place - the usual
 * technique, since digits naturally come out backwards from repeated
 * division. Returns the number of characters written. Local to this
 * file: not exposed as a general itoa(), since nothing else needs
 * one. */
static int format_uint(unsigned int value, int base, int uppercase,
                        char* out) {
    static const char* lower_digits = "0123456789abcdef";
    static const char* upper_digits = "0123456789ABCDEF";
    const char* digits = uppercase ? upper_digits : lower_digits;

    int len = 0;
    if (value == 0) {
        out[len++] = '0';
    } else {
        char reversed[16];
        int rlen = 0;
        while (value > 0) {
            reversed[rlen++] = digits[value % (unsigned int)base];
            value /= (unsigned int)base;
        }
        while (rlen > 0) {
            out[len++] = reversed[--rlen];
        }
    }
    return len;
}

int printf(const char* fmt, ...) {
    char buffer[512];
    int pos = 0;

    va_list args;
    va_start(args, fmt);

    for (const char* p = fmt; *p && pos < (int)sizeof(buffer) - 1; p++) {
        if (*p != '%') {
            buffer[pos++] = *p;
            continue;
        }

        p++;
        if (*p == '\0') {
            break;
        }

        switch (*p) {
            case 'd': {
                int value = va_arg(args, int);
                unsigned int uvalue =
                    (value < 0) ? (unsigned int)(-value) : (unsigned int)value;
                if (value < 0 && pos < (int)sizeof(buffer) - 1) {
                    buffer[pos++] = '-';
                }
                pos += format_uint(uvalue, 10, 0, buffer + pos);
                break;
            }
            case 'u':
                pos += format_uint(va_arg(args, unsigned int), 10, 0,
                                    buffer + pos);
                break;
            case 'x':
                pos += format_uint(va_arg(args, unsigned int), 16, 0,
                                    buffer + pos);
                break;
            case 's': {
                const char* s = va_arg(args, const char*);
                while (*s && pos < (int)sizeof(buffer) - 1) {
                    buffer[pos++] = *s++;
                }
                break;
            }
            case 'c':
                buffer[pos++] = (char)va_arg(args, int);
                break;
            case '%':
                buffer[pos++] = '%';
                break;
            default:
                /* Unrecognized specifier - print it literally rather
                 * than silently eating the '%' and the character, so
                 * a typo is visible instead of vanishing. */
                buffer[pos++] = '%';
                if (pos < (int)sizeof(buffer) - 1) {
                    buffer[pos++] = *p;
                }
                break;
        }
    }

    va_end(args);
    buffer[pos] = '\0';
    sys_write(buffer);
    return pos;
}
