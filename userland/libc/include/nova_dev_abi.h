#ifndef _NOVA_DEV_ABI_H
#define _NOVA_DEV_ABI_H

#include <stdint.h>

/* Month 5 / Phase 92: Device Manager & Hardware Query ABI (SYS_DEV_INFO 83) */
#define NOVA_SYS_DEV_INFO 83

/* Operations for SYS_DEV_INFO */
#define NOVA_DEV_OP_GET_SUMMARY 1
#define NOVA_DEV_OP_GET_DRIVERS 2
#define NOVA_DEV_OP_GET_DEVICES 3

/* Limits */
#define NOVA_DEV_MAX_DRIVERS    24
#define NOVA_DEV_MAX_DEVICES    32
#define NOVA_DEV_MAX_NAME       32
#define NOVA_DEV_MAX_DETAIL     64

/* Device Categories */
#define NOVA_DEV_CAT_ALL        0
#define NOVA_DEV_CAT_STORAGE    1
#define NOVA_DEV_CAT_NETWORK    2
#define NOVA_DEV_CAT_USB        3
#define NOVA_DEV_CAT_DISPLAY    4
#define NOVA_DEV_CAT_AUDIO      5
#define NOVA_DEV_CAT_INPUT      6
#define NOVA_DEV_CAT_SYSTEM     7

/* Bus Types */
#define NOVA_BUS_PCI            1
#define NOVA_BUS_ISA            2
#define NOVA_BUS_USB            3
#define NOVA_BUS_VIRTIO         4
#define NOVA_BUS_SYSTEM         5

/* Status Codes */
#define NOVA_DEV_STATUS_ONLINE  1
#define NOVA_DEV_STATUS_STANDBY 2
#define NOVA_DEV_STATUS_OFFLINE 3

/* Driver Registration Record */
typedef struct {
    char     name[NOVA_DEV_MAX_NAME];
    uint32_t phase;         /* DRIVER_PHASE_* */
    uint32_t status;        /* 1 = Initialized & Active */
    uint32_t flags;
} nova_driver_info_t;

/* Driver Table Snapshot */
typedef struct {
    uint32_t           count;
    nova_driver_info_t drivers[NOVA_DEV_MAX_DRIVERS];
} nova_driver_table_t;

/* Hardware Device Record */
typedef struct {
    char     name[NOVA_DEV_MAX_NAME];
    char     driver_name[NOVA_DEV_MAX_NAME];
    char     details[NOVA_DEV_MAX_DETAIL];
    uint32_t category;     /* NOVA_DEV_CAT_* */
    uint32_t bus_type;     /* NOVA_BUS_* */
    uint32_t status;       /* NOVA_DEV_STATUS_* */
    uint16_t vendor_id;    /* PCI or USB vendor ID */
    uint16_t device_id;    /* PCI or USB product/device ID */
    uint8_t  pci_bus;
    uint8_t  pci_dev;
    uint8_t  pci_func;
    uint8_t  irq;
    uint32_t mmio_or_io;   /* Base I/O port or MMIO physical address */
    uint32_t extra_val;    /* e.g. capacity MB or port count or link speed */
} nova_device_info_t;

/* Device Table Snapshot */
typedef struct {
    uint32_t           count;
    nova_device_info_t devices[NOVA_DEV_MAX_DEVICES];
} nova_device_table_t;

/* Device Summary Telemetry */
typedef struct {
    uint32_t total_devices;
    uint32_t total_drivers;
    uint32_t storage_count;
    uint32_t network_count;
    uint32_t usb_count;
    uint32_t display_count;
    uint32_t audio_count;
    uint32_t input_count;
    uint32_t system_count;
    uint32_t nvme_present;
    uint32_t wifi_present;
    uint32_t xhci_present;
} nova_device_summary_t;

#endif /* _NOVA_DEV_ABI_H */
