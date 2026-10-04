/*
 * virtio_pci_modern.c - see virtio_pci_modern.h's own top comment for
 * the full design and scope.
 */
#include "virtio_pci_modern.h"
#include "../pci/pci.h"
#include "../../arch/x86/mm/paging.h"
#include "../../include/kernel.h"

/* PCI_CAP_ID_VNDR - the generic PCI "this is a vendor-specific
 * capability" identifier (PCI spec, not virtio-specific) every virtio
 * PCI capability uses as its own cap_vndr byte. */
#define PCI_CAP_ID_VNDR 0x09

#define VIRTIO_PCI_CAP_COMMON_CFG 1
#define VIRTIO_PCI_CAP_NOTIFY_CFG 2

/* Byte offsets into struct virtio_pci_cap (16 bytes) - verified
 * directly against the Linux kernel's own uapi header, see virtio_
 * pci_modern.h's own top comment. */
#define CAP_OFF_CFG_TYPE 3
#define CAP_OFF_BAR      4
#define CAP_OFF_OFFSET   8
#define CAP_OFF_LENGTH   12
#define CAP_OFF_NOTIFY_MULT 16 /* virtio_pci_notify_cap's own extra field */

/* Byte offsets into struct virtio_pci_common_cfg - the kernel's own
 * VIRTIO_PCI_COMMON_* macros, used directly rather than hand-computed
 * via offsetof, to avoid a transcription step that could get one
 * wrong. */
#define COMMON_DFSELECT 0
#define COMMON_DF       4
#define COMMON_GFSELECT 8
#define COMMON_GF       12
#define COMMON_STATUS   20
#define COMMON_Q_SELECT 22
#define COMMON_Q_SIZE   24
#define COMMON_Q_ENABLE 28
#define COMMON_Q_NOFF   30
#define COMMON_Q_DESCLO 32
#define COMMON_Q_DESCHI 36
#define COMMON_Q_AVAILLO 40
#define COMMON_Q_AVAILHI 44
#define COMMON_Q_USEDLO  48
#define COMMON_Q_USEDHI  52

static void write_le32(volatile uint8_t* base, uint32_t offset, uint32_t v) {
    *(volatile uint32_t*)(base + offset) = v;
}
static uint32_t read_le32(volatile uint8_t* base, uint32_t offset) {
    return *(volatile uint32_t*)(base + offset);
}
static void write_le16(volatile uint8_t* base, uint32_t offset, uint16_t v) {
    *(volatile uint16_t*)(base + offset) = v;
}
static uint16_t read_le16(volatile uint8_t* base, uint32_t offset) {
    return *(volatile uint16_t*)(base + offset);
}
static void write_u8(volatile uint8_t* base, uint32_t offset, uint8_t v) {
    *(base + offset) = v;
}
static uint8_t read_u8(volatile uint8_t* base, uint32_t offset) {
    return *(base + offset);
}

typedef struct {
    bool found;
    uint8_t bus, device, function;
} location_t;

static location_t g_location;
static uint16_t g_want_vendor, g_want_device;

/* kernel/rust/virtio_blk.rs's own exports, reused here - see that
 * file's own Phase 80 comment and virtio_pci_modern.h's own top
 * comment for why this reuse is valid. */
extern uint32_t rust_virtqueue_avail_offset(uint16_t queue_size);
extern uint32_t rust_virtqueue_used_offset(uint16_t queue_size);

static void find_device(const pci_device_t* dev) {
    if (g_location.found) {
        return;
    }
    if (dev->vendor_id == g_want_vendor && dev->device_id == g_want_device) {
        g_location.found = true;
        g_location.bus = dev->bus;
        g_location.device = dev->device;
        g_location.function = dev->function;
    }
}

/* Maps `length` bytes starting at `phys_addr` into the kernel's own
 * page directory, identity-mapped - the same reasoning kernel/
 * drivers/video/vbe.c's own map_framebuffer_pages() already documents
 * (not shared code with it, since that function is file-static there
 * and this is a small enough loop that duplicating it here is simpler
 * than exposing a new cross-file dependency for a few lines). */
static volatile uint8_t* map_mmio(uint32_t phys_addr, uint32_t length) {
    uint32_t page_base = phys_addr & ~0xFFFu;
    uint32_t end = phys_addr + length;
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();
    for (uint32_t addr = page_base; addr < end; addr += 4096) {
        if (!paging_map_page(kernel_pd, addr, addr, PAGE_PRESENT | PAGE_WRITE)) {
            kernel_log("[FAULT] virtio-pci-modern: failed to map MMIO "
                       "page at 0x%x\n", (int)addr);
            return 0;
        }
    }
    return (volatile uint8_t*)phys_addr;
}

/* Reads BAR `bar_index`'s own raw physical base address from PCI
 * config space (offset 0x10 + 4*bar_index, the standard PCI Type 0
 * header layout). Real, checked finding from this phase's own actual
 * boot testing against QEMU's real virtio-gpu-pci device (bar_type
 * logged as 2, not the 0 this file's own header originally,
 * incorrectly assumed before ever testing against real hardware):
 * this device's own common-cfg/notify-cfg BARs are genuinely 64-bit
 * memory BARs, not 32-bit ones - a real correction, not a
 * hypothetical one. A 64-bit memory BAR occupies two consecutive
 * config-space dwords: this one holds the low 32 address bits (plus
 * the type/flags bits in ITS OWN low 4 bits, exactly like a 32-bit
 * BAR), and bar_index+1's own dword holds the high 32 bits - standard
 * PCI, not virtio-specific. This kernel has no >4GB physical
 * addressing anywhere (no PAE, confirmed throughout this whole
 * project's own paging code), so the honest, correct thing to support
 * is exactly the real case this device needs: a 64-bit-*typed* BAR
 * whose actual address happens to fit in 32 bits (upper dword genuinely
 * zero, checked here, not assumed) - not full >4GB addressing, which
 * nothing in this kernel could use anyway. A real upper dword that
 * ISN'T zero is a real, honest failure this function reports rather
 * than silently truncates. */
static uint32_t read_bar_base(uint8_t bus, uint8_t device, uint8_t function,
                               uint8_t bar_index) {
    uint32_t raw = pci_config_read32(bus, device, function,
                                      (uint8_t)(0x10 + bar_index * 4));
    if (raw & 0x1) {
        kernel_log("[FAULT] virtio-pci-modern: BAR%d is I/O space, not "
                   "memory - this driver only supports MMIO BARs\n",
                   (int)bar_index);
        return 0;
    }
    uint8_t bar_type = (uint8_t)((raw >> 1) & 0x3);
    uint32_t base = raw & 0xFFFFFFF0u;
    if (bar_type == 2) {
        /* 64-bit memory BAR - the upper 32 bits live in the next dword. */
        uint32_t high = pci_config_read32(
            bus, device, function, (uint8_t)(0x10 + (bar_index + 1) * 4));
        if (high != 0) {
            kernel_log("[FAULT] virtio-pci-modern: BAR%d is a 64-bit "
                       "memory BAR whose real physical address (high "
                       "dword 0x%x) doesn't fit in this kernel's own "
                       "32-bit addressing\n", (int)bar_index, (int)high);
            return 0;
        }
        return base;
    }
    if (bar_type != 0) {
        kernel_log("[FAULT] virtio-pci-modern: BAR%d has an unexpected "
                   "memory BAR type %d (expected 0=32-bit or 2=64-bit)\n",
                   (int)bar_index, (int)bar_type);
        return 0;
    }
    return base;
}

bool virtio_pci_modern_init(uint16_t vendor_id, uint16_t device_id,
                             virtio_pci_modern_dev_t* out) {
    g_location.found = false;
    g_want_vendor = vendor_id;
    g_want_device = device_id;
    pci_enumerate(find_device);
    if (!g_location.found) {
        return false;
    }

    out->bus = g_location.bus;
    out->device = g_location.device;
    out->function = g_location.function;
    out->common_cfg = 0;
    out->notify_base = 0;
    out->notify_off_multiplier = 0;

    /* PCI status register, bit 4 (Capabilities List) - confirms this
     * device genuinely has a capability list before this driver
     * bothers walking one, rather than assuming every PCI device
     * does (most don't). */
    uint16_t pci_status = pci_config_read16(out->bus, out->device,
                                             out->function, 0x06);
    if (!(pci_status & 0x10)) {
        kernel_log("[FAULT] virtio-pci-modern: device has no PCI "
                   "capabilities list\n");
        return false;
    }

    uint32_t common_bar_base = 0, common_offset = 0;
    uint32_t notify_bar_base = 0, notify_offset = 0, notify_length = 0;
    bool have_common = false, have_notify = false;

    /* Capabilities Pointer, PCI config-space offset 0x34 - the start
     * of the linked list; each capability's own cap_next byte (offset
     * +1 within it) continues the chain, 0 ending it. A bounded loop
     * (64 iterations - far more than any real device's capability
     * list could genuinely contain) rather than trusting cap_next to
     * eventually reach 0, matching this kernel's own established non-
     * trusting-external-input discipline (kernel/drivers/virtio/
     * virtio_blk.c's own poll_for_completion() is the same idea
     * applied to a different kind of "don't hang forever on
     * malformed/unexpected device behavior"). */
    uint8_t cap_offset = pci_config_read8(out->bus, out->device,
                                           out->function, 0x34);
    for (int i = 0; i < 64 && cap_offset != 0; i++) {
        uint8_t cap_vndr = pci_config_read8(out->bus, out->device,
                                             out->function, cap_offset);
        uint8_t cap_next = pci_config_read8(out->bus, out->device,
                                             out->function,
                                             (uint8_t)(cap_offset + 1));
        if (cap_vndr == PCI_CAP_ID_VNDR) {
            uint8_t cfg_type = pci_config_read8(
                out->bus, out->device, out->function,
                (uint8_t)(cap_offset + CAP_OFF_CFG_TYPE));
            uint8_t bar = pci_config_read8(
                out->bus, out->device, out->function,
                (uint8_t)(cap_offset + CAP_OFF_BAR));
            uint32_t off = pci_config_read32(
                out->bus, out->device, out->function,
                (uint8_t)(cap_offset + CAP_OFF_OFFSET));
            uint32_t len = pci_config_read32(
                out->bus, out->device, out->function,
                (uint8_t)(cap_offset + CAP_OFF_LENGTH));

            if (cfg_type == VIRTIO_PCI_CAP_COMMON_CFG && !have_common) {
                uint32_t base = read_bar_base(out->bus, out->device,
                                               out->function, bar);
                if (base != 0) {
                    common_bar_base = base;
                    common_offset = off;
                    have_common = true;
                }
            } else if (cfg_type == VIRTIO_PCI_CAP_NOTIFY_CFG && !have_notify) {
                uint32_t base = read_bar_base(out->bus, out->device,
                                               out->function, bar);
                if (base != 0) {
                    notify_bar_base = base;
                    notify_offset = off;
                    notify_length = len;
                    out->notify_off_multiplier = pci_config_read32(
                        out->bus, out->device, out->function,
                        (uint8_t)(cap_offset + CAP_OFF_NOTIFY_MULT));
                    have_notify = true;
                }
            }
        }
        cap_offset = cap_next;
    }

    if (!have_common || !have_notify) {
        kernel_log("[FAULT] virtio-pci-modern: missing required "
                   "capability (common_cfg=%d notify_cfg=%d)\n",
                   (int)have_common, (int)have_notify);
        return false;
    }

    /* Common config structure is a fixed 56 bytes (through
     * queue_used_hi, the last field this driver or any virtio 1.0
     * device actually needs) - mapping exactly that, not the
     * capability's own, possibly larger, advertised length (the spec
     * explicitly allows cap.length to include padding/fields unused
     * by the driver - kernel/drivers/video/vbe.c's own similar
     * "don't trust a device-reported length to be exactly what's
     * needed, map or use only what's actually read" caution applies
     * equally here). */
    out->common_cfg = map_mmio(common_bar_base + common_offset, 56);
    out->notify_base = map_mmio(notify_bar_base + notify_offset,
                                 notify_length);
    if (!out->common_cfg || !out->notify_base) {
        return false;
    }

    return true;
}

uint8_t virtio_pci_modern_read_status(const virtio_pci_modern_dev_t* dev) {
    return read_u8(dev->common_cfg, COMMON_STATUS);
}

void virtio_pci_modern_write_status(const virtio_pci_modern_dev_t* dev,
                                     uint8_t status) {
    write_u8(dev->common_cfg, COMMON_STATUS, status);
}

uint32_t virtio_pci_modern_read_device_features(const virtio_pci_modern_dev_t* dev) {
    write_le32((volatile uint8_t*)dev->common_cfg, COMMON_DFSELECT, 0);
    return read_le32(dev->common_cfg, COMMON_DF);
}

void virtio_pci_modern_write_guest_features(const virtio_pci_modern_dev_t* dev,
                                             uint32_t features) {
    write_le32((volatile uint8_t*)dev->common_cfg, COMMON_GFSELECT, 0);
    write_le32((volatile uint8_t*)dev->common_cfg, COMMON_GF, features);
}

bool virtio_pci_modern_setup_queue(const virtio_pci_modern_dev_t* dev,
                                    uint16_t queue_idx,
                                    uint32_t queue_mem_phys,
                                    uint16_t* out_queue_size,
                                    volatile uint16_t** out_notify_addr) {
    volatile uint8_t* cfg = dev->common_cfg;
    write_le16(cfg, COMMON_Q_SELECT, queue_idx);
    uint16_t queue_size = read_le16(cfg, COMMON_Q_SIZE);
    if (queue_size == 0) {
        return false;
    }

    /* The three region addresses are independent 64-bit fields, but
     * this driver only ever runs below 4GB of physical memory - the
     * high halves are always 0, not merely often 0. desc is the base
     * of the contiguous allocation; avail/used sit at the offsets
     * kernel/rust/virtio_blk.rs's own virtqueue_layout() already
     * computes for this exact queue_size (see virtio_pci_modern.h's
     * own top comment on why reusing that function here is valid). */
    uint32_t avail_offset = rust_virtqueue_avail_offset(queue_size);
    uint32_t used_offset = rust_virtqueue_used_offset(queue_size);

    write_le32(cfg, COMMON_Q_DESCLO, queue_mem_phys);
    write_le32(cfg, COMMON_Q_DESCHI, 0);
    write_le32(cfg, COMMON_Q_AVAILLO, queue_mem_phys + avail_offset);
    write_le32(cfg, COMMON_Q_AVAILHI, 0);
    write_le32(cfg, COMMON_Q_USEDLO, queue_mem_phys + used_offset);
    write_le32(cfg, COMMON_Q_USEDHI, 0);

    uint16_t notify_off = read_le16(cfg, COMMON_Q_NOFF);
    write_le16(cfg, COMMON_Q_ENABLE, 1);

    *out_queue_size = queue_size;
    *out_notify_addr = (volatile uint16_t*)
        (dev->notify_base + (uint32_t)notify_off * dev->notify_off_multiplier);
    return true;
}
