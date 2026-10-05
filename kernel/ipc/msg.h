#ifndef KERNEL_IPC_MSG_H
#define KERNEL_IPC_MSG_H

#include "../include/types.h"
#include "../../userland/libc/include/nova_msg_abi.h"

/*
 * Phase 84: app-to-app messaging. The policy - inboxes, framing, service
 * names, selective receive, per-sender fairness, accept policies - is in
 * Rust (kernel/rust/msg.rs, with the model of how it works and its tests).
 * This layer is the part that has to be C because it touches the kernel's
 * own C structures: it validates user pointers, copies the argument
 * structs and payloads in and out, and implements the small hardware-
 * abstraction functions (msg_hal_*) the Rust code calls.
 *
 * The ABI (structs, limits, errno values) is
 * userland/libc/include/nova_msg_abi.h, shared with userland.
 *
 * Every msg_sys_* returns 0 / a non-negative result, or a negative errno.
 */

int msg_sys_open(int pid, uint32_t user_ptr);
int msg_sys_close(int pid, uint32_t unused);
int msg_sys_send(int pid, uint32_t user_ptr);
int msg_sys_recv(int pid, uint32_t user_ptr);
int msg_sys_service(int pid, uint32_t user_ptr);
int msg_sys_ctl(int pid, uint32_t user_ptr);

/* Boot-time: checks the (empty) subsystem's books and logs the limits. */
void msg_init(void);

/* The Rust side (kernel/rust/msg.rs). */
int rust_msg_open(int pid, uint32_t policy, uint32_t flags,
                  uint32_t* out_depth, uint32_t* out_max);
int rust_msg_close(int pid);
int rust_msg_send(int from, uint32_t from_uid, int dest_pid,
                  const uint8_t* name, uint32_t name_len, uint32_t type,
                  uint32_t tag, uint32_t flags, const uint8_t* data,
                  uint32_t len);
int rust_msg_recv(int pid, uint32_t flags, int match_sender,
                  uint32_t match_type, uint32_t match_tag, uint8_t* buf,
                  uint32_t cap, uint32_t* info8);
int rust_msg_service(int pid, uint32_t op, const uint8_t* name,
                     uint32_t name_len, int* out_pid);
int rust_msg_ctl(int pid, uint32_t op, uint32_t arg, uint32_t* out12);
void rust_msg_process_exit(int pid);
uint32_t rust_msg_selftest(uint32_t* out3);

#endif
