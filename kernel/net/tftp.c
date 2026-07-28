/*
 * tftp.c - minimal read-only TFTP client (RFC 1350)
 */
#include "tftp.h"
#include "udp.h"
#include "net.h"
#include "../drivers/timer/timer.h"
#include "../lib/string.h"

#define TFTP_OPCODE_RRQ   1
#define TFTP_OPCODE_DATA  3
#define TFTP_OPCODE_ACK   4
#define TFTP_OPCODE_ERROR 5

#define TFTP_BLOCK_SIZE 512

static void send_ack(uint32_t server_ip, uint16_t client_port,
                      uint16_t server_port, uint16_t block_num) {
    uint8_t ack[4];
    ack[0] = 0;
    ack[1] = TFTP_OPCODE_ACK;
    ack[2] = (uint8_t)(block_num >> 8);
    ack[3] = (uint8_t)(block_num & 0xFF);
    udp_send(server_ip, client_port, server_port, ack, sizeof(ack));
}

int tftp_get(uint32_t server_ip, const char* remote_filename, void* buf,
             uint32_t buf_size) {
    static uint16_t next_client_port = 49152;
    uint16_t client_port = next_client_port++;
    if (next_client_port == 0) {
        next_client_port = 49152; /* skip over the 0 "no listener" sentinel */
    }

    udp_listen(client_port);

    /* RRQ packet: opcode, filename, 0, "octet" (binary mode), 0. */
    uint8_t request[128];
    uint32_t pos = 0;
    request[pos++] = 0;
    request[pos++] = TFTP_OPCODE_RRQ;

    size_t name_len = strlen(remote_filename);
    if (name_len > 100) {
        return -1; /* not a realistic 8.3-style filename - bail out */
    }
    memcpy(request + pos, remote_filename, name_len);
    pos += name_len;
    request[pos++] = 0;

    const char* mode = "octet";
    size_t mode_len = strlen(mode);
    memcpy(request + pos, mode, mode_len);
    pos += mode_len;
    request[pos++] = 0;

    if (!udp_send(server_ip, client_port, TFTP_SERVER_PORT, request, pos)) {
        return -1;
    }

    uint32_t total_received = 0;
    uint16_t expected_block = 1;
    uint16_t server_port = TFTP_SERVER_PORT; /* replaced once the server's
                                                 actual reply port is seen -
                                                 real TFTP servers answer
                                                 from a new ephemeral port,
                                                 not port 69 itself */
    bool locked_onto_server = false;

    /* No retransmission on a lost packet (see tftp.h) - just one overall
     * deadline for the whole transfer rather than a more forgiving
     * per-block timeout with retries. Fine for a local virtual network
     * with effectively zero packet loss. */
    uint32_t overall_deadline = timer_get_ticks() + 1000; /* ~10s */

    for (;;) {
        if (timer_get_ticks() > overall_deadline) {
            return -1;
        }

        net_poll();

        uint32_t src_ip;
        uint16_t src_port;
        uint8_t recv_buf[600];
        uint16_t recv_len;
        if (!udp_receive(&src_ip, &src_port, recv_buf, sizeof(recv_buf),
                          &recv_len)) {
            continue;
        }
        if (recv_len < 4) {
            continue; /* too short to be a valid TFTP packet */
        }

        uint16_t opcode = (uint16_t)((recv_buf[0] << 8) | recv_buf[1]);
        if (opcode == TFTP_OPCODE_ERROR) {
            return -1;
        }
        if (opcode != TFTP_OPCODE_DATA) {
            continue;
        }

        if (!locked_onto_server) {
            server_port = src_port;
            locked_onto_server = true;
        } else if (src_port != server_port) {
            continue; /* stray packet from an unexpected port - ignore */
        }

        uint16_t block_num = (uint16_t)((recv_buf[2] << 8) | recv_buf[3]);
        uint16_t data_len = (uint16_t)(recv_len - 4);

        if (block_num == expected_block) {
            if (total_received + data_len > buf_size) {
                return -1; /* wouldn't fit in the caller's buffer */
            }
            memcpy((uint8_t*)buf + total_received, recv_buf + 4, data_len);
            total_received += data_len;

            send_ack(src_ip, client_port, server_port, block_num);

            if (data_len < TFTP_BLOCK_SIZE) {
                return (int)total_received; /* short block = end of file */
            }
            expected_block++;
        } else if (block_num == (uint16_t)(expected_block - 1)) {
            /* Our previous ACK was probably lost and the server
             * retransmitted - re-ACK the same block rather than
             * treating it as new data. */
            send_ack(src_ip, client_port, server_port, block_num);
        }
        /* else: unexpected block number - ignore and keep waiting. */
    }
}
