#ifndef DISK_INSTALLER_H
#define DISK_INSTALLER_H

#include "types.h"

#define DISK_INSTALL_OP_QUERY     1
#define DISK_INSTALL_OP_DRY_RUN   2
#define DISK_INSTALL_OP_FORMAT    3
#define DISK_INSTALL_OP_SELFTEST  4

#define DISK_INSTALL_OK                    0
#define DISK_INSTALL_ERR_INVALID_PARAM   (-1)
#define DISK_INSTALL_ERR_DISK_TOO_SMALL  (-2)
#define DISK_INSTALL_ERR_DEVICE_NOT_FOUND (-3)
#define DISK_INSTALL_ERR_IO_WRITE        (-4)
#define DISK_INSTALL_ERR_IO_READ         (-5)
#define DISK_INSTALL_ERR_PERMISSION      (-6)
#define DISK_INSTALL_ERR_VERIFY_FAIL     (-7)

typedef struct __attribute__((packed)) {
    int32_t  target_device;    /* 0 = ATA, 1 = VirtIO-Blk, 2 = NVMe */
    uint32_t total_sectors;    /* 0 = auto/default (102MiB = 208,896 sectors) */
    uint32_t part1_mb;         /* FAT32 size in MB (0 = default 64) */
    uint32_t part2_mb;         /* ext2 size in MB (0 = default 32) */
    uint32_t part3_mb;         /* Journal size in MB (0 = default 4) */
    uint32_t part4_mb;         /* Crashdump size in MB (0 = default 1) */
    uint32_t dry_run;          /* 1 = validate only, 0 = live write */
    uint32_t format_fat32;     /* 1 = format FAT32, 0 = partition table only */
    uint32_t format_ext2;      /* 1 = format ext2, 0 = skip */
} nova_disk_layout_config_t;

typedef struct __attribute__((packed)) {
    int32_t  status;           /* 0 on success, < 0 on error */
    int32_t  target_device;
    uint32_t total_sectors;
    uint32_t part1_start_lba;
    uint32_t part1_sectors;
    uint32_t part2_start_lba;
    uint32_t part2_sectors;
    uint32_t part3_start_lba;
    uint32_t part3_sectors;
    uint32_t part4_start_lba;
    uint32_t part4_sectors;
    uint32_t fat_size_sectors;
    uint32_t total_clusters;
    uint32_t sectors_written;
} nova_disk_install_summary_t;

int rust_disk_installer_format(const nova_disk_layout_config_t* cfg,
                                nova_disk_install_summary_t* out_summary);
int rust_disk_installer_query(int target_dev,
                              nova_disk_install_summary_t* out_summary);
int rust_disk_installer_selftest(void);

#endif /* DISK_INSTALLER_H */
