/*
 * tcp.c - minimal TCP client (see tcp.h for the full scope note)
 */
#include "tcp.h"
#include "ip.h"
#include "ethernet.h"
#include "net.h"
#include "../drivers/timer/timer.h"
#include "../lib/string.h"

#define TCP_MAX_SEGMENT_DATA 1460 /* a conservative MSS for a standard
                                      1500-byte Ethernet MTU */

#define TCP_FLAG_FIN 0x01
#define TCP_FLAG_SYN 0x02
#define TCP_FLAG_RST 0x04
#define TCP_FLAG_PSH 0x08
#define TCP_FLAG_ACK 0x10

typedef enum {
    TCP_CLOSED,
    TCP_SYN_SENT,
    TCP_ESTABLISHED,
} tcp_state_t;

typedef struct __attribute__((packed)) {
    uint16_t src_port;
    uint16_t dest_port;
    uint32_t seq_num;
    uint32_t ack_num;
    uint8_t data_offset; /* upper 4 bits = header length in 32-bit words */
    uint8_t flags;
    uint16_t window;
    uint16_t checksum;
    uint16_t urgent_ptr;
} tcp_header_t;

static tcp_state_t state = TCP_CLOSED;
static uint32_t remote_ip;
static uint16_t remote_port;
static uint16_t local_port;

static uint32_t send_next;  /* next sequence number we will use */
static uint32_t send_acked; /* highest seq of ours acknowledged so far */
static uint32_t recv_next;  /* next sequence number we expect from the
                                peer - what we ack */
static bool peer_fin_received;

/* Single-slot incoming segment buffer - the same one-outstanding-
 * operation pattern arp.c/icmp.c/udp.c/dns.c already use. */
static bool segment_ready = false;
static uint32_t recv_seg_seq, recv_seg_ack;
static uint8_t recv_seg_flags;
static uint8_t recv_seg_data[TCP_MAX_SEGMENT_DATA];
static uint16_t recv_seg_data_len;

/* Data that has arrived in order but not yet been handed to a
 * tcp_receive() caller. */
static uint8_t pending_buf[TCP_MAX_SEGMENT_DATA];
static uint16_t pending_len = 0;

static bool send_segment(uint8_t flags, const void* data, uint16_t data_len) {
    static uint8_t packet[sizeof(tcp_header_t) + TCP_MAX_SEGMENT_DATA];
    tcp_header_t* hdr = (tcp_header_t*)packet;

    hdr->src_port = eth_htons(local_port);
    hdr->dest_port = eth_htons(remote_port);
    hdr->seq_num = eth_htonl(send_next);
    hdr->ack_num = eth_htonl(recv_next);
    hdr->data_offset = (uint8_t)(5 << 4); /* 20-byte header, no options */
    hdr->flags = flags;
    hdr->window = eth_htons(TCP_MAX_SEGMENT_DATA);
    hdr->checksum = 0;
    hdr->urgent_ptr = 0;

    if (data_len > 0) {
        memcpy(packet + sizeof(tcp_header_t), data, data_len);
    }
    uint16_t total_len = (uint16_t)(sizeof(tcp_header_t) + data_len);

    /* TCP's checksum covers a 12-byte pseudo-header (src/dst IP,
     * zero, protocol, TCP length) in addition to the real segment -
     * unlike this project's UDP, which sends 0 (disabled) and skips
     * this entirely, real-world TCP stacks universally validate this
     * checksum and silently drop segments that fail it, so it isn't
     * optional here the way it was for UDP. */
    static uint8_t checksum_buf[12 + sizeof(tcp_header_t) +
                                 TCP_MAX_SEGMENT_DATA];
    uint32_t src_ip_be = eth_htonl(NET_OUR_IP);
    uint32_t dst_ip_be = eth_htonl(remote_ip);
    memcpy(checksum_buf, &src_ip_be, 4);
    memcpy(checksum_buf + 4, &dst_ip_be, 4);
    checksum_buf[8] = 0;
    checksum_buf[9] = IP_PROTO_TCP;
    uint16_t tcp_len_be = eth_htons(total_len);
    memcpy(checksum_buf + 10, &tcp_len_be, 2);
    memcpy(checksum_buf + 12, packet, total_len);

    hdr->checksum =
        eth_htons(net_checksum16(checksum_buf, (uint16_t)(12 + total_len)));

    return ip_send(remote_ip, IP_PROTO_TCP, packet, total_len);
}

/* Polls for and processes one incoming segment (if any), updating
 * connection state accordingly - shared by tcp_send()/tcp_receive()/
 * tcp_close() so information in a segment isn't dropped just because
 * whichever function is currently waiting was looking for something
 * else in particular (e.g. data piggybacked on the ack of our own
 * most recent send). tcp_connect()'s handshake is handled separately,
 * since recv_next isn't meaningful until the peer's ISN is known. */
static bool process_incoming(void) {
    net_poll();
    if (!segment_ready) {
        return false;
    }
    segment_ready = false;

    if (recv_seg_flags & TCP_FLAG_RST) {
        state = TCP_CLOSED;
        return true;
    }

    if ((recv_seg_flags & TCP_FLAG_ACK) &&
        (int32_t)(recv_seg_ack - send_acked) > 0) {
        send_acked = recv_seg_ack;
    }

    if (recv_seg_seq == recv_next && recv_seg_data_len > 0 &&
        pending_len == 0) {
        memcpy(pending_buf, recv_seg_data, recv_seg_data_len);
        pending_len = recv_seg_data_len;
        recv_next += recv_seg_data_len;
    }

    if ((recv_seg_flags & TCP_FLAG_FIN) &&
        recv_seg_seq + recv_seg_data_len == recv_next) {
        peer_fin_received = true;
        recv_next++; /* FIN consumes one sequence number, like SYN */
    }

    return true;
}

bool tcp_connect(uint32_t remote_ip_param, uint16_t remote_port_param) {
    if (state != TCP_CLOSED) {
        return false; /* only one connection at a time - see tcp.h */
    }

    remote_ip = remote_ip_param;
    remote_port = remote_port_param;

    static uint16_t next_ephemeral_port = 55000;
    local_port = next_ephemeral_port++;
    if (next_ephemeral_port == 0) {
        next_ephemeral_port = 55000;
    }

    /* Not cryptographically random (a real security property RFC
     * 6528 cares about) - just varying enough across connections that
     * this isn't a fixed, predictable constant. Fine for a client
     * used the way this one has been tested: initiating outbound
     * connections on a private/local-ish network, not defending
     * against active ISN-guessing attacks. */
    send_next = timer_get_ticks() * 12345u + 1u;
    send_acked = send_next;
    recv_next = 0;
    peer_fin_received = false;
    pending_len = 0;
    segment_ready = false;

    state = TCP_SYN_SENT;
    if (!send_segment(TCP_FLAG_SYN, NULL, 0)) {
        state = TCP_CLOSED;
        return false;
    }
    send_next++; /* SYN consumes one sequence number */

    uint32_t deadline = timer_get_ticks() + 300; /* ~3s, matching every
                                                     other blocking
                                                     network op here */
    while (timer_get_ticks() < deadline) {
        net_poll();
        if (!segment_ready) {
            continue;
        }
        segment_ready = false;

        if (recv_seg_flags & TCP_FLAG_RST) {
            state = TCP_CLOSED;
            return false; /* connection refused */
        }
        if ((recv_seg_flags & TCP_FLAG_SYN) &&
            (recv_seg_flags & TCP_FLAG_ACK) && recv_seg_ack == send_next) {
            recv_next = recv_seg_seq + 1; /* their SYN consumes one too */
            send_acked = send_next;
            state = TCP_ESTABLISHED;
            send_segment(TCP_FLAG_ACK, NULL, 0); /* final ACK of the
                                                     3-way handshake */
            return true;
        }
    }

    state = TCP_CLOSED;
    return false; /* timeout */
}

bool tcp_send(const void* data, uint16_t len) {
    if (state != TCP_ESTABLISHED) {
        return false;
    }

    uint32_t target_ack = send_next + len;
    if (!send_segment(TCP_FLAG_ACK | TCP_FLAG_PSH, data, len)) {
        return false;
    }

    uint32_t deadline = timer_get_ticks() + 300;
    while (timer_get_ticks() < deadline) {
        process_incoming();
        if (state != TCP_ESTABLISHED) {
            return false; /* RST arrived */
        }
        if ((int32_t)(send_acked - target_ack) >= 0) {
            send_next = target_ack;
            return true;
        }
    }
    return false; /* timeout - see tcp.h: no retransmission, the
                      caller's problem to retry */
}

int tcp_receive(void* buf, uint16_t buf_size, uint32_t timeout_ticks) {
    if (state != TCP_ESTABLISHED && pending_len == 0) {
        return -1;
    }

    uint32_t deadline = timer_get_ticks() + timeout_ticks;
    while (timer_get_ticks() < deadline) {
        if (pending_len > 0) {
            uint16_t to_copy = (pending_len < buf_size) ? pending_len
                                                          : buf_size;
            memcpy(buf, pending_buf, to_copy);
            if (to_copy < pending_len) {
                /* kernel/lib/string.h has no memmove() (only the
                 * separate userland libc does) - shifting manually,
                 * low-to-high since the destination is always at a
                 * lower address than the source here, is safe without
                 * one. */
                for (uint16_t i = 0; i < pending_len - to_copy; i++) {
                    pending_buf[i] = pending_buf[to_copy + i];
                }
                pending_len = (uint16_t)(pending_len - to_copy);
            } else {
                pending_len = 0;
            }
            send_segment(TCP_FLAG_ACK, NULL, 0); /* acknowledge receipt */
            return (int)to_copy;
        }
        if (peer_fin_received) {
            return 0; /* clean close, no more data ever coming */
        }
        if (state != TCP_ESTABLISHED) {
            return -1; /* RST or similar */
        }
        process_incoming();
    }
    return -1; /* timeout */
}

void tcp_close(void) {
    if (state == TCP_CLOSED) {
        return;
    }

    send_segment(TCP_FLAG_FIN | TCP_FLAG_ACK, NULL, 0);
    uint32_t my_fin_seq = send_next;
    send_next++; /* FIN consumes one sequence number */

    uint32_t deadline = timer_get_ticks() + 300;
    while (timer_get_ticks() < deadline &&
           (int32_t)(send_acked - my_fin_seq) < 0) {
        process_incoming();
    }

    deadline = timer_get_ticks() + 300;
    while (timer_get_ticks() < deadline && !peer_fin_received) {
        process_incoming();
    }
    if (peer_fin_received) {
        send_segment(TCP_FLAG_ACK, NULL, 0); /* final ACK of their FIN -
                                                 see tcp.h on skipping
                                                 formal TIME_WAIT after
                                                 this */
    }

    state = TCP_CLOSED;
    peer_fin_received = false;
    pending_len = 0;
}

void tcp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                        uint16_t length) {
    if (length < sizeof(tcp_header_t) || state == TCP_CLOSED) {
        return;
    }

    tcp_header_t hdr;
    memcpy(&hdr, payload, sizeof(hdr));

    uint16_t sport = eth_ntohs(hdr.src_port);
    uint16_t dport = eth_ntohs(hdr.dest_port);
    if (src_ip != remote_ip || sport != remote_port || dport != local_port) {
        return; /* not for our one active connection */
    }

    uint8_t header_len = (uint8_t)((hdr.data_offset >> 4) * 4);
    if (header_len < sizeof(tcp_header_t) || header_len > length) {
        return; /* malformed */
    }

    uint16_t data_len = (uint16_t)(length - header_len);
    if (data_len > sizeof(recv_seg_data)) {
        data_len = sizeof(recv_seg_data); /* truncate defensively */
    }

    recv_seg_seq = eth_ntohl(hdr.seq_num);
    recv_seg_ack = eth_ntohl(hdr.ack_num);
    recv_seg_flags = hdr.flags;
    recv_seg_data_len = data_len;
    if (data_len > 0) {
        memcpy(recv_seg_data, payload + header_len, data_len);
    }

    segment_ready = true;
}
