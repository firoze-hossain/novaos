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
#define SYS_EXIT  2 /* EBX = exit code (Phase 23 - previously took no
                       argument). Terminates the calling process;
                       never returns */
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

/* Phase 14: capability-gated network access, the same pattern as
 * SYS_OPEN extended to a different resource type. EBX = destination
 * IPv4 address (host byte order, e.g. built with ip_make() - see
 * kernel/net/net.h), ECX = destination UDP port, EDX = pointer to a
 * NUL-terminated string payload (a real send syscall would take a
 * separate length instead of assuming text - see PROGRESS.md).
 * Returns 0 on success, -1 if the destination isn't in the calling
 * process's capability list or the underlying send fails. */
#define SYS_NET_SEND 7

/* Phase 17: the third capability-gated resource, this one a boolean
 * rather than a list - can this process create another process at
 * all. Takes no arguments: there's currently exactly one spawnable
 * task type (see kernel/task/greeter_task.h) since NovaOS has no
 * general exec-a-file mechanism yet, so there's nothing for an
 * argument to select between. Returns the new process's PID, or -1 if
 * the calling process wasn't granted spawn capability. */
#define SYS_SPAWN 8

/* Phase 23: loads and runs a real ELF32 executable as a new process -
 * see process_exec() in kernel/task/process.h for the full scope note
 * (this is exec-style spawn-and-load, not true fork()+exec() as two
 * steps). EBX = path (an 8.3 filename string), ECX = argv (a pointer
 * to an array of up to MAX_EXEC_ARGS char* pointers, all still valid
 * to dereference directly since a syscall never switches CR3 away
 * from the calling process - see syscall.c's header comment), EDX =
 * argc. Reuses the existing can_spawn capability from SYS_SPAWN
 * (Phase 17), broadened to mean "may create processes" generally
 * rather than narrowly "may run the one fixed greeter task." Returns
 * the new process's pid, or -1 if denied or the load failed. */
#define SYS_EXEC 9

/* EBX = pid to wait for. Blocks (yielding repeatedly, the same
 * pattern SYS_YIELD already uses from inside a syscall handler) until
 * that process reaches PROCESS_TERMINATED, then returns its exit
 * code. Returns -1 immediately if no such pid currently exists in the
 * process table at all. */
#define SYS_WAIT 10

/* Installs the int 0x80 gate with DPL=3 (required for ring-3 code to
 * invoke it via the INT instruction at all - the CPU checks CPL <= gate
 * DPL for software interrupts) and points it at the dedicated syscall
 * entry stub (see syscall_stub.asm). */
void syscall_init(void);

#endif
