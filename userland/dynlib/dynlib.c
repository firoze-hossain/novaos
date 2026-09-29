/*
 * dynlib.c - Phase 74: DYNLIB.SO, NovaOS's first real shared library.
 *
 * Not a toy - genuine, independently useful logic, built the same way
 * any future shared library on this project would be: -fPIC, -shared,
 * no libc of its own (freestanding, like every other userland object
 * here). See kernel/rust/dynlink.rs for the loader this depends on,
 * and its own module comment for exactly what kinds of relocations
 * this project's dynamic linker supports.
 *
 * Deliberately exercises every relocation type that loader supports,
 * so its own two consumer programs (userland/dyntest/,
 * userland/dyntest2/) double as a real functional test of all of
 * them, not just "a function call worked":
 *   - dyn_fib / dyn_crc32 / dyn_version: called across the PLT/GOT
 *     boundary from a consumer -> R_386_JMP_SLOT.
 *   - the dispatch table below is itself an array of function
 *     pointers inside this library's own .data, which is why calling
 *     dyn_apply() forces the linker to emit R_386_RELATIVE relocations
 *     for those pointers even though nothing outside this file ever
 *     touches the table directly.
 *   - dyn_version()'s return value is the address of a string literal
 *     living in .rodata, exercising ordinary same-image code-to-data
 *     addressing under -fPIC (typically R_386_32 against a symbol
 *     this same image defines, resolved locally with no external
 *     search - see dynlink.rs's own comment on that fast path).
 *
 * NOT exercised, on purpose: this library defines no external,
 * directly-referenced mutable global data symbol - i.e. nothing a
 * consumer could reach without going through a function - because
 * that is exactly what would need an R_386_COPY relocation, which
 * this project's loader deliberately does not support (see
 * dynlink.rs). counter below is file-scope static (never exported)
 * for exactly this reason.
 */

int dyn_fib(int n) {
    if (n < 0) {
        return 0;
    }
    int a = 0;
    int b = 1;
    for (int i = 0; i < n; i++) {
        int next = a + b;
        a = b;
        b = next;
    }
    return a;
}

/* A real CRC32 (the standard IEEE 802.3 polynomial, 0xEDB88320,
 * reflected/bit-at-a-time - no lookup table, so this stays simple and
 * doesn't need its own .rodata table just to prove a point the
 * dispatch table below already proves). Matches what `crc32` from
 * zlib/Python's binascii would compute for the same bytes - checked
 * against Python's own zlib.crc32() while writing this library's
 * tests, not merely assumed correct. */
unsigned int dyn_crc32(const void* data, unsigned int len) {
    const unsigned char* bytes = (const unsigned char*)data;
    unsigned int crc = 0xFFFFFFFFu;
    for (unsigned int i = 0; i < len; i++) {
        crc ^= bytes[i];
        for (int bit = 0; bit < 8; bit++) {
            unsigned int mask = 0u - (crc & 1u);
            crc = (crc >> 1) ^ (0xEDB88320u & mask);
        }
    }
    return ~crc;
}

static int op_add(int x, int y) { return x + y; }
static int op_sub(int x, int y) { return x - y; }
static int op_mul(int x, int y) { return x * y; }

typedef int (*binop_fn)(int, int);

/* File-scope, never exported - a pointer TABLE, not exported data
 * itself, so nothing outside this file ever addresses it directly.
 * The three function pointers inside it are absolute addresses within
 * THIS shared library, computed at compile time relative to a link
 * base of 0 and fixed up by R_386_RELATIVE relocations once this
 * library is actually loaded at its real runtime address - the one
 * relocation type every PIC shared object needs regardless of how
 * simple it otherwise is (.init_array/.fini_array need it too, even
 * for a library with no explicit global pointers like this one would
 * otherwise have). */
static binop_fn dispatch[3] = { op_add, op_sub, op_mul };

/* op: 0=add, 1=sub, 2=mul. Any other value returns 0. Called through
 * the dispatch table above, so a wrong entry in it (a relocation bug)
 * shows up as a wrong RESULT here, not just a crash - a genuinely
 * discriminating test. */
int dyn_apply(int op, int x, int y) {
    if (op < 0 || op > 2) {
        return 0;
    }
    return dispatch[op](x, y);
}

const char* dyn_version(void) {
    return "DYNLIB 1.0 (Phase 74)";
}
