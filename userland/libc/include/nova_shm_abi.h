#ifndef NOVA_SHM_ABI_H
#define NOVA_SHM_ABI_H

/*
 * nova_shm_abi.h - Phase 83: the ABI of NovaOS's shared-memory IPC
 * syscalls (SYS_SHM_*), shared verbatim by the kernel (kernel/ipc/shm.c,
 * kernel/rust/shm.rs's tests) and userland (novashm.h, novasys.h): ONE
 * definition rather than two copies that can drift. Dependency-free:
 * plain `unsigned int` / `int` (32 bits on this i686 target - checked by
 * the size assertions at the bottom). A host test in kernel/rust/shm.rs
 * reads THIS FILE and fails if any limit below disagrees with the kernel's
 * own constant.
 *
 * WHY THIS EXISTS
 *
 * A compositor and an app that hand pixels over through a syscall copy
 * every frame (app -> kernel -> compositor) and pay a system call per
 * frame. Shared memory maps the SAME physical frames into both address
 * spaces: the app draws straight into them, the compositor reads straight
 * out of them, and the kernel is involved only in setup.
 *
 * THE MODEL
 *
 *   1. SHM_CREATE(size)  -> a handle to a new zero-filled object. The
 *      creator is its OWNER.
 *   2. SHM_GRANT(handle, pid, rights)  -> the owner lets one specific
 *      process map it, read-only or read-write. A handle is just a small
 *      integer and is NOT authority: a process that learns it still gets
 *      -EACCES unless the owner granted it.
 *   3. SHM_MAP(handle, rights)  -> the kernel picks an address (in the
 *      region NOVA_SHM_REGION_BASE..END, with an unmapped guard page after
 *      each mapping) and returns it. Both processes now see the same
 *      memory. A read-only mapping is read-only to the CPU AND to the
 *      kernel: a syscall asked to write into it fails with -EFAULT.
 *   4. SHM_UNMAP(addr) removes one mapping. SHM_DESTROY(handle) (owner
 *      only) stops new mappings and frees the memory once the last
 *      mapping is gone.
 *
 * LIFETIME. The memory lives while anything can still reach it. A process
 * that exits releases all its mappings; the objects it OWNED are destroyed
 * (existing mappers keep theirs until they unmap or exit), so a crashed app
 * never strands memory. A fork() child inherits its parent's mappings and
 * they stay genuinely shared (they are NOT copy-on-write): writes by either
 * side are visible to the other, which is what a compositor/app pair that
 * forks wants and what distinguishes this from ordinary memory.
 *
 * SYNCHRONISATION is deliberately not a syscall. Both sides see the same
 * bytes, so they coordinate through the memory itself with atomic
 * operations (x86 is strongly ordered; use real atomics so the COMPILER
 * does not reorder either). novashm.h provides the standard answer for
 * frames: a lock-free triple buffer, in which the producer never waits for
 * the consumer and the consumer always sees a complete, most-recent frame.
 *
 * Every call returns 0 or a non-negative result on success and a NEGATIVE
 * errno on failure (not -1 plus errno).
 */

/* Syscall numbers (kernel/arch/x86/cpu/syscall.h). */
#define NOVA_SYS_SHM_CREATE   54 /* EBX = nova_shm_create_t* (in/out) */
#define NOVA_SYS_SHM_GRANT    55 /* EBX = nova_shm_grant_t* */
#define NOVA_SYS_SHM_MAP      56 /* EBX = nova_shm_map_t* (in/out) */
#define NOVA_SYS_SHM_UNMAP    57 /* EBX = address returned by SHM_MAP */
#define NOVA_SYS_SHM_DESTROY  58 /* EBX = handle */
#define NOVA_SYS_SHM_INFO     59 /* EBX = nova_shm_info_t* (in/out) */

/* Rights, for GRANT and MAP. WRITE implies READ on this hardware, so a
 * MAP asking for WRITE alone is rejected (-EINVAL), not quietly widened. */
#define NOVA_SHM_RIGHT_READ   1u
#define NOVA_SHM_RIGHT_WRITE  2u

/* Limits. */
#define NOVA_SHM_PAGE_SIZE            4096u
#define NOVA_SHM_MAX_OBJECT_BYTES     (16u * 1024u * 1024u)
#define NOVA_SHM_MAX_TOTAL_BYTES      (24u * 1024u * 1024u)
#define NOVA_SHM_MAX_OBJECTS          32u
#define NOVA_SHM_MAX_OBJECTS_PER_PROC 8u
#define NOVA_SHM_MAX_MAPS_PER_PROC    8u
#define NOVA_SHM_MAX_GRANTS           8u

/* Where mappings are placed. */
#define NOVA_SHM_REGION_BASE          0x68000000u
#define NOVA_SHM_REGION_END           0x7C000000u

/* SHM_CREATE: in: size (bytes, > 0, <= MAX_OBJECT_BYTES), flags (must be
 * 0). out: handle, actual_size (rounded up to whole pages). */
typedef struct {
    unsigned int size;
    unsigned int flags;
    unsigned int handle;
    unsigned int actual_size;
} nova_shm_create_t;

/* SHM_GRANT (owner only): let `pid` map the object with `rights`
 * (READ, or READ|WRITE); rights == 0 revokes. Revoking stops NEW mappings;
 * a mapping the process already holds stays until it unmaps or exits.
 * -ESRCH if `pid` is not a live process. */
typedef struct {
    unsigned int handle;
    int pid;
    unsigned int rights;
    unsigned int flags; /* must be 0 */
} nova_shm_grant_t;

/* SHM_MAP: in: handle, rights wanted (READ or READ|WRITE), flags (must be
 * 0). out: addr, size. The same object cannot be mapped twice by one
 * process (-EEXIST). */
typedef struct {
    unsigned int handle;
    unsigned int rights;
    unsigned int flags;
    unsigned int addr;
    unsigned int size;
} nova_shm_map_t;

/* SHM_INFO: in: handle. Allowed to the owner and to granted processes. */
typedef struct {
    unsigned int handle;
    unsigned int size;
    int owner_pid;
    unsigned int mappings;  /* live mappings, all processes */
    unsigned int my_rights; /* the caller's rights on it */
    unsigned int flags;     /* 0 */
} nova_shm_info_t;

/* Error numbers. The kernel cannot include <errno.h>; these repeat the
 * errno values it returns, and novashm.h asserts at compile time that they
 * still match userland's errno.h. */
#define NOVA_SHM_ERR_PERM    1  /* EPERM:  owner-only operation */
#define NOVA_SHM_ERR_SRCH    3  /* ESRCH:  no such process */
#define NOVA_SHM_ERR_BADF    9  /* EBADF:  no such (or destroyed) object */
#define NOVA_SHM_ERR_NOMEM  12  /* ENOMEM: out of memory / address space */
#define NOVA_SHM_ERR_ACCES  13  /* EACCES: not granted that access */
#define NOVA_SHM_ERR_FAULT  14  /* EFAULT: bad pointer */
#define NOVA_SHM_ERR_EXIST  17  /* EEXIST: already mapped by this process */
#define NOVA_SHM_ERR_INVAL  22  /* EINVAL: bad argument */
#define NOVA_SHM_ERR_NOSPC  28  /* ENOSPC: an object/mapping/grant limit */

/* Compile-time ABI checks (the array-size trick; works in C89). */
typedef char nova_shm_abi_check_create[(sizeof(nova_shm_create_t) == 16) ? 1 : -1];
typedef char nova_shm_abi_check_grant[(sizeof(nova_shm_grant_t) == 16) ? 1 : -1];
typedef char nova_shm_abi_check_map[(sizeof(nova_shm_map_t) == 20) ? 1 : -1];
typedef char nova_shm_abi_check_info[(sizeof(nova_shm_info_t) == 24) ? 1 : -1];

#endif
