/*
 * gdt.c - Global Descriptor Table
 *
 * NovaOS uses the standard "flat memory model": every segment spans the
 * full 4GB address space and all real memory protection comes from
 * paging (see mm/paging.c, Phase 2 follow-up) rather than segmentation.
 * We still define separate ring 0 / ring 3 code+data segments because
 * that ring boundary is what will let user processes (Phase 4) run
 * without being able to touch kernel memory or execute privileged
 * instructions - this is the first brick of NovaOS's security model.
 */
#include "gdt.h"

struct gdt_entry {
    uint16_t limit_low;
    uint16_t base_low;
    uint8_t  base_middle;
    uint8_t  access;
    uint8_t  granularity;
    uint8_t  base_high;
} __attribute__((packed));

struct gdt_ptr {
    uint16_t limit;
    uint32_t base;
} __attribute__((packed));

#define GDT_ENTRIES 6

static struct gdt_entry gdt[GDT_ENTRIES];
static struct gdt_ptr   gdt_pointer;

/* Implemented in gdt_flush.asm. Loads the GDTR via LGDT and reloads
 * every segment register so the CPU actually uses the new table. */
extern void gdt_flush(uint32_t gdt_ptr_addr);

void gdt_set_gate(int32_t num, uint32_t base, uint32_t limit,
                   uint8_t access, uint8_t gran) {
    gdt[num].base_low    = (uint16_t)(base & 0xFFFF);
    gdt[num].base_middle = (uint8_t)((base >> 16) & 0xFF);
    gdt[num].base_high   = (uint8_t)((base >> 24) & 0xFF);

    gdt[num].limit_low   = (uint16_t)(limit & 0xFFFF);
    gdt[num].granularity = (uint8_t)((limit >> 16) & 0x0F);

    gdt[num].granularity |= (gran & 0xF0);
    gdt[num].access       = access;
}

void gdt_init(void) {
    gdt_pointer.limit = (uint16_t)(sizeof(struct gdt_entry) * GDT_ENTRIES - 1);
    gdt_pointer.base  = (uint32_t)&gdt;

    /* Null descriptor - required by the x86 architecture. */
    gdt_set_gate(0, 0, 0, 0, 0);

    /* Kernel code: base 0, limit 4GB, ring 0, executable/readable. */
    gdt_set_gate(1, 0, 0xFFFFFFFF, 0x9A, 0xCF);
    /* Kernel data: base 0, limit 4GB, ring 0, writable. */
    gdt_set_gate(2, 0, 0xFFFFFFFF, 0x92, 0xCF);
    /* User code: base 0, limit 4GB, ring 3, executable/readable. */
    gdt_set_gate(3, 0, 0xFFFFFFFF, 0xFA, 0xCF);
    /* User data: base 0, limit 4GB, ring 3, writable. */
    gdt_set_gate(4, 0, 0xFFFFFFFF, 0xF2, 0xCF);

    gdt_flush((uint32_t)&gdt_pointer);
}
