/*
 * ip.c - minimal IPv4: no fragmentation, no options, no routing table
 */
#include "ip.h"
#include "ethernet.h"
#include "arp.h"
#include "icmp.h"
#include "net.h"
#include "../lib/string.h"
#include "../include/kernel.h"

typedef struct __attribute__((packed)) {
    uint8_t  version_ihl;
    uint8_t  tos;
    uint16_t total_length;
    uint16_t id;
    uint16_t flags_fragment;
    uint8_t  ttl;
    uint8_t  protocol;
    uint16_t checksum;
    uint32_t src_ip;
    uint32_t dest_ip;
} ip_header_t;

static uint32_t be32(uint32_t x) {
    return ((x & 0xFF) << 24) | ((x & 0xFF00) << 8) | ((x & 0xFF0000) >> 8) |
           ((x >> 24) & 0xFF);
}

bool ip_send(uint32_t dest_ip, uint8_t protocol, const void* payload,
             uint16_t payload_len) {
    uint8_t dest_mac[6];
    if (!arp_resolve(dest_ip, dest_mac)) {
        kernel_log("[WARN] ip_send: ARP resolve failed for %d.%d.%d.%d\n",
                   (int)(dest_ip >> 24) & 0xFF, (int)(dest_ip >> 16) & 0xFF,
                   (int)(dest_ip >> 8) & 0xFF, (int)dest_ip & 0xFF);
        return false;
    }

    static uint8_t packet[IP_HEADER_LEN + ETH_MTU_PAYLOAD];
    if (payload_len > ETH_MTU_PAYLOAD - IP_HEADER_LEN) {
        return false;
    }

    ip_header_t* hdr = (ip_header_t*)packet;
    hdr->version_ihl = 0x45;
    hdr->tos = 0;
    hdr->total_length = eth_htons((uint16_t)(IP_HEADER_LEN + payload_len));
    hdr->id = eth_htons(0);
    hdr->flags_fragment = eth_htons(0x4000); /* don't fragment */
    hdr->ttl = 64;
    hdr->protocol = protocol;
    hdr->checksum = 0;
    hdr->src_ip = be32(NET_OUR_IP);
    hdr->dest_ip = be32(dest_ip);
    hdr->checksum = eth_htons(net_checksum16(hdr, IP_HEADER_LEN));

    memcpy(packet + IP_HEADER_LEN, payload, payload_len);

    return eth_send(dest_mac, ETHERTYPE_IPV4, packet,
                     (uint16_t)(IP_HEADER_LEN + payload_len));
}

void ip_handle_packet(const uint8_t src_mac[6], const uint8_t* payload,
                       uint16_t length) {
    if (length < IP_HEADER_LEN) {
        return;
    }

    ip_header_t hdr;
    memcpy(&hdr, payload, IP_HEADER_LEN);

    if ((hdr.version_ihl >> 4) != 4) {
        return; /* not IPv4 */
    }
    uint8_t ihl_bytes = (uint8_t)((hdr.version_ihl & 0x0F) * 4);
    if (ihl_bytes < IP_HEADER_LEN || length < ihl_bytes) {
        return;
    }
    if (be32(hdr.dest_ip) != NET_OUR_IP) {
        return; /* not addressed to us (e.g. a broadcast we don't handle) */
    }

    const uint8_t* transport = payload + ihl_bytes;
    uint16_t total_len = eth_ntohs(hdr.total_length);
    uint16_t transport_len =
        (total_len > ihl_bytes) ? (uint16_t)(total_len - ihl_bytes) : 0;
    if (transport_len > length - ihl_bytes) {
        transport_len = (uint16_t)(length - ihl_bytes);
    }

    uint32_t src_ip = be32(hdr.src_ip);

    switch (hdr.protocol) {
        case IP_PROTO_ICMP:
            icmp_handle_packet(src_ip, transport, transport_len);
            break;
        default:
            break; /* UDP/TCP not implemented - see PROGRESS.md */
    }
    (void)src_mac;
}
