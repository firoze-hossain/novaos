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
#include "../net/tftp.h"
#include "../drivers/video/vga_graphics.h"
#include "../drivers/mouse/ps2mouse.h"
#include "../gui/compositor.h"
#include "../gui/store.h"
#include "../pkg/pkgmgr.h"
#include "../drivers/rtc/rtc.h"
#include "../drivers/pci/pci.h"
#include "firstrun.h"
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
    vga_puts("  store     - Software Center GUI: install/remove packages "
              "with the mouse\n");
    vga_puts("  pkg list  - show available packages (pkg installed, "
              "pkg install NAME, pkg remove NAME, pkg fetch NAME)\n");
    vga_puts("  tftp get FILE [IP] - fetch a file over TFTP (default "
              "server: the gateway, 10.0.2.2)\n");
    vga_puts("  date      - show the current date/time (CMOS RTC)\n");
    vga_puts("  hostname  - show this computer's configured hostname\n");
    vga_puts("  whoami    - show the configured username\n");
    vga_puts("  lspci     - list detected PCI devices\n");
    vga_puts("  reboot    - restart the machine\n");
}

static void cmd_about(void) {
    vga_printf("%s v%s\n", KERNEL_NAME, KERNEL_VERSION);
    vga_puts("Phase 13: PCI Bus Enumeration\n");
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

static void cmd_tftp(const char* arg) {
    if (!net_is_up()) {
        vga_puts("tftp: no network adapter present\n");
        return;
    }
    if (strncmp(arg, "get ", 4) != 0) {
        vga_puts("Usage: tftp get FILENAME [SERVER_IP]\n");
        return;
    }

    const char* rest = arg + 4;
    char filename[64];
    char server_str[32];
    int i = 0;
    while (rest[i] && rest[i] != ' ' && i < (int)sizeof(filename) - 1) {
        filename[i] = rest[i];
        i++;
    }
    filename[i] = '\0';

    uint32_t server_ip = NET_GATEWAY_IP;
    if (rest[i] == ' ') {
        int j = 0;
        const char* ip_part = rest + i + 1;
        while (ip_part[j] && j < (int)sizeof(server_str) - 1) {
            server_str[j] = ip_part[j];
            j++;
        }
        server_str[j] = '\0';
        uint32_t parsed;
        if (parse_ipv4(server_str, &parsed)) {
            server_ip = parsed;
        }
    }

    vga_printf("Fetching '%s' via TFTP...\n", filename);
    static uint8_t tftp_buf[8192];
    int n = tftp_get(server_ip, filename, tftp_buf, sizeof(tftp_buf));
    if (n < 0) {
        vga_puts("tftp: fetch failed (timed out or server error)\n");
        return;
    }

    if (!vfs_write_file(filename, tftp_buf, (uint32_t)n)) {
        vga_printf("tftp: fetched %d bytes but failed to save '%s' "
                   "(already exists on disk?)\n", n, filename);
        return;
    }

    vga_printf("Fetched and saved '%s' (%d bytes).\n", filename, n);
}

static void print_pkg_entry(const char* filename, const char* name,
                             const char* version, const char* description,
                             bool installed) {
    (void)filename;
    vga_printf("  %s %s - %s%s\n", name, version, description,
               installed ? "  [installed]" : "");
}

static void print_installed_entry(const char* filename, const char* name,
                                   const char* version,
                                   const char* description, bool installed) {
    (void)installed;
    vga_printf("  %s %s - %s  (%s)\n", name, version, description, filename);
}

static void cmd_pkg(const char* arg) {
    if (!vfs_is_mounted()) {
        vga_puts("pkg: no filesystem mounted\n");
        return;
    }

    if (strcmp(arg, "list") == 0) {
        vga_puts("Available packages:\n");
        pkg_list_available(print_pkg_entry);
    } else if (strcmp(arg, "installed") == 0) {
        vga_puts("Installed packages:\n");
        pkg_list_installed(print_installed_entry);
    } else if (strncmp(arg, "install ", 8) == 0) {
        const char* name = arg + 8;
        if (pkg_install(name)) {
            vga_printf("Installed '%s'.\n", name);
        } else {
            vga_printf("pkg install: failed - see 'help' or try 'pkg list' "
                       "to check the name (%s)\n", name);
        }
    } else if (strncmp(arg, "remove ", 7) == 0) {
        const char* name = arg + 7;
        if (pkg_remove(name)) {
            vga_printf("Removed '%s'.\n", name);
        } else {
            vga_printf("pkg remove: '%s' is not installed\n", name);
        }
    } else if (strncmp(arg, "fetch ", 6) == 0) {
        if (!net_is_up()) {
            vga_puts("pkg fetch: no network adapter present\n");
            return;
        }
        const char* name = arg + 6;
        char pkg_filename[20];
        int i = 0;
        while (name[i] && name[i] != ' ' && i < 15) {
            pkg_filename[i] = (char)((name[i] >= 'a' && name[i] <= 'z')
                                          ? name[i] - 32
                                          : name[i]);
            i++;
        }
        strcpy(pkg_filename + i, ".PKG");

        vga_printf("Fetching '%s' via TFTP from the gateway...\n",
                   pkg_filename);
        static uint8_t fetch_buf[8192];
        int n = tftp_get(NET_GATEWAY_IP, pkg_filename, fetch_buf,
                          sizeof(fetch_buf));
        if (n < 0) {
            vga_puts("pkg fetch: TFTP transfer failed (not found on the "
                      "server, or timed out)\n");
            return;
        }
        if (!vfs_write_file(pkg_filename, fetch_buf, (uint32_t)n)) {
            vga_printf("pkg fetch: fetched %d bytes but couldn't save "
                       "'%s' (already present locally?)\n", n, pkg_filename);
            return;
        }
        vga_printf("Fetched '%s' (%d bytes). Try 'pkg list' then "
                   "'pkg install %s'.\n", pkg_filename, n, name);
    } else {
        vga_puts("Usage: pkg list | pkg installed | pkg install NAME | "
                  "pkg remove NAME | pkg fetch NAME\n");
    }
}

static const char* MONTH_NAMES[] = {
    "?",   "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
};

static void cmd_date(void) {
    rtc_time_t now;
    rtc_read(&now);
    const char* month = (now.month >= 1 && now.month <= 12)
                             ? MONTH_NAMES[now.month]
                             : MONTH_NAMES[0];
    vga_printf("%s %d %d %d:%d:%d UTC\n", month, (int)now.day, (int)now.year,
               (int)now.hour, (int)now.minute, (int)now.second);
}

static void print_pci_device(const pci_device_t* dev) {
    vga_printf("%d:%d.%d  %x:%x  %s\n", (int)dev->bus, (int)dev->device,
               (int)dev->function, (int)dev->vendor_id, (int)dev->device_id,
               pci_class_name(dev->class_code, dev->subclass));
}

static void cmd_lspci(void) {
    vga_puts("BUS:DEV.FN  VENDOR:DEVICE  CLASS\n");
    pci_enumerate(print_pci_device);
}

static void cmd_hostname(void) {
    vga_puts(firstrun_get_hostname());
    vga_putchar('\n');
}

static void cmd_whoami(void) {
    vga_puts(firstrun_get_username());
    vga_putchar('\n');
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
    } else if (strcmp(line, "store") == 0) {
        store_run();
    } else if (strncmp(line, "cat ", 4) == 0) {
        cmd_cat(line + 4);
    } else if (strncmp(line, "ping ", 5) == 0) {
        cmd_ping(line + 5);
    } else if (strncmp(line, "tftp ", 5) == 0) {
        cmd_tftp(line + 5);
    } else if (strcmp(line, "pkg") == 0) {
        cmd_pkg("");
    } else if (strncmp(line, "pkg ", 4) == 0) {
        cmd_pkg(line + 4);
    } else if (strcmp(line, "date") == 0) {
        cmd_date();
    } else if (strcmp(line, "hostname") == 0) {
        cmd_hostname();
    } else if (strcmp(line, "whoami") == 0) {
        cmd_whoami();
    } else if (strcmp(line, "lspci") == 0) {
        cmd_lspci();
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
        vga_printf("%s@%s> ", firstrun_get_username(), firstrun_get_hostname());
        vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);

        read_line(line);
        dispatch(line);
    }
}
