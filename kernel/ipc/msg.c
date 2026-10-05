/*
 * kernel/ipc/msg.c - Phase 84: the C half of app-to-app messaging. See
 * msg.h, kernel/rust/msg.rs (the real subsystem) and
 * userland/libc/include/nova_msg_abi.h (the contract).
 *
 * Two jobs only:
 *   1. the syscall entry points: validate every user pointer BEFORE
 *      touching it (a fault in kernel mode is a panic here), copy the
 *      argument struct and payload in, call Rust, copy results out;
 *   2. the msg_hal_* functions Rust uses to ask about processes and time.
 *
 * One rule is deliberately strict: on receive, the caller's output buffer
 * is validated BEFORE Rust is called. A message is consumed by the call
 * that delivers it, so a buffer the kernel then cannot write to (NULL,
 * unmapped, or a read-only shared-memory mapping) must fail the call
 * first - it must never eat the message.
 */
#include "msg.h"
#include "../arch/x86/mm/paging.h"
#include "../drivers/timer/timer.h"
#include "../include/kernel.h"
#include "../lib/string.h"
#include "../task/process.h"

/* The syscall numbers are defined once, in the shared ABI header, and
 * repeated in kernel/arch/x86/cpu/syscall.h; this stops the two copies
 * from drifting apart (the array has a negative size if they differ). */
#include "../arch/x86/cpu/syscall.h"
typedef char msg_sysno_open[(SYS_MSG_OPEN == NOVA_SYS_MSG_OPEN) ? 1 : -1];
typedef char msg_sysno_close[(SYS_MSG_CLOSE == NOVA_SYS_MSG_CLOSE) ? 1 : -1];
typedef char msg_sysno_send[(SYS_MSG_SEND == NOVA_SYS_MSG_SEND) ? 1 : -1];
typedef char msg_sysno_recv[(SYS_MSG_RECV == NOVA_SYS_MSG_RECV) ? 1 : -1];
typedef char msg_sysno_service[(SYS_MSG_SERVICE == NOVA_SYS_MSG_SERVICE) ? 1 : -1];
typedef char msg_sysno_ctl[(SYS_MSG_CTL == NOVA_SYS_MSG_CTL) ? 1 : -1];

/* ------------------------------------------------------------------
 * The hardware-abstraction functions kernel/rust/msg.rs calls.
 * ------------------------------------------------------------------ */

int msg_hal_pid_is_live(int pid) {
    return process_is_live(pid) ? 1 : 0;
}

int msg_hal_uid_of(int pid, uint32_t* out_uid) {
    return process_uid_of(pid, out_uid) ? 1 : 0;
}

uint32_t msg_hal_ticks(void) {
    return timer_get_ticks();
}

/* ------------------------------------------------------------------
 * Helpers.
 * ------------------------------------------------------------------ */

/* Length of the NUL-terminated name in a 32-byte field, or -1 if there is
 * no NUL within it (an unterminated name is malformed, not truncated). */
static int name_length(const char* field) {
    for (int i = 0; i < 32; i++) {
        if (field[i] == '\0') {
            return i;
        }
    }
    return -1;
}

/* ------------------------------------------------------------------
 * Syscall entry points.
 * ------------------------------------------------------------------ */

int msg_sys_open(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_msg_open_t), true)) {
        return -NOVA_MSG_ERR_FAULT;
    }
    nova_msg_open_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    uint32_t depth = 0, max_payload = 0;
    int rc = rust_msg_open(pid, req.policy, req.flags, &depth, &max_payload);
    if (rc != 0) {
        return rc;
    }
    req.depth = depth;
    req.max_payload = max_payload;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

int msg_sys_close(int pid, uint32_t unused) {
    (void)unused;
    return rust_msg_close(pid);
}

int msg_sys_send(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_msg_send_t), false)) {
        return -NOVA_MSG_ERR_FAULT;
    }
    nova_msg_send_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);

    /* The sender's identity is the kernel's own record, never a field of
     * the request. */
    uint32_t uid = 0;
    if (!process_uid_of(pid, &uid)) {
        return -NOVA_MSG_ERR_SRCH;
    }
    if (req.len > NOVA_MSG_MAX_PAYLOAD) {
        return -NOVA_MSG_ERR_2BIG;
    }
    uint8_t payload[NOVA_MSG_MAX_PAYLOAD];
    if (req.len > 0) {
        if (!paging_user_range_ok(req.data, req.len, false)) {
            return -NOVA_MSG_ERR_FAULT;
        }
        memcpy(payload, (const void*)req.data, req.len);
    }
    int nlen = 0;
    if (req.dest_name[0] != '\0') {
        nlen = name_length(req.dest_name);
        if (nlen < 0) {
            return -NOVA_MSG_ERR_INVAL;
        }
    }
    return rust_msg_send(pid, uid, req.dest_pid, (const uint8_t*)req.dest_name,
                         (uint32_t)nlen, req.type, req.tag, req.flags, payload,
                         req.len);
}

int msg_sys_recv(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_msg_recv_t), true)) {
        return -NOVA_MSG_ERR_FAULT;
    }
    nova_msg_recv_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);

    uint32_t cap = req.capacity;
    if (cap > NOVA_MSG_MAX_PAYLOAD) {
        cap = NOVA_MSG_MAX_PAYLOAD; /* no message is bigger */
    }
    /* Validate (and, for copy-on-write pages, resolve) the destination
     * BEFORE Rust may consume a message: see the file header. */
    if (cap > 0 && !paging_user_range_ok(req.buf, cap, true)) {
        return -NOVA_MSG_ERR_FAULT;
    }

    uint8_t payload[NOVA_MSG_MAX_PAYLOAD];
    uint32_t info[8] = {0, 0, 0, 0, 0, 0, 0, 0};
    int rc = rust_msg_recv(pid, req.flags, req.match_sender, req.match_type,
                           req.match_tag, payload, cap, info);
    if (rc == 0 || rc == -NOVA_MSG_ERR_2BIG) {
        req.sender_pid = (int)info[0];
        req.sender_uid = info[1];
        req.type = info[2];
        req.tag = info[3];
        req.len = info[4];
        req.sent_tick = info[5];
        req.pending = info[6];
        if (rc == 0 && info[4] > 0) {
            memcpy((void*)req.buf, payload, info[4]);
        }
        memcpy((void*)user_ptr, &req, sizeof req);
    }
    return rc;
}

int msg_sys_service(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_msg_service_t), true)) {
        return -NOVA_MSG_ERR_FAULT;
    }
    nova_msg_service_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    int nlen = name_length(req.name);
    if (nlen < 0) {
        return -NOVA_MSG_ERR_INVAL;
    }
    int owner = 0;
    int rc = rust_msg_service(pid, req.op, (const uint8_t*)req.name,
                              (uint32_t)nlen, &owner);
    if (rc != 0) {
        return rc;
    }
    req.pid = owner;
    memcpy((void*)user_ptr, &req, sizeof req);
    return 0;
}

int msg_sys_ctl(int pid, uint32_t user_ptr) {
    if (!paging_user_range_ok(user_ptr, sizeof(nova_msg_ctl_t), true)) {
        return -NOVA_MSG_ERR_FAULT;
    }
    nova_msg_ctl_t req;
    memcpy(&req, (const void*)user_ptr, sizeof req);
    uint32_t out[12] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0};
    int rc = rust_msg_ctl(pid, req.op, req.arg, out);
    if (rc != 0) {
        return rc;
    }
    if (req.op == NOVA_MSG_CTL_STAT) {
        req.queued = out[0];
        req.capacity = out[1];
        req.max_payload = out[2];
        req.policy = out[3];
        req.delivered = out[4];
        req.refused = out[5];
        req.next_len = out[6];
        req.next_type = out[7];
        req.next_sender = (int)out[8];
        req.now_tick = out[9];
        req.tick_hz = out[10];
        req.reserved = out[11];
        memcpy((void*)user_ptr, &req, sizeof req);
    }
    return 0;
}

void msg_init(void) {
    uint32_t state[3] = {0, 0, 0};
    uint32_t bad = rust_msg_selftest(state);
    kernel_log("[ %s ] Messaging: %d inboxes of %d messages up to %d bytes, "
               "%d service names, per-sender cap %d, clock %dHz "
               "(state %d/%d/%d)\n",
               (bad == 0 && state[0] == 0 && state[1] == 0 && state[2] == 0) ? "OK" : "FAIL",
               (int)NOVA_MSG_MAX_INBOXES, (int)NOVA_MSG_QUEUE_DEPTH,
               (int)NOVA_MSG_MAX_PAYLOAD, (int)NOVA_MSG_MAX_SERVICES,
               (int)NOVA_MSG_MAX_PER_SENDER, (int)NOVA_MSG_TICK_HZ,
               (int)state[0], (int)state[1], (int)state[2]);
}
