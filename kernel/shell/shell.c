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
    vga_puts("  reboot    - restart the machine\n");
}

static void cmd_about(void) {
    vga_printf("%s v%s\n", KERNEL_NAME, KERNEL_VERSION);
    vga_puts("Phase 3: Paging, Physical Memory & Filesystem\n");
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
    } else if (strncmp(line, "cat ", 4) == 0) {
        cmd_cat(line + 4);
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
