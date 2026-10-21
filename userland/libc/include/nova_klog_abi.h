#ifndef NOVA_KLOG_ABI_H
#define NOVA_KLOG_ABI_H

/*
 * nova_klog_abi.h - Phase 89: Structured, queryable kernel logging subsystem
 * (SYS_KLOG_READ, SYS_KLOG_WRITE, SYS_KLOG_INFO), shared verbatim by the kernel
 * and userland (novasys.h, klogtest, novainit, and the future log viewer).
 *
 * WHAT IT IS
 *
 * Replaces the plain serial-port log stream with real, timestamped, queryable,
 * per-service log entries - the difference between "why did this crash" being
 * answerable or not. A ring-buffer log store in the kernel with query syscalls,
 * feeding both existing kernel_log() call sites and novainit's own service output.
 *
 * Every call returns 0 (or count of entries read) on success, and a NEGATIVE
 * errno on failure.
 */

#define NOVA_SYS_KLOG_READ   75 /* EBX = nova_klog_read_t* (in/out) */
#define NOVA_SYS_KLOG_WRITE  76 /* EBX = nova_klog_write_t* (in) */
#define NOVA_SYS_KLOG_INFO   77 /* EBX = nova_klog_info_t* (out) */

/* Severities matching standard syslog conventions (0 is highest priority) */
#define NOVA_KLOG_EMERG   0u  /* System unusable (panic/fault) */
#define NOVA_KLOG_ALERT   1u  /* Immediate action required */
#define NOVA_KLOG_CRIT    2u  /* Critical condition */
#define NOVA_KLOG_ERR     3u  /* Error condition */
#define NOVA_KLOG_WARN    4u  /* Warning condition */
#define NOVA_KLOG_NOTICE  5u  /* Normal but significant */
#define NOVA_KLOG_INFO    6u  /* Informational */
#define NOVA_KLOG_DEBUG   7u  /* Debug-level details */

#define NOVA_KLOG_SEV_ANY 0xFFFFFFFFu /* Match any severity in queries */

#define NOVA_KLOG_FACILITY_MAX 16
#define NOVA_KLOG_MSG_MAX      128

/* A single structured log record */
typedef struct {
    unsigned long long seq;              /* Monotonic sequence number (starts at 1) */
    unsigned int       timestamp_ticks;  /* Kernel timer ticks at event */
    unsigned int       pid;              /* Originating PID (0 = kernel) */
    unsigned int       severity;         /* NOVA_KLOG_* level */
    char               facility[NOVA_KLOG_FACILITY_MAX]; /* Subsystem/service (NUL-terminated) */
    char               msg[NOVA_KLOG_MSG_MAX];           /* Log text (NUL-terminated) */
} nova_klog_entry_t;

/* Query / Read request */
typedef struct {
    unsigned long long start_seq;        /* Read entries with seq >= start_seq (0 = oldest available) */
    unsigned int       max_entries;      /* Capacity of user entries[] buffer */
    unsigned int       max_severity;     /* Max numerical severity to accept (e.g. WARN accepts EMERG..WARN) */
    char               facility_filter[NOVA_KLOG_FACILITY_MAX]; /* If non-empty, match facility */
    unsigned long long next_seq;         /* OUT: next sequence number after returned batch */
    unsigned long long dropped_count;    /* OUT: entries lost to ring buffer wraparound */
    unsigned long long total_logged;     /* OUT: total entries logged since boot */
    nova_klog_entry_t* entries;          /* OUT: pointer to user memory buffer */
} nova_klog_read_t;

/* Write / Append request */
typedef struct {
    unsigned int severity;               /* NOVA_KLOG_* */
    const char*  facility;               /* Originating service/facility name (NULL = "user") */
    const char*  msg;                    /* Message string (NUL-terminated) */
} nova_klog_write_t;

/* Subsystem statistics */
typedef struct {
    unsigned long long oldest_seq;       /* Earliest sequence number currently in buffer */
    unsigned long long newest_seq;       /* Latest sequence number currently in buffer */
    unsigned long long total_logged;     /* Total entries logged since boot */
    unsigned long long dropped_count;    /* Total entries overwritten before read */
    unsigned int       buffer_capacity;  /* Total ring buffer slots */
    unsigned int       entry_count;      /* Active entries currently in buffer */
} nova_klog_info_t;

#endif /* NOVA_KLOG_ABI_H */
