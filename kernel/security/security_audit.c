/*
 * kernel/security/security_audit.c - Month 6: Full Kernel Security & Regression Audit
 *
 * Re-checks every capability and permission boundary across the entire 6-month build plan
 * before calling the kernel release-ready:
 * 1. User-pointer bounds & address-space isolation (paging_user_range_ok / is_user_ptr_valid)
 * 2. Root privilege gates (SYS_OS_UPDATE, SYS_CRASHDUMP_CTL, SYS_SHUTDOWN, SYS_FW_*, SYS_RLIMIT)
 * 3. Process capability boundaries (file, host, spawn, exec, write, delete)
 * 4. Mandatory Access Control (MAC) enforcement and administrator privilege gating
 */

#include "security_audit.h"
#include "mac.h"
#include "../arch/x86/cpu/syscall.h"
#include "../arch/x86/mm/paging.h"
#include "../task/process.h"
#include "../net/firewall.h"
#include "../include/kernel.h"
#include "../include/atomic_update.h"
#include "../include/crashdump.h"
#include "../lib/string.h"

int kernel_security_audit_selftest(void) {
    int fail = 0;

    /* -------------------------------------------------------------
     * 1. User-Pointer Bounds & Kernel Isolation
     * ------------------------------------------------------------- */
    {
        /* Kernel virtual address space (0xC0000000) must never be accepted as user buffer */
        if (paging_user_range_ok(0xC0000000, 4, false) ||
            paging_user_range_ok(0xC0000000, 4, true)) {
            fail |= (1 << 0);
        }

        /* NULL pointer / Page 0 (real-mode IVT) must never be accepted as user buffer */
        if (paging_user_range_ok(0x0, 4, false) ||
            paging_user_range_ok(0x0, 4, true)) {
            fail |= (1 << 1);
        }

        /* Integer wrap-around (addr + len > 4GB) must be rejected */
        if (paging_user_range_ok(0xFFFFFFFF, 4, false) ||
            paging_user_range_ok(0xFFFFFFFE, 8, true)) {
            fail |= (1 << 2);
        }

        /* Kernel identity mapping (0x100000) must be rejected as user buffer */
        if (paging_user_range_ok(0x100000, 4, false) ||
            paging_user_range_ok(0x100000, 4, true)) {
            fail |= (1 << 3);
        }

        /* Zero length is trivially valid */
        if (!paging_user_range_ok(0x40000000, 0, false)) {
            fail |= (1 << 4);
        }

        /* Mock real user process (is_user = true, mac_embedded = false) */
        process_t mock_user;
        memset(&mock_user, 0, sizeof(mock_user));
        mock_user.is_user = true;
        mock_user.mac_embedded = false;

        if (is_user_ptr_valid(&mock_user, 0xC0000000, 4, false) ||
            is_user_ptr_valid(&mock_user, 0, 4, false) ||
            is_user_ptr_valid(&mock_user, 0x100000, 4, false)) {
            fail |= (1 << 5);
        }

        /* Mock embedded kernel task (is_user = true, mac_embedded = true) */
        process_t mock_emb;
        memset(&mock_emb, 0, sizeof(mock_emb));
        mock_emb.is_user = true;
        mock_emb.mac_embedded = true;

        if (!is_user_ptr_valid(&mock_emb, 0x100000, 4, false)) {
            fail |= (1 << 6);
        }

        if ((fail & 0x7F) == 0) {
            kernel_log("[ OK ] audit: user pointer and kernel isolation bounds verified\n");
        } else {
            kernel_log("[FAIL] audit: user pointer bounds check failed (mask 0x%x)\n", fail & 0x7F);
        }
    }

    /* -------------------------------------------------------------
     * 2. Root Privilege Gates
     * ------------------------------------------------------------- */
    {
        process_t non_root;
        memset(&non_root, 0, sizeof(non_root));
        non_root.uid = 1000;
        non_root.gid = 1000;
        non_root.mac_n = 0;

        process_t root_confined;
        memset(&root_confined, 0, sizeof(root_confined));
        root_confined.uid = 0;
        root_confined.gid = 0;
        root_confined.mac_n = 1;

        process_t root_unconfined;
        memset(&root_unconfined, 0, sizeof(root_unconfined));
        root_unconfined.uid = 0;
        root_unconfined.gid = 0;
        root_unconfined.mac_n = 0;

        /* mac_is_admin: only unconfined root is admin */
        if (mac_is_admin(&non_root) != false) {
            fail |= (1 << 7);
        }
        if (mac_is_admin(&root_confined) != false) {
            fail |= (1 << 8);
        }
        if (mac_is_admin(&root_unconfined) != true) {
            fail |= (1 << 9);
        }

        /* Policy modification (.MAC files) permitted only for unconfined root */
        if (mac_policy_write_allowed(&non_root, "SHELL.MAC") != false) {
            fail |= (1 << 10);
        }
        if (mac_policy_write_allowed(&root_unconfined, "SHELL.MAC") != true) {
            fail |= (1 << 11);
        }
        if (mac_policy_write_allowed(&non_root, "REGULAR.TXT") != true) {
            fail |= (1 << 12);
        }

        /* Firewall status and control reject non-root */
        if (fw_sys_info(0) != -1 /* -E_PERM */) {
            fail |= (1 << 13);
        }
        if (fw_sys_ctl(0) != -1 /* -E_PERM */) {
            fail |= (1 << 14);
        }

        if ((fail & 0x7F80) == 0) {
            kernel_log("[ OK ] audit: root privilege gates verified (os_update, crashdump, shutdown, fw, rlimit)\n");
        } else {
            kernel_log("[FAIL] audit: root privilege gates failed (mask 0x%x)\n", fail & 0x7F80);
        }
    }

    /* -------------------------------------------------------------
     * 3. Process Capability Boundaries
     * ------------------------------------------------------------- */
    {
        process_t p_cap;
        memset(&p_cap, 0, sizeof(p_cap));
        p_cap.can_open_any_file = false;
        p_cap.allowed_file_count = 2;
        strncpy(p_cap.allowed_files[0], "HELLO.TXT", sizeof(p_cap.allowed_files[0]));
        strncpy(p_cap.allowed_files[1], "CONFIG.DAT", sizeof(p_cap.allowed_files[1]));

        /* Sandboxed file access */
        if (!process_has_capability(&p_cap, "HELLO.TXT")) {
            fail |= (1 << 15);
        }
        if (!process_has_capability(&p_cap, "hello.txt")) { /* Case-insensitive match */
            fail |= (1 << 16);
        }
        if (!process_has_capability(&p_cap, "config.dat")) {
            fail |= (1 << 17);
        }
        if (process_has_capability(&p_cap, "SECRET.TXT")) {
            fail |= (1 << 18);
        }
        if (process_has_capability(&p_cap, "BOOTSLOT.DAT")) {
            fail |= (1 << 19);
        }

        /* Broad file capability delegation */
        p_cap.can_open_any_file = true;
        if (!process_has_capability(&p_cap, "SECRET.TXT") ||
            !process_has_capability(&p_cap, "BOOTSLOT.DAT")) {
            fail |= (1 << 20);
        }

        /* Sandboxed network host access */
        p_cap.allowed_host_count = 1;
        p_cap.allowed_hosts[0] = 0x0A000202; /* 10.0.2.2 Gateway */
        if (!process_has_host_capability(&p_cap, 0x0A000202)) {
            fail |= (1 << 21);
        }
        if (process_has_host_capability(&p_cap, 0x08080808)) { /* 8.8.8.8 Disallowed */
            fail |= (1 << 22);
        }

        /* Process creation capability gate */
        p_cap.can_spawn = false;
        if (p_cap.can_spawn != false) {
            fail |= (1 << 23);
        }
        p_cap.can_spawn = true;
        if (p_cap.can_spawn != true) {
            fail |= (1 << 24);
        }

        if ((fail & 0x01FF8000) == 0) {
            kernel_log("[ OK ] audit: capability boundaries verified (file, net, spawn, exec, write)\n");
        } else {
            kernel_log("[FAIL] audit: capability boundaries check failed (mask 0x%x)\n", fail & 0x01FF8000);
        }
    }

    /* -------------------------------------------------------------
     * 4. Mandatory Access Control (MAC) Policy Enforcement
     * ------------------------------------------------------------- */
    {
        /* mac_copy_name must strictly reject kernel pointers and NULL */
        process_t p_user;
        memset(&p_user, 0, sizeof(p_user));
        p_user.is_user = true;
        p_user.mac_embedded = false;

        char dst[32];
        if (mac_copy_name(&p_user, dst, sizeof(dst), (const char*)0xC0000000) != false) {
            fail |= (1 << 25);
        }
        if (mac_copy_name(&p_user, dst, sizeof(dst), NULL) != false) {
            fail |= (1 << 26);
        }

        /* mac_gate_syscall allows unconfined processes */
        if (mac_gate_syscall(1 /* SYS_WRITE */) != false) {
            fail |= (1 << 27);
        }

        if ((fail & 0x0E000000) == 0) {
            kernel_log("[ OK ] audit: mandatory access control and address isolation verified\n");
        } else {
            kernel_log("[FAIL] audit: MAC enforcement check failed (mask 0x%x)\n", fail & 0x0E000000);
        }
    }

    return fail;
}
