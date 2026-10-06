#ifndef DRIVERS_SOUND_AUDIO_H
#define DRIVERS_SOUND_AUDIO_H

#include "../../include/types.h"
#include "../../../userland/libc/include/nova_audio_abi.h"

/*
 * Phase 85: audio mixing. The policy - streams, the resampler, the mixer,
 * volumes, the tap, the beep voice, and the management of the sound card's
 * DMA ring - is in Rust (kernel/rust/mixer.rs, with the model of how it
 * works and its tests). This layer is the part that has to be C because it
 * touches the kernel's own C structures: it validates user pointers, copies
 * argument structs and sample data in and out, and drives the sound card's
 * ring through the small ac97_ring_* functions (kernel/drivers/sound/ac97.c).
 *
 * The ABI (structs, limits, errno values) is
 * userland/libc/include/nova_audio_abi.h, shared with userland.
 *
 * Every audio_sys_* returns 0 / a non-negative result, or a negative errno.
 */

int audio_sys_open(int pid, uint32_t user_ptr);
int audio_sys_write(int pid, uint32_t user_ptr);
int audio_sys_ctl(int pid, uint32_t user_ptr);
int audio_sys_close(int pid, uint32_t user_ptr);

/* Boot-time: checks the mixer's books and registers the 10ms tick that feeds
 * the sound card. */
void audio_init(void);

/* The 10ms timer-tick listener: mixes and refills the DMA ring. */
void audio_tick(void);

/* The Rust side (kernel/rust/mixer.rs). */
int rust_audio_open(int pid, uint32_t channels, uint32_t rate, uint32_t flags,
                    uint32_t volume, uint32_t* out_handle,
                    uint32_t* out_capacity);
int rust_audio_write(int pid, uint32_t handle, const int16_t* samples,
                     uint32_t nsamples);
int rust_audio_close(int pid, uint32_t handle, uint32_t flags);
int rust_audio_ctl(int pid, uint32_t uid, uint32_t op, uint32_t handle,
                   uint32_t arg, uint32_t arg2, uint32_t* out10);
int rust_audio_tap(uint32_t uid, int16_t* buf, uint32_t frames, uint32_t* out2);
int rust_audio_beep(void);
void rust_audio_tick(void);
void rust_audio_process_exit(int pid);
uint32_t rust_audio_selftest(uint32_t* out3);

#endif
