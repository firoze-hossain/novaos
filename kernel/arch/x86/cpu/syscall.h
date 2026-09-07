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

/* Phase 24: EBX = increment (bytes, must be >= 0). Grows the calling
 * process's heap and returns the *previous* break address, or
 * (uint32_t)-1 on failure - see process_sbrk() in
 * kernel/task/process.h for the full semantics. No capability check:
 * this only ever manages the calling process's own memory, the same
 * "no gate needed" reasoning SYS_WRITE and SYS_YIELD already use. */
#define SYS_SBRK 11

/* Phase 27: true fork() - no arguments. Returns the child's pid to
 * the parent, or 0 to the child (the standard Unix fork() contract) -
 * see process_fork() in kernel/task/process.h for the full mechanism.
 * No capability check: forking duplicates only the calling process's
 * own resources, the same "no gate needed" reasoning SYS_SBRK already
 * uses. */
#define SYS_FORK 12

/* Phase 30: non-blocking keyboard read - no arguments. Returns the
 * next decoded ASCII character if one is waiting, or -1 immediately
 * if not. Deliberately non-blocking: a syscall handler runs with
 * interrupts disabled (see syscall_stub.asm), so a genuinely blocking
 * read here - waiting on the same IRQ1 interrupt that can never fire
 * while interrupts are off - would deadlock the entire kernel, not
 * just the calling process. A ring-3 caller wanting to actually wait
 * for a key is expected to loop this with SYS_YIELD in between, the
 * same pattern every other blocking wait in this kernel already uses
 * one level up (process_wait(), tcp_receive(), etc.). */
#define SYS_READ_KEY 13

/* Phase 30: EBX = buffer, ECX = buffer size. Fills the buffer with a
 * newline-separated directory listing ("NAME SIZE\n" per entry,
 * SIZE in bytes) of the mounted FAT32 root directory, and returns the
 * number of bytes written, or -1 if the buffer was too small or no
 * filesystem is mounted. No capability check: listing what files
 * exist isn't itself a read of any file's contents, the same
 * reasoning `ls` needing no special permission on a real Unix system
 * reflects (though opening any *specific* file still goes through
 * SYS_OPEN's existing capability check, unaffected by this). */
#define SYS_LIST_FILES 14

/* Phase 31: restoring shell command parity after Phase 30's ring-3
 * conversion. Each of these reads existing, already-safe-to-call
 * kernel state - no new capability gate needed, the same "no gate
 * needed" reasoning SYS_WRITE/SYS_YIELD/SYS_SBRK already use. */

/* EBX = buffer (6 bytes: year_lo, year_hi, month, day, hour, minute,
 * second - 7 bytes total, matching rtc_time_t's layout exactly so the
 * caller can just memcpy it onto a local rtc_time_t). Always
 * succeeds (the RTC itself has no failure mode this kernel detects). */
#define SYS_RTC_READ 15

/* EBX = buffer, ECX = buffer size. Fills the buffer with a newline-
 * separated PCI device listing ("BUS:DEV.FN VENDOR:DEVICE CLASS\n"
 * per entry, matching the existing `lspci` kernel-side formatting)
 * and returns the number of bytes written, or -1 if the buffer was
 * too small. */
#define SYS_LSPCI 16

/* No arguments. Plays the same short tone the kernel's own `beep`
 * self-test does. Returns 1 if AC97 hardware was found and the beep
 * was issued, 0 if no AC97 device is present. */
#define SYS_BEEP 17

/* Phase 32: converting the package manager to ring-3 (a shell
 * builtin, not a separate exec'd binary - see PROGRESS.md for why:
 * exec'd programs start with zero capabilities by default (Phase
 * 23), and there is no mechanism yet for the shell to delegate a
 * subset of its own broad access to something it execs, so a
 * genuinely separate pkg.elf couldn't actually read/write/delete
 * anything). These two are gated by the same can_open_any_file
 * capability SYS_OPEN already checks (Phase 30) - broadened in
 * meaning from "may open any file" to "has broad, trusted file
 * access", covering write and delete too, rather than adding a
 * second, separate capability for what is, for the one process that
 * has it (the shell), the same trust decision.
 *
 * EBX = filename, ECX = data pointer, EDX = size. Whole-file,
 * atomic writes only (matching vfs_write_file()'s own semantics) -
 * no partial/streamed writes. Returns 1 on success, -1 on failure
 * (no capability, or the underlying filesystem write failed). */
#define SYS_WRITE_FILE 18

/* EBX = filename. Returns 1 on success, -1 on failure (no
 * capability, or the file didn't exist / underlying delete failed). */
#define SYS_DELETE_FILE 19

/* Phase 32b: the foundational syscalls a ring-3 graphics program
 * needs - graphics mode switching, drawing primitives, and mouse
 * input. Deliberately scoped to a proof-of-concept ring-3 graphics
 * demo in this pass, not a full port of the existing compositor's
 * multi-window management or the Store's package-browsing UI - see
 * PROGRESS.md for why that's a substantially larger undertaking left
 * as honest follow-up work. No capability gate: entering graphics
 * mode and drawing to it isn't a read of anything sensitive, the
 * same reasoning SYS_WRITE/SYS_LIST_FILES already use. */

#define SYS_GFX_ENTER 20 /* no args - enters VGA Mode 13h (320x200x256) */
#define SYS_GFX_EXIT 21  /* no args - restores text mode and clears it */

/* EBX = x, ECX = y, EDX = color index (0-255, VGA's default 256-
 * color palette). */
#define SYS_GFX_PUT_PIXEL 22

/* EBX = pointer to a 5-int-wide {x, y, w, h, color} buffer - more
 * fields than fit in three registers, so passed as a small buffer the
 * kernel reads from rather than adding a fourth/fifth register
 * argument convention just for this one syscall. */
#define SYS_GFX_FILL_RECT 23

/* EBX = pointer to a caller-provided buffer matching
 * kernel/drivers/mouse/ps2mouse.h's mouse_state_t layout exactly
 * (dx, dy as ints, then three bool button flags) - written directly,
 * the same pattern SYS_RTC_READ already uses for rtc_time_t. Returns
 * 1 if a PS/2 mouse was detected at boot, 0 otherwise (matching
 * ps2mouse_is_present()) - the buffer is only meaningfully filled in
 * the former case. */
#define SYS_MOUSE_READ 24

/* Phase 33: real ping, done safely from ring-3. Deliberately split
 * into two non-blocking syscalls rather than one that waits - a
 * syscall handler runs with interrupts disabled for its whole
 * duration (see syscall_stub.asm), and icmp_ping()'s own ~3-second
 * wait depends on two different hardware IRQs (the NIC's, for a
 * reply to ever be seen; the timer's, for its own deadline check to
 * advance at all) that could never fire during that window - turning
 * a bounded 3-second network wait into an unbounded hang of the
 * entire kernel, not just the calling process. The exact same class
 * of mistake SYS_READ_KEY (Phase 30) was designed to avoid, applied
 * here to networking instead of the keyboard. A ring-3 caller wanting
 * to wait for the result loops SYS_PING_POLL with SYS_YIELD between
 * calls, the same pattern every other blocking wait in this kernel
 * already uses one level up. No capability gate: pinging a host isn't
 * a read of anything sensitive, the same reasoning
 * SYS_WRITE/SYS_LIST_FILES already use. */

/* EBX = destination IPv4 address, as a big-endian-packed uint32_t
 * (matching every other IP address this kernel already passes around
 * - see ip_send()'s own parameter). Sends the Echo Request and
 * returns immediately; no return value. */
#define SYS_PING_START 25

/* EBX = pointer to a uint32_t the RTT (in timer ticks) is written to
 * if a reply has arrived. Returns 1 if the reply arrived (RTT
 * filled in), 0 if still waiting and the ~3s deadline hasn't passed,
 * -1 if the deadline passed with no reply. */
#define SYS_PING_POLL 26

/* Installs the int 0x80 gate with DPL=3 (required for ring-3 code to
 * invoke it via the INT instruction at all - the CPU checks CPL <= gate
 * DPL for software interrupts) and points it at the dedicated syscall
 * entry stub (see syscall_stub.asm). */
void syscall_init(void);

#endif
