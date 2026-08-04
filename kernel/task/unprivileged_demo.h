#ifndef TASK_UNPRIVILEGED_DEMO_H
#define TASK_UNPRIVILEGED_DEMO_H

/* A plain process_create_user_task() process - no capabilities at
 * all, the default for every process that isn't explicitly created
 * via process_create_sandboxed_task(). Exists specifically to prove
 * the *denied* half of Phase 17's spawn capability test: the
 * "sandbox" process (kernel/task/sandbox_demo.c) proves SYS_SPAWN
 * succeeds when granted; this one proves it's refused when it isn't -
 * a separate process rather than a third code path in sandbox_demo.c,
 * since this one's whole point is having zero capabilities, which
 * sandbox_demo already doesn't have for spawning by default anyway
 * until explicitly granted. */
void unprivileged_demo_task(void);

#endif
