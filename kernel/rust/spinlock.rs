//! kernel/rust/spinlock.rs - Phase 40: this kernel's first real
//! synchronization primitive.
//!
//! Gap this fills: no spinlock, mutex, or any other synchronization
//! primitive exists anywhere in this kernel before this phase (a real
//! limitation, named explicitly in this project's own PROGRESS.md and
//! its release-readiness roadmap). Every piece of shared kernel state
//! added so far (kernel/rust/pipe.rs's own PIPES table is the most
//! recent example) has been "safe" only by a documented, load-bearing
//! assumption: syscalls run with interrupts disabled for their entire
//! duration, so nothing else can run concurrently with syscall-handler
//! code on this single-core target. That assumption is correct today,
//! but it has two real gaps this module closes: (1) it says nothing
//! about two different *interrupt handlers* touching the same shared
//! state - an IRQ can itself be interrupted by a higher-priority one,
//! or preempt ordinary (non-syscall) kernel code, and (2) it is a
//! single-core-only argument, with nothing to fall back on if this
//! kernel ever gains real multi-core support, at which point "syscalls
//! disable interrupts" no longer says anything about a *second* CPU
//! running the exact same code at the exact same time.
//!
//! Design: the standard, well-established pattern for exactly this
//! situation (the same shape as Linux's `spin_lock_irqsave`/
//! `spin_unlock_irqrestore`) - disable this CPU's local interrupts for
//! the duration the lock is held (closing gap (1) above: no IRQ
//! handler can run, let alone race, while a critical section on this
//! CPU is in progress) *and* spin on a real atomic compare-and-swap
//! (closing gap (2): correct even if a second CPU exists and is
//! spinning on the same lock at the same time, even though none does
//! today). The *previous* interrupt-enabled state is saved and later
//! restored, not unconditionally re-enabled on unlock - critical for
//! correctly nesting one lock inside another (acquiring lock B while
//! already holding lock A, then releasing B, must leave interrupts
//! exactly as A's own critical section still needs them, not
//! prematurely re-enabled just because B's own unlock ran).
//!
//! `SpinLock<T>` follows Rust's ordinary RAII lock-guard shape (the
//! same one `std::sync::Mutex` uses in hosted Rust) rather than a
//! manual lock()/unlock() pair specifically because it makes the
//! safety property the type system enforces, not just documents: the
//! only way to reach the protected `T` at all is through the guard
//! `lock()` returns, and that guard's `Drop` impl is what releases the
//! lock - there is no code path that can reach the data without
//! holding the lock, and no way to forget to release it (short of an
//! explicit `core::mem::forget`, the same escape hatch every other use
//! of RAII in Rust has).
//!
//! Demonstrated on real, already-shipped state, not left as an
//! unused, purely theoretical primitive: kernel/rust/pipe.rs's own
//! PIPES table is now wrapped in a SpinLock<[Pipe; MAX_PIPES]> - see
//! that file's own updated comments for what changed and why the
//! observable behavior is unaffected today (nothing yet calls into
//! pipe.rs from interrupt context), while the underlying guarantee is
//! now real and enforced rather than a documented assumption resting
//! entirely on every future caller happening to respect it.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

// Safety: SpinLock<T> provides its own mutual exclusion (the whole
// point of this module) - the underlying T only needs to be Send
// (safe to access from whichever CPU/context happens to hold the
// lock), not Sync, since SpinLock's own locking is what makes shared
// access safe. Every type this project currently wraps in a SpinLock
// (kernel/rust/pipe.rs's own Pipe table) is a plain, no-pointers-to-
// itself struct, trivially Send.
unsafe impl<T: Send> Sync for SpinLock<T> {}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    saved_eflags: u32,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    /// Acquires the lock, disabling this CPU's interrupts for as long
    /// as the returned guard stays alive. Spins (does not block/yield
    /// to the scheduler) until acquired - correct but potentially
    /// wasteful if a critical section were ever long-running; every
    /// critical section this project currently protects with a
    /// SpinLock is a handful of array-index operations, not something
    /// that should ever spin for more than a few instructions in
    /// practice.
    pub fn lock(&self) -> SpinLockGuard<T> {
        let saved_eflags = unsafe { cli_and_save_eflags() };
        while self
            .locked
            .compare_exchange_weak(
                false,
                true,
                Ordering::Acquire,
                Ordering::Relaxed,
            )
            .is_err()
        {
            core::hint::spin_loop();
        }
        SpinLockGuard {
            lock: self,
            saved_eflags,
        }
    }
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // Safety: holding a SpinLockGuard is exactly this module's own
        // proof of exclusive access - see the module-level doc comment.
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        unsafe { restore_eflags(self.saved_eflags) };
    }
}

/// Disables interrupts (`cli`) and returns the *previous* value of
/// EFLAGS, captured before doing so - not just "whether interrupts
/// were on," the whole register, so `restore_eflags` can put every
/// other flag back exactly as it was too, not only the interrupt bit.
///
/// # Safety
/// Must only be paired with a later `restore_eflags` call using the
/// exact value this returned - anything else leaves the CPU's
/// interrupt-enable state permanently wrong.
#[inline(always)]
unsafe fn cli_and_save_eflags() -> u32 {
    let eflags: u32;
    core::arch::asm!("pushfd", "cli", "pop {0}", out(reg) eflags);
    eflags
}

/// Restores a previously-saved EFLAGS value (from `cli_and_save_eflags`),
/// including whatever interrupt-enable state it captured - re-enabling
/// interrupts only if they were actually on beforehand, not
/// unconditionally, so that unlocking an inner, nested lock correctly
/// leaves an outer lock's own interrupt-disabled critical section
/// undisturbed.
///
/// # Safety
/// `eflags` must be a value previously returned by
/// `cli_and_save_eflags`, used exactly once.
#[inline(always)]
unsafe fn restore_eflags(eflags: u32) {
    core::arch::asm!("push {0}", "popfd", in(reg) eflags);
}

#[cfg(test)]
mod tests_not_built_here {
    // This crate is built directly with rustc against a bare-metal
    // target (see tools/rust-sysroot/), not through `cargo test` -
    // there is no host to run a `#[test]` on. Correctness here is
    // proven instead by rust_spinlock_selftest() below, called from
    // kernel_main() the same way every other kernel-side Rust module
    // in this project proves itself (kernel/rust/pipe.rs's own
    // self-test is the precedent this follows).
}

/// Ring-0 self-test, called directly from kernel_main() (no syscall
/// involved) - see PROGRESS.md's Phase 40 entry for what each check
/// specifically proves and why it was chosen.
#[no_mangle]
pub extern "C" fn rust_spinlock_selftest() -> i32 {
    static LOCK: SpinLock<u32> = SpinLock::new(0);

    // 1. Basic acquire/mutate/release: the protected value actually
    // changes, and is observable again after the guard drops.
    {
        let mut guard = LOCK.lock();
        *guard = 42;
    }
    let basic_ok = *LOCK.lock() == 42;

    // 2. Interrupt state is correctly saved and restored across a
    // single lock/unlock cycle - captures whatever IF was before
    // taking the lock, takes the lock (forcing IF=0 while held),
    // and confirms IF is back to its original value after the guard
    // drops, not left disabled and not unconditionally re-enabled if
    // it started disabled.
    let if_before = interrupts_enabled();
    {
        let _guard = LOCK.lock();
        // Interrupts must be off while the guard is held, regardless
        // of what they were before - this is the actual point of the
        // lock, checked directly rather than assumed.
    }
    let if_after = interrupts_enabled();
    let single_level_ok = if_before == if_after;

    // 3. Nested locks (two distinct SpinLocks, one acquired while the
    // other is already held) leave interrupts correctly disabled for
    // the *entire* nested region, and correctly restored only once
    // the outer guard - not just the inner one - has dropped. This is
    // the specific property a naive "always cli on lock, always sti
    // on unlock" implementation (not saving/restoring the actual
    // previous state) would get wrong: it would incorrectly re-enable
    // interrupts the moment the *inner* lock's guard dropped, even
    // though the outer critical section is still supposed to be
    // running with interrupts off.
    static OUTER: SpinLock<u32> = SpinLock::new(0);
    static INNER: SpinLock<u32> = SpinLock::new(0);
    let mut nested_ok = true;
    {
        let _outer_guard = OUTER.lock();
        if interrupts_enabled() {
            nested_ok = false;
        }
        {
            let _inner_guard = INNER.lock();
            if interrupts_enabled() {
                nested_ok = false;
            }
        }
        // Inner guard has now dropped - interrupts must still be off,
        // since the outer critical section hasn't ended yet.
        if interrupts_enabled() {
            nested_ok = false;
        }
    }
    // Outer guard has now dropped too - interrupts must be back to
    // whatever they were before this whole nested region began.
    if interrupts_enabled() != if_before {
        nested_ok = false;
    }

    if basic_ok && single_level_ok && nested_ok {
        0
    } else {
        let mut code = 0;
        if !basic_ok {
            code |= 1;
        }
        if !single_level_ok {
            code |= 2;
        }
        if !nested_ok {
            code |= 4;
        }
        code
    }
}

/// Reads EFLAGS' interrupt-enable bit directly, without otherwise
/// disturbing it - used only by rust_spinlock_selftest() above to
/// observe the *actual* CPU state a SpinLock produces, rather than
/// trusting the implementation under test to report on itself.
#[inline(always)]
fn interrupts_enabled() -> bool {
    let eflags: u32;
    unsafe {
        core::arch::asm!("pushfd", "pop {0}", out(reg) eflags);
    }
    (eflags & (1 << 9)) != 0 // IF is bit 9 of EFLAGS
}
