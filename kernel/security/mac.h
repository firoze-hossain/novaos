#ifndef KERNEL_SECURITY_MAC_H
#define KERNEL_SECURITY_MAC_H

/*
 * Phase 87: mandatory access control - the kernel side. The policy engine is
 * kernel/rust/mac.rs; this is the C that applies it at the places the existing
 * capability checks already live. The contract is userland/libc/include/
 * nova_mac_abi.h.
 *
 * Two kinds of check, because they have different hazards:
 *   - the SYSCALL gate sees only the syscall number, no user pointer, so it
 *     can sit at the very top of the dispatcher;
 *   - everything that depends on an ARGUMENT (a file name, a peer, a program to
 *     start) is decided on a validated KERNEL COPY of that argument and the
 *     handler then USES that copy. Checking the user's string and re-reading it
 *     is a time-of-check/time-of-use hole as soon as memory can be shared
 *     (Phase 83), and the existing capability checks had it.
 */

#include "../include/types.h"

#define MAC_MAX_STACK 4
#define MAC_NAME_MAX  64

#define MAC_FILE_READ   1
#define MAC_FILE_WRITE  2
#define MAC_FILE_DELETE 4
#define MAC_NET_CONNECT 1
#define MAC_NET_BIND    2
#define MAC_NET_SEND    4

struct process;

typedef struct {
    uint16_t stack[MAC_MAX_STACK];
    uint32_t n;
} mac_stack_t;

void mac_init(void);

/* The syscall gate: true if the calling process may NOT make this syscall. */
bool mac_gate_syscall(uint32_t sysno);

/* Copies a NUL-terminated name from `src` into `dst` (at most cap-1 bytes plus
 * the NUL). A user process's pointer is validated byte by byte and never
 * faults; a kernel task's is trusted. False if it was bad or too long. */
bool mac_copy_name(const struct process* p, char* dst, uint32_t cap, const char* src);

/* May `p` do this to `kname`? `kname` MUST be a kernel copy. True if it has no
 * profile. Denials are counted and logged here. */
bool mac_file_allowed(struct process* p, const char* kname, uint32_t op);
bool mac_net_allowed(struct process* p, uint32_t op, uint32_t ip, uint16_t port);

/* Writing or deleting a *.MAC file: only an unconfined root, and never once the
 * policy is frozen. True for any other name. */
bool mac_policy_write_allowed(struct process* p, const char* kname);

/* Everything process_exec_internal() must decide BEFORE it claims a slot:
 * copy the program's path, check the spawner may start it, load the program's
 * own profile (NAME.MAC) if it has one, and work out the child's stack.
 * False means: do not start it (and nothing has been claimed). */
bool mac_prepare_exec(struct process* spawner, const char* path_in, char* path_out,
                      uint32_t cap, mac_stack_t* out);
void mac_apply_stack(struct process* child, const mac_stack_t* st);
void mac_inherit_fork(struct process* child, const struct process* parent);
void mac_reset(struct process* p);

int mac_sys_info(uint32_t user_ptr);
int mac_sys_ctl(uint32_t op, uint32_t arg);

#endif
