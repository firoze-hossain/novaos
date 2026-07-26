/*
 * keyboard.c - PS/2 keyboard driver, IRQ1
 *
 * Decodes Scan Code Set 1 into ASCII (US QWERTY) and feeds a small ring
 * buffer that keyboard_get_char() drains. Keeping the IRQ handler itself
 * tiny (read port, decode, buffer) matters for security and stability:
 * it runs with interrupts re-armed only after `iret`, so a slow or
 * buggy handler here would stall every other interrupt source
 * (including the timer) for as long as it ran.
 */
#include "keyboard.h"
#include "scancodes.h"
#include "../../arch/x86/cpu/irq.h"
#include "../../arch/x86/io.h"

#define KEYBOARD_DATA_PORT 0x60
#define BUFFER_SIZE 256

static char buffer[BUFFER_SIZE];
static uint32_t buf_head = 0; /* next slot to write */
static uint32_t buf_tail = 0; /* next slot to read  */

static bool shift_held = false;

static void buffer_push(char c) {
    uint32_t next = (buf_head + 1) % BUFFER_SIZE;
    if (next == buf_tail) {
        return; /* buffer full: drop the character rather than corrupt state */
    }
    buffer[buf_head] = c;
    buf_head = next;
}

static void keyboard_irq_handler(registers_t* regs) {
    (void)regs;
    uint8_t scancode = inb(KEYBOARD_DATA_PORT);

    if (scancode == SC_LSHIFT || scancode == SC_RSHIFT) {
        shift_held = true;
        return;
    }
    if (scancode == (SC_LSHIFT | 0x80) || scancode == (SC_RSHIFT | 0x80)) {
        shift_held = false;
        return;
    }

    /* Bit 7 set means "key released" (break code) - ignore for every
     * other key; we only act on key-down for a simple text buffer. */
    if (scancode & 0x80) {
        return;
    }

    if (scancode >= 128) {
        return;
    }

    char c = shift_held ? scancode_ascii_upper[scancode]
                         : scancode_ascii_lower[scancode];
    if (c != 0) {
        buffer_push(c);
    }
}

void keyboard_init(void) {
    buf_head = 0;
    buf_tail = 0;
    shift_held = false;
    register_irq_handler(1, keyboard_irq_handler);
}

bool keyboard_has_char(void) {
    return buf_head != buf_tail;
}

char keyboard_get_char(void) {
    while (!keyboard_has_char()) {
        __asm__ volatile ("hlt");
    }
    char c = buffer[buf_tail];
    buf_tail = (buf_tail + 1) % BUFFER_SIZE;
    return c;
}
