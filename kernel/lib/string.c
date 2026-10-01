#include "string.h"

void* memcpy(void* dest, const void* src, size_t n) {
    unsigned char* d = (unsigned char*)dest;
    const unsigned char* s = (const unsigned char*)src;
    for (size_t i = 0; i < n; i++) {
        d[i] = s[i];
    }
    return dest;
}

/* Found missing, late: this kernel's own rustc-via-`cargo -Z build-
 * std` build path (Makefile's own "rustup found" branch - the
 * project's *preferred* path when a nightly toolchain is available,
 * taken by any real contributor's machine that has one) builds
 * compiler_builtins from source without its optional `mem` feature,
 * which is what would otherwise provide memcpy/memmove/memset/memcmp
 * for a freestanding target with no libc. memcpy/memset/memcmp above
 * already covered rustc's own occasional calls into them; memmove was
 * the one gap - genuinely missing, not merely unused, confirmed by a
 * real `undefined reference to 'memmove'` link failure the moment a
 * Rust module's compiled output needed it (kernel/rust/udp.rs's own
 * rust_udp_recv()/rust_udp_recvfrom(), whose copy_from_slice() calls
 * rustc chose to lower to a memmove call rather than inlining or
 * emitting memcpy - not something this kernel's own C code had ever
 * needed to call by this name either, which is exactly why this went
 * unnoticed until a real build on a real machine with the nightly
 * toolchain this project's own Makefile prefers actually hit it).
 *
 * Unlike memcpy above (undefined behaviour if the regions overlap -
 * every existing call site in this kernel already avoided that),
 * memmove must copy correctly even when they do: forward (low
 * address first) when it's safe to, backward (high address first)
 * when copying forward would overwrite source bytes not yet read.
 * The standard, textbook approach - not an optimized one (no bulk
 * word-at-a-time copying, matching memcpy/memset's own equally plain
 * byte-at-a-time style above), correct over fast. */
void* memmove(void* dest, const void* src, size_t n) {
    unsigned char* d = (unsigned char*)dest;
    const unsigned char* s = (const unsigned char*)src;
    if (d == s || n == 0) {
        return dest;
    }
    if (d < s) {
        for (size_t i = 0; i < n; i++) {
            d[i] = s[i];
        }
    } else {
        for (size_t i = n; i > 0; i--) {
            d[i - 1] = s[i - 1];
        }
    }
    return dest;
}

void* memset(void* s, int c, size_t n) {
    unsigned char* p = (unsigned char*)s;
    for (size_t i = 0; i < n; i++) {
        p[i] = (unsigned char)c;
    }
    return s;
}

size_t strlen(const char* s) {
    size_t len = 0;
    while (s[len]) len++;
    return len;
}

char* strcpy(char* dest, const char* src) {
    char* d = dest;
    while (*src) {
        *d++ = *src++;
    }
    *d = '\0';
    return dest;
}

int strcmp(const char* s1, const char* s2) {
    while (*s1 && (*s1 == *s2)) {
        s1++;
        s2++;
    }
    return *(unsigned char*)s1 - *(unsigned char*)s2;
}

int strncmp(const char* s1, const char* s2, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (s1[i] != s2[i] || s1[i] == '\0') {
            return (unsigned char)s1[i] - (unsigned char)s2[i];
        }
    }
    return 0;
}

int memcmp(const void* s1, const void* s2, size_t n) {
    const unsigned char* a = (const unsigned char*)s1;
    const unsigned char* b = (const unsigned char*)s2;
    for (size_t i = 0; i < n; i++) {
        if (a[i] != b[i]) {
            return a[i] - b[i];
        }
    }
    return 0;
}

char* strchr(const char* s, int c) {
    while (*s) {
        if (*s == c) return (char*)s;
        s++;
    }
    return NULL;
}

void utoa(unsigned int num, char* str, int base) {
    char digits[] = "0123456789ABCDEF";
    char temp[33];
    int i = 0;

    if (num == 0) {
        str[0] = '0';
        str[1] = '\0';
        return;
    }

    while (num > 0 && i < 32) {
        temp[i++] = digits[num % (unsigned int)base];
        num /= (unsigned int)base;
    }

    for (int j = 0; j < i; j++) {
        str[j] = temp[i - 1 - j];
    }
    str[i] = '\0';
}

void itoa(int num, char* str, int base) {
    /* Base 10 is the only base where a leading '-' and true magnitude
     * make sense; every other base (in practice, just 16 for %x) is
     * used for addresses and bit patterns, where the two's-complement
     * *unsigned* reading is what callers actually want (and is what
     * every other C library's %x does). Splitting on base, rather than
     * on "is num negative", is what makes utoa's own `num > 0` loop
     * safe for values like 0xDEADB000 - as a signed int that's
     * negative, and a naive "if negative, negate" would both produce
     * the wrong digits and overflow on INT_MIN. */
    if (base == 10 && num < 0) {
        str[0] = '-';
        /* -(num + 1) + 1, all in signed range, then a single unsigned
         * cast: avoids UB from negating INT_MIN directly (-INT_MIN
         * overflows a 32-bit signed int). */
        unsigned int magnitude = (unsigned int)(-(num + 1)) + 1u;
        utoa(magnitude, str + 1, base);
        return;
    }
    utoa((unsigned int)num, str, base);
}

int isdigit(int c) {
    return c >= '0' && c <= '9';
}

int isspace(int c) {
    return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\f' || c == '\v';
}