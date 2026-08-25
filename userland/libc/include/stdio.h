#ifndef NOVA_STDIO_H
#define NOVA_STDIO_H

int putchar(int c);
int puts(const char* s);

/* Supports %d, %u, %x, %s, %c, %% only - no field width/precision, no
 * floating point. Enough for real, useful programs without the scope
 * of a full C89 printf. Builds the whole formatted string into a
 * fixed 512-byte internal buffer before a single sys_write() call - a
 * hard length limit (silently truncates past it), not a streaming
 * implementation. */
int printf(const char* fmt, ...);

#endif
