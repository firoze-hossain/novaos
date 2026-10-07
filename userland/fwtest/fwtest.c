/*
 * fwtest.c - Phase 88: the in-OS conformance test for the stateful firewall
 * (SYS_FW_INFO / SYS_FW_CTL and the filter in ip_send()/ip_handle_packet()).
 * A ring-3 program against the real kernel, run as the unconfined root - the
 * firewall's administrator. kernel/task/exec_trust_demo.c runs it at boot and
 * tools/python/test_runner.py checks its "[fwtest] ok:" lines.
 *
 * It is the in-OS counterpart of kernel/rust/firewall.rs's host tests. Those
 * attack the engine itself - every TCP state, the parsers, the limits, a
 * reference model, 79 injected bugs. This proves the whole stack around it:
 * the syscalls, who may call them, rule management on the live table, and -
 * the point of the whole phase - REAL TRAFFIC through the real network stack.
 *
 * The scenarios:
 *
 *  - API AND PRIVILEGE: the shipped FIREWALL.CFG was loaded (its two rules are
 *    readable); hostile pointers fail cleanly; a logged-in non-root user is
 *    refused everything, and refused BEFORE its pointer is examined.
 *  - THE ENGINE ON THE REAL KERNEL, by probe: with probes that change nothing, a
 *    packet nobody asked for is dropped, an ACK with no connection is INVALID,
 *    impossible flag combinations, fragments and unknown protocols are refused,
 *    and a probe leaves no trace.
 *  - STATEFUL FLOWS, by committing probes that really change the table: a TCP
 *    connection through every state to TIME_WAIT and a reused tuple; a
 *    connection started from outside only after a rule invites it; UDP replies;
 *    the TFTP helper (a reply from a port other than the one asked); ICMP echo
 *    accepted exactly once; an ICMP error as RELATED; the connection table
 *    filling up and refusing the 257th flow without disturbing the others.
 *  - RULES ON THE LIVE TABLE: insert, append, delete, first-match-wins, hit and
 *    byte counters, the policy, the on/off switch, every kind of bad rule text,
 *    the 32-rule limit.
 *  - REAL TRAFFIC: a real TFTP download from the gateway works through the firewall
 *    and is tracked; a rule dropping the outbound request stops it, and removing
 *    the rule restores it; a rule dropping the ANSWERS stops it, with the request
 *    visibly sent. Where the emulated network does not answer at all, the same
 *    download is tried with the firewall OFF as a control, and the group is
 *    skipped only if that fails just as badly.
 *  - THE LOCK, last because it cannot be undone until reboot (so this whole
 *    program runs early in the boot, while the emulated network still answers).
 *
 * Every failure prints "[fwtest] FAIL: ..." (the uppercase word also trips
 * test_runner.py's global no-fail scan); passing output avoids the words that
 * scan looks for.
 */
#include <errno.h>
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failures;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 30) { \
            printf("[fwtest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

#define US    0x0A00020Fu /* 10.0.2.15, the kernel's own address */
#define GW    0x0A000202u /* 10.0.2.2 */
#define PEER  0xCB007109u /* 203.0.113.9 (documentation range: never a real host) */
#define PEER2 0xC6336407u /* 198.51.100.7 */
#define NONE  0xFFFFFFFFu

/* ---- helpers --------------------------------------------------------------------- */

typedef struct {
    int rc;
    unsigned int accept, state, rule, reason, detail, ct;
} verdict_t;

static int ctl(nova_fw_req_t* r) {
    return sys_fw_ctl(r);
}

static int simple(unsigned int op, unsigned int a0, unsigned int a1) {
    nova_fw_req_t r;
    memset(&r, 0, sizeof r);
    r.op = op;
    r.arg0 = a0;
    r.arg1 = a1;
    return ctl(&r);
}

static int add_rule(unsigned int pos, const char* text) {
    nova_fw_req_t r;
    memset(&r, 0, sizeof r);
    r.op = NOVA_FW_OP_ADD_RULE;
    r.arg0 = pos;
    strncpy(r.text, text, sizeof r.text - 1);
    return ctl(&r);
}

static int get_rule(unsigned int i, unsigned int out[18]) {
    nova_fw_req_t r;
    memset(&r, 0, sizeof r);
    r.op = NOVA_FW_OP_GET_RULE;
    r.arg0 = i;
    int rc = ctl(&r);
    memcpy(out, r.rule, sizeof r.rule);
    return rc;
}

static int get_conn(unsigned int n, unsigned int out[16]) {
    nova_fw_req_t r;
    memset(&r, 0, sizeof r);
    r.op = NOVA_FW_OP_GET_CONN;
    r.arg0 = n;
    int rc = ctl(&r);
    memcpy(out, r.conn, sizeof r.conn);
    return rc;
}

static nova_fw_info_t info(void) {
    nova_fw_info_t i;
    memset(&i, 0, sizeof i);
    (void)sys_fw_info(&i);
    return i;
}

/* One probe. `commit` makes it change the real tracked state. */
static verdict_t probe_in(const unsigned int in[18]) {
    nova_fw_req_t r;
    memset(&r, 0, sizeof r);
    r.op = NOVA_FW_OP_PROBE;
    memcpy(r.probe_in, in, sizeof r.probe_in);
    verdict_t v;
    v.rc = ctl(&r);
    v.accept = r.probe_out[NOVA_FW_PO_ACCEPT];
    v.state = r.probe_out[NOVA_FW_PO_STATE];
    v.rule = r.probe_out[NOVA_FW_PO_RULE];
    v.reason = r.probe_out[NOVA_FW_PO_REASON];
    v.detail = r.probe_out[NOVA_FW_PO_DETAIL];
    v.ct = r.probe_out[NOVA_FW_PO_CT_STATE];
    return v;
}

static verdict_t probe(unsigned dir, unsigned proto, unsigned src, unsigned dst, unsigned sport,
                       unsigned dport, unsigned flags, unsigned commit) {
    unsigned int in[18];
    memset(in, 0, sizeof in);
    in[NOVA_FW_PI_DIR] = dir;
    in[NOVA_FW_PI_PROTO] = proto;
    in[NOVA_FW_PI_SRC] = src;
    in[NOVA_FW_PI_DST] = dst;
    in[NOVA_FW_PI_SPORT] = sport;
    in[NOVA_FW_PI_DPORT] = dport;
    in[NOVA_FW_PI_FLAGS] = flags;
    in[NOVA_FW_PI_COMMIT] = commit;
    in[NOVA_FW_PI_LEN] = 100;
    return probe_in(in);
}

static verdict_t probe_icmp(unsigned dir, unsigned src, unsigned dst, unsigned type, unsigned id, unsigned commit) {
    unsigned int in[18];
    memset(in, 0, sizeof in);
    in[NOVA_FW_PI_DIR] = dir;
    in[NOVA_FW_PI_PROTO] = NOVA_FW_PROTO_ICMP;
    in[NOVA_FW_PI_SRC] = src;
    in[NOVA_FW_PI_DST] = dst;
    in[NOVA_FW_PI_ICMP_TYPE] = type;
    in[NOVA_FW_PI_ID] = id;
    in[NOVA_FW_PI_COMMIT] = commit;
    in[NOVA_FW_PI_LEN] = 64;
    return probe_in(in);
}

static verdict_t probe_icmp_error(unsigned type, unsigned src, unsigned dst, unsigned iproto,
                                  unsigned isrc, unsigned idst, unsigned isport, unsigned idport) {
    unsigned int in[18];
    memset(in, 0, sizeof in);
    in[NOVA_FW_PI_DIR] = NOVA_FW_DIR_IN;
    in[NOVA_FW_PI_PROTO] = NOVA_FW_PROTO_ICMP;
    in[NOVA_FW_PI_SRC] = src;
    in[NOVA_FW_PI_DST] = dst;
    in[NOVA_FW_PI_ICMP_TYPE] = type;
    in[NOVA_FW_PI_INNER_PROTO] = iproto;
    in[NOVA_FW_PI_INNER_SRC] = isrc;
    in[NOVA_FW_PI_INNER_DST] = idst;
    in[NOVA_FW_PI_INNER_SPORT] = isport;
    in[NOVA_FW_PI_INNER_DPORT] = idport;
    in[NOVA_FW_PI_COMMIT] = 1;
    return probe_in(in);
}

/* Finds the live tracked flow with this tuple, in either order. */
static int find_conn(unsigned proto, unsigned a_ip, unsigned a_port, unsigned b_ip, unsigned b_port, unsigned int out[16]) {
    for (unsigned n = 0; n < 300; n++) {
        if (get_conn(n, out) != 0) return 0;
        if (out[NOVA_FW_CN_PROTO] == proto && out[NOVA_FW_CN_A_IP] == a_ip && out[NOVA_FW_CN_A_PORT] == a_port &&
            out[NOVA_FW_CN_B_IP] == b_ip && out[NOVA_FW_CN_B_PORT] == b_port) {
            return 1;
        }
    }
    return 0;
}

static int now_s(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    return t.hour * 3600 + t.minute * 60 + t.second;
}

#define TCP 6u
#define UDP 17u
#define IN  NOVA_FW_DIR_IN
#define OUT NOVA_FW_DIR_OUT
#define SYN NOVA_FW_TCP_SYN
#define ACK NOVA_FW_TCP_ACK
#define FIN NOVA_FW_TCP_FIN
#define RST NOVA_FW_TCP_RST
#define PSH NOVA_FW_TCP_PSH

/* ---- group 1: the API and who may use it -------------------------------------------- */

/* Runs as FWCONF.ELF: this same program installed under another name, bound by the
 * profile FWCONF.MAC (which allows every syscall). It is ROOT, with every capability,
 * and still must not change the firewall: that is the administrator's - an
 * UNCONFINED root. Reading, and a probe that changes nothing, are any root's. */
static void confined_child(void) {
    failures = 0;
    CHECK(sys_getuid() == 0, "uid %u", sys_getuid());
    nova_fw_info_t before = info();
    unsigned int r[18];
    CHECK(sys_fw_info(&before) == 0, "a confined root may read the firewall");
    CHECK(get_rule(0, r) == 0, "and its rules");
    CHECK(probe(IN, TCP, PEER, US, 5555, 22, SYN, 0).rc == 0, "and ask what a packet would get (a probe that changes nothing)");
    CHECK(probe(IN, TCP, PEER, US, 5555, 22, SYN, 1).rc == -EPERM, "but not a probe that would change the real state");
    CHECK(add_rule(0, "rule accept in") == -EPERM, "may not add a rule");
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 0, 0) == -EPERM, "nor delete one");
    CHECK(simple(NOVA_FW_OP_FLUSH_RULES, 0, 0) == -EPERM, "nor flush them");
    CHECK(simple(NOVA_FW_OP_SET_POLICY, 0, 1) == -EPERM, "nor change the policy");
    CHECK(simple(NOVA_FW_OP_SET_ENABLED, 0, 0) == -EPERM, "nor switch it off");
    CHECK(simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0) == -EPERM, "nor forget the tracked flows");
    CHECK(simple(NOVA_FW_OP_LOCK, 0, 0) == -EPERM, "nor lock it");
    nova_fw_info_t after = info();
    CHECK(after.rules == before.rules && after.enabled == before.enabled && after.locked == before.locked &&
          after.policy_in == before.policy_in && after.policy_out == before.policy_out, "and nothing changed");
    sys_exit(failures);
}

static void nonroot_child(void) {
    failures = 0;
    CHECK(sys_login("persisted", "persisted-pw") == 0, "login");
    CHECK(sys_getuid() == 700, "uid %u", sys_getuid());
    nova_fw_info_t i;
    CHECK(sys_fw_info(&i) == -EPERM, "a non-root user must not read the firewall");
    nova_fw_req_t r;
    memset(&r, 0, sizeof r);
    r.op = NOVA_FW_OP_GET_RULE;
    CHECK(sys_fw_ctl(&r) == -EPERM, "nor its rules");
    CHECK(add_rule(0, "rule accept in") == -EPERM, "nor change them");
    CHECK(simple(NOVA_FW_OP_SET_POLICY, 0, 1) == -EPERM, "nor its policy");
    CHECK(simple(NOVA_FW_OP_SET_ENABLED, 0, 0) == -EPERM, "nor switch it off");
    CHECK(simple(NOVA_FW_OP_LOCK, 0, 0) == -EPERM, "nor lock it");
    CHECK(probe(IN, TCP, PEER, US, 1, 2, SYN, 0).rc == -EPERM, "nor probe it");
    /* privilege is decided BEFORE the pointer is looked at: a hostile pointer from a
     * non-root caller is a permission error, not a fault, and tells it nothing */
    CHECK(sys_fw_info((nova_fw_info_t*)0) == -EPERM, "a NULL pointer from a non-root user");
    CHECK(sys_fw_ctl((nova_fw_req_t*)0xC0000000u) == -EPERM, "a kernel pointer from a non-root user");
    sys_exit(failures);
}

static void test_api_and_privilege(void) {
    int f0 = failures;
    nova_fw_info_t i = info();
    CHECK(i.enabled == 1 && i.locked == 0, "enabled %u locked %u", i.enabled, i.locked);
    CHECK(i.policy_in == 0 && i.policy_out == 1, "policy in=%u out=%u: nothing in unasked, outbound open", i.policy_in, i.policy_out);
    CHECK(i.rules == 2, "FIREWALL.CFG has two rules, the firewall has %u", i.rules);
    CHECK(i.max_rules == 32 && i.max_conns == 256 && i.tick_hz == 100, "limits %u %u %u", i.max_rules, i.max_conns, i.tick_hz);

    /* the rules the shipped file declares, read back */
    unsigned int r[18];
    CHECK(get_rule(0, r) == 0, "rule 0");
    CHECK(r[NOVA_FW_RL_ACCEPT] == 1 && r[NOVA_FW_RL_DIR] == IN && r[NOVA_FW_RL_PROTO] == 0 &&
          (r[NOVA_FW_RL_MISC] & 0xFF) == (NOVA_FW_ST_ESTABLISHED | NOVA_FW_ST_RELATED),
          "rule 0 is 'accept in state established,related': accept %u dir %u proto %u states %u",
          r[NOVA_FW_RL_ACCEPT], r[NOVA_FW_RL_DIR], r[NOVA_FW_RL_PROTO], r[NOVA_FW_RL_MISC] & 0xFF);
    CHECK(get_rule(1, r) == 0, "rule 1");
    CHECK(r[NOVA_FW_RL_ACCEPT] == 1 && r[NOVA_FW_RL_PROTO] == 1 && r[NOVA_FW_RL_SRC] == 0x0A000200u &&
          r[NOVA_FW_RL_SRC_MASK] == 0xFFFFFF00u && ((r[NOVA_FW_RL_MISC] >> 8) & 1) == 1 && ((r[NOVA_FW_RL_MISC] >> 16) & 0xFF) == 8,
          "rule 1 is 'accept in proto icmp icmp-type 8 from 10.0.2.0/24 state new'");
    CHECK(get_rule(2, r) == -ENOENT, "there is no rule 2");
    CHECK(get_rule(1000, r) == -ENOENT, "or 1000");

    /* argument validation, and hostile pointers */
    nova_fw_req_t bad;
    memset(&bad, 0, sizeof bad);
    bad.op = 99;
    CHECK(sys_fw_ctl(&bad) == -EINVAL, "an unknown operation");
    bad.op = 0;
    CHECK(sys_fw_ctl(&bad) == -EINVAL, "operation 0");
    void* nowhere[] = { (void*)0, (void*)0x00100000, (void*)0x50000000, (void*)0xFFFFFFF8, (void*)0xC0000000 };
    for (unsigned k = 0; k < sizeof nowhere / sizeof nowhere[0]; k++) {
        CHECK(sys_fw_info((nova_fw_info_t*)nowhere[k]) == -EFAULT, "info with pointer %u", k);
        CHECK(sys_fw_ctl((nova_fw_req_t*)nowhere[k]) == -EFAULT, "ctl with pointer %u", k);
    }
    /* a rule text that is not NUL-terminated within its field must not run off the end */
    nova_fw_req_t big;
    memset(&big, 'x', sizeof big);
    big.op = NOVA_FW_OP_ADD_RULE;
    big.arg0 = NOVA_FW_APPEND;
    CHECK(ctl(&big) == -EINVAL, "an unterminated rule text is refused, not read past");
    CHECK(info().rules == i.rules, "and added nothing");

    int pid = sys_fork();
    if (pid == 0) nonroot_child();
    CHECK(pid > 0, "fork failed");
    int code = -1;
    if (pid > 0) code = sys_wait(pid);
    CHECK(code == 0, "the non-root child reported %d failed checks", code);
    if (failures == f0) printf("[fwtest] ok: API - the shipped rules read back, hostile pointers fail cleanly, and a non-root user is refused everything, before its pointer is examined\n");
}

/* ---- group 2: the engine on the real kernel, with probes that change nothing ------------- */

static void test_probes_change_nothing(void) {
    int f0 = failures;
    nova_fw_info_t before = info();
    verdict_t v;

    v = probe(IN, TCP, PEER, US, 5555, 22, SYN, 0);
    CHECK(v.rc == 0 && v.accept == 0 && v.state == NOVA_FW_ST_NEW && v.reason == NOVA_FW_R_POLICY && v.rule == NONE,
          "an unsolicited SYN: accept %u state %u reason %u rule %#x", v.accept, v.state, v.reason, v.rule);
    v = probe(IN, TCP, PEER, US, 80, 40999, ACK, 0);
    CHECK(v.accept == 0 && v.state == NOVA_FW_ST_INVALID && v.reason == NOVA_FW_R_INVALID && v.detail == NOVA_FW_INV_NO_CONN,
          "an ACK with no connection: reason %u detail %u", v.reason, v.detail);
    v = probe(IN, TCP, PEER, US, 80, 40999, SYN | NOVA_FW_TCP_FIN, 0);
    CHECK(v.accept == 0 && v.detail == NOVA_FW_INV_FLAGS, "SYN+FIN: detail %u", v.detail);
    v = probe(OUT, TCP, US, PEER, 40124, 80, 0, 0);
    CHECK(v.accept == 0 && v.detail == NOVA_FW_INV_FLAGS, "no flags at all, even outbound: detail %u", v.detail);
    v = probe(OUT, TCP, US, PEER, 40124, 80, FIN, 0);
    CHECK(v.accept == 0 && v.detail == NOVA_FW_INV_FLAGS, "FIN without ACK");
    v = probe(OUT, TCP, US, PEER, 40124, 80, SYN, 0);
    CHECK(v.accept == 1 && v.state == NOVA_FW_ST_NEW && v.reason == NOVA_FW_R_POLICY && v.ct == NOVA_FW_TCP_SYN_SENT,
          "an outbound SYN: accept %u state %u ct %u", v.accept, v.state, v.ct);
    v = probe_icmp(IN, PEER, US, 0, 0x7000, 0);
    CHECK(v.accept == 0 && v.detail == NOVA_FW_INV_NO_CONN, "an echo reply nobody asked for");
    v = probe_icmp(IN, GW, US, 8, 0x7000, 0);
    CHECK(v.accept == 1 && v.state == NOVA_FW_ST_NEW && v.rule == 1, "a ping from the local network is invited by rule 1: rule %#x", v.rule);
    v = probe_icmp(IN, PEER, US, 8, 0x7000, 0);
    CHECK(v.accept == 0 && v.reason == NOVA_FW_R_POLICY, "a ping from outside it is not");
    unsigned int frag[18];
    memset(frag, 0, sizeof frag);
    frag[NOVA_FW_PI_PROTO] = UDP;
    frag[NOVA_FW_PI_SRC] = PEER;
    frag[NOVA_FW_PI_DST] = US;
    frag[NOVA_FW_PI_SPORT] = 1;
    frag[NOVA_FW_PI_DPORT] = 2;
    frag[NOVA_FW_PI_FRAG] = 1;
    v = probe_in(frag);
    CHECK(v.accept == 0 && v.reason == NOVA_FW_R_FRAGMENT, "a fragment: reason %u", v.reason);
    v = probe(IN, 47, PEER, US, 0, 0, 0, 0);
    CHECK(v.accept == 0 && v.reason == NOVA_FW_R_UNSUPPORTED, "a protocol this stack does not handle: reason %u", v.reason);
    v = probe(OUT, UDP, US, PEER, 1000, 0, 0, 0);
    CHECK(v.accept == 0 && v.detail == NOVA_FW_INV_PORT0, "UDP to port 0");

    nova_fw_info_t after = info();
    CHECK(memcmp(&before, &after, sizeof before) == 0, "probes that change nothing must leave the counters, flows and rules alone");
    if (failures == f0) printf("[fwtest] ok: probes - an unsolicited SYN, an ACK with no connection, impossible flags, fragments and unknown protocols are refused; a ping is let in only from the local network; and a probe leaves no trace\n");
}

/* ---- group 3: stateful flows through the real table --------------------------------------- */

static void test_tcp_flow(void) {
    int f0 = failures;
    verdict_t v;
    const unsigned SP = 40123, DP = 80;
    unsigned int c[16];

    v = probe(OUT, TCP, US, PEER, SP, DP, SYN, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_NEW && v.ct == NOVA_FW_TCP_SYN_SENT, "SYN");
    v = probe(IN, TCP, PEER, US, 81, SP, SYN | ACK, 1);
    CHECK(!v.accept, "a SYN-ACK from the wrong port is not the reply");
    v = probe(IN, TCP, PEER2, US, DP, SP, SYN | ACK, 1);
    CHECK(!v.accept, "nor from the wrong host");
    v = probe(IN, TCP, PEER, US, DP, SP, ACK, 1);
    CHECK(!v.accept && v.detail == NOVA_FW_INV_STATE, "a bare ACK cannot answer a SYN");
    v = probe(IN, TCP, PEER, US, DP, SP, SYN | ACK, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_ESTABLISHED && v.rule == 0 && v.ct == NOVA_FW_TCP_SYN_RECV,
          "the real SYN-ACK: accept %u state %u rule %u ct %u", v.accept, v.state, v.rule, v.ct);
    v = probe(OUT, TCP, US, PEER, SP, DP, ACK, 1);
    CHECK(v.accept && v.ct == NOVA_FW_TCP_ESTABLISHED, "the handshake completes");
    for (int k = 0; k < 4; k++) {
        CHECK(probe(IN, TCP, PEER, US, DP, SP, ACK | PSH, 1).accept, "data in");
        CHECK(probe(OUT, TCP, US, PEER, SP, DP, ACK, 1).accept, "ack out");
    }
    CHECK(find_conn(TCP, US, SP, PEER, DP, c), "the flow is in the connection table");
    CHECK(c[NOVA_FW_CN_STATE] == NOVA_FW_TCP_ESTABLISHED && c[NOVA_FW_CN_REPLIED] == 1 && c[NOVA_FW_CN_ORIG_DIR] == OUT,
          "state %u replied %u orig %u", c[NOVA_FW_CN_STATE], c[NOVA_FW_CN_REPLIED], c[NOVA_FW_CN_ORIG_DIR]);
    CHECK(c[NOVA_FW_CN_PKTS_ORIG] == 6 && c[NOVA_FW_CN_PKTS_REPLY] == 5, "packet counts %u / %u (SYN, ACK + 4 data acks out; SYN-ACK + 4 data in)",
          c[NOVA_FW_CN_PKTS_ORIG], c[NOVA_FW_CN_PKTS_REPLY]);
    CHECK(c[NOVA_FW_CN_REMAINING] > 300000, "an established flow lives for an hour (%u ticks left)", c[NOVA_FW_CN_REMAINING]);

    /* a SYN is not valid inside an established flow */
    v = probe(OUT, TCP, US, PEER, SP, DP, SYN, 1);
    CHECK(!v.accept && v.detail == NOVA_FW_INV_STATE, "a SYN inside an established connection");

    /* the close: we send the first FIN */
    v = probe(OUT, TCP, US, PEER, SP, DP, FIN | ACK, 1);
    CHECK(v.accept && v.ct == NOVA_FW_TCP_FIN_WAIT, "our FIN");
    CHECK(probe(IN, TCP, PEER, US, DP, SP, ACK, 1).accept, "its ACK");
    v = probe(IN, TCP, PEER, US, DP, SP, FIN | ACK, 1);
    CHECK(v.accept && v.ct == NOVA_FW_TCP_LAST_ACK, "its FIN");
    v = probe(OUT, TCP, US, PEER, SP, DP, ACK, 1);
    CHECK(v.accept && v.ct == NOVA_FW_TCP_TIME_WAIT, "our final ACK");
    /* the tuple can be used again at once */
    v = probe(OUT, TCP, US, PEER, SP, DP, SYN, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_NEW && v.ct == NOVA_FW_TCP_SYN_SENT, "a fresh SYN reuses the finished flow's tuple");
    /* and a reset ends it */
    CHECK(probe(IN, TCP, PEER, US, DP, SP, RST | ACK, 1).ct == NOVA_FW_TCP_CLOSE, "a refused connection closes");
    if (failures == f0) printf("[fwtest] ok: TCP - a connection through SYN_SENT, SYN_RECV, ESTABLISHED, FIN_WAIT, LAST_ACK and TIME_WAIT, replies only from the exact tuple, and a reused tuple\n");
}

static void test_inbound_needs_an_invitation(void) {
    int f0 = failures;
    verdict_t v;
    nova_fw_info_t b = info();
    v = probe(IN, TCP, PEER, US, 50000, 2222, SYN, 1);
    CHECK(!v.accept && v.state == NOVA_FW_ST_NEW && v.reason == NOVA_FW_R_POLICY, "no rule invites port 2222");
    CHECK(info().conns == b.conns, "a dropped SYN creates no flow");
    CHECK(add_rule(0, "rule accept in proto tcp dport 2222 state new") == 0, "adding the invitation");
    v = probe(IN, TCP, PEER, US, 50000, 2222, SYN, 1);
    CHECK(v.accept && v.rule == 0 && v.ct == NOVA_FW_TCP_SYN_SENT, "now it is let in by rule 0");
    v = probe(IN, TCP, PEER, US, 50000, 2223, SYN, 1);
    CHECK(!v.accept, "but only that port");
    v = probe(OUT, TCP, US, PEER, 2222, 50000, SYN | ACK, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_ESTABLISHED && v.ct == NOVA_FW_TCP_SYN_RECV, "our SYN-ACK is a reply");
    v = probe(IN, TCP, PEER, US, 50000, 2222, ACK, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_ESTABLISHED && v.rule == 1 && v.ct == NOVA_FW_TCP_ESTABLISHED,
          "its ACK is established traffic (the baseline rule, now index 1): rule %u", v.rule);
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 0, 0) == 0, "withdrawing the invitation");
    CHECK(probe(IN, TCP, PEER, US, 50000, 2222, ACK | PSH, 1).accept, "the connection that was let in keeps working");
    CHECK(!probe(IN, TCP, PEER2, US, 50001, 2222, SYN, 1).accept, "a new one does not");
    if (failures == f0) printf("[fwtest] ok: invitation - a connection from outside is dropped until a rule invites it, only on that port, its handshake completes, and it survives the rule being withdrawn\n");
}

static void test_udp_icmp_and_helper(void) {
    int f0 = failures;
    verdict_t v;
    unsigned int c[16];

    v = probe(OUT, UDP, US, PEER, 41000, 53, 0, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_NEW && v.ct == NOVA_FW_UDP_UNREPLIED, "a UDP query");
    CHECK(!probe(IN, UDP, PEER, US, 54, 41000, 0, 1).accept, "an answer from the wrong port");
    CHECK(!probe(IN, UDP, PEER2, US, 53, 41000, 0, 1).accept, "or the wrong host");
    v = probe(IN, UDP, PEER, US, 53, 41000, 0, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_ESTABLISHED && v.ct == NOVA_FW_UDP_REPLIED, "the real answer");
    CHECK(find_conn(UDP, US, 41000, PEER, 53, c) && c[NOVA_FW_CN_STATE] == NOVA_FW_UDP_REPLIED, "tracked as replied");

    /* the TFTP helper: the server answers from a port of its own choosing */
    nova_fw_info_t b = info();
    v = probe(OUT, UDP, US, PEER, 41001, 69, 0, 1);
    CHECK(v.accept, "a TFTP request");
    CHECK(info().expects == b.expects + 1, "the helper now expects an answer (%u -> %u)", b.expects, info().expects);
    CHECK(!probe(IN, UDP, PEER2, US, 33333, 41001, 0, 1).accept, "not from another host");
    CHECK(!probe(IN, UDP, PEER, US, 33333, 41002, 0, 1).accept, "not to another port");
    v = probe(IN, UDP, PEER, US, 33333, 41001, 0, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_RELATED, "the answer from port 33333 is RELATED, state %u", v.state);
    CHECK(info().expects == b.expects, "and the expectation is used up");
    v = probe(IN, UDP, PEER, US, 33333, 41001, 0, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_ESTABLISHED, "after that it is an ordinary flow, even before we answer");
    CHECK(find_conn(UDP, PEER, 33333, US, 41001, c) && c[NOVA_FW_CN_ORIG_DIR] == IN, "tracked, with the server as the initiator");
    CHECK(!probe(IN, UDP, PEER, US, 33334, 41001, 0, 1).accept, "a different server port gets no such treatment");

    /* ICMP echo: accepted once per request */
    v = probe_icmp(OUT, US, PEER, 8, 0x7001, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_NEW, "an echo request");
    v = probe_icmp(IN, PEER, US, 0, 0x7001, 1);
    CHECK(v.accept && v.state == NOVA_FW_ST_ESTABLISHED, "its reply");
    v = probe_icmp(IN, PEER, US, 0, 0x7001, 1);
    CHECK(!v.accept && v.detail == NOVA_FW_INV_NO_CONN, "a second reply to one request");
    CHECK(!probe_icmp(IN, PEER, US, 0, 0x7002, 1).accept, "a reply with another identifier");
    CHECK(probe_icmp(OUT, US, PEER, 8, 0x7001, 1).accept && probe_icmp(OUT, US, PEER, 8, 0x7001, 1).accept, "two more requests ...");
    CHECK(probe_icmp(IN, PEER, US, 0, 0x7001, 1).accept && probe_icmp(IN, PEER, US, 0, 0x7001, 1).accept, "... allow two replies ...");
    CHECK(!probe_icmp(IN, PEER, US, 0, 0x7001, 1).accept, "... and no third");

    /* an ICMP error is RELATED to the flow it is about, and only to a flow we have */
    CHECK(probe(OUT, UDP, US, PEER, 41003, 9, 0, 1).accept, "a datagram to a closed port");
    v = probe_icmp_error(3, GW, US, UDP, US, PEER, 41003, 9);
    CHECK(v.accept && v.state == NOVA_FW_ST_RELATED && v.rule == 0, "its port-unreachable is RELATED: state %u", v.state);
    v = probe_icmp_error(3, GW, US, UDP, US, PEER, 41999, 9);
    CHECK(!v.accept && v.detail == NOVA_FW_INV_RELATED, "an error about a flow we do not have");
    v = probe_icmp_error(11, GW, US, UDP, US, PEER2, 41003, 9);
    CHECK(!v.accept, "or about someone else's");
    if (failures == f0) printf("[fwtest] ok: UDP, ICMP and the helper - replies only from the exact tuple, a TFTP answer from another port is RELATED, an echo is answered once per request, ICMP errors only about flows we have\n");
}

static void test_table_limit(void) {
    int f0 = failures;
    CHECK(simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0) == 0, "flushing the tracked flows");
    CHECK(info().conns == 0, "none left");
    int accepted = 0, full_at = -1;
    for (unsigned i = 0; i < 300; i++) {
        verdict_t v = probe(OUT, TCP, US, PEER, 20000 + i, 80, SYN, 1);
        if (v.accept) {
            accepted++;
        } else {
            CHECK(v.reason == NOVA_FW_R_TABLE_FULL, "refused for reason %u", v.reason);
            if (full_at < 0) full_at = (int)i;
        }
    }
    CHECK(accepted == 256 && full_at == 256, "exactly 256 flows fit (%d accepted, first refusal at %d)", accepted, full_at);
    nova_fw_info_t i = info();
    CHECK(i.conns == 256 && i.table_full >= 44, "the table is full (%u flows, %u refusals counted)", i.conns, i.table_full);
    /* a full table refuses NEW flows, not established ones */
    verdict_t v = probe(IN, TCP, PEER, US, 80, 20000, SYN | ACK, 1);
    CHECK(v.accept && v.ct == NOVA_FW_TCP_SYN_RECV, "an existing flow still progresses");
    CHECK(!probe(OUT, UDP, US, PEER, 5000, 53, 0, 1).accept, "a UDP flow needs room too");
    CHECK(!probe_icmp(OUT, US, PEER, 8, 0x7100, 1).accept, "so does a ping");
    CHECK(simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0) == 0 && info().conns == 0, "flushed again");
    CHECK(probe(OUT, TCP, US, PEER, 20000, 80, SYN, 1).accept, "and there is room");
    simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0);
    if (failures == f0) printf("[fwtest] ok: table limit - exactly 256 flows fit, the 257th is refused (TCP, UDP and ICMP alike), established flows keep working, and flushing makes room\n");
}

/* ---- group 4: rules on the live table ------------------------------------------------------ */

static void test_rule_management(void) {
    int f0 = failures;
    nova_fw_info_t b = info();
    unsigned int r[18];
    verdict_t v;
    unsigned base = b.rules;

    CHECK(add_rule(0, "rule drop out proto udp dport 9") == 0, "a rule at the front");
    CHECK(info().rules == base + 1, "counted");
    CHECK(get_rule(0, r) == 0 && r[NOVA_FW_RL_ACCEPT] == 0 && r[NOVA_FW_RL_DIR] == OUT && r[NOVA_FW_RL_PROTO] == 17 &&
          r[NOVA_FW_RL_DP_SET] == 1 && r[NOVA_FW_RL_DP_LO] == 9 && r[NOVA_FW_RL_DP_HI] == 9, "and read back");
    v = probe(OUT, UDP, US, PEER, 5000, 9, 0, 1);
    CHECK(!v.accept && v.reason == NOVA_FW_R_RULE && v.rule == 0, "it drops what it names");
    CHECK(get_rule(0, r) == 0 && r[NOVA_FW_RL_HITS] == 1 && r[NOVA_FW_RL_BYTES] == 100, "and counts the hit and its bytes: %u / %u", r[NOVA_FW_RL_HITS], r[NOVA_FW_RL_BYTES]);
    v = probe(OUT, UDP, US, PEER, 5000, 10, 0, 1);
    CHECK(v.accept && v.reason == NOVA_FW_R_POLICY, "and nothing else");
    CHECK(add_rule(NOVA_FW_APPEND, "rule accept out proto udp dport 9") == 0, "an accept for the same thing, last");
    v = probe(OUT, UDP, US, PEER, 5000, 9, 0, 1);
    CHECK(!v.accept && v.rule == 0, "the first match wins");
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 0, 0) == 0, "deleting the drop");
    v = probe(OUT, UDP, US, PEER, 5000, 9, 0, 1);
    CHECK(v.accept && v.reason == NOVA_FW_R_RULE && v.rule == base, "the later accept now decides: rule %u", v.rule);
    CHECK(simple(NOVA_FW_OP_DEL_RULE, base, 0) == 0 && info().rules == base, "back to the shipped rules");

    /* bad rules are refused whole */
    const char* bad[] = { "bogus", "rule", "rule accept in dport 22", "rule accept in state invalid", "rule accept in proto tcp dport 70000",
                          "rule accept in from 1.2.3", "rule accept in proto icmp dport 1", "", "rule accept in proto tcp proto udp" };
    for (unsigned k = 0; k < sizeof bad / sizeof bad[0]; k++) {
        CHECK(add_rule(0, bad[k]) == -EINVAL, "rule text %u must be refused", k);
    }
    CHECK(add_rule(99, "rule drop any") == -ENOENT, "a position past the end");
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 99, 0) == -ENOENT, "deleting a rule that is not there");
    CHECK(info().rules == base, "none of that changed anything");

    /* the 32-rule limit */
    unsigned added = 0;
    while (add_rule(0, "rule drop out proto udp dport 7") == 0) added++;
    CHECK(added == 32 - base, "%u rules fitted, expected %u", added, 32 - base);
    CHECK(add_rule(0, "rule drop any") == -ENOSPC, "the 33rd is refused");
    for (unsigned k = 0; k < added; k++) simple(NOVA_FW_OP_DEL_RULE, 0, 0);
    CHECK(info().rules == base, "removed again");

    /* the policy, and the switch */
    CHECK(!probe(IN, TCP, PEER, US, 5555, 22, SYN, 0).accept, "inbound is dropped ...");
    CHECK(simple(NOVA_FW_OP_SET_POLICY, 0, 1) == 0, "policy in -> accept");
    v = probe(IN, TCP, PEER, US, 5555, 22, SYN, 0);
    CHECK(v.accept && v.reason == NOVA_FW_R_POLICY && info().policy_in == 1, "... until the policy changes");
    v = probe(IN, TCP, PEER, US, 80, 40999, ACK, 0);
    CHECK(!v.accept && v.detail == NOVA_FW_INV_NO_CONN, "but INVALID is dropped whatever the policy says");
    CHECK(simple(NOVA_FW_OP_SET_POLICY, 0, 0) == 0 && info().policy_in == 0, "restored");
    CHECK(simple(NOVA_FW_OP_SET_POLICY, 7, 1) == -EINVAL, "there are two directions");
    CHECK(simple(NOVA_FW_OP_SET_ENABLED, 0, 0) == 0 && info().enabled == 0, "switched off");
    v = probe(IN, TCP, PEER, US, 80, 40999, ACK, 0);
    CHECK(v.accept && v.reason == NOVA_FW_R_DISABLED, "everything passes");
    CHECK(simple(NOVA_FW_OP_SET_ENABLED, 1, 0) == 0 && info().enabled == 1, "switched on");
    CHECK(!probe(IN, TCP, PEER, US, 80, 40999, ACK, 0).accept, "and it filters again");
    if (failures == f0) printf("[fwtest] ok: rules - insert, append, delete, first match wins, hit and byte counters, every kind of bad rule refused, the 32-rule limit, the policy, and the switch\n");
}

/* ---- group 5: REAL traffic ------------------------------------------------------------------- */

/* A real TFTP download from the gateway: a genuine UDP exchange - the request goes out to
 * port 69 and a stream of data comes back that is accepted only because the firewall
 * tracked the request. (The emulated server answers from port 69 itself, so this is an
 * ordinary ESTABLISHED reply; the helper for servers that answer from another port is
 * covered by the hand-built flows of the UDP group.) A ping to the gateway was tried
 * first and is a poor probe: the emulated network forwards ICMP to the host's own
 * network, which answers late or not at all, and that says nothing about the firewall. */
#define FETCH_NAME "WEATHER.PKG"
#define FETCH_TMP  "FWFETCH.TMP"

static int fetch(void) {
    int n = sys_tftp_fetch(GW, FETCH_NAME, FETCH_TMP);
    sys_delete_file(FETCH_TMP); /* a second download would otherwise fail to write over it */
    return n;
}

static void test_real_traffic(void) {
    int f0 = failures;
    simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0);
    nova_fw_info_t b = info();
    int base = fetch();
    nova_fw_info_t a = info();
    if (base <= 0) {
        /* The emulated network does not always answer (a sandboxed or heavily loaded
         * host: the same reason udptest and the ping demos report "no reply" and carry
         * on). That says nothing about the firewall - IF the download fails just as
         * badly with the firewall switched OFF. That control run is what makes skipping
         * honest: a download that works with the firewall off but not on is a firewall
         * bug, and fails here. (The real traffic of every boot - the gateway ping, the
         * TFTP download, DNS, the package install - already passes through the firewall
         * and is asserted by its own boot lines.) */
        CHECK(simple(NOVA_FW_OP_SET_ENABLED, 0, 0) == 0, "switching the firewall off for the control run");
        int control = fetch();
        CHECK(simple(NOVA_FW_OP_SET_ENABLED, 1, 0) == 0 && info().enabled == 1, "and back on");
        CHECK(control <= 0, "the download fails with the firewall OFF too, so the network, not the firewall, is what is not answering (control run got %d)", control);
        simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0);
        printf("[fwtest] note: the emulated network did not answer a TFTP download, with the firewall on or off, so the live-traffic checks are skipped\n");
        if (failures == f0) printf("[fwtest] ok: live traffic - skipped, the network did not answer even with the firewall off\n");
        return;
    }
    CHECK(a.accepted_out >= b.accepted_out + 1 && a.accepted_in >= b.accepted_in + 1,
          "the request went out and the answer came in through the firewall (%u/%u more)",
          a.accepted_out - b.accepted_out, a.accepted_in - b.accepted_in);
    CHECK(a.new_conns > b.new_conns, "the exchange is a tracked flow (%u new)", a.new_conns - b.new_conns);

    unsigned int c[16];
    /* 1. drop the REQUEST on its way out */
    CHECK(add_rule(0, "rule drop out proto udp dport 69") == 0, "a rule dropping outbound TFTP requests");
    nova_fw_info_t b2 = info();
    CHECK(fetch() < 0, "a real download is now stopped");
    nova_fw_info_t a2 = info();
    CHECK(a2.dropped_out > b2.dropped_out, "counted as dropped outbound (%u -> %u)", b2.dropped_out, a2.dropped_out);
    CHECK(get_rule(0, c) == 0 && c[NOVA_FW_RL_HITS] >= 1, "and the rule's own hit counter moved");
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 0, 0) == 0, "removing the rule");
    CHECK(fetch() > 0, "the download works again");

    /* 2. let the request out but drop what comes back */
    CHECK(add_rule(0, "rule drop in proto udp state established") == 0, "a rule dropping the answers");
    nova_fw_info_t b3 = info();
    CHECK(fetch() < 0, "the download is stopped on the way back");
    nova_fw_info_t a3 = info();
    CHECK(a3.dropped_in > b3.dropped_in && a3.accepted_out > b3.accepted_out,
          "the request left (%u more out) and the answer was dropped (%u more dropped in)",
          a3.accepted_out - b3.accepted_out, a3.dropped_in - b3.dropped_in);
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 0, 0) == 0 && fetch() > 0, "and it recovers");

    /* 3. with the helper's expectations flushed away the answer has no invitation */
    simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0);
    CHECK(info().expects == 0 && info().conns == 0, "flushing the table also forgets the helper's expectations");
    if (failures == f0) printf("[fwtest] ok: live traffic - a real TFTP download passes through the firewall and is tracked; a rule dropping the request stops it, a rule dropping the answer stops it with the request visibly sent, and removing the rule restores it\n");
}

/* ---- group 5b: a confined root ---------------------------------------------------------------- */

static void test_confinement(void) {
    int f0 = failures;
    nova_fw_info_t before = info();
    char* argv[] = { "FWCONF.ELF", "confined" };
    int pid = sys_exec_trusted("FWCONF.ELF", argv, 2);
    CHECK(pid > 0, "FWCONF.ELF could not be started (%d)", pid);
    int code = -1;
    if (pid > 0) code = sys_wait(pid);
    CHECK(code == 0, "the confined root reported %d failed checks", code);
    nova_fw_info_t after = info();
    CHECK(after.rules == before.rules && after.locked == 0 && after.enabled == 1, "and from here nothing changed (rules %u, locked %u)", after.rules, after.locked);
    if (failures == f0) printf("[fwtest] ok: confinement - a root process bound by a MAC profile may read the firewall and ask what a packet would get, but may not change anything, not even lock it\n");
}

/* ---- group 6: the lock (last: it cannot be undone until reboot) ------------------------------ */

static void test_lock(void) {
    int f0 = failures;
    nova_fw_info_t i = info();
    CHECK(i.locked == 0, "not locked yet");
    CHECK(simple(NOVA_FW_OP_LOCK, 0, 0) == 0, "locking");
    CHECK(info().locked == 1, "locked");
    CHECK(add_rule(0, "rule accept in") == -EPERM, "no rule can be added");
    CHECK(simple(NOVA_FW_OP_DEL_RULE, 0, 0) == -EPERM, "none removed");
    CHECK(simple(NOVA_FW_OP_FLUSH_RULES, 0, 0) == -EPERM, "none flushed");
    CHECK(simple(NOVA_FW_OP_SET_POLICY, 0, 1) == -EPERM, "the policy cannot change");
    CHECK(simple(NOVA_FW_OP_SET_ENABLED, 0, 0) == -EPERM, "it cannot be switched off");
    CHECK(info().rules == i.rules && info().policy_in == 0 && info().enabled == 1, "and nothing did");
    /* what it does NOT stop: looking, probing, and making the firewall stricter */
    unsigned int r[18];
    CHECK(get_rule(0, r) == 0, "rules can still be read");
    CHECK(!probe(IN, TCP, PEER, US, 5555, 22, SYN, 1).accept, "the filter still filters");
    CHECK(simple(NOVA_FW_OP_FLUSH_CONNS, 0, 0) == 0, "tracked flows can be flushed (that only tightens it)");
    CHECK(simple(NOVA_FW_OP_LOCK, 0, 0) == 0 && info().locked == 1, "locking again is harmless");
    if (failures == f0) printf("[fwtest] ok: lock - after the one-way lock no rule, policy or switch can change, even for root; reading, filtering and flushing flows still work\n");
}

int main(int argc, char** argv) {
    if (argc > 1 && strcmp(argv[1], "confined") == 0) confined_child(); /* never returns */
    test_api_and_privilege();
    test_probes_change_nothing();
    test_tcp_flow();
    test_inbound_needs_an_invitation();
    test_udp_icmp_and_helper();
    test_table_limit();
    test_rule_management();
    test_real_traffic();
    test_confinement();
    test_lock();

    if (failures == 0) {
        printf("[fwtest] PASS: the stateful firewall holds (%d checks)\n", checks);
        return 0;
    }
    printf("[fwtest] FAIL: %d of %d checks failed\n", failures, checks);
    return 1;
}
