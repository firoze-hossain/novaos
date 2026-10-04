#ifndef DRIVERS_VIRTIO_PCI_MODERN_H
#define DRIVERS_VIRTIO_PCI_MODERN_H

#include "../../include/types.h"

/*
 * virtio_pci_modern.h - the *modern* (Virtio 1.0+) virtio-over-PCI
 * transport, as opposed to kernel/drivers/virtio/virtio_blk.c's and
 * virtio_net.c's own legacy/transitional one.
 *
 * Why this exists as separate, new, generic infrastructure rather
 * than extending the existing legacy transport: virtio-gpu has no
 * legacy PCI interface at all. Confirmed against multiple independent
 * sources while building kernel/drivers/virtiogpu/virtiogpu.c (not
 * assumed from the legacy-blk/net precedent already in this
 * codebase): "virtio-vsock, virtio-gpu and virtio-fs postdate the
 * legacy transport, so each has only a modern device ID" - QEMU's own
 * test suite is equally explicit, calling virtio-gpu-pci out as
 * `check_modern_only`. So unlike virtio_blk.c's own BAR0-as-a-flat-
 * I/O-port-register-block model, there is no legacy fallback to reach
 * for here - this transport is the only way to talk to virtio-gpu at
 * all, and is built generically (not virtio-gpu-specific) because
 * nothing about it actually is: any future modern-only virtio device
 * this kernel adds can reuse it unchanged.
 *
 * The modern transport locates its configuration structures (common
 * config, per-queue notification, device-specific config) via a
 * linked list of PCI capabilities (a standard PCI mechanism, walked
 * via the Capabilities Pointer at config-space offset 0x34 and each
 * capability's own next-pointer byte) rather than a single, fixed,
 * BAR0-is-everything layout - each capability names which BAR its
 * structure lives in and at what offset, since unlike the legacy
 * transport's simple I/O-port model, these structures are genuinely
 * memory-mapped (MMIO) and may be split across more than one BAR.
 * Every offset and field layout below is verified against the Linux
 * kernel's own authoritative uapi header (include/uapi/linux/
 * virtio_pci.h) fetched directly while writing this file, not
 * reconstructed from a secondary description or trusted from memory -
 * the same discipline this project already applied to the Multiboot
 * and Bochs DISPI structures elsewhere (kernel/arch/x86/boot/
 * multiboot.h, kernel/drivers/video/vbe.c).
 *
 * Scope, matching this project's own established, honest-about-limits
 * convention: no MSI-X (this kernel polls for completions everywhere
 * already - kernel/drivers/virtio/virtio_blk.c's own poll_for_
 * completion() is the precedent, not an exception); both 32-bit and
 * 64-bit-*typed* memory BARs are supported (QEMU's own virtio-gpu-pci
 * device, confirmed by this phase's own real boot testing - not
 * assumed before testing, an earlier draft of this comment wrongly
 * claimed 32-bit-only before ever actually testing against it -
 * genuinely uses a 64-bit BAR), but only when the real physical
 * address fits in 32 bits (upper dword checked as zero, not assumed -
 * see virtio_pci_modern.c's own read_bar_base()), since this kernel
 * has no >4GB physical addressing anywhere to make full 64-bit BAR
 * support meaningful; feature negotiation of exactly zero optional
 * features (this driver accepts none - the same "negotiating fewer
 * features is strictly simpler and still spec-compliant" reasoning
 * kernel/rust/virtio_blk.rs's own header comment already gives for
 * the legacy transport, equally true here).
 */

/* A virtqueue's 3-region layout (descriptor table, avail ring, used
 * ring) is byte-identical between the legacy and modern transports -
 * only how the device is TOLD where they are differs (one PFN for a
 * single contiguous region, legacy; three independent 64-bit
 * addresses, modern). This driver deliberately still allocates all
 * three as one contiguous region (nothing in the modern transport
 * forbids that - the three addresses are independent, not required to
 * be different) specifically so kernel/rust/virtio_blk.rs's own
 * already-written, already-tested virtqueue_layout()/rust_virtqueue_
 * init()/rust_virtqueue_poll_used() keep working completely unchanged
 * here too - only the "tell the device where this is" step differs,
 * encapsulated in virtio_pci_modern_setup_queue() below. */
typedef struct {
    uint8_t bus, device, function;

    /* Mapped (identity-mapped, matching this kernel's own established
     * physical-addressing convention - see kernel/drivers/video/
     * vbe.c's own comment on why) MMIO pointers to each capability
     * structure this driver actually uses. NULL if that capability
     * wasn't found - virtio_pci_modern_init() itself already refuses
     * to succeed without COMMON_CFG and NOTIFY_CFG, so any real caller
     * only ever sees non-NULL here, but the field stays honestly
     * nullable rather than asserting it can never be inspected. */
    volatile uint8_t* common_cfg;
    volatile uint8_t* notify_base;
    uint32_t notify_off_multiplier;
} virtio_pci_modern_dev_t;

/* Finds a PCI device matching `vendor_id`/`device_id`, walks its
 * capability list for VIRTIO_PCI_CAP_COMMON_CFG and VIRTIO_PCI_CAP_
 * NOTIFY_CFG, maps the BAR(s) they live in, and fills `out`. Does NOT
 * perform the virtio device-status handshake itself (reset/
 * ACKNOWLEDGE/DRIVER/...) - that's the caller's own responsibility
 * (virtio_pci_modern_write_status() below), the same division of
 * labor virtio_blk_init() already has between PCI discovery and the
 * handshake sequence. Returns false, gracefully, if the device isn't
 * present at all, or is present but missing a capability this driver
 * requires - a real, honest possibility (an ISA-only build of a
 * device, say), not something to assume away. */
bool virtio_pci_modern_init(uint16_t vendor_id, uint16_t device_id,
                             virtio_pci_modern_dev_t* out);

uint8_t virtio_pci_modern_read_status(const virtio_pci_modern_dev_t* dev);
void virtio_pci_modern_write_status(const virtio_pci_modern_dev_t* dev,
                                     uint8_t status);

/* Device-offered feature bits 0-31 only (feature word 0 - this driver
 * never selects a different device_feature_select, since it accepts
 * no optional features at all and so has no reason to inspect bits
 * 32-63). */
uint32_t virtio_pci_modern_read_device_features(const virtio_pci_modern_dev_t* dev);
/* Writes guest_feature_select=0 then guest_feature=`features` - the
 * guest's own accepted-feature-bits-0-31 response. This driver always
 * passes 0 here (see this file's own top comment on why). */
void virtio_pci_modern_write_guest_features(const virtio_pci_modern_dev_t* dev,
                                             uint32_t features);

/* Selects queue `queue_idx`, reads its device-reported size (0 means
 * "this queue doesn't exist" - a real, checked possibility, not
 * assumed nonzero), writes the three 64-bit region addresses (desc/
 * avail/used - all three pointing within the SAME contiguous
 * allocation the caller made, at the offsets kernel/rust/
 * virtio_blk.rs's own virtqueue_layout() already computes for that
 * queue_size - see this file's own top comment on why that reuse is
 * valid here), and enables the queue. `queue_mem_phys` must already be
 * rust_virtqueue_init()-initialized by the caller, exactly as virtio_
 * blk_init() already does for the legacy transport. Returns the real,
 * device-reported queue size via `out_queue_size`, and the absolute
 * MMIO address this queue's own notifications must be written to
 * (queue_notify_off * notify_off_multiplier + the notify capability's
 * own base - computed here once, not recomputed by every caller) via
 * `out_notify_addr`. Returns false if the queue doesn't exist (size
 * 0) - the caller must not proceed to use a queue this returned false
 * for. */
bool virtio_pci_modern_setup_queue(const virtio_pci_modern_dev_t* dev,
                                    uint16_t queue_idx,
                                    uint32_t queue_mem_phys,
                                    uint16_t* out_queue_size,
                                    volatile uint16_t** out_notify_addr);

#endif
