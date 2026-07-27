/*
 * firstrun.c - asks for a hostname/username once, persists them via
 * sysconfig.c, greets returning users on every later boot
 */
#include "firstrun.h"
#include "../drivers/vga/vga.h"
#include "../drivers/keyboard/keyboard.h"
#include "../fs/vfs.h"
#include "../config/sysconfig.h"
#include "../lib/string.h"
#include "../include/kernel.h"

static char g_hostname[SYSCONFIG_HOSTNAME_MAX] = "novaos";
static char g_username[SYSCONFIG_USERNAME_MAX] = "user";

static void bounded_copy(char* dest, const char* src, size_t size) {
    size_t i = 0;
    while (i < size - 1 && src[i]) {
        dest[i] = src[i];
        i++;
    }
    dest[i] = '\0';
}

static void read_line_echo(char* out, int max_len) {
    int len = 0;
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
        } else if (len < max_len - 1 && c >= 32 && c < 127) {
            out[len++] = c;
            vga_putchar(c);
        }
    }
    out[len] = '\0';
}

void firstrun_check_and_run(void) {
    sysconfig_t cfg;
    if (sysconfig_load(&cfg)) {
        bounded_copy(g_hostname, cfg.hostname, sizeof(g_hostname));
        bounded_copy(g_username, cfg.username, sizeof(g_username));

        vga_set_color(VGA_COLOR_GREEN, VGA_COLOR_BLACK);
        vga_printf("Welcome back, %s! (%s)\n\n", g_username, g_hostname);
        vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);

        kernel_log("[ OK ] First-run check: returning user '%s' on '%s'\n",
                   g_username, g_hostname);
        return;
    }

    if (!vfs_is_mounted()) {
        vga_puts("No disk detected - skipping first-run setup (settings "
                  "won't persist between boots).\n\n");
        kernel_log("[ .. ] First-run check: no disk, wizard skipped\n");
        return;
    }

    vga_set_color(VGA_COLOR_CYAN, VGA_COLOR_BLACK);
    vga_puts("=========================================\n");
    vga_puts(" Welcome to NovaOS! Let's set a few things up.\n");
    vga_puts("=========================================\n\n");
    vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);

    char hostname_buf[SYSCONFIG_HOSTNAME_MAX];
    char username_buf[SYSCONFIG_USERNAME_MAX];

    vga_puts("Choose a hostname for this computer: ");
    read_line_echo(hostname_buf, sizeof(hostname_buf));
    if (hostname_buf[0] == '\0') {
        strcpy(hostname_buf, "novaos");
    }

    vga_puts("Choose a username: ");
    read_line_echo(username_buf, sizeof(username_buf));
    if (username_buf[0] == '\0') {
        strcpy(username_buf, "user");
    }

    sysconfig_t new_cfg;
    memset(&new_cfg, 0, sizeof(new_cfg));
    bounded_copy(new_cfg.hostname, hostname_buf, sizeof(new_cfg.hostname));
    bounded_copy(new_cfg.username, username_buf, sizeof(new_cfg.username));

    if (sysconfig_save(&new_cfg)) {
        vga_set_color(VGA_COLOR_GREEN, VGA_COLOR_BLACK);
        vga_printf("\nSetup complete! Welcome, %s.\n\n", username_buf);
        vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);
        kernel_log("[ OK ] First-run wizard complete: '%s' on '%s'\n",
                   username_buf, hostname_buf);
    } else {
        vga_puts("\n[WARN] Could not save settings to disk - they won't "
                  "persist to the next boot.\n\n");
        kernel_log("[WARN] First-run wizard: sysconfig_save failed\n");
    }

    bounded_copy(g_hostname, hostname_buf, sizeof(g_hostname));
    bounded_copy(g_username, username_buf, sizeof(g_username));
}

const char* firstrun_get_hostname(void) {
    return g_hostname;
}

const char* firstrun_get_username(void) {
    return g_username;
}
