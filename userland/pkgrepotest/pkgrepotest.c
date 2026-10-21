/*
 * userland/pkgrepotest/pkgrepotest.c - Phase 90: Package repository network protocol
 * support (server-side) conformance test.
 *
 * Exercises SYS_PKGREPO_CTL from an unprivileged ring-3 process:
 * 1. Hostile/invalid pointer validation (returning -EFAULT on NULL and unmapped ranges).
 * 2. Invalid command validation (returning -EINVAL).
 * 3. Initial status check.
 * 4. Start repository server on custom port (e.g., 8080).
 * 5. Duplicate start rejection (returning -EBUSY).
 * 6. Running status verification (is_running == 1, port == 8080).
 * 7. Stop repository server.
 * 8. Stopped status verification (is_running == 0).
 * 9. Stopping when already stopped returns -1.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <novasys.h>
#include <nova_pkgrepo_abi.h>

#define CHECK(cond, msg) \
    do { \
        if (!(cond)) { \
            printf("[pkgrepotest] FAIL: %s (line %d)\n", msg, __LINE__); \
            exit(1); \
        } \
    } while (0)

int main(void) {
    printf("[pkgrepotest] Starting package repository server conformance test...\n");

    /* 1. Hostile / invalid pointers on STATUS */
    int fault_null = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STATUS, NULL);
    CHECK(fault_null == -14, "sys_pkgrepo_ctl(STATUS, NULL) must return -EFAULT (-14)");

    int fault_kern = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STATUS, (void*)0xC0000000);
    CHECK(fault_kern == -14, "sys_pkgrepo_ctl(STATUS, 0xC0000000) must return -EFAULT (-14)");

    /* 2. Invalid commands */
    int inv_cmd = sys_pkgrepo_ctl(999, NULL);
    CHECK(inv_cmd == -22, "sys_pkgrepo_ctl(invalid_cmd) must return -EINVAL (-22)");

    /* 3. Initial status */
    nova_pkgrepo_status_t st;
    memset(&st, 0xFF, sizeof(st));
    int rc_st = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STATUS, &st);
    CHECK(rc_st == 0, "sys_pkgrepo_ctl(STATUS) must succeed");
    printf("[pkgrepotest] ok: initial status (running=%u, port=%u)\n", st.is_running, st.port);

    /* If it was already running, stop it first */
    if (st.is_running) {
        int rc_stop_init = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STOP, NULL);
        CHECK(rc_stop_init == 0, "stop initial server must succeed");
    }

    /* 4. Start repository server on port 8080 */
    int rc_start = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_START, (void*)8080);
    CHECK(rc_start == 0, "sys_pkgrepo_ctl(START, 8080) must succeed");
    printf("[pkgrepotest] ok: started repository server on port 8080\n");

    /* 5. Duplicate start rejection */
    int rc_dup = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_START, (void*)8080);
    CHECK(rc_dup == -16, "sys_pkgrepo_ctl duplicate start must return -EBUSY (-16)");
    printf("[pkgrepotest] ok: duplicate start rejected with -EBUSY\n");

    /* 6. Verify running status */
    memset(&st, 0, sizeof(st));
    rc_st = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STATUS, &st);
    CHECK(rc_st == 0, "query status after start must succeed");
    CHECK(st.is_running == 1, "is_running must be 1");
    CHECK(st.port == 8080, "port must be 8080");
    printf("[pkgrepotest] ok: running status verified (running=%u, port=%u)\n", st.is_running, st.port);

    /* 7. Stop repository server */
    int rc_stop = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STOP, NULL);
    CHECK(rc_stop == 0, "sys_pkgrepo_ctl(STOP) must succeed");
    printf("[pkgrepotest] ok: server stopped\n");

    /* 8. Verify stopped status */
    memset(&st, 0xFF, sizeof(st));
    rc_st = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STATUS, &st);
    CHECK(rc_st == 0, "query status after stop must succeed");
    CHECK(st.is_running == 0, "is_running must be 0");
    printf("[pkgrepotest] ok: stopped status verified (running=%u)\n", st.is_running);

    /* 9. Stopping when already stopped */
    int rc_stop2 = sys_pkgrepo_ctl(NOVA_PKGREPO_CTL_STOP, NULL);
    CHECK(rc_stop2 == -1, "stopping already-stopped server must return -1");

    printf("[pkgrepotest] PASS: package repository protocol and control contract holds\n");
    return 0;
}
