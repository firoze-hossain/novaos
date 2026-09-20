/*
 * net.c - NIC selection, the receive-poll loop, and the shared IP/
 * ICMP checksum helper
 */
#include "net.h"
#include "ethernet.h"
#include "../drivers/net/ne2000.h"
#include "../drivers/net/rtl8139.h"
#include "../drivers/virtio/virtio_net.h"
#include "../include/kernel.h"

typedef enum { NIC_NONE, NIC_VIRTIO_NET, NIC_RTL8139, NIC_NE2000 } active_nic_t;

static active_nic_t active_nic = NIC_NONE;

void net_init(void) {
    /* Phase 45: virtio-net checked first, preferred over both
     * existing drivers if present - matching real-world practice
     * (virtio is preferred over emulated real hardware whenever a
     * hypervisor offers it) and this project's own release-readiness
     * roadmap naming virtio as the standard, dramatically simpler way
     * a VM talks to its host. Falls through to the exact prior
     * behavior (prefer RTL8139, then NE2000) completely unchanged
     * when no virtio-net-pci device is attached - this project's own
     * default test config does not attach one, so every existing
     * network self-test keeps running against RTL8139 exactly as
     * before this phase; see PROGRESS.md's Phase 45 entry for the
     * separate, real-hardware verification that virtio-net itself
     * works, done deliberately outside this shared default path. */
    virtio_net_init();
    if (virtio_net_is_present()) {
        active_nic = NIC_VIRTIO_NET;
    } else {
        /* Prefer the RTL8139 PCI NIC if Phase 13's enumeration finds
         * one - it's the more capable/modern of the two drivers (real
         * DMA vs. NE2000's page-indexed remote-DMA protocol) -
         * falling back to the ISA NE2000 driver from Phase 6 if not.
         * Either, neither, or (harmlessly) both can be attached to
         * the same VM; whichever is found first here is the one
         * actually used, and the other simply sits unused rather than
         * causing a conflict. */
        rtl8139_init();
        if (rtl8139_is_present()) {
            active_nic = NIC_RTL8139;
        } else {
            ne2000_init();
            if (ne2000_is_present()) {
                active_nic = NIC_NE2000;
            } else {
                active_nic = NIC_NONE;
            }
        }
    }

    if (active_nic != NIC_NONE) {
        const char* nic_name = active_nic == NIC_VIRTIO_NET ? "virtio-net"
                                : active_nic == NIC_RTL8139  ? "RTL8139"
                                                              : "NE2000";
        kernel_log("[ OK ] Network up (%s): IP %d.%d.%d.%d, gateway "
                   "%d.%d.%d.%d\n",
                   nic_name,
                   (int)(NET_OUR_IP >> 24) & 0xFF,
                   (int)(NET_OUR_IP >> 16) & 0xFF,
                   (int)(NET_OUR_IP >> 8) & 0xFF, (int)NET_OUR_IP & 0xFF,
                   (int)(NET_GATEWAY_IP >> 24) & 0xFF,
                   (int)(NET_GATEWAY_IP >> 16) & 0xFF,
                   (int)(NET_GATEWAY_IP >> 8) & 0xFF,
                   (int)NET_GATEWAY_IP & 0xFF);
    }
}

bool net_is_up(void) {
    return active_nic != NIC_NONE;
}

bool net_driver_send(const void* frame, uint16_t length) {
    switch (active_nic) {
        case NIC_VIRTIO_NET:
            return virtio_net_send(frame, length);
        case NIC_RTL8139:
            return rtl8139_send(frame, length);
        case NIC_NE2000:
            return ne2000_send(frame, length);
        default:
            return false;
    }
}

/* kernel/rust/net_irq.rs's exported check - see that file's own doc
 * comment. */
extern bool rust_net_rx_check_and_clear(void);

/* Phase 58: kernel/rust/tcp.rs's own retransmission/pacing clock - see
 * that file's own rust_tcp_poll() doc comment for why net_poll() is
 * its call site (this kernel has no dedicated per-connection timer
 * interrupt, so it rides along on whatever already calls net_poll()
 * regularly: the idle loop, and every other blocking wait loop in this
 * kernel). */
extern void rust_tcp_poll(void);

uint16_t net_driver_receive(void* buffer) {
    switch (active_nic) {
        case NIC_VIRTIO_NET:
            /* Not gated behind the Phase 43 rust_net_rx_check_and_clear()
             * flag the RTL8139 case below uses - that flag is set only
             * by RTL8139's own IRQ handler (kernel/drivers/net/
             * rtl8139.c), not by virtio-net, which has no interrupt
             * handler of its own yet (see PROGRESS.md's Phase 45 "Known
             * limitations" - virtio-net polls virtio_net_receive()'s
             * own used-ring check every call, the same honest, still-
             * polling status RTL8139 itself was in before Phase 43).
             * virtio_net_receive() itself is a cheap, non-blocking
             * used-ring check (no PCI I/O port access at all unless a
             * packet has actually completed), not a full hardware
             * register poll the way pre-Phase-43 RTL8139 was - real,
             * but smaller than RTL8139's own original gap. */
            return virtio_net_receive(buffer);
        case NIC_RTL8139:
            /* Phase 43: the actual CPU-saving change - net_poll() (via
             * this function) gets called on every idle-loop tick
             * regardless of whether anything happened (see
             * kernel_main()'s own idle_task_entry() comment); checking
             * this flag first means that, for RTL8139 specifically,
             * an idle tick with no packet touches nothing but a
             * SpinLock'd bool - no NIC hardware register read at all
             * - rather than reading REG_CMD every single tick forever
             * regardless of whether a packet ever arrives, which is
             * what this driver did before this phase. Once the flag
             * *is* set, rtl8139_receive() below is entirely unchanged
             * from before - its own ring-buffer draining logic already
             * correctly loops until genuinely empty, this flag only
             * decides whether to bother calling it at all this tick. */
            if (!rust_net_rx_check_and_clear()) {
                return 0;
            }
            /* Drains exactly one packet per call (rtl8139_receive()'s
             * own, unchanged behavior) - if several arrive in a burst
             * before this next runs, only one drains here; the rest
             * wait for their *own* ROK interrupts (RTL8139 raises one
             * per packet, not just "buffer non-empty"), each
             * re-setting this same flag, so nothing is ever lost, just
             * drained across a few more net_poll() calls instead of
             * all at once. A real, understood, bounded latency trade-
             * off, not a correctness gap - and one that essentially
             * never matters in practice, since every caller actually
             * waiting on network activity (tcp.c/dns.c/arp.c/tftp.c/
             * icmp.c) already calls net_poll() in its own tight loop,
             * not just once per idle tick. */
            return rtl8139_receive(buffer);
        case NIC_NE2000:
            /* Phase 52: the same signal, the same reasoning, reused
             * rather than duplicated - safe because active_nic
             * guarantees RTL8139 and NE2000 are never both the active
             * device at once, so there's no real scenario where one
             * driver's own IRQ could set a flag the other driver
             * wrongly consumes. ne2000_receive() below is otherwise
             * entirely unchanged - this flag only decides whether to
             * bother calling into it at all this tick, the identical
             * shape as the RTL8139 case just above. */
            if (!rust_net_rx_check_and_clear()) {
                return 0;
            }
            return ne2000_receive(buffer);
        default:
            return 0;
    }
}

const uint8_t* net_driver_mac_address(void) {
    static const uint8_t zero_mac[6] = {0, 0, 0, 0, 0, 0};
    switch (active_nic) {
        case NIC_VIRTIO_NET:
            return virtio_net_mac_address();
        case NIC_RTL8139:
            return rtl8139_mac_address();
        case NIC_NE2000:
            return ne2000_mac_address();
        default:
            return zero_mac;
    }
}

void net_poll(void) {
    if (active_nic == NIC_NONE) {
        return;
    }

    static uint8_t frame_buffer[RTL8139_MAX_FRAME > NE2000_MAX_FRAME
                                     ? RTL8139_MAX_FRAME
                                     : NE2000_MAX_FRAME];
    uint16_t length = net_driver_receive(frame_buffer);
    if (length > 0) {
        eth_handle_frame(frame_buffer, length);
    }

    /* Phase 58: drive TCP retransmission/pacing every time something
     * already calls net_poll() - see rust_tcp_poll()'s own doc comment.
     * Unconditional (not gated behind `length > 0`): a connection needs
     * to retransmit or send more buffered data even on a tick where
     * nothing arrived. */
    rust_tcp_poll();
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
