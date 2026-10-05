#ifndef NOVASHM_H
#define NOVASHM_H

/*
 * novashm.h - Phase 83: the app-side half of shared-memory IPC. A thin,
 * header-only layer over the SYS_SHM_* syscalls (nova_shm_abi.h has the
 * model and the contract) plus the frame-handoff protocol in
 * novashm_chan.h.
 *
 * Typical use - an app and a compositor:
 *
 *   app:        nova_shm_create(&r, nova_chan_region_bytes(w, h));
 *               nova_shm_grant_pid(r.handle, compositor_pid, NOVA_SHM_RIGHT_READ|NOVA_SHM_RIGHT_WRITE);
 *               ...tell the compositor r.handle (a pipe, argv, ...)...
 *               nova_chan_init(r.addr, r.size, w, h);
 *               nova_chan_attach(&ch, r.addr, r.size, 1);
 *               loop { draw into nova_chan_back(&ch); nova_chan_commit(&ch); }
 *   compositor: nova_shm_attach(&r, handle, RIGHT_READ|RIGHT_WRITE);
 *               nova_chan_attach(&ch, r.addr, r.size, 0);
 *               each frame: if ((px = nova_chan_acquire(&ch))) copy px into the
 *               window's place in its own surface; present.
 *
 * (The consumer asks for READ|WRITE only because the channel carries a
 * stop flag and the buffer-ownership word that both sides update; a pure
 * viewer of a one-way buffer would map READ-only.)
 */

#include <errno.h>
#include <novasys.h>
#include "novashm_chan.h"

/* The kernel cannot include errno.h, so nova_shm_abi.h repeats the numbers
 * it returns. If either side is ever edited out of step this stops the
 * build instead of letting a program mis-handle an error. */
typedef char novashm_errno_perm [(NOVA_SHM_ERR_PERM  == EPERM)  ? 1 : -1];
typedef char novashm_errno_srch [(NOVA_SHM_ERR_SRCH  == ESRCH)  ? 1 : -1];
typedef char novashm_errno_badf [(NOVA_SHM_ERR_BADF  == EBADF)  ? 1 : -1];
typedef char novashm_errno_nomem[(NOVA_SHM_ERR_NOMEM == ENOMEM) ? 1 : -1];
typedef char novashm_errno_acces[(NOVA_SHM_ERR_ACCES == EACCES) ? 1 : -1];
typedef char novashm_errno_fault[(NOVA_SHM_ERR_FAULT == EFAULT) ? 1 : -1];
typedef char novashm_errno_exist[(NOVA_SHM_ERR_EXIST == EEXIST) ? 1 : -1];
typedef char novashm_errno_inval[(NOVA_SHM_ERR_INVAL == EINVAL) ? 1 : -1];
typedef char novashm_errno_nospc[(NOVA_SHM_ERR_NOSPC == ENOSPC) ? 1 : -1];

typedef struct {
    unsigned int handle;
    void* addr;
    unsigned int size;
} nova_shm_region_t;

/* Creates an object of at least `size` bytes AND maps it read/write.
 * Returns 0, or a negative errno (if the create succeeded but the map did
 * not, the object is destroyed again so nothing is stranded). */
static inline int nova_shm_create(nova_shm_region_t* r, unsigned int size) {
    nova_shm_create_t c;
    c.size = size;
    c.flags = 0;
    c.handle = 0;
    c.actual_size = 0;
    int rc = sys_shm_create(&c);
    if (rc < 0) {
        return rc;
    }
    nova_shm_map_t m;
    m.handle = c.handle;
    m.rights = NOVA_SHM_RIGHT_READ | NOVA_SHM_RIGHT_WRITE;
    m.flags = 0;
    m.addr = 0;
    m.size = 0;
    rc = sys_shm_map(&m);
    if (rc < 0) {
        sys_shm_destroy(c.handle);
        return rc;
    }
    r->handle = c.handle;
    r->addr = (void*)(unsigned long)m.addr;
    r->size = m.size;
    return 0;
}

/* Maps an existing object with `rights` (NOVA_SHM_RIGHT_*). Needs the
 * owner to have granted this process those rights (-EACCES otherwise). */
static inline int nova_shm_attach(nova_shm_region_t* r, unsigned int handle, unsigned int rights) {
    nova_shm_map_t m;
    m.handle = handle;
    m.rights = rights;
    m.flags = 0;
    m.addr = 0;
    m.size = 0;
    int rc = sys_shm_map(&m);
    if (rc < 0) {
        return rc;
    }
    r->handle = handle;
    r->addr = (void*)(unsigned long)m.addr;
    r->size = m.size;
    return 0;
}

static inline int nova_shm_detach(nova_shm_region_t* r) {
    int rc = sys_shm_unmap((unsigned int)(unsigned long)r->addr);
    if (rc == 0) {
        r->addr = 0;
    }
    return rc;
}

/* Owner only: let process `pid` map the object with `rights`; 0 revokes. */
static inline int nova_shm_grant_pid(unsigned int handle, int pid, unsigned int rights) {
    nova_shm_grant_t g;
    g.handle = handle;
    g.pid = pid;
    g.rights = rights;
    g.flags = 0;
    return sys_shm_grant(&g);
}

/* This process's own pid. There is no getpid syscall, but SHM_INFO reports
 * an object's owner, and for an object THIS process owns that is itself.
 * Returns the pid, or a negative errno. */
static inline int nova_shm_owner_pid(unsigned int handle) {
    nova_shm_info_t i;
    i.handle = handle;
    i.size = 0;
    i.owner_pid = 0;
    i.mappings = 0;
    i.my_rights = 0;
    i.flags = 0;
    int rc = sys_shm_info(&i);
    return rc < 0 ? rc : i.owner_pid;
}

#endif
