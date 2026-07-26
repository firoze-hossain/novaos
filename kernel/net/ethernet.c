/*
 * ethernet.c - Ethernet framing: build outgoing frames, dispatch
 * incoming ones by ethertype
 */
#include "ethernet.h"
#include "arp.h"
#include "ip.h"
#include "../drivers/net/ne2000.h"
#include "../lib/string.h"
#include "../include/kernel.h"

bool eth_send(const uint8_t dest_mac[6], uint16_t ethertype,
              const void* payload, uint16_t payload_len) {
    static uint8_t frame[ETH_HEADER_LEN + ETH_MTU_PAYLOAD];

    if (payload_len > ETH_MTU_PAYLOAD) {
        return false;
    }

    eth_header_t* hdr = (eth_header_t*)frame;
    memcpy(hdr->dest_mac, dest_mac, 6);
    memcpy(hdr->src_mac, ne2000_mac_address(), 6);
    hdr->ethertype = eth_htons(ethertype);

    memcpy(frame + ETH_HEADER_LEN, payload, payload_len);

    return ne2000_send(frame, (uint16_t)(ETH_HEADER_LEN + payload_len));
}

void eth_handle_frame(const uint8_t* frame, uint16_t length) {
    if (length < ETH_HEADER_LEN) {
        return;
    }

    const eth_header_t* hdr = (const eth_header_t*)frame;
    uint16_t ethertype = eth_ntohs(hdr->ethertype);
    const uint8_t* payload = frame + ETH_HEADER_LEN;
    uint16_t payload_len = (uint16_t)(length - ETH_HEADER_LEN);

    switch (ethertype) {
        case ETHERTYPE_ARP:
            arp_handle_packet(hdr->src_mac, payload, payload_len);
            break;
        case ETHERTYPE_IPV4:
            ip_handle_packet(hdr->src_mac, payload, payload_len);
            break;
        default:
            break; /* not a protocol NovaOS speaks - ignore */
    }
}
