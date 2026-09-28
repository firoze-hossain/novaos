#ifndef NOVA_STDIO_H
#define NOVA_STDIO_H

#include <stddef.h>
#include <stdarg.h>

/* ------------------------------------------------------------------ *
 * Streams (Phase 73)
 *
 * A FILE is an in-memory image of a file. fopen() loads the whole file
 * (reading it through SYS_OPEN/SYS_READ and closing the kernel handle
 * again immediately); fread/fgets/fseek/... then work on that image;
 * fwrite/fputs/... modify it and mark it dirty; fflush()/fclose() (and
 * exit(), automatically) write the whole image back. That model gives
 * exact fseek/ftell/append/read-write semantics on a kernel whose file
 * API has no seek, no partial write and no rename, and it means a FILE
 * never holds one of the kernel's 8 system-wide open-file slots - so
 * a program that forgets to fclose(), or dies, cannot leak them.
 *
 * What that costs, honestly:
 *   - a stream needs memory for its whole file (a failed load is
 *     ENOMEM, not a truncated read);
 *   - writing is whole-file: every fflush() of a dirty stream deletes
 *     the file and creates it again with the full contents. It is not
 *     atomic - a crash between the delete and the create loses the
 *     file. If the create fails, the data stays in memory and the
 *     stream stays dirty, so a later fflush() can retry;
 *   - a file name is a root-directory 8.3 name (at most 12 characters,
 *     no '/'): fopen("A_LONG_NAME.TXT", ...) is ENAMETOOLONG;
 *   - creating/writing/deleting needs the same file capability those
 *     syscalls always needed (can_open_any_file, see process.h): a
 *     program without it gets EACCES from fopen("w").
 *
 * stdout and stderr are the console, unbuffered; stdin reads one raw
 * key at a time from the keyboard (blocking, no echo, no line editing).
 * ------------------------------------------------------------------ */
typedef struct nova_file FILE;

#define EOF          (-1)
#define SEEK_SET     0
#define SEEK_CUR     1
#define SEEK_END     2
#define BUFSIZ       512
#define FOPEN_MAX    16   /* files one program may have open at once
                              (stdin/stdout/stderr not counted) */
#define FILENAME_MAX 13   /* 8.3 name + NUL */

extern FILE* stdin;
extern FILE* stdout;
extern FILE* stderr;

/* Modes: "r", "w", "a", each optionally with '+' (read and write) and/or
 * 'b' (accepted, ignored - there is no text/binary distinction).
 * "r" needs the file to exist (ENOENT otherwise); "w" creates or
 * empties it immediately; "a" creates it if missing and directs every
 * write to the end. Any other mode string is EINVAL. Returns NULL with
 * errno set on failure: ENOENT (missing), EACCES (the kernel refused -
 * this also covers a full disk or open-file table, which the kernel
 * does not distinguish), ENAMETOOLONG, EMFILE (FOPEN_MAX reached),
 * ENOMEM, EINVAL. */
FILE* fopen(const char* path, const char* mode);

/* Flushes if dirty and releases the stream. Returns 0, or EOF if the
 * final flush failed (the data is then lost - fflush() first if you
 * need to retry). The FILE pointer is invalid afterwards either way. */
int fclose(FILE* f);

/* fflush(NULL) flushes every open stream. Returns 0, or EOF with errno
 * set (the stream's error flag is also set). A no-op for streams that
 * are not dirty and for stdout/stderr/stdin. */
int fflush(FILE* f);

/* Return the number of complete items transferred; a short count means
 * end of file or an error - check feof()/ferror(). Writing to a
 * read-only stream (or reading a write-only one) sets the error flag
 * and errno == EBADF. */
size_t fread(void* ptr, size_t size, size_t nmemb, FILE* f);
size_t fwrite(const void* ptr, size_t size, size_t nmemb, FILE* f);

/* Positioning. Seeking past the end is allowed; a later write
 * zero-fills the gap. fseek() returns 0, or -1 with errno (EINVAL for
 * a negative result or bad whence; ESPIPE on stdin/stdout/stderr).
 * ftell() returns the position, or -1. */
int fseek(FILE* f, long offset, int whence);
long ftell(FILE* f);
void rewind(FILE* f);

int feof(FILE* f);
int ferror(FILE* f);
void clearerr(FILE* f);

int fgetc(FILE* f);
int getc(FILE* f);
int getchar(void);
/* Guarantees one character of pushback (a second ungetc() before a
 * read returns EOF). */
int ungetc(int c, FILE* f);
char* fgets(char* s, int n, FILE* f);

int fputc(int c, FILE* f);
int putc(int c, FILE* f);
int fputs(const char* s, FILE* f);
int putchar(int c);
int puts(const char* s);

/* Formatted output.
 *
 * Conversions: %d %i %u %x %X %o %c %s %p %%.
 * Flags: '-' (left-justify), '0' (zero-pad), '+', ' ', '#' (0x prefix
 * for %x/%X). Width and precision as digits or '*'. Length modifiers
 * h, hh, l, z, j, t are accepted (this target's int, long, size_t and
 * pointers are all 32 bits); "ll" (64-bit) is NOT supported - the
 * whole specifier is printed literally instead of misreading the
 * argument list. No floating point. An unrecognised conversion is
 * printed literally rather than silently swallowed.
 *
 * vsnprintf()/snprintf() write at most `size` bytes including the
 * terminating NUL (none at all if size is 0) and return the number of
 * characters the full result WOULD have had, so a return value >= size
 * means truncation - the C99 contract. sprintf() is unbounded, as in C.
 * printf()/fprintf()/vfprintf() format into a temporary buffer first
 * (heap-allocated only when the result exceeds BUFSIZ) and never
 * truncate; they return the character count, or -1 on failure. */
int printf(const char* fmt, ...);
int fprintf(FILE* f, const char* fmt, ...);
int vprintf(const char* fmt, va_list ap);
int vfprintf(FILE* f, const char* fmt, va_list ap);
int sprintf(char* out, const char* fmt, ...);
int snprintf(char* out, size_t size, const char* fmt, ...);
int vsnprintf(char* out, size_t size, const char* fmt, va_list ap);

/* Deletes a root-directory file. Returns 0, or -1 with errno set
 * (ENOENT if it does not exist, EACCES if the kernel refused). */
int remove(const char* path);

/* Prints "<s>: <strerror(errno)>\n" to stderr (just the message if s is
 * NULL or empty). */
void perror(const char* s);

#endif
