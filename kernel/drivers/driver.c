/*
 * driver.c - Phase 39: driver self-registration - see driver.h for the
 * full design and rationale.
 */
#include "driver.h"
#include "../include/kernel.h"

/* Provided by tools/linker.ld - bound the `.drivers` section, an
 * array of `driver_t` with no gaps (each entry placed back-to-back by
 * DRIVER_REGISTER), not individually declared/exported symbols. Zero
 * registered drivers (an empty section) is valid and already handled
 * correctly by the loop below - __drivers_start == __drivers_end,
 * same as any other empty array. */
extern driver_t __drivers_start[];
extern driver_t __drivers_end[];

void driver_init_all(driver_phase_t phase) {
    for (driver_t* d = __drivers_start; d < __drivers_end; d++) {
        if (d->phase != phase) {
            continue;
        }
        kernel_log("[ OK ] Driver '%s' initializing...\n", d->name);
        d->init();
    }
}
