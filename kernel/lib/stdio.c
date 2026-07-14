#include "stdio.h"
#include "string.h"

int vsnprintf(char* buffer, size_t size, const char* format, va_list args) {
    size_t pos = 0;
    char temp[32];

    for (const char* p = format; *p && pos < size - 1; p++) {
        if (*p != '%') {
            buffer[pos++] = *p;
            continue;
        }

        p++;
        switch (*p) {
            case 's': {
                const char* str = va_arg(args, const char*);
                while (*str && pos < size - 1) {
                    buffer[pos++] = *str++;
                }
                break;
            }
            case 'd': {
                int val = va_arg(args, int);
                itoa(val, temp, 10);
                for (char* s = temp; *s && pos < size - 1; s++) {
                    buffer[pos++] = *s;
                }
                break;
            }
            case 'x': {
                int val = va_arg(args, int);
                itoa(val, temp, 16);
                for (char* s = temp; *s && pos < size - 1; s++) {
                    buffer[pos++] = *s;
                }
                break;
            }
            case 'c': {
                char c = (char)va_arg(args, int);
                buffer[pos++] = c;
                break;
            }
            default:
                buffer[pos++] = '%';
                buffer[pos++] = *p;
                break;
        }
    }

    buffer[pos] = '\0';
    return pos;
}

int snprintf(char* buffer, size_t size, const char* format, ...) {
    va_list args;
    va_start(args, format);
    int result = vsnprintf(buffer, size, format, args);
    va_end(args);
    return result;
}