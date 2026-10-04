#ifndef DRIVERS_VIDEO_FB_H
#define DRIVERS_VIDEO_FB_H

#include "../../include/types.h"
#include "../../../userland/libc/include/nova_fb_abi.h"

/*
 * fb.h - Phase 81: the kernel side of the SYS_FB_* framebuffer API.
 * userland/libc/include/nova_fb_abi.h is the contract (and explains
 * the model and why it can hand off to a GPU later); this file is the
 * implementation's interface to the rest of the kernel.
 *
 * Layering:
 *
 *     ring 3 app  --  int 0x80  -->  syscall.c  (thin: gets pid, page
 *                                                 directory, a pointer)
 *                                        |
 *                                      fb.c   (ownership, surfaces,
 *                                        |     validation, clipping)
 *                          +-------------+-------------+
 *                       VBE backend             virtio-gpu backend
 *                       (vbe.c: CPU copy        (virtiogpu.c: copy into
 *                        into the linear         the resource's backing
 *                        framebuffer)            store + TRANSFER_TO_
 *                                                HOST_2D + RESOURCE_FLUSH
 *                                                of just the damage)
 *
 * A third backend (a 3D-capable GPU path) is one more entry in fb.c's
 * backend table: the only operations a backend implements are
 * "probe", "take over the screen", "give it back", "show these
 * pixels" and "tell me what is showing". Nothing above that line -
 * syscall.c, the ABI, every app - knows or cares which backend is
 * active.
 *
 * Every function taking a `user_ptr` validates it (paging.h's
 * paging_user_range_ok()) before touching it and returns -EFAULT
 * (negated NOVA_FB_ERR_FAULT) for a bad one; none of them can fault
 * the kernel on a hostile argument.
 */

/* Probes the backends and logs what is available. Call once at boot,
 * after the drivers it depends on (VBE, virtio-gpu) have initialized.
 * Safe to skip: with no backend every call returns -ENODEV. */
void fb_init(void);

int fb_sys_info(uint32_t user_ptr);
int fb_sys_acquire(int pid, uint32_t backend);
int fb_sys_release(int pid);
int fb_sys_create(int pid, uint32_t* page_directory, uint32_t user_ptr);
int fb_sys_destroy(int pid, uint32_t* page_directory, uint32_t handle);
int fb_sys_present(int pid, uint32_t user_ptr);
int fb_sys_readback(int pid, uint32_t user_ptr);

/* Called when a process exits (kernel/task/process.c's
 * process_exit_current()): forgets its surfaces and, if it owns the
 * display, releases it - so an app that exits (or whose process is
 * otherwise gone) without calling SYS_FB_RELEASE can never leave the
 * machine stuck in graphics mode with no one able to ask for the text
 * console back. Does NOT free surface memory: that lives in the
 * process's own page tables and is reclaimed by the address-space
 * teardown that already frees every other page it owns. */
void fb_process_exit(int pid);

/* True while any process owns the display through this API. The old
 * SYS_GFX_* handlers (syscall.c) consult it and do nothing in that
 * case, so a legacy program cannot scribble over, or switch away
 * from, a new-API client's screen. */
bool fb_display_is_owned(void);

#endif
