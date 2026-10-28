/*
 * kernel/security/security_audit.h - Month 6: Full Kernel Security & Regression Audit
 *
 * Dedicated verification pass re-checking every capability and permission boundary
 * across the entire OS plan:
 * 1. User-pointer bounds & address-space isolation (paging_user_range_ok / is_user_ptr_valid)
 * 2. Root privilege gates (OS update, crashdump export, shutdown, firewall, rlimit)
 * 3. Process capability boundaries (file, host, spawn, exec, write, delete)
 * 4. Mandatory Access Control (MAC) enforcement and administrator privilege gating
 * 5. Cross-track application least-privilege matrix, credential zeroing & IPC/memory isolation
 */

#ifndef KERNEL_SECURITY_AUDIT_H
#define KERNEL_SECURITY_AUDIT_H

#include "../include/types.h"

/* Runs the comprehensive kernel security and regression audit self-test.
 * Returns 0 if all security checks pass, non-zero bitmask on failure. */
int kernel_security_audit_selftest(void);

#endif /* KERNEL_SECURITY_AUDIT_H */
