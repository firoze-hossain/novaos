/*
 * ne2000.c - NE2000 ISA NIC driver (polling PIO, no IRQ)
 *
 * Fixed at the QEMU default I/O base for `-device ne2k_isa` (0x300) -
 * no PCI enumeration or config-space probing, matching the ATA
 * driver's "one fixed device, keep it simple" precedent from Phase 3.
 *
 * The NE2000's onboard 8KB/16KB buffer memory is organized into 256-
 * byte "pages" and accessed only through a remote DMA mechanism (set
 * a start address + byte count in registers, then stream words through
 * a single data port) - there is no way to memory-map it directly.
 * Transmit uses a couple of fixed pages; receive uses the rest as a
 * ring buffer the NIC fills and the driver drains, tracked with the
 * BNRY (driver's read pointer) and CURR (NIC's write pointer)
 * registers - see ne2000_receive()'s comments for the ring mechanics,
 * which are the fiddliest part of this driver.
 */
#include "ne2000.h"
#include "../../arch/x86/io.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"

#define NE_IOBASE 0x300

/* Page 0 registers (CR.PS = 00) */
#define REG_CR      (NE_IOBASE + 0x00)
#define REG_PSTART  (NE_IOBASE + 0x01) /* write */
#define REG_PSTOP   (NE_IOBASE + 0x02) /* write */
#define REG_BNRY    (NE_IOBASE + 0x03)
#define REG_TPSR    (NE_IOBASE + 0x04) /* write */
#define REG_TBCR0   (NE_IOBASE + 0x05) /* write */
#define REG_TBCR1   (NE_IOBASE + 0x06) /* write */
#define REG_ISR     (NE_IOBASE + 0x07)
#define REG_RSAR0   (NE_IOBASE + 0x08) /* write */
#define REG_RSAR1   (NE_IOBASE + 0x09) /* write */
#define REG_RBCR0   (NE_IOBASE + 0x0A) /* write */
#define REG_RBCR1   (NE_IOBASE + 0x0B) /* write */
#define REG_RCR     (NE_IOBASE + 0x0C) /* write */
#define REG_TCR     (NE_IOBASE + 0x0D) /* write */
#define REG_DCR     (NE_IOBASE + 0x0E) /* write */
#define REG_IMR     (NE_IOBASE + 0x0F) /* write */
#define REG_DATA    (NE_IOBASE + 0x10)
#define REG_RESET   (NE_IOBASE + 0x1F)

/* Page 1 registers (CR.PS = 01) */
#define REG_PAR0    (NE_IOBASE + 0x01) /* our MAC, 6 bytes from here */
#define REG_CURR    (NE_IOBASE + 0x07)

#define CR_STP  0x01
#define CR_STA  0x02
#define CR_TXP  0x04
#define CR_RD2  0x20 /* abort/complete remote DMA */
#define CR_PS0  0x40

#define ISR_PRX 0x01 /* packet received */
#define ISR_PTX 0x02 /* packet transmitted */
#define ISR_RXE 0x04
#define ISR_TXE 0x08
#define ISR_RDC 0x40 /* remote DMA complete */
#define ISR_RST 0x80

/* Buffer page layout: one 6-page (1536 byte) region for transmit, the
 * rest of the card's buffer memory as the receive ring - the
 * conventional layout every reference NE2000 driver uses. */
#define TX_PAGE_START 0x40
#define RX_PAGE_START 0x46
#define RX_PAGE_STOP  0x80

static bool present = false;
static uint8_t mac_address[6];
static uint8_t next_bnry;

static void select_page(uint8_t page01) {
    uint8_t cr = inb(REG_CR);
    cr = (uint8_t)((cr & ~CR_PS0) | (page01 ? CR_PS0 : 0));
    outb(REG_CR, cr);
}

void ne2000_init(void) {
    present = false;

    /* Reset: reading the reset port and writing the same value back
     * triggers a soft reset; wait for the card to signal completion
     * via ISR.RST, then clear it. */
    uint8_t reset_val = inb(REG_RESET);
    outb(REG_RESET, reset_val);

    uint32_t spins = 0;
    while (!(inb(REG_ISR) & ISR_RST)) {
        if (++spins > 1000000u) {
            kernel_log("[ .. ] NE2000: no card at 0x%x (reset timeout)\n",
                       NE_IOBASE);
            return;
        }
    }
    outb(REG_ISR, 0xFF); /* clear all status bits */

    outb(REG_CR, CR_STP | CR_RD2);   /* stop, abort any DMA */
    outb(REG_DCR, 0x49);             /* word transfers, normal FIFO */
    outb(REG_RBCR0, 0x00);
    outb(REG_RBCR1, 0x00);
    outb(REG_RCR, 0x04);             /* accept broadcast packets */
    outb(REG_TCR, 0x00);             /* normal operation, no loopback */
    outb(REG_PSTART, RX_PAGE_START);
    outb(REG_PSTOP, RX_PAGE_STOP);
    outb(REG_BNRY, RX_PAGE_START);
    next_bnry = RX_PAGE_START;

    /* Read the MAC address out of the card's PROM via remote DMA. The
     * NE2000 stores each PROM byte twice (a 16-bit-bus-width quirk),
     * so a 32-byte read yields the 6 real MAC bytes at offsets
     * 0,2,4,6,8,10. */
    outb(REG_RBCR0, 32);
    outb(REG_RBCR1, 0);
    outb(REG_RSAR0, 0);
    outb(REG_RSAR1, 0);
    outb(REG_CR, CR_STA | 0x08 /* remote read */);

    uint8_t prom[32];
    for (int i = 0; i < 16; i++) {
        uint16_t word = inw(REG_DATA);
        prom[i * 2] = (uint8_t)(word & 0xFF);
        prom[i * 2 + 1] = (uint8_t)(word >> 8);
    }
    for (int i = 0; i < 6; i++) {
        mac_address[i] = prom[i * 2];
    }

    select_page(1);
    for (int i = 0; i < 6; i++) {
        outb((uint16_t)(REG_PAR0 + i), mac_address[i]);
    }
    outb(REG_CURR, RX_PAGE_START + 1);
    select_page(0);

    outb(REG_ISR, 0xFF);
    outb(REG_IMR, 0x00); /* polled, not interrupt-driven */
    outb(REG_CR, CR_STA);

    present = true;
    kernel_log("[ OK ] NE2000 NIC at 0x%x, MAC %x:%x:%x:%x:%x:%x\n",
               NE_IOBASE, mac_address[0], mac_address[1], mac_address[2],
               mac_address[3], mac_address[4], mac_address[5]);
}

bool ne2000_is_present(void) {
    return present;
}

const uint8_t* ne2000_mac_address(void) {
    return mac_address;
}

bool ne2000_send(const void* frame, uint16_t length) {
    if (!present) {
        return false;
    }
    if (length < 60) {
        length = 60; /* Ethernet minimum frame size */
    }

    uint32_t spins = 0;
    while (inb(REG_CR) & CR_TXP) {
        if (++spins > 1000000u) {
            return false;
        }
    }

    outb(REG_ISR, ISR_RDC);
    outb(REG_RBCR0, (uint8_t)(length & 0xFF));
    outb(REG_RBCR1, (uint8_t)(length >> 8));
    outb(REG_RSAR0, 0);
    outb(REG_RSAR1, TX_PAGE_START);
    outb(REG_CR, CR_STA | 0x10 /* remote write */);

    const uint16_t* words = (const uint16_t*)frame;
    uint16_t word_count = (uint16_t)((length + 1) / 2);
    for (uint16_t i = 0; i < word_count; i++) {
        outw(REG_DATA, words[i]);
    }

    spins = 0;
    while (!(inb(REG_ISR) & ISR_RDC)) {
        if (++spins > 1000000u) {
            return false;
        }
    }
    outb(REG_ISR, ISR_RDC);

    outb(REG_TPSR, TX_PAGE_START);
    outb(REG_TBCR0, (uint8_t)(length & 0xFF));
    outb(REG_TBCR1, (uint8_t)(length >> 8));
    outb(REG_CR, CR_STA | CR_TXP);

    spins = 0;
    while (!(inb(REG_ISR) & (ISR_PTX | ISR_TXE))) {
        if (++spins > 1000000u) {
            return false;
        }
    }
    bool ok = (inb(REG_ISR) & ISR_TXE) == 0;
    outb(REG_ISR, ISR_PTX | ISR_TXE);
    return ok;
}

uint16_t ne2000_receive(void* buffer) {
    if (!present) {
        return 0;
    }

    select_page(1);
    uint8_t curr = inb(REG_CURR);
    select_page(0);

    /* `next_bnry` mirrors the hardware BNRY register: the last page
     * the driver has freed, NOT the next unread page - that's
     * next_bnry + 1 (wrapping at the top of the ring). Reading from
     * next_bnry directly (an earlier version of this function did)
     * reads the page the driver already finished with last time,
     * which is either stale or - right after init, before anything
     * has ever been read - genuinely empty, producing an all-zero
     * header. Found by comparing a QEMU packet capture (proving a
     * reply genuinely arrived and CURR advanced) against this
     * function returning nothing. */
    uint8_t read_page = (uint8_t)(next_bnry + 1);
    if (read_page > RX_PAGE_STOP - 1) {
        read_page = RX_PAGE_START;
    }

    if (read_page == curr) {
        return 0; /* nothing new */
    }

    /* Each buffered packet starts with a 4-byte NE2000 receive header
     * (status, next-packet-page, length-lo, length-hi) that the NIC
     * itself prepends - not part of the Ethernet frame. */
    uint8_t header[4];
    outb(REG_RBCR0, 4);
    outb(REG_RBCR1, 0);
    outb(REG_RSAR0, 0);
    outb(REG_RSAR1, read_page);
    outb(REG_CR, CR_STA | 0x08);
    for (int i = 0; i < 2; i++) {
        uint16_t word = inw(REG_DATA);
        header[i * 2] = (uint8_t)(word & 0xFF);
        header[i * 2 + 1] = (uint8_t)(word >> 8);
    }

    uint8_t next_page = header[1];
    uint16_t length = (uint16_t)(header[2] | (header[3] << 8));

    if (length > NE2000_MAX_FRAME || length < 4) {
        /* Corrupt ring state - resynchronize rather than trust it. */
        outb(REG_BNRY, curr);
        next_bnry = curr;
        return 0;
    }
    uint16_t data_length = (uint16_t)(length - 4);

    /* Read the actual frame data, which immediately follows the
     * header at the same buffer address (the remote DMA address
     * auto-increments, so a second, longer read continues where the
     * first left off - no need to reissue RSAR for it, but doing so
     * explicitly here is simpler to reason about and costs nothing). */
    outb(REG_RBCR0, (uint8_t)(data_length & 0xFF));
    outb(REG_RBCR1, (uint8_t)(data_length >> 8));
    outb(REG_RSAR0, 4);
    outb(REG_RSAR1, read_page);
    outb(REG_CR, CR_STA | 0x08);

    uint8_t* out = (uint8_t*)buffer;
    uint16_t word_count = (uint16_t)((data_length + 1) / 2);
    for (uint16_t i = 0; i < word_count; i++) {
        uint16_t word = inw(REG_DATA);
        out[i * 2] = (uint8_t)(word & 0xFF);
        if (i * 2 + 1 < data_length) {
            out[i * 2 + 1] = (uint8_t)(word >> 8);
        }
    }

    /* next_bnry (and the hardware BNRY register) always store the
     * last freed page, i.e. next_page - 1 - consistent with how this
     * function interprets next_bnry on entry (see read_page above). */
    uint8_t new_bnry = (next_page == RX_PAGE_START)
                            ? (uint8_t)(RX_PAGE_STOP - 1)
                            : (uint8_t)(next_page - 1);
    next_bnry = new_bnry;
    outb(REG_BNRY, new_bnry);

    return data_length;
}
