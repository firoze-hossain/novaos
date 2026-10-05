/*
 * kernel/ipc/shm.c - Phase 83: the C half of shared-memory IPC. See
 * shm.h, kernel/rust/shm.rs (the real subsystem) and
 * userland/libc/include/nova_shm_abi.h (the contract).
 *
 * Two jobs only:
 *   1. the syscall entry points: validate every user pointer BEFORE
 *      touching it (a fault in kernel mode is a panic here), copy the
 *      argument struct in, call Rust, copy results out;
 *   2. the shm_hal_* functions Rust uses to reach the allocator and the
 *      current process's page tables.
 */
#include "shm.h"
#include "../arch/x86/mm/paging.h"
#include "../arch/x86/mm/pmm.h"
#include "../include/kernel.h"
#include "../lib/string.h"
#include "../task/process.h"
#include "../task/scheduler.h"

/* The syscall numbers are defined once, in the shared ABI header, and
 * repeated in kernel/arch/x86/cpu/syscall.h; this stops the two copies
 * from drifting apart (the array has a negative size if they differ). */
#include "../arch/x86/cpu/syscall.h"
typedef char shm_sysno_create[(SYS_SHM_CREATE == NOVA_SYS_SHM_CREATE) ? 1 : -1];
typedef char shm_sysno_grant[(SYS_SHM_GRANT == NOVA_SYS_SHM_GRANT) ? 1 : -1];
typedef char shm_sysno_map[(SYS_SHM_MAP == NOVA_SYS_SHM_MAP) ? 1 : -1];
typedef char shm_sysno_unmap[(SYS_SHM_UNMAP == NOVA_SYS_SHM_UNMAP) ? 1 : -1];
typedef char shm_sysno_destroy[(SYS_SHM_DESTROY == NOVA_SYS_SHM_DESTROY) ? 1 : -1];
typedef char shm_sysno_info[(SYS_SHM_INFO == NOVA_SYS_SHM_INFO) ? 1 : -1];

/* ------------------------------------------------------------------
 * The hardware-abstraction functions kernel/rust/shm.rs calls.
 * ------------------------------------------------------------------ */

/* A frame, or 0. The allocator only hands out frames below the
 * identity-map ceiling (pmm.c's allocatable_frames()), so the kernel can
 * zero it through its physical address; the check here is defense in
 * depth because the failure it guards against is a kernel panic. */
uint32_t shm_hal_alloc_frame(void) {
    uint32_t frame = pmm_alloc_frame();
    if (frame == 0) {
        return 0;
    }
    if (frame >= PAGING_IDENTITY_MAP_BYTES) {
        pmm_free_frame(frame);
        return 0;
    }
    return frame;
}

void shm_hal_free_frame(uint32_t frame) {
    pmm_free_frame(frame);
}

void shm_hal_zero_frame(uint32_t frame) {
    memset((void*)frame, 0, 4096);
}

/* `pid` is always the calling process (the syscall's own); anything else
 * is a bug in the caller and is refused rather than obeyed. */
int shm_hal_map_page(int pid, uint32_t vaddr, uint32_t frame, uint32_t writable) {
    process_t* p = process_current();
    if (p == NULL || p->pid != pid) {
        return -1;
    }
    uint32_t flags = PAGE_PRESENT | PAGE_USER | PAGE_SHM |
                     (writable ? PAGE_WRITE : 0);
    return paging_map_page((uint32_t*)p->page_directory_phys, vaddr, frame,
                           flags) ? 0 : -1;
}

void shm_hal_unmap_page(int pid, uint32_t vaddr) {
    process_t* p = process_current();
    if (p == NULL || p->pid != pid) {
        return;
    }
    (void)paging_unmap_page((uint32_t*)p->page_directory_phys, vaddr);
}

int shm_hal_pid_is_live(int pid) {
    return process_is_live(pid) ? 1 : 0;
}

/* ------------------------------------------------------------------
 * Syscall entry points.
 * ------------------------------------------------------------------ */

int shm_sys_create(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_shm_create_t), true)) {
        return -NOVA_SHM_ERR_FAULT;
    }
    nova_shm_create_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    uint32_t handle = 0, size = 0;
    int rc = rust_shm_create(pid, req.size, req.flags, &handle, &size);
    if (rc != 0) {
        return rc;
    }
    req.handle = handle;
    req.actual_size = size;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

int shm_sys_grant(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_shm_grant_t), false)) {
        return -NOVA_SHM_ERR_FAULT;
    }
    nova_shm_grant_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    if (req.flags != 0) {
        return -NOVA_SHM_ERR_INVAL;
    }
    return rust_shm_grant(pid, req.handle, req.pid, req.rights);
}

int shm_sys_map(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_shm_map_t), true)) {
        return -NOVA_SHM_ERR_FAULT;
    }
    nova_shm_map_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    if (req.flags != 0) {
        return -NOVA_SHM_ERR_INVAL;
    }
    uint32_t addr = 0, size = 0;
    int rc = rust_shm_map(pid, req.handle, req.rights, &addr, &size);
    if (rc != 0) {
        return rc;
    }
    req.addr = addr;
    req.size = size;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

int shm_sys_unmap(int pid, uint32_t addr) {
    return rust_shm_unmap(pid, addr);
}

int shm_sys_destroy(int pid, uint32_t handle) {
    return rust_shm_destroy(pid, handle);
}

int shm_sys_info(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_shm_info_t), true)) {
        return -NOVA_SHM_ERR_FAULT;
    }
    nova_shm_info_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    if (req.flags != 0) {
        return -NOVA_SHM_ERR_INVAL;
    }
    uint32_t out[4] = {0, 0, 0, 0};
    int rc = rust_shm_info(pid, req.handle, out);
    if (rc != 0) {
        return rc;
    }
    req.size = out[0];
    req.owner_pid = (int)out[1];
    req.mappings = out[2];
    req.my_rights = out[3];
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

void shm_init(void) {
    uint32_t objects = 0, pages = 0, maps = 0;
    uint32_t bad = rust_shm_selftest(&objects, &pages, &maps);
    kernel_log("[ %s ] Shared-memory IPC: %d objects / %dMB per object / %dMB "
               "total, %d mappings per process, region 0x%x-0x%x "
               "(state %d/%d/%d)\n",
               (bad == 0 && objects == 0 && pages == 0 && maps == 0) ? "OK" : "FAIL",
               (int)NOVA_SHM_MAX_OBJECTS,
               (int)(NOVA_SHM_MAX_OBJECT_BYTES >> 20),
               (int)(NOVA_SHM_MAX_TOTAL_BYTES >> 20),
               (int)NOVA_SHM_MAX_MAPS_PER_PROC,
               (int)NOVA_SHM_REGION_BASE, (int)NOVA_SHM_REGION_END,
               (int)objects, (int)pages, (int)maps);
}
