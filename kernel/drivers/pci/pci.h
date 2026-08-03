#ifndef DRIVERS_PCI_H
#define DRIVERS_PCI_H

#include "../../include/types.h"

/* PCI configuration space access via the legacy I/O ports (0xCF8/
 * 0xCFC) every x86 chipset since the mid-90s supports, regardless of
 * whether it also offers the newer memory-mapped (MMCONFIG/ECAM)
 * mechanism - the simplest correct way to talk to PCI, and the only
 * one this driver implements. See PROGRESS.md for what that leaves
 * out (no MMCONFIG, no PCIe extended config space beyond the
 * original 256-byte layout). */

typedef struct {
    uint8_t bus;
    uint8_t device;
    uint8_t function;
    uint16_t vendor_id;
    uint16_t device_id;
    uint8_t class_code;
    uint8_t subclass;
    uint8_t prog_if;
    uint8_t revision;
    uint8_t header_type;
} pci_device_t;

typedef void (*pci_enum_callback_t)(const pci_device_t* dev);

uint32_t pci_config_read32(uint8_t bus, uint8_t device, uint8_t function,
                            uint8_t offset);
uint16_t pci_config_read16(uint8_t bus, uint8_t device, uint8_t function,
                            uint8_t offset);
uint8_t pci_config_read8(uint8_t bus, uint8_t device, uint8_t function,
                          uint8_t offset);

/* Scans every bus/device/function (256 x 32 x 8, though almost all of
 * that space is empty on any real or virtual machine - see pci.c for
 * how a miss is recognized and skipped cheaply) and calls `callback`
 * once for each function that responds with a real vendor ID.
 * Multi-function devices (header type bit 7) are only checked for
 * functions 1-7 if function 0 reports itself as multi-function - the
 * standard PCI enumeration shortcut, not an approximation. */
void pci_enumerate(pci_enum_callback_t callback);

/* Human-readable name for a handful of common (class, subclass) pairs
 * - "Unknown" for anything not in the small table. Cosmetic only. */
const char* pci_class_name(uint8_t class_code, uint8_t subclass);

#endif
