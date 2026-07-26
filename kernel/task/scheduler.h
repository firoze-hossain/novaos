#ifndef TASK_SCHEDULER_H
#define TASK_SCHEDULER_H

#include "process.h"

/* Registers a newly-created process as eligible to run. Doesn't
 * maintain a separate ready queue - round robin currently just scans
 * the whole process table each time it needs to pick a task (see
 * scheduler.c), which is simple and correct at MAX_PROCESSES=16 but
 * would want a real queue if that ever grows much larger. */
void scheduler_add(process_t* p);

/* Picks the first eligible process and switches into it. Called once,
 * after every initial task has been created; never returns. */
void scheduler_start(void);

/* Called from the timer IRQ (see kernel/init/main.c's timer tick
 * hook). Preempts the current process in favor of the next READY one,
 * round robin, if there is one. */
void scheduler_on_tick(void);

/* Voluntary reschedule - used by process_exit_current() and (later)
 * a SYS_YIELD syscall. */
void scheduler_yield(void);

process_t* scheduler_current(void);

#endif
