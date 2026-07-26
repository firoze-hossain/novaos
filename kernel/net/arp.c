/*
 * arp.c - Address Resolution Protocol (IPv4 over Ethernet only)
 */
#include "arp.h"
#include "ethernet.h"
#include "net.h"
#include "../drivers/net/ne2000.h"
#include "../drivers/timer/timer.h"
#include "../lib/string.h"

#define ARP_HTYPE_ETHERNET 1
#define ARP_PTYPE_IPV4     0x0800
#define ARP_OP_REQUEST     1
#define ARP_OP_REPLY       2

typedef struct __attribute__((packed)) {
    uint16_t htype;
    uint16_t ptype;
    uint8_t  hlen;
    uint8_t  plen;
    uint16_t oper;
    uint8_t  sha[6];
    uint32_t spa; /* big-endian on the wire; stored/compared as raw bytes */
    uint8_t  tha[6];
    uint32_t tpa;
} arp_packet_t;

static const uint8_t BROADCAST_MAC[6] = {0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF};

/* One-entry cache: enough for "resolve the gateway," which is the
 * only thing NovaOS currently ever needs to resolve. A real ARP table
 * (multiple entries, aging/expiry) is future work - see PROGRESS.md. */
static bool cache_valid = false;
static uint32_t cache_ip;
static uint8_t cache_mac[6];

static uint32_t be32_to_host(uint32_t be) {
    return ((be & 0xFF) << 24) | ((be & 0xFF00) << 8) |
           ((be & 0xFF0000) >> 8) | ((be >> 24) & 0xFF);
}
static uint32_t host_to_be32(uint32_t host) {
    return be32_to_host(host); /* symmetric operation */
}

void arp_send_request(uint32_t ip) {
    arp_packet_t pkt;
    pkt.htype = eth_htons(ARP_HTYPE_ETHERNET);
    pkt.ptype = eth_htons(ARP_PTYPE_IPV4);
    pkt.hlen = 6;
    pkt.plen = 4;
    pkt.oper = eth_htons(ARP_OP_REQUEST);
    memcpy(pkt.sha, ne2000_mac_address(), 6);
    pkt.spa = host_to_be32(NET_OUR_IP);
    memset(pkt.tha, 0, 6);
    pkt.tpa = host_to_be32(ip);

    eth_send(BROADCAST_MAC, ETHERTYPE_ARP, &pkt, sizeof(pkt));
}

bool arp_resolve(uint32_t ip, uint8_t mac_out[6]) {
    if (cache_valid && cache_ip == ip) {
        memcpy(mac_out, cache_mac, 6);
        return true;
    }

    arp_send_request(ip);

    uint32_t deadline = timer_get_ticks() + 300; /* ~3s at 100Hz */
    while (timer_get_ticks() < deadline) {
        net_poll();
        if (cache_valid && cache_ip == ip) {
            memcpy(mac_out, cache_mac, 6);
            return true;
        }
    }
    return false;
}

void arp_handle_packet(const uint8_t src_mac[6], const uint8_t* payload,
                        uint16_t length) {
    if (length < sizeof(arp_packet_t)) {
        return;
    }

    arp_packet_t pkt;
    memcpy(&pkt, payload, sizeof(pkt));

    if (eth_ntohs(pkt.htype) != ARP_HTYPE_ETHERNET ||
        eth_ntohs(pkt.ptype) != ARP_PTYPE_IPV4) {
        return;
    }

    uint32_t sender_ip = be32_to_host(pkt.spa);
    uint16_t oper = eth_ntohs(pkt.oper);

    /* Learn the sender's address either way - true of a real ARP
     * cache too, and it means a reply to our own request updates the
     * cache via the exact same code path as this opportunistic
     * learning does. */
    cache_valid = true;
    cache_ip = sender_ip;
    memcpy(cache_mac, src_mac, 6);

    if (oper == ARP_OP_REQUEST && be32_to_host(pkt.tpa) == NET_OUR_IP) {
        arp_packet_t reply;
        reply.htype = eth_htons(ARP_HTYPE_ETHERNET);
        reply.ptype = eth_htons(ARP_PTYPE_IPV4);
        reply.hlen = 6;
        reply.plen = 4;
        reply.oper = eth_htons(ARP_OP_REPLY);
        memcpy(reply.sha, ne2000_mac_address(), 6);
        reply.spa = host_to_be32(NET_OUR_IP);
        memcpy(reply.tha, pkt.sha, 6);
        reply.tpa = pkt.spa;

        eth_send(pkt.sha, ETHERTYPE_ARP, &reply, sizeof(reply));
    }
}
