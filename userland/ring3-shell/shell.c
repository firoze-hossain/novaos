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
    printf("NovaOS ring-3 shell (Phase 30) - available commands:\n");
    printf("  ls              - list files in the root directory\n");
    printf("  cat FILE        - print a file's contents\n");
    printf("  run FILE [args] - load and run a real ELF executable\n");
    printf("  echo TEXT       - print TEXT\n");
    printf("  clear           - clear the screen\n");
    printf("  help            - show this message\n");
    printf("\n");
    printf("Not yet available here (need new syscalls not built in\n");
    printf("this phase): ping, nslookup, tftp, pkg, store, beep, date,\n");
    printf("lspci. See PROGRESS.md.\n");
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
        } else {
            printf("Unknown command: %s (try 'help')\n", tokens[0]);
        }
    }

    return 0;
}
