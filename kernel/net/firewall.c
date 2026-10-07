/*
 * kernel/net/firewall.c - Phase 88: where the stateful firewall is applied.
 * The decisions are made by kernel/rust/firewall.rs; this file puts it in the
 * two places IP traffic passes, loads its configuration, formats its log lines
 * and serves the two syscalls. See firewall.h and
 * userland/libc/include/nova_fw_abi.h for the model.
 *
 * Nothing here decides anything about a packet. That is deliberate: the one
 * piece of code that says yes or no is the one that is tested against a
 * reference model and fault injection on the host.
 */
#include "firewall.h"
#include "net.h"
#include "ip.h"
#include "../task/process.h"
#include "../arch/x86/mm/paging.h"
#include "../drivers/timer/timer.h"
#include "../fs/vfs.h"
#include "../include/kernel.h"
#include "../security/mac.h"
#include "../lib/string.h"
#include "../../userland/libc/include/nova_fw_abi.h"

extern void rust_fw_init(void);
extern uint32_t rust_fw_selftest(uint32_t* out_total);
extern int rust_fw_load_config(const uint8_t* text, uint32_t len, uint32_t* out_kind, uint32_t* out_line);
extern int rust_fw_filter(uint32_t dir, uint32_t src, uint32_t dst, uint32_t proto, uint32_t frag,
                          const uint8_t* payload, uint32_t len, uint32_t now);
extern int rust_fw_probe(const uint32_t* in, uint32_t* out, uint32_t now);
extern int rust_fw_add_rule(const uint8_t* text, uint32_t len, uint32_t pos);
extern int rust_fw_del_rule(uint32_t idx);
extern int rust_fw_flush_rules(void);
extern int rust_fw_set_policy(uint32_t dir, uint32_t accept);
extern int rust_fw_set_enabled(uint32_t on);
extern void rust_fw_lock(void);
extern void rust_fw_flush_conns(void);
extern void rust_fw_info(uint32_t* out, uint32_t now);
extern int rust_fw_get_rule(uint32_t idx, uint32_t* out);
extern int rust_fw_get_conn(uint32_t n, uint32_t* out, uint32_t now);

/* The largest configuration the engine accepts is 4096 bytes; one more byte
 * lets an over-long file be seen as too long and not silently truncated into a
 * valid shorter one. Read once at boot, so a static buffer is safe. */
#define CFG_BUF_SIZE 4097
static uint8_t cfg_buf[CFG_BUF_SIZE];

typedef char fw_info_words_check[(sizeof(nova_fw_info_t) == 24 * sizeof(uint32_t)) ? 1 : -1];

/* ------------------------------------------------------------------
 * The hooks.
 * ------------------------------------------------------------------ */

bool fw_allow_out(uint32_t dest_ip, uint8_t protocol, const void* payload, uint16_t len) {
    return rust_fw_filter(NOVA_FW_DIR_OUT, NET_OUR_IP, dest_ip, protocol, 0,
                          (const uint8_t*)payload, len, timer_get_ticks()) != 0;
}

bool fw_allow_in(uint32_t src_ip, uint8_t protocol, bool fragment, const void* payload, uint16_t len) {
    return rust_fw_filter(NOVA_FW_DIR_IN, src_ip, NET_OUR_IP, protocol, fragment ? 1 : 0,
                          (const uint8_t*)payload, len, timer_get_ticks()) != 0;
}

/* ------------------------------------------------------------------
 * Logging. The engine decides WHEN a line is wanted (rate limited: the first
 * few, then one in 256) and calls this with no lock held.
 * ------------------------------------------------------------------ */

static const char* proto_name(uint32_t p) {
    return p == 6 ? "tcp" : (p == 17 ? "udp" : (p == 1 ? "icmp" : "other"));
}

static const char* reason_name(uint32_t r) {
    switch (r) {
    case NOVA_FW_R_RULE:        return "rule";
    case NOVA_FW_R_POLICY:      return "default policy";
    case NOVA_FW_R_INVALID:     return "invalid packet";
    case NOVA_FW_R_FRAGMENT:    return "fragment";
    case NOVA_FW_R_UNSUPPORTED: return "unsupported protocol";
    case NOVA_FW_R_MALFORMED:   return "malformed header";
    case NOVA_FW_R_TABLE_FULL:  return "connection table full";
    case NOVA_FW_R_HALF_OPEN:   return "too many half-open connections";
    default:                    return "other";
    }
}

void fw_hal_log(uint32_t dir, uint32_t proto, uint32_t src, uint32_t dst, uint32_t sport,
                uint32_t dport, uint32_t accept, uint32_t reason, int32_t rule) {
    if (rule >= 0) {
        kernel_log("[FW] %s %s %s %d.%d.%d.%d:%d -> %d.%d.%d.%d:%d (rule %d)\n",
                   accept ? "accepted" : "dropped", dir == 0 ? "in" : "out", proto_name(proto),
                   (int)(src >> 24) & 0xFF, (int)(src >> 16) & 0xFF, (int)(src >> 8) & 0xFF, (int)src & 0xFF,
                   (int)sport, (int)(dst >> 24) & 0xFF, (int)(dst >> 16) & 0xFF,
                   (int)(dst >> 8) & 0xFF, (int)dst & 0xFF, (int)dport, (int)rule);
    } else {
        kernel_log("[FW] %s %s %s %d.%d.%d.%d:%d -> %d.%d.%d.%d:%d (%s)\n",
                   accept ? "accepted" : "dropped", dir == 0 ? "in" : "out", proto_name(proto),
                   (int)(src >> 24) & 0xFF, (int)(src >> 16) & 0xFF, (int)(src >> 8) & 0xFF, (int)src & 0xFF,
                   (int)sport, (int)(dst >> 24) & 0xFF, (int)(dst >> 16) & 0xFF,
                   (int)(dst >> 8) & 0xFF, (int)dst & 0xFF, (int)dport, reason_name(reason));
    }
}

/* ------------------------------------------------------------------
 * Boot.
 * ------------------------------------------------------------------ */

void fw_init(void) {
    rust_fw_init(); /* the baseline: policy in drop, out accept, replies allowed back in */
    uint32_t total = 0;
    uint32_t held = rust_fw_selftest(&total);
    kernel_log("[ %s ] Firewall: stateful packet filter in ip_send()/ip_handle_packet() "
               "(baseline: nothing in unless we asked for it, outbound open), "
               "connection tracking, engine self-test %d/%d\n",
               (held == total) ? "OK" : "FAIL", (int)held, (int)total);
}

void fw_load_config(void) {
    int n = vfs_read_file("FIREWALL.CFG", cfg_buf, CFG_BUF_SIZE);
    if (n < 0) {
        kernel_log("[ OK ] Firewall: no FIREWALL.CFG, keeping the built-in baseline\n");
        return;
    }
    uint32_t kind = 0, line = 0;
    int rc = rust_fw_load_config(cfg_buf, (uint32_t)n, &kind, &line);
    if (rc != 0) {
        /* An unusable file does NOT open the firewall and does NOT close it:
         * the baseline stays, which is already the safe stance. */
        kernel_log("[WARN] FIREWALL.CFG is invalid (error %d, parse kind %d at line %d): "
                   "keeping the built-in baseline\n", rc, (int)kind, (int)line);
        return;
    }
    uint32_t info[24];
    rust_fw_info(info, timer_get_ticks());
    kernel_log("[ OK ] Firewall: %d rules from FIREWALL.CFG (%d bytes), policy in=%s out=%s, "
               "connection table of %d flows\n", (int)info[4], n, info[2] ? "accept" : "drop",
               info[3] ? "accept" : "drop", (int)info[21]);
}

/* ------------------------------------------------------------------
 * The syscalls.
 * ------------------------------------------------------------------ */

#define E_PERM  1
#define E_FAULT 14
#define E_INVAL 22

int fw_sys_info(uint32_t user_ptr) {
    process_t* p = process_current();
    if (p == NULL || p->uid != 0) {
        return -E_PERM;
    }
    if (!paging_user_range_ok(user_ptr, sizeof(nova_fw_info_t), true)) {
        return -E_FAULT;
    }
    nova_fw_info_t out;
    rust_fw_info((uint32_t*)&out, timer_get_ticks());
    memcpy((void*)user_ptr, &out, sizeof out);
    return 0;
}

int fw_sys_ctl(uint32_t user_ptr) {
    process_t* p = process_current();
    if (p == NULL || p->uid != 0) {
        return -E_PERM; /* checked before the pointer is looked at */
    }
    if (!paging_user_range_ok(user_ptr, sizeof(nova_fw_req_t), true)) {
        return -E_FAULT;
    }
    nova_fw_req_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    req.text[sizeof req.text - 1] = '\0';
    uint32_t now = timer_get_ticks();

    /* Anything that CHANGES something is the administrator's: an unconfined
     * root. (Reading, and a probe that changes nothing, are any root's.) */
    bool changes = true;
    switch (req.op) {
    case NOVA_FW_OP_GET_RULE:
    case NOVA_FW_OP_GET_CONN:
        changes = false;
        break;
    case NOVA_FW_OP_PROBE:
        changes = req.probe_in[NOVA_FW_PI_COMMIT] != 0;
        break;
    default:
        break;
    }
    if (changes && !mac_is_admin(p)) {
        kernel_log("[FW] denied pid %d '%s': changing the firewall is the administrator's "
                   "(an unconfined root)\n", p->pid, p->name);
        return -E_PERM;
    }

    int rc = 0;
    switch (req.op) {
    case NOVA_FW_OP_ADD_RULE:
        rc = rust_fw_add_rule((const uint8_t*)req.text, (uint32_t)strlen(req.text), req.arg0);
        if (rc == 0) {
            kernel_log("[FW] rule added by pid %d: %s\n", p->pid, req.text);
        }
        break;
    case NOVA_FW_OP_DEL_RULE:
        rc = rust_fw_del_rule(req.arg0);
        if (rc == 0) {
            kernel_log("[FW] rule %d deleted by pid %d\n", (int)req.arg0, p->pid);
        }
        break;
    case NOVA_FW_OP_FLUSH_RULES:
        rc = rust_fw_flush_rules();
        if (rc == 0) {
            kernel_log("[FW] all rules removed by pid %d\n", p->pid);
        }
        break;
    case NOVA_FW_OP_SET_POLICY:
        rc = rust_fw_set_policy(req.arg0, req.arg1);
        if (rc == 0) {
            kernel_log("[FW] default policy %s set to %s by pid %d\n", req.arg0 == 0 ? "in" : "out",
                       req.arg1 ? "accept" : "drop", p->pid);
        }
        break;
    case NOVA_FW_OP_SET_ENABLED:
        rc = rust_fw_set_enabled(req.arg0);
        if (rc == 0) {
            kernel_log("[FW] firewall %s by pid %d\n", req.arg0 ? "enabled" : "DISABLED", p->pid);
        }
        break;
    case NOVA_FW_OP_LOCK:
        rust_fw_lock();
        kernel_log("[ OK ] Firewall locked by pid %d: no rule, policy or enable flag can change "
                   "until reboot\n", p->pid);
        break;
    case NOVA_FW_OP_FLUSH_CONNS:
        rust_fw_flush_conns();
        break;
    case NOVA_FW_OP_PROBE:
        rc = rust_fw_probe(req.probe_in, req.probe_out, now);
        break;
    case NOVA_FW_OP_GET_RULE:
        rc = rust_fw_get_rule(req.arg0, req.rule);
        break;
    case NOVA_FW_OP_GET_CONN:
        rc = rust_fw_get_conn(req.arg0, req.conn, now);
        break;
    default:
        return -E_INVAL;
    }
    memcpy((void*)user_ptr, &req, sizeof req);
    return rc;
}

/* ------------------------------------------------------------------
 * The wire self-test. The engine is tested on the host and by FWTEST.ELF
 * through the probe interface, but neither of those pushes a packet through
 * ip_handle_packet() - the one place where the inbound hook and the fragment
 * flag are actually wired in. This does, with three hand-built packets that
 * the baseline must refuse, and reads the counters the engine keeps.
 * ------------------------------------------------------------------ */

static void put32(uint8_t* p, uint32_t v) {
    p[0] = (uint8_t)(v >> 24); p[1] = (uint8_t)(v >> 16); p[2] = (uint8_t)(v >> 8); p[3] = (uint8_t)v;
}

/* Feeds one inbound IPv4 packet from 203.0.113.9 (a documentation address, never a real
 * host) through the real receive path. `flags_frag` is the raw IP flags/fragment field. */
static void wire_inject(uint8_t proto, uint16_t flags_frag, const uint8_t* transport, uint16_t tlen) {
    uint8_t pkt[IP_HEADER_LEN + 24];
    memset(pkt, 0, sizeof pkt);
    uint16_t total = (uint16_t)(IP_HEADER_LEN + tlen);
    pkt[0] = 0x45;
    pkt[2] = (uint8_t)(total >> 8); pkt[3] = (uint8_t)total;
    pkt[6] = (uint8_t)(flags_frag >> 8); pkt[7] = (uint8_t)flags_frag;
    pkt[8] = 64;
    pkt[9] = proto;
    put32(pkt + 12, 0xCB007109u); /* 203.0.113.9 */
    put32(pkt + 16, NET_OUR_IP);
    memcpy(pkt + IP_HEADER_LEN, transport, tlen);
    static const uint8_t mac[6] = {0x52, 0x54, 0x00, 0x12, 0x34, 0x02};
    ip_handle_packet(mac, pkt, total);
}

void fw_wire_selftest(void) {
    uint32_t b[24], a[24];
    int ok = 0;
    uint8_t t[20];

    /* 1. a fragment: counted as one, and dropped */
    memset(t, 0, sizeof t);
    rust_fw_info(b, timer_get_ticks());
    wire_inject(17, 0x2000 /* more fragments */, t, 16);
    rust_fw_info(a, timer_get_ticks());
    if (a[12] == b[12] + 1 && a[9] == b[9] + 1 && a[7] == b[7]) ok++;

    /* 2. a SYN nobody asked for: dropped by the policy, no flow created */
    memset(t, 0, sizeof t);
    t[0] = 0x9C; t[1] = 0x40; /* source port 40000 */
    t[2] = 0x00; t[3] = 0x16; /* destination port 22 */
    t[12] = 0x50;             /* data offset 5 */
    t[13] = 0x02;             /* SYN */
    rust_fw_info(b, timer_get_ticks());
    wire_inject(6, 0x4000, t, 20);
    rust_fw_info(a, timer_get_ticks());
    if (a[9] == b[9] + 1 && a[7] == b[7] && a[5] == b[5]) ok++;

    /* 3. a ping from outside the local network: not invited, dropped */
    memset(t, 0, sizeof t);
    t[0] = 8; /* echo request */
    t[5] = 1; t[7] = 1;
    rust_fw_info(b, timer_get_ticks());
    wire_inject(1, 0x4000, t, 8);
    rust_fw_info(a, timer_get_ticks());
    if (a[9] == b[9] + 1 && a[7] == b[7] && a[5] == b[5]) ok++;

    kernel_log("[ %s ] Firewall: the real inbound path refuses a fragment, an unsolicited SYN and a ping "
               "from outside the local network (%d/3)\n", ok == 3 ? "OK" : "FAIL", ok);
}
