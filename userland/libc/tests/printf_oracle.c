/* printf_oracle.c - runs printf_cases.h through the HOST C library's
 * snprintf() and reports any case whose "expected" string it disagrees
 * with. See printf_cases.h. Built and run by run.sh. */
#include <stdio.h>
#include <string.h>
#include <stddef.h>

static int total, bad;

static void do_case(const char* expected, int line, const char* fmt, ...)
    __attribute__((format(printf, 3, 4)));
static void do_case(const char* expected, int line, const char* fmt, ...) {
    char buf[256];
    __builtin_va_list ap;
    __builtin_va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    __builtin_va_end(ap);
    total++;
    if (strcmp(buf, expected) != 0) {
        bad++;
        printf("ORACLE MISMATCH line %d: fmt=\"%s\" expected=\"%s\" host=\"%s\"\n",
               line, fmt, expected, buf);
    }
}
#define CASE(exp, ...) do_case(exp, __LINE__, __VA_ARGS__);

int main(void) {
#include "printf_cases.h"
    printf("printf oracle: %d cases, %d disagree with the host libc\n", total, bad);
    return bad != 0;
}
