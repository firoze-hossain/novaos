/*
 * dyntest2.c - Phase 74: DYNTEST2.ELF, a SECOND and different consumer
 * of DYNLIB.SO (see userland/dynlib/dynlib.c). Its purpose isn't just
 * "more of the same test" - it's the actual proof of the roadmap
 * task's own stated goal ("lets multiple programs share one on-disk
 * copy of a library instead of each statically duplicating it"): two
 * independently built executables, DYNTEST.ELF and this one, both
 * carry a DT_NEEDED reference to the one DYNLIB.SO file on disk
 * rather than each linking their own private copy of its code - see
 * PROGRESS.md's own Phase 74 entry for the file-size comparison that
 * confirms it.
 *
 * Deliberately exercises the library differently than dyntest.c does
 * (a running sum over many dyn_fib() calls, a CRC32 taken over that
 * sum's own bytes, chained dyn_apply() calls) so this is a real,
 * independent functional check, not a duplicate of the first one.
 */
#include <stdio.h>

extern int dyn_fib(int n);
extern unsigned int dyn_crc32(const void* data, unsigned int len);
extern int dyn_apply(int op, int x, int y);

static int failed_groups;

static void report(const char* name, int ok) {
    if (ok) {
        printf("[dyntest2] PASS: %s\n", name);
    } else {
        failed_groups++;
        printf("[dyntest2] FAIL: %s\n", name);
    }
}

int main(void) {
    /* Sum of dyn_fib(0..12) = 0+1+1+2+3+5+8+13+21+34+55+89+144 = 376 -
     * checked by hand, a different call pattern than dyntest.c's. */
    int sum = 0;
    for (int i = 0; i <= 12; i++) {
        sum += dyn_fib(i);
    }
    report("running sum of dyn_fib across many calls", sum == 376);

    /* A CRC32 taken over that sum's own 4 bytes (little-endian 376 =
     * 0x00000178), cross-checked against Python's zlib.crc32() while
     * writing this test. */
    unsigned char bytes[4] = { 0x78, 0x01, 0x00, 0x00 };
    unsigned int crc = dyn_crc32(bytes, 4);
    report("dyn_crc32 over computed data", crc == 0x8E0D3D58u);

    /* Chained dyn_apply() calls: (3+4)=7, then 7*7=49, then 49-9=40 -
     * exercises the R_386_RELATIVE dispatch table three times in a
     * row across independent calls. */
    int a = dyn_apply(0, 3, 4);
    int b = dyn_apply(2, a, a);
    int c = dyn_apply(1, b, 9);
    report("chained dyn_apply calls", a == 7 && b == 49 && c == 40);

    printf("[dyntest2] DONE: 3 groups, %d failed\n", failed_groups);
    return failed_groups;
}
