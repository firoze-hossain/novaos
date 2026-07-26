/*
 * icmp.c - ICMP Echo Request/Reply (ping) only - no other ICMP types
 */
#include "icmp.h"
#include "ip.h"
#include "net.h"
#include "ethernet.h"
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

bool icmp_ping(uint32_t dest_ip, uint32_t* out_rtt_ticks) {
    uint16_t seq = ++next_sequence;
    reply_seen = false;
    reply_sequence = 0;

    uint8_t packet[sizeof(icmp_header_t) + ICMP_PAYLOAD_LEN];
    icmp_header_t* hdr = (icmp_header_t*)packet;
    hdr->type = ICMP_TYPE_ECHO_REQUEST;
    hdr->code = 0;
    hdr->checksum = 0;
    hdr->identifier = eth_htons(PING_IDENTIFIER);
    hdr->sequence = eth_htons(seq);
    memset(packet + sizeof(icmp_header_t), 0x42, ICMP_PAYLOAD_LEN);
    hdr->checksum = eth_htons(net_checksum16(packet, sizeof(packet)));

    uint32_t start_ticks = timer_get_ticks();
    if (!ip_send(dest_ip, IP_PROTO_ICMP, packet, sizeof(packet))) {
        return false;
    }

    uint32_t deadline = start_ticks + 300; /* ~3s at 100Hz */
    while (timer_get_ticks() < deadline) {
        net_poll();
        if (reply_seen && reply_sequence == seq) {
            if (out_rtt_ticks != NULL) {
                *out_rtt_ticks = timer_get_ticks() - start_ticks;
            }
            return true;
        }
    }
    return false;
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
