/*
 * rtl8139.c - Realtek RTL8139 PCI NIC driver (polling PIO/DMA, no IRQ)
 */
#include "rtl8139.h"
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"

#define RTL_VENDOR_ID 0x10EC
#define RTL_DEVICE_ID 0x8139

/* Register offsets from the I/O base (read out of the PCI BAR0
 * config register at runtime - see rtl8139_init() - not a fixed
 * address the way NE2000's ISA I/O base is). */
#define REG_MAC0        0x00 /* 6 bytes */
#define REG_TSD0        0x10 /* 4 registers, 4 bytes apart: TSD0-3 */
#define REG_TSAD0       0x20 /* 4 registers, 4 bytes apart: TSAD0-3 */
#define REG_RBSTART     0x30
#define REG_CMD         0x37
#define REG_CAPR        0x38
#define REG_IMR         0x3C
#define REG_ISR         0x3E
#define REG_TCR         0x40
#define REG_RCR         0x44
#define REG_CONFIG1     0x52

#define CMD_RESET  0x10
#define CMD_RX_ENABLE 0x08
#define CMD_TX_ENABLE 0x04
#define CMD_BUF_EMPTY 0x01

#define RCR_ACCEPT_ALL      0x0F /* AAP|APM|AM|AB - see rtl8139_init() */

#define RX_BUFFER_SIZE (8192 + 16 + 1500)
#define TX_BUFFER_SIZE 1600
#define NUM_TX_DESCRIPTORS 4

static bool present = false;
static uint16_t io_base = 0;
static uint8_t mac_address[6];

static uint32_t rx_read_offset = 0;
static int tx_next_descriptor = 0;

/* Physically contiguous buffers the card DMAs to/from directly - see
 * rtl8139.h's header comment on why static kernel-image buffers are
 * enough here (identity-mapped low memory, no separate physical
 * allocator needed for this). */
static uint8_t rx_buffer[RX_BUFFER_SIZE] __attribute__((aligned(4)));
static uint8_t tx_buffers[NUM_TX_DESCRIPTORS][TX_BUFFER_SIZE]
    __attribute__((aligned(4)));

static void enable_bus_mastering(uint8_t bus, uint8_t device,
                                  uint8_t function) {
    uint16_t command = pci_config_read16(bus, device, function, 0x04);
    command |= 0x04; /* Bus Master Enable - required for the card to
                         actually perform DMA to system memory at all */
    outl(0xCF8, 0x80000000u | ((uint32_t)bus << 16) |
                    ((uint32_t)device << 11) | ((uint32_t)function << 8) |
                    0x04);
    outl(0xCFC, command);
}

typedef struct {
    bool found;
    uint8_t bus, device, function;
    uint32_t bar0;
} rtl_location_t;

static rtl_location_t g_location;

static void find_rtl8139(const pci_device_t* dev) {
    if (g_location.found) {
        return; /* already found one - use the first */
    }
    if (dev->vendor_id == RTL_VENDOR_ID && dev->device_id == RTL_DEVICE_ID) {
        g_location.found = true;
        g_location.bus = dev->bus;
        g_location.device = dev->device;
        g_location.function = dev->function;
        g_location.bar0 =
            pci_config_read32(dev->bus, dev->device, dev->function, 0x10);
    }
}

void rtl8139_init(void) {
    present = false;
    g_location.found = false;

    pci_enumerate(find_rtl8139);
    if (!g_location.found) {
        return; /* no RTL8139 attached - not an error, see net.c's
                    NE2000 fallback */
    }

    /* BAR0's low bit distinguishes I/O-space (1) from memory-space (0)
     * BARs; bit 0 set means this is an I/O BAR, and the actual base
     * address is the rest of the register with the bottom 2 bits
     * (which encode this flag plus a reserved bit) masked off. */
    if (!(g_location.bar0 & 0x1)) {
        kernel_log("[ .. ] RTL8139: BAR0 is memory-mapped, not I/O-mapped - "
                   "this driver only supports the I/O-space path\n");
        return;
    }
    io_base = (uint16_t)(g_location.bar0 & 0xFFFC);

    enable_bus_mastering(g_location.bus, g_location.device,
                         g_location.function);

    outb((uint16_t)(io_base + REG_CONFIG1), 0x00); /* power on */

    outb((uint16_t)(io_base + REG_CMD), CMD_RESET);
    uint32_t spins = 0;
    while (inb((uint16_t)(io_base + REG_CMD)) & CMD_RESET) {
        if (++spins > 1000000u) {
            kernel_log("[ .. ] RTL8139: reset never completed\n");
            return;
        }
    }

    for (int i = 0; i < 6; i++) {
        mac_address[i] = inb((uint16_t)(io_base + REG_MAC0 + i));
    }

    /* The card DMAs directly to this physical address - valid only
     * because rx_buffer lives in NovaOS's identity-mapped low memory
     * (see rtl8139.h). */
    outl((uint16_t)(io_base + REG_RBSTART), (uint32_t)rx_buffer);

    outw((uint16_t)(io_base + REG_IMR), 0x0000); /* polled, not interrupt-driven */

    /* Accept everything (promiscuous-ish: AAP|APM|AM|AB) rather than
     * the tighter unicast+broadcast filtering NE2000 uses - simpler
     * to get working correctly first, and this is a demo/proof-of-
     * capability driver rather than a production one (see
     * PROGRESS.md). Buffer-length bits left at 0 (8K+16), matching
     * RX_BUFFER_SIZE's base allocation. */
    outl((uint16_t)(io_base + REG_RCR), RCR_ACCEPT_ALL);

    outb((uint16_t)(io_base + REG_CMD), CMD_RX_ENABLE | CMD_TX_ENABLE);

    rx_read_offset = 0;
    tx_next_descriptor = 0;

    present = true;
    kernel_log("[ OK ] RTL8139 NIC at PCI %d:%d.%d, I/O base 0x%x, "
               "MAC %x:%x:%x:%x:%x:%x\n",
               (int)g_location.bus, (int)g_location.device,
               (int)g_location.function, (int)io_base, mac_address[0],
               mac_address[1], mac_address[2], mac_address[3],
               mac_address[4], mac_address[5]);
}

bool rtl8139_is_present(void) {
    return present;
}

const uint8_t* rtl8139_mac_address(void) {
    return mac_address;
}

bool rtl8139_send(const void* frame, uint16_t length) {
    if (!present) {
        return false;
    }
    if (length > TX_BUFFER_SIZE) {
        return false;
    }

    int slot = tx_next_descriptor;
    tx_next_descriptor = (tx_next_descriptor + 1) % NUM_TX_DESCRIPTORS;

    memcpy(tx_buffers[slot], frame, length);
    /* Ethernet's own 60-byte minimum frame size - the card pads with
     * whatever garbage is left in the buffer otherwise, same
     * consideration NE2000's driver documents. */
    uint16_t tx_length = (length < 60) ? 60 : length;

    outl((uint16_t)(io_base + REG_TSAD0 + (uint32_t)slot * 4),
         (uint32_t)tx_buffers[slot]);
    /* Writing the length to TSD both starts transmission and clears
     * the OWN bit for the card to take over the descriptor. */
    outl((uint16_t)(io_base + REG_TSD0 + (uint32_t)slot * 4), tx_length);

    uint32_t spins = 0;
    while (!(inl((uint16_t)(io_base + REG_TSD0 + (uint32_t)slot * 4)) &
             0x8000)) { /* TOK - Transmit OK */
        if (++spins > 1000000u) {
            return false;
        }
    }
    return true;
}

uint16_t rtl8139_receive(void* buffer) {
    if (!present) {
        return 0;
    }
    if (inb((uint16_t)(io_base + REG_CMD)) & CMD_BUF_EMPTY) {
        return 0; /* nothing waiting */
    }

    /* The card's write pointer trails 16 bytes behind where the
     * driver's read pointer needs to look for the next packet header
     * - a fixed hardware offset every RTL8139 driver accounts for,
     * not something specific to this implementation. */
    uint8_t* packet_header = rx_buffer + rx_read_offset;
    uint16_t status = (uint16_t)(packet_header[0] | (packet_header[1] << 8));
    uint16_t total_length =
        (uint16_t)(packet_header[2] | (packet_header[3] << 8));

    if (!(status & 0x01) || total_length < 4 ||
        total_length > RTL8139_MAX_FRAME + 4) {
        /* ROK (Receive OK) not set, or a nonsensical length - the ring
         * is out of sync (matches the class of bug Phase 6's NE2000
         * driver hit; here it manifests as a bad header rather than
         * an all-zero one). Simplest and sufficient for now: log it
         * and stop trying this poll cycle, rather than guessing at a
         * resync. */
        kernel_log("[WARN] RTL8139: unexpected packet header (status=0x%x "
                   "length=%d) - dropping this poll cycle\n", (int)status,
                   (int)total_length);
        return 0;
    }

    uint16_t data_length = (uint16_t)(total_length - 4); /* drop trailing CRC */
    uint32_t data_offset = (rx_read_offset + 4) % RX_BUFFER_SIZE;

    uint8_t* out = (uint8_t*)buffer;
    for (uint16_t i = 0; i < data_length; i++) {
        out[i] = rx_buffer[(data_offset + i) % RX_BUFFER_SIZE];
    }

    /* Advance and 4-byte-align the read pointer past this packet's
     * header + data, then reproduce the "-16, wrapped" hardware quirk
     * CAPR expects. */
    uint32_t next_offset = (rx_read_offset + 4 + total_length + 3) & ~3u;
    next_offset %= RX_BUFFER_SIZE;
    rx_read_offset = next_offset;

    outw((uint16_t)(io_base + REG_CAPR),
         (uint16_t)((rx_read_offset - 16) & 0xFFFF));

    return data_length;
}
