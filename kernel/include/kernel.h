#ifndef KERNEL_H
#define KERNEL_H

#include "types.h"
#include "version.h"

// Kernel entry point (called from bootloader)
void kernel_main(uint32_t multiboot_magic, uint32_t multiboot_info_addr);

// Kernel initialization functions
void kernel_early_init(uint32_t multiboot_magic, uint32_t multiboot_info_addr);
void kernel_late_init(void);
void kernel_panic(const char* message);

// Debugging
void kernel_log(const char* format, ...);

// Version info
#define KERNEL_NAME "NovaOS"
#define KERNEL_VERSION NOVAOS_VERSION_STRING
#define KERNEL_RELEASE_DATE "2026-07-14"

#endif