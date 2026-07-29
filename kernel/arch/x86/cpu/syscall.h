#ifndef ARCH_X86_CPU_SYSCALL_H
#define ARCH_X86_CPU_SYSCALL_H

#include "isr.h"

#define SYSCALL_VECTOR 0x80

/* Convention (deliberately simple, not Linux-ABI-compatible): syscall
 * number in EAX, arguments in EBX/ECX/EDX. SYS_WRITE/SYS_EXIT/SYS_YIELD
 * are fire-and-forget; the file I/O syscalls added in Phase 11 return
 * a value in EAX (syscall_handler() writes it into registers_t.eax,
 * which is exactly the in-memory location the entry stub's final
 * `popa` restores EAX from - no separate return-value channel needed). */
#define SYS_WRITE 1 /* EBX = pointer to a NUL-terminated string to print */
#define SYS_EXIT  2 /* terminates the calling process; never returns    */
#define SYS_YIELD 3 /* voluntarily gives up the rest of this quantum    */

/* Phase 11: capability-gated file access. A process can only SYS_OPEN
 * a filename that was explicitly granted to it at creation time (see
 * process_create_sandboxed_task() in kernel/task/process.h) - this is
 * the actual security boundary, enforced in the kernel, not something
 * ring-3 code can bypass by asking nicely. Returns -1 for "not in
 * this process's capability list" and for "capability granted but the
 * file doesn't exist on disk" alike - deliberately not distinguishing
 * the two to a ring-3 caller, so probing for which files *exist*
 * isn't a way to learn something a denied SYS_OPEN wouldn't have
 * revealed anyway. The kernel log does distinguish them, for whoever
 * is allowed to read it. */
#define SYS_OPEN  4 /* EBX = filename pointer. Returns a handle, or -1  */
#define SYS_READ  5 /* EBX = handle, ECX = buffer, EDX = max length.
                       Returns bytes read, or -1                       */
#define SYS_CLOSE 6 /* EBX = handle                                    */

/* Installs the int 0x80 gate with DPL=3 (required for ring-3 code to
 * invoke it via the INT instruction at all - the CPU checks CPL <= gate
 * DPL for software interrupts) and points it at the dedicated syscall
 * entry stub (see syscall_stub.asm). */
void syscall_init(void);

#endif
