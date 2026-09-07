/*
 * shell.c - a genuine ring-3 interactive shell (Phase 30).
 *
 * Unlike userland/shell/shell.c (the original, Phase 3-era shell,
 * still compiled directly into the kernel binary and running in
 * ring 0), this is a completely separate, standalone ELF32
 * executable - talking to the kernel only through syscalls, the
 * same way /bin/bash relates to the Linux kernel on Ubuntu. This is
 * the real completion of Phase 29's architectural point: the kernel
 * boots into this program via process_exec_as_shell() rather than
 * running a shell as a kernel task at all.
 *
 * Scope, stated honestly: supports ls/cat/run/echo/help/clear - the
 * commands buildable on syscalls that already exist (SYS_OPEN/READ/
 * CLOSE, SYS_EXEC/WAIT, SYS_LIST_FILES, SYS_WRITE) plus the two new
 * ones this phase added (SYS_READ_KEY for input). Networking
 * (ping/nslookup/tftp), package management, the GUI (store), sound
 * (beep), the real-time clock (date), and PCI enumeration (lspci)
 * are NOT available here - each would need its own new syscall
 * surface (SYS_PING, SYS_PKG_*, a way to launch the GUI, SYS_BEEP,
 * SYS_RTC_READ, SYS_LSPCI), deliberately not built in this pass. See
 * PROGRESS.md for the full, honest limitations list.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <novasys.h>
#include <stdbool.h>

/* Phase 32: the package manager, converted to a ring-3 shell builtin
 * (not a separate exec'd binary - see PROGRESS.md for why: exec'd
 * programs start with zero capabilities by default, and there's no
 * mechanism yet for the shell to delegate a subset of its own broad
 * access to something it execs). These two structs are redeclared
 * here rather than shared with userland/pkg/pkgmgr.h, the same
 * "kept as a separate copy since userland is compiled completely
 * separately" reasoning nova_rtc_time_t already follows - byte-for-
 * byte matching kernel/../pkgmgr.h's pkg_header_t and
 * install_record_t, verified against a real fixture file
 * (tools/fixtures/EDITOR.PKG) rather than assumed. */
#define PKG_NAME_MAX 16
#define PKG_VERSION_MAX 8
#define PKG_DESC_MAX 64
#define PKG_MAX_INSTALLED 16
#define PKG_INSTALL_DB "INSTALL.DB"

typedef struct __attribute__((packed)) {
    char magic[4];
    char name[PKG_NAME_MAX];
    char version[PKG_VERSION_MAX];
    char description[PKG_DESC_MAX];
    unsigned int payload_size;
} nova_pkg_header_t;

typedef struct __attribute__((packed)) {
    char name[PKG_NAME_MAX];
    char version[PKG_VERSION_MAX];
    char description[PKG_DESC_MAX];
    char app_filename[13];
} nova_install_record_t;

#define LINE_BUF_SIZE 128
#define MAX_TOKENS 8

static int read_char_blocking(void) {
    int c;
    while ((c = sys_read_key()) < 0) {
        sys_yield();
    }
    return c;
}

/* Reads one line of input, echoing each character back and handling
 * backspace, into `buf` (up to buf_size - 1 characters, NUL-
 * terminated). No line-history/arrow-key support - a real, honest
 * limitation, not an oversight. */
static void read_line(char* buf, int buf_size) {
    int len = 0;
    for (;;) {
        int c = read_char_blocking();
        if (c == '\n' || c == '\r') {
            putchar('\n');
            break;
        }
        if (c == '\b') {
            if (len > 0) {
                len--;
                sys_write("\b \b"); /* move back, erase, move back again -
                                        the standard terminal backspace
                                        trick for a character cell
                                        display */
            }
            continue;
        }
        if (c >= 32 && c < 127 && len < buf_size - 1) {
            buf[len++] = (char)c;
            putchar(c);
        }
    }
    buf[len] = '\0';
}

/* Splits `line` on spaces into up to MAX_TOKENS tokens - the same
 * simple, no-quoting parsing every command in this project's earlier
 * (ring-0) shell already used. Returns the token count. */
static int tokenize(char* line, char* tokens[MAX_TOKENS]) {
    int count = 0;
    char* p = line;
    while (*p && count < MAX_TOKENS) {
        while (*p == ' ') {
            p++;
        }
        if (!*p) {
            break;
        }
        tokens[count++] = p;
        while (*p && *p != ' ') {
            p++;
        }
        if (*p) {
            *p++ = '\0';
        }
    }
    return count;
}

static void cmd_help(void) {
    printf("NovaOS ring-3 shell (Phase 30/31/32) - available commands:\n");
    printf("  ls              - list files in the root directory\n");
    printf("  cat FILE        - print a file's contents\n");
    printf("  run FILE [args] - load and run a real ELF executable\n");
    printf("  echo TEXT       - print TEXT\n");
    printf("  clear           - clear the screen\n");
    printf("  date            - show the current date/time\n");
    printf("  lspci           - list PCI devices\n");
    printf("  beep            - play a short tone through AC97 audio\n");
    printf("  pkg list                - list available/installed packages\n");
    printf("  pkg install NAME        - install a package\n");
    printf("  pkg remove NAME         - remove an installed package\n");
    printf("  gui             - run a ring-3 graphics demo\n");
    printf("  help            - show this message\n");
    printf("\n");
    printf("Not yet available here: ping, nslookup, tftp (networking\n");
    printf("needs real timeout syscalls), store (the full compositor +\n");
    printf("Software Center UI - see PROGRESS.md for why 'gui' above\n");
    printf("is a proof-of-concept, not a full port).\n");
}

static void cmd_ls(void) {
    static char buf[2048];
    int n = sys_list_files(buf, sizeof(buf) - 1);
    if (n < 0) {
        printf("ls: no filesystem mounted, or listing too large\n");
        return;
    }
    buf[n] = '\0';
    sys_write(buf);
}

static void cmd_cat(int argc, char* argv[]) {
    if (argc < 2) {
        printf("usage: cat FILE\n");
        return;
    }
    int fd = sys_open(argv[1]);
    if (fd < 0) {
        printf("cat: cannot open '%s'\n", argv[1]);
        return;
    }
    char buf[257];
    int n;
    while ((n = sys_read(fd, buf, sizeof(buf) - 1)) > 0) {
        buf[n] = '\0';
        sys_write(buf);
    }
    sys_close(fd);
}

static void cmd_run(int argc, char* argv[]) {
    if (argc < 2) {
        printf("usage: run FILE [args...]\n");
        return;
    }
    int pid = sys_exec(argv[1], &argv[1], argc - 1);
    if (pid < 0) {
        printf("run: failed to load '%s'\n", argv[1]);
        return;
    }
    int exit_code = sys_wait(pid);
    printf("(pid %d exited with code %d)\n", pid, exit_code);
}

/* Phase 32b: convenience wrapper for the proof-of-concept ring-3
 * graphics demo (userland/coreutils/gui.c) - equivalent to
 * `run GUI.ELF`, no special capabilities needed since the graphics/
 * mouse syscalls it uses aren't capability-gated. Not the full
 * compositor/Store experience (`store` command) - see PROGRESS.md. */
static void cmd_gui(void) {
    const char* gui_argv[] = {"GUI.ELF"};
    int pid = sys_exec("GUI.ELF", gui_argv, 1);
    if (pid < 0) {
        printf("gui: failed to load GUI.ELF\n");
        return;
    }
    sys_wait(pid);
}

static void cmd_echo(int argc, char* argv[]) {
    for (int i = 1; i < argc; i++) {
        printf("%s", argv[i]);
        if (i < argc - 1) {
            printf(" ");
        }
    }
    printf("\n");
}

static void cmd_clear(void) {
    /* No SYS_CLEAR_SCREEN syscall exists yet - this is a practical,
     * if inelegant, substitute (scroll the whole visible area off)
     * rather than a real screen clear. A real limitation, documented
     * rather than silently approximated without comment. */
    for (int i = 0; i < 25; i++) {
        putchar('\n');
    }
}

/* Phase 31: restores date/lspci/beep command parity with the original
 * ring-0 shell, via three new syscalls (SYS_RTC_READ/SYS_LSPCI/
 * SYS_BEEP) added specifically for this. */

static void cmd_date(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    printf("%d-%d-%d %d:%d:%d\n", t.year, t.month, t.day, t.hour,
           t.minute, t.second);
}

static void cmd_lspci(void) {
    static char buf[2048];
    int n = sys_lspci(buf, sizeof(buf) - 1);
    if (n < 0) {
        printf("lspci: listing too large for the buffer\n");
        return;
    }
    buf[n] = '\0';
    sys_write(buf);
}

static void cmd_beep(void) {
    if (!sys_beep()) {
        printf("beep: no AC97 audio device detected\n");
    }
}

static bool str_eq_ci(const char* a, const char* b) {
    while (*a && *b) {
        char ca = *a, cb = *b;
        if (ca >= 'a' && ca <= 'z') {
            ca = (char)(ca - 32);
        }
        if (cb >= 'a' && cb <= 'z') {
            cb = (char)(cb - 32);
        }
        if (ca != cb) {
            return false;
        }
        a++;
        b++;
    }
    return *a == *b;
}

static bool str_ends_with_ci(const char* str, const char* suffix) {
    size_t str_len = strlen(str);
    size_t suffix_len = strlen(suffix);
    if (suffix_len > str_len) {
        return false;
    }
    return str_eq_ci(str + (str_len - suffix_len), suffix);
}

/* Parses SYS_LIST_FILES's "NAME SIZE\n" output and collects every
 * entry whose name ends in ".PKG". */
static int find_pkg_files(char names[][13], int max_names) {
    static char buf[2048];
    int n = sys_list_files(buf, sizeof(buf) - 1);
    if (n < 0) {
        return 0;
    }
    buf[n] = '\0';

    int count = 0;
    char* line = buf;
    while (*line && count < max_names) {
        char* newline = line;
        while (*newline && *newline != '\n') {
            newline++;
        }
        bool had_newline = (*newline == '\n');
        *newline = '\0';

        char* space = line;
        while (*space && *space != ' ') {
            space++;
        }
        *space = '\0';

        if (str_ends_with_ci(line, ".PKG")) {
            strncpy(names[count], line, 12);
            names[count][12] = '\0';
            count++;
        }

        line = had_newline ? newline + 1 : newline;
    }
    return count;
}

static int load_install_db(nova_install_record_t* records, int max_records) {
    int fd = sys_open(PKG_INSTALL_DB);
    if (fd < 0) {
        return 0; /* doesn't exist yet - no installed packages */
    }
    int total = 0;
    int n;
    int max_bytes = (int)(sizeof(nova_install_record_t) * (unsigned)max_records);
    while (total < max_bytes &&
           (n = sys_read(fd, (char*)records + total, max_bytes - total)) > 0) {
        total += n;
    }
    sys_close(fd);
    return total / (int)sizeof(nova_install_record_t);
}

static bool save_install_db(const nova_install_record_t* records, int count) {
    sys_delete_file(PKG_INSTALL_DB); /* ignore result: may not exist yet */
    return sys_write_file(PKG_INSTALL_DB, records,
                           (unsigned int)((unsigned)count *
                                          sizeof(nova_install_record_t))) > 0;
}

/* Phase 32: the package manager, converted to a ring-3 shell builtin.
 * Reimplements the same package format userland/pkg/pkgmgr.c's
 * kernel-side original does (see this file's earlier struct
 * definitions), using only syscalls - no kernel pkg-specific logic is
 * called at all, matching the pattern of genuinely moving logic to
 * ring-3 rather than just relocating source files. */
static void cmd_pkg(int argc, char* argv[]) {
    if (argc < 2) {
        printf("usage: pkg list|install NAME|remove NAME\n");
        return;
    }

    if (strcmp(argv[1], "list") == 0) {
        char pkg_files[PKG_MAX_INSTALLED][13];
        int pkg_count = find_pkg_files(pkg_files, PKG_MAX_INSTALLED);

        nova_install_record_t installed[PKG_MAX_INSTALLED];
        int installed_count = load_install_db(installed, PKG_MAX_INSTALLED);

        printf("Available packages:\n");
        for (int i = 0; i < pkg_count; i++) {
            nova_pkg_header_t hdr;
            int fd = sys_open(pkg_files[i]);
            if (fd < 0) {
                continue;
            }
            sys_read(fd, &hdr, sizeof(hdr));
            sys_close(fd);

            bool is_installed = false;
            for (int j = 0; j < installed_count; j++) {
                if (str_eq_ci(installed[j].name, hdr.name)) {
                    is_installed = true;
                    break;
                }
            }

            printf("  %s (%s) - %s", hdr.name, hdr.version, hdr.description);
            if (is_installed) {
                printf(" [installed]");
            }
            printf("\n");
        }
    } else if (strcmp(argv[1], "install") == 0) {
        if (argc < 3) {
            printf("usage: pkg install NAME\n");
            return;
        }

        char pkg_files[PKG_MAX_INSTALLED][13];
        int pkg_count = find_pkg_files(pkg_files, PKG_MAX_INSTALLED);

        nova_pkg_header_t hdr;
        static unsigned char payload[4096];
        bool found = false;

        for (int i = 0; i < pkg_count; i++) {
            int fd = sys_open(pkg_files[i]);
            if (fd < 0) {
                continue;
            }
            sys_read(fd, &hdr, sizeof(hdr));
            if (str_eq_ci(hdr.name, argv[2])) {
                sys_read(fd, payload, hdr.payload_size);
                sys_close(fd);
                found = true;
                break;
            }
            sys_close(fd);
        }

        if (!found) {
            printf("pkg: package '%s' not found\n", argv[2]);
            return;
        }

        nova_install_record_t installed[PKG_MAX_INSTALLED];
        int installed_count = load_install_db(installed, PKG_MAX_INSTALLED);

        for (int j = 0; j < installed_count; j++) {
            if (str_eq_ci(installed[j].name, hdr.name)) {
                printf("pkg: '%s' is already installed\n", hdr.name);
                return;
            }
        }

        char app_filename[13];
        int name_len = (int)strlen(hdr.name);
        if (name_len > 8) {
            name_len = 8;
        }
        for (int i = 0; i < name_len; i++) {
            char c = hdr.name[i];
            if (c >= 'a' && c <= 'z') {
                c = (char)(c - 32);
            }
            app_filename[i] = c;
        }
        strcpy(app_filename + name_len, ".APP");

        if (sys_write_file(app_filename, payload, hdr.payload_size) < 0) {
            printf("pkg: failed to write %s\n", app_filename);
            return;
        }

        if (installed_count < PKG_MAX_INSTALLED) {
            strcpy(installed[installed_count].name, hdr.name);
            strcpy(installed[installed_count].version, hdr.version);
            strcpy(installed[installed_count].description, hdr.description);
            strcpy(installed[installed_count].app_filename, app_filename);
            installed_count++;
            save_install_db(installed, installed_count);
        }

        printf("Installed '%s' -> %s\n", hdr.name, app_filename);
    } else if (strcmp(argv[1], "remove") == 0) {
        if (argc < 3) {
            printf("usage: pkg remove NAME\n");
            return;
        }

        nova_install_record_t installed[PKG_MAX_INSTALLED];
        int installed_count = load_install_db(installed, PKG_MAX_INSTALLED);

        int found_idx = -1;
        for (int j = 0; j < installed_count; j++) {
            if (str_eq_ci(installed[j].name, argv[2])) {
                found_idx = j;
                break;
            }
        }

        if (found_idx < 0) {
            printf("pkg: '%s' is not installed\n", argv[2]);
            return;
        }

        sys_delete_file(installed[found_idx].app_filename);

        for (int j = found_idx; j < installed_count - 1; j++) {
            installed[j] = installed[j + 1];
        }
        installed_count--;
        save_install_db(installed, installed_count);

        printf("Removed '%s'\n", argv[2]);
    } else {
        printf("usage: pkg list|install NAME|remove NAME\n");
    }
}

int main(int argc, char** argv, char** envp) {
    (void)argc;
    (void)argv;
    (void)envp;

    printf("NovaOS ring-3 shell (Phase 30) - type 'help' for commands\n");

    char line[LINE_BUF_SIZE];
    char* tokens[MAX_TOKENS];

    for (;;) {
        printf("> ");
        read_line(line, sizeof(line));

        int argc2 = tokenize(line, tokens);
        if (argc2 == 0) {
            continue;
        }

        if (strcmp(tokens[0], "help") == 0) {
            cmd_help();
        } else if (strcmp(tokens[0], "ls") == 0) {
            cmd_ls();
        } else if (strcmp(tokens[0], "cat") == 0) {
            cmd_cat(argc2, tokens);
        } else if (strcmp(tokens[0], "run") == 0) {
            cmd_run(argc2, tokens);
        } else if (strcmp(tokens[0], "echo") == 0) {
            cmd_echo(argc2, tokens);
        } else if (strcmp(tokens[0], "clear") == 0) {
            cmd_clear();
        } else if (strcmp(tokens[0], "date") == 0) {
            cmd_date();
        } else if (strcmp(tokens[0], "lspci") == 0) {
            cmd_lspci();
        } else if (strcmp(tokens[0], "beep") == 0) {
            cmd_beep();
        } else if (strcmp(tokens[0], "pkg") == 0) {
            cmd_pkg(argc2, tokens);
        } else if (strcmp(tokens[0], "gui") == 0) {
            cmd_gui();
        } else {
            printf("Unknown command: %s (try 'help')\n", tokens[0]);
        }
    }

    return 0;
}
