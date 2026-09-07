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
#include "../../include/kernel.h"
#include "../../drivers/video/vga_graphics.h"
#include "../../drivers/mouse/ps2mouse.h"
#include "../../net/icmp.h"

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

static void handle_gfx_enter(registers_t* regs) {
    (void)regs;
    vga_graphics_enter();
}

static void handle_gfx_exit(registers_t* regs) {
    (void)regs;
    vga_graphics_exit();
    vga_clear();
}

static void handle_gfx_put_pixel(registers_t* regs) {
    int x = (int)regs->ebx;
    int y = (int)regs->ecx;
    uint8_t color = (uint8_t)regs->edx;
    vga_put_pixel(x, y, color);
}

static void handle_gfx_fill_rect(registers_t* regs) {
    /* {x, y, w, h, color} as five consecutive ints in the caller's
     * buffer - see SYS_GFX_FILL_RECT's comment in syscall.h for why
     * a buffer instead of more register arguments. */
    int* params = (int*)regs->ebx;
    vga_fill_rect(params[0], params[1], params[2], params[3],
                  (uint8_t)params[4]);
}

static void handle_mouse_read(registers_t* regs) {
    if (!ps2mouse_is_present()) {
        regs->eax = 0;
        return;
    }
    mouse_state_t* out = (mouse_state_t*)regs->ebx;
    *out = ps2mouse_read();
    regs->eax = 1;
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

static void handle_wait(registers_t* regs) {
    int target_pid = (int)regs->ebx;
    int result = process_wait(target_pid);
    regs->eax = (uint32_t)result;
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

        default:
            kernel_log("[WARN] Unknown syscall number %d\n", (int)regs->eax);
            break;
    }
}
