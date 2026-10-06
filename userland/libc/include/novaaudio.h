#ifndef NOVAAUDIO_H
#define NOVAAUDIO_H

/*
 * novaaudio.h - Phase 85: the app-side half of audio mixing. A thin,
 * header-only layer over the SYS_AUDIO_* syscalls (nova_audio_abi.h has the
 * model and the contract), plus a little tone generation.
 *
 * Typical use - play a sound while other apps play theirs:
 *
 *   nova_audio_stream_t s;
 *   nova_audio_open(&s, 2, 44100, NOVA_AUDIO_UNITY);       // stereo, 44.1kHz
 *   nova_audio_write_all(&s, pcm, frames, 10);             // waits while it is full
 *   nova_audio_close(&s);                                  // plays out what is buffered
 *
 * The mixer converts channels and rate to the card's 48kHz stereo and sums
 * every app's stream, so none of this cares who else is playing. Nothing
 * blocks in the kernel (a syscall runs with interrupts off and there is no
 * blocked-process state), so a full stream is -EAGAIN and write_all() waits
 * with SYS_YIELD; its timeout is in whole seconds because the only clock a
 * plain process has is the RTC.
 *
 * Every function returns 0 / a non-negative result (frames accepted), or a
 * NEGATIVE errno.
 *
 * Floating point is deliberately avoided: ring-3 code shares the FPU with
 * every other process and the kernel does not save FPU state on a task
 * switch. Tones come from an integer sine table.
 */

#include <errno.h>
#include <novasys.h>
#include <string.h>

/* The kernel cannot include errno.h, so nova_audio_abi.h repeats the numbers
 * it returns. If either side is ever edited out of step this stops the build
 * instead of letting a program mis-handle an error. */
typedef char novaaudio_errno_perm [(NOVA_AUDIO_ERR_PERM  == EPERM)  ? 1 : -1];
typedef char novaaudio_errno_badf [(NOVA_AUDIO_ERR_BADF  == EBADF)  ? 1 : -1];
typedef char novaaudio_errno_again[(NOVA_AUDIO_ERR_AGAIN == EAGAIN) ? 1 : -1];
typedef char novaaudio_errno_fault[(NOVA_AUDIO_ERR_FAULT == EFAULT) ? 1 : -1];
typedef char novaaudio_errno_nodev[(NOVA_AUDIO_ERR_NODEV == ENODEV) ? 1 : -1];
typedef char novaaudio_errno_inval[(NOVA_AUDIO_ERR_INVAL == EINVAL) ? 1 : -1];
typedef char novaaudio_errno_nospc[(NOVA_AUDIO_ERR_NOSPC == ENOSPC) ? 1 : -1];

typedef struct {
    unsigned int handle;
    unsigned int channels;
    unsigned int rate;
    unsigned int capacity_frames;
} nova_audio_stream_t;

/* ---- streams ---------------------------------------------------------- */

/* Opens a stream: `channels` 1 or 2, `rate` 8000..48000, `volume`
 * 0..NOVA_AUDIO_UNITY. */
static inline int nova_audio_open(nova_audio_stream_t* s, unsigned int channels, unsigned int rate, unsigned int volume) {
    nova_audio_open_t o;
    o.channels = channels;
    o.rate = rate;
    o.flags = 0;
    o.volume = volume;
    o.handle = 0;
    o.capacity_frames = 0;
    o.period_frames = 0;
    o.reserved = 0;
    int rc = sys_audio_open(&o);
    if (rc < 0) {
        return rc;
    }
    s->handle = o.handle;
    s->channels = channels;
    s->rate = rate;
    s->capacity_frames = o.capacity_frames;
    return 0;
}

/* One non-blocking write of whole interleaved s16 frames. Returns the number
 * of frames taken (possibly fewer than asked), or -EAGAIN if the stream's
 * buffer is full. */
static inline int nova_audio_write(const nova_audio_stream_t* s, const short* samples, unsigned int frames) {
    nova_audio_write_t w;
    w.handle = s->handle;
    w.data = (unsigned int)(unsigned long)samples;
    w.frames = frames;
    w.accepted = 0;
    int rc = sys_audio_write(&w);
    return rc < 0 ? rc : (int)w.accepted;
}

static inline int nova_audio_now_s(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    return t.hour * 3600 + t.minute * 60 + t.second;
}

/* Writes every frame, waiting (with SYS_YIELD) whenever the stream is full.
 * Returns 0, or a negative errno (-EAGAIN if `timeout_s` seconds pass with
 * the stream still full). */
static inline int nova_audio_write_all(const nova_audio_stream_t* s, const short* samples, unsigned int frames, unsigned int timeout_s) {
    unsigned int done = 0;
    int start = nova_audio_now_s();
    while (done < frames) {
        int n = nova_audio_write(s, samples + (unsigned long)done * s->channels, frames - done);
        if (n > 0) {
            done += (unsigned int)n;
            start = nova_audio_now_s(); /* progress resets the timeout */
            continue;
        }
        if (n != -EAGAIN && n != 0) {
            return n;
        }
        int e = nova_audio_now_s() - start;
        if (e < 0) {
            e += 86400;
        }
        if ((unsigned int)e > timeout_s) {
            return -EAGAIN;
        }
        sys_yield();
    }
    return 0;
}

/* Plays out what is buffered, then frees the stream. */
static inline int nova_audio_close(const nova_audio_stream_t* s) {
    nova_audio_close_t c;
    c.handle = s->handle;
    c.flags = 0;
    return sys_audio_close(&c);
}

/* Frees the stream at once; what is buffered is not played. */
static inline int nova_audio_abort(const nova_audio_stream_t* s) {
    nova_audio_close_t c;
    c.handle = s->handle;
    c.flags = NOVA_AUDIO_CLOSE_ABORT;
    return sys_audio_close(&c);
}

/* ---- controls ------------------------------------------------------------ */

static inline int nova_audio_ctl(unsigned int op, unsigned int handle, unsigned int arg, unsigned int arg2, nova_audio_ctl_t* out) {
    nova_audio_ctl_t c;
    memset(&c, 0, sizeof c);
    c.op = op;
    c.handle = handle;
    c.arg = arg;
    c.arg2 = arg2;
    int rc = sys_audio_ctl(&c);
    if (rc == 0 && out != 0) {
        *out = c;
    }
    return rc;
}

static inline int nova_audio_set_volume(const nova_audio_stream_t* s, unsigned int volume) {
    return nova_audio_ctl(NOVA_AUDIO_CTL_SET_VOLUME, s->handle, volume, 0, 0);
}
static inline int nova_audio_pause(const nova_audio_stream_t* s) {
    return nova_audio_ctl(NOVA_AUDIO_CTL_PAUSE, s->handle, 0, 0, 0);
}
static inline int nova_audio_resume(const nova_audio_stream_t* s) {
    return nova_audio_ctl(NOVA_AUDIO_CTL_RESUME, s->handle, 0, 0, 0);
}
static inline int nova_audio_flush(const nova_audio_stream_t* s) {
    return nova_audio_ctl(NOVA_AUDIO_CTL_FLUSH, s->handle, 0, 0, 0);
}

/* `out` receives NOVA_AUDIO_SS_* values. */
static inline int nova_audio_stream_stat(const nova_audio_stream_t* s, unsigned int out[10]) {
    nova_audio_ctl_t c;
    int rc = nova_audio_ctl(NOVA_AUDIO_CTL_STREAM_STAT, s->handle, 0, 0, &c);
    if (rc == 0) {
        for (int i = 0; i < 10; i++) {
            out[i] = c.out[i];
        }
    }
    return rc;
}

/* `out` receives NOVA_AUDIO_MS_* values. Any process may read these. */
static inline int nova_audio_mixer_stat(unsigned int out[10]) {
    nova_audio_ctl_t c;
    int rc = nova_audio_ctl(NOVA_AUDIO_CTL_MIXER_STAT, 0, 0, 0, &c);
    if (rc == 0) {
        for (int i = 0; i < 10; i++) {
            out[i] = c.out[i];
        }
    }
    return rc;
}

/* ROOT only: the master volume (0..NOVA_AUDIO_UNITY) and mute. */
static inline int nova_audio_set_master(unsigned int volume, int mute) {
    return nova_audio_ctl(NOVA_AUDIO_CTL_SET_MASTER, 0, volume, mute ? 1u : 0u, 0);
}

/* ROOT only: copies the newest `frames` (at most 1024) mixed stereo frames
 * into `buf` (2 * frames samples), oldest first. Returns the number copied;
 * `*total` (if given) receives the frames mixed since boot. */
static inline int nova_audio_tap(short* buf, unsigned int frames, unsigned int* total) {
    nova_audio_ctl_t c;
    memset(&c, 0, sizeof c);
    c.op = NOVA_AUDIO_CTL_TAP_READ;
    c.buf = (unsigned int)(unsigned long)buf;
    c.frames = frames;
    int rc = sys_audio_ctl(&c);
    if (rc < 0) {
        return rc;
    }
    if (total != 0) {
        *total = c.out[1];
    }
    return (int)c.out[0];
}

/* ---- tones (integer only) ---------------------------------------------------- */

/* One cycle of a sine at amplitude 32767, 256 entries. */
static const short nova_audio_sine_table[256] = {
    0, 804, 1608, 2410, 3212, 4011, 4808, 5602,
    6393, 7179, 7962, 8739, 9512, 10278, 11039, 11793,
    12539, 13279, 14010, 14732, 15446, 16151, 16846, 17530,
    18204, 18868, 19519, 20159, 20787, 21403, 22005, 22594,
    23170, 23731, 24279, 24811, 25329, 25832, 26319, 26790,
    27245, 27683, 28105, 28510, 28898, 29268, 29621, 29956,
    30273, 30571, 30852, 31113, 31356, 31580, 31785, 31971,
    32137, 32285, 32412, 32521, 32609, 32678, 32728, 32757,
    32767, 32757, 32728, 32678, 32609, 32521, 32412, 32285,
    32137, 31971, 31785, 31580, 31356, 31113, 30852, 30571,
    30273, 29956, 29621, 29268, 28898, 28510, 28105, 27683,
    27245, 26790, 26319, 25832, 25329, 24811, 24279, 23731,
    23170, 22594, 22005, 21403, 20787, 20159, 19519, 18868,
    18204, 17530, 16846, 16151, 15446, 14732, 14010, 13279,
    12539, 11793, 11039, 10278, 9512, 8739, 7962, 7179,
    6393, 5602, 4808, 4011, 3212, 2410, 1608, 804,
    0, -804, -1608, -2410, -3212, -4011, -4808, -5602,
    -6393, -7179, -7962, -8739, -9512, -10278, -11039, -11793,
    -12539, -13279, -14010, -14732, -15446, -16151, -16846, -17530,
    -18204, -18868, -19519, -20159, -20787, -21403, -22005, -22594,
    -23170, -23731, -24279, -24811, -25329, -25832, -26319, -26790,
    -27245, -27683, -28105, -28510, -28898, -29268, -29621, -29956,
    -30273, -30571, -30852, -31113, -31356, -31580, -31785, -31971,
    -32137, -32285, -32412, -32521, -32609, -32678, -32728, -32757,
    -32767, -32757, -32728, -32678, -32609, -32521, -32412, -32285,
    -32137, -31971, -31785, -31580, -31356, -31113, -30852, -30571,
    -30273, -29956, -29621, -29268, -28898, -28510, -28105, -27683,
    -27245, -26790, -26319, -25832, -25329, -24811, -24279, -23731,
    -23170, -22594, -22005, -21403, -20787, -20159, -19519, -18868,
    -18204, -17530, -16846, -16151, -15446, -14732, -14010, -13279,
    -12539, -11793, -11039, -10278, -9512, -8739, -7962, -7179,
    -6393, -5602, -4808, -4011, -3212, -2410, -1608, -804,
};

/* The sine of a 32-bit phase (2^32 = one cycle), amplitude 32767. */
static inline short nova_audio_sine(unsigned int phase) {
    return nova_audio_sine_table[phase >> 24];
}

/* Fills `frames` frames of a `hz` tone at `rate` and amplitude `amp`
 * (0..32767) into `buf` (`channels` per frame), continuing from *phase so
 * successive calls join without a click. */
static inline void nova_audio_fill_tone(short* buf, unsigned int frames, unsigned int channels, unsigned int rate,
                                        unsigned int hz, int amp, unsigned int* phase) {
    unsigned int step = (0xFFFFFFFFu / rate) * hz;
    unsigned int p = *phase;
    for (unsigned int i = 0; i < frames; i++) {
        short v = (short)(((int)nova_audio_sine(p) * amp) >> 15);
        p += step;
        for (unsigned int c = 0; c < channels; c++) {
            buf[(unsigned long)i * channels + c] = v;
        }
    }
    *phase = p;
}

#endif
