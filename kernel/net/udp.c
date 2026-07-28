/*
 * udp.c - minimal UDP: send, and single-listener receive dispatch
 *
 * Only one "socket" can ever be listening at a time - a static
 * single-slot buffer, not a real port table. That's enough for
 * everything that currently uses UDP (just the TFTP client so far,
 * kernel/net/tftp.c, which is a synchronous, one-transfer-at-a-time
 * design anyway) - the same one-outstanding-operation simplification
 * arp.c's ARP cache and icmp.c's ping tracking already make. A second
 * concurrent UDP user would need a real port table; see PROGRESS.md.
 */
#include "udp.h"
#include "ip.h"
#include "ethernet.h"
#include "../lib/string.h"

typedef struct __attribute__((packed)) {
    uint16_t src_port;
    uint16_t dest_port;
    uint16_t length;
    uint16_t checksum;
} udp_header_t;

static uint16_t g_listen_port = 0;
static bool g_packet_ready = false;
static uint32_t g_packet_src_ip;
static uint16_t g_packet_src_port;
static uint8_t g_packet_data[600];
static uint16_t g_packet_len;

bool udp_send(uint32_t dest_ip, uint16_t src_port, uint16_t dest_port,
              const void* payload, uint16_t payload_len) {
    static uint8_t packet[UDP_HEADER_LEN + 600];
    if (payload_len > sizeof(packet) - UDP_HEADER_LEN) {
        return false;
    }

    udp_header_t* hdr = (udp_header_t*)packet;
    hdr->src_port = eth_htons(src_port);
    hdr->dest_port = eth_htons(dest_port);
    hdr->length = eth_htons((uint16_t)(UDP_HEADER_LEN + payload_len));
    hdr->checksum = 0; /* disabled - see header comment */

    memcpy(packet + UDP_HEADER_LEN, payload, payload_len);

    return ip_send(dest_ip, IP_PROTO_UDP, packet,
                    (uint16_t)(UDP_HEADER_LEN + payload_len));
}

void udp_listen(uint16_t port) {
    g_listen_port = port;
    g_packet_ready = false;
}

bool udp_receive(uint32_t* out_src_ip, uint16_t* out_src_port, void* buf,
                  uint16_t buf_size, uint16_t* out_len) {
    if (!g_packet_ready) {
        return false;
    }

    uint16_t copy_len = (g_packet_len < buf_size) ? g_packet_len : buf_size;
    memcpy(buf, g_packet_data, copy_len);

    *out_src_ip = g_packet_src_ip;
    *out_src_port = g_packet_src_port;
    *out_len = copy_len;

    g_packet_ready = false; /* consumed */
    return true;
}

void udp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                        uint16_t length) {
    if (length < UDP_HEADER_LEN || g_listen_port == 0) {
        return;
    }

    udp_header_t hdr;
    memcpy(&hdr, payload, UDP_HEADER_LEN);

    uint16_t dest_port = eth_ntohs(hdr.dest_port);
    if (dest_port != g_listen_port) {
        return; /* not for the current listener */
    }

    uint16_t data_len = (uint16_t)(length - UDP_HEADER_LEN);
    if (data_len > sizeof(g_packet_data)) {
        data_len = sizeof(g_packet_data);
    }

    memcpy(g_packet_data, payload + UDP_HEADER_LEN, data_len);
    g_packet_len = data_len;
    g_packet_src_ip = src_ip;
    g_packet_src_port = eth_ntohs(hdr.src_port);
    g_packet_ready = true;
}
