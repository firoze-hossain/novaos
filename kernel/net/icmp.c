/*
 * icmp.c - ICMP Echo Request/Reply (ping) only - no other ICMP types
 */
#include "icmp.h"
#include "ip.h"
#include "net.h"
#include "ethernet.h"
#include "arp.h"
#include "../drivers/timer/timer.h"
#include "../lib/string.h"
#include "../include/kernel.h"

#define ICMP_TYPE_ECHO_REPLY   0
#define ICMP_TYPE_ECHO_REQUEST 8

typedef struct __attribute__((packed)) {
    uint8_t  type;
    uint8_t  code;
    uint16_t checksum;
    uint16_t identifier;
    uint16_t sequence;
} icmp_header_t;

#define ICMP_PAYLOAD_LEN 32 /* arbitrary, matches common ping tools */

static const uint16_t PING_IDENTIFIER = 0xA1B2;
static uint16_t next_sequence = 0;

/* Records the most recent Echo Reply seen, for icmp_ping()'s busy-wait
 * loop to notice - just like arp.c's one-entry cache, this only needs
 * to track one outstanding ping at a time, since NovaOS never sends
 * more than one. */
static volatile bool reply_seen = false;
static volatile uint16_t reply_sequence = 0;

/* Phase 33: state for the non-blocking start/poll split. Two phases,
 * since ip_send() itself can block (via arp_resolve()'s own ~3s
 * busy-wait for an ARP reply) just as much as waiting for the ICMP
 * Echo Reply itself can - found only by actually testing a ping to an
 * address with no already-cached ARP entry, not the gateway a boot
 * self-test had already resolved, which made the first, simpler
 * version of this split look correct when it wasn't. */
typedef enum {
    PING_STATE_IDLE,
    PING_STATE_WAITING_ARP,
    PING_STATE_WAITING_REPLY,
} ping_state_t;

static ping_state_t ping_state = PING_STATE_IDLE;
static uint32_t pending_dest_ip = 0;
static uint32_t pending_next_hop_ip = 0;
static uint16_t pending_sequence = 0;
static uint32_t pending_start_ticks = 0;
static uint32_t pending_deadline = 0;

static void send_echo_request(uint32_t dest_ip, uint16_t seq) {
    uint8_t packet[sizeof(icmp_header_t) + ICMP_PAYLOAD_LEN];
    icmp_header_t* hdr = (icmp_header_t*)packet;
    hdr->type = ICMP_TYPE_ECHO_REQUEST;
    hdr->code = 0;
    hdr->checksum = 0;
    hdr->identifier = eth_htons(PING_IDENTIFIER);
    hdr->sequence = eth_htons(seq);
    memset(packet + sizeof(icmp_header_t), 0x42, ICMP_PAYLOAD_LEN);
    hdr->checksum = eth_htons(net_checksum16(packet, sizeof(packet)));
    ip_send(dest_ip, IP_PROTO_ICMP, packet, sizeof(packet));
    /* Once arp_is_cached() has confirmed the next hop is resolved,
     * ip_send()'s own arp_resolve() call hits that same cache and
     * returns immediately - it does not block here. */
}

void icmp_ping_start(uint32_t dest_ip) {
    uint16_t seq = ++next_sequence;
    reply_seen = false;
    reply_sequence = 0;
    pending_sequence = seq;
    pending_dest_ip = dest_ip;

    /* Mirrors ip_send()'s own next-hop selection (route via the
     * gateway for anything off our local subnet) - duplicated here,
     * not exposed as a shared helper, since it's two lines and this
     * is the only other caller that needs to know the next hop
     * *before* calling ip_send(), to check ARP caching without
     * risking ip_send()'s own blocking path. */
    bool same_subnet =
        (dest_ip & NET_NETMASK) == (NET_OUR_IP & NET_NETMASK);
    pending_next_hop_ip = same_subnet ? dest_ip : NET_GATEWAY_IP;

    pending_start_ticks = timer_get_ticks();
    pending_deadline = pending_start_ticks + 300; /* ~3s at 100Hz */

    uint8_t mac[6];
    if (arp_is_cached(pending_next_hop_ip, mac)) {
        send_echo_request(dest_ip, seq);
        ping_state = PING_STATE_WAITING_REPLY;
    } else {
        arp_send_request(pending_next_hop_ip);
        ping_state = PING_STATE_WAITING_ARP;
    }
}

int icmp_ping_poll(uint32_t* out_rtt_ticks) {
    net_poll();

    if (ping_state == PING_STATE_WAITING_ARP) {
        uint8_t mac[6];
        if (arp_is_cached(pending_next_hop_ip, mac)) {
            send_echo_request(pending_dest_ip, pending_sequence);
            ping_state = PING_STATE_WAITING_REPLY;
            /* Falls through to the WAITING_REPLY check below in the
             * same call - the Echo Request was just sent, so it's
             * correct to immediately check for a reply rather than
             * waiting for the caller's next poll. */
        } else if (timer_get_ticks() >= pending_deadline) {
            ping_state = PING_STATE_IDLE;
            return -1; /* never got a reply to the ARP request either -
                          reported as a ping timeout either way, the
                          same "no way to distinguish, and the
                          practical result is the same" reasoning the
                          original single-phase version used for a
                          send failure */
        } else {
            return 0;
        }
    }

    if (reply_seen && reply_sequence == pending_sequence) {
        if (out_rtt_ticks != NULL) {
            *out_rtt_ticks = timer_get_ticks() - pending_start_ticks;
        }
        ping_state = PING_STATE_IDLE;
        return 1;
    }
    if (timer_get_ticks() >= pending_deadline) {
        ping_state = PING_STATE_IDLE;
        return -1;
    }
    return 0;
}

bool icmp_ping(uint32_t dest_ip, uint32_t* out_rtt_ticks) {
    icmp_ping_start(dest_ip);
    for (;;) {
        int result = icmp_ping_poll(out_rtt_ticks);
        if (result == 1) {
            return true;
        }
        if (result == -1) {
            return false;
        }
    }
}


void icmp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                         uint16_t length) {
    if (length < sizeof(icmp_header_t)) {
        return;
    }

    icmp_header_t hdr;
    memcpy(&hdr, payload, sizeof(hdr));

    if (hdr.type == ICMP_TYPE_ECHO_REQUEST) {
        /* Reply with the same identifier/sequence/data, per RFC 792 -
         * this is what makes `ping` from another host work against
         * NovaOS. */
        uint8_t reply[128];
        uint16_t reply_len = length;
        if (reply_len > sizeof(reply)) {
            reply_len = sizeof(reply);
        }
        memcpy(reply, payload, reply_len);

        icmp_header_t* reply_hdr = (icmp_header_t*)reply;
        reply_hdr->type = ICMP_TYPE_ECHO_REPLY;
        reply_hdr->checksum = 0;
        reply_hdr->checksum = eth_htons(net_checksum16(reply, reply_len));

        ip_send(src_ip, IP_PROTO_ICMP, reply, reply_len);
    } else if (hdr.type == ICMP_TYPE_ECHO_REPLY) {
        if (eth_ntohs(hdr.identifier) == PING_IDENTIFIER) {
            reply_sequence = eth_ntohs(hdr.sequence);
            reply_seen = true;
        }
    }
}
