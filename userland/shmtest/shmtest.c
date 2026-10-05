/*
 * shmtest.c - Phase 83: the in-OS conformance test for the SYS_SHM_*
 * shared-memory syscalls. A real ring-3 program making real int 0x80
 * calls against the real kernel, and - because shared memory is only
 * meaningful between processes - fork()ing real peers to share with.
 * kernel/task/exec_trust_demo.c runs it at boot and tools/python/
 * test_runner.py checks its "[shmtest] PASS" lines. It is the in-OS
 * counterpart of the host tests (kernel/rust/shm.rs against a mock MMU,
 * tools/tests/novashm_test.c against the triple-buffer protocol): those
 * prove the logic, this proves the whole stack - syscall, pointer
 * validation, page-table entries, fork, process exit, and the display.
 *
 *   SHMTEST.ELF [gpu]
 *
 *   (default)  every group, with the display on the default backend
 *   gpu        only the end-to-end pixel handoff, on the virtio-gpu backend
 *
 * HOW THE PROPERTIES ARE CHECKED (each one is a way the feature could
 * be wrong in a way a plain "it returned 0" would miss):
 *
 *  - Memory is really SHARED, not copied: a forked child's write is seen
 *    by its parent, and the parent's write AFTER the fork is seen by the
 *    child. In the same child, an ordinary private page written the same
 *    way is NOT seen by the parent - the control that shows "shared" is a
 *    different thing from the copy-on-write every other page gets.
 *  - Access control is the ACL, not secrecy of the handle: a process that
 *    holds a valid handle but no grant is refused (-EACCES).
 *  - A read-only mapping stays read-only to the KERNEL too. The kernel
 *    runs with CR0.WP clear, so it would happily write through a read-only
 *    page; the test hands it a syscall whose OUTPUT buffer lies inside a
 *    read-only mapping and requires a bad-address error. (It does not
 *    simply store to the page and fault: a deliberate fault would trip the
 *    project's own panic/fault detector.)
 *  - Nothing leaks: far more shared memory than the machine has is cycled
 *    through create/map/unmap/destroy and through short-lived owner
 *    processes that never clean up; a leak would exhaust the allocator.
 *  - The orphan rules: when an owner exits, mappings others already hold
 *    keep working but nobody new can map.
 *  - Pixels really travel: a producer PROCESS draws frames into shared
 *    memory; a compositor process takes them with the lock-free triple
 *    buffer, checks every pixel of every frame against the pattern for its
 *    sequence number (a torn frame mixes two), copies it into a
 *    framebuffer surface, presents it, and reads the DISPLAY back to
 *    verify what reached the screen.
 *
 * Every failure prints "[shmtest] FAIL: ..." (the uppercase word also
 * trips test_runner.py's global no-fail assertion). Output for passing
 * checks deliberately avoids the words that assertion greps for.
 */
#include <errno.h>
#include <novagfx.h>
#include <novashm.h>
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failures;
static const char* backend_name = "auto";
static unsigned int backend_id = NOVA_FB_BACKEND_ANY;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 25) { \
            printf("[shmtest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

#define EXPECT(call, want) do { \
    int rc_ = (call); \
    checks++; \
    if (rc_ != (want)) { \
        failures++; \
        if (failures <= 25) \
            printf("[shmtest] FAIL: %s returned %d, expected %d (line %d)\n", \
                   #call, rc_, (int)(want), __LINE__); \
    } \
} while (0)

#define RW (NOVA_SHM_RIGHT_READ | NOVA_SHM_RIGHT_WRITE)
#define RO NOVA_SHM_RIGHT_READ

/* ---- thin helpers that expose the raw syscalls ------------------------ */

static int raw_create(unsigned int size, unsigned int flags, unsigned int* handle) {
    nova_shm_create_t c;
    c.size = size; c.flags = flags; c.handle = 0; c.actual_size = 0;
    int rc = sys_shm_create(&c);
    if (rc == 0) *handle = c.handle;
    return rc;
}

static int raw_map(unsigned int handle, unsigned int rights, unsigned int* addr, unsigned int* size) {
    nova_shm_map_t m;
    m.handle = handle; m.rights = rights; m.flags = 0; m.addr = 0; m.size = 0;
    int rc = sys_shm_map(&m);
    if (rc == 0) {
        if (addr) *addr = m.addr;
        if (size) *size = m.size;
    }
    return rc;
}

static int raw_info(unsigned int handle, nova_shm_info_t* out) {
    out->handle = handle; out->size = 0; out->owner_pid = 0;
    out->mappings = 0; out->my_rights = 0; out->flags = 0;
    return sys_shm_info(out);
}

/* Seconds since midnight, for timeouts (there is no other clock). */
static int now_s(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    return t.hour * 3600 + t.minute * 60 + t.second;
}

/* Waits for *v >= want. A peer that died must fail the test, not hang the
 * boot, so there is a deadline. */
static int wait_ge(volatile unsigned int* v, unsigned int want) {
    int start = now_s();
    for (;;) {
        if (__atomic_load_n(v, __ATOMIC_ACQUIRE) >= want) return 1;
        int e = now_s() - start;
        if (e < 0) e += 86400;
        if (e > 15) return 0;
        sys_yield();
    }
}

static void stage_set(volatile unsigned int* v, unsigned int n) {
    __atomic_store_n(v, n, __ATOMIC_RELEASE);
}

/* ---- shared mailbox between parent and its forked children -------------- */

typedef struct {
    volatile unsigned int parent_stage;  /* parent -> child: steps released */
    volatile unsigned int child_stage;   /* child -> parent: steps completed */
    volatile int result[12];             /* child's reports */
    volatile unsigned int handle;        /* an object the child created */
    volatile int parent_pid;
} mailbox_t;

static nova_shm_region_t mbr;
static mailbox_t* mb;

/* ---- group 1: single-process behaviour ------------------------------------ */

static void test_basics(int my_pid) {
    int f0 = failures;
    unsigned int h, addr, size;

    EXPECT(raw_create(10000, 0, &h), 0);
    nova_shm_info_t info;
    EXPECT(raw_info(h, &info), 0);
    CHECK(info.size == 3 * 4096, "10000 bytes should round up to 3 pages, got %u", info.size);
    CHECK(info.owner_pid == my_pid && info.mappings == 0 && info.my_rights == RW,
          "info: owner %d (me %d) mappings %u rights %u", info.owner_pid, my_pid, info.mappings, info.my_rights);

    EXPECT(raw_map(h, RW, &addr, &size), 0);
    CHECK(size == 3 * 4096, "mapped size %u", size);
    CHECK(addr % 4096 == 0 && addr >= NOVA_SHM_REGION_BASE && addr + size <= NOVA_SHM_REGION_END,
          "address 0x%x outside the shared-memory region", addr);
    volatile unsigned char* p = (volatile unsigned char*)(unsigned long)addr;

    /* New memory is zero. Checked over every byte of every page. */
    unsigned int nonzero = 0;
    for (unsigned int i = 0; i < size; i++) if (p[i] != 0) nonzero++;
    CHECK(nonzero == 0, "%u non-zero bytes in a fresh object (a previous owner's data?)", nonzero);

    for (unsigned int i = 0; i < size; i++) p[i] = (unsigned char)(i * 7 + 3);
    EXPECT(raw_info(h, &info), 0);
    CHECK(info.mappings == 1, "mappings %u", info.mappings);

    /* One object, one mapping per process. */
    EXPECT(raw_map(h, RW, NULL, NULL), -EEXIST);

    /* The guard page: the page after the mapping is genuinely unmapped. Probed
     * through the KERNEL (a syscall whose output buffer would run into it), not by
     * touching it, which would fault. */
    *(volatile unsigned int*)(p + size - 8) = 36; /* a valid fb_info struct_size */
    EXPECT(sys_fb_info((nova_fb_info_t*)(p + size - 8)), -EFAULT);
    *(volatile unsigned int*)(p + size - 64) = 36;
    int rc_inside = sys_fb_info((nova_fb_info_t*)(p + size - 64)); /* fully inside: fine */
    CHECK(rc_inside == 0, "a buffer wholly inside the mapping should be accepted, got %d", rc_inside);

    /* Unmap and map again: the object (and its contents) outlive a mapping. */
    EXPECT(sys_shm_unmap(addr), 0);
    EXPECT(sys_shm_unmap(addr), -EINVAL);
    EXPECT(sys_shm_unmap(addr + 4096), -EINVAL);
    EXPECT(raw_map(h, RW, &addr, &size), 0);
    p = (volatile unsigned char*)(unsigned long)addr;
    unsigned int bad = 0;
    for (unsigned int i = 0; i < size - 64; i++) if (p[i] != (unsigned char)(i * 7 + 3)) bad++;
    CHECK(bad == 0, "%u bytes changed across an unmap/map", bad);

    /* Destroy while mapped: the memory stays, the handle dies. */
    EXPECT(sys_shm_destroy(h), 0);
    EXPECT(sys_shm_destroy(h), -EBADF);
    EXPECT(raw_map(h, RO, NULL, NULL), -EBADF);
    EXPECT(raw_info(h, &info), -EBADF);
    CHECK(p[5] == (unsigned char)(5 * 7 + 3), "mapped memory vanished when its object was destroyed");
    EXPECT(sys_shm_unmap(addr), 0);

    /* A reused slot must not honour the old handle. */
    unsigned int h2;
    EXPECT(raw_create(4096, 0, &h2), 0);
    CHECK(h2 != h, "a reused object slot reissued the identical handle 0x%x", h);
    EXPECT(sys_shm_destroy(h), -EBADF);
    /* Recycled memory is zeroed. A fresh-booted machine's RAM is mostly zero
     * already, so the first "new memory is zero" check above could pass even
     * if the kernel never cleared anything. This one cannot: the frames just
     * freed held the 0x?? pattern written above, and the allocator hands
     * freed frames out again first. */
    unsigned int ha2, sz2;
    EXPECT(raw_map(h2, RW, &ha2, &sz2), 0);
    unsigned int dirty = 0;
    for (unsigned int i = 0; i < sz2; i++) if (((volatile unsigned char*)(unsigned long)ha2)[i] != 0) dirty++;
    CHECK(dirty == 0, "%u non-zero bytes in an object built from RECYCLED frames: the previous owner's data leaked", dirty);
    EXPECT(sys_shm_unmap(ha2), 0);
    EXPECT(sys_shm_destroy(h2), 0);

    /* Argument validation. */
    unsigned int dummy;
    EXPECT(raw_create(0, 0, &dummy), -EINVAL);
    EXPECT(raw_create(NOVA_SHM_MAX_OBJECT_BYTES + 1, 0, &dummy), -EINVAL);
    EXPECT(raw_create(0xFFFFFFFFu, 0, &dummy), -EINVAL);
    EXPECT(raw_create(4096, 1, &dummy), -EINVAL);
    EXPECT(raw_create(4096, 0, &h), 0);
    EXPECT(raw_map(h, 0, NULL, NULL), -EINVAL);
    EXPECT(raw_map(h, NOVA_SHM_RIGHT_WRITE, NULL, NULL), -EINVAL);
    EXPECT(raw_map(h, 4, NULL, NULL), -EINVAL);
    EXPECT(nova_shm_grant_pid(h, my_pid, RO), -EINVAL);
    EXPECT(nova_shm_grant_pid(h, 99999, RO), -ESRCH);
    EXPECT(sys_shm_destroy(0), -EBADF);
    EXPECT(sys_shm_destroy(0xFFFFFFFFu), -EBADF);
    EXPECT(sys_shm_unmap(0), -EINVAL);
    EXPECT(sys_shm_unmap(0x68000001u), -EINVAL);
    EXPECT(raw_info(0x12345, &info), -EBADF);
    EXPECT(sys_shm_destroy(h), 0);

    /* Hostile pointers: each of the struct-taking calls, three kinds of bad address. */
    void* nowhere[] = {
        (void*)0,           /* NULL */
        (void*)0x00100000,  /* the kernel image */
        (void*)0x50000000,  /* user range, never mapped */
        (void*)0xFFFFFFF8,  /* a read here wraps past 4GB */
    };
    for (unsigned i = 0; i < sizeof nowhere / sizeof nowhere[0]; i++) {
        void* q = nowhere[i];
        EXPECT(sys_shm_create((nova_shm_create_t*)q), -EFAULT);
        EXPECT(sys_shm_grant((const nova_shm_grant_t*)q), -EFAULT);
        EXPECT(sys_shm_map((nova_shm_map_t*)q), -EFAULT);
        EXPECT(sys_shm_info((nova_shm_info_t*)q), -EFAULT);
    }
    if (failures == f0) printf("[shmtest] ok: single-process behaviour (create/map/zeroed/guard page/destroy/validation/hostile pointers)\n");
}

/* ---- group 2: limits and leak-freedom --------------------------------------- */

static void test_limits_and_leaks(int my_pid) {
    int f0 = failures;
    /* This program already owns ONE object (the mailbox) and has it mapped,
     * so the per-process limits leave room for one fewer than the maximum. */
    unsigned int hs[16];
    unsigned int n = 0;
    int last_rc = 0;
    for (; n < 16; n++) {
        last_rc = raw_create(4096, 0, &hs[n]);
        if (last_rc != 0) break;
    }
    CHECK(n == NOVA_SHM_MAX_OBJECTS_PER_PROC - 1, "created %u objects before the limit, expected %u", n,
          NOVA_SHM_MAX_OBJECTS_PER_PROC - 1);
    CHECK(last_rc == -ENOSPC, "the object that exceeds the per-process limit returned %d", last_rc);
    /* Map them all: with the mailbox that is exactly the per-process mapping limit. */
    unsigned int mapped = 0, addr[16];
    for (unsigned int i = 0; i < n; i++) {
        if (raw_map(hs[i], RW, &addr[i], NULL) == 0) mapped++;
    }
    CHECK(mapped == n, "only %u of %u objects could be mapped", mapped, n);
    for (unsigned int i = 0; i < n; i++) {
        if (i < mapped) sys_shm_unmap(addr[i]);
        sys_shm_destroy(hs[i]);
    }

    /* The system-wide cap (24MB). A 16MB object fits only if the machine
     * has the memory spare; if it does not, say so rather than guess. */
    unsigned int big1, big2;
    int rc1 = raw_create(NOVA_SHM_MAX_OBJECT_BYTES, 0, &big1);
    if (rc1 == 0) {
        EXPECT(raw_create(NOVA_SHM_MAX_OBJECT_BYTES, 0, &big2), -ENOMEM); /* 32MB > 24MB */
        sys_shm_destroy(big1);
    } else {
        printf("[shmtest] note: the 24MB cap check was skipped (a 16MB object could not be allocated: %d)\n", rc1);
    }

    /* LEAK CYCLES. Allocate more shared memory than the machine has, one
     * object at a time. If any path forgot to free its frames the allocator
     * would run dry long before the loop ends. */
    int cycles_ok = 0;
    for (int i = 0; i < 80; i++) {
        unsigned int h, a;
        if (raw_create(1024 * 1024, 0, &h) != 0) break;
        if (raw_map(h, RW, &a, NULL) != 0) { sys_shm_destroy(h); break; }
        ((volatile unsigned char*)(unsigned long)a)[4096 * 100] = (unsigned char)i;
        sys_shm_unmap(a);
        sys_shm_destroy(h);
        cycles_ok++;
    }
    CHECK(cycles_ok == 80, "only %d of 80 create/map/unmap/destroy cycles of 1MB succeeded - a leak?", cycles_ok);

    /* ...and again through OWNER PROCESSES that exit without cleaning up:
     * the exit path must free what they owned. */
    int exits_ok = 0;
    for (int i = 0; i < 40; i++) {
        int pid = sys_fork();
        if (pid == 0) {
            unsigned int h, a;
            int ok = raw_create(1024 * 1024, 0, &h) == 0 && raw_map(h, RW, &a, NULL) == 0;
            if (ok) ((volatile unsigned char*)(unsigned long)a)[12345] = 0x5A;
            sys_exit(ok ? 0 : 1); /* no unmap, no destroy */
        }
        if (pid < 0) break;
        if (sys_wait(pid) == 0) exits_ok++;
    }
    CHECK(exits_ok == 40, "only %d of 40 short-lived owner processes succeeded - memory exhausted by earlier leaks?", exits_ok);
    unsigned int h_final;
    int rc_final = raw_create(8 * 1024 * 1024, 0, &h_final);
    CHECK(rc_final == 0, "an 8MB object cannot be created after the cycles: %d", rc_final);
    if (rc_final == 0) sys_shm_destroy(h_final);
    (void)my_pid;
    if (failures == f0) printf("[shmtest] ok: limits, and no leaks across 80 create/destroy cycles + 40 exiting owners\n");
}

/* A private page, for the copy-on-write control. */
static unsigned char priv_page[4096] __attribute__((aligned(4096)));

/* ---- group 3: sharing across fork() ---------------------------------------------- */

static void test_fork_sharing(void) {
    int f0 = failures;
    nova_shm_region_t s;
    EXPECT(nova_shm_create(&s, 4096), 0);
    volatile unsigned char* sp = (volatile unsigned char*)s.addr;
    priv_page[0] = 1;
    sp[0] = 1;
    sp[1] = 0x11;
    stage_set(&mb->parent_stage, 0);
    stage_set(&mb->child_stage, 0);

    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        /* the child inherited the mapping at the same address, and sees the
         * parent's earlier write */
        CHECK(sp[1] == 0x11, "the child cannot see the parent's pre-fork write");
        sp[0] = 0xAA;            /* shared: the parent must see this */
        priv_page[0] = 0xBB;     /* private: the parent must NOT see this */
        mb->result[0] = priv_page[0];
        stage_set(&mb->child_stage, 1);
        /* ...and now the parent writes AFTER the fork; this is the case a
         * copy-on-write fork would break (the parent's write would go to a
         * private copy) */
        CHECK(wait_ge(&mb->parent_stage, 1), "timed out waiting for the parent");
        CHECK(sp[2] == 0x77, "the child cannot see a write the parent made AFTER fork (got 0x%x)", sp[2]);
        sp[3] = 0xD4;
        stage_set(&mb->child_stage, 2);
        sys_exit(failures);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    CHECK(wait_ge(&mb->child_stage, 1), "timed out waiting for the child");
    /* While the child lives, the object has TWO mappings. (Checking only
     * after it exits would pass even if fork never recorded the child's.) */
    nova_shm_info_t live;
    EXPECT(raw_info(s.handle, &live), 0);
    CHECK(live.mappings == 2, "a forked child's mapping was not recorded: mappings=%u, expected 2", live.mappings);
    CHECK(sp[0] == 0xAA, "the child's write to shared memory is invisible to the parent (0x%x): fork made it copy-on-write?", sp[0]);
    CHECK(priv_page[0] == 1, "the control failed: the child's write to a PRIVATE page leaked into the parent (0x%x)", priv_page[0]);
    CHECK(mb->result[0] == 0xBB, "the child did not see its own private write");
    sp[2] = 0x77;
    stage_set(&mb->parent_stage, 1);
    CHECK(wait_ge(&mb->child_stage, 2), "timed out waiting for the child");
    CHECK(sp[3] == 0xD4, "the child's later write is invisible to the parent");
    int code = sys_wait(pid);
    CHECK(code == 0, "the child reported %d failed checks", code);
    /* after the child is gone the parent's mapping is intact */
    CHECK(sp[0] == 0xAA && sp[3] == 0xD4, "the parent's mapping changed when the child exited");
    nova_shm_info_t info;
    EXPECT(raw_info(s.handle, &info), 0);
    CHECK(info.mappings == 1, "the exited child's mapping was not released (mappings=%u)", info.mappings);
    EXPECT(nova_shm_detach(&s), 0);
    EXPECT(sys_shm_destroy(s.handle), 0);
    if (failures == f0) printf("[shmtest] ok: fork shares memory in both directions (and a private page, as the control, does not)\n");
}

/* ---- group 4: access control across processes ---------------------------------------- */

static void test_acl_between_processes(int my_pid) {
    int f0 = failures;
    unsigned int h;
    EXPECT(raw_create(4096, 0, &h), 0);
    mb->handle = h;
    stage_set(&mb->parent_stage, 0);
    stage_set(&mb->child_stage, 0);
    for (int i = 0; i < 12; i++) mb->result[i] = -9999;

    /* NOT mapped before the fork, so the child inherits no mapping of it:
     * everything it gets, it gets through the ACL. */
    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        unsigned int a, a2;
        /* 1. a valid handle and no grant: refused */
        mb->result[0] = raw_map(h, RO, &a, NULL);
        mb->result[1] = raw_map(h, RW, &a, NULL);
        nova_shm_info_t info;
        mb->result[2] = raw_info(h, &info);
        stage_set(&mb->child_stage, 1);
        CHECK(wait_ge(&mb->parent_stage, 1), "timed out (1)");
        /* 2. granted READ-ONLY: a read-only mapping works, a writable one does not */
        mb->result[3] = raw_map(h, RW, &a, NULL);
        mb->result[4] = raw_map(h, RO, &a, NULL);
        volatile unsigned char* p = (volatile unsigned char*)(unsigned long)a;
        mb->result[5] = (p[100] == 0x6C) ? 0 : -1;   /* the parent's data is visible */
        /* the KERNEL must refuse to write through the read-only page */
        mb->result[6] = sys_fb_info((nova_fb_info_t*)(p + 16));
        mb->result[7] = raw_map(h, RW, &a2, NULL); /* rights are checked before "already mapped" */
        stage_set(&mb->child_stage, 2);
        CHECK(wait_ge(&mb->parent_stage, 2), "timed out (2)");
        /* 3. upgraded to read-write */
        sys_shm_unmap(a);
        mb->result[8] = raw_map(h, RW, &a, NULL);
        p = (volatile unsigned char*)(unsigned long)a;
        p[200] = 0x3E;
        stage_set(&mb->child_stage, 3);
        CHECK(wait_ge(&mb->parent_stage, 3), "timed out (3)");
        /* 4. revoked: the existing mapping keeps working, a new one is refused */
        mb->result[9] = (p[100] == 0x6C) ? 0 : -1;
        sys_shm_unmap(a);
        mb->result[10] = raw_map(h, RO, &a, NULL);
        stage_set(&mb->child_stage, 4);
        sys_exit(failures);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    nova_shm_region_t r;
    EXPECT(nova_shm_attach(&r, h, RW), 0);
    volatile unsigned char* p = (volatile unsigned char*)r.addr;
    p[100] = 0x6C;
    *(volatile unsigned int*)(p + 16) = 36; /* a valid struct_size for the kernel-write probe */

    CHECK(wait_ge(&mb->child_stage, 1), "timed out waiting for the child (1)");
    CHECK(mb->result[0] == -EACCES && mb->result[1] == -EACCES, "an ungranted process mapped the object: ro=%d rw=%d", mb->result[0], mb->result[1]);
    CHECK(mb->result[2] == -EACCES, "an ungranted process read info: %d", mb->result[2]);
    EXPECT(nova_shm_grant_pid(h, pid, RO), 0);
    stage_set(&mb->parent_stage, 1);

    CHECK(wait_ge(&mb->child_stage, 2), "timed out waiting for the child (2)");
    CHECK(mb->result[3] == -EACCES, "a read-only grant yielded a writable mapping: %d", mb->result[3]);
    CHECK(mb->result[4] == 0, "a read-only grant could not map read-only: %d", mb->result[4]);
    CHECK(mb->result[5] == 0, "the reader could not see the owner's data");
    CHECK(mb->result[6] == -EFAULT, "the kernel wrote through a read-only mapping (syscall returned %d)", mb->result[6]);
    CHECK(mb->result[7] == -EACCES, "rights check order: %d", mb->result[7]);
    EXPECT(nova_shm_grant_pid(h, pid, RW), 0);
    stage_set(&mb->parent_stage, 2);

    CHECK(wait_ge(&mb->child_stage, 3), "timed out waiting for the child (3)");
    CHECK(mb->result[8] == 0, "an upgraded grant could not map read-write: %d", mb->result[8]);
    CHECK(p[200] == 0x3E, "the grantee's write is invisible to the owner");
    EXPECT(nova_shm_grant_pid(h, pid, 0), 0); /* revoke */
    stage_set(&mb->parent_stage, 3);

    CHECK(wait_ge(&mb->child_stage, 4), "timed out waiting for the child (4)");
    CHECK(mb->result[9] == 0, "revoking tore down an existing mapping");
    CHECK(mb->result[10] == -EACCES, "a revoked process mapped it again: %d", mb->result[10]);
    int code = sys_wait(pid);
    CHECK(code == 0, "the child reported %d failed checks", code);
    nova_shm_info_t info;
    EXPECT(raw_info(h, &info), 0);
    CHECK(info.mappings == 1, "mappings after the child exited: %u", info.mappings);
    nova_shm_detach(&r);
    sys_shm_destroy(h);
    (void)my_pid;
    if (failures == f0) printf("[shmtest] ok: access control between processes (no grant / read-only / read-write / revoked; the kernel honours read-only)\n");
}

/* ---- group 5: what happens when the owner exits ------------------------------------------ */

static void test_owner_exit(int my_pid) {
    int f0 = failures;
    stage_set(&mb->parent_stage, 0);
    stage_set(&mb->child_stage, 0);
    mb->parent_pid = my_pid;
    mb->handle = 0;
    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        nova_shm_region_t r;
        CHECK(nova_shm_create(&r, 8192) == 0, "child create");
        ((volatile unsigned char*)r.addr)[4097] = 0x9D;
        CHECK(nova_shm_grant_pid(r.handle, mb->parent_pid, RO) == 0, "child grant");
        mb->handle = r.handle;
        stage_set(&mb->child_stage, 1);
        CHECK(wait_ge(&mb->parent_stage, 1), "timed out");
        sys_exit(failures); /* the owner exits: no unmap, no destroy */
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    CHECK(wait_ge(&mb->child_stage, 1), "timed out waiting for the child");
    nova_shm_region_t r;
    EXPECT(nova_shm_attach(&r, mb->handle, RO), 0);
    CHECK(((volatile unsigned char*)r.addr)[4097] == 0x9D, "cannot read the owner's data");
    nova_shm_info_t info;
    EXPECT(raw_info(r.handle, &info), 0);
    CHECK(info.owner_pid == pid, "owner %d, expected the child %d", info.owner_pid, pid);
    stage_set(&mb->parent_stage, 1);
    int code = sys_wait(pid);
    CHECK(code == 0, "the child reported %d failed checks", code);

    /* The owner is gone. The mapping I hold must keep working; the handle must not. */
    CHECK(((volatile unsigned char*)r.addr)[4097] == 0x9D, "the mapping died with its owner");
    EXPECT(raw_info(r.handle, &info), -EBADF);
    unsigned int a;
    EXPECT(raw_map(r.handle, RO, &a, NULL), -EBADF);
    EXPECT(nova_shm_detach(&r), 0);  /* the last mapping: this frees the memory */
    EXPECT(raw_map(r.handle, RO, &a, NULL), -EBADF);
    if (failures == f0) printf("[shmtest] ok: owner exit - existing mappings survive, new ones are refused, memory is freed with the last mapping\n");
}

/* ---- group 6: pixels, end to end ----------------------------------------------------------- */

enum { FW = 128, FH = 96, WANT_FRAMES = 12, MAX_PRODUCED = 20000 };

/* Pixel (x,y) of frame `seq`. (0,0) carries the sequence number itself;
 * every other pixel depends on it too, so a frame torn between two
 * sequence numbers cannot pass for either. X byte is 0: the display
 * ignores it on present and reads it back as 0. */
static unsigned int pattern(unsigned int seq, unsigned int x, unsigned int y) {
    if (x == 0 && y == 0) return seq & 0x00FFFFFFu;
    unsigned int r = (seq * 37 + x) & 255, g = (y * 5 + seq) & 255, b = (x ^ y ^ seq) & 255;
    return (r << 16) | (g << 8) | b;
}

static int frame_matches(const unsigned int* px, unsigned int seq) {
    for (unsigned int y = 0; y < FH; y++)
        for (unsigned int x = 0; x < FW; x++)
            if (px[y * FW + x] != pattern(seq, x, y)) return 0;
    return 1;
}

static void test_pixel_handoff(void) {
    int f0 = failures;
    unsigned int bytes = nova_chan_region_bytes(FW, FH);
    unsigned int h;
    EXPECT(raw_create(bytes, 0, &h), 0);
    stage_set(&mb->parent_stage, 0);
    stage_set(&mb->child_stage, 0);

    /* Not mapped by the parent yet: the child (the "app") gets it by grant. */
    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        nova_shm_region_t r;
        int start = now_s(), rc = -1;
        while ((rc = nova_shm_attach(&r, h, RW)) != 0) {
            int e = now_s() - start; if (e < 0) e += 86400;
            if (e > 15) break;
            sys_yield();
        }
        CHECK(rc == 0, "producer: could not map the channel (%d)", rc);
        nova_chan_t ch;
        int ok = (rc == 0);
        start = now_s();
        while (ok && nova_chan_attach(&ch, r.addr, r.size, 1) != 0) {
            int e = now_s() - start; if (e < 0) e += 86400;
            if (e > 15) { ok = 0; break; }
            sys_yield();
        }
        CHECK(ok, "producer: the channel never became ready");
        unsigned int seq = 0;
        while (ok && !nova_chan_stop_requested(&ch) && seq < MAX_PRODUCED) {
            seq++;
            unsigned int* px = nova_chan_back(&ch);
            for (unsigned int y = 0; y < FH; y++)
                for (unsigned int x = 0; x < FW; x++)
                    px[y * FW + x] = pattern(seq, x, y);
            nova_chan_commit(&ch);
            if ((seq & 3) == 0) sys_yield();
        }
        sys_exit(failures);
    }
    CHECK(pid > 0, "fork failed: %d", pid);

    /* The "compositor". */
    nova_shm_region_t r;
    EXPECT(nova_shm_attach(&r, h, RW), 0);
    EXPECT(nova_shm_grant_pid(h, pid, RW), 0);
    CHECK(nova_chan_init(r.addr, r.size, FW, FH) == 0, "channel init");
    nova_chan_t ch;
    CHECK(nova_chan_attach(&ch, r.addr, r.size, 0) == 0, "compositor: attach");

    EXPECT(sys_fb_acquire(backend_id), 0);
    nova_surface_t surf;
    EXPECT(nova_surface_create(&surf, FW, FH), 0);
    unsigned int* back = (unsigned int*)malloc(FW * FH * 4);
    CHECK(back != NULL, "no memory for the readback buffer");

    unsigned int got = 0, last_seq = 0, torn = 0, mismatched_on_screen = 0, order_bad = 0;
    int start = now_s();
    while (got < WANT_FRAMES && back != NULL) {
        const unsigned int* px = nova_chan_acquire(&ch);
        if (!px) {
            int e = now_s() - start; if (e < 0) e += 86400;
            if (e > 20) break;
            sys_yield();
            continue;
        }
        unsigned int seq = px[0];
        if (seq <= last_seq) order_bad++;
        last_seq = seq;
        if (!frame_matches(px, seq)) torn++;
        /* shared memory -> the compositor's own surface -> the display */
        memcpy(surf.pixels, px, FW * FH * 4);
        EXPECT(nova_surface_present_full(&surf, 100, 50), 0);
        if (nova_display_readback(100, 50, FW, FH, back, FW * 4) == 0) {
            if (memcmp(back, px, FW * FH * 4) != 0) mismatched_on_screen++;
        } else {
            mismatched_on_screen++;
        }
        got++;
    }
    unsigned int produced = nova_chan_frames_produced(&ch);
    nova_chan_request_stop(&ch);
    CHECK(got == WANT_FRAMES, "only %u of %d frames arrived", got, WANT_FRAMES);
    CHECK(torn == 0, "%u frames were torn (pixels from two different frames)", torn);
    CHECK(order_bad == 0, "%u frames arrived out of order or repeated", order_bad);
    CHECK(mismatched_on_screen == 0, "%u frames differ between shared memory and the display", mismatched_on_screen);
    int code = sys_wait(pid);
    CHECK(code == 0, "the producer reported %d failed checks", code);
    produced = nova_chan_frames_produced(&ch);
    CHECK(produced >= got, "produced %u < consumed %u", produced, got);
    printf("[shmtest] frames: producer drew %u, compositor took %u (slow consumers skip, never tear)\n", produced, got);

    free(back);
    nova_surface_destroy(&surf);
    sys_fb_release();
    nova_shm_detach(&r);
    sys_shm_destroy(h);
    if (failures == f0) printf("[shmtest] ok: pixel handoff end to end on the %s display backend (producer process -> shared memory -> compositor -> present -> readback)\n", backend_name);
}

int main(int argc, char** argv) {
    int only_handoff = 0;
    for (int i = 1; i < argc; i++) {
        if (strcmp(argv[i], "gpu") == 0) {
            backend_name = "virtio-gpu";
            backend_id = NOVA_FB_BACKEND_VIRTIOGPU;
            only_handoff = 1;
        } else {
            printf("usage: SHMTEST.ELF [gpu]\n");
            return 2;
        }
    }

    EXPECT(nova_shm_create(&mbr, 4096), 0);
    mb = (mailbox_t*)mbr.addr;
    int my_pid = nova_shm_owner_pid(mbr.handle);
    CHECK(my_pid > 0, "could not learn my own pid: %d", my_pid);

    if (!only_handoff) {
        test_basics(my_pid);
        test_limits_and_leaks(my_pid);
        test_fork_sharing();
        test_acl_between_processes(my_pid);
        test_owner_exit(my_pid);
    }
    test_pixel_handoff();

    nova_shm_detach(&mbr);
    sys_shm_destroy(mbr.handle);

    if (failures == 0) {
        printf("[shmtest] PASS: backend=%s - the full SYS_SHM_* contract holds (%d checks)\n", backend_name, checks);
        return 0;
    }
    printf("[shmtest] FAIL: backend=%s - %d of %d checks failed\n", backend_name, failures, checks);
    return 1;
}
