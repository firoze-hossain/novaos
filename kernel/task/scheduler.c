/*
 * scheduler.c - round-robin preemptive scheduler
 */
#include "scheduler.h"
#include "../arch/x86/cpu/tss.h"
#include "../arch/x86/mm/paging.h"
#include "../include/kernel.h"

extern void switch_context(uint32_t* old_esp_out, uint32_t new_esp);

static process_t* current = NULL;
static int current_index = -1;
static uint32_t startup_esp; /* throwaway - see scheduler_start() */

void scheduler_add(process_t* p) {
    (void)p; /* nothing to do yet - see the header comment */
}

static process_t* pick_next(void) {
    for (int i = 1; i <= MAX_PROCESSES; i++) {
        int idx = (current_index + i) % MAX_PROCESSES;
        if (idx < 0) {
            idx += MAX_PROCESSES;
        }
        process_t* p = process_table_entry(idx);
        if (p != NULL &&
            (p->state == PROCESS_READY || p->state == PROCESS_RUNNING)) {
            current_index = idx;
            return p;
        }
    }
    return NULL;
}

static void do_schedule(void) {
    if (current == NULL) {
        return; /* scheduler_start() hasn't run yet */
    }

    process_t* prev = current;
    process_t* next = pick_next();

    if (next == NULL) {
        /* Nothing eligible at all - shouldn't happen, the idle task
         * never terminates, but fail safe rather than switch into
         * garbage. */
        return;
    }

    if (next == prev) {
        prev->state = PROCESS_RUNNING; /* nothing else ready; keep going */
        return;
    }

    if (prev->state == PROCESS_RUNNING) {
        prev->state = PROCESS_READY;
    }
    next->state = PROCESS_RUNNING;
    current = next;

    tss_set_kernel_stack(next->kernel_stack_top);
    paging_switch_address_space(next->page_directory_phys);
    switch_context(&prev->esp, next->esp);
    /* Execution only reaches here once `prev` is chosen to run again
     * by some future switch_context() call - i.e. this line "returns"
     * an arbitrary number of scheduler ticks later, quite normal for
     * this kind of switch. */
}

void scheduler_start(void) {
    process_t* first = pick_next();
    if (first == NULL) {
        kernel_panic("scheduler_start: no processes to run");
    }

    current = first;
    first->state = PROCESS_RUNNING;
    tss_set_kernel_stack(first->kernel_stack_top);
    paging_switch_address_space(first->page_directory_phys);

    switch_context(&startup_esp, first->esp);
    /* Never returns: the boot stack this call happened on is now
     * permanently abandoned. */
}

void scheduler_on_tick(void) {
    do_schedule();
}

void scheduler_yield(void) {
    do_schedule();
}

process_t* scheduler_current(void) {
    return current;
}
