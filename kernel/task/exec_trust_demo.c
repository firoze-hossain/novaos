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

    /* Phase 73: the same trusted-exec vehicle, reused to run the in-OS
     * half of the libc test (userland/libctest/libctest.c). It needs
     * exactly the two capabilities this task already holds and
     * SYS_EXEC_TRUSTED delegates - can_open_any_file (it creates, reads,
     * and deletes real files on FAT32 and reads real files on ext2) and
     * can_spawn (it execs itself as a child to test environment
     * inheritance). LIBCTEST.ELF prints its own per-area
     * "[libctest] PASS/FAIL: ..." lines, which tools/python/
     * test_runner.py checks individually; this only reports whether the
     * whole run exited 0. */
    const char* argv_lc[] = {"LIBCTEST.ELF"};
    int pid_lc = sys_exec_trusted("LIBCTEST.ELF", argv_lc, 1);
    int code_lc = (pid_lc >= 0) ? sys_wait(pid_lc) : -1;
    sys_write(pid_lc >= 0 && code_lc == 0
                  ? "[sandbox] PASS: LIBCTEST.ELF (the libc test) ran to "
                    "completion with every group passing.\n"
                  : "[sandbox] FAIL: LIBCTEST.ELF (the libc test) did not "
                    "exit cleanly.\n");

    /* Phase 74: real dynamic linking. DYNTEST.ELF and DYNTEST2.ELF are
     * two SEPARATE, independently built executables that both hold a
     * DT_NEEDED reference to the one DYNLIB.SO file on disk - see
     * kernel/task/process.c's load_and_link_shared_libraries() and
     * kernel/rust/dynlink.rs for what actually resolves that at exec
     * time. Plain sys_exec() would work just as well as the trusted
     * form here - loading a shared library needs no special capability
     * (see load_and_link_shared_libraries()'s own comment: its
     * vfs_read_file() calls are unconditional kernel-mode reads,
     * exactly like the main executable's own load) - sys_exec_trusted()
     * is used only for consistency with this function's other calls. */
    const char* argv_dt[] = {"DYNTEST.ELF"};
    int pid_dt = sys_exec_trusted("DYNTEST.ELF", argv_dt, 1);
    int code_dt = (pid_dt >= 0) ? sys_wait(pid_dt) : -1;
    sys_write(pid_dt >= 0 && code_dt == 0
                  ? "[sandbox] PASS: DYNTEST.ELF (dynamic linking against "
                    "DYNLIB.SO) ran to completion with every group "
                    "passing.\n"
                  : "[sandbox] FAIL: DYNTEST.ELF (dynamic linking) did not "
                    "exit cleanly.\n");

    const char* argv_dt2[] = {"DYNTEST2.ELF"};
    int pid_dt2 = sys_exec_trusted("DYNTEST2.ELF", argv_dt2, 1);
    int code_dt2 = (pid_dt2 >= 0) ? sys_wait(pid_dt2) : -1;
    sys_write(pid_dt2 >= 0 && code_dt2 == 0
                  ? "[sandbox] PASS: DYNTEST2.ELF (a second, independent "
                    "consumer of the same DYNLIB.SO) ran to completion "
                    "with every group passing.\n"
                  : "[sandbox] FAIL: DYNTEST2.ELF did not exit cleanly.\n");

    sys_exit(0);

    for (;;) { }
}
