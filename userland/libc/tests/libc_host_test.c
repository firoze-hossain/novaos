/*
 * libc_host_test.c - tests NovaOS's real libc sources (stdlib.c,
 * stdio.c, string.c - #included below, so the tests can also inspect
 * their internal state) against a FAKE KERNEL, as an ordinary 32-bit
 * static Linux program. See run.sh for how it is built and run.
 *
 * Why this exists next to the in-OS test (userland/libctest/): the
 * allocator, printf formatter, environment code and FILE layer are pure
 * logic sitting on a handful of syscalls. Here those syscalls are
 * replaced by a fake that mimics the kernel's REAL semantics (create-
 * only writes, SYS_OPEN that never checks existence, capability
 * denial, a per-call read cap), so the logic can be stress-tested
 * hundreds of thousands of times in well under a second - something a
 * QEMU boot cannot do - and every internal invariant can be checked
 * after every single step. The in-OS test then proves the same code
 * works against the genuine kernel.
 *
 * Freestanding: no host libc is linked. Output goes through raw Linux
 * write(2)/exit(2) via `int 0x80`.
 */
#include "../stdlib.c"
#include "../stdio.c"
#include "../string.c"

/* ------------------------------------------------------------------ *
 * Raw Linux i386 output (the harness's only contact with the host)
 * ------------------------------------------------------------------ */
static long lx3(long n, long a, long b, long c) {
    long r;
    __asm__ volatile ("int $0x80" : "=a"(r) : "a"(n), "b"(a), "c"(b), "d"(c)
                      : "memory");
    return r;
}

static void hout(const char* s) {
    lx3(4, 1, (long)s, (long)strlen(s));
}

static void hout_int(long v) {
    char b[16];
    int i = 15;
    b[i] = '\0';
    unsigned long u = v < 0 ? (unsigned long)(-v) : (unsigned long)v;
    do {
        b[--i] = (char)('0' + u % 10);
        u /= 10;
    } while (u != 0);
    if (v < 0) {
        b[--i] = '-';
    }
    hout(&b[i]);
}

static int checks, failures;

static void report_fail(int line, const char* expr) {
    failures++;
    hout("  FAIL line ");
    hout_int(line);
    hout(": ");
    hout(expr);
    hout("\n");
}

#define CHECK(c) do { checks++; if (!(c)) report_fail(__LINE__, #c); } while (0)
static int str_eq(const char* a, const char* b) {
    return a != NULL && b != NULL && strcmp(a, b) == 0;
}
/* Takes the strings as function arguments (not inline in the macro) so
 * comparing an array to NULL never appears in the expansion. */
static void check_str(int line, const char* expr, const char* got,
                      const char* want) {
    checks++;
    if (!str_eq(got, want)) {
        report_fail(line, expr);
        hout("    got: \"");
        hout(got != NULL ? got : "(null)");
        hout("\"\n");
    }
}
#define CHECK_STR(a, b) check_str(__LINE__, #a " == " #b, (a), (b))

/* ------------------------------------------------------------------ *
 * Fake kernel
 * ------------------------------------------------------------------ */

/* --- heap: SYS_SBRK over a static arena --- */
static char arena[96u * 1024 * 1024] __attribute__((aligned(4096)));
static size_t arena_used;
static int fail_sbrk;      /* when set, every growing sbrk fails */
static long sbrk_calls;

void* sys_sbrk(int inc) {
    sbrk_calls++;
    if (inc < 0) {
        return (void*)-1; /* like the real kernel: heap never shrinks */
    }
    if (inc > 0 && fail_sbrk) {
        return (void*)-1;
    }
    if ((size_t)inc > sizeof(arena) - arena_used) {
        return (void*)-1;
    }
    void* old = arena + arena_used;
    arena_used += (size_t)inc;
    return old;
}

/* --- console: SYS_WRITE captured into a buffer --- */
static char console[256 * 1024];
static size_t console_len;

int sys_write(const char* s) {
    size_t n = strlen(s);
    if (console_len + n < sizeof(console)) {
        memcpy(console + console_len, s, n);
        console_len += n;
        console[console_len] = '\0';
    }
    return 1;
}

/* --- keyboard: SYS_READ_KEY replays a script; -1 = "no key yet" --- */
static int key_script[64];
static int key_count, key_index;
static int yield_count;

int sys_read_key(void) {
    if (key_index < key_count) {
        return key_script[key_index++];
    }
    return -1;
}

void sys_yield(void) {
    yield_count++;
}

/* --- exit: recorded, then a longjmp back into the test that called
 *     exit(), so exit()'s real code path can be tested --- */
static void* exit_jb[5];
static int exit_code_seen;
static int exit_hook_armed;

void sys_exit(int code) {
    exit_code_seen = code;
    if (exit_hook_armed) {
        __builtin_longjmp(exit_jb, 1);
    }
    lx3(1, code, 0, 0); /* a stray exit() outside a test: really leave */
    for (;;) { }
}

/* --- filesystem: mimics the kernel's file syscalls, including its
 *     limits: SYS_OPEN never checks existence, only the first 12
 *     characters of a name are kept, writes are CREATE-ONLY (fail if the
 *     name exists), and a single SYS_READ returns at most 4096 bytes --- */
#define FS_MAX 32
typedef struct {
    int used;
    char name[16];
    unsigned char* data;
    unsigned size;
} fs_file;
static fs_file fsx[FS_MAX];
static unsigned char fs_pool[8u * 1024 * 1024];
static size_t fs_pool_used;
static int fs_can_open = 1;   /* the process may open files at all */
static int fs_can_write = 1;  /* can_open_any_file: create/delete allowed */
static int fs_read_calls;
static int fs_writes, fs_deletes;   /* how often the disk was really touched */

typedef struct { int used; char name[16]; unsigned off; } fs_handle;
static fs_handle handles[8];  /* the kernel's real table has 8 slots */

static char up(char c) { return (c >= 'a' && c <= 'z') ? (char)(c - 32) : c; }

static int name_eq(const char* a, const char* b) {
    while (*a && *b) {
        if (up(*a) != up(*b)) return 0;
        a++; b++;
    }
    return *a == '\0' && *b == '\0';
}

static fs_file* fs_find(const char* name) {
    for (int i = 0; i < FS_MAX; i++) {
        if (fsx[i].used && name_eq(fsx[i].name, name)) return &fsx[i];
    }
    return NULL;
}

static void fs_put(const char* name, const void* data, unsigned n) {
    for (int i = 0; i < FS_MAX; i++) {
        if (!fsx[i].used) {
            fsx[i].used = 1;
            strncpy(fsx[i].name, name, 15);
            fsx[i].data = fs_pool + fs_pool_used;
            fs_pool_used += n + 1;
            memcpy(fsx[i].data, data, n);
            fsx[i].size = n;
            return;
        }
    }
}

static int handles_in_use(void) {
    int n = 0;
    for (int i = 0; i < 8; i++) n += handles[i].used;
    return n;
}

int sys_open(const char* filename) {
    if (!fs_can_open) return -1;
    for (int i = 0; i < 8; i++) {
        if (!handles[i].used) {
            handles[i].used = 1;
            handles[i].off = 0;
            size_t k = 0;
            while (filename[k] && k < 12) { /* the kernel keeps 12 chars */
                handles[i].name[k] = filename[k];
                k++;
            }
            handles[i].name[k] = '\0';
            return i;
        }
    }
    return -1; /* table full */
}

int sys_read(int h, void* buf, int max_len) {
    if (h < 0 || h >= 8 || !handles[h].used) return -1;
    fs_read_calls++;
    fs_file* f = fs_find(handles[h].name);
    if (!f) return -1; /* lazily discovered: the file does not exist */
    unsigned avail = f->size > handles[h].off ? f->size - handles[h].off : 0;
    unsigned k = (unsigned)max_len < avail ? (unsigned)max_len : avail;
    if (k > 4096) k = 4096; /* per-call cap, like the fixed scratch buffer */
    memcpy(buf, f->data + handles[h].off, k);
    handles[h].off += k;
    return (int)k;
}

void sys_close(int h) {
    if (h >= 0 && h < 8) handles[h].used = 0;
}

int sys_write_file(const char* filename, const void* data, unsigned int size) {
    fs_writes++;
    if (!fs_can_write) return -1;
    if (fs_find(filename)) return -1; /* create-only: fails if it exists */
    fs_put(filename, data, size);
    return 1;
}

int sys_delete_file(const char* filename) {
    fs_deletes++;
    if (!fs_can_write) return -1;
    fs_file* f = fs_find(filename);
    if (!f) return -1;
    f->used = 0;
    return 1;
}

/* ------------------------------------------------------------------ *
 * Test plumbing
 * ------------------------------------------------------------------ */

static void console_reset(void) {
    console_len = 0;
    console[0] = '\0';
}

/* Puts every piece of state the tests touch back to "freshly started". */
static void reset_world(void) {
    heap_head = heap_tail = NULL;
    arena_used = 0;
    fail_sbrk = 0;
    env_vec = NULL;
    env_own = NULL;
    env_cap = 0;
    environ = NULL;
    errno = 0;
    console_reset();
    for (int i = 0; i < FS_MAX; i++) fsx[i].used = 0;
    fs_pool_used = 0;
    fs_can_open = fs_can_write = 1;
    fs_read_calls = 0;
    fs_writes = fs_deletes = 0;
    for (int i = 0; i < 8; i++) handles[i].used = 0;
    for (int i = 0; i < FOPEN_MAX; i++) open_streams[i] = NULL;
    stdin_obj.flags = F_OPEN | F_READ;
    stdout_obj.flags = F_OPEN | F_WRITE;
    stderr_obj.flags = F_OPEN | F_WRITE;
    key_count = key_index = yield_count = 0;
    atexit_count = 0;
    exit_flush_registered = 0;
}

/* Every allocator invariant from stdlib.c's header comment. Returns 0
 * if all hold, else the number of the one that failed. */
static int heap_check(void) {
    block_t* prev = NULL;
    for (block_t* b = heap_head; b != NULL; b = b->next) {
        if (b->prev != prev) return 1;
        if (b->magic != MAGIC_ALLOC && b->magic != MAGIC_FREE) return 2;
        if (b->size < 16 || (b->size & 15) != 0) return 3;
        if (((uintptr_t)(b + 1) & 15) != 0) return 4;
        if (prev != NULL) {
            if ((char*)b < block_end(prev)) return 5;      /* overlap/order */
            if (prev->magic == MAGIC_FREE && b->magic == MAGIC_FREE &&
                blocks_adjacent(prev, b)) return 6;         /* unmerged */
        }
        prev = b;
    }
    if (prev != heap_tail) return 7;
    if (heap_tail != NULL && heap_tail->next != NULL) return 8;
    return 0;
}

/* Deterministic pseudo-random numbers (xorshift32). */
static unsigned rng_state = 2463534242u;
static unsigned rnd(void) {
    unsigned x = rng_state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    rng_state = x;
    return x;
}

static void fill_pattern(unsigned char* p, size_t n, unsigned seed) {
    for (size_t i = 0; i < n; i++) p[i] = (unsigned char)(seed * 31u + i * 7u + (i >> 8));
}

static int pattern_ok(const unsigned char* p, size_t n, unsigned seed) {
    for (size_t i = 0; i < n; i++) {
        if (p[i] != (unsigned char)(seed * 31u + i * 7u + (i >> 8))) return 0;
    }
    return 1;
}

static const char* file_contents(const char* name, unsigned* size) {
    fs_file* f = fs_find(name);
    if (!f) { *size = 0; return NULL; }
    *size = f->size;
    return (const char*)f->data;
}

static int file_is(const char* name, const char* expect) {
    unsigned n;
    const char* d = file_contents(name, &n);
    return d != NULL && n == strlen(expect) && memcmp(d, expect, n) == 0;
}

/* ------------------------------------------------------------------ *
 * Tests: allocator
 * ------------------------------------------------------------------ */

static void test_alloc_basic(void) {
    hout("allocator: basics\n");
    reset_world();

    void* a = malloc(1);
    CHECK(a != NULL);
    CHECK(((uintptr_t)a & 15) == 0);
    void* z = malloc(0);
    CHECK(z != NULL && z != a);
    CHECK(malloc(0x40000001u) == NULL && errno == ENOMEM);
    errno = 0;
    CHECK(malloc((size_t)-1) == NULL && errno == ENOMEM);
    CHECK(heap_check() == 0);

    unsigned char* p = (unsigned char*)malloc(100);
    memset(p, 0xAB, 100);
    CHECK(p[0] == 0xAB && p[99] == 0xAB);

    /* free() of things that are not live allocations must be refused,
     * loudly, without crashing or corrupting anything. */
    console_reset();
    free(NULL);
    CHECK(console_len == 0);
    int on_stack;
    free(&on_stack);                 /* a stack address is not ours */
    CHECK(console_len > 0);
    console_reset();
    free((void*)0x1234);
    CHECK(console_len > 0);
    console_reset();
    free(p);
    CHECK(console_len == 0);
    free(p); /* double free */
    CHECK(console_len > 0);
    CHECK(heap_check() == 0);

    /* Adjacent frees coalesce: a+b become one block big enough for d. */
    reset_world();
    void* x = malloc(100);
    void* y = malloc(100);
    void* w = malloc(100);
    (void)w;
    free(y);
    free(x);
    CHECK(heap_check() == 0);
    void* d = malloc(200);
    CHECK(d == x);                       /* fits only because they merged */
    CHECK(heap_check() == 0);

    /* Splitting: a big freed block is carved, the rest stays usable. */
    reset_world();
    void* big = malloc(1000);
    void* guard = malloc(16);
    (void)guard;
    free(big);
    void* small = malloc(10);
    CHECK(small == big);
    void* rest = malloc(900);
    CHECK(rest != NULL && (char*)rest > (char*)small &&
          (char*)rest < (char*)small + 1000);
    CHECK(heap_check() == 0);

    /* A free block at the end of the heap is grown in place, not
     * stranded next to a fresh one. */
    reset_world();
    void* t1 = malloc(64);
    (void)t1;
    void* t2 = malloc(64);
    free(t2);
    size_t before = arena_used;
    void* t3 = malloc(500);            /* 500 rounds up to 512 */
    CHECK(t3 == t2);                   /* the free tail was reused... */
    CHECK(arena_used - before == 512 - 64);   /* ...and grown by exactly
                                                  the shortfall, no new
                                                  header, nothing stranded */
    CHECK(heap_check() == 0);
}

static void test_realloc(void) {
    hout("allocator: realloc / calloc\n");
    reset_world();

    /* realloc(NULL, n) == malloc(n) */
    unsigned char* p = (unsigned char*)realloc(NULL, 50);
    CHECK(p != NULL && ((uintptr_t)p & 15) == 0);
    fill_pattern(p, 50, 1);

    /* grow: content preserved */
    unsigned char* q = (unsigned char*)realloc(p, 5000);
    CHECK(q != NULL && pattern_ok(q, 50, 1));
    CHECK(heap_check() == 0);

    /* shrink in place: same pointer, content preserved */
    unsigned char* r = (unsigned char*)realloc(q, 20);
    CHECK(r == q && pattern_ok(r, 20, 1));
    CHECK(heap_check() == 0);

    /* same size: same pointer */
    CHECK(realloc(r, 20) == r);

    /* realloc(p, 0) frees */
    console_reset();
    CHECK(realloc(r, 0) == NULL);
    free(r);                       /* now a double free */
    CHECK(console_len > 0);

    /* grow in place by swallowing a free neighbour */
    reset_world();
    unsigned char* a = (unsigned char*)malloc(100);
    unsigned char* b = (unsigned char*)malloc(300);
    unsigned char* c = (unsigned char*)malloc(100);
    (void)c;
    fill_pattern(a, 100, 9);
    free(b);
    unsigned char* a2 = (unsigned char*)realloc(a, 350);
    CHECK(a2 == a && pattern_ok(a2, 100, 9));
    CHECK(heap_check() == 0);

    /* grow the LAST block in place by moving the break */
    reset_world();
    unsigned char* l1 = (unsigned char*)malloc(64);
    unsigned char* l2 = (unsigned char*)malloc(64);
    (void)l1;
    fill_pattern(l2, 64, 4);
    size_t used_before = arena_used;
    unsigned char* l3 = (unsigned char*)realloc(l2, 4096);
    CHECK(l3 == l2 && pattern_ok(l3, 64, 4));
    CHECK(arena_used > used_before);
    CHECK(heap_check() == 0);

    /* blocked on both sides: must move, old content intact, old block
     * reusable afterwards */
    reset_world();
    unsigned char* m1 = (unsigned char*)malloc(100);
    unsigned char* m2 = (unsigned char*)malloc(100);
    fill_pattern(m1, 100, 5);
    unsigned char* m3 = (unsigned char*)realloc(m1, 1000);
    CHECK(m3 != NULL && m3 != m1 && pattern_ok(m3, 100, 5));
    void* reuse = malloc(100);
    CHECK(reuse == m1);            /* the vacated block is reused */
    (void)m2;
    CHECK(heap_check() == 0);

    /* failure leaves the original valid and untouched */
    reset_world();
    unsigned char* f1 = (unsigned char*)malloc(100);
    unsigned char* f2 = (unsigned char*)malloc(100);
    (void)f2;
    fill_pattern(f1, 100, 6);
    fail_sbrk = 1;
    errno = 0;
    CHECK(realloc(f1, 100000) == NULL);
    CHECK(errno == ENOMEM);
    CHECK(pattern_ok(f1, 100, 6));
    fail_sbrk = 0;
    CHECK(realloc(f1, 200) != NULL);     /* still a valid live block */
    CHECK(heap_check() == 0);

    /* realloc of garbage / freed pointers */
    reset_world();
    void* g = malloc(32);
    free(g);
    errno = 0;
    CHECK(realloc(g, 64) == NULL && errno == EINVAL);
    errno = 0;
    CHECK(realloc((void*)0x1234, 64) == NULL && errno == EINVAL);

    /* calloc */
    reset_world();
    unsigned char* dirty = (unsigned char*)malloc(64);
    memset(dirty, 0xAA, 64);
    free(dirty);
    unsigned char* z = (unsigned char*)calloc(8, 8);
    CHECK(z == dirty);                       /* reused the dirty block */
    int allzero = 1;
    for (int i = 0; i < 64; i++) if (z[i] != 0) allzero = 0;
    CHECK(allzero);
    errno = 0;
    CHECK(calloc(65536, 65537) == NULL && errno == ENOMEM);   /* overflow */
    CHECK(calloc(0, 5) != NULL);
    CHECK(calloc(5, 0) != NULL);
    CHECK(heap_check() == 0);
}

typedef struct { unsigned char* p; size_t n; unsigned seed; } slot_t;

static void stress_once(unsigned seed, int iterations) {
    reset_world();
    rng_state = seed;
    enum { NSLOTS = 200 };
    static slot_t slots[NSLOTS];
    memset(slots, 0, sizeof(slots));
    int heap_bad = 0, corrupt = 0, unaligned = 0, failed_alloc = 0;

    for (int it = 0; it < iterations; it++) {
        unsigned idx = rnd() % NSLOTS;
        slot_t* s = &slots[idx];
        unsigned op = rnd() % 100;

        size_t want;
        unsigned sz = rnd() % 100;
        if (sz < 70) want = 1 + rnd() % 200;
        else if (sz < 95) want = 1 + rnd() % 3000;
        else want = 1 + rnd() % 40000;

        if (s->p == NULL) {
            s->p = (unsigned char*)malloc(want);
            if (s->p == NULL) { failed_alloc++; continue; }
            s->n = want;
            s->seed = rnd();
            fill_pattern(s->p, s->n, s->seed);
        } else if (op < 30) {
            if (!pattern_ok(s->p, s->n, s->seed)) corrupt++;
            free(s->p);
            s->p = NULL;
        } else if (op < 85) {
            if (!pattern_ok(s->p, s->n, s->seed)) corrupt++;
            /* occasionally make the heap unable to grow, so the failure
             * path (original must survive) is exercised too */
            int inject = (rnd() % 20) == 0;
            fail_sbrk = inject;
            size_t before_n = s->n;
            unsigned char* np = (unsigned char*)realloc(s->p, want);
            fail_sbrk = 0;
            if (np == NULL) {
                if (!pattern_ok(s->p, before_n, s->seed)) corrupt++;
            } else {
                size_t keep = want < before_n ? want : before_n;
                if (!pattern_ok(np, keep, s->seed)) corrupt++;
                s->p = np;
                s->n = want;
                s->seed = rnd();
                fill_pattern(s->p, s->n, s->seed);
            }
        } else {
            if (!pattern_ok(s->p, s->n, s->seed)) corrupt++;
        }
        if (s->p != NULL && ((uintptr_t)s->p & 15) != 0) unaligned++;
        if (heap_check() != 0) heap_bad++;

        if ((it & 1023) == 0) {
            for (int k = 0; k < NSLOTS; k++)
                if (slots[k].p && !pattern_ok(slots[k].p, slots[k].n, slots[k].seed))
                    corrupt++;
        }
    }
    for (int k = 0; k < NSLOTS; k++) {
        if (slots[k].p) {
            if (!pattern_ok(slots[k].p, slots[k].n, slots[k].seed)) corrupt++;
            free(slots[k].p);
        }
    }
    CHECK(heap_bad == 0);
    CHECK(corrupt == 0);
    CHECK(unaligned == 0);
    CHECK(heap_check() == 0);
    /* With everything freed, coalescing must have collapsed the heap to
     * (at most) a single free block. */
    int blocks = 0;
    for (block_t* b = heap_head; b; b = b->next) blocks++;
    CHECK(blocks <= 1);
    (void)failed_alloc;
}

static void test_alloc_stress(void) {
    hout("allocator: randomized stress (invariants checked every step)\n");
    stress_once(1u, 60000);
    stress_once(0xC0FFEEu, 60000);
    stress_once(0xBADF00Du, 60000);
}

/* ------------------------------------------------------------------ *
 * Tests: atexit / exit / errno / strerror
 * ------------------------------------------------------------------ */

static int order_log[8];
static int order_n;
static void h1(void) { order_log[order_n++] = 1; }
static void h2(void) { order_log[order_n++] = 2; }
static void h3(void) { order_log[order_n++] = 3; }

static void test_exit_errno(void) {
    hout("atexit / exit / errno / strerror\n");
    reset_world();
    order_n = 0;
    CHECK(atexit(h1) == 0);
    CHECK(atexit(h2) == 0);
    CHECK(atexit(h3) == 0);
    CHECK(atexit(NULL) == -1);
    exit_hook_armed = 1;
    if (__builtin_setjmp(exit_jb) == 0) {
        exit(42);
    }
    exit_hook_armed = 0;
    CHECK(exit_code_seen == 42);
    CHECK(order_n == 3 && order_log[0] == 3 && order_log[1] == 2 &&
          order_log[2] == 1);                       /* last in, first out */

    reset_world();
    int ok = 1;
    for (int i = 0; i < ATEXIT_MAX; i++) ok &= (atexit(h1) == 0);
    CHECK(ok);
    CHECK(atexit(h1) == -1);                        /* table full */

    CHECK_STR(strerror(0), "Success");
    CHECK_STR(strerror(ENOENT), "No such file or directory");
    CHECK_STR(strerror(EACCES), "Permission denied");
    CHECK_STR(strerror(ENOMEM), "Out of memory");
    CHECK_STR(strerror(99999), "Unknown error");
    CHECK_STR(strerror(-5), "Unknown error");
    /* every constant in errno.h has a real message */
    static const int all[] = { EPERM, ENOENT, ESRCH, EINTR, EIO, ENXIO, E2BIG,
        ENOEXEC, EBADF, ECHILD, EAGAIN, ENOMEM, EACCES, EFAULT, EBUSY, EEXIST,
        ENODEV, ENOTDIR, EISDIR, EINVAL, ENFILE, EMFILE, EFBIG, ENOSPC, ESPIPE,
        EROFS, EPIPE, EDOM, ERANGE, ENAMETOOLONG, ENOSYS, ENOTEMPTY, EOVERFLOW };
    int all_have = 1;
    for (unsigned i = 0; i < sizeof(all) / sizeof(all[0]); i++)
        if (strcmp(strerror(all[i]), "Unknown error") == 0) all_have = 0;
    CHECK(all_have);
    /* the numeric values are the standard POSIX ones */
    CHECK(ENOENT == 2 && EACCES == 13 && EINVAL == 22 && ENOMEM == 12 &&
          EEXIST == 17 && ERANGE == 34);
}

/* ------------------------------------------------------------------ *
 * Tests: environment
 * ------------------------------------------------------------------ */

static char* kernel_env[] = { "PATH=/bin", "HOME=/", "EMPTY=", "DUP=1", "DUP=2", 0 };

static int env_count_now(void) {
    int n = 0;
    if (environ) while (environ[n]) n++;
    return n;
}

static void test_environment(void) {
    hout("environment\n");
    reset_world();
    environ = kernel_env;

    /* reading never adopts or copies anything */
    CHECK_STR(getenv("PATH"), "/bin");
    CHECK_STR(getenv("HOME"), "/");
    CHECK_STR(getenv("EMPTY"), "");
    CHECK(getenv("NOPE") == NULL);
    CHECK(getenv("PAT") == NULL);            /* a prefix is not a match */
    CHECK(getenv("PATHX") == NULL);
    CHECK(getenv("") == NULL);
    CHECK(getenv("A=B") == NULL);
    CHECK(getenv(NULL) == NULL);
    CHECK(environ == kernel_env);
    CHECK(arena_used == 0);                  /* getenv allocated nothing */
    CHECK_STR(getenv("DUP"), "1");           /* first match wins */

    /* setenv: overwrite, no-overwrite, new */
    CHECK(setenv("PATH", "/usr/bin", 0) == 0);
    CHECK_STR(getenv("PATH"), "/bin");       /* left alone */
    CHECK(setenv("PATH", "/usr/bin", 1) == 0);
    CHECK_STR(getenv("PATH"), "/usr/bin");
    CHECK(setenv("NEWVAR", "value=with=equals", 1) == 0);
    CHECK_STR(getenv("NEWVAR"), "value=with=equals");
    CHECK(setenv("EMPTYV", "", 1) == 0);
    CHECK_STR(getenv("EMPTYV"), "");
    CHECK(environ != kernel_env);            /* adopted on first mutation */
    CHECK_STR(kernel_env[0], "PATH=/bin");   /* kernel's array untouched */
    CHECK(env_count_now() == 7);
    CHECK(environ[env_count_now()] == NULL); /* still NULL-terminated */

    /* invalid arguments */
    errno = 0; CHECK(setenv(NULL, "x", 1) == -1 && errno == EINVAL);
    errno = 0; CHECK(setenv("", "x", 1) == -1 && errno == EINVAL);
    errno = 0; CHECK(setenv("A=B", "x", 1) == -1 && errno == EINVAL);
    errno = 0; CHECK(setenv("A", NULL, 1) == -1 && errno == EINVAL);
    errno = 0; CHECK(unsetenv("") == -1 && errno == EINVAL);
    errno = 0; CHECK(unsetenv("A=B") == -1 && errno == EINVAL);

    /* aliasing: value points into the entry being replaced */
    CHECK(setenv("NEWVAR", getenv("NEWVAR"), 1) == 0);
    CHECK_STR(getenv("NEWVAR"), "value=with=equals");

    /* unsetenv: removes ALL duplicates, and is fine when absent */
    CHECK(unsetenv("DUP") == 0);
    CHECK(getenv("DUP") == NULL);
    CHECK(unsetenv("DUP") == 0);
    CHECK(unsetenv("NEVER_SET") == 0);
    CHECK_STR(getenv("HOME"), "/");          /* neighbours survive */
    CHECK(environ[env_count_now()] == NULL);
    CHECK(heap_check() == 0);

    /* putenv: caller's string becomes the entry, not a copy */
    static char mine[] = "PUT=one";
    CHECK(putenv(mine) == 0);
    CHECK_STR(getenv("PUT"), "one");
    mine[4] = 'X';
    CHECK_STR(getenv("PUT"), "Xne");         /* edits show through */
    static char mine2[] = "PUT=two";
    CHECK(putenv(mine2) == 0);
    CHECK_STR(getenv("PUT"), "two");
    CHECK(putenv("PUT") == 0);               /* "NAME" alone removes it */
    CHECK(getenv("PUT") == NULL);
    errno = 0; CHECK(putenv("=nope") == -1 && errno == EINVAL);
    errno = 0; CHECK(putenv("") == -1 && errno == EINVAL);
    errno = 0; CHECK(putenv(NULL) == -1 && errno == EINVAL);
    /* setenv over a putenv'd (not-owned) entry must not free the
     * caller's static string - that would be a wild free */
    static char mine3[] = "OWN=a";
    CHECK(putenv(mine3) == 0);
    console_reset();
    CHECK(setenv("OWN", "b", 1) == 0);
    CHECK(console_len == 0);
    CHECK_STR(getenv("OWN"), "b");
    CHECK_STR(mine3, "OWN=a");

    /* clearenv */
    CHECK(clearenv() == 0);
    CHECK(environ != NULL && environ[0] == NULL);
    CHECK(getenv("PATH") == NULL);
    CHECK(setenv("AFTER", "clear", 1) == 0);
    CHECK_STR(getenv("AFTER"), "clear");
    CHECK(env_count_now() == 1);

    /* the program may replace environ wholesale (POSIX allows it) */
    static char* mine_env[] = { "X=1", "Y=2", 0 };
    environ = mine_env;
    CHECK_STR(getenv("Y"), "2");
    CHECK(setenv("Z", "3", 1) == 0);
    CHECK_STR(getenv("X"), "1");
    CHECK_STR(getenv("Z"), "3");
    CHECK(env_count_now() == 3);
    CHECK(mine_env[2] == NULL);              /* the program's array intact */

    /* setenv when environ is NULL (a program that never ran crt0) */
    reset_world();
    environ = NULL;
    CHECK(getenv("A") == NULL);
    CHECK(unsetenv("A") == 0);
    CHECK(setenv("A", "1", 1) == 0);
    CHECK_STR(getenv("A"), "1");

    /* limits: at most NOVA_ENV_MAX_VARS variables ... */
    reset_world();
    environ = NULL;
    int all_ok = 1;
    for (int i = 0; i < NOVA_ENV_MAX_VARS; i++) {
        char name[8];
        name[0] = 'V'; name[1] = (char)('a' + i / 10); name[2] = (char)('a' + i % 10);
        name[3] = '\0';
        all_ok &= (setenv(name, "x", 1) == 0);
    }
    CHECK(all_ok);
    CHECK(env_count_now() == NOVA_ENV_MAX_VARS);
    errno = 0;
    CHECK(setenv("ONEMORE", "x", 1) == -1 && errno == ENOMEM);
    CHECK(env_count_now() == NOVA_ENV_MAX_VARS);        /* unchanged */
    CHECK(setenv("Vaa", "y", 1) == 0);                  /* replacing is fine */
    CHECK_STR(getenv("Vaa"), "y");

    /* ... and at most NOVA_ENV_MAX_BYTES bytes in total */
    reset_world();
    environ = NULL;
    static char big[NOVA_ENV_MAX_BYTES + 64];
    memset(big, 'v', sizeof(big) - 1);
    big[sizeof(big) - 1] = '\0';
    errno = 0;
    CHECK(setenv("BIG", big, 1) == -1 && errno == ENOMEM);
    CHECK(getenv("BIG") == NULL);
    /* exactly at the limit is accepted: "N=" + value + NUL == 4096 */
    big[NOVA_ENV_MAX_BYTES - 3] = '\0';
    CHECK(setenv("N", big, 1) == 0);
    unsigned total = 0;
    for (int i = 0; environ[i]; i++) total += (unsigned)strlen(environ[i]) + 1;
    CHECK(total == NOVA_ENV_MAX_BYTES);
    /* one more byte anywhere is refused, and the old value survives */
    errno = 0;
    CHECK(setenv("M", "", 1) == -1 && errno == ENOMEM);
    big[NOVA_ENV_MAX_BYTES - 3] = 'v';
    big[NOVA_ENV_MAX_BYTES - 2] = '\0';
    errno = 0;
    CHECK(setenv("N", big, 1) == -1 && errno == ENOMEM);
    CHECK(strlen(getenv("N")) == NOVA_ENV_MAX_BYTES - 3);

    /* putenv obeys the same limits */
    reset_world();
    environ = NULL;
    static char huge[NOVA_ENV_MAX_BYTES + 16];
    memset(huge, 'h', sizeof(huge) - 1);
    huge[0] = 'H'; huge[1] = '=';
    huge[sizeof(huge) - 1] = '\0';
    errno = 0;
    CHECK(putenv(huge) == -1 && errno == ENOMEM);

    /* long-running churn: overwriting/unsetting must free what it
     * allocated, so the heap does not grow without bound */
    reset_world();
    environ = kernel_env;
    for (int i = 0; i < 20; i++) {
        setenv("CHURN", "0123456789012345678901234567890123456789", 1);
    }
    size_t settled = arena_used;
    for (int i = 0; i < 5000; i++) {
        char v[24];
        snprintf(v, sizeof v, "value-%d", i);
        setenv("CHURN", v, 1);
        setenv("TMP", v, 1);
        unsetenv("TMP");
    }
    CHECK(arena_used - settled < 4096);
    CHECK(heap_check() == 0);

    /* allocation failure leaves the environment as it was */
    reset_world();
    environ = kernel_env;
    CHECK(setenv("PATH", "/first", 1) == 0);
    fail_sbrk = 1;
    /* exhaust whatever free space remains so malloc really has to grow */
    void* hog;
    while ((hog = malloc(64)) != NULL) { }
    errno = 0;
    CHECK(setenv("PATH", "/a/much/longer/value/that/needs/a/new/block/now"
                          "/a/much/longer/value/that/needs/a/new/block/now"
                          "/a/much/longer/value/that/needs/a/new/block/now",
                 1) == -1);
    CHECK(errno == ENOMEM);
    CHECK_STR(getenv("PATH"), "/first");
    fail_sbrk = 0;
}

/* ------------------------------------------------------------------ *
 * Tests: printf family
 * ------------------------------------------------------------------ */

static int case_count, case_fail;

static void do_case(const char* expected, int line, const char* fmt, ...) {
    char buf[256];
    __builtin_va_list ap;
    __builtin_va_start(ap, fmt);
    int n = vsnprintf(buf, sizeof(buf), fmt, ap);
    __builtin_va_end(ap);
    case_count++;
    checks++;
    if (strcmp(buf, expected) != 0 || n != (int)strlen(expected)) {
        case_fail++;
        failures++;
        hout("  FAIL printf_cases.h line ");
        hout_int(line);
        hout(": fmt=\"");
        hout(fmt);
        hout("\" expected=\"");
        hout(expected);
        hout("\" got=\"");
        hout(buf);
        hout("\"\n");
    }
}
#define CASE(exp, ...) do_case(exp, __LINE__, __VA_ARGS__);

static void test_printf(void) {
    hout("printf family\n");
    reset_world();

#include "printf_cases.h"

    /* truncation: the C99 contract - return the would-be length, always
     * NUL-terminate when there is room, write nothing when size is 0 */
    char b[8];
    memset(b, 'Z', sizeof b);
    CHECK(snprintf(b, 5, "%s", "abcdefgh") == 8);
    CHECK_STR(b, "abcd");
    memset(b, 'Z', sizeof b);
    CHECK(snprintf(b, 0, "%s", "abcdefgh") == 8);
    CHECK(b[0] == 'Z');                         /* untouched */
    memset(b, 'Z', sizeof b);
    CHECK(snprintf(b, 1, "%s", "abc") == 3);
    CHECK(b[0] == '\0' && b[1] == 'Z');
    CHECK(snprintf(b, sizeof b, "%s", "1234567") == 7);
    CHECK_STR(b, "1234567");
    CHECK(snprintf(b, sizeof b, "%s", "12345678") == 8);
    CHECK_STR(b, "1234567");
    CHECK(sprintf(b, "%d", 12345) == 5);
    CHECK_STR(b, "12345");

    /* NovaOS-specific behaviour that intentionally differs from glibc */
    char o[64];
    snprintf(o, sizeof o, "%lld", 5);           /* 64-bit: printed literally */
    CHECK_STR(o, "%lld");
    snprintf(o, sizeof o, "%llu|%d", 5, 7);     /* unsupported spec consumes
                                                   no argument, so %d gets 5 */
    CHECK_STR(o, "%llu|5");
    snprintf(o, sizeof o, "%q");                /* unknown conversion */
    CHECK_STR(o, "%q");
    snprintf(o, sizeof o, "abc%");              /* format ends after '%' */
    CHECK_STR(o, "abc%");
    snprintf(o, sizeof o, "abc%5");             /* ...or mid-specifier */
    CHECK_STR(o, "abc%5");
    snprintf(o, sizeof o, "%p", (void*)0);
    CHECK_STR(o, "0x0");
    snprintf(o, sizeof o, "%s|%5s|%-5s|", (char*)0, (char*)0, (char*)0);
    CHECK_STR(o, "(null)|(null)|(null)|");
    snprintf(o, sizeof o, "%99999d", 1);        /* absurd width is clamped */
    CHECK(strlen(o) == 63);

    /* printf/fprintf/vfprintf reach the console, unbuffered, complete */
    console_reset();
    CHECK(printf("x=%d y=%s\n", 5, "ok") == 9);
    CHECK_STR(console, "x=5 y=ok\n");
    console_reset();
    CHECK(fprintf(stderr, "err %u", 7u) == 5);
    CHECK_STR(console, "err 7");

    /* the old printf silently truncated at 512 bytes; this must not */
    static char longs[1200];
    memset(longs, 'a', sizeof(longs) - 1);
    longs[sizeof(longs) - 1] = '\0';
    console_reset();
    int n = printf("<%s>", longs);
    CHECK(n == 1201);
    CHECK(console_len == 1201);
    CHECK(console[0] == '<' && console[1200] == '>' && console[600] == 'a');
    console_reset();
    CHECK(printf("%s", "") == 0);
    CHECK(console_len == 0);

    /* putchar/puts keep their original direct behaviour */
    console_reset();
    putchar('Q');
    puts("hi");
    CHECK_STR(console, "Qhi\n");
}

/* ------------------------------------------------------------------ *
 * Tests: FILE streams
 * ------------------------------------------------------------------ */

static void make_big(const char* name, unsigned n) {
    unsigned char* d = (unsigned char*)malloc(n ? n : 1);
    fill_pattern(d, n, 77);
    fs_put(name, d, n);
    free(d);
}

static void test_files_open_errors(void) {
    hout("stdio: fopen errors & argument checking\n");
    reset_world();
    FILE* f;

    errno = 0; CHECK((f = fopen("NOPE.TXT", "r")) == NULL); CHECK(errno == ENOENT);
    CHECK(handles_in_use() == 0);
    errno = 0; CHECK(fopen(NULL, "r") == NULL && errno == EINVAL);
    errno = 0; CHECK(fopen("", "r") == NULL && errno == ENOENT);
    errno = 0; CHECK(fopen("ABCDEFGHIJKLM.TXT", "r") == NULL && errno == ENAMETOOLONG);
    errno = 0; CHECK(fopen("DIR/FILE.TXT", "r") == NULL && errno == ENOENT);
    fs_put("HELLO.TXT", "hi", 2);
    errno = 0; CHECK(fopen("HELLO.TXT", NULL) == NULL && errno == EINVAL);
    errno = 0; CHECK(fopen("HELLO.TXT", "z") == NULL && errno == EINVAL);
    errno = 0; CHECK(fopen("HELLO.TXT", "rw") == NULL && errno == EINVAL);
    errno = 0; CHECK(fopen("HELLO.TXT", "") == NULL && errno == EINVAL);
    errno = 0; CHECK(fopen("HELLO.TXT", "rx") == NULL && errno == EINVAL);
    CHECK((f = fopen("HELLO.TXT", "rb")) != NULL); fclose(f);
    CHECK((f = fopen("HELLO.TXT", "r+b")) != NULL); fclose(f);
    CHECK((f = fopen("HELLO.TXT", "rb+")) != NULL); fclose(f);
    CHECK((f = fopen("hello.txt", "r")) != NULL); fclose(f);   /* 8.3 is case-insensitive */

    /* the kernel refuses to open anything */
    fs_can_open = 0;
    errno = 0; CHECK(fopen("HELLO.TXT", "r") == NULL && errno == EACCES);
    fs_can_open = 1;

    /* a 12-character name is the longest the kernel keeps intact */
    fs_put("ABCDEFGH.TXT", "twelve", 6);
    CHECK((f = fopen("ABCDEFGH.TXT", "r")) != NULL);
    CHECK(f != NULL && fgetc(f) == 't');
    fclose(f);
    CHECK(handles_in_use() == 0);
}

static void test_files_read(void) {
    hout("stdio: reading\n");
    reset_world();
    static const char text[] = "Hello, world\nline two\nno newline at end";
    fs_put("T.TXT", text, sizeof(text) - 1);
    FILE* f = fopen("T.TXT", "r");
    CHECK(f != NULL);
    CHECK(handles_in_use() == 0);        /* the kernel handle is already closed */
    CHECK(ftell(f) == 0);

    char line[64];
    CHECK(fgets(line, sizeof line, f) == line);
    CHECK_STR(line, "Hello, world\n");
    CHECK(ftell(f) == 13);
    CHECK(fgets(line, sizeof line, f) == line);
    CHECK_STR(line, "line two\n");
    CHECK(fgets(line, sizeof line, f) == line);
    CHECK_STR(line, "no newline at end");           /* last line, no '\n' */
    /* Reading a last line that has no '\n' runs into end-of-file WHILE
     * reading it, so EOF is already set - the same as a real C library
     * (checked against glibc's fgets). */
    CHECK(feof(f) && !ferror(f));
    CHECK(fgets(line, sizeof line, f) == NULL);
    CHECK(feof(f) && !ferror(f));
    CHECK(fgetc(f) == EOF);

    /* rewind clears EOF; fgetc walks the bytes */
    rewind(f);
    CHECK(!feof(f) && ftell(f) == 0);
    CHECK(fgetc(f) == 'H' && fgetc(f) == 'e');
    CHECK(ftell(f) == 2);

    /* ungetc: one byte of pushback, position follows */
    CHECK(ungetc('e', f) == 'e');
    CHECK(ftell(f) == 1);
    CHECK(ungetc('x', f) == EOF);                    /* only one guaranteed */
    CHECK(fgetc(f) == 'e' && fgetc(f) == 'l');
    CHECK(ungetc(EOF, f) == EOF);

    /* fseek in all three senses */
    CHECK(fseek(f, 7, SEEK_SET) == 0 && fgetc(f) == 'w');
    CHECK(fseek(f, 2, SEEK_CUR) == 0 && ftell(f) == 10);
    CHECK(fseek(f, -2, SEEK_CUR) == 0 && ftell(f) == 8);
    CHECK(fseek(f, -5, SEEK_END) == 0 && fgetc(f) == 't');  /* "t end" */
    CHECK(fseek(f, -1, SEEK_END) == 0 && fgetc(f) == 'd');
    CHECK(fseek(f, 0, SEEK_END) == 0 && fgetc(f) == EOF && feof(f));
    CHECK(fseek(f, 0, SEEK_SET) == 0 && !feof(f));   /* seeking clears EOF */
    errno = 0; CHECK(fseek(f, -1, SEEK_SET) == -1 && errno == EINVAL);
    errno = 0; CHECK(fseek(f, 0, 99) == -1 && errno == EINVAL);
    errno = 0; CHECK(fseek(f, 0x7FFFFFFF, SEEK_END) == -1 && errno == EINVAL);
    CHECK(ftell(f) == 0);                            /* failed seeks don't move */

    /* fread: whole items, short count at EOF, EOF flag */
    char buf[64];
    rewind(f);
    CHECK(fread(buf, 1, 5, f) == 5 && memcmp(buf, "Hello", 5) == 0);
    CHECK(fread(buf, 5, 2, f) == 2);                 /* ", wor" "ld\nli" */
    CHECK(memcmp(buf, ", worl", 5) == 0);
    rewind(f);
    size_t total = sizeof(text) - 1;
    CHECK(fread(buf, 1, sizeof buf, f) == total);
    CHECK(feof(f));
    CHECK(fread(buf, 1, 1, f) == 0);
    CHECK(fread(buf, 0, 5, f) == 0 && fread(buf, 5, 0, f) == 0);
    clearerr(f);
    CHECK(!feof(f) && !ferror(f));
    errno = 0;
    CHECK(fread(buf, 0x10000, 0x10001, f) == 0 && errno == EOVERFLOW);
    CHECK(ferror(f));
    clearerr(f);

    /* fgets corner cases */
    rewind(f);
    CHECK(fgets(line, 1, f) == line && line[0] == '\0');   /* n == 1 */
    CHECK(fgets(line, 0, f) == NULL);
    CHECK(fgets(line, -3, f) == NULL);
    CHECK(fgets(line, 6, f) == line);
    CHECK_STR(line, "Hello");                              /* n-1 chars */
    CHECK(fgets(line, 4, f) == line);
    CHECK_STR(line, ", w");

    /* writing to a read-only stream is an error, not a silent no-op */
    errno = 0;
    CHECK(fwrite("x", 1, 1, f) == 0 && errno == EBADF && ferror(f));
    CHECK(fputc('x', f) == EOF);
    CHECK(fputs("x", f) == EOF);
    clearerr(f);

    CHECK(fclose(f) == 0);
    CHECK(open_streams[0] == NULL);
}

static void test_files_large(void) {
    hout("stdio: files larger than one kernel read (4 KiB)\n");
    reset_world();
    static const unsigned sizes[] = { 0, 1, 4095, 4096, 4097, 8192, 10000, 65536, 100003 };
    for (unsigned si = 0; si < sizeof(sizes) / sizeof(sizes[0]); si++) {
        unsigned n = sizes[si];
        make_big("BIG.BIN", n);

        /* one giant fread */
        int calls_before = fs_read_calls;
        FILE* f = fopen("BIG.BIN", "rb");
        CHECK(f != NULL);
        int calls_in_open = fs_read_calls - calls_before;
        unsigned char* got = (unsigned char*)malloc(n + 16);
        size_t r = fread(got, 1, n + 16, f);
        CHECK(r == n);
        CHECK(pattern_ok(got, n, 77));
        CHECK(feof(f));
        /* the kernel serves at most 4096 bytes per call, so loading must
         * have looped: at least ceil(n/4096) data reads plus the final
         * zero-length read that signals end of file */
        CHECK(calls_in_open >= (int)((n + 4095) / 4096) + 1);
        fclose(f);

        /* odd-sized chunks, and fgetc for a prefix */
        f = fopen("BIG.BIN", "rb");
        size_t off = 0;
        int intact = 1;
        while (off < n) {
            unsigned char chunk[113];
            size_t k = fread(chunk, 1, sizeof chunk, f);
            if (k == 0) break;
            for (size_t i = 0; i < k; i++)
                if (chunk[i] != (unsigned char)(77u * 31u + (off + i) * 7u + ((off + i) >> 8)))
                    intact = 0;
            off += k;
        }
        CHECK(off == n && intact);
        CHECK(ftell(f) == (long)n);
        fclose(f);

        /* seek far in and read across a 4096 boundary */
        if (n > 5000) {
            f = fopen("BIG.BIN", "rb");
            CHECK(fseek(f, 4090, SEEK_SET) == 0);
            unsigned char x[20];
            CHECK(fread(x, 1, 20, f) == 20);
            int ok = 1;
            for (int i = 0; i < 20; i++)
                if (x[i] != (unsigned char)(77u * 31u + (4090u + i) * 7u + ((4090u + i) >> 8))) ok = 0;
            CHECK(ok);
            fclose(f);
        }
        free(got);
        CHECK(handles_in_use() == 0);
        fs_file* ff = fs_find("BIG.BIN");
        if (ff) ff->used = 0;
    }
}

static void test_files_write(void) {
    hout("stdio: writing, append, update modes\n");
    reset_world();
    FILE* f;

    /* "w" creates immediately; content is buffered until flush/close */
    CHECK((f = fopen("OUT.TXT", "w")) != NULL);
    CHECK(fs_find("OUT.TXT") != NULL && fs_find("OUT.TXT")->size == 0);
    CHECK(fputs("hello ", f) >= 0);
    CHECK(fprintf(f, "%s-%d\n", "world", 42) == 9);
    CHECK(fputc('!', f) == '!');
    CHECK(fs_find("OUT.TXT")->size == 0);           /* nothing on disk yet */
    CHECK(fflush(f) == 0);
    CHECK(file_is("OUT.TXT", "hello world-42\n!"));

    /* Every flush is a delete + create on the real disk, so a stream that
     * has not changed since its last flush must not touch the disk. */
    int w0 = fs_writes, d0 = fs_deletes;
    CHECK(fflush(f) == 0 && fflush(f) == 0);
    CHECK(fs_writes == w0 && fs_deletes == d0);
    CHECK(fputs("more", f) >= 0);
    CHECK(fclose(f) == 0);
    CHECK(file_is("OUT.TXT", "hello world-42\n!more"));
    CHECK(handles_in_use() == 0);

    /* reading it back - and a stream opened only for reading must never
     * write anything, even though fclose() flushes */
    int w1 = fs_writes, d1 = fs_deletes;
    f = fopen("OUT.TXT", "r");
    char buf[64];
    CHECK(fgets(buf, sizeof buf, f) == buf);
    CHECK_STR(buf, "hello world-42\n");
    CHECK(fflush(f) == 0);
    fclose(f);
    CHECK(fs_writes == w1 && fs_deletes == d1);

    /* "w" on an existing file empties it at once */
    CHECK((f = fopen("OUT.TXT", "w")) != NULL);
    CHECK(fs_find("OUT.TXT")->size == 0);
    fclose(f);
    CHECK(file_is("OUT.TXT", ""));

    /* "a": keeps existing data, writes go to the end even after a seek */
    fs_find("OUT.TXT")->used = 0;
    fs_put("OUT.TXT", "base", 4);
    CHECK((f = fopen("OUT.TXT", "a")) != NULL);
    CHECK(fseek(f, 0, SEEK_SET) == 0);
    CHECK(fputs("+tail", f) >= 0);
    CHECK(fclose(f) == 0);
    CHECK(file_is("OUT.TXT", "base+tail"));
    /* "a" creates a missing file */
    CHECK((f = fopen("NEWA.TXT", "a")) != NULL);
    CHECK(fs_find("NEWA.TXT") != NULL);
    fputs("x", f);
    fclose(f);
    CHECK(file_is("NEWA.TXT", "x"));
    /* "a+": reads start at the beginning, writes still append */
    CHECK((f = fopen("OUT.TXT", "a+")) != NULL);
    CHECK(fgetc(f) == 'b');
    CHECK(fputs("!", f) >= 0);
    CHECK(fseek(f, 0, SEEK_SET) == 0);
    CHECK(fread(buf, 1, 20, f) == 10);
    CHECK(memcmp(buf, "base+tail!", 10) == 0);
    fclose(f);
    CHECK(file_is("OUT.TXT", "base+tail!"));

    /* "r+": read, then overwrite in place - the rest is preserved */
    fs_find("OUT.TXT")->used = 0;
    fs_put("OUT.TXT", "0123456789", 10);
    CHECK((f = fopen("OUT.TXT", "r+")) != NULL);
    CHECK(fseek(f, 3, SEEK_SET) == 0);
    CHECK(fwrite("ABC", 1, 3, f) == 3);
    CHECK(ftell(f) == 6);
    CHECK(fgetc(f) == '6');                          /* sees its own write's tail */
    CHECK(fseek(f, 0, SEEK_SET) == 0);
    CHECK(fread(buf, 1, 10, f) == 10 && memcmp(buf, "012ABC6789", 10) == 0);
    fclose(f);
    CHECK(file_is("OUT.TXT", "012ABC6789"));
    errno = 0;
    CHECK(fopen("MISSING.TXT", "r+") == NULL && errno == ENOENT);   /* must exist */

    /* seeking past the end then writing zero-fills the gap */
    CHECK((f = fopen("GAP.BIN", "w")) != NULL);
    CHECK(fseek(f, 5, SEEK_SET) == 0);
    CHECK(fputs("Z", f) >= 0);
    fclose(f);
    unsigned gn;
    const char* gd = file_contents("GAP.BIN", &gn);
    CHECK(gn == 6 && gd[0] == 0 && gd[4] == 0 && gd[5] == 'Z');

    /* "w+": write, rewind, read back */
    CHECK((f = fopen("WP.TXT", "w+")) != NULL);
    CHECK(fputs("round trip", f) >= 0);
    rewind(f);
    CHECK(fread(buf, 1, 10, f) == 10 && memcmp(buf, "round trip", 10) == 0);
    fclose(f);

    /* reading a write-only stream is an error */
    f = fopen("WO.TXT", "w");
    errno = 0;
    CHECK(fread(buf, 1, 1, f) == 0 && errno == EBADF && ferror(f));
    CHECK(fgetc(f) == EOF);
    clearerr(f);
    CHECK(!ferror(f));
    fclose(f);

    /* a big write in many small pieces, then verified byte for byte */
    f = fopen("BIGW.BIN", "wb");
    unsigned char piece[251];
    unsigned long written = 0;
    for (int i = 0; i < 400; i++) {
        for (unsigned k = 0; k < sizeof piece; k++)
            piece[k] = (unsigned char)(77u * 31u + (written + k) * 7u + ((written + k) >> 8));
        CHECK(fwrite(piece, 1, sizeof piece, f) == sizeof piece);
        written += sizeof piece;
    }
    CHECK(fclose(f) == 0);
    unsigned bn;
    const char* bd = file_contents("BIGW.BIN", &bn);
    CHECK(bn == written && pattern_ok((const unsigned char*)bd, bn, 77));
    CHECK(fwrite("x", 0x10000, 0x10001, stdout) == 0 && errno == EOVERFLOW);
    CHECK(handles_in_use() == 0);
}

static void test_files_permissions(void) {
    hout("stdio: permission failures\n");
    reset_world();
    FILE* f;
    fs_put("HELLO.TXT", "precious", 8);

    fs_can_write = 0;    /* may open/read, may NOT create or delete */
    errno = 0; CHECK(fopen("NEW.TXT", "w") == NULL && errno == EACCES);
    CHECK(fs_find("NEW.TXT") == NULL);
    errno = 0; CHECK(fopen("HELLO.TXT", "w") == NULL && errno == EACCES);
    CHECK(file_is("HELLO.TXT", "precious"));       /* NOT truncated by the attempt */
    errno = 0; CHECK(fopen("NEW2.TXT", "a") == NULL && errno == EACCES);
    CHECK((f = fopen("HELLO.TXT", "r")) != NULL);   /* reading is fine */
    fclose(f);

    /* "r+" opens (it can't know yet), but the write-back is refused, the
     * original file survives, and the failure is reported */
    CHECK((f = fopen("HELLO.TXT", "r+")) != NULL);
    CHECK(fputs("changed", f) >= 0);
    errno = 0;
    CHECK(fflush(f) == EOF && errno == EACCES && ferror(f));
    CHECK(file_is("HELLO.TXT", "precious"));
    errno = 0;
    CHECK(fclose(f) == EOF && errno == EACCES);
    CHECK(file_is("HELLO.TXT", "precious"));

    /* remove() */
    errno = 0; CHECK(remove("HELLO.TXT") == -1 && errno == EACCES);
    fs_can_write = 1;
    CHECK(remove("HELLO.TXT") == 0);
    CHECK(fs_find("HELLO.TXT") == NULL);
    errno = 0; CHECK(remove("HELLO.TXT") == -1 && errno == ENOENT);
    errno = 0; CHECK(remove(NULL) == -1 && errno == EINVAL);
    CHECK(handles_in_use() == 0);
}

static void test_files_resources(void) {
    hout("stdio: resource limits and cleanup\n");
    reset_world();
    fs_put("R.TXT", "x", 1);

    /* FOPEN_MAX streams, then EMFILE, then a slot frees up again */
    FILE* fs_[FOPEN_MAX];
    for (int i = 0; i < FOPEN_MAX; i++) {
        fs_[i] = fopen("R.TXT", "r");
        CHECK(fs_[i] != NULL);
    }
    CHECK(handles_in_use() == 0);     /* 16 open FILEs, zero kernel handles */
    errno = 0;
    CHECK(fopen("R.TXT", "r") == NULL && errno == EMFILE);
    CHECK(fclose(fs_[3]) == 0);
    FILE* again = fopen("R.TXT", "r");
    CHECK(again != NULL);
    for (int i = 0; i < FOPEN_MAX; i++) if (i != 3) fclose(fs_[i]);
    fclose(again);
    for (int i = 0; i < FOPEN_MAX; i++) CHECK(open_streams[i] == NULL);

    /* out of memory during load: clean failure, nothing leaked */
    make_big("BIGGER.BIN", 200000);
    void* reserve = malloc(256);       /* room for the FILE struct itself, */
    fail_sbrk = 1;                     /* so it is the buffer load that fails */
    void* hog;
    while ((hog = malloc(1024)) != NULL) { }
    free(reserve);
    errno = 0;
    CHECK(fopen("BIGGER.BIN", "r") == NULL && errno == ENOMEM);
    CHECK(handles_in_use() == 0);
    for (int i = 0; i < FOPEN_MAX; i++) CHECK(open_streams[i] == NULL);
    fail_sbrk = 0;

    /* repeated open/close cycles do not grow the heap */
    reset_world();
    fs_put("R.TXT", "some data here", 14);
    for (int i = 0; i < 20; i++) fclose(fopen("R.TXT", "r"));
    size_t settled = arena_used;
    for (int i = 0; i < 3000; i++) {
        FILE* f = fopen("R.TXT", "r");
        char c[4];
        fread(c, 1, 4, f);
        fclose(f);
    }
    CHECK(arena_used == settled);
    CHECK(heap_check() == 0);

    /* closing twice / closing garbage is an error, not a crash */
    FILE* f = fopen("R.TXT", "r");
    fclose(f);
    errno = 0;
    CHECK(fclose(NULL) == EOF && errno == EBADF);

    /* fflush(NULL) flushes every open dirty stream */
    reset_world();
    FILE* a = fopen("A.TXT", "w");
    FILE* b = fopen("B.TXT", "w");
    fputs("aaa", a);
    fputs("bbb", b);
    CHECK(fs_find("A.TXT")->size == 0 && fs_find("B.TXT")->size == 0);
    CHECK(fflush(NULL) == 0);
    CHECK(file_is("A.TXT", "aaa") && file_is("B.TXT", "bbb"));
    CHECK(fflush(stdout) == 0 && fflush(stderr) == 0);
    fclose(a);
    fclose(b);
}

static void test_files_exit_flush(void) {
    hout("stdio: streams are flushed automatically at exit()\n");
    reset_world();
    FILE* f = fopen("EXIT.TXT", "w");
    fputs("written but never closed", f);
    FILE* g = fopen("EXIT2.TXT", "w");
    fputs("second", g);
    CHECK(fs_find("EXIT.TXT")->size == 0);
    exit_hook_armed = 1;
    if (__builtin_setjmp(exit_jb) == 0) {
        exit(3);
    }
    exit_hook_armed = 0;
    CHECK(exit_code_seen == 3);
    CHECK(file_is("EXIT.TXT", "written but never closed"));
    CHECK(file_is("EXIT2.TXT", "second"));
    CHECK(handles_in_use() == 0);
}

static void test_console_streams(void) {
    hout("stdio: stdin / stdout / stderr\n");
    reset_world();

    console_reset();
    CHECK(fputs("out", stdout) >= 0 && fputc('!', stdout) == '!');
    CHECK(fwrite("abc", 1, 3, stderr) == 3);
    CHECK_STR(console, "out!abc");
    CHECK(fflush(stdout) == 0);

    /* a NUL byte cannot be shown on a console and is dropped, the rest
     * of the data still arrives */
    console_reset();
    CHECK(fwrite("a\0b", 1, 3, stdout) == 3);
    CHECK_STR(console, "ab");

    /* a large console write arrives whole, in order */
    static char big[3000];
    for (int i = 0; i < 2999; i++) big[i] = (char)('a' + i % 26);
    big[2999] = '\0';
    console_reset();
    CHECK(fputs(big, stdout) >= 0);
    CHECK(console_len == 2999 && memcmp(console, big, 2999) == 0);

    /* stdin: raw keys, polled (yielding while none are ready) */
    reset_world();
    key_script[0] = -1; key_script[1] = -1; key_script[2] = 'h';
    key_script[3] = 'i'; key_script[4] = '\n'; key_script[5] = 'x'; key_script[6] = -1;
    key_script[7] = 'y'; key_script[8] = '\n';
    key_count = 9;
    CHECK(fgetc(stdin) == 'h');
    CHECK(yield_count == 2);                        /* polled while empty */
    CHECK(getchar() == 'i');
    CHECK(ungetc('i', stdin) == 'i');
    char line[16];
    CHECK(fgets(line, sizeof line, stdin) == line);
    CHECK_STR(line, "i\n");
    CHECK(fread(line, 1, 10, stdin) == 3);          /* a "line" at a time */
    CHECK(line[0] == 'x' && line[1] == 'y' && line[2] == '\n');
    CHECK(yield_count == 3);                         /* polled once more */

    /* the std streams are not seekable */
    errno = 0; CHECK(ftell(stdin) == -1 && errno == ESPIPE);
    errno = 0; CHECK(fseek(stdout, 0, SEEK_SET) == -1 && errno == ESPIPE);
    errno = 0;
    CHECK(fputs("x", stdin) == EOF && errno == EBADF);     /* stdin is read-only */
    CHECK(fgetc(stdout) == EOF);                            /* stdout is write-only */

    /* perror */
    console_reset();
    errno = ENOENT;
    perror("open");
    CHECK_STR(console, "open: No such file or directory\n");
    console_reset();
    errno = EACCES;
    perror(NULL);
    CHECK_STR(console, "Permission denied\n");
    console_reset();
    errno = EINVAL;
    perror("");
    CHECK_STR(console, "Invalid argument\n");
    CHECK(errno == EINVAL);                         /* perror leaves errno alone */
}

/* ------------------------------------------------------------------ */

int hmain(void) {
    test_alloc_basic();
    test_realloc();
    test_alloc_stress();
    test_exit_errno();
    test_environment();
    test_printf();
    test_files_open_errors();
    test_files_read();
    test_files_large();
    test_files_write();
    test_files_permissions();
    test_files_resources();
    test_files_exit_flush();
    test_console_streams();

    hout("\n");
    hout_int(checks);
    hout(" checks (");
    hout_int(case_count);
    hout(" printf cases), ");
    hout_int(failures);
    hout(failures == 0 ? " failed - ALL PASSED\n" : " failed\n");
    return failures == 0 ? 0 : 1;
}

void _start(void) {
    int rc = hmain();
    lx3(1, rc, 0, 0);
    for (;;) { }
}
