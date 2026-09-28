/*
 * string.c - a small, standard subset of <string.h>. Written fresh
 * for userland rather than reusing kernel/lib/string.c - that file is
 * compiled into and only reachable from the kernel image; userland
 * programs are entirely separate binaries with no way to call into
 * kernel code except through syscalls.
 */
#include "string.h"
#include "errno.h"

size_t strlen(const char* s) {
    size_t len = 0;
    while (s[len]) {
        len++;
    }
    return len;
}

char* strcpy(char* dest, const char* src) {
    char* out = dest;
    while ((*dest++ = *src++)) {
    }
    return out;
}

char* strncpy(char* dest, const char* src, size_t n) {
    size_t i = 0;
    for (; i < n && src[i]; i++) {
        dest[i] = src[i];
    }
    for (; i < n; i++) {
        dest[i] = '\0';
    }
    return dest;
}

char* strcat(char* dest, const char* src) {
    char* out = dest;
    while (*dest) {
        dest++;
    }
    while ((*dest++ = *src++)) {
    }
    return out;
}

int strcmp(const char* a, const char* b) {
    while (*a && (*a == *b)) {
        a++;
        b++;
    }
    return (unsigned char)*a - (unsigned char)*b;
}

int strncmp(const char* a, const char* b, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (a[i] != b[i] || a[i] == '\0') {
            return (unsigned char)a[i] - (unsigned char)b[i];
        }
    }
    return 0;
}

char* strchr(const char* s, int c) {
    while (*s) {
        if (*s == (char)c) {
            return (char*)s;
        }
        s++;
    }
    return (c == '\0') ? (char*)s : NULL;
}

void* memcpy(void* dest, const void* src, size_t n) {
    unsigned char* d = (unsigned char*)dest;
    const unsigned char* s = (const unsigned char*)src;
    for (size_t i = 0; i < n; i++) {
        d[i] = s[i];
    }
    return dest;
}

void* memmove(void* dest, const void* src, size_t n) {
    unsigned char* d = (unsigned char*)dest;
    const unsigned char* s = (const unsigned char*)src;
    if (d < s) {
        for (size_t i = 0; i < n; i++) {
            d[i] = s[i];
        }
    } else if (d > s) {
        for (size_t i = n; i > 0; i--) {
            d[i - 1] = s[i - 1];
        }
    }
    return dest;
}

void* memset(void* dest, int value, size_t n) {
    unsigned char* d = (unsigned char*)dest;
    for (size_t i = 0; i < n; i++) {
        d[i] = (unsigned char)value;
    }
    return dest;
}

int memcmp(const void* a, const void* b, size_t n) {
    const unsigned char* pa = (const unsigned char*)a;
    const unsigned char* pb = (const unsigned char*)b;
    for (size_t i = 0; i < n; i++) {
        if (pa[i] != pb[i]) {
            return (int)pa[i] - (int)pb[i];
        }
    }
    return 0;
}

/* Phase 73: one message per errno.h constant, in the conventional
 * wording, plus the required "Success" for 0 and a catch-all so an
 * out-of-range value never returns NULL. Returned strings are
 * static - never freed, never modified. */
const char* strerror(int e) {
    switch (e) {
        case 0:            return "Success";
        case EPERM:        return "Operation not permitted";
        case ENOENT:       return "No such file or directory";
        case ESRCH:        return "No such process";
        case EINTR:        return "Interrupted";
        case EIO:          return "Input/output error";
        case ENXIO:        return "No such device or address";
        case E2BIG:        return "Argument list too long";
        case ENOEXEC:      return "Exec format error";
        case EBADF:        return "Bad file descriptor";
        case ECHILD:       return "No child processes";
        case EAGAIN:       return "Resource temporarily unavailable";
        case ENOMEM:       return "Out of memory";
        case EACCES:       return "Permission denied";
        case EFAULT:       return "Bad address";
        case EBUSY:        return "Resource busy";
        case EEXIST:       return "File exists";
        case ENODEV:       return "No such device";
        case ENOTDIR:      return "Not a directory";
        case EISDIR:       return "Is a directory";
        case EINVAL:       return "Invalid argument";
        case ENFILE:       return "Too many open files in system";
        case EMFILE:       return "Too many open files";
        case EFBIG:        return "File too large";
        case ENOSPC:       return "No space left on device";
        case ESPIPE:       return "Illegal seek";
        case EROFS:        return "Read-only file system";
        case EPIPE:        return "Broken pipe";
        case EDOM:         return "Numerical argument out of domain";
        case ERANGE:       return "Numerical result out of range";
        case ENAMETOOLONG: return "File name too long";
        case ENOSYS:       return "Function not implemented";
        case ENOTEMPTY:    return "Directory not empty";
        case EOVERFLOW:    return "Value too large for defined data type";
        default:           return "Unknown error";
    }
}
