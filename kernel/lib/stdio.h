#ifndef STDIO_H
#define STDIO_H

#include "../include/types.h"
#include <stdarg.h>

int vsnprintf(char* buffer, size_t size, const char* format, va_list args);
int snprintf(char* buffer, size_t size, const char* format, ...);

#endif