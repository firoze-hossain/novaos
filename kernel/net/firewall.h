#ifndef NET_FIREWALL_H
#define NET_FIREWALL_H

/*
 * Phase 88: the stateful firewall - the kernel side. The engine is
 * kernel/rust/firewall.rs; this is the C that puts it in the two places IP
 * traffic passes (ip_send() outbound, ip_handle_packet() inbound), loads its
 * configuration, formats its log lines and serves SYS_FW_INFO / SYS_FW_CTL.
 * The contract is userland/libc/include/nova_fw_abi.h.
 */

#include "../include/types.h"

/* Put the engine in its baseline: nothing gets in unless we asked for it,
 * outbound open, replies allowed back in. Called from net_init(), BEFORE any
 * NIC is brought up. An engine nobody initialised drops EVERYTHING. */
void fw_init(void);

/* Replace the baseline with FIREWALL.CFG, if there is one. Needs a filesystem,
 * so it runs later in boot. An invalid file changes nothing. */
void fw_load_config(void);

/* The filter. `payload` is the TRANSPORT bytes (no IP header). True to let the
 * packet through. */
bool fw_allow_out(uint32_t dest_ip, uint8_t protocol, const void* payload, uint16_t len);
bool fw_allow_in(uint32_t src_ip, uint8_t protocol, bool fragment, const void* payload, uint16_t len);

int fw_sys_info(uint32_t user_ptr);
int fw_sys_ctl(uint32_t user_ptr);

/* Pushes three hand-built packets through the real inbound path and checks the
 * engine's counters (see firewall.c). Logs one line. Call once the configuration is
 * loaded. */
void fw_wire_selftest(void);

#endif
