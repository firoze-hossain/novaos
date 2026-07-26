/*
 * shell.c - minimal built-in command line
 *
 * A tiny, dependency-light shell so Phase 2's keyboard/timer/heap work
 * is actually usable and testable by a person sitting at the console,
 * ahead of the full Phase 4 shell (which will add a real filesystem,
 * job control, and scripting).
 */
#include "shell.h"
#include "../drivers/vga/vga.h"
#include "../drivers/keyboard/keyboard.h"
#include "../drivers/timer/timer.h"
#include "../arch/x86/mm/heap.h"
#include "../arch/x86/mm/pmm.h"
#include "../arch/x86/io.h"
#include "../fs/vfs.h"
#include "../net/net.h"
#include "../net/icmp.h"
#include "../drivers/video/vga_graphics.h"
#include "../drivers/mouse/ps2mouse.h"
#include "../gui/compositor.h"
#include "../task/process.h"
#include "../lib/string.h"
#include "../include/kernel.h"
#include "../include/version.h"

#define LINE_MAX 256

static void read_line(char* out) {
    size_t len = 0;
    for (;;) {
        char c = keyboard_get_char();

        if (c == '\n') {
            vga_putchar('\n');
            break;
        } else if (c == '\b') {
            if (len > 0) {
                len--;
                vga_putchar('\b');
                vga_putchar(' ');
                vga_putchar('\b');
            }
        } else if (len < LINE_MAX - 1) {
            out[len++] = c;
            vga_putchar(c);
        }
    }
    out[len] = '\0';
}

static void cmd_help(void) {
    vga_puts("Available commands:\n");
    vga_puts("  help      - show this list\n");
    vga_puts("  about     - NovaOS version info\n");
    vga_puts("  echo TEXT - print TEXT\n");
    vga_puts("  clear     - clear the screen\n");
    vga_puts("  meminfo   - show heap + physical memory usage\n");
    vga_puts("  uptime    - show timer ticks since boot\n");
    vga_puts("  ls        - list files on the mounted disk\n");
    vga_puts("  cat FILE  - print a file's contents\n");
    vga_puts("  ps        - list running processes\n");
    vga_puts("  ping IP   - send an ICMP echo request (e.g. ping 10.0.2.2)\n");
    vga_puts("  gui       - enter graphics mode; drag windows with the "
              "mouse, ESC to exit\n");
    vga_puts("  reboot    - restart the machine\n");
}

static void cmd_about(void) {
    vga_printf("%s v%s\n", KERNEL_NAME, KERNEL_VERSION);
    vga_puts("Phase 7: Graphics Mode & a Minimal Windowing System\n");
}

static const char* state_name(process_state_t s) {
    switch (s) {
        case PROCESS_READY:      return "READY";
        case PROCESS_RUNNING:    return "RUNNING";
        case PROCESS_TERMINATED: return "TERMINATED";
        default:                 return "-";
    }
}

static void cmd_ps(void) {
    vga_puts("  PID  RING  STATE       NAME\n");
    for (int i = 0; i < MAX_PROCESSES; i++) {
        process_t* p = process_table_entry(i);
        if (p == NULL || p->state == PROCESS_UNUSED) {
            continue;
        }
        vga_printf("  %d    %s   %s", p->pid, p->is_user ? "u3" : "k0",
                   state_name(p->state));
        vga_puts("       ");
        vga_puts(p->name);
        vga_putchar('\n');
    }
}

static void cmd_meminfo(void) {
    heap_stats_t heap_stats;
    heap_get_stats(&heap_stats);
    vga_printf("Heap total: %d bytes\n", (int)heap_stats.total_bytes);
    vga_printf("Heap used:  %d bytes\n", (int)heap_stats.used_bytes);
    vga_printf("Heap free:  %d bytes\n", (int)heap_stats.free_bytes);

    pmm_stats_t pmm_stats;
    pmm_get_stats(&pmm_stats);
    vga_printf("Physical frames total: %d (%d MB)\n",
               (int)pmm_stats.total_frames,
               (int)(pmm_stats.total_frames * 4096 / (1024 * 1024)));
    vga_printf("Physical frames used:  %d\n", (int)pmm_stats.used_frames);
    vga_printf("Physical frames free:  %d\n", (int)pmm_stats.free_frames);
}

static void cmd_ls(void) {
    vfs_ls();
}

static void cmd_cat(const char* filename) {
    static char buf[4096];
    int n = vfs_read_file(filename, buf, sizeof(buf) - 1);
    if (n < 0) {
        vga_printf("cat: %s: not found (or no disk mounted)\n", filename);
        return;
    }
    buf[n] = '\0';
    vga_puts(buf);
    vga_putchar('\n');
}

static void cmd_uptime(void) {
    vga_printf("Ticks since boot: %d\n", (int)timer_get_ticks());
}

/* No sscanf in this freestanding libc subset (see PROGRESS.md's libc
 * gaps) - a small hand-rolled dotted-quad parser instead. Returns
 * false on anything that doesn't look like four 0-255 octets. */
static bool parse_ipv4(const char* text, uint32_t* out_ip) {
    uint8_t octets[4];
    int octet_index = 0;
    int value = -1;

    for (const char* p = text;; p++) {
        if (*p >= '0' && *p <= '9') {
            if (value < 0) {
                value = 0;
            }
            value = value * 10 + (*p - '0');
            if (value > 255) {
                return false;
            }
        } else if (*p == '.' || *p == '\0') {
            if (value < 0 || octet_index >= 4) {
                return false;
            }
            octets[octet_index++] = (uint8_t)value;
            value = -1;
            if (*p == '\0') {
                break;
            }
        } else {
            return false;
        }
    }
    if (octet_index != 4) {
        return false;
    }

    *out_ip = ip_make(octets[0], octets[1], octets[2], octets[3]);
    return true;
}

static void cmd_ping(const char* arg) {
    if (!net_is_up()) {
        vga_puts("ping: no network adapter present\n");
        return;
    }

    uint32_t dest_ip;
    if (!parse_ipv4(arg, &dest_ip)) {
        vga_puts("ping: usage: ping A.B.C.D\n");
        return;
    }

    vga_printf("Pinging %s ...\n", arg);
    uint32_t rtt_ticks = 0;
    if (icmp_ping(dest_ip, &rtt_ticks)) {
        vga_printf("Reply from %s: time=%dms\n", arg, (int)(rtt_ticks * 10));
    } else {
        vga_puts("Request timed out.\n");
    }
}

static void cmd_gui(void) {
    if (!ps2mouse_is_present()) {
        vga_puts("gui: no PS/2 mouse detected - nothing to interact with\n");
        return;
    }

    vga_puts("Entering graphics mode. Drag windows by their titlebar; "
              "press ESC to return to the shell.\n");

    vga_graphics_enter();
    compositor_init();

    /* Discard whatever movement accumulates around entry into graphics
     * mode - found during testing: a single one-shot discard wasn't
     * enough. Something (most likely QEMU's own input layer reacting
     * to the display resolution change from vga_graphics_enter(),
     * though this is a headless/monitor-driven testing artifact more
     * than a certainty about real hardware) can generate a burst of
     * mouse activity around the mode switch, and that burst doesn't
     * always land entirely within a single read taken immediately
     * after entry. Draining repeatedly for a short settling window
     * catches it regardless of exactly when it arrives, rather than
     * gambling on one read at one instant. */
    for (int i = 0; i < 20; i++) {
        (void)ps2mouse_read();
        timer_sleep_ms(10);
    }

    for (;;) {
        if (keyboard_has_char()) {
            char c = keyboard_get_char();
            if (c == 27) { /* ESC */
                break;
            }
        }

        mouse_state_t state = ps2mouse_read();
        compositor_handle_mouse(state);
        compositor_render();

        timer_sleep_ms(16); /* ~60fps cap - courteous, not load-bearing */
    }

    vga_graphics_exit();
    vga_clear();
}

static void cmd_reboot(void) {
    vga_puts("Rebooting...\n");
    /* 8042 keyboard controller "pulse output line" reset - the classic
     * portable way to reboot an x86 PC from software without ACPI. */
    uint8_t status;
    do {
        status = inb(0x64);
    } while (status & 0x02);
    outb(0x64, 0xFE);
    for (;;) {
        __asm__ volatile ("hlt");
    }
}

static void dispatch(char* line) {
    if (line[0] == '\0') {
        return;
    }

    if (strcmp(line, "help") == 0) {
        cmd_help();
    } else if (strcmp(line, "about") == 0) {
        cmd_about();
    } else if (strcmp(line, "clear") == 0) {
        vga_clear();
    } else if (strcmp(line, "meminfo") == 0) {
        cmd_meminfo();
    } else if (strcmp(line, "uptime") == 0) {
        cmd_uptime();
    } else if (strcmp(line, "reboot") == 0) {
        cmd_reboot();
    } else if (strcmp(line, "ls") == 0) {
        cmd_ls();
    } else if (strcmp(line, "ps") == 0) {
        cmd_ps();
    } else if (strcmp(line, "gui") == 0) {
        cmd_gui();
    } else if (strncmp(line, "cat ", 4) == 0) {
        cmd_cat(line + 4);
    } else if (strncmp(line, "ping ", 5) == 0) {
        cmd_ping(line + 5);
    } else if (strncmp(line, "echo ", 5) == 0) {
        vga_puts(line + 5);
        vga_putchar('\n');
    } else {
        vga_printf("Unknown command: %s (try 'help')\n", line);
    }
}

void shell_run(void) {
    char line[LINE_MAX];

    for (;;) {
        vga_set_color(VGA_COLOR_CYAN, VGA_COLOR_BLACK);
        vga_puts("nova> ");
        vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);

        read_line(line);
        dispatch(line);
    }
}
