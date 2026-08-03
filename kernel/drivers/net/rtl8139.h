#ifndef DRIVERS_NET_RTL8139_H
#define DRIVERS_NET_RTL8139_H

#include "../../include/types.h"

#define RTL8139_MAX_FRAME 1518

/* PCI-based NIC driver (Realtek RTL8139, vendor 0x10EC device 0x8139),
 * found via Phase 13's PCI enumeration rather than a fixed I/O base
 * the way the ISA NE2000 driver (Phase 6) uses - this is the payoff
 * PCI detection was building toward. Polling, no IRQ, same driver
 * style as NE2000 for consistency.
 *
 * Architecturally different from NE2000 in one important way: the
 * RTL8139 DMAs directly to/from physical system memory addresses the
 * driver gives it (a receive ring buffer, four transmit buffer slots)
 * rather than using onboard NIC memory reached through a page-indexed
 * remote-DMA protocol. Every buffer this driver hands the card must
 * therefore be physically contiguous and at an address the card can
 * reach directly - satisfied here simply by using static buffers
 * within the kernel image, which live in NovaOS's identity-mapped low
 * memory (virtual address == physical address, see kernel/arch/x86/mm
 * /paging.c) - there is no general physical-memory-allocation-for-DMA
 * API yet (see PROGRESS.md). */
void rtl8139_init(void);
bool rtl8139_is_present(void);

const uint8_t* rtl8139_mac_address(void);

bool rtl8139_send(const void* frame, uint16_t length);
uint16_t rtl8139_receive(void* buffer);

#endif
