#ifndef TASK_SANDBOX_DEMO_H
#define TASK_SANDBOX_DEMO_H

/* A ring-3 task created with a capability list containing only
 * "HELLO.TXT" for files and only the network gateway for network
 * destinations (see kernel_main()'s process_create_sandboxed_task()
 * call). Proves the Phase 11/14 capability enforcement is real, not
 * just present in the code, by actually trying an allowed and a
 * disallowed case for each resource type and reporting PASS/FAIL for
 * each - the same falsifiable-test approach Phase 5 used to prove
 * address-space isolation. */
void sandbox_demo_task(void);

#endif
