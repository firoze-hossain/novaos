/*
 * virtio_net.c - virtio-net driver, legacy/transitional PCI transport
 * only - see virtio_net.h for the full scope note.
 */
#include "virtio_net.h"
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../arch/x86/mm/pmm.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"

#define VIRTIO_VENDOR_ID 0x1AF4
#define VIRTIO_NET_DEVICE_ID 0x1000 /* legacy/transitional virtio-net -
                                        modern-only uses 0x1041 instead,
                                        out of scope (see virtio_blk.c's
                                        own comment on the same choice
                                        for virtio-blk) */

#define VIRTIO_REG_DEVICE_FEATURES 0x00
#define VIRTIO_REG_GUEST_FEATURES  0x04
#define VIRTIO_REG_QUEUE_ADDRESS   0x08
#define VIRTIO_REG_QUEUE_SIZE      0x0C
#define VIRTIO_REG_QUEUE_SELECT    0x0E
#define VIRTIO_REG_QUEUE_NOTIFY    0x10
#define VIRTIO_REG_DEVICE_STATUS   0x12
#define VIRTIO_REG_ISR_STATUS      0x13
#define VIRTIO_REG_MAC_ADDRESS     0x14 /* device-specific config start -
                                            6 bytes, the MAC address,
                                            always present regardless of
                                            which features are
                                            negotiated */

#define VIRTIO_STATUS_ACKNOWLEDGE 0x01
#define VIRTIO_STATUS_DRIVER      0x02
#define VIRTIO_STATUS_DRIVER_OK   0x04
#define VIRTIO_STATUS_FAILED      0x80

#define QUEUE_RX 0
#define QUEUE_TX 1

/* extern declarations for kernel/rust/virtio_net.rs's exported
 * functions - see that file's own doc comments for the full contract
 * of each. */
extern uint32_t rust_net_virtqueue_total_bytes(uint16_t queue_size);
extern uint32_t rust_net_virtqueue_pages_needed(uint16_t queue_size);
extern void rust_net_virtqueue_init(uint8_t* mem, uint16_t queue_size);
extern void rust_net_rx_post_buffer(uint8_t* mem, uint16_t queue_size,
                                     uint16_t desc_index, uint32_t buf_phys);
extern int32_t rust_net_rx_poll_used(uint8_t* mem, uint16_t queue_size,
                                      uint16_t* last_used_idx,
                                      uint32_t* out_total_len);
extern void rust_net_tx_submit(uint8_t* mem, uint16_t queue_size,
                                uint32_t header_phys, uint32_t data_phys,
                                uint32_t data_len);
extern bool rust_net_tx_poll_used(uint8_t* mem, uint16_t queue_size,
                                   uint16_t* last_used_idx);
extern uint32_t rust_net_rx_buffer_count(void);
extern uint32_t rust_net_rx_buffer_size(void);

/* NET_HDR_LEN duplicated here as a #define matching
 * kernel/rust/virtio_net.rs's own NET_HDR_LEN constant - not read back
 * through an accessor the way RX_BUFFER_COUNT/SIZE are, since it's
 * only ever used for one small, fixed-size static buffer below
 * (g_tx_header) whose size must be a compile-time constant either way;
 * an accessor would only move the duplication from "one number" to
 * "one array-size expression," not remove it. */
#define NET_HDR_LEN 10
#define MAX_RX_BUFFERS 8 /* matches virtio_net.rs's own RX_BUFFER_COUNT -
                             see virtio_net_init()'s own runtime check
                             against rust_net_rx_buffer_count() */
#define RX_BUFFER_SIZE (NET_HDR_LEN + 1514)

static bool present = false;
static uint16_t io_base = 0;
static uint8_t mac_address[6];

static uint16_t rx_queue_size = 0;
static uint32_t rx_queue_mem_phys = 0;
static uint16_t rx_last_used_idx = 0;
static __attribute__((aligned(4096)))
    uint8_t rx_buffers[MAX_RX_BUFFERS][RX_BUFFER_SIZE];

static uint16_t tx_queue_size = 0;
static uint32_t tx_queue_mem_phys = 0;
static uint16_t tx_last_used_idx = 0;
static __attribute__((aligned(4096))) uint8_t tx_header[NET_HDR_LEN];
static __attribute__((aligned(4096))) uint8_t tx_data[1514];

typedef struct {
    bool found;
    uint8_t bus, device, function;
    uint32_t bar0;
} virtio_net_location_t;

static virtio_net_location_t g_location;

static void find_virtio_net(const pci_device_t* dev) {
    if (g_location.found) {
        return;
    }
    if (dev->vendor_id == VIRTIO_VENDOR_ID &&
        dev->device_id == VIRTIO_NET_DEVICE_ID) {
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
    uint32_t address = 0x80000000u | ((uint32_t)bus << 16) |
                        ((uint32_t)device << 11) |
                        ((uint32_t)function << 8) | (0x04 & 0xFC);
    outl(0xCF8, address);
    outl(0xCFC, command);
}

/* Selects `queue`, reads its reported size, allocates contiguous
 * physical memory for it (pmm_alloc_contiguous(), Phase 42's own
 * prerequisite, reused here rather than reinvented), and initializes
 * it to zero. Returns false (leaving *out_phys untouched) on any
 * failure - the caller is responsible for aborting init and marking
 * FAILED status, matching virtio_blk_init()'s own established
 * failure-handling shape. */
static bool setup_queue(uint16_t queue, uint16_t* out_size,
                         uint32_t* out_phys) {
    outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_SELECT), queue);
    uint16_t size = inw((uint16_t)(io_base + VIRTIO_REG_QUEUE_SIZE));
    if (size == 0) {
        kernel_log("[FAULT] virtio-net: device reports queue %d size 0\n",
                   (int)queue);
        return false;
    }

    uint32_t pages = rust_net_virtqueue_pages_needed(size);
    uint32_t phys = pmm_alloc_contiguous(pages);
    if (phys == 0) {
        kernel_log("[FAULT] virtio-net: failed to allocate %d contiguous "
                   "pages for queue %d\n", (int)pages, (int)queue);
        return false;
    }
    rust_net_virtqueue_init((uint8_t*)phys, size);
    outl((uint16_t)(io_base + VIRTIO_REG_QUEUE_ADDRESS), phys / 4096);

    *out_size = size;
    *out_phys = phys;
    return true;
}

void virtio_net_init(void) {
    present = false;
    g_location.found = false;

    pci_enumerate(find_virtio_net);
    if (!g_location.found) {
        return;
    }

    if (!(g_location.bar0 & 0x1)) {
        kernel_log("[ .. ] virtio-net: BAR0 is memory-mapped, not "
                   "I/O-mapped - this driver only supports the legacy "
                   "I/O-space path\n");
        return;
    }
    io_base = (uint16_t)(g_location.bar0 & 0xFFFC);

    enable_bus_mastering(g_location.bus, g_location.device,
                         g_location.function);

    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS), 0);
    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
         VIRTIO_STATUS_ACKNOWLEDGE);
    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
         VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER);
    outl((uint16_t)(io_base + VIRTIO_REG_GUEST_FEATURES), 0);

    /* MAX_RX_BUFFERS (this file) and RX_BUFFER_COUNT
     * (kernel/rust/virtio_net.rs) must agree - checked at runtime
     * here rather than trusted to stay in sync by hand, since a
     * mismatch would silently under- or over-post RX buffers. A
     * kernel_panic() here (matching this codebase's own established
     * "an internal invariant this driver depends on was violated"
     * response, e.g. paging_create_address_space()'s own
     * out-of-memory panic) is correct: this is a build-time
     * programming error, not a runtime hardware condition to recover
     * from gracefully. */
    if (rust_net_rx_buffer_count() != MAX_RX_BUFFERS ||
        rust_net_rx_buffer_size() != RX_BUFFER_SIZE) {
        kernel_panic("virtio_net_init: MAX_RX_BUFFERS/RX_BUFFER_SIZE "
                     "disagree with kernel/rust/virtio_net.rs");
    }

    if (!setup_queue(QUEUE_RX, &rx_queue_size, &rx_queue_mem_phys)) {
        outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
             VIRTIO_STATUS_FAILED);
        return;
    }
    if (!setup_queue(QUEUE_TX, &tx_queue_size, &tx_queue_mem_phys)) {
        outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
             VIRTIO_STATUS_FAILED);
        return;
    }
    rx_last_used_idx = 0;
    tx_last_used_idx = 0;

    /* Pre-post every RX buffer this driver keeps in flight - the
     * device can only ever deliver a packet into a buffer the driver
     * has already handed it; nothing arrives until at least one is
     * posted. Descriptor index N is used for rx_buffers[N] throughout
     * this driver's lifetime - a fixed, 1:1 mapping, not reassigned
     * later, which is what makes recycling in virtio_net_receive()
     * below just "re-post the same slot" rather than needing to track
     * which physical buffer corresponds to which descriptor. */
    for (uint32_t i = 0; i < MAX_RX_BUFFERS && i < rx_queue_size; i++) {
        rust_net_rx_post_buffer((uint8_t*)rx_queue_mem_phys, rx_queue_size,
                                 (uint16_t)i, (uint32_t)rx_buffers[i]);
    }
    outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_NOTIFY), QUEUE_RX);

    for (int i = 0; i < 6; i++) {
        mac_address[i] =
            inb((uint16_t)(io_base + VIRTIO_REG_MAC_ADDRESS + i));
    }

    outb((uint16_t)(io_base + VIRTIO_REG_DEVICE_STATUS),
         VIRTIO_STATUS_ACKNOWLEDGE | VIRTIO_STATUS_DRIVER |
             VIRTIO_STATUS_DRIVER_OK);

    present = true;
    kernel_log("[ OK ] virtio-net at PCI %d:%d.%d, I/O base 0x%x, MAC "
               "%x:%x:%x:%x:%x:%x, RX queue size %d, TX queue size %d\n",
               (int)g_location.bus, (int)g_location.device,
               (int)g_location.function, (int)io_base, mac_address[0],
               mac_address[1], mac_address[2], mac_address[3],
               mac_address[4], mac_address[5], (int)rx_queue_size,
               (int)tx_queue_size);
}

bool virtio_net_is_present(void) {
    return present;
}

const uint8_t* virtio_net_mac_address(void) {
    return mac_address;
}

bool virtio_net_send(const void* frame, uint16_t length) {
    if (!present) {
        return false;
    }
    if (length > sizeof(tx_data)) {
        return false;
    }

    memset(tx_header, 0, sizeof(tx_header)); /* no offload features
                                                 negotiated - every
                                                 field stays zero */
    memcpy(tx_data, frame, length);

    rust_net_tx_submit((uint8_t*)tx_queue_mem_phys, tx_queue_size,
                        (uint32_t)tx_header, (uint32_t)tx_data, length);
    outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_NOTIFY), QUEUE_TX);

    /* Bounded poll for completion - matches virtio_blk.c's own
     * established "generous but not unbounded" shape, not an
     * unbounded wait. */
    for (uint32_t i = 0; i < 10000000u; i++) {
        if (rust_net_tx_poll_used((uint8_t*)tx_queue_mem_phys, tx_queue_size,
                                   &tx_last_used_idx)) {
            return true;
        }
    }
    kernel_log("[FAULT] virtio-net: TX request timed out waiting for "
               "completion\n");
    return false;
}

uint16_t virtio_net_receive(void* buffer) {
    if (!present) {
        return 0;
    }

    uint32_t total_len = 0;
    int32_t desc_id = rust_net_rx_poll_used(
        (uint8_t*)rx_queue_mem_phys, rx_queue_size, &rx_last_used_idx,
        &total_len);
    if (desc_id < 0) {
        return 0; /* nothing waiting */
    }
    if (desc_id >= MAX_RX_BUFFERS) {
        /* Should be unreachable - only descriptor indices this driver
         * itself posted (0..MAX_RX_BUFFERS) can ever complete. Refuse
         * to trust an out-of-range value rather than index out of
         * bounds into rx_buffers[]. */
        kernel_log("[WARN] virtio-net: used ring reported an "
                   "out-of-range descriptor id %d - ignoring\n",
                   (int)desc_id);
        return 0;
    }
    if (total_len < NET_HDR_LEN) {
        kernel_log("[WARN] virtio-net: received length %d shorter than "
                   "the virtio_net_hdr itself - dropping\n",
                   (int)total_len);
        rust_net_rx_post_buffer((uint8_t*)rx_queue_mem_phys, rx_queue_size,
                                 (uint16_t)desc_id,
                                 (uint32_t)rx_buffers[desc_id]);
        outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_NOTIFY), QUEUE_RX);
        return 0;
    }

    uint16_t data_len = (uint16_t)(total_len - NET_HDR_LEN);
    memcpy(buffer, rx_buffers[desc_id] + NET_HDR_LEN, data_len);

    /* Recycle: this exact buffer/descriptor slot is immediately
     * reusable once its data has been copied out - re-post it so the
     * device can deliver the next packet into it, rather than only
     * ever using each buffer once and running out. */
    rust_net_rx_post_buffer((uint8_t*)rx_queue_mem_phys, rx_queue_size,
                             (uint16_t)desc_id,
                             (uint32_t)rx_buffers[desc_id]);
    outw((uint16_t)(io_base + VIRTIO_REG_QUEUE_NOTIFY), QUEUE_RX);

    return data_len;
}

/* Not DRIVER_REGISTER()'d (unlike virtio_blk.c) - deliberately: this
 * driver must be initialized *before* net_init() decides which NIC is
 * active, and net_init() itself runs earlier than
 * DRIVER_PHASE_AFTER_PCI (see kernel/net/net.c's own call to
 * virtio_net_init(), matching how rtl8139_init()/ne2000_init() are
 * already called directly rather than through driver_init_all()).
 * Safe to call this early even though PCI enumeration's own explicit
 * self-test hasn't run yet - PCI configuration space itself is just
 * I/O port reads (0xCF8/0xCFC), available from the moment the kernel
 * is running, not dependent on any particular boot-sequence position -
 * exactly the same reasoning rtl8139_init()'s own early call already
 * relies on. */
