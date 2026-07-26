/*
 * ps2mouse.c - PS/2 mouse driver (IRQ12, standard 3-byte packets)
 */
#include "ps2mouse.h"
#include "../../arch/x86/cpu/irq.h"
#include "../../arch/x86/io.h"
#include "../../include/kernel.h"

#define PS2_DATA    0x60
#define PS2_STATUS  0x64
#define PS2_COMMAND 0x64

static bool present = false;

static uint8_t packet[3];
static int packet_index = 0;

static volatile int accum_dx = 0;
static volatile int accum_dy = 0;
static volatile bool left_button = false;
static volatile bool right_button = false;
static volatile bool middle_button = false;

static void wait_input_clear(void) {
    uint32_t spins = 0;
    while (inb(PS2_STATUS) & 0x02) {
        if (++spins > 100000u) {
            return;
        }
    }
}

static void wait_output_full(void) {
    uint32_t spins = 0;
    while (!(inb(PS2_STATUS) & 0x01)) {
        if (++spins > 100000u) {
            return;
        }
    }
}

static void mouse_write(uint8_t value) {
    wait_input_clear();
    outb(PS2_COMMAND, 0xD4); /* "next byte goes to the aux/mouse port" */
    wait_input_clear();
    outb(PS2_DATA, value);
}

static uint8_t mouse_read(void) {
    wait_output_full();
    return inb(PS2_DATA);
}

static void mouse_irq_handler(registers_t* regs) {
    (void)regs;
    uint8_t byte = inb(PS2_DATA);

    if (packet_index == 0 && !(byte & 0x08)) {
        /* Always-1 sync bit not set - we're out of alignment with the
         * packet stream (e.g. a byte was missed). Drop it and wait
         * for a byte that looks like a valid packet start instead of
         * building a garbage packet. */
        return;
    }

    packet[packet_index++] = byte;
    if (packet_index < 3) {
        return;
    }
    packet_index = 0;

    uint8_t status = packet[0];
    int dx = packet[1];
    int dy = packet[2];
    if (status & 0x10) {
        dx -= 256; /* sign-extend the 9-bit X delta */
    }
    if (status & 0x20) {
        dy -= 256; /* sign-extend the 9-bit Y delta */
    }

    accum_dx += dx;
    accum_dy += -dy; /* PS/2 reports +Y as "up"; screen Y grows downward */
    left_button = (status & 0x01) != 0;
    right_button = (status & 0x02) != 0;
    middle_button = (status & 0x04) != 0;
}

void ps2mouse_init(void) {
    present = false;

    outb(PS2_COMMAND, 0xA8); /* enable the auxiliary (mouse) port */

    wait_input_clear();
    outb(PS2_COMMAND, 0x20); /* "read controller configuration byte" */
    wait_output_full();
    uint8_t config = inb(PS2_DATA);
    config |= 0x02;  /* enable IRQ12 */
    config &= ~0x20; /* ensure the mouse clock isn't disabled */

    wait_input_clear();
    outb(PS2_COMMAND, 0x60); /* "write controller configuration byte" */
    wait_input_clear();
    outb(PS2_DATA, config);

    mouse_write(0xF6); /* set defaults */
    uint8_t ack1 = mouse_read();

    mouse_write(0xF4); /* enable data reporting */
    uint8_t ack2 = mouse_read();

    if (ack1 != 0xFA || ack2 != 0xFA) {
        kernel_log("[ .. ] PS/2 mouse: no ACK during init, assuming absent\n");
        return;
    }

    /* Drain any stale byte still sitting in the controller's output
     * buffer before switching to interrupt-driven reads. Found the
     * hard way: a single leftover byte here (observed: a duplicate of
     * the enable-reporting ACK) was enough to misalign packet framing
     * by one byte forever after - the "always-1" sync bit at bit 3 of
     * a real status byte isn't a reliable resync signal on its own,
     * since a Y-delta byte can easily also have bit 3 set by chance,
     * so a misaligned stream doesn't reliably self-correct. */
    while (inb(PS2_STATUS) & 0x01) {
        (void)inb(PS2_DATA);
    }

    packet_index = 0;
    register_irq_handler(12, mouse_irq_handler);

    present = true;
    kernel_log("[ OK ] PS/2 mouse initialized (IRQ12)\n");
}

bool ps2mouse_is_present(void) {
    return present;
}

mouse_state_t ps2mouse_read(void) {
    mouse_state_t state;
    state.dx = accum_dx;
    state.dy = accum_dy;
    state.left_button = left_button;
    state.right_button = right_button;
    state.middle_button = middle_button;

    accum_dx = 0;
    accum_dy = 0;

    return state;
}
