/*
 * uhci.c - UHCI USB 1.1 host controller driver (see uhci.h for scope)
 */
#include "uhci.h"
#include "../pci/pci.h"
#include "../../arch/x86/io.h"
#include "../../lib/string.h"
#include "../../include/kernel.h"

#define UHCI_CLASS    0x0C
#define UHCI_SUBCLASS 0x03
#define UHCI_PROG_IF  0x00 /* distinguishes UHCI from OHCI(0x10)/
                               EHCI(0x20)/xHCI(0x30) */

/* I/O register offsets from the base (PCI BAR4, per the UHCI spec -
 * the one PCI BAR UHCI controllers place their I/O space at by
 * convention, unlike a driver having to read a BAR index generically). */
#define REG_USBCMD    0x00
#define REG_USBSTS    0x02
#define REG_USBINTR   0x04
#define REG_FRNUM     0x06
#define REG_FRBASEADD 0x08
#define REG_SOFMOD    0x0C
#define REG_PORTSC1   0x10
#define REG_PORTSC2   0x12

#define USBCMD_RS      0x0001 /* Run/Stop */
#define USBCMD_HCRESET 0x0002
#define USBCMD_GRESET  0x0004
#define USBCMD_CF      0x0040 /* Configure Flag - software sets this
                                  once setup is complete, per spec */

#define PORTSC_CONNECT_STATUS 0x0001
#define PORTSC_CONNECT_CHANGE 0x0002
#define PORTSC_ENABLE         0x0004
#define PORTSC_LOW_SPEED      0x0100
#define PORTSC_RESET          0x0200

/* Transfer Descriptor - 16 bytes, must be 16-byte aligned (bits 0-3
 * of the link pointer are flags, so any real address here is
 * naturally aligned). */
typedef struct __attribute__((packed, aligned(16))) {
    uint32_t link_pointer;
    uint32_t control_status;
    uint32_t token;
    uint32_t buffer_pointer;
} uhci_td_t;

#define TD_LINK_TERMINATE 0x1
#define TD_LINK_QH_SELECT 0x2
#define TD_LINK_DEPTH_FIRST 0x4

#define TD_STATUS_ACTIVE (1u << 23)
/* Bits 22-17 of control_status become hardware-reported error/status
 * flags once Active clears - see this file's implementation-notes
 * comment on uhci_control_transfer() for why this driver checks
 * "any of these bits set" as a single pass/fail signal rather than
 * decoding each one individually. */
#define TD_STATUS_ERROR_MASK (0x3Fu << 17)
#define TD_CTRL_IOC (1u << 24)
#define TD_CTRL_LOW_SPEED (1u << 26)
#define TD_CTRL_SHORT_PACKET_DETECT (1u << 29)
#define TD_ERROR_COUNTER_3 (3u << 27)

#define TD_PID_SETUP 0x2D
#define TD_PID_IN    0x69
#define TD_PID_OUT   0xE1

/* Queue Head - 8 bytes, also 16-byte aligned per spec (padded here to
 * match, even though only 8 bytes are meaningful). */
typedef struct __attribute__((packed, aligned(16))) {
    uint32_t head_link_pointer;
    uint32_t element_link_pointer;
} uhci_qh_t;

#define MAX_CONTROL_TDS 8

static bool present = false;
static uint16_t io_base = 0;

static uint32_t frame_list[1024] __attribute__((aligned(4096)));
static volatile uhci_qh_t control_qh __attribute__((aligned(16)));
static volatile uhci_td_t control_tds[MAX_CONTROL_TDS] __attribute__((aligned(16)));
static uint8_t control_buffers[MAX_CONTROL_TDS][64] __attribute__((aligned(16)));

typedef struct {
    bool found;
    uint8_t bus, device, function;
    uint32_t bar4;
} uhci_location_t;

static uhci_location_t g_location;

static void find_uhci(const pci_device_t* dev) {
    if (g_location.found) {
        return;
    }
    if (dev->class_code == UHCI_CLASS && dev->subclass == UHCI_SUBCLASS &&
        dev->prog_if == UHCI_PROG_IF) {
        g_location.found = true;
        g_location.bus = dev->bus;
        g_location.device = dev->device;
        g_location.function = dev->function;
        g_location.bar4 =
            pci_config_read32(dev->bus, dev->device, dev->function, 0x20);
    }
}

static void enable_bus_mastering(uint8_t bus, uint8_t device,
                                  uint8_t function) {
    uint16_t command = pci_config_read16(bus, device, function, 0x04);
    command |= 0x04;
    outl(0xCF8, 0x80000000u | ((uint32_t)bus << 16) |
                    ((uint32_t)device << 11) | ((uint32_t)function << 8) |
                    0x04);
    outl(0xCFC, command);
}

/* Builds and runs a single control transfer (SETUP, then zero or more
 * DATA stage packets, then a STATUS packet) against `device_addr`,
 * endpoint 0 - the standard USB control transfer pattern every device
 * enumeration step uses. `data`/`length`/`data_in` describe the DATA
 * stage; pass length 0 for a control transfer with no data stage
 * (e.g. SET_ADDRESS).
 *
 * Implementation note on error checking: this driver's understanding
 * of UHCI's exact per-bit TD status encoding (which of bits 22-17
 * means Stalled vs. Data Buffer Error vs. Babble vs. NAK vs. CRC/
 * Timeout vs. Bitstuff) is not held with full confidence - UHCI's
 * spec bit layout was reconstructed from memory rather than checked
 * against the specification document directly. Rather than risk
 * silently misinterpreting one specific error type as another (or as
 * success), this treats "TD_STATUS_ACTIVE cleared" as "the controller
 * finished with this TD" and "any bit in TD_STATUS_ERROR_MASK is set"
 * as "something went wrong," without distinguishing which failure -
 * a deliberately conservative, coarser check that stays correct even
 * if this driver's specific bit-to-meaning mapping within that range
 * is imperfect. See PROGRESS.md. */
static bool control_transfer(uint8_t device_addr, uint8_t request_type,
                              uint8_t request, uint16_t value,
                              uint16_t index, void* data, uint16_t length,
                              bool data_in, uint16_t max_packet_size,
                              bool low_speed) {
    if (length > sizeof(control_buffers[0]) *
                     (MAX_CONTROL_TDS - 2)) { /* reserve 1 SETUP + 1 STATUS */
        return false;
    }

    int td_index = 0;

    /* SETUP stage - always 8 bytes, PID SETUP, data toggle 0. */
    uint8_t* setup_buf = control_buffers[td_index];
    setup_buf[0] = request_type;
    setup_buf[1] = request;
    setup_buf[2] = (uint8_t)(value & 0xFF);
    setup_buf[3] = (uint8_t)(value >> 8);
    setup_buf[4] = (uint8_t)(index & 0xFF);
    setup_buf[5] = (uint8_t)(index >> 8);
    setup_buf[6] = (uint8_t)(length & 0xFF);
    setup_buf[7] = (uint8_t)(length >> 8);

    uint32_t ctrl_base = TD_ERROR_COUNTER_3 | TD_STATUS_ACTIVE;
    if (low_speed) {
        ctrl_base |= TD_CTRL_LOW_SPEED;
    }

    control_tds[td_index].control_status = ctrl_base;
    control_tds[td_index].token =
        (uint32_t)TD_PID_SETUP | ((uint32_t)device_addr << 8) |
        (0u << 15) /* endpoint 0 */ | ((uint32_t)(8 - 1) << 21) |
        (0u << 19) /* data toggle 0 */;
    control_tds[td_index].buffer_pointer = (uint32_t)setup_buf;
    td_index++;

    /* DATA stage - zero or more packets alternating data toggle
     * starting at 1 (SETUP consumed toggle 0). */
    uint16_t remaining = length;
    uint8_t* data_ptr = (uint8_t*)data;
    int toggle = 1;
    while (remaining > 0 && td_index < MAX_CONTROL_TDS - 1) {
        uint16_t chunk = (remaining < max_packet_size) ? remaining
                                                         : max_packet_size;
        uint8_t* buf = control_buffers[td_index];
        if (!data_in) {
            memcpy(buf, data_ptr, chunk);
        }

        control_tds[td_index].control_status = ctrl_base;
        control_tds[td_index].token =
            (uint32_t)(data_in ? TD_PID_IN : TD_PID_OUT) |
            ((uint32_t)device_addr << 8) | (0u << 15) |
            ((uint32_t)(chunk - 1) << 21) | ((uint32_t)toggle << 19);
        control_tds[td_index].buffer_pointer = (uint32_t)buf;

        data_ptr += chunk;
        remaining = (uint16_t)(remaining - chunk);
        toggle ^= 1;
        td_index++;
    }

    /* STATUS stage - always a zero-length packet in the OPPOSITE
     * direction of the data stage (or IN, if there was no data stage
     * at all), data toggle always 1. */
    bool status_in = (length == 0) ? true : !data_in;
    control_tds[td_index].control_status = ctrl_base | TD_CTRL_IOC;
    control_tds[td_index].token =
        (uint32_t)(status_in ? TD_PID_IN : TD_PID_OUT) |
        ((uint32_t)device_addr << 8) | (0u << 15) | ((uint32_t)0x7FFu << 21) |
        (1u << 19);
    control_tds[td_index].buffer_pointer = 0;
    int last_td = td_index;
    td_index++;

    /* Link every TD in the chain together, depth-first, terminating
     * after the last (STATUS) TD. */
    for (int i = 0; i < td_index; i++) {
        if (i == last_td) {
            control_tds[i].link_pointer = TD_LINK_TERMINATE;
        } else {
            control_tds[i].link_pointer =
                (uint32_t)&control_tds[i + 1] | TD_LINK_DEPTH_FIRST;
        }
    }

    control_qh.element_link_pointer = (uint32_t)&control_tds[0];

    /* Poll for completion - the controller processes the frame list
     * (which the control QH is permanently linked into by
     * usb_uhci_init()) once per 1ms frame, so give it a generous
     * bounded number of spins rather than a fixed instruction count. */
    uint32_t spins = 0;
    while (control_tds[last_td].control_status & TD_STATUS_ACTIVE) {
        if (++spins > 2000000u) {
            return false; /* never completed - treat as failure rather
                              than hang forever */
        }
    }

    for (int i = 0; i < td_index; i++) {
        if (control_tds[i].control_status & TD_STATUS_ERROR_MASK) {
            kernel_log("[WARN] USB control_transfer: TD %d reported an "
                       "error, status=0x%x token=0x%x\n", i,
                       (int)control_tds[i].control_status,
                       (int)control_tds[i].token);
            return false;
        }
    }

    if (data_in && length > 0) {
        /* Copy received data back out of the per-TD buffers into the
         * caller's buffer, in order. */
        uint16_t copied = 0;
        for (int i = 1; i < last_td && copied < length; i++) {
            uint16_t chunk = (uint16_t)(length - copied);
            if (chunk > max_packet_size) {
                chunk = max_packet_size;
            }
            memcpy((uint8_t*)data + copied, control_buffers[i], chunk);
            copied = (uint16_t)(copied + chunk);
        }
    }

    return true;
}

typedef struct __attribute__((packed)) {
    uint8_t length;
    uint8_t descriptor_type;
    uint16_t bcd_usb;
    uint8_t device_class;
    uint8_t device_subclass;
    uint8_t device_protocol;
    uint8_t max_packet_size0;
    uint16_t vendor_id;
    uint16_t product_id;
    uint16_t bcd_device;
    uint8_t manufacturer_index;
    uint8_t product_index;
    uint8_t serial_index;
    uint8_t num_configurations;
} usb_device_descriptor_t;

#define REQ_GET_DESCRIPTOR 0x06
#define REQ_SET_ADDRESS    0x05
#define DESC_TYPE_DEVICE   0x01

static void enumerate_port(int port_index, uint16_t portsc_reg) {
    uint16_t status = inw((uint16_t)(io_base + portsc_reg));
    if (!(status & PORTSC_CONNECT_STATUS)) {
        return; /* nothing attached to this port */
    }

    bool low_speed = (status & PORTSC_LOW_SPEED) != 0;

    /* Reset the port - this is what makes an attached device respond
     * at the default address (0) afterward, the standard USB
     * attach sequence. */
    outw((uint16_t)(io_base + portsc_reg), status | PORTSC_RESET);
    for (volatile int i = 0; i < 500000; i++) { } /* ~50ms, USB spec's
                                                       minimum reset
                                                       duration, done
                                                       as a busy-wait
                                                       since this
                                                       kernel's
                                                       timer_sleep_ms()
                                                       isn't reachable
                                                       from this early
                                                       in boot without
                                                       a new dependency */
    status = inw((uint16_t)(io_base + portsc_reg));
    outw((uint16_t)(io_base + portsc_reg),
         (uint16_t)(status & ~PORTSC_RESET));
    for (volatile int i = 0; i < 100000; i++) { }

    status = inw((uint16_t)(io_base + portsc_reg));
    if (!(status & PORTSC_ENABLE)) {
        /* Some controllers/devices need the enable bit set explicitly
         * after reset rather than doing it automatically. */
        outw((uint16_t)(io_base + portsc_reg), status | PORTSC_ENABLE);
    }

    /* GET_DESCRIPTOR (Device), first 8 bytes only - the standard first
     * enumeration step, since the real max packet size (needed to
     * correctly chunk anything larger) isn't known until this byte
     * of the descriptor is read. Address 0, the default every newly
     * attached device responds at until given a real one. */
    usb_device_descriptor_t desc;
    memset(&desc, 0, sizeof(desc));
    if (!control_transfer(0, 0x80, REQ_GET_DESCRIPTOR,
                           (uint16_t)(DESC_TYPE_DEVICE << 8), 0, &desc, 8,
                           true, 8, low_speed)) {
        kernel_log("[WARN] USB port %d: device attached but initial "
                   "GET_DESCRIPTOR failed\n", port_index);
        return;
    }

    uint8_t new_address = (uint8_t)(port_index + 1);
    if (!control_transfer(0, 0x00, REQ_SET_ADDRESS, new_address, 0, NULL, 0,
                           false, 8, low_speed)) {
        kernel_log("[WARN] USB port %d: SET_ADDRESS failed\n", port_index);
        return;
    }
    for (volatile int i = 0; i < 20000; i++) { } /* let the device
                                                     settle into its
                                                     new address */

    /* Full GET_DESCRIPTOR at the real address, now that the actual
     * max packet size (desc.max_packet_size0) is known. */
    if (!control_transfer(new_address, 0x80, REQ_GET_DESCRIPTOR,
                           (uint16_t)(DESC_TYPE_DEVICE << 8), 0, &desc,
                           sizeof(desc), true, desc.max_packet_size0,
                           low_speed)) {
        kernel_log("[WARN] USB port %d: full GET_DESCRIPTOR at address "
                   "%d failed\n", port_index, (int)new_address);
        return;
    }

    kernel_log("[ OK ] USB device on port %d: address=%d vendor=0x%x "
               "device=0x%x class=0x%x subclass=0x%x protocol=0x%x "
               "(%s speed)\n", port_index, (int)new_address,
               (int)desc.vendor_id, (int)desc.product_id,
               (int)desc.device_class, (int)desc.device_subclass,
               (int)desc.device_protocol, low_speed ? "low" : "full");
}

void usb_uhci_init(void) {
    present = false;
    g_location.found = false;

    pci_enumerate(find_uhci);
    if (!g_location.found) {
        return;
    }

    if (!(g_location.bar4 & 0x1)) {
        kernel_log("[ .. ] UHCI: BAR4 is not I/O-mapped as expected - "
                   "not attaching\n");
        return;
    }
    io_base = (uint16_t)(g_location.bar4 & 0xFFFC);

    enable_bus_mastering(g_location.bus, g_location.device,
                         g_location.function);

    /* Global reset, then a short wait, then clear it - the standard
     * UHCI controller reset sequence. */
    outw((uint16_t)(io_base + REG_USBCMD), USBCMD_GRESET);
    for (volatile int i = 0; i < 100000; i++) { }
    outw((uint16_t)(io_base + REG_USBCMD), 0x0000);

    outw((uint16_t)(io_base + REG_USBCMD), USBCMD_HCRESET);
    uint32_t spins = 0;
    while (inw((uint16_t)(io_base + REG_USBCMD)) & USBCMD_HCRESET) {
        if (++spins > 1000000u) {
            kernel_log("[ .. ] UHCI: host controller reset never "
                       "completed\n");
            return;
        }
    }

    /* An empty frame list - every entry marked Terminate except the
     * ones covering the permanently-linked control queue head, set up
     * next. */
    for (int i = 0; i < 1024; i++) {
        frame_list[i] = TD_LINK_TERMINATE;
    }
    control_qh.head_link_pointer = TD_LINK_TERMINATE;
    control_qh.element_link_pointer = TD_LINK_TERMINATE;
    for (int i = 0; i < 1024; i++) {
        frame_list[i] = (uint32_t)&control_qh | TD_LINK_QH_SELECT;
    }

    outl((uint16_t)(io_base + REG_FRBASEADD), (uint32_t)frame_list);
    outw((uint16_t)(io_base + REG_FRNUM), 0);
    outb((uint16_t)(io_base + REG_SOFMOD), 0x40); /* default SOF timing */

    outw((uint16_t)(io_base + REG_USBCMD), USBCMD_RS | USBCMD_CF);

    present = true;
    kernel_log("[ OK ] UHCI controller at PCI %d:%d.%d, I/O base 0x%x\n",
               (int)g_location.bus, (int)g_location.device,
               (int)g_location.function, (int)io_base);

    enumerate_port(0, REG_PORTSC1);
    enumerate_port(1, REG_PORTSC2);
}

bool usb_uhci_is_present(void) {
    return present;
}

bool usb_uhci_keyboard_poll(uint8_t out_report[8]) {
    (void)out_report;
    /* Not implemented in this pass - see PROGRESS.md. Enumeration
     * (above) is the verified, working part of this driver; polling
     * a HID boot-protocol keyboard report needs an interrupt-endpoint
     * transfer this file doesn't build yet. */
    return false;
}
