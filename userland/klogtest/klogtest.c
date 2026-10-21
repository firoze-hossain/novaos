/*
 * userland/klogtest/klogtest.c - Phase 89: Structured, queryable kernel
 * logging subsystem conformance test.
 *
 * Exercises the SYS_KLOG_* syscalls from an unprivileged ring-3 process:
 * 1. Initial info query (SYS_KLOG_INFO).
 * 2. Appending structured entries (SYS_KLOG_WRITE) with custom facility & severity.
 * 3. Querying ring-buffer records (SYS_KLOG_READ) with exact field verification.
 * 4. Severity threshold filtering (suppressing lower-priority entries).
 * 5. Facility filtering (selecting only specific service records).
 * 6. Incremental streaming with monotonic sequence numbers (tail/follow semantics).
 * 7. Hostile/invalid pointer validation (returning -EFAULT without panicking).
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <novasys.h>
#include <nova_klog_abi.h>

#define CHECK(cond, msg) \
    do { \
        if (!(cond)) { \
            printf("[klogtest] FAIL: %s (line %d)\n", msg, __LINE__); \
            exit(1); \
        } \
    } while (0)

int main(void) {
    printf("[klogtest] Starting structured kernel logging conformance test...\n");

    /* 1. Hostile pointer tests */
    int fault_r = sys_klog_read(NULL);
    CHECK(fault_r == -14, "sys_klog_read(NULL) must return -EFAULT (-14)");

    int fault_w = sys_klog_write(NULL);
    CHECK(fault_w == -14, "sys_klog_write(NULL) must return -EFAULT (-14)");

    int fault_i = sys_klog_info(NULL);
    CHECK(fault_i == -14, "sys_klog_info(NULL) must return -EFAULT (-14)");

    /* 2. Initial info query */
    nova_klog_info_t info_before;
    memset(&info_before, 0, sizeof(info_before));
    int info_res = sys_klog_info(&info_before);
    CHECK(info_res == 0, "sys_klog_info must succeed");
    CHECK(info_before.buffer_capacity == 256, "capacity must be 256");
    CHECK(info_before.total_logged > 0, "total_logged should be > 0 from boot messages");
    CHECK(info_before.newest_seq > 0, "newest_seq should be > 0 from boot messages");

    printf("[klogtest] ok: info query (total_logged=%llu, newest_seq=%llu)\n",
           info_before.total_logged, info_before.newest_seq);

    /* 3. Write structured entries */
    nova_klog_write_t w1;
    w1.severity = NOVA_KLOG_WARN;
    w1.facility = "klogtest";
    w1.msg = "marker_warning_alpha";
    int w1_res = sys_klog_write(&w1);
    CHECK(w1_res == 0, "sys_klog_write w1 must succeed");

    nova_klog_write_t w2;
    w2.severity = NOVA_KLOG_ERR;
    w2.facility = "klogtest";
    w2.msg = "marker_error_beta";
    int w2_res = sys_klog_write(&w2);
    CHECK(w2_res == 0, "sys_klog_write w2 must succeed");

    nova_klog_write_t w3;
    w3.severity = NOVA_KLOG_INFO;
    w3.facility = "otherfac";
    w3.msg = "marker_info_gamma";
    int w3_res = sys_klog_write(&w3);
    CHECK(w3_res == 0, "sys_klog_write w3 must succeed");

    /* 4. Read back entries starting from info_before.newest_seq */
    nova_klog_entry_t entries[16];
    memset(entries, 0, sizeof(entries));

    nova_klog_read_t r_all;
    memset(&r_all, 0, sizeof(r_all));
    r_all.start_seq = info_before.newest_seq + 1;
    r_all.max_entries = 16;
    r_all.max_severity = NOVA_KLOG_SEV_ANY;
    r_all.facility_filter[0] = '\0';
    r_all.entries = entries;

    int read_count = sys_klog_read(&r_all);
    CHECK(read_count >= 3, "must read at least our 3 written entries");

    int found_alpha = 0, found_beta = 0, found_gamma = 0;
    unsigned long long alpha_seq = 0;

    for (int i = 0; i < read_count; i++) {
        if (strcmp(entries[i].msg, "marker_warning_alpha") == 0) {
            found_alpha = 1;
            alpha_seq = entries[i].seq;
            CHECK(entries[i].severity == NOVA_KLOG_WARN, "alpha severity must be WARN");
            CHECK(strcmp(entries[i].facility, "klogtest") == 0, "alpha facility must be klogtest");
            CHECK(entries[i].pid > 0, "entry PID must be stamped");
        } else if (strcmp(entries[i].msg, "marker_error_beta") == 0) {
            found_beta = 1;
            CHECK(entries[i].severity == NOVA_KLOG_ERR, "beta severity must be ERR");
            CHECK(strcmp(entries[i].facility, "klogtest") == 0, "beta facility must be klogtest");
        } else if (strcmp(entries[i].msg, "marker_info_gamma") == 0) {
            found_gamma = 1;
            CHECK(entries[i].severity == NOVA_KLOG_INFO, "gamma severity must be INFO");
            CHECK(strcmp(entries[i].facility, "otherfac") == 0, "gamma facility must be otherfac");
        }
    }
    CHECK(found_alpha && found_beta && found_gamma, "all 3 entries must be found with exact data");
    printf("[klogtest] ok: append and readback round-trip\n");

    /* 5. Severity threshold filtering */
    // Filter for max_severity = NOVA_KLOG_ERR (3)
    // Should match beta (ERR=3), but NOT alpha (WARN=4) or gamma (INFO=6)
    memset(entries, 0, sizeof(entries));
    nova_klog_read_t r_sev;
    memset(&r_sev, 0, sizeof(r_sev));
    r_sev.start_seq = info_before.newest_seq + 1;
    r_sev.max_entries = 16;
    r_sev.max_severity = NOVA_KLOG_ERR;
    r_sev.facility_filter[0] = '\0';
    r_sev.entries = entries;

    int sev_count = sys_klog_read(&r_sev);
    for (int i = 0; i < sev_count; i++) {
        CHECK(entries[i].severity <= NOVA_KLOG_ERR, "severity must be <= ERR");
        CHECK(strcmp(entries[i].msg, "marker_warning_alpha") != 0, "WARN must be filtered out");
        CHECK(strcmp(entries[i].msg, "marker_info_gamma") != 0, "INFO must be filtered out");
    }
    printf("[klogtest] ok: severity threshold filtering\n");

    /* 6. Facility filtering */
    // Filter for facility = "otherfac"
    memset(entries, 0, sizeof(entries));
    nova_klog_read_t r_fac;
    memset(&r_fac, 0, sizeof(r_fac));
    r_fac.start_seq = info_before.newest_seq + 1;
    r_fac.max_entries = 16;
    r_fac.max_severity = NOVA_KLOG_SEV_ANY;
    strncpy(r_fac.facility_filter, "otherfac", sizeof(r_fac.facility_filter) - 1);
    r_fac.entries = entries;

    int fac_count = sys_klog_read(&r_fac);
    CHECK(fac_count >= 1, "otherfac filter must return at least 1 entry");
    for (int i = 0; i < fac_count; i++) {
        CHECK(strcmp(entries[i].facility, "otherfac") == 0, "facility must match filter");
    }
    printf("[klogtest] ok: facility filtering\n");

    /* 7. Incremental streaming via start_seq */
    // Reading with start_seq = alpha_seq + 1 should not see alpha again
    memset(entries, 0, sizeof(entries));
    nova_klog_read_t r_stream;
    memset(&r_stream, 0, sizeof(r_stream));
    r_stream.start_seq = alpha_seq + 1;
    r_stream.max_entries = 16;
    r_stream.max_severity = NOVA_KLOG_SEV_ANY;
    r_stream.facility_filter[0] = '\0';
    r_stream.entries = entries;

    int stream_count = sys_klog_read(&r_stream);
    for (int i = 0; i < stream_count; i++) {
        CHECK(entries[i].seq > alpha_seq, "entries must have seq > alpha_seq");
        CHECK(strcmp(entries[i].msg, "marker_warning_alpha") != 0, "alpha must not be re-read");
    }
    printf("[klogtest] ok: incremental streaming with monotonic sequence numbers\n");

    printf("[klogtest] PASS: the full SYS_KLOG_* structured logging contract holds\n");
    return 0;
}
