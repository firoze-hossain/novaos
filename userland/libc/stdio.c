/*
 * stdio.c - formatted output (printf family), and FILE streams.
 * See stdio.h for the full contract and its honest limits - in
 * particular the "a FILE is an in-memory image of a whole file" model
 * every function below is built around.
 */
#include "stdio.h"
#include "novasys.h"
#include "stdlib.h"
#include "string.h"
#include "errno.h"
#include <stdarg.h>
#include <stdint.h>

/* ================================================================== *
 * Formatted output
 * ================================================================== */

typedef struct {
    char* out;
    size_t cap;
    size_t len; /* characters the COMPLETE result has, written or not */
} fmt_sink_t;

/* Stores c if there is room (always leaving space for the NUL) but
 * always counts it, so the final length is the C99 "would have been"
 * length even when the buffer was too small. */
static void sink_put(fmt_sink_t* s, char c) {
    if (s->cap > 0 && s->len < s->cap - 1) {
        s->out[s->len] = c;
    }
    s->len++;
}

static void sink_pad(fmt_sink_t* s, char c, int n) {
    while (n-- > 0) {
        sink_put(s, c);
    }
}

/* Writes an unsigned value in `base` with an optional sign character
 * and prefix ("0x"), honouring width, '-' and '0' flags and precision
 * (minimum digit count). As in C: an explicit precision cancels the
 * '0' flag, and a zero value with precision 0 prints no digits. */
static void emit_uint(fmt_sink_t* s, unsigned int v, unsigned int base,
                      int upper, char sign, const char* prefix, int left,
                      int zero, int width, int prec) {
    static const char lower_digits[] = "0123456789abcdef";
    static const char upper_digits[] = "0123456789ABCDEF";
    const char* digits = upper ? upper_digits : lower_digits;

    char rev[12]; /* 32 bits in base 8 needs 11 digits */
    int n = 0;
    if (!(v == 0 && prec == 0)) {
        if (v == 0) {
            rev[n++] = '0';
        }
        while (v != 0) {
            rev[n++] = digits[v % base];
            v /= base;
        }
    }

    int zeros = 0;
    if (prec >= 0) {
        if (prec > n) {
            zeros = prec - n;
        }
        zero = 0;
    }
    int plen = 0;
    while (prefix != NULL && prefix[plen] != '\0') {
        plen++;
    }
    int body = (sign != 0 ? 1 : 0) + plen + zeros + n;
    int padn = width > body ? width - body : 0;

    if (!left && !zero) {
        sink_pad(s, ' ', padn);
    }
    if (sign != 0) {
        sink_put(s, sign);
    }
    for (int i = 0; i < plen; i++) {
        sink_put(s, prefix[i]);
    }
    if (!left && zero) {
        sink_pad(s, '0', padn);
    }
    sink_pad(s, '0', zeros);
    while (n > 0) {
        sink_put(s, rev[--n]);
    }
    if (left) {
        sink_pad(s, ' ', padn);
    }
}

int vsnprintf(char* out, size_t cap, const char* fmt, va_list ap) {
    fmt_sink_t s = { out, cap, 0 };

    for (const char* p = fmt; *p != '\0'; p++) {
        if (*p != '%') {
            sink_put(&s, *p);
            continue;
        }

        const char* spec_start = p;
        p++;

        int left = 0, zero = 0, plus = 0, space = 0, alt = 0;
        for (;; p++) {
            if (*p == '-')      left = 1;
            else if (*p == '0') zero = 1;
            else if (*p == '+') plus = 1;
            else if (*p == ' ') space = 1;
            else if (*p == '#') alt = 1;
            else break;
        }

        int width = 0;
        if (*p == '*') {
            width = va_arg(ap, int);
            if (width < 0) {
                left = 1;
                width = -width;
            }
            p++;
        } else {
            while (*p >= '0' && *p <= '9') {
                width = width * 10 + (*p - '0');
                if (width > 65535) {
                    width = 65535; /* keep absurd widths from wrapping */
                }
                p++;
            }
        }

        int prec = -1;
        if (*p == '.') {
            p++;
            prec = 0;
            if (*p == '*') {
                prec = va_arg(ap, int);
                if (prec < 0) {
                    prec = -1;
                }
                p++;
            } else {
                while (*p >= '0' && *p <= '9') {
                    prec = prec * 10 + (*p - '0');
                    if (prec > 65535) {
                        prec = 65535;
                    }
                    p++;
                }
            }
        }

        int hcount = 0, lcount = 0;
        for (;; p++) {
            if (*p == 'h')      hcount++;
            else if (*p == 'l') lcount++;
            else if (*p == 'z' || *p == 'j' || *p == 't') { /* 32-bit */ }
            else break;
        }

        if (*p == '\0') {
            /* The format ended in the middle of a specifier: emit what
             * was there rather than reading past the terminator. */
            for (const char* q = spec_start; q < p; q++) {
                sink_put(&s, *q);
            }
            break;
        }

        if (lcount >= 2) {
            /* %lld and friends need 64-bit arguments this libc does
             * not format (and 64-bit division would need libgcc,
             * which userland does not link). Print the specifier as
             * written instead of misreading the argument list. */
            for (const char* q = spec_start; q <= p; q++) {
                sink_put(&s, *q);
            }
            continue;
        }

        switch (*p) {
            case 'd':
            case 'i': {
                int v = va_arg(ap, int);
                if (hcount == 1) {
                    v = (short)v;
                } else if (hcount >= 2) {
                    v = (signed char)v;
                }
                char sign = 0;
                unsigned int mag;
                if (v < 0) {
                    sign = '-';
                    mag = 0u - (unsigned int)v; /* correct for INT_MIN */
                } else {
                    mag = (unsigned int)v;
                    sign = plus ? '+' : (space ? ' ' : 0);
                }
                emit_uint(&s, mag, 10, 0, sign, NULL, left, zero, width,
                          prec);
                break;
            }
            case 'u':
            case 'x':
            case 'X':
            case 'o': {
                unsigned int v = va_arg(ap, unsigned int);
                if (hcount == 1) {
                    v &= 0xFFFFu;
                } else if (hcount >= 2) {
                    v &= 0xFFu;
                }
                unsigned int base = (*p == 'u') ? 10u : (*p == 'o') ? 8u : 16u;
                const char* prefix = NULL;
                if (alt && v != 0) {
                    if (*p == 'x') {
                        prefix = "0x";
                    } else if (*p == 'X') {
                        prefix = "0X";
                    } else if (*p == 'o') {
                        prefix = "0";
                    }
                }
                emit_uint(&s, v, base, *p == 'X', 0, prefix, left, zero,
                          width, prec);
                break;
            }
            case 'p': {
                void* ptr = va_arg(ap, void*);
                emit_uint(&s, (unsigned int)(uintptr_t)ptr, 16, 0, 0, "0x",
                          left, zero, width, prec);
                break;
            }
            case 'c': {
                char c = (char)va_arg(ap, int);
                int padn = width > 1 ? width - 1 : 0;
                if (!left) {
                    sink_pad(&s, ' ', padn);
                }
                sink_put(&s, c);
                if (left) {
                    sink_pad(&s, ' ', padn);
                }
                break;
            }
            case 's': {
                const char* str = va_arg(ap, const char*);
                if (str == NULL) {
                    str = "(null)";
                }
                int n = 0;
                while (str[n] != '\0' && (prec < 0 || n < prec)) {
                    n++;
                }
                int padn = width > n ? width - n : 0;
                if (!left) {
                    sink_pad(&s, ' ', padn);
                }
                for (int i = 0; i < n; i++) {
                    sink_put(&s, str[i]);
                }
                if (left) {
                    sink_pad(&s, ' ', padn);
                }
                break;
            }
            case '%':
                sink_put(&s, '%');
                break;
            default:
                /* Unrecognised conversion: print it as written so a
                 * typo is visible instead of vanishing. */
                for (const char* q = spec_start; q <= p; q++) {
                    sink_put(&s, *q);
                }
                break;
        }
    }

    if (cap > 0) {
        out[s.len < cap ? s.len : cap - 1] = '\0';
    }
    return (int)s.len;
}

int snprintf(char* out, size_t size, const char* fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(out, size, fmt, ap);
    va_end(ap);
    return n;
}

int sprintf(char* out, const char* fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vsnprintf(out, 0x7FFFFFFFu, fmt, ap);
    va_end(ap);
    return n;
}

/* ================================================================== *
 * FILE streams
 * ================================================================== */

struct nova_file {
    unsigned char* buf; /* the file's whole contents (NULL for std streams) */
    size_t size;        /* valid bytes in buf */
    size_t cap;         /* bytes allocated */
    size_t pos;         /* current position, may exceed size after a seek */
    unsigned int flags;
    int unget;          /* pushed-back byte, or -1 */
    int kind;
    char name[16];      /* 8.3 name (<= 12 chars) + NUL */
};

#define F_OPEN   0x01u
#define F_READ   0x02u
#define F_WRITE  0x04u
#define F_APPEND 0x08u
#define F_DIRTY  0x10u
#define F_EOF    0x20u
#define F_ERR    0x40u

enum { K_FILE = 0, K_STDIN, K_STDOUT, K_STDERR };

#define FILE_LOAD_CHUNK 4096 /* == the kernel's per-call SYS_READ cap */

static struct nova_file stdin_obj  = { NULL, 0, 0, 0, F_OPEN | F_READ, -1,
                                        K_STDIN, "" };
static struct nova_file stdout_obj = { NULL, 0, 0, 0, F_OPEN | F_WRITE, -1,
                                        K_STDOUT, "" };
static struct nova_file stderr_obj = { NULL, 0, 0, 0, F_OPEN | F_WRITE, -1,
                                        K_STDERR, "" };
FILE* stdin  = &stdin_obj;
FILE* stdout = &stdout_obj;
FILE* stderr = &stderr_obj;

/* Every stream fopen() has handed out and fclose() has not yet taken
 * back - what fflush(NULL) and the exit-time flush walk. */
static FILE* open_streams[FOPEN_MAX];
static int exit_flush_registered;

static int stream_ok(const FILE* f) {
    return f != NULL && (f->flags & F_OPEN) != 0;
}

/* Console output for stdout/stderr. sys_write() takes a C string, so
 * bytes go out in NUL-free chunks (a NUL byte cannot be shown on a
 * console anyway and is dropped). 512 matches the old printf's single
 * buffer, so any output that fit in one sys_write() before still does. */
static void console_write(const unsigned char* p, size_t n) {
    char chunk[512];
    size_t i = 0;
    while (i < n) {
        size_t k = 0;
        while (i < n && k < sizeof(chunk) - 1) {
            if (p[i] != 0) {
                chunk[k++] = (char)p[i];
            }
            i++;
        }
        chunk[k] = '\0';
        if (k > 0) {
            sys_write(chunk);
        }
    }
}

/* Ensures the stream's buffer has room for `need` bytes. */
static int stream_reserve(FILE* f, size_t need) {
    if (need <= f->cap) {
        return 0;
    }
    size_t ncap = f->cap != 0 ? f->cap : 512;
    while (ncap < need) {
        if (ncap > 0x40000000u) {
            errno = ENOMEM;
            return -1;
        }
        ncap *= 2;
    }
    unsigned char* nb = (unsigned char*)realloc(f->buf, ncap);
    if (nb == NULL) {
        return -1; /* errno == ENOMEM from realloc */
    }
    f->buf = nb;
    f->cap = ncap;
    return 0;
}

/* Root-directory 8.3 names only: this kernel's open-file table stores
 * at most 12 characters (anything longer would be silently truncated
 * to a DIFFERENT file's name), and the filesystem has no directories. */
static int check_name(const char* path) {
    if (path == NULL) {
        errno = EINVAL;
        return -1;
    }
    size_t n = strlen(path);
    if (n == 0) {
        errno = ENOENT;
        return -1;
    }
    if (n > 12) {
        errno = ENAMETOOLONG;
        return -1;
    }
    for (size_t i = 0; i < n; i++) {
        if (path[i] == '/') {
            errno = ENOENT; /* no directories exist */
            return -1;
        }
    }
    return 0;
}

/* Does the file exist? SYS_OPEN never checks (it only claims a handle
 * slot - see kernel handle_open()), so the only real test is whether a
 * read against the handle succeeds: -1 means missing, 0 or 1 means
 * present. *denied is set when SYS_OPEN itself refused. */
static int probe_exists(const char* name, int* denied) {
    int h = sys_open(name);
    if (h < 0) {
        *denied = 1;
        return 0;
    }
    char c;
    int r = sys_read(h, &c, 1);
    sys_close(h);
    *denied = 0;
    return r >= 0;
}

/* Reads the whole file into f->buf. The kernel handle exists only for
 * the duration of this function. Returns 0, or -1 with errno set. */
static int load_file(FILE* f, const char* name) {
    int h = sys_open(name);
    if (h < 0) {
        errno = EACCES;
        return -1;
    }
    int first = 1;
    for (;;) {
        if (stream_reserve(f, f->size + FILE_LOAD_CHUNK) < 0) {
            sys_close(h);
            return -1;
        }
        int n = sys_read(h, f->buf + f->size, FILE_LOAD_CHUNK);
        if (n < 0) {
            sys_close(h);
            /* A failure on the FIRST read means the file is not there;
             * later, it means something went wrong mid-file. */
            errno = first ? ENOENT : EIO;
            return -1;
        }
        if (n == 0) {
            break;
        }
        f->size += (size_t)n;
        first = 0;
    }
    sys_close(h);
    return 0;
}

/* "w": create the file, or empty it if it exists. The kernel's file
 * API is create-only, so emptying means delete-then-create. */
static int create_empty(const char* name) {
    int denied = 0;
    if (probe_exists(name, &denied)) {
        if (sys_delete_file(name) < 0) {
            errno = EACCES;
            return -1;
        }
    } else if (denied) {
        errno = EACCES;
        return -1;
    }
    if (sys_write_file(name, "", 0) < 0) {
        errno = EACCES; /* refused; may also be disk-full (see errno.h) */
        return -1;
    }
    return 0;
}

/* Writes a dirty stream's image back: delete + create, since no
 * in-place update exists. Not atomic - see stdio.h. */
static int flush_stream(FILE* f) {
    if (f->kind != K_FILE || (f->flags & F_DIRTY) == 0) {
        return 0;
    }
    static const char empty = 0;
    const void* data = (f->size != 0) ? (const void*)f->buf
                                        : (const void*)&empty;
    int d = sys_delete_file(f->name);
    int w = sys_write_file(f->name, data, (unsigned int)f->size);
    if (w < 0) {
        f->flags |= F_ERR;
        /* Both steps refused: the kernel would not let us replace the
         * file at all (permissions). Only the create refused: the
         * delete had gone through, so the disk rejected the data. */
        errno = (d < 0) ? EACCES : EIO;
        return -1;
    }
    f->flags &= ~F_DIRTY;
    return 0;
}

static void flush_all_at_exit(void) {
    for (int i = 0; i < FOPEN_MAX; i++) {
        if (open_streams[i] != NULL) {
            flush_stream(open_streams[i]);
        }
    }
}

static int parse_mode(const char* mode, int* base, int* plus) {
    if (mode == NULL) {
        return -1;
    }
    char c = mode[0];
    if (c != 'r' && c != 'w' && c != 'a') {
        return -1;
    }
    *base = c;
    *plus = 0;
    for (int i = 1; mode[i] != '\0'; i++) {
        if (mode[i] == '+') {
            *plus = 1;
        } else if (mode[i] != 'b') {
            return -1; /* C11 "x", glibc "e"/"m"... are not supported */
        }
    }
    return 0;
}

FILE* fopen(const char* path, const char* mode) {
    int base, plus;
    if (parse_mode(mode, &base, &plus) < 0) {
        errno = EINVAL;
        return NULL;
    }
    if (check_name(path) < 0) {
        return NULL;
    }
    int slot = -1;
    for (int i = 0; i < FOPEN_MAX; i++) {
        if (open_streams[i] == NULL) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        errno = EMFILE;
        return NULL;
    }
    FILE* f = (FILE*)calloc(1, sizeof(FILE));
    if (f == NULL) {
        return NULL;
    }
    size_t nl = strlen(path);
    memcpy(f->name, path, nl + 1);
    f->unget = -1;
    f->kind = K_FILE;
    f->flags = F_OPEN;

    if (base == 'r') {
        if (load_file(f, path) < 0) {
            goto fail;
        }
        f->flags |= F_READ | (plus ? F_WRITE : 0u);
    } else if (base == 'w') {
        if (create_empty(path) < 0) {
            goto fail;
        }
        f->flags |= F_WRITE | (plus ? F_READ : 0u);
    } else { /* 'a' */
        if (load_file(f, path) < 0) {
            if (errno != ENOENT) {
                goto fail;
            }
            if (sys_write_file(path, "", 0) < 0) {
                errno = EACCES;
                goto fail;
            }
        }
        f->flags |= F_WRITE | F_APPEND | (plus ? F_READ : 0u);
        f->pos = plus ? 0 : f->size;
    }

    open_streams[slot] = f;
    if (!exit_flush_registered) {
        /* If the table is somehow full this simply fails and an
         * un-fclose()d write is lost at exit - the same outcome as the
         * program crashing, and no worse than before atexit existed. */
        if (atexit(flush_all_at_exit) == 0) {
            exit_flush_registered = 1;
        }
    }
    return f;

fail:
    {
        int saved = errno; /* free() must not be able to clobber it */
        free(f->buf);
        free(f);
        errno = saved;
    }
    return NULL;
}

int fflush(FILE* f) {
    if (f == NULL) {
        int rc = 0;
        for (int i = 0; i < FOPEN_MAX; i++) {
            if (open_streams[i] != NULL && flush_stream(open_streams[i]) < 0) {
                rc = EOF;
            }
        }
        return rc;
    }
    if (!stream_ok(f)) {
        errno = EBADF;
        return EOF;
    }
    return flush_stream(f) < 0 ? EOF : 0;
}

int fclose(FILE* f) {
    if (!stream_ok(f)) {
        errno = EBADF;
        return EOF;
    }
    if (f->kind != K_FILE) {
        f->flags &= ~F_OPEN; /* the three std streams are static */
        return 0;
    }
    int rc = flush_stream(f) < 0 ? EOF : 0;
    int saved = errno;
    for (int i = 0; i < FOPEN_MAX; i++) {
        if (open_streams[i] == f) {
            open_streams[i] = NULL;
        }
    }
    f->flags = 0;
    free(f->buf);
    free(f);
    if (rc != 0) {
        errno = saved;
    }
    return rc;
}

int fgetc(FILE* f) {
    if (!stream_ok(f)) {
        errno = EBADF;
        return EOF;
    }
    if ((f->flags & F_READ) == 0) {
        f->flags |= F_ERR;
        errno = EBADF;
        return EOF;
    }
    if (f->unget >= 0) {
        int c = f->unget;
        f->unget = -1;
        return c;
    }
    if (f->kind == K_STDIN) {
        int k;
        while ((k = sys_read_key()) < 0) {
            sys_yield(); /* sys_read_key() polls; give the CPU away */
        }
        return k & 0xFF;
    }
    if (f->pos >= f->size) {
        f->flags |= F_EOF;
        return EOF;
    }
    return f->buf[f->pos++];
}

int getc(FILE* f) {
    return fgetc(f);
}

int getchar(void) {
    return fgetc(stdin);
}

int ungetc(int c, FILE* f) {
    if (!stream_ok(f) || c == EOF || f->unget >= 0 ||
        (f->flags & F_READ) == 0) {
        return EOF;
    }
    f->unget = c & 0xFF;
    f->flags &= ~F_EOF;
    return f->unget;
}

size_t fread(void* ptr, size_t size, size_t nmemb, FILE* f) {
    if (!stream_ok(f)) {
        errno = EBADF;
        return 0;
    }
    if (size == 0 || nmemb == 0) {
        return 0;
    }
    if (nmemb > 0xFFFFFFFFu / size) {
        f->flags |= F_ERR;
        errno = EOVERFLOW;
        return 0;
    }
    if ((f->flags & F_READ) == 0) {
        f->flags |= F_ERR;
        errno = EBADF;
        return 0;
    }
    size_t total = size * nmemb;
    unsigned char* out = (unsigned char*)ptr;
    size_t got = 0;

    if (f->kind == K_STDIN) {
        /* A terminal returns a line at a time; do the same. */
        while (got < total) {
            int c = fgetc(f);
            if (c == EOF) {
                break;
            }
            out[got++] = (unsigned char)c;
            if (c == '\n') {
                break;
            }
        }
        return got / size;
    }

    if (f->unget >= 0) {
        out[got++] = (unsigned char)f->unget;
        f->unget = -1;
    }
    size_t avail = (f->size > f->pos) ? f->size - f->pos : 0;
    size_t take = total - got;
    if (take > avail) {
        take = avail;
    }
    if (take > 0) {
        memcpy(out + got, f->buf + f->pos, take);
        f->pos += take;
        got += take;
    }
    if (got < total) {
        f->flags |= F_EOF;
    }
    return got / size;
}

size_t fwrite(const void* ptr, size_t size, size_t nmemb, FILE* f) {
    if (!stream_ok(f)) {
        errno = EBADF;
        return 0;
    }
    if (size == 0 || nmemb == 0) {
        return 0;
    }
    if (nmemb > 0xFFFFFFFFu / size) {
        f->flags |= F_ERR;
        errno = EOVERFLOW;
        return 0;
    }
    if ((f->flags & F_WRITE) == 0) {
        f->flags |= F_ERR;
        errno = EBADF;
        return 0;
    }
    size_t total = size * nmemb;

    if (f->kind == K_STDOUT || f->kind == K_STDERR) {
        console_write((const unsigned char*)ptr, total);
        return nmemb;
    }

    if (f->flags & F_APPEND) {
        f->pos = f->size; /* append mode: every write goes to the end */
    }
    size_t end = f->pos + total;
    if (end < f->pos) {
        f->flags |= F_ERR;
        errno = EFBIG;
        return 0;
    }
    if (stream_reserve(f, end) < 0) {
        f->flags |= F_ERR;
        return 0;
    }
    if (f->pos > f->size) {
        /* Writing after a seek past the end: the gap reads as zeros. */
        memset(f->buf + f->size, 0, f->pos - f->size);
    }
    memcpy(f->buf + f->pos, ptr, total);
    f->pos = end;
    if (end > f->size) {
        f->size = end;
    }
    f->flags |= F_DIRTY;
    return nmemb;
}

int fputc(int c, FILE* f) {
    unsigned char ch = (unsigned char)c;
    return fwrite(&ch, 1, 1, f) == 1 ? (int)ch : EOF;
}

int putc(int c, FILE* f) {
    return fputc(c, f);
}

int fputs(const char* s, FILE* f) {
    size_t n = strlen(s);
    if (n == 0) {
        return 0;
    }
    return fwrite(s, 1, n, f) == n ? 0 : EOF;
}

char* fgets(char* s, int n, FILE* f) {
    if (n <= 0) {
        return NULL;
    }
    if (n == 1) {
        s[0] = '\0';
        return s;
    }
    int i = 0;
    while (i < n - 1) {
        int c = fgetc(f);
        if (c == EOF) {
            break;
        }
        s[i++] = (char)c;
        if (c == '\n') {
            break;
        }
    }
    if (i == 0) {
        return NULL; /* end of file (or error) before any character */
    }
    s[i] = '\0';
    return s;
}

int fseek(FILE* f, long offset, int whence) {
    if (!stream_ok(f)) {
        errno = EBADF;
        return -1;
    }
    if (f->kind != K_FILE) {
        errno = ESPIPE;
        return -1;
    }
    long base;
    if (whence == SEEK_SET) {
        base = 0;
    } else if (whence == SEEK_CUR) {
        base = (long)f->pos - (f->unget >= 0 ? 1 : 0);
    } else if (whence == SEEK_END) {
        base = (long)f->size;
    } else {
        errno = EINVAL;
        return -1;
    }
    /* base is in [0, 1 GiB], so the only overflow risk is a huge
     * positive offset. */
    if (offset > 0 && (unsigned long)offset > 0x7FFFFFFFul - (unsigned long)base) {
        errno = EINVAL;
        return -1;
    }
    long np = base + offset;
    if (np < 0) {
        errno = EINVAL;
        return -1;
    }
    f->pos = (size_t)np;
    f->unget = -1;
    f->flags &= ~F_EOF;
    return 0;
}

long ftell(FILE* f) {
    if (!stream_ok(f)) {
        errno = EBADF;
        return -1;
    }
    if (f->kind != K_FILE) {
        errno = ESPIPE;
        return -1;
    }
    return (long)f->pos - (f->unget >= 0 ? 1 : 0);
}

void rewind(FILE* f) {
    if (fseek(f, 0, SEEK_SET) == 0) {
        f->flags &= ~F_ERR;
    }
}

int feof(FILE* f) {
    return stream_ok(f) && (f->flags & F_EOF) != 0;
}

int ferror(FILE* f) {
    return stream_ok(f) && (f->flags & F_ERR) != 0;
}

void clearerr(FILE* f) {
    if (stream_ok(f)) {
        f->flags &= ~(F_EOF | F_ERR);
    }
}

int remove(const char* path) {
    if (check_name(path) < 0) {
        return -1;
    }
    int denied = 0;
    if (!probe_exists(path, &denied)) {
        errno = denied ? EACCES : ENOENT;
        return -1;
    }
    if (sys_delete_file(path) < 0) {
        errno = EACCES;
        return -1;
    }
    return 0;
}

/* ================================================================== *
 * printf family on top of the streams
 * ================================================================== */

int vfprintf(FILE* f, const char* fmt, va_list ap) {
    char small[BUFSIZ]; /* covers anything the old fixed-size printf
                            could print, with no heap use */
    va_list ap2;
    va_copy(ap2, ap);
    int n = vsnprintf(small, sizeof(small), fmt, ap);
    int rc;
    if (n < 0) {
        rc = n;
    } else if ((size_t)n < sizeof(small)) {
        rc = (fwrite(small, 1, (size_t)n, f) == (size_t)n || n == 0) ? n : -1;
    } else {
        char* big = (char*)malloc((size_t)n + 1);
        if (big == NULL) {
            rc = -1; /* errno == ENOMEM from malloc */
        } else {
            vsnprintf(big, (size_t)n + 1, fmt, ap2);
            rc = (fwrite(big, 1, (size_t)n, f) == (size_t)n) ? n : -1;
            free(big);
        }
    }
    va_end(ap2);
    return rc;
}

int vprintf(const char* fmt, va_list ap) {
    return vfprintf(stdout, fmt, ap);
}

int printf(const char* fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vfprintf(stdout, fmt, ap);
    va_end(ap);
    return n;
}

int fprintf(FILE* f, const char* fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    int n = vfprintf(f, fmt, ap);
    va_end(ap);
    return n;
}

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

void perror(const char* s) {
    int e = errno; /* the writes below must not be able to change it */
    if (s != NULL && *s != '\0') {
        fputs(s, stderr);
        fputs(": ", stderr);
    }
    fputs(strerror(e), stderr);
    fputc('\n', stderr);
}
