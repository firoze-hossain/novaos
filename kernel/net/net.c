/*
 * net.c - NIC init, the receive-poll loop, and the shared IP/ICMP
 * checksum helper
 */
#include "net.h"
#include "ethernet.h"
#include "../drivers/net/ne2000.h"
#include "../include/kernel.h"

void net_init(void) {
    ne2000_init();
    if (ne2000_is_present()) {
        kernel_log("[ OK ] Network up: IP %d.%d.%d.%d, gateway %d.%d.%d.%d\n",
                   (int)(NET_OUR_IP >> 24) & 0xFF, (int)(NET_OUR_IP >> 16) & 0xFF,
                   (int)(NET_OUR_IP >> 8) & 0xFF, (int)NET_OUR_IP & 0xFF,
                   (int)(NET_GATEWAY_IP >> 24) & 0xFF,
                   (int)(NET_GATEWAY_IP >> 16) & 0xFF,
                   (int)(NET_GATEWAY_IP >> 8) & 0xFF,
                   (int)NET_GATEWAY_IP & 0xFF);
    }
}

bool net_is_up(void) {
    return ne2000_is_present();
}

void net_poll(void) {
    if (!ne2000_is_present()) {
        return;
    }

    static uint8_t frame_buffer[NE2000_MAX_FRAME];
    uint16_t length = ne2000_receive(frame_buffer);
    if (length > 0) {
        eth_handle_frame(frame_buffer, length);
    }
}

uint16_t net_checksum16(const void* data, uint16_t length) {
    const uint8_t* bytes = (const uint8_t*)data;
    uint32_t sum = 0;

    for (uint16_t i = 0; i + 1 < length; i += 2) {
        sum += (uint16_t)((bytes[i] << 8) | bytes[i + 1]);
    }
    if (length & 1) {
        sum += (uint16_t)(bytes[length - 1] << 8);
    }
    while (sum >> 16) {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    return (uint16_t)~sum;
}
