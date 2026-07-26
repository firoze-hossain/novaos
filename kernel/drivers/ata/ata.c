/*
 * ata.c - ATA PIO driver, primary bus, master device only
 */
#include "ata.h"
#include "../../arch/x86/io.h"
#include "../../include/kernel.h"

#define ATA_PRIMARY_IO   0x1F0
#define ATA_PRIMARY_CTRL 0x3F6

#define REG_DATA        (ATA_PRIMARY_IO + 0)
#define REG_ERROR       (ATA_PRIMARY_IO + 1)
#define REG_SECCOUNT    (ATA_PRIMARY_IO + 2)
#define REG_LBA_LOW     (ATA_PRIMARY_IO + 3)
#define REG_LBA_MID     (ATA_PRIMARY_IO + 4)
#define REG_LBA_HIGH    (ATA_PRIMARY_IO + 5)
#define REG_DRIVE_HEAD  (ATA_PRIMARY_IO + 6)
#define REG_STATUS      (ATA_PRIMARY_IO + 7)
#define REG_COMMAND     (ATA_PRIMARY_IO + 7)

#define STATUS_ERR  0x01
#define STATUS_DRQ  0x08
#define STATUS_SRV  0x10
#define STATUS_DF   0x20
#define STATUS_BSY  0x80

#define CMD_READ_SECTORS 0x20
#define CMD_IDENTIFY     0xEC

static bool drive_present = false;

static void wait_busy_clear(void) {
    while (inb(REG_STATUS) & STATUS_BSY) { }
}

static bool wait_drq_or_error(void) {
    while (1) {
        uint8_t status = inb(REG_STATUS);
        if (status & STATUS_ERR) {
            return false;
        }
        if (status & STATUS_DRQ) {
            return true;
        }
    }
}

void ata_init(void) {
    drive_present = false;

    outb(REG_DRIVE_HEAD, 0xA0); /* select master, LBA not needed for IDENTIFY */
    io_wait();
    outb(REG_SECCOUNT, 0);
    outb(REG_LBA_LOW, 0);
    outb(REG_LBA_MID, 0);
    outb(REG_LBA_HIGH, 0);
    outb(REG_COMMAND, CMD_IDENTIFY);

    uint8_t status = inb(REG_STATUS);
    if (status == 0) {
        kernel_log("[ .. ] ATA primary master: not present\n");
        return;
    }

    wait_busy_clear();

    /* A non-zero LBA mid/high at this point means this is an ATAPI
     * device (e.g. a CD-ROM) responding to IDENTIFY, not a plain ATA
     * disk - out of scope for this driver. */
    if (inb(REG_LBA_MID) != 0 || inb(REG_LBA_HIGH) != 0) {
        kernel_log("[ .. ] ATA primary master: ATAPI device, skipping\n");
        return;
    }

    if (!wait_drq_or_error()) {
        kernel_log("[ .. ] ATA primary master: IDENTIFY error\n");
        return;
    }

    uint16_t identify_data[256];
    for (int i = 0; i < 256; i++) {
        identify_data[i] = inw(REG_DATA);
    }

    /* Model string is words 27-46, each word byte-swapped relative to
     * normal string order (an ATA spec quirk from its 16-bit PIO
     * heritage) and space-padded rather than NUL-terminated. */
    char model[41];
    for (int i = 0; i < 20; i++) {
        uint16_t word = identify_data[27 + i];
        model[i * 2] = (char)(word >> 8);
        model[i * 2 + 1] = (char)(word & 0xFF);
    }
    model[40] = '\0';
    for (int i = 39; i >= 0 && model[i] == ' '; i--) {
        model[i] = '\0';
    }

    drive_present = true;
    kernel_log("[ OK ] ATA primary master detected (LBA28 PIO): %s\n", model);
}

bool ata_is_present(void) {
    return drive_present;
}

bool ata_read_sectors(uint32_t lba, uint8_t sector_count, void* buffer) {
    if (!drive_present || sector_count == 0) {
        return false;
    }

    uint16_t* buf16 = (uint16_t*)buffer;

    wait_busy_clear();

    outb(REG_DRIVE_HEAD, (uint8_t)(0xE0 | ((lba >> 24) & 0x0F)));
    outb(REG_SECCOUNT, sector_count);
    outb(REG_LBA_LOW, (uint8_t)(lba & 0xFF));
    outb(REG_LBA_MID, (uint8_t)((lba >> 8) & 0xFF));
    outb(REG_LBA_HIGH, (uint8_t)((lba >> 16) & 0xFF));
    outb(REG_COMMAND, CMD_READ_SECTORS);

    for (uint8_t s = 0; s < sector_count; s++) {
        wait_busy_clear();
        if (!wait_drq_or_error()) {
            kernel_log("[FAULT] ATA read error at LBA %d\n", (int)(lba + s));
            return false;
        }
        for (int i = 0; i < ATA_SECTOR_SIZE / 2; i++) {
            buf16[s * (ATA_SECTOR_SIZE / 2) + i] = inw(REG_DATA);
        }
    }

    return true;
}
