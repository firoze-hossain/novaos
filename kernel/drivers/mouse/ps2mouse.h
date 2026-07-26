#ifndef DRIVERS_MOUSE_PS2MOUSE_H
#define DRIVERS_MOUSE_PS2MOUSE_H

#include "../../include/types.h"

typedef struct {
    int dx;    /* relative movement since the last read, signed */
    int dy;    /* positive = moved down on screen */
    bool left_button;
    bool right_button;
    bool middle_button;
} mouse_state_t;

/* Enables the 8042 controller's auxiliary (mouse) port, tells the
 * mouse to use default settings and start reporting, and registers
 * the IRQ12 handler that assembles the standard 3-byte packet format. */
void ps2mouse_init(void);
bool ps2mouse_is_present(void);

/* Returns the accumulated relative movement/button state since the
 * last call, then resets the accumulator - a typical "read deltas"
 * polling interface for a compositor's render loop. */
mouse_state_t ps2mouse_read(void);

#endif
