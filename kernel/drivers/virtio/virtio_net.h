#ifndef DRIVERS_VIRTIO_NET_H
#define DRIVERS_VIRTIO_NET_H

#include "../../include/types.h"

/* virtio_net.c - Phase 45: this kernel's second virtio driver.
 *
 * Legacy/transitional PCI transport only, matching virtio_blk.c's own
 * choice and this kernel's other PCI drivers (ac97.c/uhci.c neither
 * implement MMIO-capability config either).
 *
 * Integration scope, deliberately: registered with kernel/net/net.c's
 * existing NIC-selection dispatch (the same `active_nic` mechanism
 * RTL8139/NE2000 already share) as a THIRD option, checked before
 * RTL8139/NE2000 - if present, it's used; if not (this project's own
 * default test config does not attach a virtio-net-pci device), the
 * existing RTL8139/NE2000 fallback behaves exactly as it did before
 * this phase, completely unaffected. This means every one of this
 * project's existing network-dependent self-tests (ping/DNS/TFTP/TCP)
 * keeps running against RTL8139 exactly as before unless a
 * virtio-net-pci device is explicitly attached - real, separate
 * verification for that case is documented in PROGRESS.md's Phase 45
 * entry, not folded into the shared, default test suite (the same
 * "verify manually before ever touching shared infrastructure"
 * discipline Phase 42's virtio-blk work already established). */

void virtio_net_init(void);
bool virtio_net_is_present(void);
bool virtio_net_send(const void* frame, uint16_t length);
uint16_t virtio_net_receive(void* buffer);
const uint8_t* virtio_net_mac_address(void);

#endif
