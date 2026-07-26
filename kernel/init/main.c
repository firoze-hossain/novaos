#include "../include/kernel.h"
#include "../drivers/vga/vga.h"
#include "../drivers/serial/serial.h"
#include "../drivers/timer/timer.h"
#include "../drivers/keyboard/keyboard.h"
#include "../arch/x86/cpu/gdt.h"
#include "../arch/x86/cpu/idt.h"
#include "../arch/x86/mm/heap.h"
#include "../shell/shell.h"
#include "../lib/string.h"
#include "../lib/stdio.h"
#include <stdarg.h>

#define TIMER_FREQUENCY_HZ 100

/* kernel_log() is the one logging function every subsystem uses. It
 * always goes to serial (visible in `make debug`, CI, and
 * scripts/test.sh's boot-log assertions) and is intentionally silent
 * on the VGA console so boot messages don't clutter the screen the
 * user actually interacts with. */
void kernel_log(const char* format, ...) {
    va_list args;
    va_start(args, format);

    char buffer[256];
    vsnprintf(buffer, sizeof(buffer), format, args);
    serial_puts(buffer);

    va_end(args);
}

void kernel_panic(const char* message) {
    __asm__ volatile ("cli");

    kernel_log("[PANIC] %s\n", message);

    vga_set_color(VGA_COLOR_RED, VGA_COLOR_BLACK);
    vga_puts("\n\n!!! KERNEL PANIC !!!\n");
    vga_puts(message);
    vga_puts("\nSystem halted.\n");

    while (1) {
        __asm__ volatile ("hlt");
    }
}

/* Everything that must happen before interrupts are safe to enable:
 * segmentation (GDT), the interrupt table itself (IDT/ISR/IRQ, which
 * also remaps and fully masks the PIC), and the heap, since several
 * drivers will want to kmalloc() during their own init in later phases. */
void kernel_early_init(void) {
    serial_init();
    kernel_log("NovaOS booting (kernel v%s)...\n", KERNEL_VERSION);

    gdt_init();
    kernel_log("[ OK ] GDT initialized\n");

    idt_init();
    kernel_log("[ OK ] IDT/ISR/IRQ initialized, PIC remapped to 0x20-0x2F\n");

    heap_init();
    kernel_log("[ OK ] Heap initialized (2 MB arena)\n");
}

/* Everything that needs interrupts to exist as a concept, but not yet
 * enabled: driver registration. IF is turned on right at the end, once
 * every handler a currently-unmasked IRQ could call is already in place. */
void kernel_late_init(void) {
    timer_init(TIMER_FREQUENCY_HZ);
    kernel_log("[ OK ] PIT timer initialized at %d Hz (IRQ0)\n",
               TIMER_FREQUENCY_HZ);

    keyboard_init();
    kernel_log("[ OK ] PS/2 keyboard initialized (IRQ1)\n");

    __asm__ volatile ("sti");
    kernel_log("[ OK ] Interrupts enabled\n");
}

static void print_banner(void) {
    vga_set_color(VGA_COLOR_CYAN, VGA_COLOR_BLACK);
    vga_puts("   _   _                  _____  _____ \n");
    vga_puts("  | \\ | |                / ____|/ ____|\n");
    vga_puts("  |  \\| | _____   _____| (___ | (___  \n");
    vga_puts("  | . ` |/ _ \\ \\ / / _ \\\\___ \\ \\___ \\ \n");
    vga_puts("  | |\\  | (_) \\ V / (_) |___) |____) |\n");
    vga_puts("  |_| \\_|\\___/ \\_/ \\___/_____/_____/ \n\n");

    vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);
    vga_printf("  " KERNEL_NAME " v" KERNEL_VERSION "\n");
    vga_printf("  Built on " __DATE__ " at " __TIME__ "\n\n");

    vga_set_color(VGA_COLOR_GREEN, VGA_COLOR_BLACK);
    vga_puts("  [OK] GDT / IDT / PIC configured\n");
    vga_puts("  [OK] PIT timer + PS/2 keyboard online\n");
    vga_puts("  [OK] Kernel heap ready\n\n");

    vga_set_color(VGA_COLOR_LIGHT_GREY, VGA_COLOR_BLACK);
    vga_puts("  Type 'help' for a list of commands\n\n");
}

void kernel_main(void) {
    kernel_early_init();

    vga_init();
    vga_clear();

    kernel_late_init();

    print_banner();
    shell_run(); /* never returns */
}
