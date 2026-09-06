/*
 * cat.c - the first genuine ring-3 "coreutils" program (Phase 29).
 *
 * This is the real proof of the kernel/userland separation this
 * phase is about: unlike userland/shell/shell.c (still compiled
 * directly into the kernel binary as a ring-0 task, calling
 * vfs_read_file() and friends as plain C function calls), this file
 * is compiled completely separately into its own standalone ELF32
 * executable and runs in ring 3, talking to the kernel *only*
 * through the same syscall interface (SYS_OPEN/SYS_READ/SYS_CLOSE)
 * any real userland program would use - the same category of thing
 * `cat` on Ubuntu is to the Linux kernel. See PROGRESS.md for the
 * honest scope note on how much of userland/shell/gui/pkg is *not*
 * yet converted this way, and why.
 */
#include <novasys.h>

int main(int argc, char** argv, char** envp) {
    (void)envp;

    if (argc < 2) {
        sys_write("usage: cat FILENAME\n");
        return 1;
    }

    int fd = sys_open(argv[1]);
    if (fd < 0) {
        sys_write("cat: cannot open file\n");
        return 1;
    }

    char buf[257];
    int n;
    while ((n = sys_read(fd, buf, sizeof(buf) - 1)) > 0) {
        buf[n] = '\0';
        sys_write(buf);
    }

    sys_close(fd);
    return 0;
}
