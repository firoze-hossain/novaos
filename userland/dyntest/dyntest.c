/*
 * dyntest.c - Phase 74: DYNTEST.ELF, the primary functional test of
 * real dynamic linking against DYNLIB.SO (see userland/dynlib/
 * dynlib.c). Declares DYNLIB.SO's functions with plain `extern`, no
 * different from how it would declare a function in a static library
 * - that ordinariness is the point: from this file's own perspective,
 * calling a dynamically-linked function looks exactly like calling
 * any other one. The difference is entirely in how it's BUILT (see
 * build.sh: -l:DYNLIB.SO instead of a statically-linked .o) and in
 * what the kernel does at exec time before this file's first
 * instruction ever runs (kernel/task/process.c's
 * load_and_link_shared_libraries(), kernel/rust/dynlink.rs).
 *
 * Every "[dyntest] PASS/FAIL: ..." line is checked individually by
 * tools/python/test_runner.py, the same convention userland/libctest/
 * established.
 */
#include <stdio.h>

extern int dyn_fib(int n);
extern unsigned int dyn_crc32(const void* data, unsigned int len);
extern int dyn_apply(int op, int x, int y);
extern const char* dyn_version(void);

static int failed_groups;

static void report(const char* name, int ok) {
    if (ok) {
        printf("[dyntest] PASS: %s\n", name);
    } else {
        failed_groups++;
        printf("[dyntest] FAIL: %s\n", name);
    }
}

int main(void) {
    /* Real function calls across the PLT/GOT boundary (R_386_JMP_SLOT,
     * resolved eagerly at exec time - see dynlink.rs). Known-correct
     * Fibonacci values, checked by hand. */
    int ok = dyn_fib(0) == 0 && dyn_fib(1) == 1 && dyn_fib(2) == 1 &&
              dyn_fib(10) == 55 && dyn_fib(20) == 6765 && dyn_fib(-5) == 0;
    report("dyn_fib across the shared-library boundary", ok);

    /* A real CRC32, cross-checked against Python's zlib.crc32() while
     * writing this test (see dynlib.c's own comment) - not merely
     * "returns a number," a specific, independently-verifiable one. */
    ok = dyn_crc32("", 0) == 0x00000000u &&
         dyn_crc32("a", 1) == 0xE8B7BE43u &&
         dyn_crc32("The quick brown fox jumps over the lazy dog", 43) ==
             0x414FA339u;
    report("dyn_crc32 across the shared-library boundary", ok);

    /* Calls dyn_apply(), which inside DYNLIB.SO itself calls through a
     * file-scope function-pointer dispatch table - the specific
     * mechanism that forces R_386_RELATIVE relocations to exist in
     * this library at all (see dynlib.c). A wrong entry in that table
     * (a relocation bug) would show up here as a WRONG RESULT, not
     * just a crash: a genuinely discriminating check, not just "it
     * didn't segfault." */
    ok = dyn_apply(0, 3, 4) == 7 &&   /* add */
         dyn_apply(1, 10, 3) == 7 &&  /* sub */
         dyn_apply(2, 6, 7) == 42 &&  /* mul */
         dyn_apply(3, 1, 1) == 0;     /* invalid op -> 0 */
    report("dyn_apply (R_386_RELATIVE dispatch table)", ok);

    /* A pointer INTO the shared library's own .rodata, returned across
     * the boundary and dereferenced here - proves same-image
     * code-to-data addressing survived relocation correctly, not just
     * that a function returned. */
    const char* v = dyn_version();
    ok = v != 0;
    for (const char* want = "DYNLIB 1.0 (Phase 74)"; ok && *want; want++, v++) {
        if (*v != *want) {
            ok = 0;
        }
    }
    report("dyn_version string pointer", ok);

    printf("[dyntest] DONE: 4 groups, %d failed\n", failed_groups);
    return failed_groups;
}
