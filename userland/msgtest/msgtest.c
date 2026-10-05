/*
 * msgtest.c - Phase 84: the in-OS conformance test for the SYS_MSG_*
 * messaging syscalls. A real ring-3 program making real int 0x80 calls
 * against the real kernel, and - because messaging is only meaningful
 * between processes - fork()ing real peers to talk to. kernel/task/
 * exec_trust_demo.c runs it at boot and tools/python/test_runner.py checks
 * its "[msgtest] ok:" / "[msgtest] PASS" lines. It is the in-OS counterpart
 * of the host tests (kernel/rust/msg.rs against a mock process table and an
 * independent model): those prove the logic, this proves the whole stack -
 * syscall, user-pointer validation, kernel-stamped identity, fork, process
 * exit, the tick clock, and cooperation with shared memory.
 *
 * It is organised around the roadmap's own scenario - "the file manager
 * tells the text editor to open a file" - and the properties that make that
 * safe and useful, each of which is a way the feature could be wrong in a
 * way a plain "it returned 0" would miss:
 *
 *  - FRAMING: one send is exactly one receive, for sizes 0, 1, 500, 1024.
 *  - IDENTITY: the sender pid and uid on a message are the kernel's record
 *    of the sender, checked against the truth (the pid learned from the
 *    service registry; sys_getuid()).
 *  - NOTHING IS LOST OR TRUNCATED: a too-small buffer fails with -E2BIG,
 *    leaves the message queued and reports the size needed; a bad buffer
 *    (NULL, kernel, unmapped, a READ-ONLY shared-memory mapping) fails
 *    -EFAULT and likewise leaves the message queued.
 *  - SELECTIVE RECEIVE: replies are taken by tag, out of arrival order,
 *    leaving the others in the inbox.
 *  - FAIRNESS AND POLICY ACROSS PROCESSES: an allowlist inbox refuses a
 *    stranger, and a process that floods an inbox up to its share cannot
 *    stop a different process from getting a message in.
 *  - LIFETIME: names, inboxes and queues of exited processes are gone, and
 *    sixty short-lived owners in a row (more than the 32-entry tables) all
 *    succeed, so nothing leaks.
 *  - BEYOND A KILOBYTE: a message carries at most 1024 bytes; an 8KB buffer
 *    is handed over as a shared-memory handle (Phase 83) in a message.
 *
 * Every failure prints "[msgtest] FAIL: ..." (the uppercase word also
 * trips test_runner.py's global no-fail assertion). Output for passing
 * checks deliberately avoids the words that assertion greps for.
 */
#include <errno.h>
#include <novamsg.h>
#include <novashm.h>
#include <novasys.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failures;

#define CHECK(cond, ...) do { \
    checks++; \
    if (!(cond)) { \
        failures++; \
        if (failures <= 25) { \
            printf("[msgtest] FAIL: %s (line %d) ", #cond, __LINE__); \
            printf(__VA_ARGS__); \
            printf("\n"); \
        } \
    } \
} while (0)

#define EXPECT(call, want) do { \
    int rc_ = (call); \
    checks++; \
    if (rc_ != (want)) { \
        failures++; \
        if (failures <= 25) \
            printf("[msgtest] FAIL: %s returned %d, expected %d (line %d)\n", \
                   #call, rc_, (int)(want), __LINE__); \
    } \
} while (0)

/* Message types used by the scenarios (type 0 is reserved for "any"). */
enum {
    T_OPEN_FILE = 1, T_OPENED = 2, T_PING = 3, T_PONG = 4, T_UNKNOWN = 5,
    T_SHM_SUM = 7, T_SUM = 8, T_QUIT = 9, T_READY = 10, T_ALLOWED = 11,
    T_FROM_CHILD = 12, T_ALLOW_PID = 20, T_OK = 21, T_FLOOD = 31,
    T_FRIEND = 30, T_CHURN = 40, T_REPORT = 50, T_FROM_ROOT = 60, T_FROM_SAME = 61, T_FROM_OTHER = 62
};

static int my_pid;

/* ---- helpers shared with SHMTEST's approach --------------------------------- */

/* Seconds since midnight, for timeouts that must work even when the
 * messaging clock is what is under test. */
static int now_s(void) {
    nova_rtc_time_t t;
    sys_rtc_read(&t);
    return t.hour * 3600 + t.minute * 60 + t.second;
}

static int wait_ge(volatile unsigned int* v, unsigned int want) {
    int start = now_s();
    for (;;) {
        if (__atomic_load_n(v, __ATOMIC_ACQUIRE) >= want) return 1;
        int e = now_s() - start;
        if (e < 0) e += 86400;
        if (e > 15) return 0;
        sys_yield();
    }
}

static void stage_set(volatile unsigned int* v, unsigned int n) {
    __atomic_store_n(v, n, __ATOMIC_RELEASE);
}

/* A shared-memory mailbox for the few places where processes must be
 * sequenced WITHOUT using the messaging under test. */
typedef struct {
    volatile unsigned int sink_stage;   /* 1: sink may allow the parent; 2: sink may drain */
    volatile unsigned int friend_go;    /* friend may send */
    volatile unsigned int uid_stage;    /* uid sink: 1 = every sender has finished */
} mailbox_t;
static nova_shm_region_t mbr;
static mailbox_t* mb;

static void fill(unsigned char* p, unsigned int n, unsigned int seed) {
    for (unsigned int i = 0; i < n; i++) p[i] = (unsigned char)(seed * 131u + i * 7u + (i >> 3));
}
static int check_fill(const unsigned char* p, unsigned int n, unsigned int seed) {
    for (unsigned int i = 0; i < n; i++)
        if (p[i] != (unsigned char)(seed * 131u + i * 7u + (i >> 3))) return 0;
    return 1;
}

static unsigned int checksum(const unsigned char* p, unsigned int n) {
    unsigned int s = 0;
    for (unsigned int i = 0; i < n; i++) s += (unsigned int)p[i] * (i + 1u);
    return s;
}

/* Sends, retrying while the receiver's inbox is momentarily full. */
static int send_retry(int pid, unsigned int type, unsigned int tag, const void* data, unsigned int len) {
    for (int i = 0; i < 2000; i++) {
        int rc = nova_msg_send(pid, type, tag, data, len);
        if (rc != -EAGAIN) return rc;
        sys_yield();
    }
    return -EAGAIN;
}

/* ---- group 1: one process, its own inbox --------------------------------------- */

static void test_inbox_basics(void) {
    int f0 = failures;
    nova_msg_info_t info;
    static unsigned char buf[1100], big[1100];
    nova_msg_ctl_t st;

    /* without an inbox: nothing works, and says so */
    EXPECT(nova_msg_try_recv(&info, buf, sizeof buf), -EBADF);
    EXPECT(nova_msg_stat(&st), -EBADF);
    EXPECT(nova_msg_service_register("msgtest.self"), -EBADF);
    EXPECT(nova_msg_allow(5), -EBADF);
    EXPECT(nova_msg_close(), -EBADF);

    nova_msg_open_t o;
    o.policy = NOVA_MSG_ACCEPT_ANY; o.flags = 0; o.depth = 0; o.max_payload = 0;
    EXPECT(sys_msg_open(&o), 0);
    CHECK(o.depth == NOVA_MSG_QUEUE_DEPTH && o.max_payload == NOVA_MSG_MAX_PAYLOAD,
          "open reported depth %u / max payload %u", o.depth, o.max_payload);
    EXPECT(nova_msg_open(NOVA_MSG_ACCEPT_ANY), -EEXIST);
    EXPECT(nova_msg_open(3), -EINVAL);
    o.policy = 0; o.flags = 1;
    EXPECT(sys_msg_open(&o), -EINVAL);

    /* learn my own pid: there is no getpid, but the registry knows */
    EXPECT(nova_msg_service_register("msgtest.self"), 0);
    my_pid = nova_msg_service_lookup("msgtest.self");
    CHECK(my_pid > 0, "lookup of my own name gave %d", my_pid);

    /* identity and metadata, checked against the truth */
    unsigned int t0 = nova_msg_ticks();
    EXPECT(nova_msg_send(my_pid, 7, 99, "hello", 5), 0);
    int n = nova_msg_try_recv(&info, buf, sizeof buf);
    CHECK(n == 5 && memcmp(buf, "hello", 5) == 0, "payload: %d", n);
    CHECK(info.sender == my_pid, "sender pid %d is not mine (%d)", info.sender, my_pid);
    CHECK(info.sender_uid == sys_getuid(), "sender uid %u is not mine (%u)", info.sender_uid, sys_getuid());
    CHECK(info.type == 7 && info.tag == 99 && info.len == 5 && info.pending == 0,
          "type %u tag %u len %u pending %u", info.type, info.tag, info.len, info.pending);
    unsigned int t1 = nova_msg_ticks();
    CHECK((int)(info.sent_tick - t0) >= 0 && (int)(t1 - info.sent_tick) >= 0,
          "the kernel's send time %u is outside [%u, %u]", info.sent_tick, t0, t1);

    /* an address by name reaches the same place */
    EXPECT(nova_msg_send_name("msgtest.self", 8, 5, "byname", 6), 0);
    n = nova_msg_try_recv(&info, buf, sizeof buf);
    CHECK(n == 6 && info.type == 8 && info.sender == my_pid, "send by name: %d", n);

    /* framing: one send is exactly one receive, whatever the size */
    static const unsigned int sizes[] = {0, 1, 500, 1024};
    for (unsigned int i = 0; i < 4; i++) {
        fill(big, sizes[i], i + 1);
        EXPECT(nova_msg_send(my_pid, 2, i, big, sizes[i]), 0);
    }
    for (unsigned int i = 0; i < 4; i++) {
        memset(buf, 0, sizeof buf);
        n = nova_msg_try_recv(&info, buf, sizeof buf);
        CHECK(n == (int)sizes[i] && info.len == sizes[i], "frame %u: got %d, expected %u", i, n, sizes[i]);
        CHECK(check_fill(buf, sizes[i], i + 1), "frame %u content", i);
    }

    /* validation */
    EXPECT(nova_msg_send(my_pid, 0, 0, "x", 1), -EINVAL);
    EXPECT(nova_msg_send(my_pid, 1, 0, big, 1025), -E2BIG);
    EXPECT(nova_msg_send(0, 1, 0, "x", 1), -EINVAL);
    EXPECT(nova_msg_send(-7, 1, 0, "x", 1), -EINVAL);
    EXPECT(nova_msg_send(99999, 1, 0, "x", 1), -ESRCH);
    EXPECT(nova_msg_send_name("no.such.service", 1, 0, "x", 1), -ENOENT);
    nova_msg_send_t s;
    memset(&s, 0, sizeof s);
    s.type = 1; s.len = 1; s.data = (unsigned int)(unsigned long)"x";
    memcpy(s.dest_name, "bad name", 8);
    EXPECT(sys_msg_send(&s), -EINVAL);
    memset(s.dest_name, 'a', 32); /* no terminator: malformed, not truncated */
    EXPECT(sys_msg_send(&s), -EINVAL);
    memset(s.dest_name, 0, 32);
    s.dest_pid = my_pid; s.flags = 1;
    EXPECT(sys_msg_send(&s), -EINVAL);
    EXPECT(nova_msg_try_recv(&info, buf, sizeof buf), -EAGAIN); /* nothing above was queued */

    /* FIFO; and one sender may hold only its share of an inbox (8 of 16) */
    for (unsigned int i = 0; i < NOVA_MSG_MAX_PER_SENDER; i++) EXPECT(nova_msg_send(my_pid, 1, i, &i, 4), 0);
    EXPECT(nova_msg_send(my_pid, 1, 99, "x", 1), -EAGAIN);
    EXPECT(nova_msg_stat(&st), 0);
    CHECK(st.queued == NOVA_MSG_MAX_PER_SENDER && st.refused >= 1 && st.capacity == 16 && st.tick_hz == 100,
          "stat: queued %u refused %u capacity %u hz %u", st.queued, st.refused, st.capacity, st.tick_hz);
    for (unsigned int i = 0; i < NOVA_MSG_MAX_PER_SENDER; i++) {
        n = nova_msg_try_recv(&info, buf, sizeof buf);
        CHECK(n == 4 && info.tag == i, "FIFO position %u delivered tag %u", i, info.tag);
    }

    /* never truncated, never consumed by a failed receive, peek leaves it */
    EXPECT(nova_msg_send(my_pid, 5, 77, "0123456789", 10), 0);
    EXPECT(nova_msg_send(my_pid, 5, 78, "second", 6), 0);
    EXPECT(nova_msg_try_recv(&info, buf, 4), -E2BIG);
    CHECK(info.len == 10 && info.tag == 77, "E2BIG must report the size needed (len %u)", info.len);
    EXPECT(nova_msg_try_recv_ex(&info, buf, 0, NOVA_MSG_RECV_PEEK, 0, 0, 0), -E2BIG);
    CHECK(info.len == 10, "a zero-size peek asks how big the next message is");
    EXPECT(nova_msg_stat(&st), 0);
    CHECK(st.queued == 2 && st.next_len == 10 && st.next_type == 5, "stat after failures: queued %u next %u", st.queued, st.next_len);
    n = nova_msg_try_recv_ex(&info, buf, sizeof buf, NOVA_MSG_RECV_PEEK, 0, 0, 0);
    CHECK(n == 10 && memcmp(buf, "0123456789", 10) == 0 && info.pending == 2, "peek: %d pending %u", n, info.pending);
    n = nova_msg_try_recv(&info, buf, sizeof buf);
    CHECK(n == 10 && info.pending == 1, "consume: %d pending %u", n, info.pending);
    n = nova_msg_try_recv(&info, buf, sizeof buf);
    CHECK(n == 6 && memcmp(buf, "second", 6) == 0 && info.pending == 0, "second: %d", n);

    /* selective receive: by type, by sender, by tag, oldest match first */
    EXPECT(nova_msg_send(my_pid, 3, 1, "a", 1), 0);
    EXPECT(nova_msg_send(my_pid, 4, 2, "b", 1), 0);
    EXPECT(nova_msg_send(my_pid, 3, 2, "c", 1), 0);
    n = nova_msg_try_recv_ex(&info, buf, sizeof buf, 0, 0, 4, 0);
    CHECK(n == 1 && buf[0] == 'b', "by type");
    n = nova_msg_try_recv_ex(&info, buf, sizeof buf, NOVA_MSG_RECV_MATCH_TAG, 0, 3, 2);
    CHECK(n == 1 && buf[0] == 'c', "by type and tag");
    EXPECT(nova_msg_try_recv_ex(&info, buf, sizeof buf, 0, my_pid + 1000, 0, 0), -EAGAIN);
    n = nova_msg_try_recv_ex(&info, buf, sizeof buf, 0, my_pid, 0, 0);
    CHECK(n == 1 && buf[0] == 'a', "by sender: what is left is still there");
    EXPECT(nova_msg_try_recv(&info, buf, sizeof buf), -EAGAIN);

    /* real timeouts on the kernel clock (10ms ticks); and 0 means one attempt */
    unsigned int a = nova_msg_ticks();
    EXPECT(nova_msg_recv_wait(&info, buf, sizeof buf, 0, 0, 50), -EAGAIN);
    unsigned int b = nova_msg_ticks();
    CHECK(b - a >= 4 && b - a <= 80, "a 50ms wait took %u ticks", b - a);
    a = nova_msg_ticks();
    EXPECT(nova_msg_recv_wait(&info, buf, sizeof buf, 0, 0, 0), -EAGAIN);
    CHECK(nova_msg_ticks() - a <= 2, "a zero-timeout wait must not wait");

    /* control validation */
    EXPECT(nova_msg_set_policy(9), -EINVAL);
    EXPECT(nova_msg_ctl_op(99, 0), -EINVAL);
    EXPECT(nova_msg_allow(0), -EINVAL);

    if (failures == f0) printf("[msgtest] ok: inbox basics - framing, kernel-stamped identity, validation, FIFO, per-sender cap, E2BIG and peek, selective receive, real timeouts\n");
}

/* ---- group 2: hostile pointers; a bad buffer must never eat a message ----------------- */

static void test_hostile_pointers(void) {
    int f0 = failures;
    nova_msg_info_t info;
    static unsigned char buf[64];
    void* nowhere[] = {
        (void*)0,           /* NULL */
        (void*)0x00100000,  /* the kernel image */
        (void*)0x50000000,  /* user range, never mapped */
        (void*)0xFFFFFFF8,  /* a read here wraps past 4GB */
    };
    for (unsigned i = 0; i < sizeof nowhere / sizeof nowhere[0]; i++) {
        void* q = nowhere[i];
        EXPECT(sys_msg_open((nova_msg_open_t*)q), -EFAULT);
        EXPECT(sys_msg_send((const nova_msg_send_t*)q), -EFAULT);
        EXPECT(sys_msg_recv((nova_msg_recv_t*)q), -EFAULT);
        EXPECT(sys_msg_service((nova_msg_service_t*)q), -EFAULT);
        EXPECT(sys_msg_ctl((nova_msg_ctl_t*)q), -EFAULT);
        /* a good request struct whose DATA pointer is bad */
        nova_msg_send_t s;
        memset(&s, 0, sizeof s);
        s.dest_pid = my_pid; s.type = 1; s.len = 10; s.data = (unsigned int)(unsigned long)q;
        EXPECT(sys_msg_send(&s), -EFAULT);
    }
    CHECK(nova_msg_try_recv(&info, buf, sizeof buf) == -EAGAIN, "a failed send queued something");

    /* a bad DESTINATION buffer must fail the receive WITHOUT consuming the message */
    EXPECT(nova_msg_send(my_pid, 6, 5, "keepme", 6), 0);
    for (unsigned i = 0; i < sizeof nowhere / sizeof nowhere[0]; i++) {
        EXPECT(nova_msg_try_recv(&info, nowhere[i], 16), -EFAULT);
    }
    int n = nova_msg_try_recv(&info, buf, sizeof buf);
    CHECK(n == 6 && memcmp(buf, "keepme", 6) == 0, "the message was consumed by a receive that failed (%d)", n);

    /* ...including a destination in a READ-ONLY shared-memory mapping (Phase 83):
     * the kernel runs with CR0.WP clear and would otherwise write straight through */
    nova_shm_region_t rw, ro;
    EXPECT(nova_shm_create(&rw, 4096), 0);
    unsigned int handle = rw.handle;
    EXPECT(nova_shm_detach(&rw), 0);
    EXPECT(nova_shm_attach(&ro, handle, NOVA_SHM_RIGHT_READ), 0);
    EXPECT(nova_msg_send(my_pid, 6, 6, "intact", 6), 0);
    EXPECT(nova_msg_try_recv(&info, ro.addr, 16), -EFAULT);
    n = nova_msg_try_recv(&info, buf, sizeof buf);
    CHECK(n == 6 && memcmp(buf, "intact", 6) == 0, "a read-only mapping was written to, or the message lost (%d)", n);
    nova_shm_detach(&ro);
    sys_shm_destroy(handle);

    if (failures == f0) printf("[msgtest] ok: hostile pointers and unwritable buffers fail with a bad-address error and never consume a message (including a read-only shared-memory mapping)\n");
}

/* ---- group 3: the service registry --------------------------------------------------------- */

static void test_services(void) {
    int f0 = failures;
    nova_msg_info_t info;
    static unsigned char buf[64];

    EXPECT(nova_msg_service_register("msgtest.alpha"), 0);
    EXPECT(nova_msg_service_register("msgtest.alpha"), 0);   /* re-registering your own name is a no-op */
    CHECK(nova_msg_service_lookup("msgtest.alpha") == my_pid, "lookup");
    EXPECT(nova_msg_service_lookup("msgtest.nobody"), -ENOENT);
    EXPECT(nova_msg_service_unregister("msgtest.nobody"), -ENOENT);
    static const char* const bad[] = {"", "bad name", "a/b", "tab\there", "0123456789012345678901234567890123"};
    for (unsigned i = 0; i < sizeof bad / sizeof bad[0]; i++) {
        EXPECT(nova_msg_service_register(bad[i]), -EINVAL);
        EXPECT(nova_msg_service_lookup(bad[i]), -EINVAL);
    }
    /* a process holds at most 4 names: "msgtest.self" and "msgtest.alpha" plus two more */
    EXPECT(nova_msg_service_register("msgtest.b3"), 0);
    EXPECT(nova_msg_service_register("msgtest.b4"), 0);
    EXPECT(nova_msg_service_register("msgtest.b5"), -ENOSPC);
    EXPECT(nova_msg_service_unregister("msgtest.b3"), 0);
    EXPECT(nova_msg_service_unregister("msgtest.b4"), 0);

    int pid = sys_fork();
    if (pid == 0) {
        failures = 0;
        /* fork gives the child NOTHING: not the parent's inbox, queue or names */
        CHECK(nova_msg_try_recv(&info, buf, sizeof buf) == -EBADF, "the child can see its parent's inbox");
        CHECK(nova_msg_service_lookup("msgtest.alpha") == my_pid, "a child can look up the parent's name");
        CHECK(nova_msg_service_register("msgtest.zzz") == -EBADF, "registering needs an inbox");
        CHECK(nova_msg_open(NOVA_MSG_ACCEPT_ANY) == 0, "child open");
        CHECK(nova_msg_service_register("msgtest.alpha") == -EEXIST, "a live process's name was stolen");
        CHECK(nova_msg_service_unregister("msgtest.alpha") == -EPERM, "unregistered someone else's name");
        CHECK(nova_msg_service_unregister("msgtest.self") == -EPERM, "unregistered someone else's name (2)");
        CHECK(nova_msg_service_register("msgtest.child") == 0, "child register");
        CHECK(nova_msg_send_name("msgtest.alpha", T_FROM_CHILD, 3, "from child", 10) == 0, "send by name to the parent");
        sys_exit(failures);
    }
    CHECK(pid > 0, "fork failed: %d", pid);
    int n = nova_msg_recv_wait(&info, buf, sizeof buf, pid, T_FROM_CHILD, 3000);
    CHECK(n == 10 && memcmp(buf, "from child", 10) == 0, "the child's message: %d", n);
    CHECK(info.sender == pid, "the message says it came from %d, not the child (%d)", info.sender, pid);
    int code = sys_wait(pid);
    CHECK(code == 0, "the child reported %d failed checks", code);

    /* the child is gone: its name, inbox and place in the world went with it */
    EXPECT(nova_msg_service_lookup("msgtest.child"), -ENOENT);
    EXPECT(nova_msg_send(pid, 1, 0, "x", 1), -ESRCH);
    EXPECT(nova_msg_send_name("msgtest.child", 1, 0, "x", 1), -ENOENT);
    EXPECT(nova_msg_service_register("msgtest.child"), 0);          /* the name is free again */
    EXPECT(nova_msg_service_unregister("msgtest.child"), 0);
    EXPECT(nova_msg_service_unregister("msgtest.alpha"), 0);

    if (failures == f0) printf("[msgtest] ok: service registry - register, lookup, unregister, ownership, the per-process limit, and names released when the owner exits\n");
}

/* ---- group 4: request / response - the file manager asks the editor ------------------------- */

/* The editor process. */
static void editor_main(void) {
    failures = 0;
    static unsigned char in[1100], out[1100];
    nova_msg_info_t info;
    CHECK(nova_msg_open(NOVA_MSG_ACCEPT_ANY) == 0, "editor: open");
    CHECK(nova_msg_service_register("msgtest.editor") == 0, "editor: register");
    for (;;) {
        int n = nova_msg_recv_wait(&info, in, sizeof in, 0, 0, 8000);
        if (n < 0) {
            CHECK(0, "editor: timed out waiting for a request (%d)", n);
            break;
        }
        if (info.type == T_QUIT) {
            break;
        } else if (info.type == T_OPEN_FILE) {
            memcpy(out, "opened:", 7);
            memcpy(out + 7, in, (unsigned)n);
            CHECK(send_retry(info.sender, T_OPENED, info.tag, out, 7u + (unsigned)n) == 0, "editor: reply");
        } else if (info.type == T_PING) {
            CHECK(send_retry(info.sender, T_PONG, info.tag, in, (unsigned)n) == 0, "editor: pong");
        } else if (info.type == T_SHM_SUM && n == 4) {
            /* a message is at most 1KB; bigger data arrives as a shared-memory handle */
            unsigned int handle;
            memcpy(&handle, in, 4);
            nova_shm_region_t r;
            unsigned int sum = 0xFFFFFFFFu;
            if (nova_shm_attach(&r, handle, NOVA_SHM_RIGHT_READ) == 0) {
                sum = checksum((const unsigned char*)r.addr, r.size);
                nova_shm_detach(&r);
            } else {
                CHECK(0, "editor: could not map the shared buffer it was told about");
            }
            CHECK(send_retry(info.sender, T_SUM, info.tag, &sum, 4) == 0, "editor: sum reply");
        } else {
            CHECK(send_retry(info.sender, T_UNKNOWN, info.tag, "?", 1) == 0, "editor: unknown reply");
        }
    }
    sys_exit(failures);
}

static void test_request_response(void) {
    int f0 = failures;
    static unsigned char buf[1100], req[1100];
    nova_msg_info_t info;

    int pid = sys_fork();
    if (pid == 0) {
        editor_main();
    }
    CHECK(pid > 0, "fork failed: %d", pid);

    /* the file manager finds the editor by NAME - no pid, no inherited handle */
    int ep = -1;
    int start = now_s();
    while ((ep = nova_msg_service_lookup("msgtest.editor")) < 0) {
        int e = now_s() - start;
        if (e < 0) e += 86400;
        if (e > 10) break;
        sys_yield();
    }
    CHECK(ep == pid, "the registered editor is %d, expected the child %d", ep, pid);

    /* 1. "open this file" */
    const char* path = "/home/user/notes.txt";
    unsigned int plen = (unsigned int)strlen(path);
    int n = nova_msg_call_name("msgtest.editor", T_OPEN_FILE, path, plen, &info, buf, sizeof buf, 3000);
    CHECK(n == (int)(7 + plen) && memcmp(buf, "opened:", 7) == 0 && memcmp(buf + 7, path, plen) == 0, "reply: %d", n);
    CHECK(info.sender == pid && info.type == T_OPENED, "reply came from %d type %u", info.sender, info.type);

    /* 2. a request the editor does not understand gets an answer, not silence */
    n = nova_msg_call(pid, 77, "x", 1, &info, buf, sizeof buf, 3000);
    CHECK(n == 1 && info.type == T_UNKNOWN, "unknown request: %d type %u", n, info.type);

    /* 3. several requests in flight; take the replies OUT OF ORDER by tag */
    static const char* const files[] = {"/a", "/b", "/c"};
    for (unsigned i = 0; i < 3; i++) EXPECT(nova_msg_send(pid, T_OPEN_FILE, 101 + i, files[i], 2), 0);
    static const unsigned order[] = {2, 0, 1}; /* the third reply first */
    for (unsigned k = 0; k < 3; k++) {
        unsigned i = order[k];
        n = nova_msg_recv_wait_ex(&info, buf, sizeof buf, NOVA_MSG_RECV_MATCH_TAG, pid, 0, 101 + i, 3000);
        CHECK(n == 9 && memcmp(buf, "opened:", 7) == 0 && memcmp(buf + 7, files[i], 2) == 0, "reply for tag %u: %d", 101 + i, n);
        if (k == 0) {
            nova_msg_ctl_t st;
            EXPECT(nova_msg_stat(&st), 0);
            CHECK(st.queued == 2, "after taking one reply by tag the other two must still be queued (%u)", st.queued);
        }
    }

    /* 4. integrity over 300 round trips of every size class */
    int bad_trips = 0;
    unsigned int last_tag = 0;
    for (unsigned i = 0; i < 300; i++) {
        unsigned int len = (i * 37u) % 1025u;
        fill(req, len, i);
        n = nova_msg_call(pid, T_PING, req, len, &info, buf, sizeof buf, 3000);
        if (n != (int)len || !check_fill(buf, len, i) || info.type != T_PONG || info.sender != pid || info.tag <= last_tag) bad_trips++;
        last_tag = info.tag;
    }
    CHECK(bad_trips == 0, "%d of 300 round trips came back wrong, short, out of order or from the wrong sender", bad_trips);

    /* 5. more than a kilobyte: an 8KB buffer travels as a shared-memory handle */
    nova_shm_region_t r;
    EXPECT(nova_shm_create(&r, 8192), 0);
    fill((unsigned char*)r.addr, r.size, 4242);
    unsigned int want = checksum((const unsigned char*)r.addr, r.size);
    EXPECT(nova_shm_grant_pid(r.handle, pid, NOVA_SHM_RIGHT_READ), 0);
    unsigned int handle = r.handle;
    n = nova_msg_call(pid, T_SHM_SUM, &handle, 4, &info, buf, sizeof buf, 3000);
    unsigned int got = 0;
    if (n == 4) memcpy(&got, buf, 4);
    CHECK(n == 4 && got == want, "the editor's checksum of the shared 8KB (%u) differs from mine (%u), n=%d", got, want, n);
    nova_shm_detach(&r);
    sys_shm_destroy(r.handle);

    /* 6. shut it down; no replies may be left behind */
    EXPECT(nova_msg_send(pid, T_QUIT, 0, "", 0), 0);
    int code = sys_wait(pid);
    CHECK(code == 0, "the editor reported %d failed checks", code);
    nova_msg_ctl_t st;
    EXPECT(nova_msg_stat(&st), 0);
    CHECK(st.queued == 0, "%u stray messages left in the inbox", st.queued);
    EXPECT(nova_msg_service_lookup("msgtest.editor"), -ENOENT);

    if (failures == f0) printf("[msgtest] ok: request/response between processes - the file manager asks the editor (found by name) to open a file, replies matched by tag out of order, 300 round trips with every payload size intact, and an 8KB payload handed over as a shared-memory handle\n");
}

/* ---- group 5: receiver policy, and one sender cannot lock out another ----------------------- */

static int sink_pid_g;

static void sink_main(void) {
    failures = 0;
    static unsigned char in[1100];
    nova_msg_info_t info;
    CHECK(nova_msg_open(NOVA_MSG_ACCEPT_ALLOWLIST) == 0, "sink: open");
    CHECK(nova_msg_send(my_pid, T_READY, 0, "", 0) == 0, "sink: ready");
    CHECK(wait_ge(&mb->sink_stage, 1), "sink: timed out (1)");
    CHECK(nova_msg_allow(my_pid) == 0, "sink: allow the parent");
    CHECK(nova_msg_send(my_pid, T_ALLOWED, 0, "", 0) == 0, "sink: allowed");
    /* the parent tells us about a second process to admit */
    int n = nova_msg_recv_wait(&info, in, sizeof in, my_pid, T_ALLOW_PID, 5000);
    CHECK(n == 4, "sink: the pid message: %d", n);
    int friend_pid = 0;
    if (n == 4) memcpy(&friend_pid, in, 4);
    CHECK(nova_msg_allow(friend_pid) == 0, "sink: allow the friend");
    CHECK(nova_msg_send(my_pid, T_OK, 0, "", 0) == 0, "sink: ok");
    CHECK(wait_ge(&mb->sink_stage, 2), "sink: timed out (2)");

    /* drain: 8 from the parent in order, 1 from the friend */
    int from_parent = 0, from_friend = 0, in_order = 1;
    for (;;) {
        n = nova_msg_try_recv(&info, in, sizeof in);
        if (n < 0) break;
        if (info.sender == my_pid) {
            unsigned int v = 0;
            if (n == 4) memcpy(&v, in, 4);
            if (info.type != T_FLOOD || v != (unsigned)from_parent) in_order = 0;
            from_parent++;
        } else if (info.sender == friend_pid) {
            from_friend++;
        }
    }
    nova_msg_ctl_t st;
    CHECK(nova_msg_stat(&st) == 0, "sink: stat");
    int report[4] = {from_parent, from_friend, in_order, (int)st.refused};
    CHECK(send_retry(my_pid, T_REPORT, 0, report, sizeof report) == 0, "sink: report");
    sys_exit(failures);
}

static void friend_main(void) {
    failures = 0;
    CHECK(wait_ge(&mb->friend_go, 1), "friend: timed out");
    /* the parent has already filled ITS share of the sink's inbox */
    CHECK(nova_msg_send(sink_pid_g, T_FRIEND, 5, "friend", 6) == 0, "a different sender was locked out by a flood");
    sys_exit(failures);
}

static void test_policy_and_fairness(void) {
    int f0 = failures;
    static unsigned char buf[64];
    nova_msg_info_t info;
    stage_set(&mb->sink_stage, 0);
    stage_set(&mb->friend_go, 0);

    int sink = sys_fork();
    if (sink == 0) {
        sink_main();
    }
    CHECK(sink > 0, "fork failed: %d", sink);
    sink_pid_g = sink;

    int n = nova_msg_recv_wait(&info, buf, sizeof buf, sink, T_READY, 5000);
    CHECK(n == 0, "the sink never became ready: %d", n);
    /* the sink listens, but only to an explicit list that does not include us */
    EXPECT(nova_msg_send(sink, 1, 0, "x", 1), -EACCES);
    stage_set(&mb->sink_stage, 1);
    n = nova_msg_recv_wait(&info, buf, sizeof buf, sink, T_ALLOWED, 5000);
    CHECK(n == 0, "the sink never allowed us: %d", n);

    int friend_pid = sys_fork();
    if (friend_pid == 0) {
        friend_main();
    }
    CHECK(friend_pid > 0, "fork failed: %d", friend_pid);
    EXPECT(nova_msg_send(sink, T_ALLOW_PID, 0, &friend_pid, 4), 0);
    n = nova_msg_recv_wait(&info, buf, sizeof buf, sink, T_OK, 5000);
    CHECK(n == 0, "the sink never confirmed: %d", n);

    /* flood the sink up to our share (8 of its 16 slots): the 9th is refused */
    for (unsigned int i = 0; i < NOVA_MSG_MAX_PER_SENDER; i++) EXPECT(nova_msg_send(sink, T_FLOOD, 0, &i, 4), 0);
    EXPECT(nova_msg_send(sink, T_FLOOD, 0, "x", 1), -EAGAIN);
    /* ...and now a DIFFERENT process must still be able to get through */
    stage_set(&mb->friend_go, 1);
    int fcode = sys_wait(friend_pid);
    CHECK(fcode == 0, "the friend reported %d failed checks", fcode);
    stage_set(&mb->sink_stage, 2);

    n = nova_msg_recv_wait(&info, buf, sizeof buf, sink, T_REPORT, 5000);
    CHECK(n == 16, "the sink's report: %d", n);
    int report[4] = {0, 0, 0, 0};
    if (n == 16) memcpy(report, buf, 16);
    CHECK(report[0] == 8 && report[1] == 1, "the sink received %d from the flooder and %d from the friend (expected 8 and 1)", report[0], report[1]);
    CHECK(report[2] == 1, "the flooder's messages arrived out of order");
    CHECK(report[3] >= 2, "the sink counted only %d refusals (the stranger and the 9th message make 2)", report[3]);
    int scode = sys_wait(sink);
    CHECK(scode == 0, "the sink reported %d failed checks", scode);

    if (failures == f0) printf("[msgtest] ok: receiver policy and fairness - an allowlist refuses a stranger, and a sender that fills its share of an inbox cannot lock another sender out\n");
}

/* ---- group 6: kernel-stamped uids and the same-user policy ---------------------------------- */

/* This program runs as root, so checking a message's uid against sys_getuid()
 * (as group 1 does) could not tell a real stamp from a constant 0. Here the
 * senders hold REAL, DISTINCT, non-zero identities: sys_login() makes a
 * process uid 700 ("persisted") or uid 800 ("admin"). */
static void uid_sink_main(void) {
    failures = 0;
    static unsigned char in[64];
    nova_msg_info_t info;
    CHECK(sys_login("persisted", "persisted-pw") == 0, "uid sink: login");
    CHECK(sys_getuid() == 700, "uid sink: uid is %u", sys_getuid());
    CHECK(nova_msg_open(NOVA_MSG_ACCEPT_SAME_UID) == 0, "uid sink: open");
    CHECK(nova_msg_service_register("msgtest.sink700") == 0, "uid sink: register");
    CHECK(wait_ge(&mb->uid_stage, 1), "uid sink: timed out");

    /* exactly two messages may be here: root's (root is exempt) and the uid-700
     * child's. The uid-800 child's must have been refused at send time. */
    unsigned int count = 0, root_uid = 12345, same_uid = 12345, bad = 0;
    int n;
    while ((n = nova_msg_try_recv(&info, in, sizeof in)) >= 0) {
        count++;
        if (info.type == T_FROM_ROOT) root_uid = info.sender_uid;
        else if (info.type == T_FROM_SAME) same_uid = info.sender_uid;
        else bad++;
    }
    nova_msg_ctl_t st;
    CHECK(nova_msg_stat(&st) == 0, "uid sink: stat");
    unsigned int report[5] = {count, root_uid, same_uid, bad, st.refused};
    CHECK(send_retry(my_pid, T_REPORT, 0, report, sizeof report) == 0, "uid sink: report");
    sys_exit(failures);
}

static void uid_sender_main(const char* user, const char* password, unsigned int want_uid, unsigned int type, int expect_rc) {
    failures = 0;
    CHECK(sys_login(user, password) == 0, "uid sender: login as %s", user);
    CHECK(sys_getuid() == want_uid, "uid sender: uid is %u, expected %u", sys_getuid(), want_uid);
    int rc = nova_msg_send_name("msgtest.sink700", type, 1, "hi", 2);
    CHECK(rc == expect_rc, "uid %u sending to the same-user sink got %d, expected %d", want_uid, rc, expect_rc);
    sys_exit(failures);
}

static void test_uid_stamping_and_same_user_policy(void) {
    int f0 = failures;
    static unsigned char buf[64];
    nova_msg_info_t info;
    stage_set(&mb->uid_stage, 0);

    int sink = sys_fork();
    if (sink == 0) {
        uid_sink_main();
    }
    CHECK(sink > 0, "fork failed: %d", sink);
    int start = now_s();
    while (nova_msg_service_lookup("msgtest.sink700") < 0) {
        int e = now_s() - start;
        if (e < 0) e += 86400;
        if (e > 10) break;
        sys_yield();
    }

    /* root is exempt from the same-user policy */
    CHECK(sys_getuid() == 0, "this test expects to start as root (uid %u)", sys_getuid());
    EXPECT(nova_msg_send_name("msgtest.sink700", T_FROM_ROOT, 1, "hi", 2), 0);
    int same = sys_fork();
    if (same == 0) {
        uid_sender_main("persisted", "persisted-pw", 700, T_FROM_SAME, 0);
    }
    int other = sys_fork();
    if (other == 0) {
        uid_sender_main("admin", "admin-correct-password", 800, T_FROM_OTHER, -EACCES);
    }
    CHECK(same > 0 && other > 0, "fork failed");
    int c1 = sys_wait(same), c2 = sys_wait(other);
    CHECK(c1 == 0, "the same-user sender reported %d failed checks", c1);
    CHECK(c2 == 0, "the other-user sender reported %d failed checks", c2);
    stage_set(&mb->uid_stage, 1);

    int n = nova_msg_recv_wait(&info, buf, sizeof buf, sink, T_REPORT, 5000);
    CHECK(n == 20, "the uid sink's report: %d", n);
    unsigned int r[5] = {0, 0, 0, 0, 0};
    if (n == 20) memcpy(r, buf, 20);
    CHECK(r[0] == 2, "the sink holds %u messages, expected 2 (root's and the same user's)", r[0]);
    CHECK(r[1] == 0, "root's message was stamped with uid %u, expected 0", r[1]);
    CHECK(r[2] == 700, "the same-user child's message was stamped with uid %u, expected 700", r[2]);
    CHECK(r[3] == 0, "%u messages of an unexpected type reached the sink (the other user got through?)", r[3]);
    CHECK(r[4] >= 1, "the sink counted %u refusals, expected at least the other user's", r[4]);
    int code = sys_wait(sink);
    CHECK(code == 0, "the uid sink reported %d failed checks", code);

    if (failures == f0) printf("[msgtest] ok: kernel-stamped uids (0 and 700, from real logins) and the same-user policy - root and the same user get in, a different user is refused\n");
}

/* ---- group 7: tables recycle ----------------------------------------------------------------- */

static void test_churn(void) {
    int f0 = failures;
    static unsigned char buf[16];
    nova_msg_info_t info;
    int good = 0;
    /* more short-lived owners than there are inbox or name slots (32 each) */
    for (unsigned int i = 0; i < 60; i++) {
        int pid = sys_fork();
        if (pid == 0) {
            failures = 0;
            CHECK(nova_msg_open(NOVA_MSG_ACCEPT_ANY) == 0, "churn child: open");
            CHECK(nova_msg_service_register("msgtest.churn") == 0, "churn child: register (the previous owner's name must be free)");
            CHECK(nova_msg_send(my_pid, T_CHURN, i, &i, 4) == 0, "churn child: send");
            sys_exit(failures); /* no close, no unregister */
        }
        if (pid < 0) break;
        int n = nova_msg_recv_wait(&info, buf, sizeof buf, pid, T_CHURN, 3000);
        int code = sys_wait(pid);
        if (n == 4 && info.tag == i && code == 0 && nova_msg_service_lookup("msgtest.churn") == -ENOENT) good++;
    }
    CHECK(good == 60, "only %d of 60 short-lived inbox/name owners worked - a leak in the inbox or name table?", good);
    nova_msg_ctl_t st;
    EXPECT(nova_msg_stat(&st), 0);
    CHECK(st.queued == 0, "%u stray messages", st.queued);
    if (failures == f0) printf("[msgtest] ok: table churn - 60 short-lived owners of an inbox and a name (more than the 32-entry tables hold) leave nothing behind\n");
}

int main(void) {
    EXPECT(nova_shm_create(&mbr, 4096), 0);
    mb = (mailbox_t*)mbr.addr;

    test_inbox_basics();
    if (my_pid > 0) {
        test_hostile_pointers();
        test_services();
        test_request_response();
        test_policy_and_fairness();
        test_uid_stamping_and_same_user_policy();
        test_churn();
    }

    nova_shm_detach(&mbr);
    sys_shm_destroy(mbr.handle);

    if (failures == 0) {
        printf("[msgtest] PASS: the full SYS_MSG_* contract holds (%d checks)\n", checks);
        return 0;
    }
    printf("[msgtest] FAIL: %d of %d checks failed\n", failures, checks);
    return 1;
}
