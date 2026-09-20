/*
 * tss.c - Task State Segment (software task switching support only)
 */
#include "tss.h"
#include "gdt.h"
#include "../../../include/smp.h"
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

/* Phase 57: one TSS per schedulable CPU - see tss.h's own comment. */
static struct tss_entry tss[SCHED_MAX_CPUS];

static inline void ltr(uint16_t selector) {
    __asm__ volatile ("ltr %0" : : "r"(selector));
}

void tss_init(void) {
    for (int cpu = 0; cpu < SCHED_MAX_CPUS; cpu++) {
        uint32_t base = (uint32_t)&tss[cpu];
        uint32_t limit = base + sizeof(tss[cpu]);

        memset(&tss[cpu], 0, sizeof(tss[cpu]));

        tss[cpu].ss0 = GDT_KERNEL_DATA;
        tss[cpu].esp0 = 0; /* set for real before that CPU's first
                               switch into ring 3 */
        /* iomap_base >= sizeof(tss) means "no I/O permission bitmap":
         * every port access from ring 3 traps (#GP), matching
         * NovaOS's current "user code never touches hardware
         * directly" model - true on every CPU alike. */
        tss[cpu].iomap_base = sizeof(tss[cpu]);

        /* 0x89 = present, ring 0, type 0x9 (32-bit TSS, not busy).
         * Byte granularity (not 4KB pages) since the TSS is only
         * ~104 bytes. Every CPU's descriptor is installed here, by
         * the BSP, in this one boot-time call - see gdt.h's own
         * comment for why this is safe to do all at once even for
         * APs that don't exist yet. */
        gdt_set_gate(GDT_TSS_GATE_INDEX(cpu), base, limit, 0x89, 0x00);
    }
}

void tss_load_this_cpu(uint8_t cpu_index) {
    ltr(GDT_TSS_SELECTOR(cpu_index));
}

void tss_set_kernel_stack(uint8_t cpu_index, uint32_t esp0) {
    if (cpu_index < SCHED_MAX_CPUS) {
        tss[cpu_index].esp0 = esp0;
    }
}
