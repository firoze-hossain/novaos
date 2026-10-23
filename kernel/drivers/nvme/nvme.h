#ifndef DRIVERS_NVME_H
#define DRIVERS_NVME_H

#include "../../include/types.h"

/*
 * nvme.h - Month 5 (Driver breadth & performance): NVMe driver
 *
 * Modern SSD storage support following the virtio-blk driver's
 * "expose a block device to the VFS" shape.
 *
 * Compliant with NVMe Base Specification (1.0 - 1.4):
 * - PCI Class 0x01 (Mass Storage), Subclass 0x08 (Non-Volatile Memory),
 *   ProgIF 0x02 (NVM Express).
 * - 64-bit BAR0 MMIO register space mapping.
 * - Controller reset, capability inspection, and queue configuration.
 * - Admin Submission & Completion Queues for configuration.
 * - I/O Submission & Completion Queues for sector transfers.
 * - Zero-allocation DMA staging buffers in kernel identity-mapped space.
 * - Standard 512-byte / 4096-byte LBA block I/O.
 */

/* PCI class definitions */
#define PCI_CLASS_STORAGE       0x01
#define PCI_SUBCLASS_NVME       0x08
#define PCI_PROGIF_NVME         0x02

/* Controller Register Offsets (relative to BAR0) */
#define NVME_REG_CAP            0x0000 /* 64-bit: Controller Capabilities */
#define NVME_REG_VS             0x0008 /* 32-bit: Version */
#define NVME_REG_INTMS          0x000C /* 32-bit: Interrupt Mask Set */
#define NVME_REG_INTMC          0x0010 /* 32-bit: Interrupt Mask Clear */
#define NVME_REG_CC             0x0014 /* 32-bit: Controller Configuration */
#define NVME_REG_CSTS           0x001C /* 32-bit: Controller Status */
#define NVME_REG_NSSR           0x0020 /* 32-bit: NVM Subsystem Reset (optional) */
#define NVME_REG_AQA            0x0024 /* 32-bit: Admin Queue Attributes */
#define NVME_REG_ASQ            0x0028 /* 64-bit: Admin Submission Queue Base */
#define NVME_REG_ACQ            0x0030 /* 64-bit: Admin Completion Queue Base */

/* Controller Configuration (CC) bit masks */
#define NVME_CC_EN              (1u << 0)
#define NVME_CC_CSS_NVM         (0u << 4)
#define NVME_CC_MPS_4K          (0u << 7)   /* 2 ^ (12 + 0) = 4096 bytes */
#define NVME_CC_AMS_RR          (0u << 11)  /* Round robin */
#define NVME_CC_SHN_NONE        (0u << 14)
#define NVME_CC_IOSQES_64       (6u << 16)  /* 2 ^ 6 = 64 bytes */
#define NVME_CC_IOCQES_16       (4u << 20)  /* 2 ^ 4 = 16 bytes */

/* Controller Status (CSTS) bit masks */
#define NVME_CSTS_RDY           (1u << 0)
#define NVME_CSTS_CFS           (1u << 1)
#define NVME_CSTS_SHST_MASK     (3u << 2)

/* Admin Command Opcodes */
#define NVME_ADMIN_OPC_DELETE_IO_SQ   0x00
#define NVME_ADMIN_OPC_CREATE_IO_SQ   0x01
#define NVME_ADMIN_OPC_GET_LOG_PAGE   0x02
#define NVME_ADMIN_OPC_DELETE_IO_CQ   0x04
#define NVME_ADMIN_OPC_CREATE_IO_CQ   0x05
#define NVME_ADMIN_OPC_IDENTIFY       0x06
#define NVME_ADMIN_OPC_ABORT          0x08
#define NVME_ADMIN_OPC_SET_FEATURES   0x09
#define NVME_ADMIN_OPC_GET_FEATURES   0x0A

/* NVM Command Opcodes (I/O) */
#define NVME_NVM_OPC_FLUSH            0x00
#define NVME_NVM_OPC_WRITE            0x01
#define NVME_NVM_OPC_READ             0x02

/* Identify Command CNS values */
#define NVME_IDENTIFY_CNS_NS          0x00
#define NVME_IDENTIFY_CNS_CTRL        0x01

/* Submission Queue Entry (SQE) - 64 bytes */
typedef struct __attribute__((packed)) {
    uint8_t  opc;           /* Opcode */
    uint8_t  fuse : 2;      /* Fused Operation */
    uint8_t  rsvd1 : 4;     /* Reserved */
    uint8_t  psdt : 2;      /* PRP or SGL (0 = PRP) */
    uint16_t cid;           /* Command Identifier */
    uint32_t nsid;          /* Namespace Identifier */
    uint64_t rsvd2;         /* Reserved */
    uint64_t mptr;          /* Metadata Pointer */
    uint64_t prp1;          /* PRP Entry 1 */
    uint64_t prp2;          /* PRP Entry 2 */
    uint32_t cdw10;         /* Command Dword 10 */
    uint32_t cdw11;         /* Command Dword 11 */
    uint32_t cdw12;         /* Command Dword 12 */
    uint32_t cdw13;         /* Command Dword 13 */
    uint32_t cdw14;         /* Command Dword 14 */
    uint32_t cdw15;         /* Command Dword 15 */
} nvme_sqe_t;

/* Completion Queue Entry (CQE) - 16 bytes */
typedef struct __attribute__((packed)) {
    uint32_t dw0;           /* Command-specific result */
    uint32_t dw1;           /* Reserved */
    uint16_t sq_head;       /* SQ Head Pointer */
    uint16_t sq_id;         /* SQ Identifier */
    uint16_t cid;           /* Command Identifier */
    uint16_t status;        /* Phase Tag (bit 0) + Status Field (bits 15:1) */
} nvme_cqe_t;

/* Identify Controller Structure (4096 bytes) */
typedef struct __attribute__((packed)) {
    uint16_t vid;           /* PCI Vendor ID */
    uint16_t ssvid;         /* PCI Subsystem Vendor ID */
    char     sn[20];        /* Serial Number */
    char     mn[40];        /* Model Number */
    char     fr[8];         /* Firmware Revision */
    uint8_t  rab;           /* Recommended Arbitration Burst */
    uint8_t  ieee[3];       /* IEEE OUI Identifier */
    uint8_t  cmic;          /* Controller Multi-Path I/O and Namespace Sharing Capabilities */
    uint8_t  mdts;          /* Maximum Data Transfer Size */
    uint16_t cntlid;        /* Controller ID */
    uint32_t ver;           /* Version */
    uint32_t rtd3r;         /* RTD3 Resume Latency */
    uint32_t rtd3e;         /* RTD3 Entry Latency */
    uint32_t oaes;          /* Optional Asynchronous Events Supported */
    uint8_t  rsvd96[160];
    uint16_t oacs;          /* Optional Admin Command Support */
    uint8_t  acl;           /* Abort Command Limit */
    uint8_t  aerl;          /* Asynchronous Event Request Limit */
    uint8_t  frmw;          /* Firmware Updates */
    uint8_t  lpa;           /* Log Page Attributes */
    uint8_t  elpe;          /* Error Log Page Entries */
    uint8_t  npss;          /* Number of Power States Support */
    uint8_t  avscc;         /* Admin Vendor Specific Command Configuration */
    uint8_t  apsta;         /* Autonomous Power State Transition Attributes */
    uint8_t  rsvd266[246];
    uint32_t nn;            /* Number of Namespaces */
    uint8_t  rsvd516[3580]; /* Pad to 4096 bytes */
} nvme_id_ctrl_t;

/* LBA Format Data Structure (4 bytes) */
typedef struct __attribute__((packed)) {
    uint16_t ms;            /* Metadata Size */
    uint8_t  lbads;         /* LBA Data Size (2^n bytes) */
    uint8_t  rp : 2;        /* Relative Performance */
    uint8_t  rsvd : 6;
} nvme_lbaf_t;

/* Identify Namespace Structure (4096 bytes) */
typedef struct __attribute__((packed)) {
    uint64_t nsze;          /* Namespace Size (total blocks) */
    uint64_t ncap;          /* Namespace Capacity */
    uint64_t nuse;          /* Namespace Utilization */
    uint8_t  nsfeat;        /* Namespace Features */
    uint8_t  nlbaf;         /* Number of LBA Formats */
    uint8_t  flbas;         /* Formatted LBA Size */
    uint8_t  mc;            /* Metadata Capabilities */
    uint8_t  dpc;           /* End-to-end Data Protection Capabilities */
    uint8_t  dps;           /* End-to-end Data Protection Type Settings */
    uint8_t  nmic;          /* Namespace Multi-path I/O and Namespace Sharing Capabilities */
    uint8_t  rescap;        /* Reservation Capabilities */
    uint8_t  fpi;           /* Format Progress Indicator */
    uint8_t  rsvd33;
    uint16_t nawun;         /* Namespace Atomic Write Unit Normal */
    uint16_t nawupf;        /* Namespace Atomic Write Unit Power Fail */
    uint16_t nacwu;         /* Namespace Atomic Compare & Write Unit */
    uint16_t nabsn;         /* Namespace Atomic Boundary Size Normal */
    uint16_t nabo;          /* Namespace Atomic Boundary Offset */
    uint16_t nabspf;        /* Namespace Atomic Boundary Size Power Fail */
    uint8_t  rsvd46[82];
    nvme_lbaf_t lbaf[16];   /* LBA Format list */
    uint8_t  rsvd192[3904]; /* Pad to 4096 bytes */
} nvme_id_ns_t;

/* Driver API */
void     nvme_init(void);
bool     nvme_is_present(void);
uint64_t nvme_get_sector_count(void);
uint32_t nvme_get_sector_size(void);

/* Single-sector transfer (matches virtio-blk interface) */
bool     nvme_read_sector(uint64_t sector, void* buf);
bool     nvme_write_sector(uint64_t sector, const void* buf);

/* Multi-sector transfer (matches blockdev interface) */
bool     nvme_read_sectors(uint64_t start_lba, uint8_t sector_count, void* buf);
bool     nvme_write_sectors(uint64_t start_lba, uint8_t sector_count, const void* buf);

/* Driver verification self-test */
int      nvme_selftest(void);

#endif /* DRIVERS_NVME_H */
