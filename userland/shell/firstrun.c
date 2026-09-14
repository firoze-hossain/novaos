/*
 * firstrun.c - asks for a hostname/username once, persists them via
 * sysconfig.c, greets returning users on every later boot
 */
#include "firstrun.h"
#include "../../kernel/drivers/vga/vga.h"
#include "../../kernel/drivers/keyboard/keyboard.h"
#include "../../kernel/fs/vfs.h"
#include "../../kernel/config/sysconfig.h"
#include "../../kernel/config/userscfg.h"
#include "../../kernel/lib/string.h"
#include "../../kernel/include/kernel.h"

/* kernel/rust/users.rs's exported account-creation function - see
 * that file's own doc comment for the full contract. */
extern bool rust_users_add(const uint8_t* username_ptr, uint32_t username_len,
                            uint32_t uid, uint32_t gid,
                            const uint8_t* password_ptr,
                            uint32_t password_len);

static char g_hostname[SYSCONFIG_HOSTNAME_MAX] = "novaos";
static char g_username[SYSCONFIG_USERNAME_MAX] = "user";
/* Phase 37: defaults to this project's own current, single userland's
 * init program - see firstrun.h's own comment on when/why this stays
 * at the default rather than whatever a loaded SYSTEM.CFG says. */
static char g_init_path[SYSCONFIG_INIT_PATH_MAX] = "SHELL.ELF";

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

/* Phase 48: same structure as read_line_echo() above, but shows '*'
 * for every typed character instead of the character itself - the
 * familiar password-prompt convention. Doesn't make the stored
 * credential itself any more secure (see kernel/rust/users.rs's own
 * documented limitation: FNV-1a is not a cryptographic hash) - this
 * only avoids the separate, basic problem of the password appearing
 * in plain text on screen while being typed. */
static void read_line_noecho(char* out, int max_len) {
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
            vga_putchar('*');
        }
    }
    out[len] = '\0';
}

void firstrun_check_and_run(void) {
    sysconfig_t cfg;
    if (sysconfig_load(&cfg)) {
        bounded_copy(g_hostname, cfg.hostname, sizeof(g_hostname));
        bounded_copy(g_username, cfg.username, sizeof(g_username));
        /* Phase 37: an old-format SYSTEM.CFG can never reach this
         * branch at all (sysconfig_load()'s exact-size check already
         * rejected it, sending this boot down the first-run path
         * instead - see sysconfig.h). This guard instead covers a
         * *new*-format config whose init_path was simply never set
         * (e.g. written by code that only knew about hostname/
         * username) - g_init_path's own compiled-in default is kept
         * rather than overwriting it with an empty string. */
        if (cfg.init_path[0] != '\0') {
            bounded_copy(g_init_path, cfg.init_path, sizeof(g_init_path));
        }

        vga_set_color(VGA_COLOR_GREEN, VGA_COLOR_BLACK);
        vga_printf("Welcome back, %s! (%s)\n\n", g_username, g_hostname);
        vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);

        kernel_log("[ OK ] First-run check: returning user '%s' on '%s'\n",
                   g_username, g_hostname);

        /* Phase 48: a returning user also means real, previously-
         * created accounts (see the first-boot branch below) should
         * exist in kernel/rust/users.rs's own database - load them
         * now, the same "returning user -> load, don't re-create"
         * shape sysconfig_load() just above already established for
         * SYSTEM.CFG. A false return here (no USERS.CFG on disk,
         * despite a real SYSTEM.CFG existing) is a real, benign case
         * - e.g. a SYSTEM.CFG written before this phase existed at
         * all - so it's only logged, not treated as an error. */
        if (userscfg_load()) {
            kernel_log("[ OK ] First-run check: USERS.CFG loaded - "
                       "accounts restored\n");
        } else {
            kernel_log("[ .. ] First-run check: no USERS.CFG found - "
                       "no accounts loaded\n");
        }
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

    /* Phase 48: the first account this wizard ever creates - uid 0
     * (root), matching Unix's own "the machine's first, owning
     * account is the admin" convention (see process.h's own comment
     * on process_t's uid/gid for why uid 0 specifically means root
     * throughout this kernel). A blank password is accepted rather
     * than rejected/re-prompted - a deliberate, honest simplification
     * for this first pass, not a security recommendation; a future
     * "require a non-empty password" check is real, separate,
     * smaller-scoped follow-up work. */
    char password_buf[64];
    vga_puts("Choose a password: ");
    read_line_noecho(password_buf, sizeof(password_buf));

    bool user_added = rust_users_add(
        (const uint8_t*)username_buf, (uint32_t)strlen(username_buf), 0, 0,
        (const uint8_t*)password_buf, (uint32_t)strlen(password_buf));
    if (user_added) {
        if (!userscfg_save()) {
            vga_puts("[WARN] Could not save the account to disk - it "
                     "won't persist to the next boot.\n");
            kernel_log("[WARN] First-run wizard: userscfg_save failed\n");
        }
    } else {
        vga_puts("[WARN] Could not create the account.\n");
        kernel_log("[WARN] First-run wizard: rust_users_add failed\n");
    }

    sysconfig_t new_cfg;
    memset(&new_cfg, 0, sizeof(new_cfg));
    bounded_copy(new_cfg.hostname, hostname_buf, sizeof(new_cfg.hostname));
    bounded_copy(new_cfg.username, username_buf, sizeof(new_cfg.username));
    /* Phase 37: explicit, not left as the zero-fill above plus a
     * reader-side fallback - a SYSTEM.CFG this wizard writes should be
     * a complete, self-describing record of what this system boots
     * into, not something that only works by relying on every future
     * reader happening to default it the same way. */
    bounded_copy(new_cfg.init_path, g_init_path, sizeof(new_cfg.init_path));

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

const char* firstrun_get_init_path(void) {
    return g_init_path;
}
