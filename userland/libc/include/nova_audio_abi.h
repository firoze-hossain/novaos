#ifndef NOVA_AUDIO_ABI_H
#define NOVA_AUDIO_ABI_H

/*
 * nova_audio_abi.h - Phase 85: the ABI of NovaOS's audio-mixing syscalls
 * (SYS_AUDIO_*), shared verbatim by the kernel (kernel/drivers/sound/
 * audio.c, kernel/rust/mixer.rs's tests) and userland (novaaudio.h,
 * novasys.h): ONE definition rather than two copies that can drift.
 * Dependency-free: plain `unsigned int` / `int` (32 bits on this i686
 * target - checked by the size assertions at the bottom). A host test in
 * kernel/rust/mixer.rs reads THIS FILE and fails if any limit or code below
 * disagrees with the kernel's own constant.
 *
 * WHY THIS EXISTS
 *
 * Before this phase there was no audio API at all: no app could open a
 * stream or hand the kernel a single sample. The only thing was SYS_BEEP,
 * which reset the sound card's DMA engine and played one hard-coded 0.3
 * second square wave from a static buffer - so a second sound cut the first
 * off, and "the sound device" was effectively owned by whoever called last.
 * The card itself has ONE PCM output. To let several apps play at once the
 * kernel has to sit in the middle: each app gets its own STREAM, a MIXER
 * sums them, and the card is fed a single continuous stream of the sum.
 *
 * THE MODEL
 *
 *   STREAM   AUDIO_OPEN gives an app a stream: mono or stereo, signed 16-bit
 *            little-endian PCM, any rate from 8000 to 48000Hz. The mixer
 *            converts channels and rate (linear interpolation) so apps need
 *            not care that the hardware runs at 48000Hz stereo.
 *   WRITE    AUDIO_WRITE copies whole frames into the stream's buffer and
 *            returns how many were taken. It never blocks: a full buffer is
 *            -EAGAIN (or a short write) and the app waits with SYS_YIELD, as
 *            everywhere in this kernel (a syscall runs with interrupts off
 *            and there is no blocked-process state). novaaudio.h wraps this
 *            into write_all() with a timeout and drain().
 *   MIX      every 10ms the kernel mixes one 480-frame period: each stream's
 *            next frames, scaled by that stream's volume, summed, scaled by
 *            the master volume, and saturated to 16 bits. A stream that has
 *            run dry contributes silence and is counted as an UNDERRUN; it
 *            never stalls the others.
 *   CLOSE    AUDIO_CLOSE plays what is already buffered and then frees the
 *            stream (so an app can write its last second of sound and close
 *            immediately); with NOVA_AUDIO_CLOSE_ABORT it frees at once.
 *            When a process EXITS, its streams are aborted: it cannot stay
 *            behind to be heard, so drain (wait for queued_frames == 0)
 *            before exiting if the end of the sound matters.
 *   BEEP     SYS_BEEP no longer takes over the card: it adds a built-in
 *            tone voice to the mixer, so a beep mixes with whatever else is
 *            playing instead of cutting it off.
 *
 * PRIVILEGE. A stream belongs to the process that opened it: only that
 * process can write to, control or close it. Two controls affect or expose
 * EVERYONE's sound and are therefore root-only (-EPERM otherwise): the
 * MASTER volume/mute, and the TAP - a copy of the most recent mixed output,
 * which is what a recorder, a visualizer or a test would read. A volume
 * control for ordinary users belongs in a root service that apps reach with
 * messaging (nova_msg_abi.h).
 *
 * Every call returns 0 or a non-negative result on success and a NEGATIVE
 * errno on failure (not -1 plus errno).
 */

/* Syscall numbers (kernel/arch/x86/cpu/syscall.h). */
#define NOVA_SYS_AUDIO_OPEN   66 /* EBX = nova_audio_open_t* (in/out) */
#define NOVA_SYS_AUDIO_WRITE  67 /* EBX = nova_audio_write_t* (in/out) */
#define NOVA_SYS_AUDIO_CTL    68 /* EBX = nova_audio_ctl_t* (in/out) */
#define NOVA_SYS_AUDIO_CLOSE  69 /* EBX = nova_audio_close_t* */

/* The hardware format the mixer produces (the AC97 default). */
#define NOVA_AUDIO_SAMPLE_RATE   48000u
#define NOVA_AUDIO_PERIOD_FRAMES 480u    /* 10ms: one mix, one DMA buffer */
#define NOVA_AUDIO_MIN_RATE      8000u
#define NOVA_AUDIO_MAX_RATE      48000u

/* Limits. */
#define NOVA_AUDIO_MAX_STREAMS          8u
#define NOVA_AUDIO_MAX_STREAMS_PER_PROC 4u
#define NOVA_AUDIO_STREAM_RING_SAMPLES  16384u /* per stream: 8192 stereo or 16384 mono frames */
#define NOVA_AUDIO_TAP_FRAMES           4096u  /* ~85ms of recent mixed output */
#define NOVA_AUDIO_UNITY                256u   /* volume 256 = 100%; the maximum */

/* AUDIO_CLOSE flags. */
#define NOVA_AUDIO_CLOSE_ABORT  1u /* free now; do not play out what is buffered */

/* AUDIO_CTL operations. */
#define NOVA_AUDIO_CTL_SET_VOLUME  1u /* handle, arg = 0..UNITY: this stream's volume */
#define NOVA_AUDIO_CTL_PAUSE       2u /* handle */
#define NOVA_AUDIO_CTL_RESUME      3u /* handle */
#define NOVA_AUDIO_CTL_FLUSH       4u /* handle: discard buffered frames */
#define NOVA_AUDIO_CTL_STREAM_STAT 5u /* handle -> out[] (below) */
#define NOVA_AUDIO_CTL_MIXER_STAT  6u /* any process -> out[] (below) */
#define NOVA_AUDIO_CTL_SET_MASTER  7u /* ROOT: arg = 0..UNITY volume, arg2 = 1 to mute */
#define NOVA_AUDIO_CTL_TAP_READ    8u /* ROOT: buf, frames -> the newest `frames` mixed frames */

/* STREAM_STAT out[]: */
#define NOVA_AUDIO_SS_QUEUED     0 /* frames buffered, not yet mixed */
#define NOVA_AUDIO_SS_CAPACITY   1 /* the buffer's size in frames */
#define NOVA_AUDIO_SS_PLAYED     2 /* frames this stream has contributed to the mix */
#define NOVA_AUDIO_SS_WRITTEN    3 /* frames accepted by AUDIO_WRITE */
#define NOVA_AUDIO_SS_UNDERRUN_EVENTS 4 /* times the stream ran dry after starting */
#define NOVA_AUDIO_SS_UNDERRUN_FRAMES 5 /* output frames it was missing */
#define NOVA_AUDIO_SS_VOLUME     6
#define NOVA_AUDIO_SS_FLAGS      7 /* bit0 paused, bit1 closing (draining) */
#define NOVA_AUDIO_SS_RATE       8
#define NOVA_AUDIO_SS_CHANNELS   9

/* MIXER_STAT out[]: */
#define NOVA_AUDIO_MS_ACTIVE     0 /* open streams */
#define NOVA_AUDIO_MS_HW_RUNNING 1 /* the card's DMA engine is running */
#define NOVA_AUDIO_MS_MIXED      2 /* frames mixed since boot (wraps) */
#define NOVA_AUDIO_MS_CLIPPED    3 /* samples that had to be saturated */
#define NOVA_AUDIO_MS_HW_UNDERRUNS 4 /* times the card ran out of mixed audio */
#define NOVA_AUDIO_MS_RESTARTS   5 /* times the DMA engine was (re)started */
#define NOVA_AUDIO_MS_MASTER     6
#define NOVA_AUDIO_MS_MUTED      7
#define NOVA_AUDIO_MS_QUEUED_PERIODS 8 /* mixed periods ahead of the card */
#define NOVA_AUDIO_MS_VOICES     9 /* built-in tone voices (beeps) playing */

/* AUDIO_OPEN: in: channels (1 or 2), rate (8000..48000), flags (must be
 * 0), volume (0..UNITY). out: handle, capacity_frames, period_frames. */
typedef struct {
    unsigned int channels;
    unsigned int rate;
    unsigned int flags;
    unsigned int volume;
    unsigned int handle;
    unsigned int capacity_frames;
    unsigned int period_frames;
    unsigned int reserved;
} nova_audio_open_t;

/* AUDIO_WRITE: in: handle, data (user pointer to `frames` frames of
 * interleaved s16le), frames. out: accepted (frames taken; may be fewer
 * than asked). -EAGAIN if nothing could be taken. */
typedef struct {
    unsigned int handle;
    unsigned int data;
    unsigned int frames;
    unsigned int accepted;
} nova_audio_write_t;

typedef struct {
    unsigned int handle;
    unsigned int flags;
} nova_audio_close_t;

typedef struct {
    unsigned int op;
    unsigned int handle;
    unsigned int arg;
    unsigned int arg2;
    unsigned int buf;
    unsigned int frames;
    unsigned int out[10];
} nova_audio_ctl_t;

/* Error numbers. The kernel cannot include <errno.h>; these repeat the
 * errno values it returns, and novaaudio.h asserts at compile time that
 * they still match userland's errno.h. */
#define NOVA_AUDIO_ERR_PERM    1  /* root-only control */
#define NOVA_AUDIO_ERR_BADF    9  /* not your stream (or already closed) */
#define NOVA_AUDIO_ERR_AGAIN  11  /* the stream's buffer is full */
#define NOVA_AUDIO_ERR_FAULT  14  /* bad pointer */
#define NOVA_AUDIO_ERR_NODEV  19  /* no sound hardware */
#define NOVA_AUDIO_ERR_INVAL  22  /* bad argument */
#define NOVA_AUDIO_ERR_NOSPC  28  /* a stream limit */

/* Compile-time ABI checks (the array-size trick; works in C89). */
typedef char nova_audio_abi_check_open[(sizeof(nova_audio_open_t) == 32) ? 1 : -1];
typedef char nova_audio_abi_check_write[(sizeof(nova_audio_write_t) == 16) ? 1 : -1];
typedef char nova_audio_abi_check_close[(sizeof(nova_audio_close_t) == 8) ? 1 : -1];
typedef char nova_audio_abi_check_ctl[(sizeof(nova_audio_ctl_t) == 64) ? 1 : -1];

#endif
