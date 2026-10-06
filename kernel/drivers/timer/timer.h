#ifndef DRIVERS_TIMER_H
#define DRIVERS_TIMER_H

#include "../../include/types.h"

/* Phase 76: moved here from kernel/init/main.c's own boot sequence,
 * where it lived as a private #define only main.c itself could see -
 * this driver's self-registration (see timer.c's own DRIVER_REGISTER
 * call) needs a frequency to configure without main.c passing one in,
 * since DRIVER_REGISTER requires a plain void(*)(void) init function.
 * 100Hz: a reasonable-feeling default for a round-robin scheduler with
 * a handful of tasks, not tuned against anything in particular - the
 * same value and the same reasoning this constant always had, just
 * now visible to (and overridable by, should a future caller ever
 * want to) anything that includes this header rather than main.c
 * alone. */
#define TIMER_FREQUENCY_HZ 100

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

/* Phase 85: additional per-tick callbacks. The single tick hook above is the
 * scheduler's; the audio mixer needs a 10ms tick of its own to keep the
 * sound card's DMA ring fed, and it must not replace the scheduler's. A
 * listener runs from the timer interrupt, EVERY tick, so it must be short
 * and must not block. Returns false if the (small, fixed) table is full. */
bool timer_add_tick_listener(void (*fn)(void));

/* Phase 86: was the tick now being handled taken while the CPU was running
 * USER code? Meaningful only from inside a tick listener. Killing a process
 * from the tick is safe only when this is true (the process then holds no
 * kernel lock): see kernel/task/rlimit.c. */
bool timer_tick_was_user(void);

#endif
