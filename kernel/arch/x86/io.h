/*
 * io.h - Low-level x86 I/O port access
 *
 * Every driver that talks to hardware (VGA, serial, PIT, PS/2 keyboard,
 * the 8259 PIC, ...) goes through these three primitives. Centralizing
 * them avoids the duplicated outb()/inb() copies that tend to appear in
 * every driver file and drift out of sync.
 */
#ifndef ARCH_X86_IO_H
#define ARCH_X86_IO_H

#include "../../include/types.h"

static inline void outb(uint16_t port, uint8_t value) {
    __asm__ volatile ("outb %0, %1" : : "a"(value), "Nd"(port));
}

static inline uint8_t inb(uint16_t port) {
    uint8_t ret;
    __asm__ volatile ("inb %1, %0" : "=a"(ret) : "Nd"(port));
    return ret;
}

static inline void outw(uint16_t port, uint16_t value) {
    __asm__ volatile ("outw %0, %1" : : "a"(value), "Nd"(port));
}

static inline uint16_t inw(uint16_t port) {
    uint16_t ret;
    __asm__ volatile ("inw %1, %0" : "=a"(ret) : "Nd"(port));
    return ret;
}

/* Added in Phase 13 for PCI configuration space access (ports 0xCF8/
 * 0xCFC are read/written as full 32-bit dwords) - no earlier driver
 * needed anything wider than 16 bits. */
static inline void outl(uint16_t port, uint32_t value) {
    __asm__ volatile ("outl %0, %1" : : "a"(value), "Nd"(port));
}

static inline uint32_t inl(uint16_t port) {
    uint32_t ret;
    __asm__ volatile ("inl %1, %0" : "=a"(ret) : "Nd"(port));
    return ret;
}

/* Small delay used after PIC/CMOS writes on real hardware that need the
 * bus to settle. Writing to an unused POST diagnostic port takes about
 * 1-4 microseconds and is a standard OSDev idiom. */
static inline void io_wait(void) {
    outb(0x80, 0);
}

#endif /* ARCH_X86_IO_H */
