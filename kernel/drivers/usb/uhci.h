#ifndef DRIVERS_USB_UHCI_H
#define DRIVERS_USB_UHCI_H

#include "../../include/types.h"

/* UHCI (Universal Host Controller Interface) USB 1.1 driver -
 * deliberately the simplest of the four USB host controller
 * interfaces (UHCI/OHCI/EHCI/xHCI) to target: an I/O-port register
 * interface (consistent with every other driver in this tree, unlike
 * EHCI/xHCI's memory-mapped registers), found via Phase 13's PCI
 * enumeration by class code (0x0C/0x03) plus prog_if 0x00 (the byte
 * that actually distinguishes UHCI from OHCI/0x10, EHCI/0x20, and
 * xHCI/0x30, all of which otherwise share the same class/subclass).
 *
 * Scope: enumerates up to 2 root hub ports, resets whichever has a
 * device attached, performs the standard control-transfer-based
 * enumeration sequence (GET_DESCRIPTOR partial, SET_ADDRESS,
 * GET_DESCRIPTOR full) to identify the device, and - if it looks like
 * a HID keyboard - reads its boot-protocol interrupt reports (the
 * simplest HID report format, specifically designed for BIOS/
 * bootloader use, needing no report-descriptor parsing). One
 * transfer in flight at a time - the same single-outstanding-
 * operation pattern this project's other drivers use, not a real
 * multi-device/multi-endpoint scheduler. See PROGRESS.md for the
 * full scope note and what a real USB stack would additionally need
 * (bulk/isochronous transfers, hubs beyond the root hub, mass storage
 * or other device classes, more than one device at a time). */

void usb_uhci_init(void);
bool usb_uhci_is_present(void);

/* Non-blocking: returns true and fills `out_report` (8 bytes, the
 * standard USB HID boot keyboard report layout: modifier byte,
 * reserved byte, then up to 6 simultaneous keycodes) if a keyboard
 * was found during enumeration and has a new report ready since the
 * last call. False otherwise (no keyboard found, or no new report
 * yet) - the caller is expected to poll this the same way
 * kernel/drivers/mouse/ps2mouse.h's driver is polled. */
bool usb_uhci_keyboard_poll(uint8_t out_report[8]);

#endif
