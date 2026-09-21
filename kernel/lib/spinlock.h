#ifndef LIB_SPINLOCK_H
#define LIB_SPINLOCK_H

#include "../include/types.h"

/*
 * spinlock.h - Phase 57: a C-callable interrupt-safe spinlock,
 * for the C side of this kernel (kernel/task/, kernel/arch/x86/mm/,
 * kernel/arch/x86/cpu/, kernel/fs/) to actually apply the "genuinely
 * hardest part" this project's own release-readiness roadmap named:
 * auditing and locking the shared kernel structures Phase 56's own
 * SMP bring-up brought a second real CPU core online to concurrently
 * touch, which SpinLock (kernel/rust/spinlock.rs, Phase 40) could not
 * do by itself - that primitive is Rust-only, and process_table[],
 * the PMM bitmap, the heap allocator, and every filesystem driver's
 * own scratch buffers are all plain C.
 *
 * Design deliberately mirrors kernel/rust/spinlock.rs exactly, field
 * for field: disable this CPU's local interrupts for the duration the
 * lock is held (so an IRQ handler on THIS CPU can never itself race a
 * critical section already in progress here), spin on a real atomic
 * compare-and-swap (so a second CPU spinning on the same lock at the
 * same real, physical instant is still correct, not just "correct
 * enough on one core"), and save/restore the *previous* interrupt-
 * enabled state rather than unconditionally re-enabling on release -
 * required for correctly nesting one lock inside another. See that
 * Rust module's own doc comment for the fuller rationale; this is the
 * same primitive, just usable from C.
 *
 * Not reentrant: acquiring a spinlock this CPU already holds deadlocks
 * it against itself, the same documented restriction SpinLock<T> has.
 * Every call site added in this phase acquires at most one spinlock at
 * a time and never calls into another locked critical section while
 * already holding one - see each call site's own comment for why (in
 * particular, kernel/task/scheduler.c's do_schedule() never holds
 * scheduler_lock across switch_context() - see that function's own
 * comment for why that specific ordering is load-bearing, not stylistic).
 */

typedef struct {
    volatile uint32_t locked; /* 0 = free, 1 = held - accessed only via
                                  the atomic builtins below, never a
                                  plain read/write */
} spinlock_t;

void spinlock_init(spinlock_t* lock);

/* Disables interrupts on this CPU, then spins until the lock is
 * acquired. Returns the EFLAGS value captured *before* disabling
 * interrupts (bit 9 = the previous IF state) - pass this back to
 * spinlock_release() unchanged so nesting restores the right thing. */
uint32_t spinlock_acquire(spinlock_t* lock);

/* Releases the lock and restores interrupts to whatever state
 * `saved_eflags` (spinlock_acquire()'s own return value) recorded -
 * re-enabling them only if they were actually on before this critical
 * section began, not unconditionally. */
void spinlock_release(spinlock_t* lock, uint32_t saved_eflags);

/* Releases the lock's own atomic state ONLY - deliberately leaves
 * interrupts exactly as they currently are, touching nothing.
 * kernel/task/scheduler.c's own do_schedule() is the one, narrow
 * reason this exists: it must release scheduler_lock (so a second CPU
 * spinning on it isn't blocked) *before* calling switch_context(),
 * but must NOT let spinlock_release()'s own conditional `sti` re-
 * enable interrupts on THIS CPU in that same gap - doing so lets a
 * timer tick fire and recursively re-enter do_schedule() before
 * switch_context() has actually saved the current task's own context,
 * a real, confirmed source of corruption (a second, nested scheduling
 * decision running on top of a task whose own state was never
 * properly saved). The caller is responsible for restoring interrupts
 * itself, with the *original* saved_eflags from its own
 * spinlock_acquire() call, only once switch_context() actually
 * returns - see do_schedule()'s own comment for the exact sequence. */
void spinlock_release_no_restore(spinlock_t* lock);

#endif
