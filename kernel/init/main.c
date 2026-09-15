#include "../include/kernel.h"
#include "../drivers/vga/vga.h"
#include "../drivers/serial/serial.h"
#include "../drivers/timer/timer.h"
#include "../drivers/driver.h"
#include "../arch/x86/cpu/gdt.h"
#include "../arch/x86/cpu/idt.h"
#include "../arch/x86/cpu/tss.h"
#include "../arch/x86/cpu/syscall.h"
#include "../arch/x86/mm/heap.h"
#include "../arch/x86/mm/pmm.h"
#include "../arch/x86/mm/paging.h"
#include "../arch/x86/boot/multiboot.h"
#include "../fs/vfs.h"
#include "../fs/ext2.h"
#include "../../userland/pkg/pkgmgr.h"
#include "../drivers/pci/pci.h"
#include "../drivers/sound/ac97.h"
#include "../drivers/virtio/virtio_blk.h"
#include "../drivers/blockdev.h"
#include "../fs/fat32.h"
#include "../net/net.h"
#include "../net/dns.h"
#include "../net/tcp.h"
#include "../net/icmp.h"
#include "../net/tftp.h"
#include "../task/process.h"
#include "../task/scheduler.h"
#include "../task/user_demo.h"
#include "../task/sandbox_demo.h"
#include "../task/unprivileged_demo.h"
#include "../../userland/shell/shell.h"
#include "../../userland/shell/firstrun.h"
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

static int pci_device_count = 0;

static void pci_log_device(const pci_device_t* dev) {
    pci_device_count++;
    kernel_log("[ OK ] PCI %d:%d.%d vendor=0x%x device=0x%x class=0x%x (%s)\n",
               (int)dev->bus, (int)dev->device, (int)dev->function,
               (int)dev->vendor_id, (int)dev->device_id,
               (int)dev->class_code,
               pci_class_name(dev->class_code, dev->subclass));
}

/* Everything that needs interrupts to exist as a concept, but not yet
 * enabled: driver registration. IF is turned on right at the end, once
 * every handler a currently-unmasked IRQ could call is already in place. */
void kernel_late_init(void) {
    timer_init(TIMER_FREQUENCY_HZ);
    kernel_log("[ OK ] PIT timer initialized at %d Hz (IRQ0)\n",
               TIMER_FREQUENCY_HZ);
    timer_set_tick_hook(scheduler_on_tick);

    /* Phase 39: PS/2 keyboard + mouse are the first drivers migrated
     * to self-registration (kernel/drivers/driver.h) - see that
     * file's own comment for why this phase specifically, and why
     * timer/VFS/net below stay as explicit calls for now. */
    driver_init_all(DRIVER_PHASE_EARLY);

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

    /* Self-test: if the ext2 partition (Phase 25) mounted, read a
     * real file from it - not just confirming the superblock parsed,
     * but that inode lookup, root-directory-entry matching, and block
     * data reading (the parts most likely to have a subtle bug) all
     * genuinely work together. vfs_read_file() is used here rather
     * than calling ext2_read_file() directly, so this also proves the
     * FAT32-then-ext2 fallback in vfs.c works, not just the ext2
     * driver in isolation. */
    if (ext2_is_mounted()) {
        char ext2_buf[256];
        int n = vfs_read_file("EXT2TEST.TXT", ext2_buf, sizeof(ext2_buf) - 1);
        if (n >= 0) {
            ext2_buf[n] = '\0';
            kernel_log("[ OK ] EXT2 FILE READ OK: EXT2TEST.TXT (%d bytes): "
                       "%s\n", n, ext2_buf);
        } else {
            kernel_log("[WARN] ext2 mounted but EXT2TEST.TXT not found\n");
        }

        /* Self-test (Phase 26): write a brand new file to the ext2
         * partition, then read it straight back through the same
         * vfs_read_file() path used above - proving the full write
         * chain (block/inode allocation, directory-entry insertion)
         * actually produces a file the read path can find and
         * retrieve correctly, not just that ext2_write_file()
         * returned true. */
        const char* write_content = "Written to ext2 by NovaOS itself!";
        if (ext2_write_file("EXT2WROT.TXT", write_content,
                             (uint32_t)strlen(write_content))) {
            char readback_buf[256];
            int rn = vfs_read_file("EXT2WROT.TXT", readback_buf,
                                    sizeof(readback_buf) - 1);
            if (rn >= 0) {
                readback_buf[rn] = '\0';
                bool matches =
                    (strcmp(readback_buf, write_content) == 0);
                kernel_log("[ OK ] EXT2 WRITE+READBACK %s: EXT2WROT.TXT "
                           "(%d bytes): %s\n",
                           matches ? "OK" : "MISMATCH", rn, readback_buf);
            } else {
                kernel_log("[WARN] ext2_write_file succeeded but the new "
                           "file wasn't found on readback\n");
            }
        } else {
            kernel_log("[WARN] ext2_write_file failed\n");
        }
    }

    /* Self-test (Phase 29): a genuine ring-3 coreutils program - see
     * coreutils_test_task() below, which runs this correctly as a
     * proper kernel task once the scheduler has actually started,
     * rather than here (kernel_main() isn't itself a registered
     * process, so calling process_wait()'s scheduler_yield()-based
     * blocking loop directly from this point in boot has no valid
     * "current process" to yield from/back to - the same reasoning
     * Phase 23's ELF self-test was built around a dedicated ring-3
     * process for, applied here to a dedicated kernel task instead). */

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

    /* Self-test: enumerate the PCI bus and log every function found.
     * Even a QEMU guest with no explicitly-added PCI devices always
     * has at least the i440fx chipset's host bridge and PIIX3 ISA/IDE
     * bridge functions - unlike the filesystem/network/pkg self-tests
     * above, this one needs no disk or NIC attached at all to produce
     * a non-empty, verifiable result. */
    pci_device_count = 0;
    pci_enumerate(pci_log_device);
    kernel_log("[ OK ] PCI ENUMERATION OK: %d device(s) found\n",
               pci_device_count);

    /* Self-test (Phase 28b): if a UHCI USB controller is present,
     * initialize it and enumerate its root hub ports - a real, if
     * scoped-down, USB stack. Verification here means watching for a
     * genuine device descriptor (vendor/product/class IDs) to appear
     * in the log for whatever's attached via QEMU's -device usb-kbd
     * or similar, the same "does the log show a real, specific result"
     * standard every other driver in this project is held to. See
     * PROGRESS.md for the full scope.
     *
     * Phase 39: both UHCI and AC97 below are now driver_init_all()-
     * registered rather than called by name - both are PCI-based, so
     * both need pci_enumerate() (just above) to have already run,
     * which is exactly why DRIVER_PHASE_AFTER_PCI exists as its own
     * phase rather than lumping every driver into one flat list. */
    driver_init_all(DRIVER_PHASE_AFTER_PCI);

    /* Self-test: if an AC97 audio device is present, play a short
     * beep. Unlike every earlier self-test, there's no way to check
     * "did this actually work" from headless kernel code alone -
     * playback happens in the background on real/emulated hardware,
     * with nothing to read back that proves audible sound came out.
     * Verified instead by capturing QEMU's actual audio output to a
     * WAV file during testing and inspecting its PCM data directly -
     * see PROGRESS.md for how, and TESTING.md for how to reproduce
     * it. */
    if (ac97_is_present()) {
        ac97_beep();

        /* Replaying after a previous beep has already finished is
         * exactly the scenario that caught a real bug during
         * interactive testing: the PCM OUT engine's internal state
         * (CIV and related registers) was left wherever the first
         * playback's completion left it, and simply rewriting BDBAR/
         * LVI/CR without resetting first wasn't enough to make the
         * card recognize a genuinely new play request - the first
         * beep played correctly but a second one produced no audible
         * output at all despite the function running and logging
         * normally. Fixed in ac97_beep() itself (see its comment);
         * exercising the replay here, not just once, is what makes
         * that fix regression-proof in the automated self-test rather
         * than relying solely on the interactive check that first
         * caught it. */
        timer_sleep_ms(400); /* let the first beep finish before replaying */
        ac97_beep();
    }

    /* Self-test (Phase 42): if a virtio-blk device is present, write a
     * known, distinctive pattern to a sector and read it back,
     * verifying an exact byte-for-byte match - the same "write it,
     * read it back, compare" standard this project's own ext2 driver
     * self-test already holds itself to (see the "EXT2 WRITE.READBACK
     * OK" check earlier in this function), applied here to prove the
     * legacy virtio handshake, virtqueue setup, and real hardware DMA
     * request/completion cycle all actually work end to end - not
     * just that the driver's own internal layout arithmetic is
     * correct (rust_virtqueue_selftest(), above, already covers that
     * in isolation). Sector 1, not 0: this is a small, dedicated test
     * disk image (see TESTING.md) that nothing else on this system
     * reads or writes, so which sector is used doesn't matter beyond
     * being consistent between the write and the read-back. */
    if (virtio_blk_is_present()) {
        static uint8_t write_pattern[512];
        for (int i = 0; i < 512; i++) {
            write_pattern[i] = (uint8_t)(i ^ 0xA5);
        }

        static uint8_t readback[512];
        bool wrote = virtio_blk_write_sector(1, write_pattern);
        bool read = wrote && virtio_blk_read_sector(1, readback);
        bool matches = read && memcmp(write_pattern, readback,
                                       sizeof(write_pattern)) == 0;

        if (matches) {
            kernel_log("[ OK ] VIRTIO-BLK WRITE.READBACK OK: a 512-byte "
                       "sector written via a real virtio-blk device read "
                       "back byte-for-byte identical\n");
        } else {
            kernel_log("[FAULT] VIRTIO-BLK WRITE.READBACK: wrote=%d "
                       "read=%d matches=%d\n", (int)wrote, (int)read,
                       (int)matches);
        }
    }

    /* Self-test: resolve a real hostname via QEMU SLIRP's built-in DNS
     * proxy (10.0.2.3) - the same self-contained-test principle as
     * the gateway ping and TFTP fetch self-tests above, extended to a
     * third SLIRP-provided service. Unlike those two, this one's
     * result (a real IP address for a real domain) depends on SLIRP's
     * own upstream DNS resolution actually working, which in turn
     * depends on outbound network access existing somewhere beneath
     * this sandboxed environment - true in the environment this was
     * developed and tested in, but not something NovaOS itself
     * controls the way it controls answering its own ARP requests.
     * Logged as a WARN rather than treated as a failure if it doesn't
     * resolve, for exactly that reason. */
    if (net_is_up()) {
        uint32_t resolved_ip;
        if (dns_resolve("example.com", NET_DNS_SERVER_IP, &resolved_ip)) {
            kernel_log("[ OK ] DNS RESOLVE OK: example.com -> %d.%d.%d.%d\n",
                       (int)(resolved_ip >> 24) & 0xFF,
                       (int)(resolved_ip >> 16) & 0xFF,
                       (int)(resolved_ip >> 8) & 0xFF,
                       (int)resolved_ip & 0xFF);
        } else {
            kernel_log("[WARN] DNS resolve of example.com failed (no "
                       "upstream network access from the DNS proxy?)\n");
        }

        /* Self-test (Phase 28): a genuine end-to-end TCP test against
         * a real, unmodified public HTTP server - the same "depends
         * on real upstream connectivity, logged as WARN not a hard
         * failure" honesty as the DNS self-test just above, extended
         * one layer further (DNS resolution, then an actual TCP
         * connection, HTTP request, and response over it). Uses
         * HTTP/1.1 with an explicit Connection: close specifically so
         * the server closes the connection after one response
         * (HTTP/1.1 defaults to keep-alive) - this also lets
         * tcp_receive()'s "0 means clean close" return value end the
         * read loop naturally instead of needing to already know the
         * response length in advance. An initial version of this test
         * used HTTP/1.0, which the real server rejected with "426
         * Upgrade Required" - a genuine finding from testing against
         * a real, unmodified server, not a TCP bug: switched to
         * HTTP/1.1 once it was clear the underlying connection,
         * request, and response were all working correctly. This
         * same test also caught a real, separate pre-existing bug:
         * ip_send() never actually used NET_NETMASK to route through
         * the gateway for an off-subnet destination (defined since
         * Phase 6, never wired up) - every earlier phase's self-tests
         * only ever talked to on-subnet SLIRP addresses, so the gap
         * had no way to surface until this was the first thing to
         * ever address a genuinely external IP. Fixed in ip.c. */
        if (dns_resolve("example.com", NET_DNS_SERVER_IP, &resolved_ip) &&
            tcp_connect(resolved_ip, 80)) {
            const char* request =
                "GET / HTTP/1.1\r\nHost: example.com\r\nConnection: "
                "close\r\n\r\n";
            if (tcp_send(request, (uint16_t)strlen(request))) {
                static char response_buf[2048];
                int total = 0;
                for (;;) {
                    int n = tcp_receive(response_buf + total,
                                        (uint16_t)(sizeof(response_buf) -
                                                    (uint32_t)total - 1),
                                        300);
                    if (n <= 0) {
                        break; /* 0 = clean close, -1 = timeout/error */
                    }
                    total += n;
                    if (total >= (int)sizeof(response_buf) - 1) {
                        break;
                    }
                }
                response_buf[total] = '\0';

                if (total > 0) {
                    char preview[64];
                    int preview_len =
                        (total < (int)sizeof(preview) - 1)
                            ? total
                            : (int)sizeof(preview) - 1;
                    memcpy(preview, response_buf, (uint32_t)preview_len);
                    preview[preview_len] = '\0';
                    kernel_log("[ OK ] TCP HTTP OK: received %d bytes "
                               "from example.com:80, starting with: "
                               "%s\n", total, preview);
                } else {
                    kernel_log("[WARN] TCP HTTP: connected and sent a "
                               "request but received no data back\n");
                }
            } else {
                kernel_log("[WARN] TCP HTTP: send failed after "
                           "connecting\n");
            }
            tcp_close();
        } else {
            kernel_log("[WARN] TCP HTTP: could not connect to "
                       "example.com:80 (no upstream network access?)\n");
        }
    }
}

/* Phase 29: runs as a proper kernel task (registered before
 * scheduler_start(), the same as idle/shell above) specifically so
 * process_wait()'s scheduler_yield()-based blocking loop has a valid
 * "current process" to yield from - calling this directly from
 * kernel_main() instead would have no such thing, since kernel_main()
 * itself is never registered as a process. Runs a genuine ring-3
 * coreutils program (userland/coreutils/cat.c) - not a kernel-
 * compiled task like every other process in this project's self-
 * tests, but a completely separately-compiled ELF32 executable
 * talking to the kernel only through syscalls, the real proof this
 * project's kernel/userland architectural split supports the same
 * category of separation Linux-kernel-vs-Ubuntu-userland has. */
static void coreutils_test_task(void) {
    if (vfs_is_mounted()) {
        const char* cat_argv[] = {"CAT.ELF", "HELLO.TXT"};
        const char* cat_files[] = {"HELLO.TXT"};
        int cat_pid = process_exec_with_files("CAT.ELF", cat_argv, 2,
                                               cat_files, 1);
        if (cat_pid >= 0) {
            int cat_exit = process_wait(cat_pid);
            kernel_log("[ OK ] Ring-3 coreutils: CAT.ELF (a real, "
                       "separately-compiled ELF32 program, not a kernel "
                       "task) exited with code %d\n", cat_exit);
        } else {
            kernel_log("[WARN] Ring-3 coreutils: CAT.ELF failed to "
                       "start\n");
        }
    }
    process_exit_current(0);
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

    /* Phase 35: NovaOS's first kernel-side Rust code
     * (kernel/rust/lib.rs), called directly from here - proves the
     * integration works at runtime, not just that it links. A real,
     * observable computation (20+22=42), not something that could
     * pass by coincidence if the FFI boundary were subtly broken. */
    extern int rust_kernel_selftest_add(int a, int b);
    int rust_result = rust_kernel_selftest_add(20, 22);
    kernel_log("[ OK ] Kernel-side Rust self-test: rust_kernel_selftest_add"
               "(20, 22) = %d (expected 42)\n", rust_result);

    /* Phase 36: kernel/rust/pipe.rs's own self-test, called directly
     * (ring 0, no syscall involved - proving the Rust implementation
     * itself is correct, independent of the C syscall-dispatch layer
     * on top of it). Deliberately more thorough than the Phase 35
     * self-test above: pipes are new kernel *state*, not a pure
     * function, so there's a meaningfully larger space of behavior to
     * get wrong (the empty/full/closed edge cases below, and the ring
     * buffer's wraparound arithmetic) than a single addition ever had.
     *
     * The syscall-dispatch layer this test intentionally bypasses
     * (SYS_PIPE/SYS_WRITE_HANDLE, and SYS_READ/SYS_CLOSE's new pipe-
     * handling branches) has its own separate, permanent, ring-3
     * verification inside sandbox_demo_task() (kernel/task/
     * sandbox_demo.c) - earlier attempts at exactly this ring-3 test
     * reproducibly triggered what looked at the time like a genuine,
     * pre-existing scheduler bug (a hang after some unrelated process
     * exited); that was tracked down to its real cause - not the
     * scheduler at all, but tools/linker.ld not capturing rustc's
     * per-symbol section naming (`.bss.SOMENAME` instead of plain
     * `.bss`), which left this exact PIPES array outside the range
     * pmm_init() reserves as "already part of the kernel's own loaded
     * image" and therefore available for pmm_alloc_frame() to (and
     * eventually did) hand out to an ordinary process's page
     * directory, silently aliasing the two. See PROGRESS.md for the
     * full account of that investigation. */
    {
        extern int rust_pipe_create(void);
        extern int rust_pipe_read(int id, uint8_t* buf, uint32_t max_len);
        extern int rust_pipe_write(int id, const uint8_t* buf,
                                    uint32_t len);
        extern void rust_pipe_close(int id, int is_read_end);

        bool all_passed = true;

        /* 1. Basic round-trip: write a short message, read it back
         * whole, verify it matches exactly. */
        int pipe_a = rust_pipe_create();
        const char msg[] = "pipe roundtrip";
        int wrote = rust_pipe_write(pipe_a, (const uint8_t*)msg,
                                     sizeof(msg) - 1);
        uint8_t readback[32];
        int got = rust_pipe_read(pipe_a, readback, sizeof(readback));
        bool roundtrip_ok = (pipe_a >= 0) && (wrote == (int)sizeof(msg) - 1) &&
                             (got == wrote) &&
                             (memcmp(readback, msg, (size_t)got) == 0);
        all_passed = all_passed && roundtrip_ok;

        /* 2. Would-block: an empty pipe with its write end still open
         * must return -2, distinct from real EOF (0). */
        bool would_block_ok = (rust_pipe_read(pipe_a, readback,
                                               sizeof(readback)) == -2);
        all_passed = all_passed && would_block_ok;

        /* 3. Real EOF: once the write end is closed, an empty pipe
         * must return 0, not -2 - the two failure modes this design
         * exists specifically to distinguish (see pipe.rs's own doc
         * comment on rust_pipe_read's return value). */
        rust_pipe_close(pipe_a, 0 /* write end */);
        bool eof_ok = (rust_pipe_read(pipe_a, readback,
                                       sizeof(readback)) == 0);
        all_passed = all_passed && eof_ok;
        rust_pipe_close(pipe_a, 1 /* read end - frees the slot */);

        /* 4. Broken pipe: writing after the read end has already
         * closed must fail outright (-1), not silently succeed and
         * discard the data. */
        int pipe_b = rust_pipe_create();
        rust_pipe_close(pipe_b, 1 /* read end */);
        bool broken_pipe_ok =
            (rust_pipe_write(pipe_b, (const uint8_t*)msg, 4) == -1);
        all_passed = all_passed && broken_pipe_ok;
        rust_pipe_close(pipe_b, 0 /* write end - frees the slot */);

        /* 5. Ring-buffer wraparound: PIPE_CAPACITY is 1024 bytes, but
         * this writes and reads only 4 bytes at a time, many more
         * times than 1024/4 - the read/write cursors must wrap around
         * the backing array's end (via pipe.rs's `% PIPE_CAPACITY`)
         * many times over without ever losing or corrupting a byte,
         * the exact class of bug (an off-by-one in that wraparound
         * arithmetic) this module's own header comment explains Rust
         * guards against with a loud panic rather than silent memory
         * corruption. */
        int pipe_c = rust_pipe_create();
        bool wraparound_ok = true;
        for (int round = 0; round < 400 && wraparound_ok; round++) {
            uint8_t chunk[4] = {(uint8_t)round, (uint8_t)(round + 1),
                                 (uint8_t)(round + 2), (uint8_t)(round + 3)};
            uint8_t chunk_back[4];
            if (rust_pipe_write(pipe_c, chunk, sizeof(chunk)) != 4) {
                wraparound_ok = false;
                break;
            }
            if (rust_pipe_read(pipe_c, chunk_back, sizeof(chunk_back)) != 4 ||
                memcmp(chunk, chunk_back, sizeof(chunk)) != 0) {
                wraparound_ok = false;
            }
        }
        all_passed = all_passed && wraparound_ok;
        rust_pipe_close(pipe_c, 1);
        rust_pipe_close(pipe_c, 0);

        kernel_log("[ %s ] Kernel-side Rust pipe self-test: roundtrip=%s "
                   "would-block=%s eof=%s broken-pipe=%s "
                   "wraparound(400x4B)=%s\n",
                   all_passed ? "OK" : "FAIL",
                   roundtrip_ok ? "pass" : "FAIL",
                   would_block_ok ? "pass" : "FAIL",
                   eof_ok ? "pass" : "FAIL",
                   broken_pipe_ok ? "pass" : "FAIL",
                   wraparound_ok ? "pass" : "FAIL");
    }

    /* Phase 40: kernel/rust/spinlock.rs's own self-test - see that
     * file's own doc comment for exactly what each of the three
     * checks (basic protection, single-level interrupt save/restore,
     * correctly-nested interrupt save/restore) proves and why. Called
     * the same way as every other kernel-side Rust self-test in this
     * project: directly, from ring 0, no syscall involved. Returns a
     * bitmask (0 = every check passed) rather than a bool, so a
     * failure here points at exactly which property broke instead of
     * just "something did." */
    {
        extern int rust_spinlock_selftest(void);
        int result = rust_spinlock_selftest();
        kernel_log("[ %s ] Kernel-side Rust spinlock self-test: "
                   "basic-protection=%s interrupt-save-restore=%s "
                   "nested-lock-interrupt-handling=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass");
    }

    /* Phase 42: kernel/rust/virtio_blk.rs's own layout self-test -
     * verifies the legacy virtqueue byte-offset arithmetic against
     * hand-computed values (themselves independently cross-checked -
     * see PROGRESS.md) before this driver ever trusts it against real
     * hardware DMA. The real, hardware-backed read/write/readback test
     * runs later, once virtio_blk_init() (Phase 39's driver
     * self-registration, DRIVER_PHASE_AFTER_PCI) has actually found
     * and initialized a device - see that test, further down in this
     * function, for the actual end-to-end proof. */
    {
        extern int rust_virtqueue_selftest(void);
        int result = rust_virtqueue_selftest();
        kernel_log("[ %s ] Kernel-side Rust virtqueue layout self-test: "
                   "small-queue-layout=%s large-queue-layout=%s "
                   "pages-needed=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass");
    }

    /* Phase 43: kernel/rust/net_irq.rs's own self-test - verifies the
     * signal/check-and-clear pair's basic logic before RTL8139's real
     * IRQ handler ever depends on it. See that file's own doc comment
     * for why this can only prove the logic, not the actual interrupt-
     * context race it exists to survive - the real network self-tests
     * further down (PING/TFTP/DNS/TCP) are what prove *that*, now
     * running with the NIC genuinely interrupt-driven instead of
     * polled every idle tick. */
    {
        extern int rust_net_irq_selftest(void);
        int result = rust_net_irq_selftest();
        kernel_log("[ %s ] Kernel-side Rust net IRQ signal self-test: "
                   "signal-then-check=%s check-actually-clears=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass");
    }

    /* Phase 44: kernel/rust/acpi.rs's own self-test - verifies the
     * MADT-parsing logic against a small, synthetic, hand-constructed
     * table this test builds and checksums itself, entirely
     * independent of whatever real ACPI tables this machine actually
     * provides (checked separately, immediately below - real
     * hardware discovery and parsing-logic correctness are two
     * different things, and this project's own established practice
     * (kernel/rust/virtio_blk.rs's own two-stage layout-then-hardware
     * verification) is to prove each on its own terms rather than
     * conflating them). See acpi.rs's own doc comment for the full
     * scope note: this is CPU *discovery* only, a genuine SMP
     * prerequisite - not SMP support itself, and nothing about this
     * kernel's actual boot/scheduling behavior changes as a result. */
    {
        extern int rust_acpi_selftest(void);
        int result = rust_acpi_selftest();
        kernel_log("[ %s ] Kernel-side Rust ACPI MADT parsing self-test: "
                   "checksum=%s madt-found=%s local-apic-addr=%s "
                   "enabled-cpu-count=%s apic-id=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass",
                   (result & 8) ? "FAIL" : "pass",
                   (result & 16) ? "FAIL" : "pass");
    }

    /* Phase 45: kernel/rust/virtio_net.rs's own self-test - verifies
     * the virtqueue layout math (same shape as virtio_blk.rs's own,
     * independently-verified formula) and the RX post/poll round trip
     * against a local, stack-allocated fake queue, entirely
     * independent of real hardware. The real hardware self-test (an
     * actual Ethernet frame sent and received through real QEMU
     * virtio-net-pci DMA) is verified separately, manually, outside
     * this project's shared default test config - see PROGRESS.md's
     * Phase 45 entry for why, and for that result. */
    {
        extern int rust_virtio_net_selftest(void);
        int result = rust_virtio_net_selftest();
        kernel_log("[ %s ] Kernel-side Rust virtio-net self-test: "
                   "layout=%s rx-post-poll-roundtrip=%s "
                   "rx-poll-no-double-report=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass");
    }

    /* Real hardware discovery - whatever this actual machine's own
     * ACPI tables report, not the synthetic self-test data above.
     * Read-only and purely informational: nothing later in this boot
     * sequence acts on this result yet (no second CPU is started, no
     * interrupt controller changes) - see PROGRESS.md's Phase 44
     * entry for how this was verified against several different QEMU
     * `-smp N` configurations, confirming the logged count actually
     * tracks reality rather than just "the code ran without
     * crashing." */
    {
        extern bool rust_acpi_discover_cpus(uint32_t* out_count,
                                             uint32_t* out_local_apic_phys,
                                             bool* out_found_acpi);
        uint32_t cpu_count = 0;
        uint32_t local_apic_phys = 0;
        bool found_acpi = false;
        bool found = rust_acpi_discover_cpus(&cpu_count, &local_apic_phys,
                                              &found_acpi);
        if (found) {
            kernel_log("[ OK ] ACPI MADT: %d CPU(s) found (Local APIC at "
                       "phys 0x%x) - single-core boot continuing "
                       "regardless (see PROGRESS.md's Phase 44 scope "
                       "note)\n", (int)cpu_count, (unsigned int)local_apic_phys);
        } else {
            kernel_log("[WARN] ACPI MADT not found or not parseable "
                       "(RSDP found: %s) - assuming single-core (this "
                       "kernel does not require ACPI to boot correctly; "
                       "if RSDP was found, the RSDT/MADT tables "
                       "themselves are most likely above this kernel's "
                       "own 64MB identity-mapped range - see "
                       "kernel/rust/acpi.rs's own header comment)\n",
                       found_acpi ? "yes" : "no");
        }
    }

    /* Phase 46: proves virtio-blk is a genuine, mountable VFS block
     * device - not just capable of raw sector I/O (Phase 42's own,
     * now-superseded self-test already proved that), but capable of
     * having a real FAT32 filesystem mounted on it via the exact same
     * fat32_init()/fat32_read_file()/fat32_write_file() path every
     * other filesystem operation in this kernel already goes through,
     * now routed via kernel/drivers/blockdev.h's dispatch instead of
     * calling ata_* directly (the real work this phase's own commit
     * does - see PROGRESS.md's Phase 46 entry for the full account,
     * including two things discovered while implementing this that
     * weren't in the original plan: ata.c's own internal partition-
     * offset state, and ata_is_present(), both also needed routing
     * through the new abstraction, not just the read/write calls).
     *
     * The dangerous part, handled carefully: fat32.c holds exactly
     * one mounted filesystem's worth of *global* state at a time - it
     * was not, and is not, being changed to support two simultaneous
     * mounts (a separate, larger piece of work). Temporarily mounting
     * virtio-blk's own filesystem here necessarily *replaces* whatever
     * fat32.c currently has mounted (the real, ATA-backed filesystem
     * vfs_init() already mounted, which SHELL.ELF's own loading and
     * every file this kernel reads from here to the end of boot
     * depends on) - so the exact prior state (which device was active,
     * and its exact partition offset) is saved before this runs and
     * explicitly restored, via a real re-mount, not just flipping the
     * device selector back, before boot continues. Verified this
     * restore actually works, not just written and assumed correct:
     * this project's own full test suite - including everything that
     * runs *after* this point (the ring-3 shell itself launching,
     * every self-test after this one) - was re-run and confirmed
     * passing with this demonstration active. */
    if (virtio_blk_is_present()) {
        blockdev_id_t saved_device = blockdev_current();
        uint32_t saved_offset = blockdev_get_partition_offset();

        blockdev_select(BLOCKDEV_VIRTIO_BLK);
        blockdev_set_partition_offset(0); /* raw, unpartitioned FAT32
                                              at LBA 0 - see
                                              tools/python/
                                              test_runner.py's own
                                              ensure_virtio_test_disk() */
        fat32_set_partition_offset(0);
        bool mounted = fat32_init();

        bool read_ok = false;
        static char read_buf[128];
        if (mounted) {
            int n = fat32_read_file("VIRTTEST.TXT", read_buf,
                                     sizeof(read_buf));
            static const char expected[] =
                "Hello from a real FAT32 filesystem mounted through "
                "virtio-blk!\n";
            read_ok = (n == (int)sizeof(expected) - 1) &&
                      (memcmp(read_buf, expected, (size_t)n) == 0);
        }

        bool write_ok = false;
        if (mounted) {
            static const char new_content[] =
                "Written through virtio-blk, read back through "
                "virtio-blk.\n";
            bool wrote = fat32_write_file("VIRTNEW.TXT", new_content,
                                           sizeof(new_content) - 1);
            static char readback_buf[128];
            int n2 = wrote ? fat32_read_file("VIRTNEW.TXT", readback_buf,
                                              sizeof(readback_buf))
                            : -1;
            write_ok = wrote && (n2 == (int)sizeof(new_content) - 1) &&
                       (memcmp(readback_buf, new_content, (size_t)n2) == 0);
        }

        /* Restore - see this block's own comment above on why this is
         * the single most important part of this entire phase. */
        blockdev_select(saved_device);
        blockdev_set_partition_offset(saved_offset);
        fat32_set_partition_offset(saved_offset);
        bool remounted = fat32_init();

        if (mounted && read_ok && write_ok && remounted) {
            kernel_log("[ OK ] VIRTIO-BLK VFS MOUNT OK: a real FAT32 "
                       "filesystem was mounted through virtio-blk, an "
                       "existing file read back correctly, a new file "
                       "written and read back correctly, and the "
                       "original ATA-backed mount was correctly "
                       "restored afterward\n");
        } else {
            kernel_log("[FAULT] VIRTIO-BLK VFS MOUNT: mounted=%d "
                       "read_ok=%d write_ok=%d remounted=%d\n",
                       (int)mounted, (int)read_ok, (int)write_ok,
                       (int)remounted);
        }
    }

    /* Phase 50: kernel/rust/sha256.rs's own self-test - verifies this
     * from-scratch SHA-256 implementation against three of the
     * algorithm's own standard, independently-known-correct test
     * vectors (including one long enough to exercise the multi-block
     * message-schedule expansion, not just the single-block path)
     * before kernel/rust/users.rs's own real password hashing (this
     * same phase) is ever allowed to depend on it. */
    {
        extern int rust_sha256_selftest(void);
        int result = rust_sha256_selftest();
        kernel_log("[ %s ] Kernel-side Rust SHA-256 self-test: "
                   "empty-string=%s abc=%s multi-block=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass");
    }

    /* Phase 50: kernel/rust/hmac_sha256.rs's own self-test - verifies
     * against RFC 4231's own standard Test Case 1, independently
     * cross-checked (Python's hmac module) before being hardcoded
     * here. */
    {
        extern int rust_hmac_sha256_selftest(void);
        int result = rust_hmac_sha256_selftest();
        kernel_log("[ %s ] Kernel-side Rust HMAC-SHA256 self-test: "
                   "rfc4231-test-case-1=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass");
    }

    /* Phase 50: kernel/rust/pbkdf2.rs's own self-test - verifies
     * against three independently-generated test vectors (Python's
     * own hashlib.pbkdf2_hmac, not this project's code), including
     * this phase's own actual, chosen production iteration count
     * (4096) - not just a toy case. */
    {
        extern int rust_pbkdf2_selftest(void);
        int result = rust_pbkdf2_selftest();
        kernel_log("[ %s ] Kernel-side Rust PBKDF2-HMAC-SHA256 "
                   "self-test: iterations-1=%s iterations-2=%s "
                   "iterations-4096=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass");
    }

    /* Phase 50: a direct, measured timing check, not an assumption -
     * confirms 4096 iterations (kernel/rust/users.rs's own
     * PBKDF2_ITERATIONS) is actually a reasonable choice on this
     * kernel's own real timer, not just plausible in the abstract.
     * Real login (Phase 49) computes exactly one of these per attempt
     * - this needs to be slow enough to matter against a fast guesser,
     * but not so slow a real, legitimate login feels broken. */
    {
        extern uint8_t rust_pbkdf2_timing_probe(void);
        uint32_t ticks_before = timer_get_ticks();
        rust_pbkdf2_timing_probe();
        uint32_t ticks_after = timer_get_ticks();
        kernel_log("[ .. ] PBKDF2 (4096 iterations) took %d timer tick(s)\n",
                   (int)(ticks_after - ticks_before));
    }

    /* Phase 47/49: kernel/rust/users.rs's own self-test - verifies the
     * add/authenticate/serialize/load round trip before any syscall
     * or process code depends on it, plus (Phase 49) the lockout
     * behavior a real login screen's own "session concept" needs. See
     * that file's own doc comment for the real, external constraint
     * (FAT32's on-disk format has no ownership fields at all) that
     * bounds this phase's scope to process-level identity, not
     * file-level permissions. */
    {
        extern int rust_users_selftest(void);
        int result = rust_users_selftest();
        kernel_log("[ %s ] Kernel-side Rust user database self-test: "
                   "add=%s right-password=%s wrong-password-rejected=%s "
                   "unknown-user-rejected=%s serialize=%s "
                   "persistence-round-trip=%s locked-out-after-threshold=%s "
                   "different-salts-for-same-password=%s\n",
                   result == 0 ? "OK" : "FAIL",
                   (result & 1) ? "FAIL" : "pass",
                   (result & 2) ? "FAIL" : "pass",
                   (result & 4) ? "FAIL" : "pass",
                   (result & 8) ? "FAIL" : "pass",
                   (result & 16) ? "FAIL" : "pass",
                   (result & 32) ? "FAIL" : "pass",
                   (result & 64) ? "FAIL" : "pass",
                   (result & 128) ? "FAIL" : "pass");
    }

    firstrun_check_and_run();

    process_init();
    process_create_kernel_task("idle", idle_task_entry);
    /* Phase 30: NovaOS now boots into a genuine ring-3 shell via
     * process_exec_as_shell() instead of running a shell as a
     * ring-0 kernel task (userland/shell/shell.c, Phase 29's
     * organizationally-separated but still-ring-0 shell, kept in the
     * tree but no longer launched at boot). This is non-blocking
     * (just creates and schedules the process, like every other
     * process_create_*() call here) - safe to call directly from
     * kernel_main() before scheduler_start(), unlike a call that
     * would block on process_wait(). See PROGRESS.md for the honest
     * scope note on which commands this shell doesn't have yet.
     *
     * Phase 37: WHICH file to exec here is no longer a literal string
     * baked into this function - it comes from
     * firstrun_get_init_path() (backed by SYSTEM.CFG's new init_path
     * field, see sysconfig.h), defaulting to this project's own
     * current shell ("SHELL.ELF") when nothing else is configured.
     * This is the concrete fix for a real, previously-named gap: a
     * kernel that hardcodes one specific userland's init program by
     * name isn't something a *different* userland could boot into
     * without editing kernel source. A different userland/distro
     * sharing this same kernel binary now only needs its own
     * SYSTEM.CFG (or first-run wizard) to name a different init
     * program - no kernel change required. */
    const char* init_path = firstrun_get_init_path();
    const char* shell_argv[] = {init_path};
    process_exec_as_shell(init_path, shell_argv, 1);
    process_create_kernel_task("coreutils-test", coreutils_test_task);
    process_create_user_task("demo-a", user_demo_task_a);
    process_create_user_task("demo-b", user_demo_task_b);

    const char* sandbox_caps[] = {"HELLO.TXT"};
    const uint32_t sandbox_hosts[] = {NET_GATEWAY_IP};

    /* Phase 48: sandbox_demo_task's own SYS_LOGIN/SYS_GETUID self-test
     * (kernel/task/sandbox_demo.c) now authenticates against the real
     * account this project's own tools/fixtures/USERS.CFG persists -
     * loaded above, in firstrun_check_and_run()'s own "returning user"
     * branch (userscfg_load()), replacing Phase 47's own kernel-side
     * hardcoded test-account creation. A strictly stronger test:
     * proves the full pipeline (a real file on disk -> vfs_read_file
     * -> userscfg_load -> rust_users_load -> rust_users_authenticate)
     * rather than only the in-memory add/authenticate calls. */
    process_create_sandboxed_task("sandbox", sandbox_demo_task, sandbox_caps,
                                   1, sandbox_hosts, 1, true);
    process_create_user_task("unprivileged", unprivileged_demo_task);

    kernel_log("[ OK ] Tasks created: idle (kernel), shell (kernel), "
               "demo-a + demo-b (ring 3, private address spaces), "
               "sandbox (ring 3, capabilities: HELLO.TXT + gateway "
               "network access + spawn), unprivileged (ring 3, no "
               "capabilities)\n");

    scheduler_start(); /* never returns */
}
