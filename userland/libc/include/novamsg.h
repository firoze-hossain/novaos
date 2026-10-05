#ifndef NOVAMSG_H
#define NOVAMSG_H

/*
 * novamsg.h - Phase 84: the app-side half of messaging. A thin, header-only
 * layer over the SYS_MSG_* syscalls (nova_msg_abi.h has the model and the
 * contract). It adds what the kernel deliberately does not do - WAITING -
 * because a syscall handler cannot block (interrupts are off, and this
 * kernel has no blocked-process state): the kernel returns -EAGAIN and
 * these helpers poll with SYS_YIELD against the kernel's 100Hz tick
 * counter, so timeouts are real milliseconds (10ms granularity).
 *
 * The scenario this exists for - "the file manager tells the editor to open
 * a file":
 *
 *   editor:        nova_msg_open(NOVA_MSG_ACCEPT_ANY);
 *                  nova_msg_service_register("editor");
 *                  for (;;) {
 *                      n = nova_msg_recv_wait(&info, buf, sizeof buf, 0, 0, 1000);
 *                      if (n >= 0 && info.type == MY_OPEN_FILE) {
 *                          ...open buf[0..n]...
 *                          nova_msg_send(info.sender, MY_OPENED, info.tag, "ok", 2);
 *                      }
 *                  }
 *   file manager:  nova_msg_open(NOVA_MSG_ACCEPT_ANY);
 *                  n = nova_msg_call_name("editor", MY_OPEN_FILE, path, strlen(path),
 *                                         &reply_info, reply, sizeof reply, 500);
 *
 * Message TYPES are yours to define (non-zero); a TAG is a correlation id.
 * nova_msg_call() picks a fresh tag and waits for the reply that carries it
 * from the process it asked, leaving every other message in the inbox.
 *
 * Every function returns 0 / a non-negative result (a length, a pid), or a
 * NEGATIVE errno. "Timed out" is -EAGAIN: nothing arrived in time.
 */

#include <errno.h>
#include <novasys.h>
#include <string.h>

/* The kernel cannot include errno.h, so nova_msg_abi.h repeats the numbers
 * it returns. If either side is ever edited out of step this stops the
 * build instead of letting a program mis-handle an error. */
typedef char novamsg_errno_perm [(NOVA_MSG_ERR_PERM  == EPERM)  ? 1 : -1];
typedef char novamsg_errno_noent[(NOVA_MSG_ERR_NOENT == ENOENT) ? 1 : -1];
typedef char novamsg_errno_srch [(NOVA_MSG_ERR_SRCH  == ESRCH)  ? 1 : -1];
typedef char novamsg_errno_2big [(NOVA_MSG_ERR_2BIG  == E2BIG)  ? 1 : -1];
typedef char novamsg_errno_badf [(NOVA_MSG_ERR_BADF  == EBADF)  ? 1 : -1];
typedef char novamsg_errno_again[(NOVA_MSG_ERR_AGAIN == EAGAIN) ? 1 : -1];
typedef char novamsg_errno_acces[(NOVA_MSG_ERR_ACCES == EACCES) ? 1 : -1];
typedef char novamsg_errno_fault[(NOVA_MSG_ERR_FAULT == EFAULT) ? 1 : -1];
typedef char novamsg_errno_exist[(NOVA_MSG_ERR_EXIST == EEXIST) ? 1 : -1];
typedef char novamsg_errno_inval[(NOVA_MSG_ERR_INVAL == EINVAL) ? 1 : -1];
typedef char novamsg_errno_nospc[(NOVA_MSG_ERR_NOSPC == ENOSPC) ? 1 : -1];

/* What a receive tells you about the message it delivered. */
typedef struct {
    int sender;                 /* pid, stamped by the kernel */
    unsigned int sender_uid;    /* uid, stamped by the kernel */
    unsigned int type;
    unsigned int tag;
    unsigned int len;
    unsigned int sent_tick;     /* kernel tick (100Hz) when it was sent */
    unsigned int pending;       /* messages left in the inbox */
} nova_msg_info_t;

/* ---- inbox ----------------------------------------------------------- */

/* Opens this process's inbox. `policy` is one of NOVA_MSG_ACCEPT_*. */
static inline int nova_msg_open(unsigned int policy) {
    nova_msg_open_t o;
    o.policy = policy;
    o.flags = 0;
    o.depth = 0;
    o.max_payload = 0;
    return sys_msg_open(&o);
}

static inline int nova_msg_close(void) {
    return sys_msg_close();
}

/* ---- sending --------------------------------------------------------- */

static inline int nova_msg_send_raw(int pid, const char* service, unsigned int type, unsigned int tag,
                                    const void* data, unsigned int len) {
    nova_msg_send_t s;
    memset(&s, 0, sizeof s);
    s.dest_pid = pid;
    if (service != 0) {
        unsigned int n = (unsigned int)strlen(service);
        if (n == 0 || n > NOVA_MSG_NAME_MAX) {
            return -EINVAL;
        }
        memcpy(s.dest_name, service, n);
    }
    s.type = type;
    s.tag = tag;
    s.flags = 0;
    s.len = len;
    s.data = (unsigned int)(unsigned long)data;
    return sys_msg_send(&s);
}

/* Send to a process by pid. -EAGAIN means "full, try again". */
static inline int nova_msg_send(int pid, unsigned int type, unsigned int tag, const void* data, unsigned int len) {
    return nova_msg_send_raw(pid, 0, type, tag, data, len);
}

/* Send to whichever live process has registered `service`. */
static inline int nova_msg_send_name(const char* service, unsigned int type, unsigned int tag,
                                     const void* data, unsigned int len) {
    return nova_msg_send_raw(0, service, type, tag, data, len);
}

/* ---- receiving --------------------------------------------------------- */

/* One non-blocking receive attempt with every filter. `from` 0 = any
 * sender, `type` 0 = any type, `tag` only if flags has NOVA_MSG_RECV_MATCH_TAG.
 * Returns the message length, or -EAGAIN (nothing matching), -E2BIG (buffer
 * too small: info->len says how big; the message is NOT consumed), -EBADF
 * (no inbox open). With NOVA_MSG_RECV_PEEK the message stays queued. */
static inline int nova_msg_try_recv_ex(nova_msg_info_t* info, void* buf, unsigned int cap, unsigned int flags,
                                       int from, unsigned int type, unsigned int tag) {
    nova_msg_recv_t r;
    memset(&r, 0, sizeof r);
    r.flags = flags;
    r.match_sender = from;
    r.match_type = type;
    r.match_tag = tag;
    r.buf = (unsigned int)(unsigned long)buf;
    r.capacity = cap;
    int rc = sys_msg_recv(&r);
    if ((rc == 0 || rc == -E2BIG) && info != 0) {
        info->sender = r.sender_pid;
        info->sender_uid = r.sender_uid;
        info->type = r.type;
        info->tag = r.tag;
        info->len = r.len;
        info->sent_tick = r.sent_tick;
        info->pending = r.pending;
    }
    return rc == 0 ? (int)r.len : rc;
}

/* The oldest message, whatever it is. */
static inline int nova_msg_try_recv(nova_msg_info_t* info, void* buf, unsigned int cap) {
    return nova_msg_try_recv_ex(info, buf, cap, 0, 0, 0, 0);
}

/* Milliseconds -> kernel ticks, rounded UP so a wait is never shorter than
 * asked. 32-bit arithmetic on purpose: this runs freestanding, without
 * libgcc's 64-bit division. Saturates rather than overflowing. */
static inline unsigned int nova_msg_ms_to_ticks(unsigned int ms) {
    unsigned int ms_per_tick = 1000u / NOVA_MSG_TICK_HZ;
    if (ms > 0xFFFFFFFFu - ms_per_tick) {
        return 0xFFFFFFFFu / ms_per_tick;
    }
    return (ms + ms_per_tick - 1u) / ms_per_tick;
}

/* The kernel tick counter (100Hz): the clock for timeouts. Needs an open inbox. */
static inline unsigned int nova_msg_ticks(void) {
    nova_msg_ctl_t c;
    memset(&c, 0, sizeof c);
    c.op = NOVA_MSG_CTL_STAT;
    return sys_msg_ctl(&c) == 0 ? c.now_tick : 0;
}

/* Waits up to `timeout_ms` (10ms granularity) for a matching message,
 * polling with SYS_YIELD. Returns as try_recv_ex does, with a timeout
 * reported as -EAGAIN. A timeout of 0 is a single attempt. */
static inline int nova_msg_recv_wait_ex(nova_msg_info_t* info, void* buf, unsigned int cap, unsigned int flags,
                                        int from, unsigned int type, unsigned int tag, unsigned int timeout_ms) {
    unsigned int start = nova_msg_ticks();
    unsigned int budget = nova_msg_ms_to_ticks(timeout_ms);
    for (;;) {
        int rc = nova_msg_try_recv_ex(info, buf, cap, flags, from, type, tag);
        if (rc != -EAGAIN) {
            return rc;
        }
        if (budget == 0 || (int)(nova_msg_ticks() - start) >= (int)budget) {
            return -EAGAIN;
        }
        sys_yield();
    }
}

/* The common case: the oldest message from `from` (0 = anyone) of `type`
 * (0 = any type), waiting up to `timeout_ms`. */
static inline int nova_msg_recv_wait(nova_msg_info_t* info, void* buf, unsigned int cap, int from,
                                     unsigned int type, unsigned int timeout_ms) {
    return nova_msg_recv_wait_ex(info, buf, cap, 0, from, type, 0, timeout_ms);
}

/* Sleeps about `ms` milliseconds without consuming messages (needs an inbox). */
static inline void nova_msg_sleep_ms(unsigned int ms) {
    unsigned int start = nova_msg_ticks();
    unsigned int budget = nova_msg_ms_to_ticks(ms);
    while ((int)(nova_msg_ticks() - start) < (int)budget) {
        sys_yield();
    }
}

/* ---- service names ------------------------------------------------------- */

static inline int nova_msg_service_op(unsigned int op, const char* name) {
    nova_msg_service_t s;
    memset(&s, 0, sizeof s);
    unsigned int n = (unsigned int)strlen(name);
    if (n == 0 || n > NOVA_MSG_NAME_MAX) {
        return -EINVAL;
    }
    memcpy(s.name, name, n);
    s.op = op;
    int rc = sys_msg_service(&s);
    return rc < 0 ? rc : s.pid;
}

static inline int nova_msg_service_register(const char* name) {
    return nova_msg_service_op(NOVA_MSG_SVC_REGISTER, name);
}
static inline int nova_msg_service_unregister(const char* name) {
    return nova_msg_service_op(NOVA_MSG_SVC_UNREGISTER, name);
}
/* Returns the pid of the live process that holds `name`, or -ENOENT. */
static inline int nova_msg_service_lookup(const char* name) {
    return nova_msg_service_op(NOVA_MSG_SVC_LOOKUP, name);
}

/* ---- policy ------------------------------------------------------------------- */

static inline int nova_msg_ctl_op(unsigned int op, unsigned int arg) {
    nova_msg_ctl_t c;
    memset(&c, 0, sizeof c);
    c.op = op;
    c.arg = arg;
    return sys_msg_ctl(&c);
}
static inline int nova_msg_allow(int pid)  { return nova_msg_ctl_op(NOVA_MSG_CTL_ALLOW, (unsigned int)pid); }
static inline int nova_msg_deny(int pid)   { return nova_msg_ctl_op(NOVA_MSG_CTL_DENY, (unsigned int)pid); }
static inline int nova_msg_set_policy(unsigned int policy) { return nova_msg_ctl_op(NOVA_MSG_CTL_POLICY, policy); }

static inline int nova_msg_stat(nova_msg_ctl_t* out) {
    memset(out, 0, sizeof *out);
    out->op = NOVA_MSG_CTL_STAT;
    return sys_msg_ctl(out);
}

/* ---- request / response --------------------------------------------------------- */

/* Sends a request to process `pid` and waits for ITS reply, which must
 * carry the fresh tag chosen here: other messages in the inbox (including
 * other replies) are left untouched. If the peer's inbox is momentarily
 * full the send is retried until the timeout. Returns the reply's length,
 * or a negative errno (-EAGAIN: no reply in time). This process needs an
 * inbox of its own to receive the reply. */
static inline int nova_msg_call(int pid, unsigned int type, const void* req, unsigned int req_len,
                                nova_msg_info_t* reply_info, void* reply, unsigned int reply_cap,
                                unsigned int timeout_ms) {
    static unsigned int next_tag = 0x00A00000u;
    unsigned int tag = ++next_tag;
    unsigned int start = nova_msg_ticks();
    unsigned int budget = nova_msg_ms_to_ticks(timeout_ms);
    int rc;
    while ((rc = nova_msg_send(pid, type, tag, req, req_len)) == -EAGAIN) {
        if ((int)(nova_msg_ticks() - start) >= (int)budget) {
            return -EAGAIN;
        }
        sys_yield();
    }
    if (rc < 0) {
        return rc;
    }
    unsigned int elapsed = nova_msg_ticks() - start;
    unsigned int left_ms = elapsed >= budget ? 0u : (budget - elapsed) * (1000u / NOVA_MSG_TICK_HZ);
    return nova_msg_recv_wait_ex(reply_info, reply, reply_cap, NOVA_MSG_RECV_MATCH_TAG, pid, 0, tag, left_ms);
}

/* The same, to a service by name. */
static inline int nova_msg_call_name(const char* service, unsigned int type, const void* req, unsigned int req_len,
                                     nova_msg_info_t* reply_info, void* reply, unsigned int reply_cap,
                                     unsigned int timeout_ms) {
    int pid = nova_msg_service_lookup(service);
    if (pid < 0) {
        return pid;
    }
    return nova_msg_call(pid, type, req, req_len, reply_info, reply, reply_cap, timeout_ms);
}

#endif
