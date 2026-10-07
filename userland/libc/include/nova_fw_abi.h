#ifndef NOVA_FW_ABI_H
#define NOVA_FW_ABI_H

/*
 * nova_fw_abi.h - Phase 88: the ABI of NovaOS's stateful firewall
 * (SYS_FW_INFO, SYS_FW_CTL), shared verbatim by the kernel
 * (kernel/net/firewall.c) and userland (novasys.h, fwtest). Dependency-free:
 * plain `unsigned int` / `char` (checked by the size assertions below).
 *
 * WHAT IT IS
 *
 * Every IP packet in either direction passes the filter in ip_send() and
 * ip_handle_packet(). The filter decides it from an ordered RULE list and from
 * CONNECTION STATE: a reply to something we sent is recognised and let in, a
 * packet that belongs to no flow and is not a valid way to start one is dropped
 * whatever the rules say. The engine is kernel/rust/firewall.rs; its header
 * comment is the full account, including what it does NOT do.
 *
 * Packet states a rule may match: new, established, related. INVALID packets
 * (a TCP segment with no connection, an impossible flag combination, an
 * unsolicited ICMP echo reply, a fragment, ...) are always dropped.
 *
 * Rule language (a line of FIREWALL.CFG, or the text of FW_OP_ADD_RULE):
 *
 *   rule <accept|drop> <in|out|any> [proto tcp|udp|icmp|any] [from CIDR] [to CIDR]
 *        [sport PORTS] [dport PORTS] [state new,established,related]
 *        [icmp-type N] [log]
 *
 * The baseline (and the shipped FIREWALL.CFG) is: nothing gets in unless we asked
 * for it (policy in drop), outbound is open, replies are allowed back in.
 *
 * WHO MAY DO WHAT. Reading (FW_INFO, FW_OP_GET_RULE, FW_OP_GET_CONN, a
 * non-committing FW_OP_PROBE) needs uid 0. Everything that CHANGES anything -
 * rules, policy, enabling, flushing tracked flows, locking, a committing probe -
 * needs the MAC administrator: an unconfined root. FW_OP_LOCK is one-way until
 * reboot: afterwards no rule, policy or enable flag can change.
 *
 * Every call returns 0 on success and a NEGATIVE errno on failure.
 */

#define NOVA_SYS_FW_INFO 73 /* EBX = nova_fw_info_t* (out) */
#define NOVA_SYS_FW_CTL  74 /* EBX = nova_fw_req_t*  (in/out) */

#define NOVA_FW_OP_ADD_RULE      1u /* arg0 = position (NOVA_FW_APPEND = last), text = the rule */
#define NOVA_FW_OP_DEL_RULE      2u /* arg0 = index */
#define NOVA_FW_OP_FLUSH_RULES   3u
#define NOVA_FW_OP_SET_POLICY    4u /* arg0 = direction (0 in, 1 out), arg1 = 1 accept / 0 drop */
#define NOVA_FW_OP_SET_ENABLED   5u /* arg0 = 1 on / 0 off (off: everything passes, nothing is tracked) */
#define NOVA_FW_OP_LOCK          6u /* one-way until reboot */
#define NOVA_FW_OP_FLUSH_CONNS   7u
#define NOVA_FW_OP_PROBE         8u /* probe_in -> probe_out */
#define NOVA_FW_OP_GET_RULE      9u /* arg0 = index -> rule[] */
#define NOVA_FW_OP_GET_CONN     10u /* arg0 = n (the n-th live tracked flow) -> conn[] */

#define NOVA_FW_APPEND 0xFFFFFFFFu

#define NOVA_FW_DIR_IN  0u
#define NOVA_FW_DIR_OUT 1u

#define NOVA_FW_PROTO_ICMP 1u
#define NOVA_FW_PROTO_TCP  6u
#define NOVA_FW_PROTO_UDP 17u

#define NOVA_FW_TCP_FIN 0x01u
#define NOVA_FW_TCP_SYN 0x02u
#define NOVA_FW_TCP_RST 0x04u
#define NOVA_FW_TCP_PSH 0x08u
#define NOVA_FW_TCP_ACK 0x10u
#define NOVA_FW_TCP_URG 0x20u

/* packet states, and what a probe reports */
#define NOVA_FW_ST_NEW         1u
#define NOVA_FW_ST_ESTABLISHED 2u
#define NOVA_FW_ST_RELATED     4u
#define NOVA_FW_ST_INVALID     8u

/* why a verdict was reached (probe_out[3]) */
#define NOVA_FW_R_RULE        1u
#define NOVA_FW_R_POLICY      2u
#define NOVA_FW_R_INVALID     3u
#define NOVA_FW_R_FRAGMENT    4u
#define NOVA_FW_R_UNSUPPORTED 5u
#define NOVA_FW_R_MALFORMED   6u
#define NOVA_FW_R_TABLE_FULL  7u
#define NOVA_FW_R_HALF_OPEN   8u
#define NOVA_FW_R_DISABLED    9u

/* why a packet was invalid (probe_out[4]) */
#define NOVA_FW_INV_FLAGS    1u
#define NOVA_FW_INV_NO_CONN  2u
#define NOVA_FW_INV_STATE    3u
#define NOVA_FW_INV_PORT0    4u
#define NOVA_FW_INV_ICMP     5u
#define NOVA_FW_INV_RELATED  6u

/* tracked-flow states (conn[1], probe_out[5]) */
#define NOVA_FW_TCP_SYN_SENT    1u
#define NOVA_FW_TCP_SYN_RECV    2u
#define NOVA_FW_TCP_ESTABLISHED 3u
#define NOVA_FW_TCP_FIN_WAIT    4u
#define NOVA_FW_TCP_LAST_ACK    5u
#define NOVA_FW_TCP_TIME_WAIT   6u
#define NOVA_FW_TCP_CLOSE       7u
#define NOVA_FW_UDP_UNREPLIED   1u
#define NOVA_FW_UDP_REPLIED     2u

/* indexes into probe_in[] */
#define NOVA_FW_PI_DIR        0  /* 0 inbound, 1 outbound */
#define NOVA_FW_PI_PROTO      1
#define NOVA_FW_PI_SRC        2  /* host order, like every IP in this kernel */
#define NOVA_FW_PI_DST        3
#define NOVA_FW_PI_SPORT      4
#define NOVA_FW_PI_DPORT      5
#define NOVA_FW_PI_FLAGS      6  /* TCP flags */
#define NOVA_FW_PI_ICMP_TYPE  7
#define NOVA_FW_PI_ICMP_CODE  8
#define NOVA_FW_PI_ID         9  /* ICMP echo identifier */
#define NOVA_FW_PI_FRAG       10 /* nonzero: this is (part of) a fragment */
#define NOVA_FW_PI_COMMIT     11 /* nonzero: change the real state (administrator only) */
#define NOVA_FW_PI_INNER_PROTO 12 /* an ICMP error's quoted packet; 0 = none */
#define NOVA_FW_PI_INNER_SRC  13
#define NOVA_FW_PI_INNER_DST  14
#define NOVA_FW_PI_INNER_SPORT 15
#define NOVA_FW_PI_INNER_DPORT 16
#define NOVA_FW_PI_LEN        17 /* transport length, for the byte counters */

/* indexes into probe_out[] */
#define NOVA_FW_PO_ACCEPT   0
#define NOVA_FW_PO_STATE    1
#define NOVA_FW_PO_RULE     2 /* the deciding rule, or 0xFFFFFFFF for none */
#define NOVA_FW_PO_REASON   3
#define NOVA_FW_PO_DETAIL   4
#define NOVA_FW_PO_CT_STATE 5

/* indexes into rule[] */
#define NOVA_FW_RL_DIR 0
#define NOVA_FW_RL_PROTO 1
#define NOVA_FW_RL_ACCEPT 2
#define NOVA_FW_RL_LOG 3
#define NOVA_FW_RL_SRC 4
#define NOVA_FW_RL_SRC_MASK 5
#define NOVA_FW_RL_DST 6
#define NOVA_FW_RL_DST_MASK 7
#define NOVA_FW_RL_SP_SET 8
#define NOVA_FW_RL_SP_LO 9
#define NOVA_FW_RL_SP_HI 10
#define NOVA_FW_RL_DP_SET 11
#define NOVA_FW_RL_DP_LO 12
#define NOVA_FW_RL_DP_HI 13
#define NOVA_FW_RL_MISC 14 /* states | icmp_set << 8 | icmp_type << 16 */
#define NOVA_FW_RL_HITS 15
#define NOVA_FW_RL_BYTES 16

/* indexes into conn[] */
#define NOVA_FW_CN_PROTO 0
#define NOVA_FW_CN_STATE 1
#define NOVA_FW_CN_ORIG_DIR 2
#define NOVA_FW_CN_REPLIED 3
#define NOVA_FW_CN_A_IP 4
#define NOVA_FW_CN_B_IP 5
#define NOVA_FW_CN_A_PORT 6
#define NOVA_FW_CN_B_PORT 7
#define NOVA_FW_CN_REMAINING 8 /* ticks until it expires */
#define NOVA_FW_CN_PKTS_ORIG 9
#define NOVA_FW_CN_PKTS_REPLY 10
#define NOVA_FW_CN_BYTES_ORIG 11
#define NOVA_FW_CN_BYTES_REPLY 12
#define NOVA_FW_CN_AGE 13
#define NOVA_FW_CN_PENDING 14

typedef struct {
    unsigned int enabled;
    unsigned int locked;
    unsigned int policy_in;   /* 1 accept, 0 drop */
    unsigned int policy_out;
    unsigned int rules;
    unsigned int conns;       /* live tracked flows */
    unsigned int expects;     /* live helper expectations */
    unsigned int accepted_in;
    unsigned int accepted_out;
    unsigned int dropped_in;
    unsigned int dropped_out;
    unsigned int invalid;
    unsigned int fragments;
    unsigned int table_full;
    unsigned int half_open;
    unsigned int new_conns;
    unsigned int related;
    unsigned int unsupported;
    unsigned int malformed;
    unsigned int logged;
    unsigned int max_rules;
    unsigned int max_conns;
    unsigned int tick_hz;
    unsigned int reserved;
} nova_fw_info_t;

typedef struct {
    unsigned int op;
    unsigned int arg0;
    unsigned int arg1;
    char text[128];
    unsigned int probe_in[18];
    unsigned int probe_out[8];
    unsigned int rule[18];
    unsigned int conn[16];
} nova_fw_req_t;

typedef char nova_fw_info_abi_check[(sizeof(nova_fw_info_t) == 24 * 4) ? 1 : -1];
typedef char nova_fw_req_abi_check[(sizeof(nova_fw_req_t) == 12 + 128 + (18 + 8 + 18 + 16) * 4) ? 1 : -1];

#endif
