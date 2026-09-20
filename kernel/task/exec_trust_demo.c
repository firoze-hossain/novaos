/*
 * exec_trust_demo.c - proves Phase 59's SYS_EXEC_TRUSTED capability
 * delegation is real
 *
 * See exec_trust_demo.h for the full reasoning. This file's own
 * syscall wrappers are deliberately duplicated raw `int 0x80` inline
 * asm, not shared with userland/libc/syscall.c - the same "no shared
 * bridge header" FFI convention kernel/task/sandbox_demo.c already
 * established for exactly this situation (a kernel-compiled ring-3
 * task, not a separately-built userland program).
 */
#include "exec_trust_demo.h"
#include "../arch/x86/cpu/syscall.h"

static inline void sys_write(const char* str) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_WRITE), "b"(str) : "memory", "cc");
}

static inline int sys_exec(const char* path, const char** argv, int argc) {
    int result = SYS_EXEC;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc)
                       : "memory", "cc");
    return result;
}

/* Phase 59: the one new syscall this file exists to exercise - see
 * kernel/arch/x86/cpu/syscall.h's own comment on SYS_EXEC_TRUSTED for
 * the full contract. Same EBX/ECX/EDX convention as sys_exec() above,
 * different syscall number. */
static inline int sys_exec_trusted(const char* path, const char** argv,
                                    int argc) {
    int result = SYS_EXEC_TRUSTED;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc)
                       : "memory", "cc");
    return result;
}

static inline int sys_wait(int pid) {
    int result = SYS_WAIT;
    __asm__ volatile ("int $0x80" : "+a"(result) : "b"(pid) : "memory", "cc");
    return result;
}

static inline void sys_exit(int exit_code) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT), "b"(exit_code));
}

void exec_trust_demo_task(void) {
    sys_write("\n[sandbox] Starting SYS_EXEC_TRUSTED self-test; this task "
              "was created with can_spawn and can_open_any_file both "
              "granted directly (process_create_sandboxed_task_trusted()) "
              "so it can exec TPROBE.ELF two different ways.\n");

    const char* argv0[] = {"TPROBE.ELF"};

    /* First: plain SYS_EXEC. Even though *this* task has can_open_any_
     * file, plain SYS_EXEC never delegates it - the new process should
     * get nothing, exactly like any other ordinary program, and
     * TPROBE.ELF's own first SYS_WRITE_FILE call should fail
     * immediately, making it exit 1. */
    int pid_plain = sys_exec("TPROBE.ELF", argv0, 1);
    int code_plain = (pid_plain >= 0) ? sys_wait(pid_plain) : -1;

    /* Second: SYS_EXEC_TRUSTED. This task's own can_open_any_file
     * (true) should be delegated to the new process, so TPROBE.ELF's
     * write/read-back/delete/confirm-deleted sequence should all
     * succeed this time, exiting 0. */
    int pid_trusted = sys_exec_trusted("TPROBE.ELF", argv0, 1);
    int code_trusted = (pid_trusted >= 0) ? sys_wait(pid_trusted) : -1;

    if (code_plain != 0 && code_trusted == 0) {
        sys_write("[sandbox] PASS: SYS_EXEC_TRUSTED delegates can_open_any_"
                  "file correctly (plain SYS_EXEC to TPROBE.ELF got "
                  "nothing and it failed; SYS_EXEC_TRUSTED delegated this "
                  "task's own capability and it succeeded).\n");
    } else {
        sys_write("[sandbox] FAIL: SYS_EXEC_TRUSTED capability delegation "
                  "did not behave as expected.\n");
    }

    sys_exit(0);

    for (;;) { }
}
