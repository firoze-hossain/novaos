/*
 * novashm_test.c - Phase 83: host-side tests for userland/libc/include/
 * novashm_chan.h, the lock-free triple buffer that carries pixel frames
 * between a producer process and a consumer process through shared memory.
 *
 * A protocol like this fails in a way a quick run never shows: once in
 * millions of frames, two processes occasionally own the same buffer. So
 * the evidence has three independent parts:
 *
 *  1. MODEL CHECK. The algorithm is re-stated as a small state machine in
 *     which every atomic step is separately schedulable, and EVERY
 *     reachable state is visited (not a sample of interleavings: all of
 *     them). Invariants: the three buffer indices are always a permutation
 *     of {0,1,2}; the buffer the consumer is reading is never the one the
 *     producer is writing; a frame the consumer holds never changes under
 *     it; frames arrive in strictly increasing order. The checker is then
 *     pointed at two deliberately BROKEN protocols - a plain double buffer,
 *     and a triple buffer whose commit is a load followed by a store
 *     instead of one exchange - and must report a violation in each. A
 *     checker that cannot fail proves nothing.
 *  2. STRESS. The real header code runs between two real threads for
 *     hundreds of thousands of frames; every frame the consumer takes is
 *     scanned pixel by pixel for tearing, and sequence numbers must rise.
 *  3. VALIDATION. attach() trusts nothing it reads from shared memory;
 *     malformed headers, short regions and absurd dimensions are refused.
 *
 * Build: see tools/tests/run_shm_tests.sh (also under ThreadSanitizer,
 * which checks the acquire/release pairing the stress test relies on).
 */
#include <pthread.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "novashm_chan.h"

static long checks, failures;
#define CHECK(cond, ...) do { checks++; if (!(cond)) { failures++; \
    if (failures <= 15) { printf("FAIL %s:%d: %s\n   ", __FILE__, __LINE__, #cond); \
    printf(__VA_ARGS__); printf("\n"); } } } while (0)

/* ---------------------------------------------------------------------
 * 1. The model checker
 * ------------------------------------------------------------------- */

enum variant { TRIPLE, DOUBLE_BROKEN, TRIPLE_SPLIT_COMMIT };

#define MAX_FRAMES 5 /* frames the producer draws; bounds the state space */

typedef struct {
    unsigned char state;     /* latest | DIRTY */
    unsigned char back;      /* producer's buffer */
    unsigned char front;     /* consumer's buffer */
    unsigned char content[3];/* ghost: frame id stored in each buffer (0 = none) */
    unsigned char next_id;   /* last frame id the producer drew */
    unsigned char drawn;     /* producer has drawn into `back` but not committed */
    unsigned char pc_phase;  /* TRIPLE_SPLIT_COMMIT: 1 = loaded, store pending */
    unsigned char pc_old;    /* ... and the value it loaded */
    unsigned char cphase;    /* consumer: 1 = has checked, exchange pending */
    unsigned char saw_dirty;
    unsigned char last_seen; /* newest frame id the consumer has taken */
    unsigned char held_id;   /* id of the frame the consumer holds (at acquire) */
    unsigned char holding;   /* consumer holds a frame */
} S;

static const char* violation;

static int perm_ok(const S* s, enum variant v) {
    if (v == DOUBLE_BROKEN) {
        return 1; /* no 3-way partition to speak of; the semantic checks catch it */
    }
    unsigned a = s->state & 3u, b = s->back, c = s->front;
    return a < 3 && b < 3 && c < 3 && a != b && a != c && b != c;
}

/* Applies one step; returns 0 if the step is not enabled in `s`. */
static int step(const S* s, S* out, int op, enum variant v) {
    *out = *s;
    switch (op) {
    case 0: /* P_DRAW: the producer writes a new frame into its buffer */
        if (s->drawn || s->next_id >= MAX_FRAMES) return 0;
        out->next_id = s->next_id + 1;
        out->content[s->back] = out->next_id;
        out->drawn = 1;
        if (v == DOUBLE_BROKEN) {
            /* ghost check: the consumer must not be reading the buffer being drawn */
            if (s->holding && s->front == s->back) {
                violation = "producer drew into the buffer the consumer is reading";
            }
        }
        return 1;
    case 1: /* P_COMMIT (or the first half of a split commit) */
        if (!s->drawn) return 0;
        if (v == TRIPLE) {
            unsigned old = s->state;
            out->state = (unsigned char)(s->back | NOVA_CHAN_DIRTY);
            out->back = old & 3u;
            out->drawn = 0;
        } else if (v == DOUBLE_BROKEN) {
            /* two buffers {0,1}: publish by flipping */
            out->state = (unsigned char)(s->back | NOVA_CHAN_DIRTY);
            out->back = (unsigned char)(1u - s->back);
            out->drawn = 0;
        } else { /* TRIPLE_SPLIT_COMMIT: load now, store in the next step */
            if (s->pc_phase) return 0;
            out->pc_phase = 1;
            out->pc_old = s->state;
        }
        return 1;
    case 2: /* second half of the split commit */
        if (v != TRIPLE_SPLIT_COMMIT || !s->pc_phase) return 0;
        out->state = (unsigned char)(s->back | NOVA_CHAN_DIRTY);
        out->back = s->pc_old & 3u;
        out->drawn = 0;
        out->pc_phase = 0;
        return 1;
    case 3: /* C_CHECK: the consumer looks at the dirty bit */
        if (s->cphase) return 0;
        out->cphase = 1;
        out->saw_dirty = (s->state & NOVA_CHAN_DIRTY) ? 1 : 0;
        return 1;
    case 4: /* C_XCHG: take the newest frame, if the check saw one */
        if (!s->cphase) return 0;
        out->cphase = 0;
        if (!s->saw_dirty) return 1;
        if (v == DOUBLE_BROKEN) {
            out->front = s->state & 1u;
            out->state = (unsigned char)(s->state & ~NOVA_CHAN_DIRTY);
        } else {
            unsigned old = s->state;
            out->state = (unsigned char)s->front;
            out->front = old & 3u;
        }
        out->holding = 1;
        out->held_id = out->content[out->front];
        if (out->held_id <= s->last_seen) {
            violation = "frames did not arrive in strictly increasing order";
        }
        out->last_seen = out->held_id;
        return 1;
    case 5: /* C_READ: the consumer inspects its held frame */
        if (!s->holding) return 0;
        if (s->content[s->front] != s->held_id) {
            violation = "a frame changed while the consumer held it (torn)";
        }
        return 1;
    }
    return 0;
}

/* Visited-set and queue for the exploration. */
enum { CAP = 1 << 20 };
static S* g_states;
static unsigned char* g_visited;
static S** g_queue;
static long g_count, g_qtail;

static unsigned long hash_state(const S* s) {
    unsigned long h = 1469598103934665603UL;
    const unsigned char* b = (const unsigned char*)s;
    for (unsigned i = 0; i < sizeof(S); i++) {
        h = (h ^ b[i]) * 1099511628211UL;
    }
    return h;
}

static void insert_state(const S* s) {
    unsigned long k = hash_state(s) & (CAP - 1);
    while (g_visited[k] && memcmp(&g_states[k], s, sizeof(S)) != 0) {
        k = (k + 1) & (CAP - 1);
    }
    if (!g_visited[k]) {
        g_visited[k] = 1;
        g_states[k] = *s;
        g_queue[g_qtail++] = &g_states[k];
        g_count++;
    }
}

/* Visits every reachable state. Returns the number of states, or -1 on the
 * first violation (with `violation` set). */
static long explore(enum variant v) {
    if (!g_states) {
        g_states = malloc(sizeof(S) * CAP);
        g_visited = malloc(CAP);
        g_queue = malloc(sizeof(S*) * CAP);
    }
    memset(g_visited, 0, CAP);
    g_count = 0;
    g_qtail = 0;
    violation = NULL;
    S init;
    memset(&init, 0, sizeof init);
    if (v == DOUBLE_BROKEN) {
        init.state = 1; init.back = 0; init.front = 1;
    } else {
        init.state = 2; init.back = 0; init.front = 1;
    }
    insert_state(&init);
    long head = 0;
    while (head < g_qtail) {
        S cur = *g_queue[head++];
        if (!perm_ok(&cur, v)) {
            violation = "buffer indices are not a permutation of {0,1,2}";
            return -1;
        }
        for (int op = 0; op <= 5; op++) {
            S next;
            if (step(&cur, &next, op, v)) {
                if (violation) return -1;
                insert_state(&next);
            }
        }
    }
    return g_count;
}

static void test_model(void) {
    long n = explore(TRIPLE);
    CHECK(n > 100 && violation == NULL, "the real protocol must satisfy every invariant (%ld states) - %s",
          n, violation ? violation : "ok");
    printf("model check: the triple buffer was verified over all %ld reachable states\n", n);

    long b1 = explore(DOUBLE_BROKEN);
    CHECK(b1 < 0 && violation != NULL, "the checker failed to flag a plain double buffer");
    printf("model check: a plain double buffer is correctly flagged: %s\n", violation ? violation : "(none)");

    long b2 = explore(TRIPLE_SPLIT_COMMIT);
    CHECK(b2 < 0 && violation != NULL, "the checker failed to flag a non-atomic commit");
    printf("model check: a load-then-store commit is correctly flagged: %s\n", violation ? violation : "(none)");
}

/* ---------------------------------------------------------------------
 * 2. Threaded stress test of the real code
 * ------------------------------------------------------------------- */

enum { W = 24, H = 10, FRAMES = 300000 };

typedef struct {
    nova_chan_t ch;
} Side;

static void* producer(void* arg) {
    Side* p = (Side*)arg;
    for (unsigned f = 1; f <= FRAMES; f++) {
        unsigned int* px = nova_chan_back(&p->ch);
        /* a frame is W*H pixels all equal to its number: any mixing of two
         * frames shows up as pixels that disagree */
        for (unsigned i = 0; i < W * H; i++) {
            px[i] = f;
            /* Give the consumer the CPU while this frame is only half
             * drawn. On a single host core the two threads otherwise
             * almost never overlap, and "the consumer runs while the
             * producer is mid-frame" is exactly the situation a broken
             * buffer-ownership protocol fails in. */
            if (i == W * H / 2 && (f & 3) == 0) {
                sched_yield();
            }
        }
        nova_chan_commit(&p->ch);
        if ((f & 15) == 0) {
            sched_yield();
        }
    }
    return NULL;
}

static long g_seen, g_torn, g_out_of_order;
static unsigned g_last;

static void* consumer(void* arg) {
    Side* c = (Side*)arg;
    unsigned last = 0;
    for (;;) {
        const unsigned int* px = nova_chan_acquire(&c->ch);
        if (!px) {
            if (last == FRAMES) break;
            sched_yield();
            continue;
        }
        unsigned f = px[0];
        for (unsigned i = 1; i < W * H; i++) {
            if (px[i] != f) { g_torn++; break; }
            /* ...and let the producer run while this frame is only half
             * checked: if the consumer's buffer were also the producer's,
             * the second half would now hold a different frame. */
            if (i == W * H / 2 && (g_seen & 3) == 0) {
                sched_yield();
            }
        }
        if (f <= last) g_out_of_order++;
        last = f;
        g_seen++;
        if (last == FRAMES) break;
    }
    g_last = last;
    return NULL;
}

static void test_stress(void) {
    unsigned bytes = nova_chan_region_bytes(W, H);
    void* region = aligned_alloc(4096, bytes);
    memset(region, 0, bytes);
    CHECK(nova_chan_init(region, bytes, W, H) == 0, "init");
    Side p, c;
    CHECK(nova_chan_attach(&p.ch, region, bytes, 1) == 0 && nova_chan_attach(&c.ch, region, bytes, 0) == 0, "attach");
    pthread_t tp, tc;
    pthread_create(&tc, NULL, consumer, &c);
    pthread_create(&tp, NULL, producer, &p);
    pthread_join(tp, NULL);
    pthread_join(tc, NULL);
    CHECK(g_torn == 0, "%ld torn frames", g_torn);
    CHECK(g_out_of_order == 0, "%ld frames arrived out of order or repeated", g_out_of_order);
    CHECK(g_last == FRAMES, "the final frame never arrived (last=%u)", g_last);
    CHECK(g_seen >= 1 && g_seen <= FRAMES, "frames seen %ld", g_seen);
    CHECK(nova_chan_frames_produced(&p.ch) == FRAMES, "produced count");
    printf("stress: %ld of %d frames reached the consumer (the rest were skipped, "
           "by design), 0 torn, 0 out of order\n", g_seen, FRAMES);
    free(region);
}

/* ---------------------------------------------------------------------
 * 3. Validation
 * ------------------------------------------------------------------- */

static void test_validation(void) {
    CHECK(nova_chan_region_bytes(0, 10) == 0 && nova_chan_region_bytes(10, 0) == 0, "zero dimension");
    CHECK(nova_chan_region_bytes(NOVA_CHAN_MAX_DIM + 1, 1) == 0 && nova_chan_region_bytes(1, NOVA_CHAN_MAX_DIM + 1) == 0, "oversize");
    CHECK(nova_chan_region_bytes(1, 1) == 4096 + 3 * 4096, "1x1 needs a header page + 3 buffer pages");
    CHECK(nova_chan_region_bytes(64, 64) == 4096 + 3 * 16384, "64x64");
    CHECK(nova_chan_region_bytes(33, 33) == 4096 + 3 * 8192, "33x33 rounds each buffer up to whole pages");

    unsigned bytes = nova_chan_region_bytes(32, 8);
    unsigned char* r = aligned_alloc(4096, bytes);
    memset(r, 0, bytes);
    nova_chan_t c;
    CHECK(nova_chan_attach(&c, r, bytes, 1) == -1, "attach before init must be refused (polling callers rely on this)");
    CHECK(nova_chan_init(r, bytes - 1, 32, 8) == -1, "region one byte too small");
    CHECK(nova_chan_init(r, bytes, 0, 8) == -1 && nova_chan_init(NULL, bytes, 32, 8) == -1, "bad init args");
    CHECK(nova_chan_attach(&c, r, bytes, 1) == -1, "failed inits must not leave a valid-looking header");
    CHECK(nova_chan_init(r, bytes, 32, 8) == 0, "init");
    CHECK(nova_chan_attach(&c, r, bytes - 1, 0) == -1, "attach with a region smaller than the channel needs");
    CHECK(nova_chan_attach(&c, r, 100, 0) == -1, "attach with a region smaller than the header");
    CHECK(nova_chan_attach(&c, NULL, bytes, 0) == -1, "null region");

    /* the other process wrote the header: a hostile or buggy one must not
     * be able to make attach() return pointers outside the region */
    nova_chan_header_t* h = (nova_chan_header_t*)r;
    nova_chan_header_t good = *h;
    h->width = 4096; h->height = 4096;
    CHECK(nova_chan_attach(&c, r, bytes, 0) == -1, "header claiming dimensions the region cannot hold");
    *h = good; h->stride = good.stride + 4;
    CHECK(nova_chan_attach(&c, r, bytes, 0) == -1, "inconsistent stride");
    *h = good; h->buffer_bytes = good.buffer_bytes * 4;
    CHECK(nova_chan_attach(&c, r, bytes, 0) == -1, "inflated buffer size");
    *h = good; h->width = 0;
    CHECK(nova_chan_attach(&c, r, bytes, 0) == -1, "zero width");
    *h = good; h->magic = 0;
    CHECK(nova_chan_attach(&c, r, bytes, 0) == -1, "wrong magic");
    *h = good;
    CHECK(nova_chan_attach(&c, r, bytes, 0) == 0, "a restored good header attaches again");

    /* starting indices: producer owns 0, consumer owns 1, 2 is in transit */
    nova_chan_t pr;
    CHECK(nova_chan_attach(&pr, r, bytes, 1) == 0, "attach producer");
    CHECK(pr.idx == 0 && c.idx == 1 && (h->state & 3u) == 2 && !(h->state & NOVA_CHAN_DIRTY), "initial partition");
    CHECK(nova_chan_acquire(&c) == NULL, "nothing committed yet: no frame");
    unsigned int* back = nova_chan_back(&pr);
    CHECK((unsigned char*)back == r + 4096, "producer's first buffer is the first after the header page");
    back[0] = 0xABCD;
    nova_chan_commit(&pr);
    const unsigned int* got = nova_chan_acquire(&c);
    CHECK(got != NULL && got[0] == 0xABCD, "the committed frame is what the consumer receives");
    CHECK(nova_chan_acquire(&c) == NULL, "the same frame is not delivered twice");
    CHECK(nova_chan_current(&c) == got, "current() is the last acquired frame");
    /* two commits before the consumer looks: only the newest survives */
    nova_chan_back(&pr)[0] = 1; nova_chan_commit(&pr);
    nova_chan_back(&pr)[0] = 2; nova_chan_commit(&pr);
    got = nova_chan_acquire(&c);
    CHECK(got != NULL && got[0] == 2, "a slow consumer skips to the newest frame");
    CHECK(nova_chan_acquire(&c) == NULL, "and the skipped frame never appears later");
    CHECK(!nova_chan_stop_requested(&pr), "stop flag starts clear");
    nova_chan_request_stop(&c);
    CHECK(nova_chan_stop_requested(&pr), "stop flag is visible to the producer");
    CHECK(nova_chan_frames_produced(&pr) == 3, "produced counter");
    free(r);
}

int main(void) {
    test_validation();
    test_model();
    test_stress();
    printf("novashm host test: %ld checks, %ld failures\n", checks, failures);
    return failures ? 1 : 0;
}
