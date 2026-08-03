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
#include "../../net/udp.h"
#include "../../task/process.h"
#include "../../task/scheduler.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"

extern void isr128(void);

#define MAX_OPEN_FILES 8

typedef struct {
    bool in_use;
    int owner_pid; /* only the process that opened a handle may use it -
                       guessing/reusing another process's handle number
                       is not a way around the capability check that
                       already happened at SYS_OPEN time */
    char filename[13];
    uint32_t offset;
} open_file_t;

static open_file_t open_files[MAX_OPEN_FILES];

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

    int slot = -1;
    for (int i = 0; i < MAX_OPEN_FILES; i++) {
        if (!open_files[i].in_use) {
            slot = i;
            break;
        }
    }
    if (slot < 0) {
        kernel_log("[FAULT] SYS_OPEN: open file table full\n");
        regs->eax = (uint32_t)-1;
        return;
    }

    open_files[slot].in_use = true;
    open_files[slot].owner_pid = p->pid;
    open_files[slot].offset = 0;
    size_t i = 0;
    while (filename[i] && i < sizeof(open_files[slot].filename) - 1) {
        open_files[slot].filename[i] = filename[i];
        i++;
    }
    open_files[slot].filename[i] = '\0';

    kernel_log("[SYSCALL] pid %d SYS_OPEN('%s') -> handle %d (capability "
               "granted)\n", p->pid, filename, slot);
    regs->eax = (uint32_t)slot;
}

static void handle_read(registers_t* regs) {
    process_t* p = process_current();
    int handle = (int)regs->ebx;
    void* buf = (void*)regs->ecx;
    uint32_t max_len = regs->edx;

    if (handle < 0 || handle >= MAX_OPEN_FILES || !open_files[handle].in_use ||
        open_files[handle].owner_pid != (p != NULL ? p->pid : -1)) {
        kernel_log("[SECURITY] pid %d SYS_READ with an invalid or "
                   "not-owned handle %d\n", p != NULL ? p->pid : -1, handle);
        regs->eax = (uint32_t)-1;
        return;
    }

    /* No real per-handle buffering - just re-reads the whole file (up
     * to a fixed scratch size) on every call and slices out whatever
     * the current offset/max_len asks for. Fine for the small demo
     * files this is exercised against; a real implementation would
     * want the underlying vfs_read_file() to support an offset
     * directly instead of always reading from the start. */
    static uint8_t scratch[4096];
    int total = vfs_read_file(open_files[handle].filename, scratch,
                               sizeof(scratch));
    if (total < 0) {
        regs->eax = (uint32_t)-1;
        return;
    }

    uint32_t remaining = ((uint32_t)total > open_files[handle].offset)
                              ? (uint32_t)total - open_files[handle].offset
                              : 0;
    uint32_t to_copy = (remaining < max_len) ? remaining : max_len;
    memcpy(buf, scratch + open_files[handle].offset, to_copy);
    open_files[handle].offset += to_copy;

    regs->eax = to_copy;
}

static void handle_close(registers_t* regs) {
    process_t* p = process_current();
    int handle = (int)regs->ebx;

    if (handle >= 0 && handle < MAX_OPEN_FILES && open_files[handle].in_use &&
        open_files[handle].owner_pid == (p != NULL ? p->pid : -1)) {
        open_files[handle].in_use = false;
    }
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
            process_exit_current(); /* never returns */
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

        default:
            kernel_log("[WARN] Unknown syscall number %d\n", (int)regs->eax);
            break;
    }
}
