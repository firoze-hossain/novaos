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

/* Phase 54: kernel/rust/crashdump.rs's own real-fault entry point -
 * see that module's header comment, and kernel_panic_fault()'s own
 * implementation (kernel/init/main.c), for the full design. Used only
 * by the two call sites that already have real CPU register state at
 * the moment of a fault - kernel/arch/x86/cpu/isr.c's isr_handler()
 * and kernel/arch/x86/mm/paging.c's page_fault_handler(). Every other
 * kernel_panic() call site (an explicit, software-detected invariant
 * violation - a heap corruption check, a size-agreement check, and so
 * on - not a CPU exception) keeps calling plain kernel_panic()
 * unchanged; that function is now a thin wrapper passing NULL/false/0
 * for the fields a software-detected panic has no real CPU register
 * state for. `struct registers` (kernel/arch/x86/cpu/isr.h's
 * `registers_t`) is forward-declared here rather than included, so
 * kernel.h - included nearly everywhere in this kernel - doesn't have
 * to pull in the interrupt-frame layout just for this one prototype;
 * the two real call sites already include isr.h for their own use of
 * that type. */
struct registers;
void kernel_panic_fault(const char* message, const struct registers* regs,
                         bool has_fault_addr, uint32_t fault_addr);

// Debugging
void kernel_log(const char* format, ...);

// Version info
#define KERNEL_NAME "NovaOS"
#define KERNEL_VERSION NOVAOS_VERSION_STRING
#define KERNEL_RELEASE_DATE "2026-07-14"

#endif