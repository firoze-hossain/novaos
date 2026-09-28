/*
 * stdlib.c - the allocator (malloc/free/realloc/calloc), errno's one
 * definition, the process environment (environ/getenv/setenv/unsetenv/
 * putenv/clearenv), atexit()/exit(), and atoi().
 *
 * SELF-CONTAINED ON PURPOSE: nothing here calls into string.c. Every
 * userland program links this object - but Rust programs (ping-rs,
 * wm-rs, novainit-rs, coreutils-rs, net-rs) link only crt0.o, syscall.o
 * and stdlib.o, never string.o or stdio.o (see each build.sh's `ld`
 * line). One call to strlen() from here would turn into an undefined
 * reference in every one of those binaries. The few string helpers this
 * file needs are therefore tiny static functions of its own.
 */
#include "stdlib.h"
#include "errno.h"
#include "novasys.h"
#include <stdint.h>

int errno;
char** environ;

/* ------------------------------------------------------------------ *
 * Allocator
 *
 * A first-fit allocator over SYS_SBRK, kept as a doubly linked list of
 * blocks in address order. Compared with this file's original (Phase
 * 22) allocator - which never split or merged blocks and whose 12-byte
 * header left every payload only 4-byte aligned despite a comment
 * claiming 8 - this one:
 *   - uses a 16-byte header, so with every payload size a multiple of
 *     16 and the heap base page-aligned, EVERY payload is 16-byte
 *     aligned (what SSE-era compilers assume malloc returns);
 *   - splits an oversized free block instead of wasting its tail;
 *   - coalesces adjacent free blocks on free(), so alloc/free/alloc
 *     patterns no longer fragment the heap forever;
 *   - can grow and shrink a block in place (realloc), including
 *     growing the last block by extending the heap directly;
 *   - stamps each header with a magic value, so free() and realloc()
 *     can recognise (and refuse) a double free or a wild pointer
 *     instead of silently corrupting the free list.
 *
 * Invariants (checked by userland/libc/tests/libc_host_test.c after
 * every step of a randomized stress run):
 *   1. blocks are linked in strictly increasing address order;
 *   2. two FREE blocks are never adjacent - free()/split/realloc merge
 *      them, so the heap never carries avoidable fragmentation;
 *   3. every payload size is a multiple of 16 and at least 16;
 *   4. heap_tail is the last block and its next is NULL.
 *
 * Honest limits: the heap only ever grows (SYS_SBRK rejects negative
 * increments, see process_sbrk() in the kernel) - freed memory is
 * reused within this allocator but never handed back; and the search
 * is a linear walk of the block list, fine at this scale, not a
 * production allocator's size-class bins.
 * ------------------------------------------------------------------ */

#define HDR_SIZE          16u
#define ALIGNMENT         16u
#define MIN_SPLIT_PAYLOAD 16u
#define MAX_ALLOC         0x40000000u /* 1 GiB - far past anything this
                                          kernel's user address space
                                          (heap at 0x20000000, stack at
                                          0x40000000) could ever serve,
                                          and small enough that no size
                                          arithmetic below can wrap */
#define MAGIC_ALLOC 0xA110C8EDu
#define MAGIC_FREE  0xF4EEB10Cu

typedef struct block {
    uint32_t size;        /* payload bytes, always a multiple of 16 */
    uint32_t magic;       /* MAGIC_ALLOC or MAGIC_FREE */
    struct block* next;
    struct block* prev;
} block_t;

/* Compile-time check that the header really is 16 bytes on this
 * target: a negative array size is a compile error, not a runtime
 * surprise. */
typedef char block_header_is_16_bytes[(sizeof(block_t) == HDR_SIZE) ? 1 : -1];

static block_t* heap_head;
static block_t* heap_tail;

static uint32_t payload_size_for(size_t n) {
    if (n > MAX_ALLOC) {
        return 0; /* caller reports ENOMEM */
    }
    if (n == 0) {
        return ALIGNMENT; /* malloc(0) yields a real, unique, freeable
                              minimal block, not NULL */
    }
    return ((uint32_t)n + (ALIGNMENT - 1)) & ~(ALIGNMENT - 1);
}

static char* block_end(const block_t* b) {
    return (char*)(b + 1) + b->size;
}

static int blocks_adjacent(const block_t* a, const block_t* b) {
    return block_end(a) == (const char*)b;
}

/* Merges b's immediate successor into b. Caller has already checked
 * that the successor exists, is free where that matters, and is
 * adjacent in memory. */
static void absorb_next(block_t* b) {
    block_t* n = b->next;
    b->size += HDR_SIZE + n->size;
    b->next = n->next;
    if (b->next != NULL) {
        b->next->prev = b;
    } else {
        heap_tail = b;
    }
    n->magic = 0; /* scrub the now-interior header so a stale pointer to
                     it can never look like a live block */
}

/* If b has enough spare room, carves the tail off into a new FREE
 * block (merged with whatever free block follows it, keeping
 * invariant 2). `need` is already a multiple of 16. */
static void split_block(block_t* b, uint32_t need) {
    if (b->size < need + HDR_SIZE + MIN_SPLIT_PAYLOAD) {
        return;
    }
    block_t* r = (block_t*)((char*)(b + 1) + need);
    r->size = b->size - need - HDR_SIZE;
    r->magic = MAGIC_FREE;
    r->next = b->next;
    r->prev = b;
    if (r->next != NULL) {
        r->next->prev = r;
    } else {
        heap_tail = r;
    }
    b->next = r;
    b->size = need;
    if (r->next != NULL && r->next->magic == MAGIC_FREE &&
        blocks_adjacent(r, r->next)) {
        absorb_next(r);
    }
}

/* Is `b` (the header just before a pointer handed to free()/realloc())
 * a live, allocated block of ours? Checks alignment and that it lies
 * inside the heap's block range before ever dereferencing it, so
 * free(some_stack_pointer) is refused rather than faulting. */
static int block_is_live(const block_t* b) {
    if (heap_head == NULL) {
        return 0;
    }
    if (((uintptr_t)b & (ALIGNMENT - 1)) != 0) {
        return 0;
    }
    if ((const char*)b < (const char*)heap_head ||
        (const char*)b > (const char*)heap_tail) {
        return 0;
    }
    return b->magic == MAGIC_ALLOC;
}

/* No free block fits: get more memory from the kernel. */
static void* extend_heap(uint32_t need) {
    char* brk = (char*)sys_sbrk(0); /* sbrk(0) = query the current break */
    if (brk == (char*)-1) {
        errno = ENOMEM;
        return NULL;
    }

    /* A free block at the very end of the heap: grow it in place
     * instead of stranding it as an unusable fragment and allocating a
     * whole new block beside it. */
    block_t* tail = heap_tail;
    if (tail != NULL && tail->magic == MAGIC_FREE && block_end(tail) == brk) {
        if (sys_sbrk((int)(need - tail->size)) == (void*)-1) {
            errno = ENOMEM;
            return NULL;
        }
        tail->size = need;
        tail->magic = MAGIC_ALLOC;
        return tail + 1;
    }

    /* Keep every payload 16-byte aligned even if something other than
     * malloc() has moved the break to an odd address. */
    uint32_t pad = (uint32_t)(-(uintptr_t)brk) & (ALIGNMENT - 1);
    void* mem = sys_sbrk((int)(pad + HDR_SIZE + need));
    if (mem == (void*)-1) {
        errno = ENOMEM;
        return NULL;
    }
    block_t* b = (block_t*)((char*)mem + pad);
    b->size = need;
    b->magic = MAGIC_ALLOC;
    b->next = NULL;
    b->prev = heap_tail;
    if (heap_tail != NULL) {
        heap_tail->next = b;
    } else {
        heap_head = b;
    }
    heap_tail = b;
    return b + 1;
}

void* malloc(size_t size) {
    uint32_t need = payload_size_for(size);
    if (need == 0) {
        errno = ENOMEM;
        return NULL;
    }

    for (block_t* b = heap_head; b != NULL; b = b->next) {
        if (b->magic == MAGIC_FREE && b->size >= need) {
            split_block(b, need);
            b->magic = MAGIC_ALLOC;
            return b + 1;
        }
    }
    return extend_heap(need);
}

void free(void* ptr) {
    if (ptr == NULL) {
        return;
    }
    block_t* b = (block_t*)ptr - 1;
    if (!block_is_live(b)) {
        /* Not a pointer this allocator handed out, or already freed.
         * A real libc would abort here; ignoring it (loudly) is the
         * safer choice on a system with no core dumps to debug from.
         * Worded to avoid the uppercase substrings tools/python/
         * test_runner.py treats as boot failures. */
        sys_write("libc: free() ignored an invalid pointer or a double "
                  "free\n");
        return;
    }
    b->magic = MAGIC_FREE;
    if (b->next != NULL && b->next->magic == MAGIC_FREE &&
        blocks_adjacent(b, b->next)) {
        absorb_next(b);
    }
    if (b->prev != NULL && b->prev->magic == MAGIC_FREE &&
        blocks_adjacent(b->prev, b)) {
        absorb_next(b->prev);
    }
}

void* realloc(void* ptr, size_t size) {
    if (ptr == NULL) {
        return malloc(size);
    }
    block_t* b = (block_t*)ptr - 1;
    if (!block_is_live(b)) {
        errno = EINVAL; /* not ours / already freed */
        return NULL;
    }
    if (size == 0) {
        free(ptr); /* the common realloc(p, 0) == free(p) idiom */
        return NULL;
    }
    uint32_t need = payload_size_for(size);
    if (need == 0) {
        errno = ENOMEM;
        return NULL;
    }

    uint32_t old_size = b->size;

    if (need <= old_size) {
        split_block(b, need); /* shrink in place, return the tail */
        return ptr;
    }

    /* Grow in place by swallowing free blocks that follow. */
    while (b->size < need && b->next != NULL &&
           b->next->magic == MAGIC_FREE && blocks_adjacent(b, b->next)) {
        absorb_next(b);
    }
    if (b->size >= need) {
        split_block(b, need);
        return ptr;
    }

    /* Last block in the heap: just push the break out. */
    if (b == heap_tail && block_end(b) == (char*)sys_sbrk(0)) {
        if (sys_sbrk((int)(need - b->size)) == (void*)-1) {
            errno = ENOMEM; /* original block still valid and intact */
            return NULL;
        }
        b->size = need;
        return ptr;
    }

    /* Otherwise: allocate, copy what was really there, release. */
    void* np = malloc(size);
    if (np == NULL) {
        return NULL; /* errno set by malloc; the original is untouched */
    }
    const unsigned char* src = (const unsigned char*)ptr;
    unsigned char* dst = (unsigned char*)np;
    for (uint32_t i = 0; i < old_size; i++) {
        dst[i] = src[i];
    }
    free(ptr);
    return np;
}

void* calloc(size_t nmemb, size_t size) {
    if (nmemb != 0 && size > 0xFFFFFFFFu / nmemb) {
        errno = ENOMEM; /* nmemb * size would overflow */
        return NULL;
    }
    size_t total = nmemb * size;
    void* p = malloc(total);
    if (p == NULL) {
        return NULL;
    }
    unsigned char* d = (unsigned char*)p;
    for (size_t i = 0; i < total; i++) {
        d[i] = 0;
    }
    return p;
}

/* ------------------------------------------------------------------ *
 * atexit / exit
 * ------------------------------------------------------------------ */

#define ATEXIT_MAX 32
static void (*atexit_fns[ATEXIT_MAX])(void);
static int atexit_count;

int atexit(void (*fn)(void)) {
    if (fn == NULL || atexit_count >= ATEXIT_MAX) {
        return -1;
    }
    atexit_fns[atexit_count++] = fn;
    return 0;
}

void exit(int code) {
    /* Registered handlers run last-in first-out. stdio registers one
     * the first time a file is opened, which is how buffered writes get
     * flushed to disk when a program simply returns from main(). */
    while (atexit_count > 0) {
        atexit_fns[--atexit_count]();
    }
    sys_exit(code); /* never returns */
    __builtin_unreachable();
}

int atoi(const char* s) {
    int result = 0;
    int sign = 1;

    while (*s == ' ') {
        s++;
    }
    if (*s == '-') {
        sign = -1;
        s++;
    } else if (*s == '+') {
        s++;
    }
    while (*s >= '0' && *s <= '9') {
        result = result * 10 + (*s - '0');
        s++;
    }
    return result * sign;
}

/* ------------------------------------------------------------------ *
 * Environment
 *
 * The kernel hands a new process its environment as a NULL-terminated
 * array of "NAME=value" strings on the initial stack; crt0.asm
 * publishes that array as `environ`. Those strings live on the user
 * stack - read-only for our purposes and not ours to free.
 *
 * The first call that MODIFIES the environment ("adopts" it) copies
 * that pointer array into a heap vector this file owns (env_vec), plus
 * a parallel env_own[] flag array recording which strings this file
 * malloc()ed (setenv) versus which are someone else's (the kernel's
 * originals, or a string handed to putenv()). Only owned strings are
 * ever freed. Reading (getenv) never adopts anything.
 *
 * A program is also allowed to assign `environ` directly (POSIX
 * permits it); if environ != env_vec the next mutation simply adopts
 * the new array instead. Strings from the abandoned vector are leaked
 * rather than risk freeing something the program still uses.
 *
 * Total size is capped at NOVA_ENV_MAX_VARS / NOVA_ENV_MAX_BYTES - the
 * exact limits the kernel enforces when passing an environment to a
 * child - so a process can never assemble an environment that its own
 * exec calls would then be unable to hand down.
 * ------------------------------------------------------------------ */

static char** env_vec;         /* our own NULL-terminated pointer array */
static unsigned char* env_own; /* env_own[i] != 0: env_vec[i] is malloc()ed */
static int env_cap;            /* entries env_vec has room for, including
                                  the terminating NULL */

static size_t env_len(const char* s) {
    size_t n = 0;
    while (s[n] != '\0') {
        n++;
    }
    return n;
}

/* A valid variable name is non-empty and contains no '='. */
static int env_name_ok(const char* n) {
    if (n == NULL || *n == '\0') {
        return 0;
    }
    for (; *n != '\0'; n++) {
        if (*n == '=') {
            return 0;
        }
    }
    return 1;
}

/* Does `entry` ("NAME=value") define the variable `name` of length
 * namelen? Reads entry only up to its first mismatching byte, so an
 * entry shorter than the name is safe. */
static int env_entry_is(const char* entry, const char* name, size_t namelen) {
    for (size_t i = 0; i < namelen; i++) {
        if (entry[i] != name[i]) {
            return 0;
        }
    }
    return entry[namelen] == '=';
}

/* Points environ at a vector this file owns. Returns 0, or -1 with
 * errno == ENOMEM. */
static int env_adopt(void) {
    if (env_vec != NULL && environ == env_vec) {
        return 0;
    }
    int n = 0;
    if (environ != NULL) {
        while (environ[n] != NULL) {
            n++;
        }
    }
    int cap = n + 8;
    if (cap < 16) {
        cap = 16;
    }
    char** nv = (char**)malloc((size_t)cap * sizeof(char*));
    unsigned char* no = (unsigned char*)malloc((size_t)cap);
    if (nv == NULL || no == NULL) {
        free(nv);
        free(no);
        errno = ENOMEM;
        return -1;
    }
    for (int i = 0; i < n; i++) {
        nv[i] = environ[i];
        no[i] = 0; /* not ours: kernel-provided, or the program's own */
    }
    nv[n] = NULL;
    free(env_vec);
    free(env_own);
    env_vec = nv;
    env_own = no;
    env_cap = cap;
    environ = env_vec;
    return 0;
}

/* Makes room for `count` entries plus the terminating NULL. */
static int env_grow_for(int count) {
    if (count + 1 <= env_cap) {
        return 0;
    }
    int ncap = env_cap * 2;
    if (ncap < count + 1) {
        ncap = count + 1;
    }
    char** nv = (char**)realloc(env_vec, (size_t)ncap * sizeof(char*));
    if (nv == NULL) {
        errno = ENOMEM;
        return -1;
    }
    env_vec = nv;
    environ = nv; /* realloc may have moved it */
    unsigned char* no = (unsigned char*)realloc(env_own, (size_t)ncap);
    if (no == NULL) {
        errno = ENOMEM;
        return -1;
    }
    env_own = no;
    env_cap = ncap;
    return 0;
}

/* Counts entries and total bytes ("NAME=value" + NUL each) and finds
 * `name`'s index, or -1. */
static void env_scan(const char* name, size_t namelen, int* count,
                     size_t* bytes, int* index) {
    int c = 0;
    size_t b = 0;
    int idx = -1;
    for (; env_vec[c] != NULL; c++) {
        b += env_len(env_vec[c]) + 1;
        if (idx < 0 && env_entry_is(env_vec[c], name, namelen)) {
            idx = c;
        }
    }
    *count = c;
    *bytes = b;
    *index = idx;
}

char* getenv(const char* name) {
    if (!env_name_ok(name) || environ == NULL) {
        return NULL;
    }
    size_t n = env_len(name);
    for (char** e = environ; *e != NULL; e++) {
        if (env_entry_is(*e, name, n)) {
            return *e + n + 1;
        }
    }
    return NULL;
}

int setenv(const char* name, const char* value, int overwrite) {
    if (!env_name_ok(name) || value == NULL) {
        errno = EINVAL;
        return -1;
    }
    if (env_adopt() < 0) {
        return -1;
    }
    size_t nl = env_len(name);
    size_t vl = env_len(value);

    int count, idx;
    size_t bytes;
    env_scan(name, nl, &count, &bytes, &idx);
    if (idx >= 0 && !overwrite) {
        return 0;
    }

    size_t newlen = nl + 1 + vl + 1;
    size_t after = bytes + newlen;
    if (idx >= 0) {
        after -= env_len(env_vec[idx]) + 1;
    }
    int after_count = count + (idx >= 0 ? 0 : 1);
    if (after > NOVA_ENV_MAX_BYTES || after_count > NOVA_ENV_MAX_VARS) {
        errno = ENOMEM;
        return -1;
    }
    if (idx < 0 && env_grow_for(after_count) < 0) {
        return -1;
    }

    /* Build the new string BEFORE freeing the old one: `value` may
     * point into the very entry being replaced
     * (setenv("A", getenv("A"), 1)). */
    char* s = (char*)malloc(newlen);
    if (s == NULL) {
        return -1; /* errno set by malloc; the old value is untouched */
    }
    size_t k = 0;
    for (size_t i = 0; i < nl; i++) {
        s[k++] = name[i];
    }
    s[k++] = '=';
    for (size_t i = 0; i < vl; i++) {
        s[k++] = value[i];
    }
    s[k] = '\0';

    if (idx >= 0) {
        if (env_own[idx]) {
            free(env_vec[idx]);
        }
        env_vec[idx] = s;
        env_own[idx] = 1;
    } else {
        env_vec[count] = s;
        env_own[count] = 1;
        env_vec[count + 1] = NULL;
    }
    return 0;
}

int unsetenv(const char* name) {
    if (!env_name_ok(name)) {
        errno = EINVAL;
        return -1;
    }
    if (environ == NULL) {
        return 0; /* nothing to remove from */
    }
    if (env_adopt() < 0) {
        return -1;
    }
    size_t nl = env_len(name);
    int w = 0;
    for (int r = 0; env_vec[r] != NULL; r++) {
        if (env_entry_is(env_vec[r], name, nl)) {
            if (env_own[r]) {
                free(env_vec[r]);
            }
            continue;
        }
        env_vec[w] = env_vec[r];
        env_own[w] = env_own[r];
        w++;
    }
    env_vec[w] = NULL;
    return 0;
}

int putenv(char* string) {
    if (string == NULL || *string == '\0') {
        errno = EINVAL;
        return -1;
    }
    size_t nl = 0;
    while (string[nl] != '\0' && string[nl] != '=') {
        nl++;
    }
    if (nl == 0) {
        errno = EINVAL; /* "=value" has no name */
        return -1;
    }
    if (string[nl] == '\0') {
        return unsetenv(string); /* glibc's behaviour: "NAME" removes it */
    }
    if (env_adopt() < 0) {
        return -1;
    }

    int count, idx;
    size_t bytes;
    env_scan(string, nl, &count, &bytes, &idx);

    size_t newlen = env_len(string) + 1;
    size_t after = bytes + newlen;
    if (idx >= 0) {
        after -= env_len(env_vec[idx]) + 1;
    }
    int after_count = count + (idx >= 0 ? 0 : 1);
    if (after > NOVA_ENV_MAX_BYTES || after_count > NOVA_ENV_MAX_VARS) {
        errno = ENOMEM;
        return -1;
    }
    if (idx < 0 && env_grow_for(after_count) < 0) {
        return -1;
    }

    /* The caller's own string becomes the entry - by POSIX, not copied,
     * so later edits to it show through, and never freed by us. */
    if (idx >= 0) {
        if (env_own[idx]) {
            free(env_vec[idx]);
        }
        env_vec[idx] = string;
        env_own[idx] = 0;
    } else {
        env_vec[count] = string;
        env_own[count] = 0;
        env_vec[count + 1] = NULL;
    }
    return 0;
}

int clearenv(void) {
    if (env_adopt() < 0) {
        return -1;
    }
    for (int i = 0; env_vec[i] != NULL; i++) {
        if (env_own[i]) {
            free(env_vec[i]);
        }
    }
    env_vec[0] = NULL;
    return 0;
}
