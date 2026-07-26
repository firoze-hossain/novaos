#ifndef DRIVERS_TIMER_H
#define DRIVERS_TIMER_H

#include "../../include/types.h"

/* Programs the 8253/8254 PIT (IRQ0) to fire at `frequency_hz` and
 * registers the tick handler. This is what will drive preemptive
 * scheduling in Phase 4 - for now it just counts ticks and offers a
 * busy-wait sleep(), which the shell uses for e.g. a "sleep" command. */
void timer_init(uint32_t frequency_hz);

uint32_t timer_get_ticks(void);

/* Busy-waits (in a low-power `hlt` loop) until `ms` milliseconds have
 * elapsed, based on the configured tick frequency. */
void timer_sleep_ms(uint32_t ms);

/* Registers a function to be called on every tick, from inside the
 * IRQ0 handler. This is how the Phase 4 scheduler gets a chance to
 * preempt the running task without timer.c needing to know anything
 * about processes or scheduling - it just calls whatever's registered
 * here, if anything. At most one hook is supported (the scheduler is
 * the only caller so far); a real multi-hook list can replace this if
 * a second caller ever needs one. */
void timer_set_tick_hook(void (*hook)(void));

#endif
