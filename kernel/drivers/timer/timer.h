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

#endif
