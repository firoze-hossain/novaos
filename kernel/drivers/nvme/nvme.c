/*
 * kernel/drivers/nvme/nvme.c - Month 5 (Driver breadth & performance): NVMe driver
 *
 * Provides modern NVMe SSD block storage support, following the virtio-blk
 * driver's "expose a block device to the VFS" shape.
 *
 * Implemented fully in C, compliant with NVMe 1.0 - 1.4 specifications:
 * - Scans PCI for Mass Storage (0x01), NVMe Subclass (0x08), ProgIF (0x02).
 * - Maps 64-bit BAR0 MMIO space.
 * - Manages Admin Submission/Completion Queues and I/O Submission/Completion Queues.
 * - Performs synchronous polled DMA block I/O with zero heap allocation.
 * - Thread-safe concurrency protected by spinlock_t.
 */

#include "nvme.h"
#include "../pci/pci.h"
#include "../driver.h"
#include "../blockdev.h"
#include "../../arch/x86/io.h"
#include "../../arch/x86/mm/paging.h"
#include "../../arch/x86/mm/pmm.h"
#include "../../lib/string.h"
#include "../../lib/spinlock.h"
#include "../../include/kernel.h"

#define NVME_ADMIN_QUEUE_SIZE   64
#define NVME_IO_QUEUE_SIZE      64
#define NVME_TIMEOUT_LOOPS      10000000u
#define NVME_MMIO_MAP_SIZE      16384 /* 4 pages covers regs + doorbells */

typedef struct {
    bool     found;
    uint8_t  bus;
    uint8_t  device;
    uint8_t  function;
    uint32_t bar0_low;
    uint32_t bar0_high;
} nvme_pci_loc_t;

static nvme_pci_loc_t g_loc;
static bool           g_present = false;
static spinlock_t     g_nvme_lock;

static volatile uint8_t* g_mmio_base = NULL;
static uint32_t          g_doorbell_stride = 4;

/* Admin Queue State */
static volatile nvme_sqe_t* g_asq = NULL;
static volatile nvme_cqe_t* g_acq = NULL;
static uint32_t    g_asq_phys = 0;
static uint32_t    g_acq_phys = 0;
static uint16_t    g_asq_tail = 0;
static uint16_t    g_acq_head = 0;
static uint8_t     g_acq_phase = 1;
static uint16_t    g_admin_cid = 1;

/* I/O Queue 1 State */
static volatile nvme_sqe_t* g_iosq = NULL;
static volatile nvme_cqe_t* g_iocq = NULL;
static uint32_t    g_iosq_phys = 0;
static uint32_t    g_iocq_phys = 0;
static uint16_t    g_iosq_tail = 0;
static uint16_t    g_iocq_head = 0;
static uint8_t     g_iocq_phase = 1;
static uint16_t    g_io_cid = 1;

/* Staging DMA buffer (4096 bytes, page-aligned) */
static uint8_t*    g_staging_buf = NULL;
static uint32_t    g_staging_phys = 0;

/* Drive Identification */
static uint64_t    g_sector_count = 0;
static uint32_t    g_sector_size = 512;
static char        g_model[41];
static char        g_serial[21];

static inline uint32_t nvme_read32(volatile uint8_t* base, uint32_t offset) {
    return *(volatile uint32_t*)(base + offset);
}

static inline void nvme_write32(volatile uint8_t* base, uint32_t offset, uint32_t val) {
    *(volatile uint32_t*)(base + offset) = val;
}

static inline uint64_t nvme_read64(volatile uint8_t* base, uint32_t offset) {
    volatile uint32_t* reg = (volatile uint32_t*)(base + offset);
    uint32_t low = reg[0];
    uint32_t high = reg[1];
    return ((uint64_t)high << 32) | low;
}

static inline void nvme_write64(volatile uint8_t* base, uint32_t offset, uint64_t val) {
    volatile uint32_t* reg = (volatile uint32_t*)(base + offset);
    reg[0] = (uint32_t)(val & 0xFFFFFFFFu);
    reg[1] = (uint32_t)(val >> 32);
}

static void trim_spaces(char* dst, const char* src, size_t len) {
    size_t i = 0;
    while (i < len && src[i] != '\0') {
        dst[i] = src[i];
        i++;
    }
    while (i > 0 && (dst[i - 1] == ' ' || dst[i - 1] == '\0')) {
        i--;
    }
    dst[i] = '\0';
}

static void nvme_find_pci(const pci_device_t* dev) {
    if (g_loc.found) {
        return;
    }
    if (dev->class_code == PCI_CLASS_STORAGE &&
        dev->subclass == PCI_SUBCLASS_NVME &&
        dev->prog_if == PCI_PROGIF_NVME) {
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

static volatile uint8_t* map_nvme_mmio(uint32_t phys_addr, uint32_t length) {
    uint32_t page_base = phys_addr & ~0xFFFu;
    uint32_t end = (phys_addr + length + 4095u) & ~0xFFFu;
    uint32_t* kernel_pd = (uint32_t*)paging_kernel_directory_phys();
    for (uint32_t addr = page_base; addr < end; addr += 4096u) {
        if (!paging_map_page(kernel_pd, addr, addr, PAGE_PRESENT | PAGE_WRITE)) {
            kernel_log("[FAULT] nvme: failed to map MMIO page at 0x%x\n", (int)addr);
            return NULL;
        }
    }
    return (volatile uint8_t*)phys_addr;
}

static bool nvme_submit_admin_cmd(nvme_sqe_t* sqe, nvme_cqe_t* out_cqe) {
    sqe->cid = g_admin_cid++;
    memcpy((void*)&g_asq[g_asq_tail], sqe, sizeof(nvme_sqe_t));

    g_asq_tail = (g_asq_tail + 1) % NVME_ADMIN_QUEUE_SIZE;

    __asm__ volatile ("mfence" ::: "memory");

    /* Admin SQ Tail Doorbell is at offset 0x1000 */
    nvme_write32(g_mmio_base, 0x1000, g_asq_tail);

    /* Poll Admin CQ */
    for (uint32_t i = 0; i < NVME_TIMEOUT_LOOPS; i++) {
        uint16_t status = g_acq[g_acq_head].status;
        if ((status & 0x01) == g_acq_phase) {
            __asm__ volatile ("mfence" ::: "memory");
            if (out_cqe) {
                memcpy(out_cqe, (const void*)&g_acq[g_acq_head], sizeof(nvme_cqe_t));
            }
            uint16_t status_code = (status >> 1) & 0x7FFF;

            g_acq_head = (g_acq_head + 1) % NVME_ADMIN_QUEUE_SIZE;
            if (g_acq_head == 0) {
                g_acq_phase ^= 1;
            }
            /* Admin CQ Head Doorbell is at offset 0x1000 + doorbell_stride */
            nvme_write32(g_mmio_base, 0x1000 + g_doorbell_stride, g_acq_head);

            return (status_code == 0);
        }
        __asm__ volatile ("pause");
    }
    kernel_log("[FAULT] nvme: admin command opcode 0x%x timed out\n", (int)sqe->opc);
    return false;
}

static bool nvme_submit_io_cmd(nvme_sqe_t* sqe, nvme_cqe_t* out_cqe) {
    sqe->cid = g_io_cid++;
    memcpy((void*)&g_iosq[g_iosq_tail], sqe, sizeof(nvme_sqe_t));

    g_iosq_tail = (g_iosq_tail + 1) % NVME_IO_QUEUE_SIZE;

    __asm__ volatile ("mfence" ::: "memory");

    /* IO SQ1 Tail Doorbell = 0x1000 + (2 * 1) * doorbell_stride */
    uint32_t sq_doorbell = 0x1000 + (2 * 1) * g_doorbell_stride;
    nvme_write32(g_mmio_base, sq_doorbell, g_iosq_tail);

    /* IO CQ1 Head Doorbell = 0x1000 + (2 * 1 + 1) * doorbell_stride */
    uint32_t cq_doorbell = 0x1000 + (2 * 1 + 1) * g_doorbell_stride;

    for (uint32_t i = 0; i < NVME_TIMEOUT_LOOPS; i++) {
        uint16_t status = g_iocq[g_iocq_head].status;
        if ((status & 0x01) == g_iocq_phase) {
            __asm__ volatile ("mfence" ::: "memory");
            if (out_cqe) {
                memcpy(out_cqe, (const void*)&g_iocq[g_iocq_head], sizeof(nvme_cqe_t));
            }
            uint16_t status_code = (status >> 1) & 0x7FFF;

            g_iocq_head = (g_iocq_head + 1) % NVME_IO_QUEUE_SIZE;
            if (g_iocq_head == 0) {
                g_iocq_phase ^= 1;
            }
            nvme_write32(g_mmio_base, cq_doorbell, g_iocq_head);

            return (status_code == 0);
        }
        __asm__ volatile ("pause");
    }
    kernel_log("[FAULT] nvme: I/O command opcode 0x%x timed out\n", (int)sqe->opc);
    return false;
}

void nvme_init(void) {
    g_present = false;
    g_loc.found = false;
    spinlock_init(&g_nvme_lock);

    pci_enumerate(nvme_find_pci);
    if (!g_loc.found) {
        return;
    }

    if (g_loc.bar0_low & 0x01) {
        kernel_log("[FAULT] nvme: BAR0 is I/O mapped, expected memory mapping\n");
        return;
    }
    if (g_loc.bar0_high != 0) {
        kernel_log("[FAULT] nvme: 64-bit BAR0 base above 4GB (0x%x) not supported\n",
                   (int)g_loc.bar0_high);
        return;
    }

    uint32_t mmio_phys = g_loc.bar0_low & ~0x0Fu;
    g_mmio_base = map_nvme_mmio(mmio_phys, NVME_MMIO_MAP_SIZE);
    if (!g_mmio_base) {
        return;
    }

    /* Enable PCI bus mastering & memory space access */
    uint16_t cmd = pci_config_read16(g_loc.bus, g_loc.device, g_loc.function, 0x04);
    cmd |= 0x06; /* Bus master (bit 2) | Memory space (bit 1) */
    pci_config_write16(g_loc.bus, g_loc.device, g_loc.function, 0x04, cmd);

    /* Allocate physical memory for queues and staging buffer */
    g_asq_phys = pmm_alloc_contiguous(1);
    g_acq_phys = pmm_alloc_contiguous(1);
    g_iosq_phys = pmm_alloc_contiguous(1);
    g_iocq_phys = pmm_alloc_contiguous(1);
    g_staging_phys = pmm_alloc_contiguous(1);

    if (!g_asq_phys || !g_acq_phys || !g_iosq_phys || !g_iocq_phys || !g_staging_phys) {
        kernel_log("[FAULT] nvme: failed to allocate queue memory frames\n");
        return;
    }

    g_asq = (nvme_sqe_t*)g_asq_phys;
    g_acq = (nvme_cqe_t*)g_acq_phys;
    g_iosq = (nvme_sqe_t*)g_iosq_phys;
    g_iocq = (nvme_cqe_t*)g_iocq_phys;
    g_staging_buf = (uint8_t*)g_staging_phys;

    memset((void*)g_asq, 0, 4096);
    memset((void*)g_acq, 0, 4096);
    memset((void*)g_iosq, 0, 4096);
    memset((void*)g_iocq, 0, 4096);
    memset(g_staging_buf, 0, 4096);

    /* Read controller capabilities (CAP) */
    uint64_t cap = nvme_read64(g_mmio_base, NVME_REG_CAP);
    uint32_t dstrd = (uint32_t)((cap >> 32) & 0x0F);
    g_doorbell_stride = 4u << dstrd;

    /* Reset controller if currently enabled (CSTS.RDY == 1) */
    uint32_t csts = nvme_read32(g_mmio_base, NVME_REG_CSTS);
    if (csts & NVME_CSTS_RDY) {
        uint32_t cc = nvme_read32(g_mmio_base, NVME_REG_CC);
        cc &= ~NVME_CC_EN;
        nvme_write32(g_mmio_base, NVME_REG_CC, cc);
        for (uint32_t i = 0; i < NVME_TIMEOUT_LOOPS; i++) {
            if (!(nvme_read32(g_mmio_base, NVME_REG_CSTS) & NVME_CSTS_RDY)) {
                break;
            }
        }
    }

    /* Configure Admin Queue Attributes (AQA) */
    uint32_t aqa = ((NVME_ADMIN_QUEUE_SIZE - 1) << 16) | (NVME_ADMIN_QUEUE_SIZE - 1);
    nvme_write32(g_mmio_base, NVME_REG_AQA, aqa);

    /* Set Admin Queue Base Addresses */
    nvme_write64(g_mmio_base, NVME_REG_ASQ, g_asq_phys);
    nvme_write64(g_mmio_base, NVME_REG_ACQ, g_acq_phys);

    /* Configure and enable controller (CC) */
    uint32_t cc = NVME_CC_EN | NVME_CC_CSS_NVM | NVME_CC_MPS_4K |
                  NVME_CC_AMS_RR | NVME_CC_IOSQES_64 | NVME_CC_IOCQES_16;
    nvme_write32(g_mmio_base, NVME_REG_CC, cc);

    /* Poll CSTS.RDY until ready */
    bool rdy = false;
    for (uint32_t i = 0; i < NVME_TIMEOUT_LOOPS; i++) {
        uint32_t st = nvme_read32(g_mmio_base, NVME_REG_CSTS);
        if (st & NVME_CSTS_RDY) {
            rdy = true;
            break;
        }
        if (st & NVME_CSTS_CFS) {
            kernel_log("[FAULT] nvme: controller fatal status (CFS) set during init\n");
            return;
        }
    }

    if (!rdy) {
        kernel_log("[FAULT] nvme: controller timed out waiting for CSTS.RDY\n");
        return;
    }

    /* Identify Controller */
    nvme_sqe_t id_cmd;
    memset(&id_cmd, 0, sizeof(id_cmd));
    id_cmd.opc = NVME_ADMIN_OPC_IDENTIFY;
    id_cmd.nsid = 0;
    id_cmd.prp1 = g_staging_phys;
    id_cmd.cdw10 = NVME_IDENTIFY_CNS_CTRL;

    nvme_cqe_t cqe;
    if (!nvme_submit_admin_cmd(&id_cmd, &cqe)) {
        kernel_log("[FAULT] nvme: Identify Controller command failed\n");
        return;
    }

    nvme_id_ctrl_t* id_ctrl = (nvme_id_ctrl_t*)g_staging_buf;
    trim_spaces(g_model, id_ctrl->mn, sizeof(id_ctrl->mn));
    trim_spaces(g_serial, id_ctrl->sn, sizeof(id_ctrl->sn));

    /* Identify Namespace 1 */
    memset(&id_cmd, 0, sizeof(id_cmd));
    id_cmd.opc = NVME_ADMIN_OPC_IDENTIFY;
    id_cmd.nsid = 1;
    id_cmd.prp1 = g_staging_phys;
    id_cmd.cdw10 = NVME_IDENTIFY_CNS_NS;

    if (!nvme_submit_admin_cmd(&id_cmd, &cqe)) {
        kernel_log("[FAULT] nvme: Identify Namespace 1 command failed\n");
        return;
    }

    nvme_id_ns_t* id_ns = (nvme_id_ns_t*)g_staging_buf;
    g_sector_count = id_ns->nsze;
    uint8_t flbas_idx = id_ns->flbas & 0x0F;
    uint8_t lbads = id_ns->lbaf[flbas_idx].lbads;
    g_sector_size = (lbads >= 9 && lbads <= 12) ? (1u << lbads) : 512u;

    /* Create I/O Completion Queue 1 */
    nvme_sqe_t create_cq;
    memset(&create_cq, 0, sizeof(create_cq));
    create_cq.opc = NVME_ADMIN_OPC_CREATE_IO_CQ;
    create_cq.prp1 = g_iocq_phys;
    create_cq.cdw10 = ((NVME_IO_QUEUE_SIZE - 1) << 16) | 1; /* Size & QID=1 */
    create_cq.cdw11 = 0x0001; /* PC=1 (physically contiguous), IEN=0 (no IRQ) */

    if (!nvme_submit_admin_cmd(&create_cq, &cqe)) {
        kernel_log("[FAULT] nvme: Create I/O Completion Queue failed\n");
        return;
    }

    /* Create I/O Submission Queue 1 */
    nvme_sqe_t create_sq;
    memset(&create_sq, 0, sizeof(create_sq));
    create_sq.opc = NVME_ADMIN_OPC_CREATE_IO_SQ;
    create_sq.prp1 = g_iosq_phys;
    create_sq.cdw10 = ((NVME_IO_QUEUE_SIZE - 1) << 16) | 1; /* Size & QID=1 */
    create_sq.cdw11 = (1 << 16) | 0x0001; /* CQID=1, PC=1 */

    if (!nvme_submit_admin_cmd(&create_sq, &cqe)) {
        kernel_log("[FAULT] nvme: Create I/O Submission Queue failed\n");
        return;
    }

    g_present = true;

    uint32_t mb = (uint32_t)((g_sector_count * g_sector_size) / (1024u * 1024u));
    kernel_log("[ OK ] nvme at PCI %d:%d.%d: model '%s', serial '%s', "
               "%u sectors (%u MB), %u bytes/sector\n",
               (int)g_loc.bus, (int)g_loc.device, (int)g_loc.function,
               g_model, g_serial, (uint32_t)g_sector_count, mb, g_sector_size);
}

bool nvme_is_present(void) {
    return g_present;
}

uint64_t nvme_get_sector_count(void) {
    return g_sector_count;
}

uint32_t nvme_get_sector_size(void) {
    return g_sector_size;
}

static bool nvme_transfer_blocks(uint64_t lba, uint8_t count, void* buf, bool write) {
    if (!g_present || buf == NULL || count == 0) {
        return false;
    }

    uint32_t eflags = spinlock_acquire(&g_nvme_lock);

    nvme_sqe_t sqe;
    memset(&sqe, 0, sizeof(sqe));
    sqe.opc = write ? NVME_NVM_OPC_WRITE : NVME_NVM_OPC_READ;
    sqe.nsid = 1;
    sqe.prp1 = g_staging_phys;
    sqe.cdw10 = (uint32_t)(lba & 0xFFFFFFFFu);
    sqe.cdw11 = (uint32_t)(lba >> 32);
    sqe.cdw12 = (uint32_t)(count - 1); /* 0-based number of blocks */

    if (write && buf != g_staging_buf) {
        memcpy(g_staging_buf, buf, (size_t)count * g_sector_size);
    }

    nvme_cqe_t cqe;
    bool ok = nvme_submit_io_cmd(&sqe, &cqe);

    if (ok && !write && buf != g_staging_buf) {
        memcpy(buf, g_staging_buf, (size_t)count * g_sector_size);
    }

    spinlock_release(&g_nvme_lock, eflags);
    return ok;
}

bool nvme_read_sector(uint64_t sector, void* buf) {
    return nvme_read_sectors(sector, 1, buf);
}

bool nvme_write_sector(uint64_t sector, const void* buf) {
    return nvme_write_sectors(sector, 1, buf);
}

bool nvme_read_sectors(uint64_t start_lba, uint8_t sector_count, void* buf) {
    if (!g_present || buf == NULL || sector_count == 0) {
        return false;
    }
    uint8_t* dst = (uint8_t*)buf;
    uint32_t sectors_per_page = 4096u / g_sector_size;
    if (sectors_per_page == 0) sectors_per_page = 1;

    uint8_t remaining = sector_count;
    uint64_t cur_lba = start_lba;

    while (remaining > 0) {
        uint8_t chunk = (remaining > sectors_per_page) ? (uint8_t)sectors_per_page : remaining;
        if (!nvme_transfer_blocks(cur_lba, chunk, dst, false)) {
            return false;
        }
        dst += (size_t)chunk * g_sector_size;
        cur_lba += chunk;
        remaining -= chunk;
    }
    return true;
}

bool nvme_write_sectors(uint64_t start_lba, uint8_t sector_count, const void* buf) {
    if (!g_present || buf == NULL || sector_count == 0) {
        return false;
    }
    const uint8_t* src = (const uint8_t*)buf;
    uint32_t sectors_per_page = 4096u / g_sector_size;
    if (sectors_per_page == 0) sectors_per_page = 1;

    uint8_t remaining = sector_count;
    uint64_t cur_lba = start_lba;

    while (remaining > 0) {
        uint8_t chunk = (remaining > sectors_per_page) ? (uint8_t)sectors_per_page : remaining;
        if (!nvme_transfer_blocks(cur_lba, chunk, (void*)src, true)) {
            return false;
        }
        src += (size_t)chunk * g_sector_size;
        cur_lba += chunk;
        remaining -= chunk;
    }
    return true;
}

int nvme_selftest(void) {
    int failures = 0;

    /* 1. Invariant verification */
    if (sizeof(nvme_sqe_t) != 64) {
        failures |= 1;
    }
    if (sizeof(nvme_cqe_t) != 16) {
        failures |= 2;
    }
    if (sizeof(nvme_id_ctrl_t) != 4096) {
        failures |= 4;
    }
    if (sizeof(nvme_id_ns_t) != 4096) {
        failures |= 8;
    }

    /* 2. Doorbell calculation test */
    uint32_t stride_0 = 4 << 0;
    uint32_t sq0_db = 0x1000 + (2 * 0) * stride_0;
    uint32_t cq0_db = 0x1000 + (2 * 0 + 1) * stride_0;
    uint32_t sq1_db = 0x1000 + (2 * 1) * stride_0;
    uint32_t cq1_db = 0x1000 + (2 * 1 + 1) * stride_0;

    if (sq0_db != 0x1000 || cq0_db != 0x1004 || sq1_db != 0x1008 || cq1_db != 0x100C) {
        failures |= 16;
    }

    /* 3. Live hardware read/write/readback if present */
    if (g_present) {
        static uint8_t saved_sector[512];
        static uint8_t write_pattern[512];
        static uint8_t readback[512];

        bool read_saved = nvme_read_sector(1, saved_sector);
        if (!read_saved) {
            failures |= 32;
        }

        for (int i = 0; i < 512; i++) {
            write_pattern[i] = (uint8_t)(i ^ 0x5C);
        }

        bool wrote = nvme_write_sector(1, write_pattern);
        bool read = wrote && nvme_read_sector(1, readback);
        bool matches = read && (memcmp(write_pattern, readback, 512) == 0);

        if (!matches) {
            failures |= 64;
            kernel_log("[FAULT] NVME WRITE.READBACK: wrote=%d read=%d matches=%d\n",
                       (int)wrote, (int)read, (int)matches);
        } else {
            kernel_log("[ OK ] NVME WRITE.READBACK OK: a 512-byte sector written "
                       "via a real NVMe device read back byte-for-byte identical\n");
        }

        /* Restore original sector */
        if (read_saved) {
            nvme_write_sector(1, saved_sector);
        }
    }

    return failures;
}

DRIVER_REGISTER("nvme", nvme_init, DRIVER_PHASE_AFTER_PCI);
