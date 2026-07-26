#ifndef DRIVERS_NET_NE2000_H
#define DRIVERS_NET_NE2000_H

#include "../../include/types.h"

#define NE2000_MAX_FRAME 1518

/* Polling PIO driver for the NE2000 ISA NIC at the fixed QEMU default
 * I/O base (0x300) - no PCI enumeration, no IRQ (ISR is polled the
 * same way the ATA driver polls status), matching the rest of
 * NovaOS's driver style so far. See PROGRESS.md for the full scope
 * and what a real driver would add (IRQ-driven RX, PCI variant
 * support, multiple NICs). */
void ne2000_init(void);
bool ne2000_is_present(void);

const uint8_t* ne2000_mac_address(void);

/* Sends one raw Ethernet frame (caller fills in the 14-byte Ethernet
 * header). Returns true on success. */
bool ne2000_send(const void* frame, uint16_t length);

/* Non-blocking: copies the next received frame (if any) into `buffer`
 * (must be at least NE2000_MAX_FRAME bytes) and returns its length, or
 * 0 if nothing has arrived. */
uint16_t ne2000_receive(void* buffer);

#endif
