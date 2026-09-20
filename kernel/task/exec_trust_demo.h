#ifndef TASK_EXEC_TRUST_DEMO_H
#define TASK_EXEC_TRUST_DEMO_H

/* Phase 59: a real ring-3 task, created (see kernel_main()'s
 * process_create_sandboxed_task_trusted() call) with can_spawn and
 * can_open_any_file both granted - proving SYS_EXEC_TRUSTED's own
 * capability-delegation model (see kernel/arch/x86/cpu/syscall.h's
 * comment on SYS_EXEC_TRUSTED, and kernel/task/process.c's comment on
 * process_exec_trusted()) is real, not just present in the code, the
 * same falsifiable-test approach sandbox_demo_task() already
 * established for SYS_NET_SEND/SYS_OPEN. Execs the same tiny on-disk
 * probe program (userland/coreutils/tprobe.c -> TPROBE.ELF) two ways
 * from the same trusted caller - once via plain SYS_EXEC (expected to
 * get nothing delegated, exactly like an ordinary program), once via
 * SYS_EXEC_TRUSTED (expected to inherit this task's own
 * can_open_any_file) - and reports PASS only if the two calls
 * genuinely produced different outcomes. */
void exec_trust_demo_task(void);

#endif
