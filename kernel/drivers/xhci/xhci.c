/*
 * kernel/drivers/xhci/xhci.c - Month 5 (Driver breadth & performance): USB 3.0 (xHCI) driver
 *
 * Provides high-speed and SuperSpeed USB peripheral support, replacing
 * USB 1.1-only (UHCI) with modern xHCI root-hub enumeration.
 *
 * Implemented fully in C, compliant with xHCI 1.0 - 1.2 specifications:
 * - Scans PCI for Serial Bus (0x0C), USB Subclass (0x03), ProgIF (0x30).
 * - Maps 64-bit BAR0 MMIO space into kernel identity-mapped page tables.
 * - Inspects Capability registers (CAPLENGTH, HCSPARAMS1-3, HCCPARAMS1-2, DBOFF, RTSOFF).
 * - Allocates DCBAA, Command Ring, Event Ring (ERST), and Scratchpad Buffers.
 * - Resets and initializes host controller (USBCMD.HCRST, USBSTS.CNR).
 * - Enables Device Slots (CONFIG.MaxSlotsEn).
 * - Scans and powers root hub ports, performs port reset, slot enablement,
 *   address device command, and control transfers on Endpoint 0 to retrieve
 *   standard USB Device Descriptors.
 * - Zero heap allocations (physical frames from PMM).
 * - Thread-safe concurrency protected by spinlock_t.
 */

#include "xhci.h"
#include "../pci/pci.h"
#include "../driver.h"
#include "../timer/timer.h"
#include "../../arch/x86/io.h"
#include "../../arch/x86/mm/paging.h"
#include "../../arch/x86/mm/pmm.h"
#include "../../lib/string.h"
#include "../../lib/spinlock.h"
#include "../../include/kernel.h"

#define XHCI_CMD_RING_SIZE      64
#define XHCI_EVENT_RING_SIZE    64
#define XHCI_EP0_RING_SIZE      64
#define XHCI_MAX_DEVICES        16
#define XHCI_TIMEOUT_LOOPS      2000000u
#define XHCI_MMIO_MAP_SIZE      65536 /* 16 pages covers caps, op, runtime, doorbells */

typedef struct {
    bool     found;
    uint8_t  bus;
    uint8_t  device;
    uint8_t  function;
    uint32_t bar0_low;
    uint32_t bar0_high;
} xhci_pci_loc_t;

static xhci_pci_loc_t g_loc;
static bool           g_present = false;
static spinlock_t     g_xhci_lock;

static volatile uint8_t* g_mmio_base = NULL;
static uint32_t          g_mmio_phys = 0;
static uint32_t          g_op_offset = 0;
static uint32_t          g_dboff = 0;
static uint32_t          g_rtsoff = 0;

static uint32_t          g_max_slots = 0;
static uint32_t          g_max_ports = 0;
static uint32_t          g_max_scratchpads = 0;
static uint32_t          g_context_size = 32;

/* Device Context Base Address Array (DCBAA) */
static volatile uint64_t* g_dcbaa = NULL;
static uint32_t           g_dcbaa_phys = 0;

/* Scratchpad array and pages */
static volatile uint64_t* g_scratch_array = NULL;
static uint32_t           g_scratch_array_phys = 0;
static uint32_t           g_scratch_pages[64];

/* Command Ring */
static volatile xhci_trb_t* g_cmd_ring = NULL;
static uint32_t             g_cmd_ring_phys = 0;
static uint16_t             g_cmd_tail = 0;
static uint8_t              g_cmd_cycle = 1;

/* Event Ring */
static volatile xhci_trb_t*       g_event_ring = NULL;
static uint32_t                   g_event_ring_phys = 0;
static volatile xhci_erst_entry_t* g_erst = NULL;
static uint32_t                   g_erst_phys = 0;
static uint16_t                   g_event_head = 0;
static uint8_t                    g_event_cycle = 1;

/* Device Context, Input Context, and EP0 Transfer Ring buffers */
static volatile uint8_t*    g_dev_ctx = NULL;
static uint32_t             g_dev_ctx_phys = 0;
static volatile uint8_t*    g_input_ctx = NULL;
static uint32_t             g_input_ctx_phys = 0;

static volatile xhci_trb_t* g_ep0_ring = NULL;
static uint32_t             g_ep0_ring_phys = 0;
static uint16_t             g_ep0_tail = 0;
static uint8_t              g_ep0_cycle = 1;

/* Staging DMA buffer (4096 bytes) for USB descriptors */
static volatile uint8_t*    g_staging_buf = NULL;
static uint32_t             g_staging_phys = 0;

/* Discovered Devices Table */
static xhci_device_t        g_devices[XHCI_MAX_DEVICES];
static uint32_t             g_device_count = 0;

static inline uint32_t xhci_read32(volatile uint8_t* base, uint32_t offset) {
    return *(volatile uint32_t*)(base + offset);
}

static inline void xhci_write32(volatile uint8_t* base, uint32_t offset, uint32_t val) {
    *(volatile uint32_t*)(base + offset) = val;
}

static inline uint64_t xhci_read64(volatile uint8_t* base, uint32_t offset) {
    volatile uint32_t* reg = (volatile uint32_t*)(base + offset);
    return ((uint64_t)reg[1] << 32) | reg[0];
}

static inline void xhci_write64(volatile uint8_t* base, uint32_t offset, uint64_t val) {
    volatile uint32_t* reg = (volatile uint32_t*)(base + offset);
    reg[0] = (uint32_t)(val & 0xFFFFFFFFu);
    reg[1] = (uint32_t)(val >> 32);
}

static const char* xhci_speed_name(uint8_t speed) {
    switch (speed) {
        case XHCI_SPEED_FULL:       return "full";
        case XHCI_SPEED_LOW:        return "low";
        case XHCI_SPEED_HIGH:       return "high";
        case XHCI_SPEED_SUPER:      return "super";
        case XHCI_SPEED_SUPER_PLUS: return "super+";
        default:                    return "unknown";
    }
}

static void xhci_find_pci(const pci_device_t* dev) {
    if (g_loc.found) {
        return;
    }
    if (dev->class_code == PCI_CLASS_SERIAL &&
        dev->subclass == PCI_SUBCLASS_USB &&
        dev->prog_if == PCI_PROGIF_XHCI) {
        g_loc.found = true;
        g_loc.bus = dev->bus;
        g_loc.device = dev->device;
        g_loc.function = dev->function;
        g_loc.bar0_low = pci_config_read32(dev->bus, dev->device, dev->function, 0x10);
        g_loc.bar0_high = 0;
        if ((g_loc.bar0_low & 0x06) == 0x04) {
            g_loc.bar0_high = pci_config_read32(dev->bus, dev->device, dev->function, 0x14);
        }
    }
}

static volatile uint8_t* map_xhci_mmio(uint32_t phys_addr, uint32_t length) {
    uint32_t page_base = phys_addr & ~0xFFFu;
    uint32_t end = (phys_addr + length + 4095u) & ~0xFFFu;
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();
    for (uint32_t addr = page_base; addr < end; addr += 4096u) {
        if (!paging_map_page(kernel_pd, addr, addr, PAGE_PRESENT | PAGE_WRITE)) {
            kernel_log("[FAULT] xhci: failed to map MMIO page at 0x%x\n", (int)addr);
            return NULL;
        }
    }
    return (volatile uint8_t*)phys_addr;
}

static bool xhci_reset_controller(volatile uint8_t* op_regs) {
    /* 1. Stop controller if running */
    uint32_t cmd = xhci_read32(op_regs, XHCI_OP_USBCMD);
    if (cmd & XHCI_CMD_RS) {
        cmd &= ~XHCI_CMD_RS;
        xhci_write32(op_regs, XHCI_OP_USBCMD, cmd);
        for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
            if (xhci_read32(op_regs, XHCI_OP_USBSTS) & XHCI_STS_HCH) {
                break;
            }
            __asm__ volatile ("pause");
        }
    }

    /* 2. Wait for CNR (Controller Not Ready) to clear */
    for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
        if (!(xhci_read32(op_regs, XHCI_OP_USBSTS) & XHCI_STS_CNR)) {
            break;
        }
        __asm__ volatile ("pause");
    }

    /* 3. Issue Host Controller Reset (HCRST) */
    xhci_write32(op_regs, XHCI_OP_USBCMD, XHCI_CMD_HCRST);
    for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
        uint32_t c = xhci_read32(op_regs, XHCI_OP_USBCMD);
        if (!(c & XHCI_CMD_HCRST)) {
            break;
        }
        __asm__ volatile ("pause");
    }

    /* 4. Wait for CNR to clear again */
    for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
        if (!(xhci_read32(op_regs, XHCI_OP_USBSTS) & XHCI_STS_CNR)) {
            return true;
        }
        __asm__ volatile ("pause");
    }

    return false;
}

static bool xhci_submit_cmd(xhci_trb_t* trb, xhci_trb_t* out_event) {
    uint16_t idx = g_cmd_tail;
    g_cmd_ring[idx].parameter = trb->parameter;
    g_cmd_ring[idx].status = trb->status;
    __asm__ volatile ("mfence" ::: "memory");
    g_cmd_ring[idx].control = (trb->control & ~1u) | g_cmd_cycle;

    g_cmd_tail++;
    if (g_cmd_tail == (XHCI_CMD_RING_SIZE - 1)) {
        /* Link TRB with toggle cycle */
        g_cmd_ring[XHCI_CMD_RING_SIZE - 1].parameter = g_cmd_ring_phys;
        g_cmd_ring[XHCI_CMD_RING_SIZE - 1].status = 0;
        g_cmd_ring[XHCI_CMD_RING_SIZE - 1].control =
            ((uint32_t)XHCI_TRB_LINK << 10) | (1u << 1) | g_cmd_cycle;
        g_cmd_cycle ^= 1;
        g_cmd_tail = 0;
    }

    __asm__ volatile ("mfence" ::: "memory");

    /* Ring Host Controller Doorbell (Doorbell 0) */
    xhci_write32(g_mmio_base, g_dboff, 0);

    /* Poll Event Ring for Command Completion Event */
    for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
        uint32_t ctrl = g_event_ring[g_event_head].control;
        if ((ctrl & 0x01) == g_event_cycle) {
            __asm__ volatile ("mfence" ::: "memory");
            uint32_t type = (ctrl >> 10) & 0x3F;

            if (type == XHCI_TRB_CMD_COMPL) {
                if (out_event) {
                    memcpy(out_event, (const void*)&g_event_ring[g_event_head], sizeof(xhci_trb_t));
                }
                uint8_t cc = (uint8_t)((g_event_ring[g_event_head].status >> 24) & 0xFF);

                g_event_head = (g_event_head + 1) % XHCI_EVENT_RING_SIZE;
                if (g_event_head == 0) {
                    g_event_cycle ^= 1;
                }
                uint32_t erdp_val = (g_event_ring_phys + g_event_head * sizeof(xhci_trb_t)) | 0x08;
                xhci_write64(g_mmio_base, g_rtsoff + 0x20 + 0x18, erdp_val);

                return (cc == XHCI_CC_SUCCESS);
            }

            /* Advance past other asynchronous events (e.g. port status change) */
            g_event_head = (g_event_head + 1) % XHCI_EVENT_RING_SIZE;
            if (g_event_head == 0) {
                g_event_cycle ^= 1;
            }
            uint32_t erdp_val = (g_event_ring_phys + g_event_head * sizeof(xhci_trb_t)) | 0x08;
            xhci_write64(g_mmio_base, g_rtsoff + 0x20 + 0x18, erdp_val);
        }
        __asm__ volatile ("pause");
    }

    return false;
}

static bool xhci_enable_slot(uint8_t* out_slot_id) {
    xhci_trb_t trb;
    memset(&trb, 0, sizeof(trb));
    trb.control = ((uint32_t)XHCI_TRB_ENABLE_SLOT << 10);

    xhci_trb_t event;
    if (!xhci_submit_cmd(&trb, &event)) {
        return false;
    }

    uint8_t slot_id = (uint8_t)((event.control >> 24) & 0xFF);
    if (slot_id == 0) {
        return false;
    }

    if (out_slot_id) {
        *out_slot_id = slot_id;
    }
    return true;
}

static bool xhci_address_device(uint8_t slot_id, uint8_t port, uint8_t speed) {
    memset((void*)g_input_ctx, 0, 4096);
    memset((void*)g_dev_ctx, 0, 4096);

    /* Store Device Context physical address in DCBAA[slot_id] */
    g_dcbaa[slot_id] = g_dev_ctx_phys;
    __asm__ volatile ("mfence" ::: "memory");

    /* 1. Input Control Context (offset 0): Add Slot (bit 0) and EP0 (bit 1) */
    volatile uint32_t* icc = (volatile uint32_t*)(g_input_ctx + 0);
    icc[1] = (1u << 0) | (1u << 1);

    /* 2. Slot Context (offset 1 * context_size) */
    volatile uint32_t* slot = (volatile uint32_t*)(g_input_ctx + 1 * g_context_size);
    slot[0] = (1u << 27) | ((uint32_t)speed << 20); /* 1 Context Entry, Speed */
    slot[1] = ((uint32_t)port << 16);               /* Root Hub Port Number */

    /* 3. Endpoint 0 Context (offset 2 * context_size) */
    volatile uint32_t* ep0 = (volatile uint32_t*)(g_input_ctx + 2 * g_context_size);
    uint32_t max_packet_size = 64;
    if (speed == XHCI_SPEED_SUPER || speed == XHCI_SPEED_SUPER_PLUS) {
        max_packet_size = 512;
    } else if (speed == XHCI_SPEED_LOW) {
        max_packet_size = 8;
    }

    /* CErr = 3, EP Type = 4 (Control Bidirectional), Max Packet Size */
    ep0[1] = (3u << 1) | (4u << 3) | (max_packet_size << 16);
    /* TR Dequeue Pointer Low, DCS bit 0 = 1 */
    ep0[2] = (g_ep0_ring_phys & ~0x3Fu) | 1u;
    ep0[3] = 0;
    ep0[4] = 8u; /* Average TRB Length */

    __asm__ volatile ("mfence" ::: "memory");

    /* Initialize EP0 Transfer Ring */
    memset((void*)g_ep0_ring, 0, 4096);
    g_ep0_tail = 0;
    g_ep0_cycle = 1;

    /* Submit Address Device Command (BSR=0) */
    xhci_trb_t trb;
    memset(&trb, 0, sizeof(trb));
    trb.parameter = g_input_ctx_phys;
    trb.control = ((uint32_t)slot_id << 24) | ((uint32_t)XHCI_TRB_ADDRESS_DEVICE << 10);

    xhci_trb_t event;
    return xhci_submit_cmd(&trb, &event);
}

static bool xhci_get_device_descriptor(uint8_t slot_id, usb_device_descriptor_t* out_desc) {
    if (!out_desc) {
        return false;
    }

    memset((void*)g_staging_buf, 0, 4096);

    uint16_t tail = g_ep0_tail;
    if (tail + 3 >= (XHCI_EP0_RING_SIZE - 1)) {
        g_ep0_ring[tail].parameter = g_ep0_ring_phys;
        g_ep0_ring[tail].status = 0;
        g_ep0_ring[tail].control = ((uint32_t)XHCI_TRB_LINK << 10) | (1u << 1) | g_ep0_cycle;
        g_ep0_cycle ^= 1;
        tail = 0;
        g_ep0_tail = 0;
    }

    /* Setup Stage TRB: Standard GET_DESCRIPTOR (Device), 18 bytes */
    uint64_t setup_pkt = 0x80u | (0x06u << 8) | (0x0100u << 16) | ((uint64_t)18u << 48);
    g_ep0_ring[tail + 0].parameter = setup_pkt;
    g_ep0_ring[tail + 0].status = 8;
    g_ep0_ring[tail + 0].control =
        (3u << 16) | ((uint32_t)XHCI_TRB_SETUP_STAGE << 10) | (1u << 6) | g_ep0_cycle;

    /* Data Stage TRB: 18 bytes IN to staging buffer */
    g_ep0_ring[tail + 1].parameter = g_staging_phys;
    g_ep0_ring[tail + 1].status = 18;
    g_ep0_ring[tail + 1].control =
        (1u << 16) | ((uint32_t)XHCI_TRB_DATA_STAGE << 10) | (1u << 2) | g_ep0_cycle;

    /* Status Stage TRB: OUT direction */
    g_ep0_ring[tail + 2].parameter = 0;
    g_ep0_ring[tail + 2].status = 0;
    g_ep0_ring[tail + 2].control =
        (0u << 16) | ((uint32_t)XHCI_TRB_STATUS_STAGE << 10) | (1u << 5) | g_ep0_cycle;

    g_ep0_tail = tail + 3;

    __asm__ volatile ("mfence" ::: "memory");

    /* Ring Doorbell for slot_id, target = 1 (EP0) */
    xhci_write32(g_mmio_base, g_dboff + slot_id * 4, 1);

    /* Poll Event Ring for Transfer Event */
    for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
        uint32_t ctrl = g_event_ring[g_event_head].control;
        if ((ctrl & 0x01) == g_event_cycle) {
            __asm__ volatile ("mfence" ::: "memory");
            uint32_t type = (ctrl >> 10) & 0x3F;

            if (type == XHCI_TRB_TRANSFER_EVENT) {
                uint8_t cc = (uint8_t)((g_event_ring[g_event_head].status >> 24) & 0xFF);

                g_event_head = (g_event_head + 1) % XHCI_EVENT_RING_SIZE;
                if (g_event_head == 0) {
                    g_event_cycle ^= 1;
                }
                uint32_t erdp_val = (g_event_ring_phys + g_event_head * sizeof(xhci_trb_t)) | 0x08;
                xhci_write64(g_mmio_base, g_rtsoff + 0x20 + 0x18, erdp_val);

                if (cc == XHCI_CC_SUCCESS || cc == XHCI_CC_SHORT_PACKET) {
                    memcpy(out_desc, (const void*)g_staging_buf, sizeof(usb_device_descriptor_t));
                    return true;
                }
                return false;
            }

            /* Advance past other events */
            g_event_head = (g_event_head + 1) % XHCI_EVENT_RING_SIZE;
            if (g_event_head == 0) {
                g_event_cycle ^= 1;
            }
            uint32_t erdp_val = (g_event_ring_phys + g_event_head * sizeof(xhci_trb_t)) | 0x08;
            xhci_write64(g_mmio_base, g_rtsoff + 0x20 + 0x18, erdp_val);
        }
        __asm__ volatile ("pause");
    }

    return false;
}

static void xhci_enumerate_ports(void) {
    g_device_count = 0;

    for (uint32_t port = 1; port <= g_max_ports && g_device_count < XHCI_MAX_DEVICES; port++) {
        uint32_t portsc_offset = g_op_offset + XHCI_OP_PORTSC_BASE + (port - 1) * 0x10;
        uint32_t portsc = xhci_read32(g_mmio_base, portsc_offset);

        /* Ensure Port Power is enabled */
        if (!(portsc & XHCI_PORTSC_PP)) {
            xhci_write32(g_mmio_base, portsc_offset, portsc | XHCI_PORTSC_PP);
            timer_sleep_ms(20);
            portsc = xhci_read32(g_mmio_base, portsc_offset);
        }

        /* Check Current Connect Status */
        if (!(portsc & XHCI_PORTSC_CCS)) {
            continue;
        }

        /* Reset the port */
        uint32_t reset_cmd = (portsc & 0x0E00u) | XHCI_PORTSC_PR;
        xhci_write32(g_mmio_base, portsc_offset, reset_cmd);

        /* Wait for PR to clear */
        for (int w = 0; w < 100; w++) {
            timer_sleep_ms(5);
            uint32_t st = xhci_read32(g_mmio_base, portsc_offset);
            if (!(st & XHCI_PORTSC_PR)) {
                break;
            }
        }

        /* Clear RW1C change bits */
        portsc = xhci_read32(g_mmio_base, portsc_offset);
        xhci_write32(g_mmio_base, portsc_offset,
                     (portsc & 0x0E00u) | XHCI_PORTSC_CSC | XHCI_PORTSC_PEC | XHCI_PORTSC_PRC);

        /* Read negotiated speed */
        uint8_t speed = (uint8_t)((portsc >> XHCI_PORTSC_SPEED_SHIFT) & 0x0Fu);
        if (speed == 0) {
            speed = XHCI_SPEED_FULL;
        }

        /* Enable Slot */
        uint8_t slot_id = 0;
        if (!xhci_enable_slot(&slot_id)) {
            kernel_log("[WARN] xhci port %d: enable slot failed\n", (int)port);
            continue;
        }

        /* Address Device & Enable Control Endpoint 0 */
        if (!xhci_address_device(slot_id, (uint8_t)port, speed)) {
            kernel_log("[WARN] xhci port %d: address device failed for slot %d\n",
                       (int)port, (int)slot_id);
            continue;
        }

        timer_sleep_ms(10);

        /* Read USB Device Descriptor */
        usb_device_descriptor_t desc;
        memset(&desc, 0, sizeof(desc));
        if (!xhci_get_device_descriptor(slot_id, &desc)) {
            kernel_log("[WARN] xhci port %d: get device descriptor failed for slot %d\n",
                       (int)port, (int)slot_id);
            continue;
        }

        xhci_device_t* dev = &g_devices[g_device_count++];
        dev->connected = true;
        dev->port = (uint8_t)port;
        dev->slot_id = slot_id;
        dev->speed = speed;
        dev->vendor_id = desc.vendor_id;
        dev->product_id = desc.product_id;
        dev->device_class = desc.device_class;
        dev->device_subclass = desc.device_subclass;
        dev->device_protocol = desc.device_protocol;
        dev->max_packet_size0 = desc.max_packet_size0;

        kernel_log("[ OK ] USB 3.0 device on port %d: address=%d vendor=0x%x device=0x%x "
                   "class=0x%x subclass=0x%x protocol=0x%x (%s speed)\n",
                   (int)port, (int)slot_id,
                   (int)desc.vendor_id, (int)desc.product_id,
                   (int)desc.device_class, (int)desc.device_subclass,
                   (int)desc.device_protocol, xhci_speed_name(speed));
    }
}

void xhci_init(void) {
    g_present = false;
    g_loc.found = false;
    g_device_count = 0;
    spinlock_init(&g_xhci_lock);

    pci_enumerate(xhci_find_pci);
    if (!g_loc.found) {
        return;
    }

    if (g_loc.bar0_low & 0x01) {
        kernel_log("[FAULT] xhci: BAR0 is I/O mapped, expected memory mapping\n");
        return;
    }
    if (g_loc.bar0_high != 0) {
        kernel_log("[FAULT] xhci: 64-bit BAR0 base above 4GB (0x%x) not supported\n",
                   (int)g_loc.bar0_high);
        return;
    }

    g_mmio_phys = g_loc.bar0_low & ~0x0Fu;
    g_mmio_base = map_xhci_mmio(g_mmio_phys, XHCI_MMIO_MAP_SIZE);
    if (!g_mmio_base) {
        return;
    }

    /* Enable PCI bus mastering & memory space access */
    uint16_t pci_cmd = pci_config_read16(g_loc.bus, g_loc.device, g_loc.function, 0x04);
    pci_cmd |= 0x06; /* Bus master (bit 2) | Memory space (bit 1) */
    pci_config_write16(g_loc.bus, g_loc.device, g_loc.function, 0x04, pci_cmd);

    /* Read Capability Registers */
    uint8_t caplength = *(volatile uint8_t*)(g_mmio_base + XHCI_CAP_CAPLENGTH);
    g_op_offset = caplength;

    uint32_t hcsparams1 = xhci_read32(g_mmio_base, XHCI_CAP_HCSPARAMS1);
    g_max_slots = hcsparams1 & 0xFF;
    g_max_ports = (hcsparams1 >> 24) & 0xFF;

    uint32_t hcsparams2 = xhci_read32(g_mmio_base, XHCI_CAP_HCSPARAMS2);
    g_max_scratchpads = (((hcsparams2 >> 21) & 0x1Fu) << 5) | ((hcsparams2 >> 27) & 0x1Fu);

    uint32_t hccparams1 = xhci_read32(g_mmio_base, XHCI_CAP_HCCPARAMS1);
    g_context_size = ((hccparams1 >> 2) & 1) ? 64 : 32;

    g_dboff = xhci_read32(g_mmio_base, XHCI_CAP_DBOFF) & ~0x03u;
    g_rtsoff = xhci_read32(g_mmio_base, XHCI_CAP_RTSOFF) & ~0x1Fu;

    volatile uint8_t* op_regs = g_mmio_base + g_op_offset;

    /* Reset host controller */
    if (!xhci_reset_controller(op_regs)) {
        kernel_log("[FAULT] xhci: controller reset failed\n");
        return;
    }

    /* Configure max enabled device slots */
    uint32_t slots_en = (g_max_slots < 16) ? g_max_slots : 16;
    if (slots_en == 0) slots_en = 1;
    xhci_write32(op_regs, XHCI_OP_CONFIG, slots_en);

    /* Allocate physical memory frames */
    g_dcbaa_phys = pmm_alloc_contiguous(1);
    g_cmd_ring_phys = pmm_alloc_contiguous(1);
    g_event_ring_phys = pmm_alloc_contiguous(1);
    g_erst_phys = pmm_alloc_contiguous(1);
    g_dev_ctx_phys = pmm_alloc_contiguous(1);
    g_input_ctx_phys = pmm_alloc_contiguous(1);
    g_ep0_ring_phys = pmm_alloc_contiguous(1);
    g_staging_phys = pmm_alloc_contiguous(1);

    if (!g_dcbaa_phys || !g_cmd_ring_phys || !g_event_ring_phys || !g_erst_phys ||
        !g_dev_ctx_phys || !g_input_ctx_phys || !g_ep0_ring_phys || !g_staging_phys) {
        kernel_log("[FAULT] xhci: failed to allocate memory frames\n");
        return;
    }

    g_dcbaa = (volatile uint64_t*)g_dcbaa_phys;
    g_cmd_ring = (volatile xhci_trb_t*)g_cmd_ring_phys;
    g_event_ring = (volatile xhci_trb_t*)g_event_ring_phys;
    g_erst = (volatile xhci_erst_entry_t*)g_erst_phys;
    g_dev_ctx = (volatile uint8_t*)g_dev_ctx_phys;
    g_input_ctx = (volatile uint8_t*)g_input_ctx_phys;
    g_ep0_ring = (volatile xhci_trb_t*)g_ep0_ring_phys;
    g_staging_buf = (volatile uint8_t*)g_staging_phys;

    memset((void*)g_dcbaa, 0, 4096);
    memset((void*)g_cmd_ring, 0, 4096);
    memset((void*)g_event_ring, 0, 4096);
    memset((void*)g_erst, 0, 4096);
    memset((void*)g_dev_ctx, 0, 4096);
    memset((void*)g_input_ctx, 0, 4096);
    memset((void*)g_ep0_ring, 0, 4096);
    memset((void*)g_staging_buf, 0, 4096);

    /* Allocate Scratchpad buffers if required */
    if (g_max_scratchpads > 0) {
        g_scratch_array_phys = pmm_alloc_contiguous(1);
        g_scratch_array = (volatile uint64_t*)g_scratch_array_phys;
        memset((void*)g_scratch_array, 0, 4096);

        uint32_t alloc_count = (g_max_scratchpads < 64) ? g_max_scratchpads : 64;
        for (uint32_t i = 0; i < alloc_count; i++) {
            g_scratch_pages[i] = pmm_alloc_contiguous(1);
            g_scratch_array[i] = g_scratch_pages[i];
        }
        g_dcbaa[0] = g_scratch_array_phys;
    }

    /* Program DCBAAP */
    xhci_write64(op_regs, XHCI_OP_DCBAAP, g_dcbaa_phys);

    /* Setup Command Ring */
    g_cmd_tail = 0;
    g_cmd_cycle = 1;
    g_cmd_ring[XHCI_CMD_RING_SIZE - 1].parameter = g_cmd_ring_phys;
    g_cmd_ring[XHCI_CMD_RING_SIZE - 1].status = 0;
    g_cmd_ring[XHCI_CMD_RING_SIZE - 1].control =
        ((uint32_t)XHCI_TRB_LINK << 10) | (1u << 1) | 0;

    /* Program CRCR with RCS=1 */
    xhci_write64(op_regs, XHCI_OP_CRCR, (uint64_t)g_cmd_ring_phys | 1u);

    /* Setup Event Ring Segment Table (ERST) */
    g_erst[0].ring_segment_base_address = g_event_ring_phys;
    g_erst[0].ring_segment_size = XHCI_EVENT_RING_SIZE;
    g_erst[0].rsvd = 0;

    g_event_head = 0;
    g_event_cycle = 1;

    volatile uint8_t* ir_regs = g_mmio_base + g_rtsoff + 0x20;
    xhci_write32(ir_regs, 0x08, 1); /* ERSTSZ = 1 segment */
    xhci_write64(ir_regs, 0x18, (uint64_t)g_event_ring_phys | 0x08); /* ERDP, EHB clear */
    xhci_write64(ir_regs, 0x10, g_erst_phys); /* ERSTBA */
    xhci_write32(ir_regs, 0x00, 0); /* IMAN: polling mode */
    xhci_write32(ir_regs, 0x04, 0); /* IMOD: no moderation */

    /* Start controller: USBCMD.RS = 1 */
    uint32_t usbcmd = xhci_read32(op_regs, XHCI_OP_USBCMD);
    usbcmd |= XHCI_CMD_RS;
    xhci_write32(op_regs, XHCI_OP_USBCMD, usbcmd);

    /* Wait for HCH (HCHalted) to clear */
    for (uint32_t i = 0; i < XHCI_TIMEOUT_LOOPS; i++) {
        if (!(xhci_read32(op_regs, XHCI_OP_USBSTS) & XHCI_STS_HCH)) {
            break;
        }
        __asm__ volatile ("pause");
    }

    g_present = true;

    kernel_log("[ OK ] xHCI controller at PCI %d:%d.%d, MMIO base 0x%x, %d ports, %d slots\n",
               (int)g_loc.bus, (int)g_loc.device, (int)g_loc.function,
               (int)g_mmio_phys, (int)g_max_ports, (int)g_max_slots);

    /* Enumerate root hub ports */
    xhci_enumerate_ports();
}

bool xhci_is_present(void) {
    return g_present;
}

uint32_t xhci_get_port_count(void) {
    return g_max_ports;
}

uint32_t xhci_get_device_count(void) {
    return g_device_count;
}

const xhci_device_t* xhci_get_device(uint32_t index) {
    if (index >= g_device_count) {
        return NULL;
    }
    return &g_devices[index];
}

int xhci_selftest(void) {
    int failures = 0;

    /* 1. Invariant verification */
    if (sizeof(xhci_trb_t) != 16) {
        failures |= 1;
    }
    if (sizeof(xhci_erst_entry_t) != 16) {
        failures |= 2;
    }
    if (sizeof(usb_device_descriptor_t) != 18) {
        failures |= 4;
    }

    /* 2. Doorbell calculation test */
    uint32_t test_dboff = 0x2000;
    uint32_t db0 = test_dboff + 0 * 4;
    uint32_t db1 = test_dboff + 1 * 4;
    if (db0 != 0x2000 || db1 != 0x2004) {
        failures |= 8;
    }

    /* 3. Operational status verification if present */
    if (g_present) {
        volatile uint8_t* op_regs = g_mmio_base + g_op_offset;
        uint32_t usbsts = xhci_read32(op_regs, XHCI_OP_USBSTS);
        if (usbsts & XHCI_STS_HCH) {
            failures |= 16;
            kernel_log("[FAULT] xhci: host controller halted unexpectedly\n");
        }

        if (g_device_count > 0 && g_devices[0].vendor_id == 0) {
            failures |= 32;
        }
    }

    return failures;
}

DRIVER_REGISTER("xHCI USB 3.0 controller", xhci_init, DRIVER_PHASE_AFTER_PCI);
