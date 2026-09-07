#ifndef NET_ICMP_H
#define NET_ICMP_H

#include "../include/types.h"

/* Sends an ICMP Echo Request to `dest_ip` and busy-waits (via
 * net_poll(), same pattern as arp_resolve()) for up to ~3 seconds for
 * a matching Echo Reply. Returns true if one arrived before the
 * timeout. `out_rtt_ticks`, if non-NULL, receives the round-trip time
 * in timer ticks (100/sec by default - see kernel/drivers/timer).
 *
 * Safe to call from ordinary kernel code (interrupts enabled), which
 * is the only context this function itself still runs in - NOT from
 * a syscall handler (interrupts disabled for the syscall's whole
 * duration, per syscall_stub.asm's cli/sti): both the NIC's own IRQ
 * (needed for net_poll() to ever see the reply) and the timer's IRQ
 * (needed for timer_get_ticks() to advance at all, which this
 * function's own timeout logic depends on) would never fire, turning
 * a bounded 3-second wait into an unbounded hang of the entire
 * kernel, not just the calling process - the same class of mistake
 * SYS_READ_KEY (Phase 30) was deliberately designed to avoid.
 * SYS_PING (Phase 33) uses icmp_ping_start()/icmp_ping_poll() below
 * instead, for exactly this reason - this function is now
 * implemented in terms of them, preserved unchanged for existing
 * kernel-side callers (the boot self-test, the legacy ring-0 shell). */
bool icmp_ping(uint32_t dest_ip, uint32_t* out_rtt_ticks);

/* The non-blocking split of the above, safe to call from a syscall
 * handler. Sends the Echo Request and records bookkeeping state,
 * then returns immediately - does not wait at all. */
void icmp_ping_start(uint32_t dest_ip);

/* Non-blocking: call this repeatedly (e.g. once per SYS_PING_POLL
 * syscall, with the ring-3 caller yielding between calls) after
 * icmp_ping_start(). Calls net_poll() once per call (so it still
 * needs to be invoked repeatedly to actually receive anything - it
 * does not itself wait for a reply) and checks the current state.
 * Returns 1 if the matching reply has arrived (out_rtt_ticks filled
 * in if non-NULL), 0 if still waiting and the ~3s deadline hasn't
 * passed yet, -1 if the deadline passed with no reply. */
int icmp_ping_poll(uint32_t* out_rtt_ticks);

/* Called by ip_handle_packet() for incoming ICMP messages: answers
 * Echo Requests addressed to us, and records Echo Replies for
 * icmp_ping()/icmp_ping_poll() to notice. */
void icmp_handle_packet(uint32_t src_ip, const uint8_t* payload,
                         uint16_t length);

#endif
