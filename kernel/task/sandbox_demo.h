#ifndef TASK_SANDBOX_DEMO_H
#define TASK_SANDBOX_DEMO_H

/* A ring-3 task created with a capability list containing only
 * "HELLO.TXT" (see kernel_main()'s process_create_sandboxed_task()
 * call). Proves the Phase 11 capability enforcement is real, not just
 * present in the code, by actually trying both an allowed and a
 * disallowed SYS_OPEN and reporting PASS/FAIL for each - the same
 * falsifiable-test approach Phase 5 used to prove address-space
 * isolation. */
void sandbox_demo_task(void);

#endif
