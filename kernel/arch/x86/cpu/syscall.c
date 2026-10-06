/*
 * syscall.c - int 0x80 dispatch
 *
 * This is the ONLY sanctioned way ring-3 code talks to the kernel - a
 * user task has no other path to VGA output, process control,
 * filesystem access, or anything else privileged, which is the actual
 * security property ring 3 exists to provide (see
 * kernel/task/user_demo.c: it really can't just call vga_puts()
 * directly - that's ring-0-only code, and calling it from ring 3
 * would fault, not merely be bad practice).
 *
 * Phase 11 adds file I/O (SYS_OPEN/SYS_READ/SYS_CLOSE), gated by each
 * process's capability list (process_t.allowed_files, see
 * kernel/task/process.h) - this file is where that gate is actually
 * enforced. A process created via the plain process_create_user_task()
 * has an empty capability list and can open nothing at all; only
 * process_create_sandboxed_task() grants specific filenames.
 */
#include "syscall.h"
#include "idt.h"
#include "gdt.h"
#include "../../drivers/vga/vga.h"
#include "../../fs/vfs.h"
#include "../../drivers/keyboard/keyboard.h"
#include "../../drivers/rtc/rtc.h"
#include "../../drivers/pci/pci.h"
#include "../../drivers/sound/ac97.h"
#include "../../net/udp.h"
#include "../../task/process.h"
#include "../../task/scheduler.h"
#include "../../task/greeter_task.h"
#include "../../lib/string.h"
#include "../../lib/stdio.h"
#include "../../lib/spinlock.h"
#include "../../include/kernel.h"
#include "../../drivers/video/vga_graphics.h"
#include "../../drivers/video/fb.h"
#include "../../ipc/shm.h"
#include "../../ipc/msg.h"
#include "../../drivers/sound/audio.h"
#include "../../task/rlimit.h"
#include "../../../userland/libc/include/nova_rlimit_abi.h"
#include "../mm/paging.h"
#include "../../drivers/mouse/ps2mouse.h"
#include "../../net/icmp.h"
#include "../../net/dns.h"
#include "../../net/tftp.h"
#include "../../net/net.h"

extern void isr128(void);

#define MAX_OPEN_FILES 8

/* Phase 36: which kind of resource a handle actually refers to. The
 * table stayed VFS-only from Phase 11 through Phase 35; pipes (see
 * kernel/rust/pipe.rs) are the first non-VFS-backed handle kind, so
 * every entry now needs to say which it is. pipe_id is only
 * meaningful when kind != OPEN_KIND_VFS. */
typedef enum {
    OPEN_KIND_VFS = 0,
    OPEN_KIND_PIPE_READ,
    OPEN_KIND_PIPE_WRITE,
    /* Phase 58: a TCP socket handle - the same "extend open_files[]
     * with a new non-VFS-backed kind" pattern pipes already
     * established, so SYS_READ/SYS_WRITE_HANDLE/SYS_CLOSE already
     * dispatch to it "for free" once handle_read()/handle_write_handle()/
     * handle_close() below recognize this kind - see kernel/rust/tcp.rs
     * for the real recv()/send()/close() logic each dispatches to. */
    OPEN_KIND_SOCKET,
    /* Phase 78: a UDP socket handle - kept as its own, separate kind
     * rather than reused alongside OPEN_KIND_SOCKET above, because
     * every dispatch site that checks `kind` needs to know which
     * protocol engine (kernel/rust/tcp.rs vs. kernel/rust/udp.rs) a
     * given handle actually belongs to - the two are not
     * interchangeable (a UDP handle has no SYS_LISTEN/SYS_ACCEPT, and
     * now has its own SYS_SENDTO/SYS_RECVFROM a TCP handle doesn't). */
    OPEN_KIND_UDP_SOCKET,
} open_kind_t;

typedef struct {
    bool in_use;
    int owner_pid; /* only the process that opened a handle may use it -
                       guessing/reusing another process's handle number
                       is not a way around the capability check that
                       already happened at SYS_OPEN time */
    open_kind_t kind;
    int pipe_id;    /* meaningful only when kind != OPEN_KIND_VFS - also
                        doubles as the socket's own connection/socket id
                        (kernel/rust/tcp.rs's for OPEN_KIND_SOCKET,
                        kernel/rust/udp.rs's for OPEN_KIND_UDP_SOCKET -
                        each its own, separate table, so the SAME small
                        integer here means a different thing depending
                        on `kind`) when kind is either socket kind, the
                        same field reused rather than adding a
                        second, mutually-exclusive id field */
    char filename[13];
    uint32_t offset;
} open_file_t;

static open_file_t open_files[MAX_OPEN_FILES];

/* Phase 57: guards the whole open_files[] table AND handle_read()'s
 * shared `scratch` buffer below - both are genuinely reachable from
 * two CPUs at once now (any ring-3 process on either CPU can be
 * mid-syscall at the same physical instant). Coarse-grained on
 * purpose: this table is tiny (MAX_OPEN_FILES=8) and every critical
 * section under this lock is short, plain memory work with no
 * blocking call inside it (vfs_read_file() itself is protected
 * separately by vfs.c's own vfs_lock - see that file), so one lock
 * for the whole table is the honest, simple choice rather than a
 * finer-grained per-slot scheme this phase doesn't need. */
static spinlock_t open_files_lock;

/* Phase 36: kernel/rust/pipe.rs's exported functions - see that
 * file's own doc comments for the full contract of each. Declared
 * once here since (unlike kernel/init/main.c's single-call-site
 * self-test) every one of these is used from more than one function
 * below. */
extern int rust_pipe_create(void);
extern int rust_pipe_read(int id, uint8_t* buf, uint32_t max_len);
extern int rust_pipe_write(int id, const uint8_t* buf, uint32_t len);
extern void rust_pipe_close(int id, int is_read_end);

/* Phase 55: kernel/rust/acpi.rs's own real ACPI shutdown entry point -
 * see that module's own doc comment and syscall.h's own SYS_SHUTDOWN
 * comment for the full contract. */
extern int rust_acpi_shutdown(void);

/* Phase 58: kernel/rust/tcp.rs's own exported functions - see that
 * file's own doc comments for the full contract of each. */
extern int rust_tcp_socket(void);
extern int rust_tcp_bind(int id, uint16_t port);
extern int rust_tcp_listen(int id, int backlog);
extern int rust_tcp_accept(int id);
extern int rust_tcp_connect(int id, uint32_t remote_ip, uint16_t remote_port);
extern int rust_tcp_send(int id, const uint8_t* buf, uint32_t len);
extern int rust_tcp_recv(int id, uint8_t* buf, uint32_t max_len);
extern void rust_tcp_close(int id);

/* Phase 78: kernel/rust/udp.rs's own exported functions - see that
 * file's own doc comments for the full contract of each. A separate
 * table/id space from the rust_tcp_* ones above - see OPEN_KIND_UDP_
 * SOCKET's own comment on why a handle's `kind` is what disambiguates
 * which of the two a given `pipe_id` actually indexes into. */
extern int rust_udp_socket(void);
extern int rust_udp_bind(int id, uint16_t port);
extern int rust_udp_connect(int id, uint32_t remote_ip, uint16_t remote_port);
extern int rust_udp_send(int id, const uint8_t* buf, uint32_t len);
extern int rust_udp_recv(int id, uint8_t* buf, uint32_t max_len);
extern int rust_udp_sendto(int id, const uint8_t* buf, uint32_t len,
                            uint32_t dest_ip, uint16_t dest_port);
extern int rust_udp_recvfrom(int id, uint8_t* buf, uint32_t max_len,
                              uint32_t* out_src_ip, uint16_t* out_src_port);
extern void rust_udp_close(int id);

static int str_eq_ci(const char* a, const char* b) {
    while (*a && *b) {
        char ca = (*a >= 'A' && *a <= 'Z') ? (char)(*a + 32) : *a;
        char cb = (*b >= 'A' && *b <= 'Z') ? (char)(*b + 32) : *b;
        if (ca != cb) {
            return 0;
        }
        a++;
        b++;
    }
    return *a == '\0' && *b == '\0';
}

static bool process_has_capability(const process_t* p, const char* filename) {
    if (p->can_open_any_file) {
        return true; /* Phase 30 - see the field's comment in process.h */
    }
    for (int i = 0; i < p->allowed_file_count; i++) {
        if (str_eq_ci(p->allowed_files[i], filename)) {
            return true;
        }
    }
    return false;
}

/* Phase 14: same idea as process_has_capability() above, for the
 * network host capability list instead of filenames. */
static bool process_has_host_capability(const process_t* p, uint32_t ip) {
    for (int i = 0; i < p->allowed_host_count; i++) {
        if (p->allowed_hosts[i] == ip) {
            return true;
        }
    }
    return false;
}

void syscall_init(void) {
    /* 0xEE = present, ring 3 DPL, 32-bit interrupt gate. The DPL is
     * what actually matters here: the CPU checks CPL <= gate DPL for
     * a software interrupt raised via INT, so a ring-3-only gate
     * (DPL=0, like every other IDT entry) would take a #GP fault the
     * instant user code tried `int 0x80` - this is the one interrupt
     * vector deliberately opened up to ring 3. */
    idt_set_gate(SYSCALL_VECTOR, (uint32_t)isr128, GDT_KERNEL_CODE, 0xEE);
    spinlock_init(&open_files_lock);
}

static void handle_open(registers_t* regs) {
    process_t* p = process_current();
    const char* filename = (const char*)regs->ebx;

    if (p == NULL || !process_has_capability(p, filename)) {
        kernel_log("[SECURITY] pid %d denied SYS_OPEN('%s') - not in its "
                   "capability list\n", p != NULL ? p->pid : -1, filename);
        regs->eax = (uint32_t)-1;
        return;
    }

    uint32_t flags = spinlock_acquire(&open_files_lock);

    int slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!open_files[i].in_use) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        spinlock_release(&open_files_lock, flags);
        kernel_log("[FAULT] SYS_OPEN: open file table full\n");
        regs->eax = (uint32_t)-1;
        return;
    }

    /* Claimed immediately, still under the lock, so a second CPU
     * calling handle_open() at the same physical instant can never
     * see this same slot as free too - the same scan-then-claim race
     * process.c's allocate_slot() closes for process_table[]. */
    open_files[slot].in_use = true;
    open_files[slot].owner_pid = p->pid;
    open_files[slot].kind = OPEN_KIND_VFS;
    open_files[slot].offset = 0;
    size_t i = 0;
    while (filename[i] && i < sizeof(open_files[slot].filename) - 1) {
        open_files[slot].filename[i] = filename[i];
        i++;
    }
    open_files[slot].filename[i] = '\0';

    spinlock_release(&open_files_lock, flags);

    kernel_log("[SYSCALL] pid %d SYS_OPEN('%s') -> handle %d (capability "
               "granted)\n", p->pid, filename, slot);
    regs->eax = (uint32_t)slot;
}

/* Phase 57: shared across every caller of handle_read() below -
 * genuinely reachable from two CPUs at once now, so it needs the same
 * open_files_lock that protects the table itself, held across the
 * whole read-then-slice sequence (not just the table lookup): two
 * processes calling SYS_READ at the same physical instant, unlocked,
 * could interleave their vfs_read_file() calls into this one buffer
 * and each copy out a slice of the OTHER process's file. */
static uint8_t read_scratch[4096];

static void handle_read(registers_t* regs) {
    process_t* p = process_current();
    int handle = (int)regs->ebx;
    void* buf = (void*)regs->ecx;
    uint32_t max_len = regs->edx;

    uint32_t flags = spinlock_acquire(&open_files_lock);

    if (handle < 0 || handle >= MAX_OPEN_FILES || !open_files[handle].in_use ||
        open_files[handle].owner_pid != (p != NULL ? p->pid : -1)) {
        spinlock_release(&open_files_lock, flags);
        kernel_log("[SECURITY] pid %d SYS_READ with an invalid or "
                   "not-owned handle %d\n", p != NULL ? p->pid : -1, handle);
        regs->eax = (uint32_t)-1;
        return;
    }

    if (open_files[handle].kind == OPEN_KIND_PIPE_READ) {
        /* Phase 36: the pipe ring-buffer logic itself lives entirely
         * in kernel/rust/pipe.rs - this is just the dispatch. See
         * that file's own doc comment for rust_pipe_read()'s full
         * return-value contract (in particular, -2 means "would
         * block", not an error - SYS_READ passes it through to the
         * caller unchanged, the same way it already passes through
         * -1 for a real error). rust_pipe_read() has its own
         * independent locking (see pipe.rs) - open_files_lock only
         * needs to protect the handle-table lookup that got us here,
         * not the pipe itself, so it's released before calling out. */
        int pipe_id = open_files[handle].pipe_id;
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)rust_pipe_read(pipe_id, (uint8_t*)buf, max_len);
        return;
    }

    if (open_files[handle].kind == OPEN_KIND_SOCKET) {
        /* Phase 58: same reasoning as the pipe-read branch just above -
         * rust_tcp_recv() has its own independent locking (see
         * kernel/rust/tcp.rs), so open_files_lock is released first;
         * its return-value contract is deliberately identical to
         * rust_pipe_read()'s (see that function's own doc comment),
         * which is exactly what lets SYS_READ dispatch to either kind
         * through this one shared path. */
        int conn_id = open_files[handle].pipe_id;
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)rust_tcp_recv(conn_id, (uint8_t*)buf, max_len);
        return;
    }

    if (open_files[handle].kind == OPEN_KIND_UDP_SOCKET) {
        /* Phase 78: only meaningful for a connected UDP socket - see
         * rust_udp_recv()'s own doc comment, which returns -1 (not a
         * crash or a hang) for an unconnected one, exactly the "real,
         * honest failure" EDESTADDRREQ-style contract a plain SYS_READ
         * on an unconnected UDP socket should have. */
        int sock_id = open_files[handle].pipe_id;
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)rust_udp_recv(sock_id, (uint8_t*)buf, max_len);
        return;
    }

    /* Phase 73: a real offset read. This used to re-read the whole file
     * into read_scratch on every call and slice out the requested piece
     * - which meant a file was only ever readable up to the scratch
     * buffer's 4096 bytes: every byte past that was silently
     * unreachable and reads just reported end of file. (fat32/ext2's
     * whole-file readers cap at buf_size without saying so.) Now the
     * filesystem is asked for exactly [offset, offset + want) and
     * nothing else, so a file of any size can be read to the end.
     *
     * One call returns at most sizeof(read_scratch) bytes - a short read
     * is always legal for read(), and every caller in the tree already
     * loops until it gets 0 (cat.c, cp.rs, the ring-3 shell, libc's
     * fopen()). The bounce buffer is kept (rather than reading straight
     * into the caller's buffer) for the same reason it always existed:
     * the copy into user memory happens under open_files_lock either
     * way, and one shared kernel buffer keeps that copy a plain
     * memcpy() of data the filesystem has already finished producing.
     *
     * A file that does not exist reports -1 (the first read on a lazily
     * opened handle is where that is discovered - see handle_open());
     * end of file is 0. max_len == 0 is therefore a valid, cheap
     * existence probe. */
    uint32_t want = (max_len < sizeof(read_scratch)) ? max_len
                                                     : (uint32_t)sizeof(read_scratch);
    int got = vfs_read_file_range(open_files[handle].filename,
                                  open_files[handle].offset, read_scratch,
                                  want);
    if (got < 0) {
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)-1;
        return;
    }

    memcpy(buf, read_scratch, (uint32_t)got);
    open_files[handle].offset += (uint32_t)got;

    spinlock_release(&open_files_lock, flags);
    regs->eax = (uint32_t)got;
}

static void handle_write_handle(registers_t* regs) {
    process_t* p = process_current();
    int handle = (int)regs->ebx;
    const void* buf = (const void*)regs->ecx;
    uint32_t len = regs->edx;

    uint32_t flags = spinlock_acquire(&open_files_lock);

    if (handle < 0 || handle >= MAX_OPEN_FILES || !open_files[handle].in_use ||
        open_files[handle].owner_pid != (p != NULL ? p->pid : -1)) {
        spinlock_release(&open_files_lock, flags);
        kernel_log("[SECURITY] pid %d SYS_WRITE_HANDLE with an invalid or "
                   "not-owned handle %d\n", p != NULL ? p->pid : -1, handle);
        regs->eax = (uint32_t)-1;
        return;
    }

    if (open_files[handle].kind == OPEN_KIND_SOCKET) {
        /* Phase 58: same reasoning as handle_read()'s socket branch -
         * rust_tcp_send() has its own independent locking, and its
         * return-value contract (bytes actually accepted, possibly
         * less than `len` - or -1) is deliberately identical to
         * rust_pipe_write()'s. */
        int conn_id = open_files[handle].pipe_id;
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)rust_tcp_send(conn_id, (const uint8_t*)buf, len);
        return;
    }

    if (open_files[handle].kind == OPEN_KIND_UDP_SOCKET) {
        /* Phase 78: only meaningful for a connected UDP socket - see
         * rust_udp_send()'s own doc comment (-1, the real
         * EDESTADDRREQ-style failure, for an unconnected one). */
        int sock_id = open_files[handle].pipe_id;
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)rust_udp_send(sock_id, (const uint8_t*)buf, len);
        return;
    }

    if (open_files[handle].kind != OPEN_KIND_PIPE_WRITE) {
        /* Explicit, honest scope limit for this phase - see this
         * syscall's own comment in syscall.h. */
        spinlock_release(&open_files_lock, flags);
        regs->eax = (uint32_t)-1;
        return;
    }

    int pipe_id = open_files[handle].pipe_id;
    spinlock_release(&open_files_lock, flags);

    /* rust_pipe_write() has its own independent locking (see
     * pipe.rs) - released open_files_lock before calling out, same
     * reasoning as handle_read()'s pipe-read path above. */
    regs->eax = (uint32_t)rust_pipe_write(pipe_id, (const uint8_t*)buf, len);
}

/* Phase 47: SYS_LOGIN/SYS_GETUID - see syscall.h's own comment on each
 * for the full argument/return contract. */
static void handle_login(registers_t* regs) {
    process_t* p = process_current();
    const char* username = (const char*)regs->ebx;
    const char* password = (const char*)regs->ecx;

    if (p == NULL) {
        regs->eax = (uint32_t)-1;
        return;
    }

    bool ok = process_login(p, username, password);
    if (ok) {
        kernel_log("[SYSCALL] pid %d SYS_LOGIN('%s') -> success, now uid "
                   "%d gid %d\n", p->pid, username, (int)p->uid,
                   (int)p->gid);
        regs->eax = 0;
    } else {
        kernel_log("[SECURITY] pid %d SYS_LOGIN('%s') -> rejected (wrong "
                   "password or unknown username)\n", p->pid, username);
        regs->eax = (uint32_t)-1;
    }
}

static void handle_getuid(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p != NULL) ? p->uid : 0;
}

static void handle_sudo(registers_t* regs) {
    process_t* p = process_current();
    const char* password = (const char*)regs->ebx;

    if (p == NULL) {
        regs->eax = (uint32_t)-1;
        return;
    }

    bool ok = process_sudo(p, password);
    if (ok) {
        kernel_log("[SYSCALL] pid %d SYS_SUDO -> success, now uid %d "
                   "gid %d\n", p->pid, (int)p->uid, (int)p->gid);
        regs->eax = 0;
    } else {
        kernel_log("[SECURITY] pid %d SYS_SUDO -> rejected (wrong "
                   "password or not in the admin group)\n", p->pid);
        regs->eax = (uint32_t)-1;
    }
}

static void handle_shutdown(registers_t* regs) {
    process_t* p = process_current();
    kernel_log("[SYSCALL] pid %d SYS_SHUTDOWN - attempting a real ACPI "
               "power-off\n", p ? p->pid : -1);

    int result = rust_acpi_shutdown();

    /* Reached only on failure - a real, working ACPI shutdown never
     * returns here at all (see rust_acpi_shutdown()'s own doc comment
     * for exactly what each negative value means). */
    kernel_log("[FAULT] SYS_SHUTDOWN: ACPI power-off failed (code %d) "
               "- the machine is still running\n", result);
    regs->eax = (uint32_t)result;
}

static void handle_pipe(registers_t* regs) {
    process_t* p = process_current();
    int* out = (int*)regs->ebx; /* {read_handle, write_handle} */

    if (p == NULL) {
        regs->eax = (uint32_t)-1;
        return;
    }

    uint32_t flags = spinlock_acquire(&open_files_lock);

    int read_slot = -1, write_slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!open_files[i].in_use) {
            if (read_slot < 0) {
                read_slot = i;
            } else {
                write_slot = i;
                break;
            }
        }
    }
    if (read_slot < 0 || write_slot < 0) {
        spinlock_release(&open_files_lock, flags);
        kernel_log("[FAULT] SYS_PIPE: open file table full\n");
        regs->eax = (uint32_t)-1;
        return;
    }

    /* Claim both slots immediately, still under the lock - same
     * scan-then-claim race as handle_open() above, just for two slots
     * at once. */
    open_files[read_slot].in_use = true;
    open_files[read_slot].owner_pid = p->pid;
    open_files[write_slot].in_use = true;
    open_files[write_slot].owner_pid = p->pid;

    spinlock_release(&open_files_lock, flags);

    int pipe_id = rust_pipe_create();
    if (pipe_id < 0) {
        /* Roll back the claim - rust_pipe_create() failing is rare
         * (out of pipe slots) but leaving these two marked in_use
         * forever would leak them. */
        uint32_t rollback_flags = spinlock_acquire(&open_files_lock);
        open_files[read_slot].in_use = false;
        open_files[write_slot].in_use = false;
        spinlock_release(&open_files_lock, rollback_flags);
        kernel_log("[FAULT] SYS_PIPE: no free pipe slots\n");
        regs->eax = (uint32_t)-1;
        return;
    }

    flags = spinlock_acquire(&open_files_lock);
    open_files[read_slot].kind = OPEN_KIND_PIPE_READ;
    open_files[read_slot].pipe_id = pipe_id;
    open_files[write_slot].kind = OPEN_KIND_PIPE_WRITE;
    open_files[write_slot].pipe_id = pipe_id;
    spinlock_release(&open_files_lock, flags);

    out[0] = read_slot;
    out[1] = write_slot;
    kernel_log("[SYSCALL] pid %d SYS_PIPE -> read handle %d, write handle %d "
               "(pipe %d)\n", p->pid, read_slot, write_slot, pipe_id);
    regs->eax = 0;
}

static void handle_close(registers_t* regs) {
    process_t* p = process_current();
    int handle = (int)regs->ebx;

    uint32_t flags = spinlock_acquire(&open_files_lock);

    if (handle >= 0 && handle < MAX_OPEN_FILES && open_files[handle].in_use &&
        open_files[handle].owner_pid == (p != NULL ? p->pid : -1)) {
        open_kind_t kind = open_files[handle].kind;
        int pipe_id = open_files[handle].pipe_id;
        open_files[handle].in_use = false;
        spinlock_release(&open_files_lock, flags);

        /* rust_pipe_close() has its own independent locking - called
         * after releasing open_files_lock, same reasoning as every
         * other pipe.rs call site above. The slot itself is already
         * freed above, under the lock, so no other CPU can claim this
         * handle number for a new open while this pipe teardown is
         * still in flight. */
        if (kind == OPEN_KIND_PIPE_READ) {
            rust_pipe_close(pipe_id, 1);
        } else if (kind == OPEN_KIND_PIPE_WRITE) {
            rust_pipe_close(pipe_id, 0);
        } else if (kind == OPEN_KIND_SOCKET) {
            /* Phase 58: fire-and-forget, same reasoning as the pipe
             * cases above - rust_tcp_close() has its own independent
             * locking, and this handle's own slot is already freed
             * (above, under open_files_lock) before this runs, so no
             * other CPU can reuse this handle number while the TCP-
             * level close is still finishing in the background (see
             * rust_tcp_close()'s own doc comment on why closing the
             * handle doesn't mean the connection tears down
             * instantly). */
            rust_tcp_close(pipe_id);
        } else if (kind == OPEN_KIND_UDP_SOCKET) {
            /* Phase 78: unlike TCP, nothing "finishes in the
             * background" for UDP (no connection to tear down) -
             * rust_udp_close() just frees the socket slot immediately,
             * but is still called after this handle's own open_files[]
             * slot is freed, for the same reason: simpler to reason
             * about one consistent close ordering for every socket
             * kind than a special case for the one that happens not to
             * need it. */
            rust_udp_close(pipe_id);
        }
        return;
    }

    spinlock_release(&open_files_lock, flags);
}

static void handle_read_key(registers_t* regs) {
    if (keyboard_has_char()) {
        /* Safe to call the "blocking" keyboard_get_char() here
         * specifically because we just confirmed a character is
         * already buffered - it will return immediately without ever
         * reaching its internal hlt-wait loop, which would otherwise
         * deadlock the kernel from inside an interrupts-disabled
         * syscall handler. See SYS_READ_KEY's comment in syscall.h. */
        regs->eax = (uint32_t)(int)keyboard_get_char();
    } else {
        regs->eax = (uint32_t)-1;
    }
}

/* Phase 30: staging state for handle_list_files() below -
 * vfs_list_files() is callback-based (kernel-internal style), but a
 * syscall needs to fill a caller-provided buffer instead - this
 * accumulates each entry into a static buffer the callback appends
 * to, then the syscall handler copies the result out. Safe as plain
 * (non-reentrant) static state: syscalls execute one at a time with
 * interrupts disabled, so nothing else can be mid-listing
 * concurrently. */
static char list_files_staging[2048];
static int list_files_staging_len;

static void list_files_callback(const char* name, uint32_t size,
                                 bool is_directory) {
    (void)is_directory;
    int remaining = (int)sizeof(list_files_staging) - list_files_staging_len;
    if (remaining <= 0) {
        return; /* buffer already full - drop any further entries
                    rather than overflow */
    }
    int written = snprintf(list_files_staging + list_files_staging_len,
                            (size_t)remaining, "%s %d\n", name, (int)size);
    if (written > 0) {
        list_files_staging_len += written;
    }
}

static void handle_list_files(registers_t* regs) {
    char* buf = (char*)regs->ebx;
    int buf_size = (int)regs->ecx;

    if (!vfs_is_mounted()) {
        regs->eax = (uint32_t)-1;
        return;
    }

    list_files_staging_len = 0;
    vfs_list_files(list_files_callback);

    if (list_files_staging_len >= buf_size) {
        regs->eax = (uint32_t)-1; /* caller's buffer too small */
        return;
    }

    memcpy(buf, list_files_staging, (size_t)list_files_staging_len);
    regs->eax = (uint32_t)list_files_staging_len;
}

/* Phase 31: restores shell command parity after Phase 30's ring-3
 * conversion, one syscall at a time. Each of these three reads
 * already-safe-to-call kernel state with no capability gate needed -
 * the same "no gate needed" reasoning SYS_WRITE/SYS_YIELD/SYS_SBRK
 * already use. */

static void handle_rtc_read(registers_t* regs) {
    rtc_time_t* out = (rtc_time_t*)regs->ebx;
    rtc_read(out);
    regs->eax = 0;
}

/* Same staging-accumulator pattern as list_files_callback() above,
 * for pci_enumerate()'s callback interface instead of
 * vfs_list_files()'s. */
static char lspci_staging[2048];
static int lspci_staging_len;

static void lspci_callback(const pci_device_t* dev) {
    int remaining = (int)sizeof(lspci_staging) - lspci_staging_len;
    if (remaining <= 0) {
        return;
    }
    int written = snprintf(
        lspci_staging + lspci_staging_len, (size_t)remaining,
        "%d:%d.%d %x:%x %s\n", dev->bus, dev->device, dev->function,
        dev->vendor_id, dev->device_id,
        pci_class_name(dev->class_code, dev->subclass));
    if (written > 0) {
        lspci_staging_len += written;
    }
}

static void handle_lspci(registers_t* regs) {
    char* buf = (char*)regs->ebx;
    int buf_size = (int)regs->ecx;

    lspci_staging_len = 0;
    pci_enumerate(lspci_callback);

    if (lspci_staging_len >= buf_size) {
        regs->eax = (uint32_t)-1;
        return;
    }

    memcpy(buf, lspci_staging, (size_t)lspci_staging_len);
    regs->eax = (uint32_t)lspci_staging_len;
}

static void handle_beep(registers_t* regs) {
    if (!ac97_is_present()) {
        regs->eax = 0;
        return;
    }
    ac97_beep();
    regs->eax = 1;
}

static void handle_write_file(registers_t* regs) {
    process_t* p = process_current();
    const char* filename = (const char*)regs->ebx;
    const void* data = (const void*)regs->ecx;
    uint32_t size = regs->edx;

    if (p == NULL || !p->can_open_any_file) {
        kernel_log("[SECURITY] pid %d denied SYS_WRITE_FILE('%s') - "
                   "no broad file access capability\n",
                   p != NULL ? p->pid : -1, filename);
        regs->eax = (uint32_t)-1;
        return;
    }

    regs->eax = vfs_write_file(filename, data, size) ? 1u : (uint32_t)-1;
}

static void handle_delete_file(registers_t* regs) {
    process_t* p = process_current();
    const char* filename = (const char*)regs->ebx;

    if (p == NULL || !p->can_open_any_file) {
        kernel_log("[SECURITY] pid %d denied SYS_DELETE_FILE('%s') - "
                   "no broad file access capability\n",
                   p != NULL ? p->pid : -1, filename);
        regs->eax = (uint32_t)-1;
        return;
    }

    regs->eax = vfs_delete_file(filename) ? 1u : (uint32_t)-1;
}

/* Phase 81: while a process owns the display through SYS_FB_ACQUIRE,
 * the legacy SYS_GFX_* calls do nothing (they return void, so there is
 * no error to give). Without this, a program still using the old API
 * could switch the display out from under a new-API client or draw
 * 320x200 palette pixels over its picture. */
static void handle_gfx_enter(registers_t* regs) {
    (void)regs;
    if (fb_display_is_owned()) {
        return;
    }
    vga_graphics_enter();
}

static void handle_gfx_exit(registers_t* regs) {
    (void)regs;
    if (fb_display_is_owned()) {
        return;
    }
    vga_graphics_exit();
    vga_clear();
}

static void handle_gfx_put_pixel(registers_t* regs) {
    if (fb_display_is_owned()) {
        return;
    }
    int x = (int)regs->ebx;
    int y = (int)regs->ecx;
    uint8_t color = (uint8_t)regs->edx;
    vga_put_pixel(x, y, color);
}

static void handle_gfx_fill_rect(registers_t* regs) {
    if (fb_display_is_owned()) {
        return;
    }
    /* {x, y, w, h, color} as five consecutive ints in the caller's
     * buffer - see SYS_GFX_FILL_RECT's comment in syscall.h for why
     * a buffer instead of more register arguments.
     *
     * Phase 81: the pointer is now validated first. It used to be
     * dereferenced as-is, and since any kernel-mode page fault is a
     * kernel panic here (isr.c/paging.c), a ring-3 program passing a
     * bad pointer to this call could take the whole machine down.
     * The legacy call returns void, so a bad pointer is simply
     * ignored. (Other older syscalls share the same unvalidated-
     * pointer pattern; this one is fixed because it belongs to the
     * graphics family this phase replaces - the rest is outside its
     * scope.) */
    if (!paging_user_range_ok(regs->ebx, 5 * sizeof(int), false)) {
        return;
    }
    int params[5];
    memcpy(params, (const void*)regs->ebx, sizeof params);
    vga_fill_rect(params[0], params[1], params[2], params[3],
                  (uint8_t)params[4]);
}

static void handle_mouse_read(registers_t* regs) {
    if (!ps2mouse_is_present()) {
        regs->eax = 0;
        return;
    }
    /* Phase 81: validated, for the same reason handle_gfx_fill_rect()
     * now is - this is the other half of the graphics input path
     * every graphics program uses, and it wrote through the raw user
     * pointer. A bad pointer reports "no mouse data" (0). */
    if (!paging_user_range_ok(regs->ebx, sizeof(mouse_state_t), true)) {
        regs->eax = 0;
        return;
    }
    mouse_state_t state = ps2mouse_read();
    memcpy((void*)regs->ebx, &state, sizeof state);
    regs->eax = 1;
}

/* Phase 81: the SYS_FB_* handlers. Deliberately thin - all the policy
 * (ownership, validation, clipping, backends) lives in kernel/drivers/
 * video/fb.c; these only supply who is asking (pid, page directory)
 * and the raw argument. No capability gate, for the same reason the
 * SYS_GFX_* calls have none: drawing to the screen reads nothing
 * sensitive, and exclusive ownership (SYS_FB_ACQUIRE -> -EBUSY) already
 * stops one program from interfering with another's display. */
static void handle_fb_info(registers_t* regs) {
    regs->eax = (uint32_t)fb_sys_info(regs->ebx);
}

static void handle_fb_acquire(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL) ? (uint32_t)-NOVA_FB_ERR_PERM
                            : (uint32_t)fb_sys_acquire(p->pid, regs->ebx);
}

static void handle_fb_release(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL) ? (uint32_t)-NOVA_FB_ERR_PERM
                            : (uint32_t)fb_sys_release(p->pid);
}

static void handle_fb_create(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL || !p->is_user)
                    ? (uint32_t)-NOVA_FB_ERR_PERM
                    : (uint32_t)fb_sys_create(
                          p->pid, (uint32_t*)p->page_directory_phys,
                          regs->ebx);
}

static void handle_fb_destroy(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL || !p->is_user)
                    ? (uint32_t)-NOVA_FB_ERR_PERM
                    : (uint32_t)fb_sys_destroy(
                          p->pid, (uint32_t*)p->page_directory_phys,
                          regs->ebx);
}

static void handle_fb_present(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL) ? (uint32_t)-NOVA_FB_ERR_PERM
                            : (uint32_t)fb_sys_present(p->pid, regs->ebx);
}

/* Phase 83: shared memory. Same shape as the SYS_FB_* handlers: the
 * calling process's pid and the raw argument go to kernel/ipc/shm.c, which
 * validates every user pointer; a kernel-mode task (no page-table-backed
 * user memory to share) gets -EPERM. No capability gate: what a process may
 * map is decided by the per-object ACL the owner sets (SYS_SHM_GRANT), not
 * by a per-process permission, and the quotas bound what one process can
 * hold. */
#define SHM_HANDLER(NAME, CALL)                                              \
    static void NAME(registers_t* regs) {                                    \
        process_t* p = process_current();                                    \
        regs->eax = (p == NULL || !p->is_user)                               \
                        ? (uint32_t)-NOVA_SHM_ERR_PERM                       \
                        : (uint32_t)CALL(p->pid, regs->ebx);                 \
    }

SHM_HANDLER(handle_shm_create, shm_sys_create)
SHM_HANDLER(handle_shm_grant, shm_sys_grant)
SHM_HANDLER(handle_shm_map, shm_sys_map)
SHM_HANDLER(handle_shm_unmap, shm_sys_unmap)
SHM_HANDLER(handle_shm_destroy, shm_sys_destroy)
SHM_HANDLER(handle_shm_info, shm_sys_info)

/* Phase 84: messaging. Same shape as the SHM handlers above. No capability
 * gate: a process can only be messaged if it opened an inbox, and what the
 * inbox accepts is the receiver's own policy (MSG_OPEN / MSG_CTL), so the
 * per-process capability lists (files, hosts, spawn) - which guard system
 * resources - have nothing to add. The sender's pid and uid come from the
 * process table in msg_sys_send(), never from the request. */
#define MSG_HANDLER(NAME, CALL)                                              \
    static void NAME(registers_t* regs) {                                    \
        process_t* p = process_current();                                    \
        regs->eax = (p == NULL || !p->is_user)                               \
                        ? (uint32_t)-NOVA_MSG_ERR_PERM                       \
                        : (uint32_t)CALL(p->pid, regs->ebx);                 \
    }

MSG_HANDLER(handle_msg_open, msg_sys_open)
MSG_HANDLER(handle_msg_close, msg_sys_close)
MSG_HANDLER(handle_msg_send, msg_sys_send)
MSG_HANDLER(handle_msg_recv, msg_sys_recv)
MSG_HANDLER(handle_msg_service, msg_sys_service)
MSG_HANDLER(handle_msg_ctl, msg_sys_ctl)

/* Phase 85: audio. Same shape as the handlers above. No capability gate: a
 * stream belongs to its opener and nothing about it is visible to anyone
 * else; the two controls that affect or expose EVERYONE's sound (the master
 * volume and the tap) are gated on the caller's uid inside audio_sys_ctl(),
 * using the kernel's own record of it. */
#define AUDIO_HANDLER(NAME, CALL)                                            \
    static void NAME(registers_t* regs) {                                    \
        process_t* p = process_current();                                    \
        regs->eax = (p == NULL || !p->is_user)                               \
                        ? (uint32_t)-NOVA_AUDIO_ERR_PERM                     \
                        : (uint32_t)CALL(p->pid, regs->ebx);                 \
    }

AUDIO_HANDLER(handle_audio_open, audio_sys_open)
AUDIO_HANDLER(handle_audio_write, audio_sys_write)
AUDIO_HANDLER(handle_audio_ctl, audio_sys_ctl)
AUDIO_HANDLER(handle_audio_close, audio_sys_close)

/* Phase 86: resource limits. Any process may call it for itself (tightening
 * only, unless root); touching another process is root-only - all decided in
 * rlimit_sys() from the kernel's own record of the caller's uid. */
static void handle_rlimit(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL || !p->is_user)
                    ? (uint32_t)-NOVA_RLIMIT_ERR_PERM
                    : (uint32_t)rlimit_sys(p->pid, regs->ebx);
}

static void handle_fb_readback(registers_t* regs) {
    process_t* p = process_current();
    regs->eax = (p == NULL) ? (uint32_t)-NOVA_FB_ERR_PERM
                            : (uint32_t)fb_sys_readback(p->pid, regs->ebx);
}

static void handle_ping_start(registers_t* regs) {
    uint32_t dest_ip = regs->ebx;
    icmp_ping_start(dest_ip);
}

static void handle_ping_poll(registers_t* regs) {
    uint32_t* out_rtt = (uint32_t*)regs->ebx;
    regs->eax = (uint32_t)icmp_ping_poll(out_rtt);
}

/* Phase 14's SYS_NET_SEND: the same capability-gate-then-act pattern
 * as handle_open() above, just for a network destination instead of a
 * filename. Uses a fixed source port for this demo syscall rather
 * than allocating a real ephemeral one per call - fine for "prove the
 * capability check works," not meant to be a general sockets API (see
 * PROGRESS.md). */
#define SYS_NET_SEND_SOURCE_PORT 51000

static void handle_net_send(registers_t* regs) {
    process_t* p = process_current();
    uint32_t dest_ip = regs->ebx;
    uint16_t dest_port = (uint16_t)regs->ecx;
    const char* message = (const char*)regs->edx;

    if (p == NULL || !process_has_host_capability(p, dest_ip)) {
        kernel_log("[SECURITY] pid %d denied SYS_NET_SEND to %d.%d.%d.%d - "
                   "not in its capability list\n",
                   p != NULL ? p->pid : -1, (int)(dest_ip >> 24) & 0xFF,
                   (int)(dest_ip >> 16) & 0xFF, (int)(dest_ip >> 8) & 0xFF,
                   (int)dest_ip & 0xFF);
        regs->eax = (uint32_t)-1;
        return;
    }

    bool ok = udp_send(dest_ip, SYS_NET_SEND_SOURCE_PORT, dest_port, message,
                        (uint16_t)strlen(message));
    kernel_log("[SYSCALL] pid %d SYS_NET_SEND to %d.%d.%d.%d:%d (capability "
               "granted) -> %s\n", p->pid, (int)(dest_ip >> 24) & 0xFF,
               (int)(dest_ip >> 16) & 0xFF, (int)(dest_ip >> 8) & 0xFF,
               (int)dest_ip & 0xFF, (int)dest_port, ok ? "sent" : "failed");
    regs->eax = ok ? 0 : (uint32_t)-1;
}

/* Phase 17's SYS_SPAWN: the same capability-gate-then-act pattern as
 * handle_open()/handle_net_send() above, for process creation instead
 * of a filename or a network destination. process_create_user_task()
 * (not process_create_sandboxed_task()) is used for the new process
 * deliberately - a spawned process starts with no capabilities of its
 * own; the ability to spawn does not imply the ability to grant
 * capabilities to what's spawned; see PROGRESS.md. */
static void handle_spawn(registers_t* regs) {
    process_t* p = process_current();

    if (p == NULL || !p->can_spawn) {
        kernel_log("[SECURITY] pid %d denied SYS_SPAWN - spawn capability "
                   "not granted\n", p != NULL ? p->pid : -1);
        regs->eax = (uint32_t)-1;
        return;
    }

    int new_pid = process_create_user_task("spawned", greeter_task);
    kernel_log("[SYSCALL] pid %d SYS_SPAWN (capability granted) -> new "
               "pid %d\n", p->pid, new_pid);
    regs->eax = (uint32_t)new_pid;
}

/* Phase 23's SYS_EXEC: reuses can_spawn (Phase 17) rather than adding
 * a fourth capability type - the honest framing is that "may create
 * processes" is one capability whether the new process runs the
 * fixed greeter task or a real loaded ELF, not two different
 * privileges. */
static void handle_exec(registers_t* regs) {
    process_t* p = process_current();

    if (p == NULL || !p->can_spawn) {
        kernel_log("[SECURITY] pid %d denied SYS_EXEC - spawn capability "
                   "not granted\n", p != NULL ? p->pid : -1);
        regs->eax = (uint32_t)-1;
        return;
    }

    const char* path = (const char*)regs->ebx;
    const char** argv = (const char**)regs->ecx;
    int argc = (int)regs->edx;

    int new_pid = process_exec(path, argv, argc);
    kernel_log("[SYSCALL] pid %d SYS_EXEC('%s') (capability granted) -> "
               "new pid %d\n", p->pid, path, new_pid);
    regs->eax = (uint32_t)new_pid;
}

/* Phase 59's SYS_EXEC_TRUSTED - see kernel/arch/x86/cpu/syscall.h's own
 * comment on this syscall for the full reasoning. Gated by can_spawn,
 * exactly like plain SYS_EXEC above (a process still needs the basic
 * "may create processes" capability either way) - the difference this
 * syscall adds is entirely inside process_exec_trusted() itself, which
 * reads the calling process's own can_open_any_file and passes that
 * same value to the child, rather than always false. */
static void handle_exec_trusted(registers_t* regs) {
    process_t* p = process_current();

    if (p == NULL || !p->can_spawn) {
        kernel_log("[SECURITY] pid %d denied SYS_EXEC_TRUSTED - spawn "
                   "capability not granted\n", p != NULL ? p->pid : -1);
        regs->eax = (uint32_t)-1;
        return;
    }

    const char* path = (const char*)regs->ebx;
    const char** argv = (const char**)regs->ecx;
    int argc = (int)regs->edx;

    int new_pid = process_exec_trusted(path, argv, argc);
    kernel_log("[SYSCALL] pid %d SYS_EXEC_TRUSTED('%s') (can_open_any_file=%d "
               "delegated) -> new pid %d\n", p->pid, path,
               (int)p->can_open_any_file, new_pid);
    regs->eax = (uint32_t)new_pid;
}

/* Phase 73: counts the entries of a caller-supplied, NULL-terminated
 * envp array, never looking at more than MAX_EXEC_ENV + 1 slots. Returns
 * the count, or -1 if the array is longer than MAX_EXEC_ENV (which
 * process_exec_internal() would refuse anyway - rejecting here just
 * avoids walking further into caller memory to find out). A NULL envp
 * is a valid, empty environment. */
static int count_envp(const char** envp) {
    if (envp == NULL) {
        return 0;
    }
    int n = 0;
    while (envp[n] != NULL) {
        n++;
        if (n > MAX_EXEC_ENV) {
            return -1;
        }
    }
    return n;
}

/* Phase 73's SYS_EXEC_ENV - see syscall.h's own comment on this syscall
 * for the full contract and why it is a separate number from SYS_EXEC.
 * Gated by can_spawn, granting the new process nothing - exactly like
 * handle_exec() above, plus an environment. */
static void handle_exec_env(registers_t* regs) {
    process_t* p = process_current();

    if (p == NULL || !p->can_spawn) {
        kernel_log("[SECURITY] pid %d denied SYS_EXEC_ENV - spawn capability "
                   "not granted\n", p != NULL ? p->pid : -1);
        regs->eax = (uint32_t)-1;
        return;
    }

    const char* path = (const char*)regs->ebx;
    const char** argv = (const char**)regs->ecx;
    int argc = (int)regs->edx;
    const char** envp = (const char**)regs->esi;

    int envc = count_envp(envp);
    if (envc < 0) {
        kernel_log("[WARN] pid %d SYS_EXEC_ENV('%s'): environment has more "
                   "than %d entries\n", p->pid, path, MAX_EXEC_ENV);
        regs->eax = (uint32_t)-1;
        return;
    }

    int new_pid = process_exec_env(path, argv, argc, envp, envc);
    kernel_log("[SYSCALL] pid %d SYS_EXEC_ENV('%s', %d env) (capability "
               "granted) -> new pid %d\n", p->pid, path, envc, new_pid);
    regs->eax = (uint32_t)new_pid;
}

/* Phase 73's SYS_EXEC_TRUSTED_ENV - handle_exec_trusted() plus an
 * environment; the delegation logic itself lives in
 * process_exec_trusted_env(), unchanged from process_exec_trusted(). */
static void handle_exec_trusted_env(registers_t* regs) {
    process_t* p = process_current();

    if (p == NULL || !p->can_spawn) {
        kernel_log("[SECURITY] pid %d denied SYS_EXEC_TRUSTED_ENV - spawn "
                   "capability not granted\n", p != NULL ? p->pid : -1);
        regs->eax = (uint32_t)-1;
        return;
    }

    const char* path = (const char*)regs->ebx;
    const char** argv = (const char**)regs->ecx;
    int argc = (int)regs->edx;
    const char** envp = (const char**)regs->esi;

    int envc = count_envp(envp);
    if (envc < 0) {
        kernel_log("[WARN] pid %d SYS_EXEC_TRUSTED_ENV('%s'): environment "
                   "has more than %d entries\n", p->pid, path, MAX_EXEC_ENV);
        regs->eax = (uint32_t)-1;
        return;
    }

    int new_pid = process_exec_trusted_env(path, argv, argc, envp, envc);
    kernel_log("[SYSCALL] pid %d SYS_EXEC_TRUSTED_ENV('%s', %d env) "
               "(can_open_any_file=%d delegated) -> new pid %d\n", p->pid,
               path, envc, (int)p->can_open_any_file, new_pid);
    regs->eax = (uint32_t)new_pid;
}

/* Phase 60's SYS_DNS_RESOLVE - see syscall.h's own comment on this
 * syscall for the full reasoning (why it's ungated, why dns_resolve()
 * itself needed a Phase-58-style scheduler_yield() fix first). */
static void handle_dns_resolve(registers_t* regs) {
    const char* hostname = (const char*)regs->ebx;
    uint32_t* out_ip = (uint32_t*)regs->ecx;

    bool ok = dns_resolve(hostname, NET_DNS_SERVER_IP, out_ip);
    kernel_log("[SYSCALL] SYS_DNS_RESOLVE('%s') -> %s\n", hostname,
               ok ? "ok" : "failed");
    regs->eax = ok ? 1u : (uint32_t)-1;
}

/* Phase 60's SYS_TFTP_FETCH - see syscall.h's own comment on this
 * syscall for the full reasoning. TFTP_FETCH_MAX_BYTES bounds the
 * kernel-side staging buffer tftp_get() reads into before this
 * function hands the whole thing to vfs_write_file() in one call -
 * 256KB, the same bound userland/coreutils-rs/cp.rs already uses for
 * its own local-copy buffer (see that file's own comment on why this
 * kernel's "no incremental file write" contract makes a fixed
 * whole-transfer buffer the only shape available either way), not a
 * new, separately-chosen number. */
#define TFTP_FETCH_MAX_BYTES (256 * 1024)
static uint8_t tftp_fetch_staging[TFTP_FETCH_MAX_BYTES];

static void handle_tftp_fetch(registers_t* regs) {
    process_t* p = process_current();
    uint32_t server_ip = regs->ebx;
    const char* remote_filename = (const char*)regs->ecx;
    const char* local_filename = (const char*)regs->edx;

    if (p == NULL || !p->can_open_any_file) {
        kernel_log("[SECURITY] pid %d denied SYS_TFTP_FETCH('%s' -> '%s') - "
                   "no broad file access capability\n",
                   p != NULL ? p->pid : -1, remote_filename, local_filename);
        regs->eax = (uint32_t)-1;
        return;
    }

    int n = tftp_get(server_ip, remote_filename, tftp_fetch_staging,
                      sizeof(tftp_fetch_staging));
    if (n < 0) {
        kernel_log("[SYSCALL] pid %d SYS_TFTP_FETCH('%s' -> '%s') failed "
                   "(timeout, TFTP error, or too large for the %d-byte "
                   "staging buffer)\n", p->pid, remote_filename,
                   local_filename, TFTP_FETCH_MAX_BYTES);
        regs->eax = (uint32_t)-1;
        return;
    }

    bool wrote = vfs_write_file(local_filename, tftp_fetch_staging,
                                 (uint32_t)n);
    kernel_log("[SYSCALL] pid %d SYS_TFTP_FETCH('%s' -> '%s') fetched %d "
               "bytes, write %s\n", p->pid, remote_filename, local_filename,
               n, wrote ? "ok" : "failed");
    regs->eax = wrote ? (uint32_t)n : (uint32_t)-1;
}

static void handle_wait(registers_t* regs) {
    int target_pid = (int)regs->ebx;
    int result = process_wait(target_pid);
    regs->eax = (uint32_t)result;
}

/* Phase 71: SYS_WAIT_NONBLOCK's own handler - see that syscall's own
 * doc comment (kernel/arch/x86/cpu/syscall.h) and process_wait_
 * nonblock()'s own (kernel/task/process.h) for the full contract.
 * Deliberately writes the exit code out through a caller-provided
 * pointer (ecx) rather than returning it directly, unlike handle_
 * wait() above - a real exit code can itself be negative (a program
 * is free to `return -1;`), so folding "still running" / "doesn't
 * exist" / "terminated with code N" into a single signed return value
 * the way handle_wait() does would make a genuinely negative exit
 * code indistinguishable from one of those other states. The pointer
 * itself is trusted exactly as every other pointer-output syscall in
 * this file already is (handle_mouse_read() above, no different) -
 * not a new trust boundary. */
static void handle_wait_nonblock(registers_t* regs) {
    int pid = (int)regs->ebx;
    int* out_exit_code = (int*)regs->ecx;
    bool exists = false;
    int exit_code = 0;
    bool terminated = process_wait_nonblock(pid, &exit_code, &exists);
    if (terminated) {
        *out_exit_code = exit_code;
        regs->eax = 0;
    } else if (exists) {
        regs->eax = (uint32_t)-1; /* exists, still running */
    } else {
        regs->eax = (uint32_t)-2; /* no such process */
    }
}

static void handle_sbrk(registers_t* regs) {
    process_t* p = process_current();
    int increment = (int)regs->ebx;
    regs->eax = process_sbrk(p, increment);
}

static void handle_fork(registers_t* regs) {
    int child_pid = process_fork(regs);
    /* The parent's own return value (the child's pid, or -1 on
     * failure) - the child's return value (0) was already baked into
     * its own copy of these same registers by process_fork() itself,
     * on the child's own kernel stack, not this line. */
    regs->eax = (uint32_t)child_pid;
}

/* Phase 58: the Berkeley-sockets-style syscall API - see syscall.h's
 * own comment on each syscall number for the full contract. A socket
 * is just another open_files[] entry (OPEN_KIND_SOCKET, this table's
 * pipe_id field reused as the connection id - see open_file_t's own
 * comment), the same pattern pipes already established, so recv()/
 * send()/close() are already handled above by handle_read()/
 * handle_write_handle()/handle_close() and don't need their own
 * syscalls here. */

static void handle_socket(registers_t* regs) {
    process_t* p = process_current();
    if (p == NULL) {
        regs->eax = (uint32_t)-1;
        return;
    }

    int conn_id = rust_tcp_socket();
    if (conn_id < 0) {
        kernel_log("[FAULT] SYS_SOCKET: no free TCP connection slots\n");
        regs->eax = (uint32_t)-1;
        return;
    }

    uint32_t flags = spinlock_acquire(&open_files_lock);
    int slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!open_files[i].in_use) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        spinlock_release(&open_files_lock, flags);
        rust_tcp_close(conn_id);
        kernel_log("[FAULT] SYS_SOCKET: open file table full\n");
        regs->eax = (uint32_t)-1;
        return;
    }
    open_files[slot].in_use = true;
    open_files[slot].owner_pid = p->pid;
    open_files[slot].kind = OPEN_KIND_SOCKET;
    open_files[slot].pipe_id = conn_id;
    spinlock_release(&open_files_lock, flags);

    kernel_log("[SYSCALL] pid %d SYS_SOCKET -> handle %d (tcp conn %d)\n",
               p->pid, slot, conn_id);
    regs->eax = (uint32_t)slot;
}

/* Phase 78: SYS_SOCKET_UDP - identical shape to handle_socket() above,
 * creating a kernel/rust/udp.rs socket (OPEN_KIND_UDP_SOCKET) instead
 * of a TCP one. Kept as its own function rather than folding a branch
 * into handle_socket() itself: the two have no shared logic worth
 * factoring out beyond the open_files[] slot-claiming loop (which
 * handle_fork()/handle_pipe() above also each repeat their own copy
 * of, this project's existing convention for this particular bit of
 * duplication rather than a shared helper). */
static void handle_socket_udp(registers_t* regs) {
    process_t* p = process_current();
    if (p == NULL) {
        regs->eax = (uint32_t)-1;
        return;
    }

    int sock_id = rust_udp_socket();
    if (sock_id < 0) {
        kernel_log("[FAULT] SYS_SOCKET_UDP: no free UDP socket slots\n");
        regs->eax = (uint32_t)-1;
        return;
    }

    uint32_t flags = spinlock_acquire(&open_files_lock);
    int slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!open_files[i].in_use) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        spinlock_release(&open_files_lock, flags);
        rust_udp_close(sock_id);
        kernel_log("[FAULT] SYS_SOCKET_UDP: open file table full\n");
        regs->eax = (uint32_t)-1;
        return;
    }
    open_files[slot].in_use = true;
    open_files[slot].owner_pid = p->pid;
    open_files[slot].kind = OPEN_KIND_UDP_SOCKET;
    open_files[slot].pipe_id = sock_id;
    spinlock_release(&open_files_lock, flags);

    kernel_log("[SYSCALL] pid %d SYS_SOCKET_UDP -> handle %d (udp sock %d)\n",
               p->pid, slot, sock_id);
    regs->eax = (uint32_t)slot;
}

/* Shared validation for SYS_BIND/SYS_LISTEN/SYS_ACCEPT/SYS_CONNECT/
 * SYS_SENDTO/SYS_RECVFROM: confirms `handle` is a currently-open,
 * calling-process-owned socket handle of EITHER kind (Phase 78 -
 * originally TCP-only) and returns its underlying connection/socket
 * id, filling in *out_kind so the caller knows which of kernel/rust/
 * tcp.rs or kernel/rust/udp.rs that id actually indexes into, or -1
 * (and logs, the same [SECURITY] pattern every other handle-
 * validating syscall in this file already uses) if not. Always
 * releases open_files_lock itself before returning - callers never see
 * it held. */
static int lookup_socket_conn_id(int handle, open_kind_t* out_kind) {
    process_t* p = process_current();
    uint32_t flags = spinlock_acquire(&open_files_lock);
    if (handle < 0 || handle >= MAX_OPEN_FILES || !open_files[handle].in_use ||
        open_files[handle].owner_pid != (p != NULL ? p->pid : -1) ||
        (open_files[handle].kind != OPEN_KIND_SOCKET &&
         open_files[handle].kind != OPEN_KIND_UDP_SOCKET)) {
        spinlock_release(&open_files_lock, flags);
        kernel_log("[SECURITY] pid %d used an invalid/not-owned/non-socket "
                   "handle %d for a socket syscall\n",
                   p != NULL ? p->pid : -1, handle);
        return -1;
    }
    int conn_id = open_files[handle].pipe_id;
    *out_kind = open_files[handle].kind;
    spinlock_release(&open_files_lock, flags);
    return conn_id;
}

static void handle_bind(registers_t* regs) {
    open_kind_t kind;
    int conn_id = lookup_socket_conn_id((int)regs->ebx, &kind);
    if (conn_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }
    if (kind == OPEN_KIND_UDP_SOCKET) {
        regs->eax = (uint32_t)rust_udp_bind(conn_id, (uint16_t)regs->ecx);
    } else {
        regs->eax = (uint32_t)rust_tcp_bind(conn_id, (uint16_t)regs->ecx);
    }
}

static void handle_listen(registers_t* regs) {
    open_kind_t kind;
    int conn_id = lookup_socket_conn_id((int)regs->ebx, &kind);
    if (conn_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }
    if (kind != OPEN_KIND_SOCKET) {
        /* Phase 78: UDP is connectionless - there is nothing to
         * listen for a connection ON. A real, explicit failure
         * (ENOTSUP-equivalent), not silently treated as a no-op
         * success. */
        process_t* p = process_current();
        kernel_log("[FAULT] pid %d SYS_LISTEN on a UDP socket (handle %d) "
                   "- not supported, UDP has no connections to listen for\n",
                   p != NULL ? p->pid : -1, (int)regs->ebx);
        regs->eax = (uint32_t)-1;
        return;
    }
    regs->eax = (uint32_t)rust_tcp_listen(conn_id, (int)regs->ecx);
}

static void handle_connect(registers_t* regs) {
    open_kind_t kind;
    int conn_id = lookup_socket_conn_id((int)regs->ebx, &kind);
    if (conn_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }
    process_t* p = process_current();
    uint32_t dest_ip = regs->ecx;
    uint16_t dest_port = (uint16_t)regs->edx;
    if (kind == OPEN_KIND_UDP_SOCKET) {
        /* Phase 78: records a fixed peer - no handshake, no capability
         * gate (see kernel/rust/udp.rs's own top comment for exactly
         * why, mirroring SYS_CONNECT's own already-documented,
         * deliberate scope cut for TCP below rather than introducing
         * a new inconsistency between the two). */
        kernel_log("[SYSCALL] pid %d SYS_CONNECT (UDP) handle %d -> "
                   "%d.%d.%d.%d:%d\n", p != NULL ? p->pid : -1,
                   (int)regs->ebx, (int)(dest_ip >> 24) & 0xFF,
                   (int)(dest_ip >> 16) & 0xFF, (int)(dest_ip >> 8) & 0xFF,
                   (int)dest_ip & 0xFF, (int)dest_port);
        regs->eax = (uint32_t)rust_udp_connect(conn_id, dest_ip, dest_port);
        return;
    }
    kernel_log("[SYSCALL] pid %d SYS_CONNECT handle %d -> %d.%d.%d.%d:%d\n",
               p != NULL ? p->pid : -1, (int)regs->ebx,
               (int)(dest_ip >> 24) & 0xFF, (int)(dest_ip >> 16) & 0xFF,
               (int)(dest_ip >> 8) & 0xFF, (int)dest_ip & 0xFF,
               (int)dest_port);
    regs->eax = (uint32_t)rust_tcp_connect(conn_id, dest_ip, dest_port);
}

/* Blocking (see rust_tcp_accept()'s own doc comment) - deliberately
 * called with open_files_lock already released (lookup_socket_conn_id()
 * above releases it before returning), the same "never hold a lock
 * across a blocking/yielding call" discipline Phase 57's own audit
 * established for every other lock in this kernel. */
static void handle_accept(registers_t* regs) {
    open_kind_t kind;
    int conn_id = lookup_socket_conn_id((int)regs->ebx, &kind);
    if (conn_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }
    if (kind != OPEN_KIND_SOCKET) {
        /* Phase 78: same reasoning as handle_listen() above - UDP has
         * no connections to accept. */
        process_t* p = process_current();
        kernel_log("[FAULT] pid %d SYS_ACCEPT on a UDP socket (handle %d) "
                   "- not supported\n", p != NULL ? p->pid : -1,
                   (int)regs->ebx);
        regs->eax = (uint32_t)-1;
        return;
    }

    int new_conn_id = rust_tcp_accept(conn_id);
    if (new_conn_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }

    process_t* p = process_current();
    uint32_t flags = spinlock_acquire(&open_files_lock);
    int slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!open_files[i].in_use) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        spinlock_release(&open_files_lock, flags);
        rust_tcp_close(new_conn_id);
        kernel_log("[FAULT] SYS_ACCEPT: open file table full\n");
        regs->eax = (uint32_t)-1;
        return;
    }
    open_files[slot].in_use = true;
    open_files[slot].owner_pid = p != NULL ? p->pid : -1;
    open_files[slot].kind = OPEN_KIND_SOCKET;
    open_files[slot].pipe_id = new_conn_id;
    spinlock_release(&open_files_lock, flags);

    kernel_log("[SYSCALL] pid %d SYS_ACCEPT -> handle %d (tcp conn %d)\n",
               p != NULL ? p->pid : -1, slot, new_conn_id);
    regs->eax = (uint32_t)slot;
}

/* Phase 78: the exact 6-byte layout SYS_SENDTO/SYS_RECVFROM's own
 * address-pointer arguments use - see syscall.h's own comment on each
 * for the full contract. Packed (no padding) since this is read/
 * written directly at a ring-3-supplied pointer, byte-for-byte - not
 * a struct either side merely happens to agree on, but the literal
 * wire contract between this kernel and userland/libc/include/
 * novasys.h's own identical nova_udp_addr_t, kept in sync by comment
 * and convention rather than a shared header, the same as every other
 * small FFI-boundary struct in this project (see e.g. kernel/rust/
 * dynlink.rs's own structs mirroring kernel/task/elf.c's). */
typedef struct __attribute__((packed)) {
    uint32_t ip;
    uint16_t port;
} nova_udp_addr_t;

static void handle_sendto(registers_t* regs) {
    open_kind_t kind;
    int sock_id = lookup_socket_conn_id((int)regs->ebx, &kind);
    if (sock_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }
    process_t* p = process_current();
    if (kind != OPEN_KIND_UDP_SOCKET) {
        kernel_log("[FAULT] pid %d SYS_SENDTO on a TCP socket (handle %d) "
                   "- not supported, use SYS_WRITE_HANDLE instead\n",
                   p != NULL ? p->pid : -1, (int)regs->ebx);
        regs->eax = (uint32_t)-1;
        return;
    }
    const nova_udp_addr_t* addr = (const nova_udp_addr_t*)regs->ecx;
    const uint8_t* buf = (const uint8_t*)regs->edx;
    uint32_t len = regs->esi;
    uint32_t dest_ip = addr->ip;
    uint16_t dest_port = addr->port;
    int sent = rust_udp_sendto(sock_id, buf, len, dest_ip, dest_port);
    kernel_log("[SYSCALL] pid %d SYS_SENDTO handle %d -> %d.%d.%d.%d:%d "
               "(%d bytes) -> %s\n", p != NULL ? p->pid : -1, (int)regs->ebx,
               (int)(dest_ip >> 24) & 0xFF, (int)(dest_ip >> 16) & 0xFF,
               (int)(dest_ip >> 8) & 0xFF, (int)dest_ip & 0xFF,
               (int)dest_port, (int)len, sent >= 0 ? "sent" : "failed");
    regs->eax = (uint32_t)sent;
}

static void handle_recvfrom(registers_t* regs) {
    open_kind_t kind;
    int sock_id = lookup_socket_conn_id((int)regs->ebx, &kind);
    if (sock_id < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }
    if (kind != OPEN_KIND_UDP_SOCKET) {
        process_t* p = process_current();
        kernel_log("[FAULT] pid %d SYS_RECVFROM on a TCP socket (handle %d) "
                   "- not supported, use SYS_READ instead\n",
                   p != NULL ? p->pid : -1, (int)regs->ebx);
        regs->eax = (uint32_t)-1;
        return;
    }
    uint8_t* buf = (uint8_t*)regs->ecx;
    uint32_t max_len = regs->edx;
    nova_udp_addr_t* out_addr = (nova_udp_addr_t*)regs->esi;
    uint32_t src_ip = 0;
    uint16_t src_port = 0;
    int n = rust_udp_recvfrom(sock_id, buf, max_len, &src_ip, &src_port);
    if (n > 0 && out_addr != NULL) {
        out_addr->ip = src_ip;
        out_addr->port = src_port;
    }
    regs->eax = (uint32_t)n;
}

void syscall_handler(registers_t* regs) {
    switch (regs->eax) {
        case SYS_WRITE: {
            const char* str = (const char*)regs->ebx;
            vga_puts(str);
            process_t* p = process_current();
            kernel_log("[SYSCALL] SYS_WRITE from pid %d ('%s')\n",
                       p != NULL ? p->pid : -1, str);
            break;
        }

        case SYS_EXIT:
            process_exit_current((int)regs->ebx); /* never returns */
            break;

        case SYS_YIELD:
            scheduler_yield();
            break;

        case SYS_OPEN:
            handle_open(regs);
            break;

        case SYS_READ:
            handle_read(regs);
            break;

        case SYS_CLOSE:
            handle_close(regs);
            break;

        case SYS_NET_SEND:
            handle_net_send(regs);
            break;

        case SYS_SPAWN:
            handle_spawn(regs);
            break;

        case SYS_EXEC:
            handle_exec(regs);
            break;

        case SYS_WAIT:
            handle_wait(regs);
            break;

        case SYS_SBRK:
            handle_sbrk(regs);
            break;

        case SYS_FORK:
            handle_fork(regs);
            break;

        case SYS_READ_KEY:
            handle_read_key(regs);
            break;

        case SYS_LIST_FILES:
            handle_list_files(regs);
            break;

        case SYS_RTC_READ:
            handle_rtc_read(regs);
            break;

        case SYS_LSPCI:
            handle_lspci(regs);
            break;

        case SYS_BEEP:
            handle_beep(regs);
            break;

        case SYS_WRITE_FILE:
            handle_write_file(regs);
            break;

        case SYS_DELETE_FILE:
            handle_delete_file(regs);
            break;

        case SYS_GFX_ENTER:
            handle_gfx_enter(regs);
            break;

        case SYS_GFX_EXIT:
            handle_gfx_exit(regs);
            break;

        case SYS_GFX_PUT_PIXEL:
            handle_gfx_put_pixel(regs);
            break;

        case SYS_GFX_FILL_RECT:
            handle_gfx_fill_rect(regs);
            break;

        case SYS_MOUSE_READ:
            handle_mouse_read(regs);
            break;

        case SYS_PING_START:
            handle_ping_start(regs);
            break;

        case SYS_PING_POLL:
            handle_ping_poll(regs);
            break;

        case SYS_PIPE:
            handle_pipe(regs);
            break;

        case SYS_WRITE_HANDLE:
            handle_write_handle(regs);
            break;

        case SYS_LOGIN:
            handle_login(regs);
            break;

        case SYS_GETUID:
            handle_getuid(regs);
            break;

        case SYS_SUDO:
            handle_sudo(regs);
            break;

        case SYS_SHUTDOWN:
            handle_shutdown(regs);
            break;

        case SYS_SOCKET:
            handle_socket(regs);
            break;

        case SYS_BIND:
            handle_bind(regs);
            break;

        case SYS_LISTEN:
            handle_listen(regs);
            break;

        case SYS_ACCEPT:
            handle_accept(regs);
            break;

        case SYS_CONNECT:
            handle_connect(regs);
            break;

        case SYS_EXEC_TRUSTED:
            handle_exec_trusted(regs);
            break;

        case SYS_DNS_RESOLVE:
            handle_dns_resolve(regs);
            break;

        case SYS_TFTP_FETCH:
            handle_tftp_fetch(regs);
            break;

        case SYS_WAIT_NONBLOCK:
            handle_wait_nonblock(regs);
            break;

        case SYS_EXEC_ENV:
            handle_exec_env(regs);
            break;

        case SYS_EXEC_TRUSTED_ENV:
            handle_exec_trusted_env(regs);
            break;

        case SYS_SOCKET_UDP:
            handle_socket_udp(regs);
            break;

        case SYS_SENDTO:
            handle_sendto(regs);
            break;

        case SYS_RECVFROM:
            handle_recvfrom(regs);
            break;

        case SYS_SHM_CREATE:
            handle_shm_create(regs);
            break;

        case SYS_SHM_GRANT:
            handle_shm_grant(regs);
            break;

        case SYS_SHM_MAP:
            handle_shm_map(regs);
            break;

        case SYS_SHM_UNMAP:
            handle_shm_unmap(regs);
            break;

        case SYS_SHM_DESTROY:
            handle_shm_destroy(regs);
            break;

        case SYS_SHM_INFO:
            handle_shm_info(regs);
            break;

        case SYS_MSG_OPEN:
            handle_msg_open(regs);
            break;

        case SYS_MSG_CLOSE:
            handle_msg_close(regs);
            break;

        case SYS_MSG_SEND:
            handle_msg_send(regs);
            break;

        case SYS_MSG_RECV:
            handle_msg_recv(regs);
            break;

        case SYS_MSG_SERVICE:
            handle_msg_service(regs);
            break;

        case SYS_MSG_CTL:
            handle_msg_ctl(regs);
            break;

        case SYS_AUDIO_OPEN:
            handle_audio_open(regs);
            break;

        case SYS_AUDIO_WRITE:
            handle_audio_write(regs);
            break;

        case SYS_AUDIO_CTL:
            handle_audio_ctl(regs);
            break;

        case SYS_AUDIO_CLOSE:
            handle_audio_close(regs);
            break;

        case SYS_RLIMIT:
            handle_rlimit(regs);
            break;

        case SYS_FB_INFO:
            handle_fb_info(regs);
            break;

        case SYS_FB_ACQUIRE:
            handle_fb_acquire(regs);
            break;

        case SYS_FB_RELEASE:
            handle_fb_release(regs);
            break;

        case SYS_FB_CREATE:
            handle_fb_create(regs);
            break;

        case SYS_FB_DESTROY:
            handle_fb_destroy(regs);
            break;

        case SYS_FB_PRESENT:
            handle_fb_present(regs);
            break;

        case SYS_FB_READBACK:
            handle_fb_readback(regs);
            break;

        default:
            kernel_log("[WARN] Unknown syscall number %d\n", (int)regs->eax);
            break;
    }
}
