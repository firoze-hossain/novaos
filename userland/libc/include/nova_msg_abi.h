#ifndef NOVA_MSG_ABI_H
#define NOVA_MSG_ABI_H

/*
 * nova_msg_abi.h - Phase 84: the ABI of NovaOS's app-to-app messaging
 * syscalls (SYS_MSG_*), shared verbatim by the kernel (kernel/ipc/msg.c,
 * kernel/rust/msg.rs's tests) and userland (novamsg.h, novasys.h): ONE
 * definition rather than two copies that can drift. Dependency-free: plain
 * `unsigned int` / `int` (32 bits on this i686 target - checked by the
 * size assertions at the bottom). A host test in kernel/rust/msg.rs reads
 * THIS FILE and fails if any limit or code below disagrees with the
 * kernel's own constant.
 *
 * WHY THIS EXISTS - what pipes cannot do
 *
 * "The file manager tells the text editor to open a file." A pipe (Phase
 * 36) cannot carry that:
 *   - it is a BYTE STREAM: two messages written back to back are
 *     indistinguishable from one, so every user invents its own framing;
 *   - it has no IDENTITY: the reader cannot tell who wrote, and several
 *     writers' bytes interleave;
 *   - it has no ADDRESS: you must already hold the pipe's handle, which
 *     only fork() can give you - unrelated apps cannot find each other;
 *   - it is 1KB, and a reader that wants only one kind of message must
 *     read everything and keep the rest.
 *
 * THE MODEL
 *
 *   INBOX     A process opts in to receiving with MSG_OPEN. Its inbox is a
 *             bounded FIFO of whole messages. A process with no inbox
 *             cannot be sent to, so nothing can fill memory on behalf of a
 *             process that never asked to listen.
 *   MESSAGE   One send is one receive: a type (non-zero, app-defined), a
 *             tag (app-defined correlation id), and up to
 *             NOVA_MSG_MAX_PAYLOAD bytes. The KERNEL stamps the sender's
 *             pid, uid and the send time; a sender cannot forge them.
 *             Bigger data goes through shared memory (nova_shm_abi.h) with
 *             the message carrying the handle.
 *   ADDRESS   By pid, or by SERVICE NAME: a process registers "editor" and
 *             anyone can send to "editor" without knowing its pid. Names
 *             are released when the owner exits.
 *   RECEIVE   Oldest first; or, selectively, the oldest from a given
 *             sender / of a given type / with a given tag - which is what
 *             request/response needs (wait for THE reply, leave the rest).
 *   POLICY    The receiver decides who may send: anyone, only the same
 *             user (or root), or an explicit allowlist. Refused messages
 *             never occupy queue space.
 *   FAIRNESS  No single sender may hold more than NOVA_MSG_MAX_PER_SENDER
 *             slots of one inbox, so a flood from one process cannot lock
 *             everyone else out of a service.
 *
 * NOTHING BLOCKS IN THE KERNEL. A syscall handler runs with interrupts
 * disabled, and this kernel has no blocked-process state at all (even
 * wait() is a yield loop), so a call that cannot complete returns -EAGAIN
 * and the caller waits with SYS_YIELD - the same convention as pipes,
 * SYS_READ_KEY and SYS_WAIT. novamsg.h wraps that into recv_wait() and
 * call() with real millisecond timeouts, using the tick counter MSG_CTL
 * exposes (100Hz).
 *
 * LIFETIME. Messages are not persistent: a process that exits loses its
 * inbox, its queued messages and its service names. fork() gives the child
 * nothing (no inbox, no names, no copy of the parent's queue).
 *
 * Every call returns 0 or a non-negative result on success and a NEGATIVE
 * errno on failure (not -1 plus errno).
 */

/* Syscall numbers (kernel/arch/x86/cpu/syscall.h). */
#define NOVA_SYS_MSG_OPEN     60 /* EBX = nova_msg_open_t* (in/out) */
#define NOVA_SYS_MSG_CLOSE    61 /* no argument */
#define NOVA_SYS_MSG_SEND     62 /* EBX = nova_msg_send_t* */
#define NOVA_SYS_MSG_RECV     63 /* EBX = nova_msg_recv_t* (in/out) */
#define NOVA_SYS_MSG_SERVICE  64 /* EBX = nova_msg_service_t* (in/out) */
#define NOVA_SYS_MSG_CTL      65 /* EBX = nova_msg_ctl_t* (in/out) */

/* Limits. */
#define NOVA_MSG_MAX_PAYLOAD      1024u
#define NOVA_MSG_QUEUE_DEPTH      16u
#define NOVA_MSG_MAX_PER_SENDER   8u
#define NOVA_MSG_MAX_INBOXES      32u
#define NOVA_MSG_MAX_SERVICES     32u
#define NOVA_MSG_MAX_NAMES_PER_PROC 4u
#define NOVA_MSG_MAX_ALLOW        8u
#define NOVA_MSG_NAME_MAX         31u   /* characters; the field is 32 with the NUL */
#define NOVA_MSG_TICK_HZ          100u

/* Who may send to an inbox (MSG_OPEN policy, MSG_CTL policy change). */
#define NOVA_MSG_ACCEPT_ANY       0u
#define NOVA_MSG_ACCEPT_SAME_UID  1u  /* sender's uid == receiver's, or sender is root */
#define NOVA_MSG_ACCEPT_ALLOWLIST 2u  /* only pids added with NOVA_MSG_CTL_ALLOW */

/* MSG_RECV flags. */
#define NOVA_MSG_RECV_PEEK        1u  /* look, do not remove */
#define NOVA_MSG_RECV_MATCH_TAG   2u  /* only a message whose tag == match_tag */

/* MSG_SERVICE operations. */
#define NOVA_MSG_SVC_REGISTER     1u
#define NOVA_MSG_SVC_UNREGISTER   2u
#define NOVA_MSG_SVC_LOOKUP       3u

/* MSG_CTL operations. */
#define NOVA_MSG_CTL_STAT         1u
#define NOVA_MSG_CTL_ALLOW        2u  /* arg = pid */
#define NOVA_MSG_CTL_DENY         3u  /* arg = pid */
#define NOVA_MSG_CTL_POLICY       4u  /* arg = NOVA_MSG_ACCEPT_* */

/* MSG_OPEN: in: policy, flags (must be 0). out: depth, max_payload. */
typedef struct {
    unsigned int policy;
    unsigned int flags;
    unsigned int depth;
    unsigned int max_payload;
} nova_msg_open_t;

/* MSG_SEND. Destination: dest_name if dest_name[0] != 0, else dest_pid.
 * type must be non-zero. flags must be 0. `data` is a user pointer to `len`
 * bytes (may be 0 with len 0). Errors:
 *   -EINVAL bad type/flags/name/pid   -E2BIG  len > MAX_PAYLOAD
 *   -ESRCH  no such process           -ENOENT no such service, or the
 *                                              process has no inbox
 *   -EACCES the receiver's policy refuses you
 *   -EAGAIN the queue is full, or you hold your per-sender share: retry
 *   -EFAULT bad pointer */
typedef struct {
    int dest_pid;
    char dest_name[32];
    unsigned int type;
    unsigned int tag;
    unsigned int flags;
    unsigned int len;
    unsigned int data;
} nova_msg_send_t;

/* MSG_RECV: never blocks. in: flags, match_sender (0 = any), match_type
 * (0 = any), match_tag (used with NOVA_MSG_RECV_MATCH_TAG), buf, capacity.
 * out (filled whenever a matching message exists, even on -E2BIG):
 * sender_pid, sender_uid, type, tag, len, sent_tick, pending (messages
 * left in the queue after this call; for PEEK, including this one).
 *   -EAGAIN no (matching) message      -EBADF  you have no inbox
 *   -E2BIG  buffer too small: the message is NOT consumed and `len` says
 *           how big a buffer it needs (PEEK with capacity 0 asks "how big?")
 *   -EFAULT buf is not writable memory (including a read-only shared
 *           mapping): the message is NOT consumed */
typedef struct {
    unsigned int flags;
    int match_sender;
    unsigned int match_type;
    unsigned int match_tag;
    unsigned int buf;
    unsigned int capacity;
    int sender_pid;
    unsigned int sender_uid;
    unsigned int type;
    unsigned int tag;
    unsigned int len;
    unsigned int sent_tick;
    unsigned int pending;
    unsigned int reserved;
} nova_msg_recv_t;

/* MSG_SERVICE: op, name. LOOKUP fills pid.
 *   REGISTER   needs an inbox (-EBADF), a valid name ([A-Za-z0-9._-], 1..31
 *              chars, else -EINVAL), a free name (-EEXIST if another live
 *              process holds it; re-registering your own is a no-op), room
 *              (-ENOSPC: at most MAX_NAMES_PER_PROC per process).
 *   UNREGISTER only the owner (-EPERM), -ENOENT if unknown.
 *   LOOKUP     any process; -ENOENT if no live process holds the name. */
typedef struct {
    unsigned int op;
    char name[32];
    int pid;
} nova_msg_service_t;

/* MSG_CTL: op, arg. STAT fills the rest (it works for the caller's own
 * inbox only). The tick counter is global, so it is also the clock for
 * timeouts. */
typedef struct {
    unsigned int op;
    unsigned int arg;
    unsigned int queued;
    unsigned int capacity;
    unsigned int max_payload;
    unsigned int policy;
    unsigned int delivered;   /* messages this inbox has handed out */
    unsigned int refused;     /* sends refused by policy or fullness */
    unsigned int next_len;    /* the oldest message, if any */
    unsigned int next_type;
    int next_sender;
    unsigned int now_tick;
    unsigned int tick_hz;
    unsigned int reserved;
} nova_msg_ctl_t;

/* Error numbers. The kernel cannot include <errno.h>; these repeat the
 * errno values it returns, and novamsg.h asserts at compile time that they
 * still match userland's errno.h. */
#define NOVA_MSG_ERR_PERM    1
#define NOVA_MSG_ERR_NOENT   2
#define NOVA_MSG_ERR_SRCH    3
#define NOVA_MSG_ERR_2BIG    7
#define NOVA_MSG_ERR_BADF    9
#define NOVA_MSG_ERR_AGAIN  11
#define NOVA_MSG_ERR_ACCES  13
#define NOVA_MSG_ERR_FAULT  14
#define NOVA_MSG_ERR_EXIST  17
#define NOVA_MSG_ERR_INVAL  22
#define NOVA_MSG_ERR_NOSPC  28

/* Compile-time ABI checks (the array-size trick; works in C89). */
typedef char nova_msg_abi_check_open[(sizeof(nova_msg_open_t) == 16) ? 1 : -1];
typedef char nova_msg_abi_check_send[(sizeof(nova_msg_send_t) == 56) ? 1 : -1];
typedef char nova_msg_abi_check_recv[(sizeof(nova_msg_recv_t) == 56) ? 1 : -1];
typedef char nova_msg_abi_check_service[(sizeof(nova_msg_service_t) == 40) ? 1 : -1];
typedef char nova_msg_abi_check_ctl[(sizeof(nova_msg_ctl_t) == 56) ? 1 : -1];

#endif
