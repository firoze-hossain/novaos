/*
 * virtio_blk.c - virtio-blk driver, legacy/transitional PCI transport
 * only - see virtio_blk.h for the full scope note.
 */
#include "virtio_blk.h"
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../arch/x86/mm/pmm.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"
#include "../driver.h"

#define VIRTIO_VENDOR_ID 0x1AF4
/* Legacy/transitional virtio-blk device ID. Modern-only virtio-blk
 * (no legacy support at all) uses 0x1042 instead - out of scope here,
 * since this driver only implements the legacy I/O-port transport
 * (matching this kernel's other PCI drivers, ac97.c/uhci.c, neither
 * of which implements MMIO-capability-based config either). */
#define VIRTIO_BLK_DEVICE_ID 0x1001

/* Legacy virtio PCI I/O-port register offsets, relative to BAR0 - see
 * the virtio 1.0 spec, section 4.1.4.8 ("Legacy Interfaces: A Note on
 * PCI Device Layout"). */
#define VIRTIO_REG_DEVICE_FEATURES 0x00 /* 32-bit RO */
#define VIRTIO_REG_GUEST_FEATURES  0x04 /* 32-bit RW */
#define VIRTIO_REG_QUEUE_ADDRESS   0x08 /* 32-bit RW - PFN, not a byte address */
#define VIRTIO_REG_QUEUE_SIZE      0x0C /* 16-bit RO */
#define VIRTIO_REG_QUEUE_SELECT    0x0E /* 16-bit RW */
#define VIRTIO_REG_QUEUE_NOTIFY    0x10 /* 16-bit WO */
#define VIRTIO_REG_DEVICE_STATUS   0x12 /* 8-bit RW */
#define VIRTIO_REG_ISR_STATUS      0x13 /* 8-bit RO */

#define VIRTIO_STATUS_ACKNOWLEDGE 0x01
#define VIRTIO_STATUS_DRIVER      0x02
#define VIRTIO_STATUS_DRIVER_OK   0x04
#define VIRTIO_STATUS_FAILED      0x80

#define VIRTIO_BLK_T_IN  0 /* read */
#define VIRTIO_BLK_T_OUT 1 /* write */

/* virtio_blk_req's on-the-wire header, sent as descriptor 0 of each
 * request - type + reserved (historically "ioprio", unused since) +
 * sector, exactly 16 bytes, matching virtio spec section 5.2.6. Static
 * storage, not stack-allocated: this address is handed to the device
 * as a DMA target, which must stay valid until the request completes
 * - a stack buffer would be technically fine given this driver blocks
 * until completion before returning, but static, fixed, page-aligned
 * buffers make "is this address still valid when the device reads it"
 * a non-question rather than something relying on stack-frame
 * lifetime reasoning to get right. */
typedef struct __attribute__((packed)) {
    uint32_t type;
    uint32_t reserved;
    uint64_t sector;
} virtio_blk_req_header_t;

static __attribute__((aligned(4096))) virtio_blk_req_header_t g_header;
static __attribute__((aligned(4096))) uint8_t g_data[512];
static __attribute__((aligned(4096))) uint8_t g_status;

static bool present = false;
static uint16_t io_base = 0;
static uint16_t queue_size = 0;
static uint32_t queue_mem_phys = 0;
static uint16_t last_used_idx = 0;

/* extern declarations for kernel/rust/virtio_blk.rs's exported
 * functions - see that file's own doc comments for the full contract
 * of each. */
extern uint32_t rust_virtqueue_total_bytes(uint16_t queue_size);
extern uint32_t rust_virtqueue_pages_needed(uint16_t queue_size);
extern void rust_virtqueue_init(uint8_t* mem, uint16_t queue_size);
extern uint16_t rust_virtqueue_submit_request(
    uint8_t* mem, uint16_t queue_size, uint32_t header_phys,
    uint32_t header_len, uint32_t data_phys, uint32_t data_len,
    uint32_t status_phys, bool write);
extern int32_t rust_virtqueue_poll_used(uint8_t* mem, uint16_t queue_size,
                                         uint16_t* last_used_idx);
extern int rust_virtqueue_selftest(void);

typedef struct {
    bool found;
    uint8_t bus, device, function;
    uint32_t bar0;
} virtio_location_t;

static virtio_location_t g_location;

static void find_virtio_blk(const pci_device_t* dev) {
    if (g_location.found) {
        return;
    }
    if (dev->vendor_id == VIRTIO_VENDOR_ID &&
        dev->device_id == VIRTIO_BLK_DEVICE_ID) {
        g_location.found = true;
        g_location.bus = dev->bus;
        g_location.device = dev->device;
        g_location.function = dev->function;
        g_location.bar0 =
            pci_config_read32(dev->bus, dev->device, dev->function, 0x10);
    }
}

static void enable_bus_mastering(uint8_t bus, uint8_t device,
                                  uint8_t function) {
    uint16_t command = pci_config_read16(bus, device, function, 0x04);
    command |= 0x04;
    /* PCI config space writes go through the same 0xCF8/0xCFC pair
     * pci_config_read32() itself uses - no dedicated pci_config_write
     * helper exists yet in this codebase (ac97.c/uhci.c don't need
     * one either, for the same reason: bus mastering is the only
     * config-space *write* any current driver makes), so this
     * open-codes the same address-then-data sequence pci.c's own
     * reader uses, matching ac97.c's own precedent exactly rather
     * than inventing a new pattern for one call site. */
    uint32_t address = 0x80000000u | ((uint32_t)bus << 16) |
                        ((uint32_t)device << 11) |
                        ((uint32_t)function << 8) | (0x04 & 0xFC);
    outl(0xCF8, address);
    outl(0xCFC, command);
}

/* Bounded poll for a completion - not unbounded, matching this
 * kernel's established non-blocking-first convention even here: a
 * device that never completes a request (a real hardware/emulation
 * bug, or this driver itself being wrong) fails the request instead
 * of hanging the kernel forever. 10 million iterations is generous
 * (QEMU's own virtio-blk backend typically completes a single-sector
 * request in well under a millisecond) without being unbounded. */
static bool poll_for_completion(void) {
    for (uint32_t i = 0; i < 10000000u; i++) {
        int32_t id = rust_virtqueue_poll_used((uint8_t*)queue_mem_phys,
                                               queue_size, &last_used_idx);
        if (id >= 0) {
            return true;
        }
    }
    return false;
}

static bool do_request(uint64_t sector, void* data, bool write) {
    if (!present) {
        return false;
    }

    g_header.type = write ? VIRTIO_BLK_T_OUT : VIRTIO_BLK_T_IN;
    g_header.reserved = 0;
    g_header.sector = sector;
    if (write) {
        memcpy(g_data, data, 512);
    }
    g_status = 0xFF; /* a value no real status code uses, so a device
                         that never touches this byte at all is still
                         distinguishable from one that reported OK. */

    rust_virtqueue_submit_request(
        (uint8_t*)queue_mem_phys, queue_size, (uint32_t)&g_header,
        sizeof(g_header), (uint32_t)g_data, 512, (uint32_t)&g_status,
        write);

    outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_NOTIFY), 0);

    if (!poll_for_completion()) {
        kernel_log("[FAULT] virtio-blk: request timed out waiting for "
                   "completion\n");
        return false;
    }

    if (g_status != 0) {
        kernel_log("[FAULT] virtio-blk: device reported error status %d\n",
                   (int)g_status);
        return false;
    }

    if (!write) {
        memcpy(data, g_data, 512);
    }
    return true;
}

void virtio_blk_init(void) {
    present = false;
    g_location.found = false;

    pci_enumerate(find_virtio_blk);
    if (!g_location.found) {
        return;
    }

    if (!(g_location.bar0 & 0x1)) {
        kernel_log("[ .. ] virtio-blk: BAR0 is memory-mapped, not "
                   "I/O-mapped - this driver only supports the legacy "
                   "I/O-space path\n");
        return;
    }
    io_base = (uint16_t)(g_location.bar0 & 0xFFFC);

    enable_bus_mastering(g_location.bus, g_location.device,
                         g_location.function);

    /* Legacy status handshake (virtio spec section 3.1.1, legacy
     * variant - no FEATURES_OK step, that's a modern-interface-only
     * concept): reset, ACKNOWLEDGE, DRIVER, negotiate features (this
     * driver accepts none - see kernel/rust/virtio_blk.rs's own note
     * on not negotiating VIRTIO_F_EVENT_IDX), set up the queue, then
     * DRIVER_OK. */
    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS), 0);
    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
         VIRTIO_STATUS_ACKNOWLEDGE);
    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
         VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);

    outl((uint16_t)(io_base + VIRTIO_REG_GUEST_FEATURES), 0);

    outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_SELECT), 0);
    queue_size = inw((uint16_t)(io_base + VIRTIO_REG_QUEUE_SIZE));
    if (queue_size == 0) {
        kernel_log("[FAULT] virtio-blk: device reports queue 0 size 0\n");
        outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
             VIRTIO_STATUS_FAILED);
        return;
    }

    uint32_t pages = rust_virtqueue_pages_needed(queue_size);
    queue_mem_phys = pmm_alloc_contiguous(pages);
    if (queue_mem_phys == 0) {
        kernel_log("[FAULT] virtio-blk: failed to allocate %d contiguous "
                   "pages for the virtqueue\n", (int)pages);
        outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
             VIRTIO_STATUS_FAILED);
        return;
    }
    rust_virtqueue_init((uint8_t*)queue_mem_phys, queue_size);
    last_used_idx = 0;

    /* Queue Address is a PFN (page frame number = phys_addr / 4096),
     * not a byte address - the legacy interface's own convention,
     * different from every other physical-address field this driver
     * (or kernel/rust/virtio_blk.rs) passes around as a plain byte
     * address elsewhere. */
    outl((uint16_t)(io_base + VIRTIO_REG_QUEUE_ADDRESS),
         queue_mem_phys / 4096);

    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
         VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER |
             VIRTIO_STATUS_DRIVER_OK);

    present = true;
    kernel_log("[ OK ] virtio-blk at PCI %d:%d.%d, I/O base 0x%x, queue "
               "size %d (%d contiguous pages at phys 0x%x)\n",
               (int)g_location.bus, (int)g_location.device,
               (int)g_location.function, (int)io_base, (int)queue_size,
               (int)pages, (unsigned int)queue_mem_phys);
}

bool virtio_blk_is_present(void) {
    return present;
}

bool virtio_blk_read_sector(uint64_t sector, void* buf) {
    return do_request(sector, buf, false);
}

bool virtio_blk_write_sector(uint64_t sector, const void* buf) {
    return do_request(sector, (void*)buf, true);
}

DRIVER_REGISTER("virtio-blk", virtio_blk_init, DRIVER_PHASE_AFTER_PCI);
