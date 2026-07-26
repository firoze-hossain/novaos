/*
 * tss.c - Task State Segment (software task switching support only)
 */
#include "tss.h"
#include "gdt.h"
#include "../../lib/string.h"

struct tss_entry {
    uint32_t prev_tss;
    uint32_t esp0;   /* ring-0 stack pointer to load on a ring3->ring0 trap */
    uint32_t ss0;    /* ring-0 stack segment, likewise */
    uint32_t esp1, ss1, esp2, ss2;
    uint32_t cr3;
    uint32_t eip, eflags;
    uint32_t eax, ecx, edx, ebx, esp, ebp, esi, edi;
    uint32_t es, cs, ss, ds, fs, gs;
    uint32_t ldt;
    uint16_t trap;
    uint16_t iomap_base;
} __attribute__((packed));

static struct tss_entry tss;

static inline void ltr(uint16_t selector) {
    __asm__ volatile ("ltr %0" : : "r"(selector));
}

void tss_init(void) {
    uint32_t base = (uint32_t)&tss;
    uint32_t limit = base + sizeof(tss);

    memset(&tss, 0, sizeof(tss));

    tss.ss0 = GDT_KERNEL_DATA;
    tss.esp0 = 0; /* set for real before the first switch into ring 3 */
    /* iomap_base >= sizeof(tss) means "no I/O permission bitmap": every
     * port access from ring 3 traps (#GP), matching NovaOS's current
     * "user code never touches hardware directly" model. */
    tss.iomap_base = sizeof(tss);

    /* 0x89 = present, ring 0, type 0x9 (32-bit TSS, not busy). Byte
     * granularity (not 4KB pages) since the TSS is only ~104 bytes. */
    gdt_set_gate(5, base, limit, 0x89, 0x00);

    ltr(GDT_TSS_SELECTOR);
}

void tss_set_kernel_stack(uint32_t esp0) {
    tss.esp0 = esp0;
}
