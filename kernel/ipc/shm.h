#ifndef KERNEL_IPC_SHM_H
#define KERNEL_IPC_SHM_H

#include "../include/types.h"
#include "../../userland/libc/include/nova_shm_abi.h"

/*
 * Phase 83: shared-memory IPC. The policy - the object table, the ACL,
 * reference counts, placement, every lifecycle rule - is in Rust
 * (kernel/rust/shm.rs, with the model of how it works and its tests).
 * This layer is the part that has to be C because it touches the
 * kernel's own C structures: it validates user pointers, copies the
 * argument structs in and out, and implements the small hardware-
 * abstraction functions (shm_hal_*) the Rust code calls to allocate
 * frames and edit page tables.
 *
 * The ABI (structs, limits, errno values) is
 * userland/libc/include/nova_shm_abi.h, shared with userland.
 *
 * Every shm_sys_* returns 0 / a non-negative result, or a negative errno.
 */

int shm_sys_create(int pid, uint32_t user_ptr);
int shm_sys_grant(int pid, uint32_t user_ptr);
int shm_sys_map(int pid, uint32_t user_ptr);
int shm_sys_unmap(int pid, uint32_t addr);
int shm_sys_destroy(int pid, uint32_t handle);
int shm_sys_info(int pid, uint32_t user_ptr);

/* Boot-time: checks the (empty) subsystem's books and logs the limits. */
void shm_init(void);

/* The Rust side (kernel/rust/shm.rs). */
int rust_shm_create(int pid, uint32_t size, uint32_t flags,
                    uint32_t* out_handle, uint32_t* out_size);
int rust_shm_grant(int pid, uint32_t handle, int target, uint32_t rights);
int rust_shm_map(int pid, uint32_t handle, uint32_t rights,
                 uint32_t* out_addr, uint32_t* out_size);
int rust_shm_unmap(int pid, uint32_t addr);
int rust_shm_destroy(int pid, uint32_t handle);
int rust_shm_info(int pid, uint32_t handle, uint32_t* out4);
void rust_shm_process_exit(int pid);
int rust_shm_fork(int parent, int child);
uint32_t rust_shm_selftest(uint32_t* objects, uint32_t* pages, uint32_t* maps);

#endif
