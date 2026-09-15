//! kernel/rust/net_irq.rs - Phase 43: the signal RTL8139's IRQ
//! handler uses to tell net_poll() a packet actually arrived, instead
//! of net_poll() touching NIC hardware registers on every idle tick
//! regardless of whether anything happened. Phase 52 adds the
//! matching signal for transmit completion (RTL8139's own TX path,
//! and NE2000, both converted from a hardware-register busy-wait to
//! this same pattern in that same phase - see PROGRESS.md's own Phase
//! 52 entry).
//!
//! Gap this fills: kernel/init/main.c's idle_task_entry() calls
//! net_poll() every time it wakes from `hlt` - which, since `hlt`
//! itself wakes on every timer tick, means the NIC's hardware command
//! register gets read roughly TIMER_FREQUENCY_HZ times per second,
//! forever, whether or not a packet has actually arrived. This
//! project's own release-readiness roadmap names exactly this pattern
//! ("all current drivers poll instead of using interrupts... wastes
//! CPU continuously, even when doing nothing"), and RTL8139.c's own
//! header comment was explicit about it being a deliberate, known gap
//! ("polling PIO/DMA, no IRQ").
//!
//! Design: a single, sticky "something arrived" flag - not a precise
//! count, deliberately. The actual packet-draining logic
//! (rtl8139_receive(), unchanged by this phase - see that file's own
//! comment) already loops correctly until the hardware ring buffer is
//! genuinely empty; this flag's only job is "should net_poll() bother
//! calling into that logic at all this tick," not "how many packets
//! are there." A `SpinLock<bool>` is the right, if slightly heavier-
//! than-strictly-needed, choice over a bare atomic specifically
//! because this project already has SpinLock (Phase 40) built and
//! proven for exactly this shape of problem (shared state touched from
//! both interrupt and non-interrupt context) - reusing it here is the
//! real, concrete "future code that needs this" case that module's own
//! header comment anticipated, not a new primitive invented to declare
//! victory over a different subsystem's IRQ integration problem.
//!
//! `TX_COMPLETE` (Phase 52) is the same shape, deliberately, for a
//! genuinely different use case - not RX's "something arrived
//! eventually, at some unpredictable point" but "I just submitted
//! exactly one transmit request, and I'm waiting specifically for
//! *that* one's own completion before submitting another" (both
//! drivers this signal serves only ever have one TX request in flight
//! at a time, by their own existing, unchanged design - see
//! rtl8139_send()'s/ne2000_send()'s own comments). A single sticky
//! flag is exactly sufficient for that one-request-at-a-time shape,
//! the same reasoning RX_PENDING's own doc comment already gives for
//! why a flag, not a counter, is the right level of precision here.
//!
//! One `SpinLock<bool>` shared by two, mutually-exclusive drivers
//! (RTL8139, NE2000) is deliberate, not an oversight: `kernel/net/
//! net.c`'s own `active_nic` dispatch guarantees only one of them is
//! ever the active device at a time, so there is no real scenario
//! where both could signal or check this concurrently - reusing the
//! same, already-tested primitive is lower-risk than adding a second,
//! parallel one for a case that can't actually arise.

use crate::spinlock::SpinLock;

static RX_PENDING: SpinLock<bool> = SpinLock::new(false);
static TX_COMPLETE: SpinLock<bool> = SpinLock::new(false);

/// Called from RTL8139's IRQ handler (interrupt context) when the
/// ISR's ROK (Receive OK) bit was set - marks that at least one
/// packet is waiting in the hardware ring buffer for net_poll() to
/// drain.
#[no_mangle]
pub extern "C" fn rust_net_rx_signal() {
    let mut pending = RX_PENDING.lock();
    *pending = true;
}

/// Called from net_poll() (ordinary, non-interrupt context) - returns
/// whether a packet was signaled as pending since the last call, and
/// atomically clears the flag in the same locked critical section (not
/// two separate operations, which would leave a window where a new
/// IRQ's signal could be silently lost between a read and a later
/// clear).
#[no_mangle]
pub extern "C" fn rust_net_rx_check_and_clear() -> bool {
    let mut pending = RX_PENDING.lock();
    let was_pending = *pending;
    *pending = false;
    was_pending
}

/// Phase 52: called from a NIC's IRQ handler (interrupt context) when
/// the hardware signals transmit completion (RTL8139's own TOK bit,
/// or NE2000's own PTX/TXE bits) - marks that the one, currently
/// in-flight transmit request has finished.
#[no_mangle]
pub extern "C" fn rust_net_tx_signal_complete() {
    let mut complete = TX_COMPLETE.lock();
    *complete = true;
}

/// Phase 52: called from rtl8139_send()'s/ne2000_send()'s own bounded
/// wait loop (ordinary, non-interrupt context) - same check-and-clear
/// shape as the RX side, for the same reason (a single locked
/// operation, not a separate read then a separate clear, so a real
/// completion signaled between the two can never be silently missed).
#[no_mangle]
pub extern "C" fn rust_net_tx_check_and_clear() -> bool {
    let mut complete = TX_COMPLETE.lock();
    let was_complete = *complete;
    *complete = false;
    was_complete
}

/// Ring-0 self-test, called directly from kernel_main() - proves the
/// signal/check-and-clear pair behaves correctly under exactly the
/// pattern RTL8139's real IRQ handler and net_poll() actually use it
/// under: signal (simulating an IRQ firing), check-and-clear observes
/// `true` exactly once, a second check-and-clear (simulating the next
/// idle tick, nothing new having arrived) observes `false`. Phase 52
/// adds the identical proof for TX_COMPLETE, exercised the same way
/// rtl8139_send()'s/ne2000_send()'s own real wait loop uses it.
///
/// Deliberately does NOT assume either flag starts `false` when this
/// runs - by the time it does (after net_init() and this project's
/// own ping/TFTP/DNS self-tests have already exercised real network
/// traffic), RTL8139's *real* IRQ handler may well have already
/// signaled this exact flag from a real packet arriving - which is
/// himself evidence this phase's actual goal (genuinely interrupt-
/// driven reception) is working, not something this test should
/// paper over by asserting a starting state it can't actually
/// guarantee. An explicit clear first establishes a known state
/// before testing the logic this function actually exists to prove.
#[no_mangle]
pub extern "C" fn rust_net_irq_selftest() -> i32 {
    let mut code = 0;

    let _ = rust_net_rx_check_and_clear(); // establish a known state

    rust_net_rx_signal();
    if !rust_net_rx_check_and_clear() {
        code |= 2;
    }

    // The clear in the check above must actually have cleared it.
    if rust_net_rx_check_and_clear() {
        code |= 4;
    }

    let _ = rust_net_tx_check_and_clear(); // establish a known state

    rust_net_tx_signal_complete();
    if !rust_net_tx_check_and_clear() {
        code |= 8;
    }

    if rust_net_tx_check_and_clear() {
        code |= 16;
    }

    code
}
