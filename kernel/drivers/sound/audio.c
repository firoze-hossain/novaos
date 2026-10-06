/*
 * kernel/drivers/sound/audio.c - Phase 85: the C half of audio mixing. See
 * audio.h, kernel/rust/mixer.rs (the real subsystem) and
 * userland/libc/include/nova_audio_abi.h (the contract).
 *
 * Two jobs only:
 *   1. the syscall entry points: validate every user pointer BEFORE
 *      touching it (a fault in kernel mode is a panic here), copy the
 *      argument struct and sample data in, call Rust, copy results out;
 *   2. the 10ms tick that lets the Rust pump feed the sound card.
 */
#include "audio.h"
#include "ac97.h"
#include "../timer/timer.h"
#include "../../include/kernel.h"
#include "../../lib/string.h"
#include "../../arch/x86/mm/paging.h"
#include "../../task/process.h"

/* The syscall numbers are defined once, in the shared ABI header, and
 * repeated in kernel/arch/x86/cpu/syscall.h; this stops the two copies from
 * drifting apart (the array has a negative size if they differ). */
#include "../../arch/x86/cpu/syscall.h"
typedef char audio_sysno_open[(SYS_AUDIO_OPEN == NOVA_SYS_AUDIO_OPEN) ? 1 : -1];
typedef char audio_sysno_write[(SYS_AUDIO_WRITE == NOVA_SYS_AUDIO_WRITE) ? 1 : -1];
typedef char audio_sysno_ctl[(SYS_AUDIO_CTL == NOVA_SYS_AUDIO_CTL) ? 1 : -1];
typedef char audio_sysno_close[(SYS_AUDIO_CLOSE == NOVA_SYS_AUDIO_CLOSE) ? 1 : -1];

/* A write is copied into the kernel in chunks of this many SAMPLES, so the
 * kernel stack holds 2KB of audio at a time, not a whole user buffer. */
#define WRITE_CHUNK_SAMPLES 1024

/* The most frames one TAP_READ returns (4KB on the kernel stack). */
#define TAP_READ_MAX_FRAMES 1024

int audio_sys_open(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_audio_open_t), true)) {
        return -NOVA_AUDIO_ERR_FAULT;
    }
    nova_audio_open_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    uint32_t handle = 0, capacity = 0;
    int rc = rust_audio_open(pid, req.channels, req.rate, req.flags,
                             req.volume, &handle, &capacity);
    if (rc != 0) {
        return rc;
    }
    req.handle = handle;
    req.capacity_frames = capacity;
    req.period_frames = NOVA_AUDIO_PERIOD_FRAMES;
    req.reserved = 0;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

int audio_sys_write(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_audio_write_t), true)) {
        return -NOVA_AUDIO_ERR_FAULT;
    }
    nova_audio_write_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    req.accepted = 0;

    /* How big is a frame? Only the stream knows (mono or stereo), and a
     * handle that is not the caller's fails here, before any pointer in the
     * request is trusted. */
    uint32_t stat[10] = {0};
    int rc = rust_audio_ctl(pid, 0, NOVA_AUDIO_CTL_STREAM_STAT, req.handle, 0,
                            0, stat);
    if (rc != 0) {
        return rc;
    }
    uint32_t ch = stat[NOVA_AUDIO_SS_CHANNELS];
    uint32_t capacity = stat[NOVA_AUDIO_SS_CAPACITY];
    if (ch != 1 && ch != 2) {
        return -NOVA_AUDIO_ERR_INVAL;
    }
    /* A stream cannot take more than its buffer holds, so asking for more is
     * a short write, not an error - and this bounds the range checked below. */
    uint32_t frames = req.frames > capacity ? capacity : req.frames;
    if (frames == 0) {
        memcpy((void*)user_ptr, &req, sizeof req);
        return 0;
    }
    if (!paging_user_range_ok(req.data, frames * ch * 2, false)) {
        return -NOVA_AUDIO_ERR_FAULT;
    }

    int16_t chunk[WRITE_CHUNK_SAMPLES];
    uint32_t chunk_frames = WRITE_CHUNK_SAMPLES / ch;
    uint32_t done = 0;
    int last = 0;
    while (done < frames) {
        uint32_t n = frames - done;
        if (n > chunk_frames) {
            n = chunk_frames;
        }
        memcpy(chunk, (const void*)(req.data + done * ch * 2), n * ch * 2);
        last = rust_audio_write(pid, req.handle, chunk, n * ch);
        if (last < 0) {
            break;
        }
        done += (uint32_t)last;
        if ((uint32_t)last < n) {
            break; /* the stream's buffer is full */
        }
    }
    req.accepted = done;
    memcpy((void*)user_ptr, &req, sizeof req);
    if (done == 0 && last < 0) {
        return last; /* -EAGAIN when full, -EBADF for a bad handle... */
    }
    return 0;
}

int audio_sys_ctl(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_audio_ctl_t), true)) {
        return -NOVA_AUDIO_ERR_FAULT;
    }
    nova_audio_ctl_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);

    /* The caller's uid is the kernel's own record, never a request field. */
    uint32_t uid = 0;
    if (!process_uid_of(pid, &uid)) {
        return -NOVA_AUDIO_ERR_BADF;
    }

    if (req.op == NOVA_AUDIO_CTL_TAP_READ) {
        uint32_t frames = req.frames > TAP_READ_MAX_FRAMES ? TAP_READ_MAX_FRAMES
                                                           : req.frames;
        /* Privilege first, so an unprivileged caller learns nothing - not even
         * whether its pointer was good. */
        if (uid != 0) {
            return -NOVA_AUDIO_ERR_PERM;
        }
        if (frames > 0 && !paging_user_range_ok(req.buf, frames * 4, true)) {
            return -NOVA_AUDIO_ERR_FAULT;
        }
        int16_t buf[TAP_READ_MAX_FRAMES * 2];
        uint32_t out2[2] = {0, 0};
        int rc = rust_audio_tap(uid, buf, frames, out2);
        if (rc != 0) {
            return rc;
        }
        if (out2[0] > 0) {
            memcpy((void*)req.buf, buf, out2[0] * 4);
        }
        req.out[0] = out2[0];
        req.out[1] = out2[1];
        memcpy((void*)user_ptr, &req, sizeof req);
        return 0;
    }

    uint32_t out[10] = {0};
    int rc = rust_audio_ctl(pid, uid, req.op, req.handle, req.arg, req.arg2, out);
    if (rc != 0) {
        return rc;
    }
    if (req.op == NOVA_AUDIO_CTL_STREAM_STAT || req.op == NOVA_AUDIO_CTL_MIXER_STAT) {
        for (int i = 0; i < 10; i++) {
            req.out[i] = out[i];
        }
        memcpy((void*)user_ptr, &req, sizeof req);
    }
    return 0;
}

int audio_sys_close(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_audio_close_t), false)) {
        return -NOVA_AUDIO_ERR_FAULT;
    }
    nova_audio_close_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    return rust_audio_close(pid, req.handle, req.flags);
}

void audio_tick(void) {
    rust_audio_tick();
}

void audio_init(void) {
    uint32_t state[3] = {0, 0, 0};
    uint32_t bad = rust_audio_selftest(state);
    bool ok = bad == 0 && state[0] == 0 && state[1] == 0 &&
              state[2] == NOVA_AUDIO_UNITY;
    bool hooked = timer_add_tick_listener(audio_tick);
    kernel_log("[ %s ] Audio mixer: %d streams (%d per process), %d-%dHz "
               "mono/stereo s16, mixed to %dHz stereo in %dms periods, "
               "hardware %s%s\n",
               (ok && hooked) ? "OK" : "FAIL", (int)NOVA_AUDIO_MAX_STREAMS,
               (int)NOVA_AUDIO_MAX_STREAMS_PER_PROC,
               (int)NOVA_AUDIO_MIN_RATE, (int)NOVA_AUDIO_MAX_RATE,
               (int)NOVA_AUDIO_SAMPLE_RATE,
               (int)(NOVA_AUDIO_PERIOD_FRAMES * 1000 / NOVA_AUDIO_SAMPLE_RATE),
               ac97_is_present() ? "present" : "absent (streams cannot be opened)",
               hooked ? "" : " - TICK LISTENER TABLE FULL");
}
