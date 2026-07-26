#include "string.h"

void* memcpy(void* dest, const void* src, size_t n) {
    unsigned char* d = (unsigned char*)dest;
    const unsigned char* s = (const unsigned char*)src;
    for (size_t i = 0; i < n; i++) {
        d[i] = s[i];
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