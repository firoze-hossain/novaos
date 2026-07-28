#include "../include/kernel.h"
#include "../drivers/vga/vga.h"
#include "../drivers/serial/serial.h"
#include "../drivers/timer/timer.h"
#include "../drivers/keyboard/keyboard.h"
#include "../drivers/mouse/ps2mouse.h"
#include "../arch/x86/cpu/gdt.h"
#include "../arch/x86/cpu/idt.h"
#include "../arch/x86/cpu/tss.h"
#include "../arch/x86/cpu/syscall.h"
#include "../arch/x86/mm/heap.h"
#include "../arch/x86/mm/pmm.h"
#include "../arch/x86/mm/paging.h"
#include "../arch/x86/boot/multiboot.h"
#include "../fs/vfs.h"
#include "../pkg/pkgmgr.h"
#include "../net/net.h"
#include "../net/icmp.h"
#include "../net/tftp.h"
#include "../task/process.h"
#include "../task/scheduler.h"
#include "../task/user_demo.h"
#include "../shell/shell.h"
#include "../shell/firstrun.h"
#include "../lib/string.h"
#include "../lib/stdio.h"
#include <stdarg.h>

#define TIMER_FREQUENCY_HZ 100

/* kernel_log() is the one logging function every subsystem uses. It
 * always goes to serial (visible in `make debug`, CI, and
 * scripts/test.sh's boot-log assertions) and is intentionally silent
 * on the VGA console so boot messages don't clutter the screen the
 * user actually interacts with. */
void kernel_log(const char* format, ...) {
    va_list args;
    va_start(args, format);

    char buffer[256];
    vsnprintf(buffer, sizeof(buffer), format, args);
    serial_puts(buffer);

    va_end(args);
}

void kernel_panic(const char* message) {
    __asm__ volatile ("cli");

    kernel_log("[PANIC] %s\n", message);

    vga_set_color(VGA_COLOR_RED, VGA_COLOR_BLACK);
    vga_puts("\n\n!!! KERNEL PANIC !!!\n");
    vga_puts(message);
    vga_puts("\nSystem halted.\n");

    while (1) {
        __asm__ volatile ("hlt");
    }
}

/* Everything that must happen before interrupts are safe to enable:
 * segmentation (GDT), the interrupt table itself (IDT/ISR/IRQ, which
 * also remaps and fully masks the PIC), physical memory discovery and
 * paging, and the heap, since several drivers will want to kmalloc()
 * during their own init in later phases. */
void kernel_early_init(uint32_t multiboot_magic, uint32_t multiboot_info_addr) {
    serial_init();
    kernel_log("NovaOS booting (kernel v%s)...\n", KERNEL_VERSION);

    gdt_init();
    kernel_log("[ OK ] GDT initialized\n");

    tss_init();
    kernel_log("[ OK ] TSS installed\n");

    idt_init();
    kernel_log("[ OK ] IDT/ISR/IRQ initialized, PIC remapped to 0x20-0x2F\n");

    syscall_init();
    kernel_log("[ OK ] Syscall gate installed (int 0x80, ring 3 accessible)\n");

    bool magic_valid = (multiboot_magic == MULTIBOOT_BOOTLOADER_MAGIC);
    if (!magic_valid) {
        kernel_log("[WARN] Not booted via a valid Multiboot loader "
                   "(magic=0x%x) - physical memory map unavailable\n",
                   (int)multiboot_magic);
    }
    pmm_init((const multiboot_info_t*)multiboot_info_addr, magic_valid);

    paging_init();

    heap_init();
    kernel_log("[ OK ] Heap initialized (2 MB arena)\n");
}

/* Everything that needs interrupts to exist as a concept, but not yet
 * enabled: driver registration. IF is turned on right at the end, once
 * every handler a currently-unmasked IRQ could call is already in place. */
void kernel_late_init(void) {
    timer_init(TIMER_FREQUENCY_HZ);
    kernel_log("[ OK ] PIT timer initialized at %d Hz (IRQ0)\n",
               TIMER_FREQUENCY_HZ);
    timer_set_tick_hook(scheduler_on_tick);

    keyboard_init();
    kernel_log("[ OK ] PS/2 keyboard initialized (IRQ1)\n");

    ps2mouse_init();

    vfs_init();

    net_init();

    __asm__ volatile ("sti");
    kernel_log("[ OK ] Interrupts enabled\n");

    /* Self-test: if a disk is attached with a FAT32-formatted test
     * image containing HELLO.TXT (see `make disk.img` / TESTING.md),
     * read it and log the result. This is what lets `make test` verify
     * the whole ATA -> FAT32 -> VFS chain headlessly, the same way
     * Phase 2's boot markers verified GDT/IDT/IRQ without needing a
     * keyboard attached. Silently skipped (not a failure) if no disk
     * is present, since NovaOS should still boot fine without one. */
    if (vfs_is_mounted()) {
        char file_buf[256];
        int n = vfs_read_file("HELLO.TXT", file_buf, sizeof(file_buf) - 1);
        if (n >= 0) {
            file_buf[n] = '\0';
            kernel_log("[ OK ] FILE READ OK: HELLO.TXT (%d bytes): %s\n", n,
                       file_buf);
        } else {
            kernel_log("[WARN] FAT32 mounted but HELLO.TXT not found\n");
        }
    }

    /* Self-test: if a NIC is attached, ping the gateway (QEMU user-
     * mode networking's SLIRP stack always answers pings to itself at
     * 10.0.2.2, regardless of whether the sandbox/host has real
     * outbound network access - see net.h). This exercises the full
     * NE2000 -> Ethernet -> ARP -> IP -> ICMP round trip headlessly,
     * the same pattern as the filesystem self-test above. Silently
     * skipped if no NIC is present. */
    if (net_is_up()) {
        uint32_t rtt_ticks = 0;
        if (icmp_ping(NET_GATEWAY_IP, &rtt_ticks)) {
            kernel_log("[ OK ] PING OK: gateway replied in %d ticks (~%dms)\n",
                       (int)rtt_ticks, (int)(rtt_ticks * 10));
        } else {
            kernel_log("[WARN] Gateway did not reply to ping (no route? "
                       "check QEMU -netdev config)\n");
        }

        /* Self-test: fetch a test package over TFTP from the gateway
         * (QEMU's SLIRP runs a TFTP server there when configured with
         * -netdev ...,tftp=DIR - see Makefile's NET_FLAGS and
         * tools/fixtures/tftproot/). Proves the whole
         * UDP -> TFTP -> network-fetch chain works headlessly, no
         * real network access required, the same self-contained-test
         * principle as the ping self-test just above. */
        static uint8_t tftp_buf[512];
        int tftp_n = tftp_get(NET_GATEWAY_IP, "WEATHER.PKG", tftp_buf,
                               sizeof(tftp_buf));
        if (tftp_n > 0) {
            kernel_log("[ OK ] TFTP FETCH OK: WEATHER.PKG (%d bytes)\n",
                       tftp_n);
        } else {
            kernel_log("[WARN] TFTP fetch of WEATHER.PKG failed (no TFTP "
                       "server configured? check QEMU -netdev tftp=...)\n");
        }
    }

    /* Self-test: if the demo packages are present (EDITOR.PKG, GAME.PKG
     * - see `make disk.img` / tools/fixtures), install one and read
     * back its installed payload - this is what actually proves Phase
     * 8's new FAT32 write support works end to end, not just that it
     * compiles - then remove it and confirm pkg_is_installed() agrees
     * it's gone. Silently skipped if no disk is present. */
    if (vfs_is_mounted()) {
        if (pkg_install("Editor")) {
            char installed_buf[256];
            int n = vfs_read_file("EDITOR.APP", installed_buf,
                                   sizeof(installed_buf) - 1);
            if (n > 0) {
                installed_buf[n] = '\0';
                kernel_log("[ OK ] PKG INSTALL OK: EDITOR.APP (%d bytes): %s\n",
                           n, installed_buf);
            } else {
                kernel_log("[WARN] pkg self-test: EDITOR.APP not readable "
                           "after install\n");
            }

            bool was_installed = pkg_is_installed("Editor");
            pkg_remove("Editor");
            bool still_installed = pkg_is_installed("Editor");

            if (was_installed && !still_installed) {
                kernel_log("[ OK ] PKG REMOVE OK: Editor no longer "
                           "installed\n");
            } else {
                kernel_log("[WARN] pkg self-test: remove didn't behave as "
                           "expected\n");
            }
        } else {
            kernel_log("[WARN] pkg self-test: install of 'Editor' failed\n");
        }
    }
}

static void print_banner(void) {
    vga_set_color(VGA_COLOR_CYAN, VGA_COLOR_BLACK);
    vga_puts("   _   _                  _____  _____ \n");
    vga_puts("  | \\ | |                / ____|/ ____|\n");
    vga_puts("  |  \\| | _____   _____| (___ | (___  \n");
    vga_puts("  | . ` |/ _ \\ \\ / / _ \\\\___ \\ \\___ \\ \n");
    vga_puts("  | |\\  | (_) \\ V / (_) |___) |____) |\n");
    vga_puts("  |_| \\_|\\___/ \\_/ \\___/_____/_____/ \n\n");

    vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);
    vga_printf("  " KERNEL_NAME " v" KERNEL_VERSION "\n");
    vga_printf("  Built on " __DATE__ " at " __TIME__ "\n\n");

    vga_set_color(VGA_COLOR_GREEN, VGA_COLOR_BLACK);
    vga_puts("  [OK] GDT / IDT / PIC configured\n");
    vga_puts("  [OK] PIT timer + PS/2 keyboard online\n");
    vga_puts("  [OK] Kernel heap ready\n\n");

    vga_set_color(VGA_COLOR_LIGHT_GREY, VGA_COLOR_BLACK);
    vga_puts("  Type 'help' for a list of commands\n\n");
}

static void idle_task_entry(void) {
    /* Runs whenever nothing else is READY. hlt is fine here - this is
     * a ring-0 kernel task, not the ring-3 demo. Also where the NIC
     * gets polled: the NE2000 driver has no IRQ (see PROGRESS.md), so
     * something has to regularly check it for received frames, and
     * idle is, almost by definition, the task with the most spare
     * cycles to spend doing that. */
    for (;;) {
        net_poll();
        __asm__ volatile ("hlt");
    }
}

void kernel_main(uint32_t multiboot_magic, uint32_t multiboot_info_addr) {
    kernel_early_init(multiboot_magic, multiboot_info_addr);

    vga_init();
    vga_clear();

    kernel_late_init();

    print_banner();

    firstrun_check_and_run();

    process_init();
    process_create_kernel_task("idle", idle_task_entry);
    process_create_kernel_task("shell", shell_run);
    process_create_user_task("demo-a", user_demo_task_a);
    process_create_user_task("demo-b", user_demo_task_b);
    kernel_log("[ OK ] Tasks created: idle (kernel), shell (kernel), "
               "demo-a + demo-b (ring 3, private address spaces)\n");

    scheduler_start(); /* never returns */
}
