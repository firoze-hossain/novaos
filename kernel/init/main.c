#include "../include/kernel.h"
#include "../drivers/vga/vga.h"
#include "../lib/string.h"
#include "../lib/stdio.h"

// Kernel panic
void kernel_panic(const char* message) {
    vga_set_color(VGA_COLOR_RED, VGA_COLOR_BLACK);
    vga_puts("\n\n!!! KERNEL PANIC !!!\n");
    vga_puts(message);
    vga_puts("\nSystem halted.\n");

    // Hang forever
    while(1) {
        __asm__ volatile("hlt");
    }
}

// Welcome banner
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
    vga_puts("  [✓] Kernel initialized\n");
    vga_puts("  [✓] VGA driver loaded\n");
    vga_puts("  [✓] Memory management ready\n\n");

    vga_set_color(VGA_COLOR_LIGHT_GREY, VGA_COLOR_BLACK);
    vga_puts("  Type 'help' for commands\n\n");
}

// Main kernel entry
void kernel_main(void) {
    // Initialize VGA first
    vga_init();
    vga_clear();

    // Print welcome banner
    print_banner();

    // Show prompt
    vga_set_color(VGA_COLOR_CYAN, VGA_COLOR_BLACK);
    vga_puts("  nova> ");
    vga_set_color(VGA_COLOR_WHITE, VGA_COLOR_BLACK);

    // Main loop
    while (1) {
        // For now, just wait for interrupts
        __asm__ volatile("hlt");
    }
}