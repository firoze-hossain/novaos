/*
 * hello.c - a real C program for NovaOS, proving the minimal libc
 * port (Phase 24) actually works: printf with %d/%s, real argv
 * access, and malloc/strcpy/strcat/free, not just raw syscalls the
 * way tools/elf-fixtures/hello.asm (Phase 23) exercised.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char** argv, char** envp) {
    (void)envp; /* always empty for now - see PROGRESS.md */

    printf("Hello from a REAL C program on NovaOS!\n");
    printf("argc = %d\n", argc);
    for (int i = 0; i < argc; i++) {
        printf("argv[%d] = %s\n", i, argv[i]);
    }

    char* buf = (char*)malloc(64);
    if (buf == NULL) {
        printf("malloc failed!\n");
        return 1;
    }
    strcpy(buf, "malloc'd string: ");
    strcat(buf, "it works!");
    printf("%s\n", buf);
    free(buf);

    return 7; /* a specific, checkable exit code - distinct from
                 HELLO.ELF's 42, so make test can tell the two test
                 executables' results apart */
}
