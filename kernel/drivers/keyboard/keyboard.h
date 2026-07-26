#ifndef DRIVERS_KEYBOARD_H
#define DRIVERS_KEYBOARD_H

#include "../../include/types.h"

/* Registers the IRQ1 handler and unmasks the line on the PIC. */
void keyboard_init(void);

/* Non-blocking: returns true if a decoded character is waiting. */
bool keyboard_has_char(void);

/* Blocking (sleeps in a low-power `hlt` loop between IRQs): returns the
 * next decoded ASCII character, consuming it from the input buffer. */
char keyboard_get_char(void);

#endif
