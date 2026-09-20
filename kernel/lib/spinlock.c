/*
 * spinlock.c - see spinlock.h for the full design rationale.
 *
 * Uses GCC's __atomic built-ins (real `lock cmpxchg`/`lock xchg`
 * machine instructions on x86, not a library call - freestanding-safe,
 * no libatomic dependency) rather than hand-written inline asm, purely
 * for readability; the generated code is the same either way. `pushf`/
 * `popf` (not `cli`/`sti` alone) capture and restore the *entire*
 * previous EFLAGS-derived interrupt state precisely, the same
 * technique kernel/rust/spinlock.rs's own `cli_and_save_eflags()`
 * uses.
 */
#include "spinlock.h"

void spinlock_init(spinlock_t* lock) {
    __atomic_store_n(&lock->locked, 0u, __ATOMIC_RELEASE);
}

uint32_t spinlock_acquire(spinlock_t* lock) {
    uint32_t saved_eflags;
    __asm__ volatile (
        "pushf\n\t"
        "pop %0\n\t"
        "cli"
        : "=r"(saved_eflags)
        :
        : "memory"
    );

    /* Not the "weak" compare-exchange form: a spurious failure on a
     * platform without real 32-bit CAS would need this retried anyway,
     * and x86 always has a real `lock cmpxchg`, so "strong" costs
     * nothing here and keeps the loop's own logic simpler to read. */
    for (;;) {
        uint32_t expected = 0u;
        if (__atomic_compare_exchange_n(&lock->locked, &expected, 1u, 0,
                                         __ATOMIC_ACQUIRE, __ATOMIC_RELAXED)) {
            break;
        }
        /* core::hint::spin_loop()'s C equivalent - a `pause`
         * instruction between failed attempts, cheap and standard on
         * x86 to avoid needlessly hammering the cache-coherency bus
         * while spinning (real, if minor, benefit now that a second
         * physical core can genuinely be spinning on the same lock at
         * the same time - not something a single-core kernel ever
         * needed to care about before Phase 56). */
        __asm__ volatile ("pause" ::: "memory");
    }

    return saved_eflags;
}

void spinlock_release(spinlock_t* lock, uint32_t saved_eflags) {
    __atomic_store_n(&lock->locked, 0u, __ATOMIC_RELEASE);

    /* Bit 9 of EFLAGS is IF (the interrupt-enable flag) - restore
     * only if it was actually set before spinlock_acquire() disabled
     * it, not unconditionally, so a lock acquired from within an
     * already-cli'd context (e.g. one critical section nested inside
     * another) doesn't prematurely re-enable interrupts the outer
     * context still needs off. */
    if (saved_eflags & 0x200u) {
        __asm__ volatile ("sti" ::: "memory");
    }
}
