/*
 * audiotest.c - Phase 85: the in-OS conformance test for the SYS_AUDIO_*
 * audio-mixing syscalls. A real ring-3 program making real int 0x80 calls
 * against the real kernel and the real (emulated) AC97 sound card, and -
 * because the point is several apps sharing one output - forking real
 * peers that play at the same time. kernel/task/exec_trust_demo.c runs it
 * at boot and tools/python/test_runner.py checks its "[audiotest] ok:" /
 * "[audiotest] PASS" lines. It is the in-OS counterpart of the host tests
 * (kernel/rust/mixer.rs against a mock that simulates the DMA engine and an
 * independent oracle): those prove the logic, this proves the whole stack -
 * syscall, user-pointer validation, kernel-stamped identity, fork and exit,
 * the timer tick that feeds the card, and the card itself.
 *
 * HOW IT SEES WHAT WAS MIXED. A test cannot hear. The mixer keeps a TAP of
 * its most recent output, readable by root (and this program runs as root):
 * the program feeds known signals into its own streams and its children's,
 * then reads back the mix and checks it EXACTLY (constants: the sum of two
 * streams is the sum, volumes scale it, the master scales the lot, a mono
 * stream is in both channels, saturation clips and is counted) or by
 * integer Goertzel analysis (tones: two apps playing 440Hz and 660Hz at
 * different sample rates and channel counts both appear in the output at
 * their own pitch and nowhere else). Floating point is deliberately not
 * used: ring-3 code shares the FPU with every other process and the kernel
 * does not save FPU state on a task switch.
 *
 * The properties, each a way the feature could be wrong in a way a plain
 * "it returned 0" would miss:
 *
 *  - MIXING IS ARITHMETIC, NOT REPLACEMENT: the output is the saturated sum.
 *  - FORMATS: mono is duplicated to both channels; a stream at 22050Hz keeps
 *    its pitch when converted to the card's 48kHz.
 *  - SHARING ACROSS PROCESSES: a forked child's stream mixes with its
 *    parent's, and disappears from the mix when the child exits.
 *  - THE BEEP MIXES: SYS_BEEP adds a tone to what is playing instead of
 *    taking the card over.
 *  - PRIVILEGE: the master volume and the tap are root-only; another user's
 *    process can still play, and cannot touch anyone else's stream.
 *  - NOTHING LEAKS: sixty-odd stream slots' worth of short-lived owners
 *    leave nothing behind, and the card is allowed to go idle when silent.
 *  - THE CARD IS REALLY BEING FED: the mixer's frame counter advances at
 *    roughly the real-time rate while something plays.
 *
 * Every failure prints "[audiotest] FAIL: ..." (the uppercase word also
 * trips test_runner.py's global no-fail assertion). Output for passing
 * checks deliberately avoids the words that assertion greps for.
 */
#include <errno.h>
#include <novaaudio.h>
#include <novashm.h>
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failures;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 25) { \
            printf("[audiotest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

#define EXPECT(call, want) do { \
    int rc_ = (call); \
    checks++; \
    if (rc_ != (want)) { \
        failures++; \
        if (failures <= 25) \
            printf("[audiotest] FAIL: %s returned %d, expected %d (line %d)\n", \
                   #call, rc_, (int)(want), __LINE__); \
    } \
} while (0)

#define UNITY NOVA_AUDIO_UNITY

static int now_s(void) {
    return nova_audio_now_s();
}

static int elapsed_s(int start) {
    int e = now_s() - start;
    return e < 0 ? e + 86400 : e;
}

/* A shared-memory mailbox (Phase 83) for sequencing children WITHOUT using the
 * audio under test. */
typedef struct {
    volatile unsigned int go;     /* the child may start playing */
    volatile unsigned int stop;   /* the child must stop and exit */
    volatile unsigned int ready;  /* the child's stream is open and feeding */
} mailbox_t;
static nova_shm_region_t mbr;
static mailbox_t* mb;

static void stage_set(volatile unsigned int* v, unsigned int n) {
    __atomic_store_n(v, n, __ATOMIC_RELEASE);
}
static int stage_wait(volatile unsigned int* v, unsigned int want, int timeout_s) {
    int start = now_s();
    while (__atomic_load_n(v, __ATOMIC_ACQUIRE) < want) {
        if (elapsed_s(start) > timeout_s) return 0;
        sys_yield();
    }
    return 1;
}

/* ---- feeders: keep streams topped up with a known signal --------------------- */

enum { MODE_DC = 0, MODE_TONE = 1 };

typedef struct {
    nova_audio_stream_t s;
    int open;
    int mode;
    short dc;
    unsigned int hz, amp, phase;
} feeder_t;

static feeder_t F[4];

static int feeder_open(feeder_t* f, unsigned int channels, unsigned int rate, unsigned int volume) {
    memset(f, 0, sizeof *f);
    int rc = nova_audio_open(&f->s, channels, rate, volume);
    f->open = (rc == 0);
    return rc;
}

static void feeder_dc(feeder_t* f, short value) {
    nova_audio_flush(&f->s); /* drop the old signal still buffered */
    f->mode = MODE_DC;
    f->dc = value;
}

static void feeder_tone(feeder_t* f, unsigned int hz, unsigned int amp) {
    nova_audio_flush(&f->s);
    f->mode = MODE_TONE;
    f->hz = hz;
    f->amp = amp;
}

/* Tops one stream up to ~50ms of audio of its current signal. */
static void feed_one(feeder_t* f) {
    static short buf[480 * 2];
    if (!f->open) return;
    unsigned int st[10];
    if (nova_audio_stream_stat(&f->s, st) != 0) return;
    unsigned int want = f->s.rate / 20; /* 50ms of input frames */
    if (st[NOVA_AUDIO_SS_QUEUED] >= want) return;
    unsigned int frames = 480;
    if (f->mode == MODE_DC) {
        for (unsigned int i = 0; i < frames * f->s.channels; i++) buf[i] = f->dc;
    } else {
        nova_audio_fill_tone(buf, frames, f->s.channels, f->s.rate, f->hz, (int)f->amp, &f->phase);
    }
    (void)nova_audio_write(&f->s, buf, frames);
}

static void feed_all(void) {
    for (int i = 0; i < 4; i++) feed_one(&F[i]);
}

/* ---- reading the mix back ------------------------------------------------------ */

static short tapbuf[2048];

static int tap_now(void) {
    return nova_audio_tap(tapbuf, 1024, 0);
}

/* Longest run of consecutive tap frames where BOTH channels equal `v`. */
static int longest_run(short v) {
    int best = 0, cur = 0;
    for (int i = 0; i < 1024; i++) {
        if (tapbuf[i * 2] == v && tapbuf[i * 2 + 1] == v) {
            if (++cur > best) best = cur;
        } else {
            cur = 0;
        }
    }
    return best;
}

/* Keeps the feeders running until the mix settles at the constant `v` (a run
 * of at least 300 frames in the tap), or `timeout_s` passes. */
static int wait_for_level(short v, int timeout_s) {
    int start = now_s();
    for (;;) {
        feed_all();
        if (tap_now() == 1024 && longest_run(v) >= 300) return 1;
        if (elapsed_s(start) > timeout_s) return 0;
        sys_yield();
    }
}

/* Goertzel: the power of `x` (stride 2: the left channel of an interleaved
 * buffer) at the frequency whose coefficient 2cos(w) is `coeff` (Q14), in
 * integer arithmetic. Samples are scaled down so nothing overflows. */
static long long goertzel(const short* x, int n, int coeff) {
    long long s1 = 0, s2 = 0;
    for (int i = 0; i < n; i++) {
        long long s = (long long)(x[i * 2] >> 2) + (((long long)coeff * s1) >> 14) - s2;
        s2 = s1;
        s1 = s;
    }
    return s1 * s1 + s2 * s2 - (((long long)coeff * s1) >> 14) * s2;
}

enum { C440 = 32714, C550 = 32683, C660 = 32646, C700 = 32631, C1000 = 32488, C1500 = 32138 };

/* ---- group 1: one process: validation, limits, buffers, hostile pointers ---------- */

static void test_basics(void) {
    int f0 = failures;
    nova_audio_stream_t s[5];

    EXPECT(nova_audio_open(&s[0], 0, 48000, UNITY), -EINVAL);
    EXPECT(nova_audio_open(&s[0], 3, 48000, UNITY), -EINVAL);
    EXPECT(nova_audio_open(&s[0], 2, 7999, UNITY), -EINVAL);
    EXPECT(nova_audio_open(&s[0], 2, 48001, UNITY), -EINVAL);
    EXPECT(nova_audio_open(&s[0], 2, 48000, UNITY + 1), -EINVAL);
    nova_audio_open_t o;
    o.channels = 2; o.rate = 48000; o.flags = 1; o.volume = UNITY; o.handle = 0; o.capacity_frames = 0; o.period_frames = 0; o.reserved = 0;
    EXPECT(sys_audio_open(&o), -EINVAL);

    EXPECT(nova_audio_open(&s[0], 2, 48000, UNITY), 0);
    CHECK(s[0].capacity_frames == 8192, "a stereo stream buffers %u frames", s[0].capacity_frames);
    EXPECT(nova_audio_open(&s[1], 1, 22050, UNITY), 0);
    CHECK(s[1].capacity_frames == 16384, "a mono stream buffers %u frames", s[1].capacity_frames);
    EXPECT(nova_audio_open(&s[2], 2, 44100, 100), 0);
    EXPECT(nova_audio_open(&s[3], 2, 8000, 0), 0);
    EXPECT(nova_audio_open(&s[4], 2, 48000, UNITY), -ENOSPC); /* 4 per process */

    /* a paused stream fills up and then says "again": nothing ever blocks */
    EXPECT(nova_audio_pause(&s[0]), 0);
    static short pcm[8192 * 2];
    int n = nova_audio_write(&s[0], pcm, 8192);
    CHECK(n == 8192, "an empty stream took %d of 8192 frames", n);
    EXPECT(nova_audio_write(&s[0], pcm, 100), -EAGAIN);
    unsigned int st[10];
    EXPECT(nova_audio_stream_stat(&s[0], st), 0);
    CHECK(st[NOVA_AUDIO_SS_QUEUED] == 8192 && st[NOVA_AUDIO_SS_CAPACITY] == 8192 && st[NOVA_AUDIO_SS_FLAGS] == 1 &&
          st[NOVA_AUDIO_SS_PLAYED] == 0 && st[NOVA_AUDIO_SS_UNDERRUN_FRAMES] == 0,
          "paused stat: queued %u flags %u played %u underruns %u", st[0], st[7], st[2], st[5]);
    EXPECT(nova_audio_flush(&s[0]), 0);
    EXPECT(nova_audio_stream_stat(&s[0], st), 0);
    CHECK(st[NOVA_AUDIO_SS_QUEUED] == 0, "flush left %u frames", st[NOVA_AUDIO_SS_QUEUED]);

    /* controls validate */
    EXPECT(nova_audio_set_volume(&s[2], UNITY + 1), -EINVAL);
    EXPECT(nova_audio_set_volume(&s[2], 50), 0);
    EXPECT(nova_audio_ctl(99, s[2].handle, 0, 0, 0), -EINVAL);
    EXPECT(nova_audio_set_master(UNITY + 1, 0), -EINVAL);
    nova_audio_stream_t bogus = {0x7777, 2, 48000, 100};
    EXPECT(nova_audio_write(&bogus, pcm, 10), -EBADF);
    EXPECT(nova_audio_set_volume(&bogus, 1), -EBADF);
    EXPECT(nova_audio_close(&bogus), -EBADF);

    /* a closed handle is dead; a reused slot does not honour the old handle */
    nova_audio_stream_t old = s[3];
    EXPECT(nova_audio_close(&s[3]), 0);
    EXPECT(nova_audio_write(&old, pcm, 10), -EBADF);
    EXPECT(nova_audio_close(&old), -EBADF);
    nova_audio_stream_t again;
    EXPECT(nova_audio_open(&again, 2, 8000, 0), 0);
    CHECK(again.handle != old.handle, "a reused slot reissued the identical handle 0x%x", old.handle);
    EXPECT(nova_audio_write(&old, pcm, 10), -EBADF);

    /* hostile pointers to every call, and a good request whose DATA pointer is bad */
    void* nowhere[] = { (void*)0, (void*)0x00100000, (void*)0x50000000, (void*)0xFFFFFFF8 };
    for (unsigned i = 0; i < sizeof nowhere / sizeof nowhere[0]; i++) {
        void* q = nowhere[i];
        EXPECT(sys_audio_open((nova_audio_open_t*)q), -EFAULT);
        EXPECT(sys_audio_write((nova_audio_write_t*)q), -EFAULT);
        EXPECT(sys_audio_ctl((nova_audio_ctl_t*)q), -EFAULT);
        EXPECT(sys_audio_close((const nova_audio_close_t*)q), -EFAULT);
        EXPECT(nova_audio_write(&again, (const short*)q, 100), -EFAULT);
        nova_audio_ctl_t c;
        memset(&c, 0, sizeof c);
        c.op = NOVA_AUDIO_CTL_TAP_READ; c.buf = (unsigned int)(unsigned long)q; c.frames = 64;
        EXPECT(sys_audio_ctl(&c), -EFAULT); /* root, so the pointer is what fails */
    }

    nova_audio_abort(&again);
    for (int i = 0; i < 3; i++) nova_audio_abort(&s[i]);
    unsigned int ms[10];
    EXPECT(nova_audio_mixer_stat(ms), 0);
    CHECK(ms[NOVA_AUDIO_MS_ACTIVE] == 0, "%u streams left open", ms[NOVA_AUDIO_MS_ACTIVE]);

    if (failures == f0) printf("[audiotest] ok: streams - validation, per-process limit, buffers that say again, stale and hostile handles and pointers\n");
}

/* ---- group 2: mixing is arithmetic, checked on the mixer's own output -------------- */

static void child_player(short dc, unsigned int volume) {
    failures = 0;
    feeder_t f;
    CHECK(feeder_open(&f, 2, 48000, volume) == 0, "child: open");
    f.mode = MODE_DC;
    f.dc = dc;
    stage_set(&mb->ready, 1);
    int start = now_s();
    while (!__atomic_load_n(&mb->stop, __ATOMIC_ACQUIRE) && elapsed_s(start) < 20) {
        feed_one(&f);
        sys_yield();
    }
    sys_exit(failures); /* no close: the exit hook must remove the stream */
}

static void test_mixing(void) {
    int f0 = failures;
    unsigned int ms[10], ms2[10];

    /* one stereo stream */
    CHECK(feeder_open(&F[0], 2, 48000, UNITY) == 0, "open S1");
    feeder_dc(&F[0], 1000);
    CHECK(wait_for_level(1000, 6), "the mix never settled at S1's 1000");

    /* + a MONO stream: it must appear in BOTH channels, and the two must SUM */
    CHECK(feeder_open(&F[1], 1, 48000, UNITY) == 0, "open S2");
    feeder_dc(&F[1], 500);
    CHECK(wait_for_level(1500, 6), "1000 + a mono 500 did not mix to 1500 in both channels");

    /* per-stream volume scales that stream alone: 500 at volume 128 is 250 */
    EXPECT(nova_audio_set_volume(&F[1].s, 128), 0);
    CHECK(wait_for_level(1250, 6), "stream volume 128/256 did not halve the mono stream (expected 1250)");

    /* the master volume scales the SUM: (1000 + 250) * 128/256 = 625; mute is 0 */
    EXPECT(nova_audio_set_master(128, 0), 0);
    CHECK(wait_for_level(625, 6), "master volume 128/256 did not halve the mix (expected 625)");
    EXPECT(nova_audio_set_master(128, 1), 0);
    CHECK(wait_for_level(0, 6), "the mute did not silence the mix");
    EXPECT(nova_audio_mixer_stat(ms), 0);
    CHECK(ms[NOVA_AUDIO_MS_MUTED] == 1 && ms[NOVA_AUDIO_MS_MASTER] == 128, "stat: muted %u master %u", ms[7], ms[6]);
    EXPECT(nova_audio_set_master(UNITY, 0), 0);
    CHECK(wait_for_level(1250, 6), "restoring the master did not restore the mix");

    /* saturation: 30000 + 30000 clips to 32767, and the clipping is COUNTED */
    EXPECT(nova_audio_set_volume(&F[1].s, UNITY), 0);
    EXPECT(nova_audio_mixer_stat(ms), 0);
    feeder_dc(&F[0], 30000);
    feeder_dc(&F[1], 30000);
    CHECK(wait_for_level(32767, 6), "30000 + 30000 did not saturate to 32767");
    EXPECT(nova_audio_mixer_stat(ms2), 0);
    CHECK(ms2[NOVA_AUDIO_MS_CLIPPED] > ms[NOVA_AUDIO_MS_CLIPPED], "saturated samples were not counted (%u -> %u)",
          ms[NOVA_AUDIO_MS_CLIPPED], ms2[NOVA_AUDIO_MS_CLIPPED]);
    feeder_dc(&F[0], -30000);
    feeder_dc(&F[1], -30000);
    CHECK(wait_for_level(-32768, 6), "-30000 + -30000 did not saturate to -32768");

    /* a stream at volume 0 is silent but still there */
    feeder_dc(&F[0], 1000);
    feeder_dc(&F[1], 700);
    EXPECT(nova_audio_set_volume(&F[1].s, 0), 0);
    CHECK(wait_for_level(1000, 6), "a stream at volume 0 still contributed");
    nova_audio_abort(&F[1].s);
    F[1].open = 0;

    /* ANOTHER PROCESS's stream mixes with ours, and leaves the mix when it exits */
    stage_set(&mb->ready, 0);
    stage_set(&mb->stop, 0);
    int pid = sys_fork();
    if (pid == 0) {
        child_player(3000, UNITY);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    CHECK(stage_wait(&mb->ready, 1, 10), "the child never opened its stream");
    CHECK(wait_for_level(4000, 8), "a child's stream (3000) did not mix with ours (1000) to 4000");
    EXPECT(nova_audio_mixer_stat(ms), 0);
    CHECK(ms[NOVA_AUDIO_MS_ACTIVE] == 2, "two processes' streams should be open (%u)", ms[NOVA_AUDIO_MS_ACTIVE]);
    stage_set(&mb->stop, 1);
    int code = sys_wait(pid); /* the child exits WITHOUT closing its stream */
    CHECK(code == 0, "the child reported %d failed checks", code);
    CHECK(wait_for_level(1000, 6), "the exited child's stream is still in the mix");
    EXPECT(nova_audio_mixer_stat(ms), 0);
    CHECK(ms[NOVA_AUDIO_MS_ACTIVE] == 1, "the exited child's stream was not removed (%u open)", ms[NOVA_AUDIO_MS_ACTIVE]);

    nova_audio_abort(&F[0].s);
    F[0].open = 0;
    if (failures == f0) printf("[audiotest] ok: mixing is arithmetic - exact sums, mono in both channels, stream and master volume, mute, counted saturation, a child's stream mixes in and leaves at exit\n");
}

/* A write bigger than the kernel's 2KB copy chunk is copied in several pieces.
 * Every other write in this program is one chunk of constant or tone data, which
 * could not reveal a wrong offset in the later chunks. A RAMP can: the mixer's
 * output must be a contiguous run of consecutive values. */
static void test_chunked_write(void) {
    int f0 = failures;
    static short ramp[4096 * 2];
    nova_audio_stream_t s;
    EXPECT(nova_audio_open(&s, 2, 48000, UNITY), 0);
    for (int i = 0; i < 4096; i++) {
        ramp[i * 2] = (short)(i + 1000);        /* left: +1 per frame */
        ramp[i * 2 + 1] = (short)(-3 * i - 5);  /* right: -3 per frame */
    }
    int n = nova_audio_write(&s, ramp, 4096);
    CHECK(n == 4096, "a 4096-frame write took %d frames", n);
    /* poll the tap as the ramp plays through it (it lasts ~85ms) for a long run
     * of exactly +1 / -3 steps. A stereo chunk is 512 frames, so ONE chunk holds
     * at most 511 steps: a run of 600 or more can only exist if the chunks join
     * up correctly (a wrong offset repeats a chunk and breaks the run at its
     * boundary - a threshold of 300 would let that through). */
    int start = now_s(), best = 0;
    while (elapsed_s(start) < 4 && best < 600) {
        if (tap_now() == 1024) {
            int run = 0;
            for (int i = 1; i < 1024; i++) {
                int dl = tapbuf[i * 2] - tapbuf[(i - 1) * 2];
                int dr = tapbuf[i * 2 + 1] - tapbuf[(i - 1) * 2 + 1];
                if (dl == 1 && dr == -3) { if (++run > best) best = run; } else run = 0;
            }
        }
    }
    CHECK(best >= 600, "the multi-chunk write was not copied contiguously: the longest ramp run in the mix is %d frames (one chunk is at most 511)", best);
    nova_audio_abort(&s);
    if (failures == f0) printf("[audiotest] ok: a write larger than the kernel copy chunk arrives contiguous (a 4096-frame ramp plays out as exact steps)\n");
}

/* ---- group 3: different formats keep their pitch --------------------------------------- */

static void test_formats(void) {
    int f0 = failures;
    /* app A: stereo 48kHz, 440Hz.  app B: MONO 22050Hz, 660Hz. */
    CHECK(feeder_open(&F[0], 2, 48000, UNITY) == 0, "open A");
    CHECK(feeder_open(&F[1], 1, 22050, UNITY) == 0, "open B");
    feeder_tone(&F[0], 440, 12000);
    feeder_tone(&F[1], 660, 12000);

    /* let them play a moment, then analyse the newest 1024 frames of the mix */
    int start = now_s();
    long long p440 = 0, p550 = 0, p660 = 0, p1500 = 0;
    int ok = 0;
    while (elapsed_s(start) < 8) {
        feed_all();
        if (tap_now() != 1024) continue;
        p440 = goertzel(tapbuf, 1024, C440);
        p660 = goertzel(tapbuf, 1024, C660);
        p550 = goertzel(tapbuf, 1024, C550);
        p1500 = goertzel(tapbuf, 1024, C1500);
        if (p440 > 8 * p550 && p660 > 8 * p550 && p440 > 8 * p1500 && p660 > 8 * p1500 && p440 > 1000 && p660 > 1000) { ok = 1; break; }
        sys_yield();
    }
    CHECK(ok, "the mix does not contain both tones at their pitches: P(440)=%d P(660)=%d P(550)=%d P(1500)=%d (scaled)",
          (int)(p440 >> 20), (int)(p660 >> 20), (int)(p550 >> 20), (int)(p1500 >> 20));

    /* the 22050Hz mono tone ALONE: still 660Hz, not shifted by the rate conversion */
    EXPECT(nova_audio_pause(&F[0].s), 0);
    nova_audio_flush(&F[0].s);
    start = now_s();
    ok = 0;
    while (elapsed_s(start) < 8) {
        feed_one(&F[1]);
        if (tap_now() != 1024) continue;
        p440 = goertzel(tapbuf, 1024, C440);
        p660 = goertzel(tapbuf, 1024, C660);
        p550 = goertzel(tapbuf, 1024, C550);
        if (p660 > 8 * p550 && p660 > 20 * p440 && p660 > 1000) { ok = 1; break; }
        sys_yield();
    }
    CHECK(ok, "a 660Hz tone at 22050Hz mono did not keep its pitch when converted (P660 %d, P550 %d, P440 %d)",
          (int)(p660 >> 20), (int)(p550 >> 20), (int)(p440 >> 20));
    nova_audio_abort(&F[0].s);
    nova_audio_abort(&F[1].s);
    F[0].open = F[1].open = 0;
    if (failures == f0) printf("[audiotest] ok: formats - a 440Hz stereo/48k app and a 660Hz mono/22.05k app both appear in the mix at their own pitch\n");
}

/* ---- group 4: the beep mixes instead of taking the card over --------------------------- */

static void test_beep(void) {
    int f0 = failures;
    CHECK(feeder_open(&F[0], 2, 48000, UNITY) == 0, "open");
    feeder_dc(&F[0], 1000);
    CHECK(wait_for_level(1000, 6), "the stream never settled at 1000");
    EXPECT(sys_beep(), 1);
    /* the beep is +-8000 of square wave ON TOP of the stream's constant 1000 */
    int start = now_s(), saw = 0;
    while (elapsed_s(start) < 5) {
        feed_all();
        if (tap_now() == 1024) {
            int n = 0;
            for (int i = 0; i < 1024; i++) if (tapbuf[i * 2] == 9000 || tapbuf[i * 2] == -7000) n++;
            if (n >= 200) { saw = 1; break; }
        }
        sys_yield();
    }
    CHECK(saw, "SYS_BEEP did not mix a tone on top of the playing stream (expected frames of 9000 / -7000)");
    /* ...and the stream underneath was not cut off: once the 0.3s tone ends it is alone again */
    CHECK(wait_for_level(1000, 8), "after the beep the stream is not playing any more - the beep took the card over");
    nova_audio_abort(&F[0].s);
    F[0].open = 0;
    if (failures == f0) printf("[audiotest] ok: SYS_BEEP adds a tone to what is playing (1000 +- 8000) and the stream underneath carries on\n");
}

/* ---- group 5: counters, draining, exit cleanup, the card really being fed ---------------- */

static void churn_child(void) {
    failures = 0;
    nova_audio_stream_t st[4];
    static short pcm[480 * 2];
    for (int i = 0; i < 4; i++) {
        CHECK(nova_audio_open(&st[i], 2, 48000, UNITY) == 0, "churn child: open %d (earlier owners' slots must be free)", i);
        (void)nova_audio_write(&st[i], pcm, 480);
    }
    sys_exit(failures); /* four streams with data in them, no close */
}

static void test_lifecycle(void) {
    int f0 = failures;
    unsigned int ms[10], st[10];
    nova_audio_stream_t s;
    static short pcm[8192 * 2];

    /* a stream that plays out its data, then runs dry, reports exactly that */
    for (int i = 0; i < 4800 * 2; i++) pcm[i] = 200;
    EXPECT(nova_audio_open(&s, 2, 48000, UNITY), 0);
    EXPECT(nova_audio_write(&s, pcm, 4800), 4800);
    int start = now_s(), done = 0;
    while (elapsed_s(start) < 6) {
        if (nova_audio_stream_stat(&s, st) == 0 && st[NOVA_AUDIO_SS_QUEUED] == 0 && st[NOVA_AUDIO_SS_PLAYED] == 4800 &&
            st[NOVA_AUDIO_SS_UNDERRUN_EVENTS] >= 1) { done = 1; break; }
        sys_yield();
    }
    CHECK(done, "after playing 4800 frames: queued %u played %u underruns %u (expected 0 / 4800 / >= 1)",
          st[NOVA_AUDIO_SS_QUEUED], st[NOVA_AUDIO_SS_PLAYED], st[NOVA_AUDIO_SS_UNDERRUN_EVENTS]);
    CHECK(st[NOVA_AUDIO_SS_WRITTEN] == 4800, "written counter %u", st[NOVA_AUDIO_SS_WRITTEN]);

    /* close DRAINS: with 170ms buffered the stream lingers (closing), then is gone */
    for (int i = 0; i < 8000 * 2; i++) pcm[i] = 200;
    EXPECT(nova_audio_write(&s, pcm, 8000), 8000);
    EXPECT(nova_audio_close(&s), 0);
    int rc = nova_audio_stream_stat(&s, st);
    CHECK(rc == 0 && (st[NOVA_AUDIO_SS_FLAGS] & 2), "a closing stream with data buffered should still exist, flagged closing (rc %d flags %u)", rc, st[NOVA_AUDIO_SS_FLAGS]);
    EXPECT(nova_audio_write(&s, pcm, 10), -EBADF); /* but takes no more data */
    start = now_s();
    while (elapsed_s(start) < 6 && nova_audio_stream_stat(&s, st) == 0) sys_yield();
    EXPECT(nova_audio_stream_stat(&s, st), -EBADF); /* played out, then freed */

    /* sixty streams' worth of short-lived owners (20 processes x 4 streams, no close):
     * the table holds 8, so any leak makes a later child's open fail */
    for (int i = 0; i < 20; i++) {
        int pid = sys_fork();
        if (pid == 0) churn_child();
        if (pid < 0) { CHECK(0, "fork failed"); break; }
        int code = sys_wait(pid);
        CHECK(code == 0, "churn child %d reported %d failed checks", i, code);
    }
    EXPECT(nova_audio_mixer_stat(ms), 0);
    CHECK(ms[NOVA_AUDIO_MS_ACTIVE] == 0, "%u streams were leaked by exited processes", ms[NOVA_AUDIO_MS_ACTIVE]);

    /* the card is REALLY being fed: while a stream plays, the mixer's frame counter
     * advances at roughly the real-time rate (48000/s). Measured over 3 RTC seconds,
     * starting on a second boundary. */
    CHECK(feeder_open(&F[0], 2, 48000, UNITY) == 0, "open");
    feeder_dc(&F[0], 100);
    int t0 = now_s();
    while (now_s() == t0) { feed_all(); sys_yield(); }
    EXPECT(nova_audio_mixer_stat(ms), 0);
    unsigned int frames0 = ms[NOVA_AUDIO_MS_MIXED];
    CHECK(ms[NOVA_AUDIO_MS_HW_RUNNING] == 1, "the DMA engine should be running while a stream plays");
    int tstart = now_s();
    while (elapsed_s(tstart) < 3) { feed_all(); sys_yield(); }
    EXPECT(nova_audio_mixer_stat(ms), 0);
    unsigned int per_s = (ms[NOVA_AUDIO_MS_MIXED] - frames0) / 3;
    CHECK(per_s >= 24000 && per_s <= 72000, "the mixer produced %u frames/s; the card consumes 48000/s", per_s);
    printf("[audiotest] note: the mixer produced %u frames/s while a stream played (the card consumes 48000/s)\n", per_s);
    CHECK(ms[NOVA_AUDIO_MS_RESTARTS] >= 1, "the DMA engine was never started");

    /* and when everything is silent the card is allowed to go idle */
    nova_audio_abort(&F[0].s);
    F[0].open = 0;
    start = now_s();
    int idle = 0;
    while (elapsed_s(start) < 6) {
        if (nova_audio_mixer_stat(ms) == 0 && ms[NOVA_AUDIO_MS_HW_RUNNING] == 0 && ms[NOVA_AUDIO_MS_QUEUED_PERIODS] == 0) { idle = 1; break; }
        sys_yield();
    }
    CHECK(idle, "the DMA engine never went idle after the last stream closed (running %u, %u periods queued)",
          ms[NOVA_AUDIO_MS_HW_RUNNING], ms[NOVA_AUDIO_MS_QUEUED_PERIODS]);

    if (failures == f0) printf("[audiotest] ok: lifecycle - counters, close drains then frees, 20 exiting owners leak nothing, the card is fed in real time and idles when silent\n");
}

/* ---- group 6: privilege --------------------------------------------------------------------- */

static void test_privilege(void) {
    int f0 = failures;
    CHECK(feeder_open(&F[0], 2, 48000, UNITY) == 0, "open");
    feeder_dc(&F[0], 1000);
    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        /* a different user: ordinary playback is fine, everyone-affecting controls are not */
        CHECK(sys_login("persisted", "persisted-pw") == 0, "child: login");
        CHECK(sys_getuid() == 700, "child: uid is %u", sys_getuid());
        EXPECT(nova_audio_set_master(10, 1), -EPERM);
        static short b[64];
        EXPECT(nova_audio_tap(b, 32, 0), -EPERM);
        nova_audio_ctl_t c;
        memset(&c, 0, sizeof c);
        c.op = NOVA_AUDIO_CTL_TAP_READ; c.buf = 0; c.frames = 32; /* a bad pointer must not be reported before privilege */
        EXPECT(sys_audio_ctl(&c), -EPERM);
        nova_audio_stream_t mine;
        static short pcm[480 * 2];
        EXPECT(nova_audio_open(&mine, 2, 48000, UNITY), 0);
        CHECK(nova_audio_write(&mine, pcm, 480) == 480, "a non-root user cannot play");
        unsigned int ms[10];
        EXPECT(nova_audio_mixer_stat(ms), 0);       /* statistics expose no audio */
        EXPECT(nova_audio_write(&F[0].s, pcm, 10), -EBADF); /* the parent's stream is not ours */
        EXPECT(nova_audio_set_volume(&F[0].s, 0), -EBADF);
        EXPECT(nova_audio_close(&F[0].s), -EBADF);
        EXPECT(nova_audio_close(&mine), 0);
        sys_exit(failures);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    int code = sys_wait(pid);
    CHECK(code == 0, "the other user's process reported %d failed checks", code);
    unsigned int ms[10];
    EXPECT(nova_audio_mixer_stat(ms), 0);
    CHECK(ms[NOVA_AUDIO_MS_MASTER] == UNITY && ms[NOVA_AUDIO_MS_MUTED] == 0, "a refused master change took effect (master %u muted %u)",
          ms[NOVA_AUDIO_MS_MASTER], ms[NOVA_AUDIO_MS_MUTED]);
    CHECK(wait_for_level(1000, 6), "the other user disturbed our stream");
    nova_audio_abort(&F[0].s);
    F[0].open = 0;
    if (failures == f0) printf("[audiotest] ok: privilege - another user can play but cannot set the master volume, read the tap, or touch our stream\n");
}

int main(void) {
    EXPECT(nova_shm_create(&mbr, 4096), 0);
    mb = (mailbox_t*)mbr.addr;

    nova_audio_stream_t probe;
    int rc = nova_audio_open(&probe, 2, 48000, UNITY);
    if (rc == -ENODEV) {
        printf("[audiotest] note: no sound hardware (AC97) - nothing to test\n");
        printf("[audiotest] FAIL: this test needs the AC97 device the test harness attaches\n");
        return 1;
    }
    if (rc == 0) {
        nova_audio_abort(&probe);
    }

    test_basics();
    test_mixing();
    test_chunked_write();
    test_formats();
    test_beep();
    test_lifecycle();
    test_privilege();

    nova_shm_detach(&mbr);
    sys_shm_destroy(mbr.handle);

    if (failures == 0) {
        printf("[audiotest] PASS: the full SYS_AUDIO_* contract holds (%d checks)\n", checks);
        return 0;
    }
    printf("[audiotest] FAIL: %d of %d checks failed\n", failures, checks);
    return 1;
}
