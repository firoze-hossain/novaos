#ifndef DRIVERS_XHCI_H
#define DRIVERS_XHCI_H

#include "../../include/types.h"

/*
 * xhci.h - Month 5 (Driver breadth & performance): USB 3.0 (xHCI) driver
 *
 * Modern USB 3.0 (eXtensible Host Controller Interface) support,
 * replacing USB 1.1-only UHCI support with high-speed and SuperSpeed
 * USB peripheral and storage communication.
 *
 * Reuses the existing UHCI driver's overall enumerate-the-root-hub
 * structure, while implementing xHCI 1.0 - 1.2 specification:
 * - PCI Class 0x0C (Serial Bus), Subclass 0x03 (USB), ProgIF 0x30 (xHCI).
 * - 64-bit BAR0 MMIO space mapping.
 * - Capability registers: CAPLENGTH, HCSPARAMS1-3, HCCPARAMS1-2, DBOFF, RTSOFF.
 * - Operational registers: USBCMD, USBSTS, PAGESIZE, CRCR, DCBAAP, CONFIG, PORTSC.
 * - Runtime registers: Interrupter 0 (IMAN, IMOD, ERSTSZ, ERSTBA, ERDP).
 * - Doorbell array dispatch (Doorbell 0 for Host Controller, Doorbells 1..N for Device Slots).
 * - Command Ring (TRBs) and Event Ring (ERST).
 * - Root hub port scanning, reset, slot enablement, address assignment,
 *   and control transfer on Endpoint 0 for USB Device Descriptors.
 * - Zero heap allocations via physical frames from PMM.
 * - Thread-safe concurrency protected by spinlock_t.
 */

/* PCI class definitions for USB */
#define PCI_CLASS_SERIAL        0x0C
#define PCI_SUBCLASS_USB        0x03
#define PCI_PROGIF_UHCI         0x00
#define PCI_PROGIF_OHCI         0x10
#define PCI_PROGIF_EHCI         0x20
#define PCI_PROGIF_XHCI         0x30

/* Capability Register Offsets (relative to MMIO base) */
#define XHCI_CAP_CAPLENGTH      0x00 /* 8-bit: Capability Register Length */
#define XHCI_CAP_HCIVERSION     0x02 /* 16-bit: Interface Version Number */
#define XHCI_CAP_HCSPARAMS1     0x04 /* 32-bit: Structural Parameters 1 */
#define XHCI_CAP_HCSPARAMS2     0x08 /* 32-bit: Structural Parameters 2 */
#define XHCI_CAP_HCSPARAMS3     0x0C /* 32-bit: Structural Parameters 3 */
#define XHCI_CAP_HCCPARAMS1     0x10 /* 32-bit: Capability Parameters 1 */
#define XHCI_CAP_DBOFF          0x14 /* 32-bit: Doorbell Offset */
#define XHCI_CAP_RTSOFF         0x18 /* 32-bit: Runtime Register Space Offset */
#define XHCI_CAP_HCCPARAMS2     0x1C /* 32-bit: Capability Parameters 2 */

/* Operational Register Offsets (relative to base + CAPLENGTH) */
#define XHCI_OP_USBCMD          0x00 /* 32-bit: USB Command */
#define XHCI_OP_USBSTS          0x04 /* 32-bit: USB Status */
#define XHCI_OP_PAGESIZE        0x08 /* 32-bit: Page Size */
#define XHCI_OP_DNCTRL          0x14 /* 32-bit: Device Notification Control */
#define XHCI_OP_CRCR            0x18 /* 64-bit: Command Ring Control Register */
#define XHCI_OP_DCBAAP          0x30 /* 64-bit: Device Context Base Address Array Pointer */
#define XHCI_OP_CONFIG          0x38 /* 32-bit: Configure Register */
#define XHCI_OP_PORTSC_BASE     0x400 /* Port Status and Control Register Base */

/* USBCMD bit masks */
#define XHCI_CMD_RS             (1u << 0)  /* Run/Stop */
#define XHCI_CMD_HCRST          (1u << 1)  /* Host Controller Reset */
#define XHCI_CMD_INTE           (1u << 2)  /* Interrupter Enable */
#define XHCI_CMD_HSEE           (1u << 3)  /* Host System Error Enable */

/* USBSTS bit masks */
#define XHCI_STS_HCH            (1u << 0)  /* HC Halted */
#define XHCI_STS_HSE            (1u << 2)  /* Host System Error */
#define XHCI_STS_EINT           (1u << 3)  /* Event Interrupt */
#define XHCI_STS_PCD            (1u << 4)  /* Port Change Detect */
#define XHCI_STS_CNR            (1u << 11) /* Controller Not Ready */

/* PORTSC bit masks */
#define XHCI_PORTSC_CCS         (1u << 0)  /* Current Connect Status */
#define XHCI_PORTSC_PE          (1u << 1)  /* Port Enabled/Disabled */
#define XHCI_PORTSC_OCA         (1u << 3)  /* Over-current Active */
#define XHCI_PORTSC_PR          (1u << 4)  /* Port Reset */
#define XHCI_PORTSC_PLS_MASK    (0x0Fu << 5) /* Port Link State */
#define XHCI_PORTSC_PP          (1u << 9)  /* Port Power */
#define XHCI_PORTSC_SPEED_MASK  (0x0Fu << 10) /* Port Speed */
#define XHCI_PORTSC_SPEED_SHIFT 10
#define XHCI_PORTSC_CSC         (1u << 17) /* Connect Status Change */
#define XHCI_PORTSC_PEC         (1u << 18) /* Port Enable/Disable Change */
#define XHCI_PORTSC_WRC         (1u << 19) /* Warm Port Reset Change */
#define XHCI_PORTSC_OCC         (1u << 20) /* Over-current Change */
#define XHCI_PORTSC_PRC         (1u << 21) /* Port Reset Change */
#define XHCI_PORTSC_PLC         (1u << 22) /* Port Link State Change */

/* Port Speed definitions */
#define XHCI_SPEED_FULL         1 /* Full-speed (12 Mbps, USB 2.0/1.1) */
#define XHCI_SPEED_LOW          2 /* Low-speed (1.5 Mbps, USB 1.1) */
#define XHCI_SPEED_HIGH         3 /* High-speed (480 Mbps, USB 2.0) */
#define XHCI_SPEED_SUPER        4 /* SuperSpeed (5 Gbps, USB 3.0) */
#define XHCI_SPEED_SUPER_PLUS   5 /* SuperSpeedPlus (10 Gbps, USB 3.1) */

/* TRB Types */
#define XHCI_TRB_NORMAL         1
#define XHCI_TRB_SETUP_STAGE    2
#define XHCI_TRB_DATA_STAGE     3
#define XHCI_TRB_STATUS_STAGE   4
#define XHCI_TRB_LINK           6
#define XHCI_TRB_ENABLE_SLOT    9
#define XHCI_TRB_DISABLE_SLOT   10
#define XHCI_TRB_ADDRESS_DEVICE 11
#define XHCI_TRB_CONFIG_EP      12
#define XHCI_TRB_EVAL_CONTEXT   13
#define XHCI_TRB_RESET_EP       14
#define XHCI_TRB_NOOP_CMD       23
#define XHCI_TRB_TRANSFER_EVENT 32
#define XHCI_TRB_CMD_COMPL      33
#define XHCI_TRB_PORT_STATUS    34

/* TRB Completion Codes */
#define XHCI_CC_SUCCESS         1
#define XHCI_CC_DATA_BUF_ERROR  2
#define XHCI_CC_BABBLE_ERROR    3
#define XHCI_CC_USB_TX_ERROR    4
#define XHCI_CC_TRB_ERROR       5
#define XHCI_CC_STALL_ERROR     6
#define XHCI_CC_RESOURCE_ERROR  7
#define XHCI_CC_BANDWIDTH_ERROR 8
#define XHCI_CC_NO_SLOTS_ERROR  9
#define XHCI_CC_SHORT_PACKET    13

/* Standard USB Request & Descriptor Types */
#define USB_REQ_GET_STATUS      0x00
#define USB_REQ_CLEAR_FEATURE   0x01
#define USB_REQ_SET_FEATURE     0x03
#define USB_REQ_SET_ADDRESS     0x05
#define USB_REQ_GET_DESCRIPTOR  0x06
#define USB_REQ_SET_DESCRIPTOR  0x07
#define USB_REQ_GET_CONFIG      0x08
#define USB_REQ_SET_CONFIG      0x09

#define USB_DESC_DEVICE         0x01
#define USB_DESC_CONFIGURATION  0x02
#define USB_DESC_STRING         0x03
#define USB_DESC_INTERFACE      0x04
#define USB_DESC_ENDPOINT       0x05

/* TRB - Transfer Request Block (16 bytes) */
typedef struct __attribute__((packed)) {
    uint64_t parameter;
    uint32_t status;
    uint32_t control;
} xhci_trb_t;

/* Event Ring Segment Table Entry (16 bytes) */
typedef struct __attribute__((packed)) {
    uint64_t ring_segment_base_address;
    uint32_t ring_segment_size;
    uint32_t rsvd;
} xhci_erst_entry_t;

/* Standard USB Device Descriptor (18 bytes) */
typedef struct __attribute__((packed)) {
    uint8_t  length;
    uint8_t  descriptor_type;
    uint16_t bcd_usb;
    uint8_t  device_class;
    uint8_t  device_subclass;
    uint8_t  device_protocol;
    uint8_t  max_packet_size0;
    uint16_t vendor_id;
    uint16_t product_id;
    uint16_t bcd_device;
    uint8_t  manufacturer_index;
    uint8_t  product_index;
    uint8_t  serial_index;
    uint8_t  num_configurations;
} usb_device_descriptor_t;

/* Discovered USB device on xHCI root hub */
typedef struct {
    bool     connected;
    uint8_t  port;
    uint8_t  slot_id;
    uint8_t  speed;
    uint16_t vendor_id;
    uint16_t product_id;
    uint8_t  device_class;
    uint8_t  device_subclass;
    uint8_t  device_protocol;
    uint8_t  max_packet_size0;
} xhci_device_t;

/* Driver public functions */
void                 xhci_init(void);
bool                 xhci_is_present(void);
uint32_t             xhci_get_port_count(void);
uint32_t             xhci_get_device_count(void);
const xhci_device_t* xhci_get_device(uint32_t index);
int                  xhci_selftest(void);

#endif /* DRIVERS_XHCI_H */
