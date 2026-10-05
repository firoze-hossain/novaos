#ifndef NOVASHM_CHAN_H
#define NOVASHM_CHAN_H

/*
 * novashm_chan.h - Phase 83: the standard way to hand pixel frames from
 * one process to another through shared memory: a lock-free TRIPLE
 * BUFFER.
 *
 * Why this exists. Shared memory (nova_shm_abi.h) gives two processes the
 * same bytes; it deliberately gives them no synchronisation, because the
 * right protocol depends on the traffic. For frames the right one is this:
 * the PRODUCER (an app) draws into a buffer, the CONSUMER (the compositor)
 * shows the newest complete frame, and NEITHER EVER WAITS FOR THE OTHER. A
 * plain double buffer cannot do that: with two buffers, once the producer
 * has handed one over it must start on the other while the consumer may
 * still be reading it. Three buffers can always be partitioned into one the
 * producer owns, one the consumer owns, and one in transit:
 *
 *      producer owns [back]   consumer owns [front]   [latest] in the
 *                                                       middle, with a
 *                                                       DIRTY bit saying
 *                                                       whether it holds a
 *                                                       frame the consumer
 *                                                       has not taken yet
 *
 *   commit  (producer): atomically swap [back] with [latest]+DIRTY. The
 *           frame just drawn becomes the newest; the producer takes the old
 *           [latest] as its next drawing buffer (any frame the consumer
 *           never took is simply overwritten - the producer is never held
 *           up by a slow consumer).
 *   acquire (consumer): if DIRTY, atomically swap [front] with [latest]
 *           (clearing DIRTY). The consumer now owns the newest complete
 *           frame; the one it finished with goes into transit.
 *
 * Each swap is ONE atomic exchange, so the three indices are always a
 * permutation of {0,1,2}: no buffer is ever owned by both sides, so a
 * frame is never seen half-drawn, and frames arrive in order (some may be
 * skipped, none repeated or reordered). The exchanges use acquire/release
 * ordering, so every pixel written before a commit is visible to the
 * consumer that acquires it. tools/tests/novashm_test.c checks the
 * protocol exhaustively (every interleaving of the atomic steps) and
 * stress-tests this code with two real threads.
 *
 * This header is portable C with no system calls so that test can compile
 * it natively. One producer and one consumer per channel.
 *
 * Layout of the shared region: a 4096-byte header page, then three buffers
 * of `buffer_bytes` each (width*height*4 rounded up to a whole page).
 */

#define NOVA_CHAN_MAGIC         0x4E434841u /* "NCHA" */
#define NOVA_CHAN_HEADER_BYTES  4096u
#define NOVA_CHAN_BUFFERS       3u
#define NOVA_CHAN_DIRTY         4u
#define NOVA_CHAN_MAX_DIM       4096u

typedef struct {
    unsigned int magic;        /* written LAST by init: attach waits for it */
    unsigned int width;
    unsigned int height;
    unsigned int stride;       /* bytes per row */
    unsigned int buffer_bytes; /* per buffer, page multiple */
    unsigned int state;        /* atomic: bits 0-1 = "latest" index, bit 2 = DIRTY */
    unsigned int stop;         /* atomic: set by the consumer, polled by the producer */
    unsigned int produced;     /* atomic: frames committed, for statistics */
} nova_chan_header_t;

typedef struct {
    nova_chan_header_t* hdr;
    unsigned char* bufs;
    unsigned int idx;          /* the buffer THIS side currently owns */
    unsigned int buffer_bytes;
    unsigned int is_producer;
} nova_chan_t;

static inline unsigned int nova_chan_buffer_bytes(unsigned int width, unsigned int height) {
    unsigned int bytes = width * height * 4u;
    return (bytes + 4095u) & ~4095u;
}

/* Bytes the shared object must have; 0 if the dimensions are unusable. */
static inline unsigned int nova_chan_region_bytes(unsigned int width, unsigned int height) {
    if (width == 0 || height == 0 || width > NOVA_CHAN_MAX_DIM || height > NOVA_CHAN_MAX_DIM) {
        return 0;
    }
    return NOVA_CHAN_HEADER_BYTES + NOVA_CHAN_BUFFERS * nova_chan_buffer_bytes(width, height);
}

/* Initialises a channel in `region` (zero-filled shared memory is the
 * expected input, which is what SHM_CREATE returns). Call once, before
 * either side attaches. Returns 0, or -1 for bad dimensions / a region too
 * small. The magic is stored last with release ordering, so an attacher
 * that sees it sees the rest. */
static inline int nova_chan_init(void* region, unsigned int region_bytes,
                                 unsigned int width, unsigned int height) {
    unsigned int need = nova_chan_region_bytes(width, height);
    if (region == 0 || need == 0 || region_bytes < need) {
        return -1;
    }
    nova_chan_header_t* h = (nova_chan_header_t*)region;
    h->width = width;
    h->height = height;
    h->stride = width * 4u;
    h->buffer_bytes = nova_chan_buffer_bytes(width, height);
    __atomic_store_n(&h->state, 2u, __ATOMIC_RELAXED); /* latest = 2, clean */
    __atomic_store_n(&h->stop, 0u, __ATOMIC_RELAXED);
    __atomic_store_n(&h->produced, 0u, __ATOMIC_RELAXED);
    __atomic_store_n(&h->magic, NOVA_CHAN_MAGIC, __ATOMIC_RELEASE);
    return 0;
}

/* Attaches one side. Validates everything it reads from shared memory -
 * the other process wrote it, so it is untrusted. Returns 0, or -1 if the
 * channel is not initialised yet (callers poll) or is malformed. */
static inline int nova_chan_attach(nova_chan_t* c, void* region,
                                   unsigned int region_bytes, int is_producer) {
    if (region == 0 || region_bytes < NOVA_CHAN_HEADER_BYTES) {
        return -1;
    }
    nova_chan_header_t* h = (nova_chan_header_t*)region;
    if (__atomic_load_n(&h->magic, __ATOMIC_ACQUIRE) != NOVA_CHAN_MAGIC) {
        return -1;
    }
    unsigned int w = h->width, ht = h->height;
    unsigned int need = nova_chan_region_bytes(w, ht);
    if (need == 0 || region_bytes < need || h->buffer_bytes != nova_chan_buffer_bytes(w, ht) ||
        h->stride != w * 4u) {
        return -1;
    }
    c->hdr = h;
    c->bufs = (unsigned char*)region + NOVA_CHAN_HEADER_BYTES;
    c->buffer_bytes = h->buffer_bytes;
    c->is_producer = is_producer ? 1u : 0u;
    c->idx = is_producer ? 0u : 1u; /* producer: back = 0; consumer: front = 1 */
    return 0;
}

/* PRODUCER: the buffer to draw the next frame into (width*height XRGB
 * pixels, row stride hdr->stride). It is yours alone until you commit. */
static inline unsigned int* nova_chan_back(const nova_chan_t* c) {
    return (unsigned int*)(c->bufs + c->idx * c->buffer_bytes);
}

/* PRODUCER: publish the frame in nova_chan_back() as the newest, and take
 * a fresh buffer to draw into. Never blocks. */
static inline void nova_chan_commit(nova_chan_t* c) {
    unsigned int old = __atomic_exchange_n(&c->hdr->state, c->idx | NOVA_CHAN_DIRTY, __ATOMIC_ACQ_REL);
    c->idx = old & 3u;
    __atomic_fetch_add(&c->hdr->produced, 1u, __ATOMIC_RELAXED);
}

/* CONSUMER: if a newer complete frame has been committed, take it and
 * return its pixels; otherwise return NULL (keep showing the last one,
 * which nova_chan_current() still returns). Never blocks. */
static inline const unsigned int* nova_chan_acquire(nova_chan_t* c) {
    if (!(__atomic_load_n(&c->hdr->state, __ATOMIC_RELAXED) & NOVA_CHAN_DIRTY)) {
        return 0;
    }
    unsigned int old = __atomic_exchange_n(&c->hdr->state, c->idx, __ATOMIC_ACQ_REL);
    c->idx = old & 3u;
    return (const unsigned int*)(c->bufs + c->idx * c->buffer_bytes);
}

/* CONSUMER: the frame most recently acquired (all zero before the first). */
static inline const unsigned int* nova_chan_current(const nova_chan_t* c) {
    return (const unsigned int*)(c->bufs + c->idx * c->buffer_bytes);
}

/* CONSUMER asks the producer to stop; PRODUCER polls. */
static inline void nova_chan_request_stop(nova_chan_t* c) {
    __atomic_store_n(&c->hdr->stop, 1u, __ATOMIC_RELEASE);
}
static inline int nova_chan_stop_requested(const nova_chan_t* c) {
    return __atomic_load_n(&c->hdr->stop, __ATOMIC_ACQUIRE) != 0;
}
static inline unsigned int nova_chan_frames_produced(const nova_chan_t* c) {
    return __atomic_load_n(&c->hdr->produced, __ATOMIC_RELAXED);
}

#endif
