#ifndef _NOVA_PROC_ABI_H
#define _NOVA_PROC_ABI_H

#include <stdint.h>

/* Month 5: Activity & Performance Monitor ABI (SYS_PROC_INFO 84) */
#define NOVA_SYS_PROC_INFO 84

/* Operations for SYS_PROC_INFO */
#define NOVA_PROC_OP_GET_SYSINFO 1
#define NOVA_PROC_OP_GET_TABLE   2
#define NOVA_PROC_OP_GET_ENTRY   3

/* Constants & Limits */
#define NOVA_PROC_MAX_ENTRIES   64
#define NOVA_PROC_NAME_MAX      32

/* Process States */
#define NOVA_PROC_STATE_UNUSED      0
#define NOVA_PROC_STATE_ALLOCATING  1
#define NOVA_PROC_STATE_READY       2
#define NOVA_PROC_STATE_RUNNING     3
#define NOVA_PROC_STATE_TERMINATED  4

/* System-Wide Activity & Performance Metrics */
typedef struct {
    uint32_t uptime_ticks;       /* System uptime in 10ms timer ticks */
    uint32_t cpu_count;          /* Online SMP CPU core count */
    uint32_t cpu_usage_pct;      /* Overall system CPU utilization (0-100%) */
    uint32_t total_frames;       /* Total tracked physical RAM frames (4KB each) */
    uint32_t used_frames;        /* Allocated physical RAM frames */
    uint32_t free_frames;        /* Free physical RAM frames */
    uint32_t total_processes;    /* Total active processes in process table */
    uint32_t running_processes;  /* Processes currently in READY or RUNNING */
    uint32_t user_processes;     /* Userland processes */
    uint32_t kernel_processes;   /* Kernel-level tasks */
    uint32_t context_switches;   /* Context switch counter */
} nova_sys_activity_t;

/* Single Process Activity Record */
typedef struct {
    uint32_t pid;
    char     name[NOVA_PROC_NAME_MAX];
    uint32_t state;             /* NOVA_PROC_STATE_* */
    uint32_t is_user;          /* 1 = Ring-3 userland, 0 = Ring-0 kernel task */
    uint32_t uid;              /* User ID (0 = root) */
    uint32_t gid;              /* Group ID */
    uint32_t cpu_ticks;        /* Cumulative CPU ticks consumed */
    uint32_t cpu_percent;      /* CPU percent usage in current window (0-100%) */
    uint32_t mem_pages;        /* Allocated/mapped virtual pages */
    uint32_t mem_bytes;        /* Memory footprint in bytes */
    uint32_t throttled;        /* 1 if currently throttled by rlimit */
    uint32_t off_cpu;          /* 1 if switched off CPU */
} nova_proc_entry_t;

/* Process Table Snapshot */
typedef struct {
    uint32_t          count;
    nova_proc_entry_t entries[NOVA_PROC_MAX_ENTRIES];
} nova_proc_table_t;

#endif /* _NOVA_PROC_ABI_H */
