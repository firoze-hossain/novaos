/*
 * timer.c - Programmable Interval Timer (PIT) driver, IRQ0
 */
#include "timer.h"
#include "../../arch/x86/cpu/irq.h"
#include "../../arch/x86/io.h"

#define PIT_CHANNEL0_DATA 0x40
#define PIT_COMMAND       0x43
#define PIT_BASE_FREQUENCY 1193182u

static volatile uint32_t ticks = 0;
static uint32_t configured_frequency_hz = 100;

static void timer_tick(registers_t* regs) {
    (void)regs;
    ticks++;
}

void timer_init(uint32_t frequency_hz) {
    if (frequency_hz == 0) {
        frequency_hz = 100;
    }
    configured_frequency_hz = frequency_hz;

    uint32_t divisor = PIT_BASE_FREQUENCY / frequency_hz;

    outb(PIT_COMMAND, 0x36); /* channel 0, lo/hi byte, mode 3 (square wave) */
    outb(PIT_CHANNEL0_DATA, (uint8_t)(divisor & 0xFF));
    outb(PIT_CHANNEL0_DATA, (uint8_t)((divisor >> 8) & 0xFF));

    register_irq_handler(0, timer_tick);
}

uint32_t timer_get_ticks(void) {
    return ticks;
}

void timer_sleep_ms(uint32_t ms) {
    uint32_t target = ticks + (ms * configured_frequency_hz) / 1000;
    while (ticks < target) {
        __asm__ volatile ("hlt");
    }
}
