/*
 * exec_trust_demo.c - proves Phase 59's SYS_EXEC_TRUSTED capability
 * delegation is real
 *
 * See exec_trust_demo.h for the full reasoning. This file's own
 * syscall wrappers are deliberately duplicated raw `int 0x80` inline
 * asm, not shared with userland/libc/syscall.c - the same "no shared
 * bridge header" FFI convention kernel/task/sandbox_demo.c already
 * established for exactly this situation (a kernel-compiled ring-3
 * task, not a separately-built userland program).
 */
#include "exec_trust_demo.h"
#include "../arch/x86/cpu/syscall.h"

static inline void sys_write(const char* str) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_WRITE), "b"(str) : "memory", "cc");
}

static inline int sys_exec(const char* path, const char** argv, int argc) {
    int result = SYS_EXEC;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc)
                       : "memory", "cc");
    return result;
}

/* Phase 59: the one new syscall this file exists to exercise - see
 * kernel/arch/x86/cpu/syscall.h's own comment on SYS_EXEC_TRUSTED for
 * the full contract. Same EBX/ECX/EDX convention as sys_exec() above,
 * different syscall number. */
static inline int sys_exec_trusted(const char* path, const char** argv,
                                    int argc) {
    int result = SYS_EXEC_TRUSTED;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc)
                       : "memory", "cc");
    return result;
}

static inline int sys_wait(int pid) {
    int result = SYS_WAIT;
    __asm__ volatile ("int $0x80" : "+a"(result) : "b"(pid) : "memory", "cc");
    return result;
}

static inline void sys_exit(int exit_code) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT), "b"(exit_code));
}

void exec_trust_demo_task(void) {
    sys_write("\n[sandbox] Starting SYS_EXEC_TRUSTED self-test; this task "
              "was created with can_spawn and can_open_any_file both "
              "granted directly (process_create_sandboxed_task_trusted()) "
              "so it can exec TPROBE.ELF two different ways.\n");

    const char* argv0[] = {"TPROBE.ELF"};

    /* First: plain SYS_EXEC. Even though *this* task has can_open_any_
     * file, plain SYS_EXEC never delegates it - the new process should
     * get nothing, exactly like any other ordinary program, and
     * TPROBE.ELF's own first SYS_WRITE_FILE call should fail
     * immediately, making it exit 1. */
    int pid_plain = sys_exec("TPROBE.ELF", argv0, 1);
    int code_plain = (pid_plain >= 0) ? sys_wait(pid_plain) : -1;

    /* Second: SYS_EXEC_TRUSTED. This task's own can_open_any_file
     * (true) should be delegated to the new process, so TPROBE.ELF's
     * write/read-back/delete/confirm-deleted sequence should all
     * succeed this time, exiting 0. */
    int pid_trusted = sys_exec_trusted("TPROBE.ELF", argv0, 1);
    int code_trusted = (pid_trusted >= 0) ? sys_wait(pid_trusted) : -1;

    if (code_plain != 0 && code_trusted == 0) {
        sys_write("[sandbox] PASS: SYS_EXEC_TRUSTED delegates can_open_any_"
                  "file correctly (plain SYS_EXEC to TPROBE.ELF got "
                  "nothing and it failed; SYS_EXEC_TRUSTED delegated this "
                  "task's own capability and it succeeded).\n");
    } else {
        sys_write("[sandbox] FAIL: SYS_EXEC_TRUSTED capability delegation "
                  "did not behave as expected.\n");
    }

    /* Phase 73: the same trusted-exec vehicle, reused to run the in-OS
     * half of the libc test (userland/libctest/libctest.c). It needs
     * exactly the two capabilities this task already holds and
     * SYS_EXEC_TRUSTED delegates - can_open_any_file (it creates, reads,
     * and deletes real files on FAT32 and reads real files on ext2) and
     * can_spawn (it execs itself as a child to test environment
     * inheritance). LIBCTEST.ELF prints its own per-area
     * "[libctest] PASS/FAIL: ..." lines, which tools/python/
     * test_runner.py checks individually; this only reports whether the
     * whole run exited 0. */
    const char* argv_lc[] = {"LIBCTEST.ELF"};
    int pid_lc = sys_exec_trusted("LIBCTEST.ELF", argv_lc, 1);
    int code_lc = (pid_lc >= 0) ? sys_wait(pid_lc) : -1;
    sys_write(pid_lc >= 0 && code_lc == 0
                  ? "[sandbox] PASS: LIBCTEST.ELF (the libc test) ran to "
                    "completion with every group passing.\n"
                  : "[sandbox] FAIL: LIBCTEST.ELF (the libc test) did not "
                    "exit cleanly.\n");

    /* Phase 74: real dynamic linking. DYNTEST.ELF and DYNTEST2.ELF are
     * two SEPARATE, independently built executables that both hold a
     * DT_NEEDED reference to the one DYNLIB.SO file on disk - see
     * kernel/task/process.c's load_and_link_shared_libraries() and
     * kernel/rust/dynlink.rs for what actually resolves that at exec
     * time. Plain sys_exec() would work just as well as the trusted
     * form here - loading a shared library needs no special capability
     * (see load_and_link_shared_libraries()'s own comment: its
     * vfs_read_file() calls are unconditional kernel-mode reads,
     * exactly like the main executable's own load) - sys_exec_trusted()
     * is used only for consistency with this function's other calls. */
    const char* argv_dt[] = {"DYNTEST.ELF"};
    int pid_dt = sys_exec_trusted("DYNTEST.ELF", argv_dt, 1);
    int code_dt = (pid_dt >= 0) ? sys_wait(pid_dt) : -1;
    sys_write(pid_dt >= 0 && code_dt == 0
                  ? "[sandbox] PASS: DYNTEST.ELF (dynamic linking against "
                    "DYNLIB.SO) ran to completion with every group "
                    "passing.\n"
                  : "[sandbox] FAIL: DYNTEST.ELF (dynamic linking) did not "
                    "exit cleanly.\n");

    const char* argv_dt2[] = {"DYNTEST2.ELF"};
    int pid_dt2 = sys_exec_trusted("DYNTEST2.ELF", argv_dt2, 1);
    int code_dt2 = (pid_dt2 >= 0) ? sys_wait(pid_dt2) : -1;
    sys_write(pid_dt2 >= 0 && code_dt2 == 0
                  ? "[sandbox] PASS: DYNTEST2.ELF (a second, independent "
                    "consumer of the same DYNLIB.SO) ran to completion "
                    "with every group passing.\n"
                  : "[sandbox] FAIL: DYNTEST2.ELF did not exit cleanly.\n");

    /* Phase 75: the actual point of the process table becoming
     * growable, proven directly rather than inferred. PROCESS_TABLE_
     * CHUNK_SIZE (process.h) is 32 - every process this whole boot
     * sequence has created so far, across every self-test above, adds
     * up to comfortably fewer than that (confirmed directly: no
     * "process table grew" log line appears anywhere before this
     * point in a real boot), so nothing earlier in this file has ever
     * actually exercised growth. This loop execs and waits on 45
     * separate, tiny, already-proven processes (HELLOC.ELF, unrelated
     * to anything else this file tests) one after another -
     * comfortably past 32 - and since process slots are still never
     * recycled even after this phase (see growtable.rs's own comment
     * on why that stays a deliberately separate question), this is
     * real, cumulative, one-boot process-table growth, not a
     * contrived synthetic stress case: exactly the same "many
     * processes across one boot, most already finished" pattern that
     * genuinely exhausted the OLD fixed ceiling once before (see
     * process.h's own Phase 72 history) - just now past 32 instead of
     * past 16. Every single one succeeding (a real pid AND exit code
     * 0) is the actual proof; any failure here means the table either
     * failed to grow or grew incorrectly. */
    bool growth_ok = true;
    for (int i = 0; i < 45; i++) {
        const char* argv_h[] = {"HELLOC.ELF"};
        int pid_h = sys_exec_trusted("HELLOC.ELF", argv_h, 1);
        /* HELLOC.ELF deliberately returns 7, not 0 - see userland/
         * examples/hello.c's own comment (a specific, checkable value
         * distinct from HELLO.ELF's own 42, so make test's log-based
         * checks can tell the two apart). */
        int code_h = (pid_h >= 0) ? sys_wait(pid_h) : -1;
        if (pid_h < 0 || code_h != 7) {
            growth_ok = false;
        }
    }
    sys_write(growth_ok
                  ? "[sandbox] PASS: process table grew past its original "
                    "32-slot starting capacity - 45 processes exec'd and "
                    "waited on, one after another, in this one boot.\n"
                  : "[sandbox] FAIL: the process table did not grow "
                    "correctly - at least one of 45 sequential execs past "
                    "its original capacity did not succeed cleanly.\n");

    /* Phase 78: real, multi-socket UDP - UDPTEST.ELF (userland/net-rs/
     * udptest.rs) builds a genuine DNS query by hand and exercises the
     * new raw socket API (SYS_SOCKET_UDP/SYS_SENDTO/SYS_CONNECT/SYS_
     * WRITE_HANDLE), both the unconnected and connected paths,
     * printing its own detailed [udptest] PASS/FAIL/INFO lines. Only
     * socket creation, DNS query construction, and SYS_CONNECT's own
     * pure local bookkeeping (no network I/O at all - see kernel/
     * rust/udp.rs's own rust_udp_connect()) are hard-asserted on, via
     * this program's own exit code - a real, previously unconsidered
     * dependency this phase's own testing surfaced: ip_send() ARP-
     * resolves its next hop SYNCHRONOUSLY as part of sending, so even
     * the SEND itself (not just any reply) can legitimately fail when
     * this environment's own virtual network is unreachable, not only
     * the receive side. Whether a real packet actually made it onto
     * the wire or a real reply came back is logged in full detail but
     * never hard-asserted, the same honest reason kernel/init/main.c's
     * own real DNS/HTTP self-tests against example.com are logged, not
     * hard-asserted, anywhere in tools/python/test_runner.py. No
     * special capability needed (UDP sendto/connect have no allowed_
     * hosts[] gate - see kernel/rust/udp.rs's own top comment for
     * exactly why), so plain sys_exec_trusted() here delegates nothing
     * this program actually uses - called anyway, for the same
     * consistency with this function's other calls as everywhere else
     * in this file. */
    const char* argv_udp[] = {"UDPTEST.ELF"};
    int pid_udp = sys_exec_trusted("UDPTEST.ELF", argv_udp, 1);
    int code_udp = (pid_udp >= 0) ? sys_wait(pid_udp) : -1;
    sys_write(pid_udp >= 0 && code_udp == 0
                  ? "[sandbox] PASS: UDPTEST.ELF (real UDP sockets, both "
                    "the unconnected and connected API paths) created "
                    "its sockets and connected correctly.\n"
                  : "[sandbox] FAIL: UDPTEST.ELF did not exit cleanly - "
                    "UDP socket creation or SYS_CONNECT itself failed, "
                    "not merely a network-unreachable send.\n");

    /* Phase 81: the framebuffer graphics API (SYS_FB_*) - GFXTEST.ELF
     * (userland/gfxtest/gfxtest.c) is a ~120-check conformance suite
     * that runs the whole API from ring 3 against the real kernel and
     * checks the real display contents by reading them back, printing
     * its own "[gfxtest] PASS/FAIL" lines. It needs no delegated
     * capability (drawing to the screen reads nothing sensitive - see
     * kernel/arch/x86/cpu/syscall.c's SYS_FB_* handlers), so plain
     * sys_exec() is the right call here, not sys_exec_trusted():
     * handing a test program rights it does not use would only weaken
     * what the test shows.
     *
     * Order matters. "leak" first: it acquires the display and exits
     * WITHOUT releasing it. Each full run below must then be able to
     * acquire the display (the process-exit hook handed it back) and
     * must find it black (acquire cleared the leaker's picture) - the
     * suite's first checks. Then the suite once per backend: "auto"
     * (whatever the kernel picks), "vbe" and "gpu" explicitly, so BOTH
     * display paths - the CPU framebuffer copy and the virtio-gpu
     * TRANSFER_TO_HOST_2D + RESOURCE_FLUSH path - are exercised end to
     * end on every boot. */
    static const char* const gfx_leak[] = {"GFXTEST.ELF", "vbe", "leak"};
    static const char* const gfx_auto[] = {"GFXTEST.ELF", "auto"};
    static const char* const gfx_vbe[]  = {"GFXTEST.ELF", "vbe"};
    static const char* const gfx_gpu[]  = {"GFXTEST.ELF", "gpu"};

    int pid_gl = sys_exec("GFXTEST.ELF", (const char**)gfx_leak, 3);
    int code_gl = (pid_gl >= 0) ? sys_wait(pid_gl) : -1;
    sys_write(pid_gl >= 0 && code_gl == 0
                  ? "[sandbox] PASS: GFXTEST.ELF leak (a process that exits "
                    "holding the display).\n"
                  : "[sandbox] FAIL: GFXTEST.ELF leak run did not exit "
                    "cleanly.\n");

    int pid_ga = sys_exec("GFXTEST.ELF", (const char**)gfx_auto, 2);
    int code_ga = (pid_ga >= 0) ? sys_wait(pid_ga) : -1;
    sys_write(pid_ga >= 0 && code_ga == 0
                  ? "[sandbox] PASS: GFXTEST.ELF auto (framebuffer API "
                    "conformance, default backend).\n"
                  : "[sandbox] FAIL: GFXTEST.ELF auto - the framebuffer "
                    "API conformance suite did not pass.\n");

    int pid_gv = sys_exec("GFXTEST.ELF", (const char**)gfx_vbe, 2);
    int code_gv = (pid_gv >= 0) ? sys_wait(pid_gv) : -1;
    sys_write(pid_gv >= 0 && code_gv == 0
                  ? "[sandbox] PASS: GFXTEST.ELF vbe (framebuffer API "
                    "conformance, VESA/VBE backend).\n"
                  : "[sandbox] FAIL: GFXTEST.ELF vbe - the framebuffer "
                    "API conformance suite did not pass.\n");

    int pid_gg = sys_exec("GFXTEST.ELF", (const char**)gfx_gpu, 2);
    int code_gg = (pid_gg >= 0) ? sys_wait(pid_gg) : -1;
    sys_write(pid_gg >= 0 && code_gg == 0
                  ? "[sandbox] PASS: GFXTEST.ELF gpu (framebuffer API "
                    "conformance, virtio-gpu backend).\n"
                  : "[sandbox] FAIL: GFXTEST.ELF gpu - the framebuffer "
                    "API conformance suite did not pass.\n");

    /* Phase 83: shared-memory IPC. SHMTEST.ELF forks real peers and checks
     * the whole SYS_SHM_* contract from ring 3 (genuine sharing vs. a
     * private-page control, the ACL, read-only honoured by the kernel, owner
     * exit, leak-freedom, and a producer process feeding a compositor
     * process whose output is read back from the display). Like GFXTEST it
     * needs no delegated capability: what a process may map is decided by
     * the per-object ACL, so plain sys_exec() - not sys_exec_trusted() - is
     * the right call. The default run covers every group; the "gpu" run
     * repeats only the pixel handoff on the virtio-gpu backend. */
    static const char* const shm_all[] = {"SHMTEST.ELF"};
    static const char* const shm_gpu[] = {"SHMTEST.ELF", "gpu"};

    int pid_sa = sys_exec("SHMTEST.ELF", (const char**)shm_all, 1);
    int code_sa = (pid_sa >= 0) ? sys_wait(pid_sa) : -1;
    sys_write(pid_sa >= 0 && code_sa == 0
                  ? "[sandbox] PASS: SHMTEST.ELF (shared-memory IPC "
                    "conformance, default display backend).\n"
                  : "[sandbox] FAIL: SHMTEST.ELF - the shared-memory IPC "
                    "conformance suite did not pass.\n");

    int pid_sg = sys_exec("SHMTEST.ELF", (const char**)shm_gpu, 2);
    int code_sg = (pid_sg >= 0) ? sys_wait(pid_sg) : -1;
    sys_write(pid_sg >= 0 && code_sg == 0
                  ? "[sandbox] PASS: SHMTEST.ELF gpu (pixel handoff "
                    "through shared memory, virtio-gpu backend).\n"
                  : "[sandbox] FAIL: SHMTEST.ELF gpu - the shared-memory "
                    "pixel handoff did not pass on the virtio-gpu backend.\n");

    /* Phase 84: app-to-app messaging. MSGTEST.ELF forks real peers - an
     * "editor" service the "file manager" (itself) finds by name, an
     * allowlisted sink, a second sender - and checks the whole SYS_MSG_*
     * contract from ring 3: framing, kernel-stamped identity, selective
     * receive, never-consumed-on-error, fairness between senders, names
     * released at exit, and an 8KB payload handed over by shared-memory
     * handle. Like GFXTEST and SHMTEST it needs no delegated capability
     * (what a process receives is decided by its own inbox policy), so plain
     * sys_exec() is the right call. */
    static const char* const msg_all[] = {"MSGTEST.ELF"};
    int pid_m = sys_exec("MSGTEST.ELF", (const char**)msg_all, 1);
    int code_m = (pid_m >= 0) ? sys_wait(pid_m) : -1;
    sys_write(pid_m >= 0 && code_m == 0
                  ? "[sandbox] PASS: MSGTEST.ELF (app-to-app messaging "
                    "conformance).\n"
                  : "[sandbox] FAIL: MSGTEST.ELF - the app-to-app messaging "
                    "conformance suite did not pass.\n");

    sys_exit(0);

    for (;;) { }
}
