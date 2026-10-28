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

    /* Phase 88: the stateful firewall. FWTEST.ELF is the unconfined root and
     * firewall administrator: it drives SYS_FW_CTL against the real filter -
     * probes, hand-built TCP/UDP/ICMP flows, the 256-flow table limit, rule
     * management, one real TFTP download through the filter - and last locks
     * the rules one way. Started TRUSTED so it keeps this task's capabilities.
     *
     * It runs FIRST, not last, though it ends with the one-way lock: its
     * live-traffic group needs the emulated network to answer, and that only
     * happens early in the boot. Later, with the other suites competing for
     * the host's CPUs, replies come seconds late or not at all (see the "no
     * reply" notes of udptest and the ping demos). Nothing after it changes a
     * rule, so the lock costs the others nothing. */
    static const char* const fw_all[] = {"FWTEST.ELF"};
    int pid_fw = sys_exec_trusted("FWTEST.ELF", (char**)fw_all, 1);
    int code_fw = (pid_fw >= 0) ? sys_wait(pid_fw) : -1;
    sys_write(pid_fw >= 0 && code_fw == 0
                  ? "[sandbox] PASS: FWTEST.ELF (stateful firewall "
                    "conformance).\n"
                  : "[sandbox] FAIL: FWTEST.ELF - the stateful firewall "
                    "conformance suite did not pass.\n");

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

    /* Phase 85: audio mixing. AUDTEST.ELF plays several streams at once -
     * its own and forked children's, at different rates and channel counts -
     * through the real AC97 driver's DMA ring, reads the mixer's output back
     * through its root-only tap, and checks the mix exactly (constants) or
     * by integer Goertzel analysis (tones). Like the other conformance
     * programs it needs no delegated capability: a stream belongs to its
     * opener, and the controls that affect everyone are gated on uid inside
     * the kernel, so plain sys_exec() is the right call. */
    static const char* const audio_all[] = {"AUDTEST.ELF"};
    int pid_a = sys_exec("AUDTEST.ELF", (const char**)audio_all, 1);
    int code_a = (pid_a >= 0) ? sys_wait(pid_a) : -1;
    sys_write(pid_a >= 0 && code_a == 0
                  ? "[sandbox] PASS: AUDTEST.ELF (audio mixing "
                    "conformance).\n"
                  : "[sandbox] FAIL: AUDTEST.ELF - the audio mixing "
                    "conformance suite did not pass.\n");

    /* Phase 86: per-process resource limits. RLIMTEST.ELF creates real
     * runaways - processes that spin forever without a system call, that grow
     * the heap until stopped, and that fork until stopped - and checks that
     * the limits end them: a CPU-time limit terminating a process from the
     * timer tick (on either CPU), a CPU share throttling one in the
     * scheduler, a memory limit stopping the heap path at exactly the limit,
     * and a process-count limit that a fork bomb cannot exceed. Like the
     * other conformance programs it needs no delegated capability. */
    static const char* const rlimit_all[] = {"RLIMTEST.ELF"};
    int pid_r = sys_exec("RLIMTEST.ELF", (const char**)rlimit_all, 1);
    int code_r = (pid_r >= 0) ? sys_wait(pid_r) : -1;
    sys_write(pid_r >= 0 && code_r == 0
                  ? "[sandbox] PASS: RLIMTEST.ELF (per-process resource "
                    "limits conformance).\n"
                  : "[sandbox] FAIL: RLIMTEST.ELF - the resource limits "
                    "conformance suite did not pass.\n");

    /* Phase 89: structured, queryable kernel logging subsystem. KLOGTEST.ELF
     * exercises SYS_KLOG_INFO, SYS_KLOG_WRITE, SYS_KLOG_READ, severity and
     * facility filtering, incremental streaming, and hostile pointers from
     * ring 3. Needs no delegated capability. */
    static const char* const klog_all[] = {"KLOGTEST.ELF"};
    int pid_klog = sys_exec("KLOGTEST.ELF", (const char**)klog_all, 1);
    int code_klog = (pid_klog >= 0) ? sys_wait(pid_klog) : -1;
    sys_write(pid_klog >= 0 && code_klog == 0
                  ? "[sandbox] PASS: KLOGTEST.ELF (structured kernel logging "
                    "conformance).\n"
                  : "[sandbox] FAIL: KLOGTEST.ELF - the structured kernel logging "
                    "conformance suite did not pass.\n");

    /* Phase 90: package-repository network protocol support, server side.
     * PKGRTEST.ELF exercises SYS_PKGREPO_CTL, invalid/hostile pointers,
     * server start/stop lifecycle, port configuration, and running state
     * verification from ring 3. */
    static const char* const pkgrepo_all[] = {"PKGRTEST.ELF"};
    int pid_pkgrepo = sys_exec("PKGRTEST.ELF", (const char**)pkgrepo_all, 1);
    int code_pkgrepo = (pid_pkgrepo >= 0) ? sys_wait(pid_pkgrepo) : -1;
    sys_write(pid_pkgrepo >= 0 && code_pkgrepo == 0
                  ? "[sandbox] PASS: PKGRTEST.ELF (package repository "
                    "conformance).\n"
                  : "[sandbox] FAIL: PKGRTEST.ELF - the package repository "
                    "conformance suite did not pass.\n");

    /* Month 3 - Core desktop apps: FILES.ELF --selftest.
     * Proves VFS directory listing, metadata classification, mouse selection,
     * text file preview, move, and delete operations from ring 3. */
    static const char* const files_all[] = {"FILES.ELF", "--selftest"};
    int pid_files = sys_exec_trusted("FILES.ELF", (char**)files_all, 2);
    int code_files = (pid_files >= 0) ? sys_wait(pid_files) : -1;
    sys_write(pid_files >= 0 && code_files == 0
                  ? "[sandbox] PASS: FILES.ELF --selftest - file browser, "
                    "open, move, and delete all behaved correctly.\n"
                  : "[sandbox] FAIL: FILES.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 3 & Month 4: SETTINGS.ELF --selftest.
     * Proves modular plugin-panel settings architecture, reading and
     * writing SYSTEM.CFG, USERS.CFG, and SERVICES.CFG, plus live
     * kernel firewall telemetry and rule table (SYS_FW_INFO, SYS_FW_CTL). */
    static const char* const settings_all[] = {"SETTINGS.ELF", "--selftest"};
    int pid_settings = sys_exec_trusted("SETTINGS.ELF", (char**)settings_all, 2);
    int code_settings = (pid_settings >= 0) ? sys_wait(pid_settings) : -1;
    sys_write(pid_settings >= 0 && code_settings == 0
                  ? "[sandbox] PASS: SETTINGS.ELF --selftest - settings shell, "
                    "plugin panels, and live firewall rule table all behaved correctly.\n"
                  : "[sandbox] FAIL: SETTINGS.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 3 - Core desktop apps: TERM.ELF --selftest.
     * Proves in-window terminal emulator, VT cell grid, character flow,
     * wrapping, scrolling, ANSI escape sequence decoding, interactive line
     * editing, and shell command execution dispatch from ring 3. */
    static const char* const term_all[] = {"TERM.ELF", "--selftest"};
    int pid_term = sys_exec_trusted("TERM.ELF", (char**)term_all, 2);
    int code_term = (pid_term >= 0) ? sys_wait(pid_term) : -1;
    sys_write(pid_term >= 0 && code_term == 0
                  ? "[sandbox] PASS: TERM.ELF --selftest - in-window terminal "
                    "wrapper and shell output dispatch all behaved correctly.\n"
                  : "[sandbox] FAIL: TERM.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 3 - Core desktop apps: EDIT.ELF --selftest.
     * Proves multi-line text buffer, line splitting/merging, cursor navigation,
     * typography scaling, file loading/saving roundtrip, and toolbar interaction. */
    static const char* const edit_all[] = {"EDIT.ELF", "--selftest"};
    int pid_edit = sys_exec_trusted("EDIT.ELF", (char**)edit_all, 2);
    int code_edit = (pid_edit >= 0) ? sys_wait(pid_edit) : -1;
    sys_write(pid_edit >= 0 && code_edit == 0
                  ? "[sandbox] PASS: EDIT.ELF --selftest - text buffer, cursor navigation, "
                    "font rendering, and file IO roundtrip all behaved correctly.\n"
                  : "[sandbox] FAIL: EDIT.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 3 - Core desktop apps: STORE.ELF --selftest.
     * Proves package manifest parsing, HMAC-SHA256 signature verification,
     * install database record management (install/remove), repository catalog search,
     * in-kernel package repository HTTP telemetry (SYS_PKGREPO_CTL), and graphical store presentation. */
    static const char* const store_all[] = {"STORE.ELF", "--selftest"};
    int pid_store = sys_exec_trusted("STORE.ELF", (char**)store_all, 2);
    int code_store = (pid_store >= 0) ? sys_wait(pid_store) : -1;
    sys_write(pid_store >= 0 && code_store == 0
                  ? "[sandbox] PASS: STORE.ELF --selftest - package manifest parsing, "
                    "HMAC-SHA256 signature verification, install database state, and UI presentation all behaved correctly.\n"
                  : "[sandbox] FAIL: STORE.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 3 - Core desktop apps: LOGIN.ELF --selftest.
     * Proves USERS.CFG/SYSTEM.CFG parsing, SYS_LOGIN credentials authentication,
     * identity isolation & privilege transition (persisted uid 700, admin uid 800),
     * password buffer handling and masking, hero digital clock, session selector,
     * and graphical login presentation on BufferSurface32. */
    static const char* const login_all[] = {"LOGIN.ELF", "--selftest"};
    int pid_login = sys_exec_trusted("LOGIN.ELF", (char**)login_all, 2);
    int code_login = (pid_login >= 0) ? sys_wait(pid_login) : -1;
    sys_write(pid_login >= 0 && code_login == 0
                  ? "[sandbox] PASS: LOGIN.ELF --selftest - user account parsing, "
                    "SYS_LOGIN authentication, identity transition, and login UI presentation all behaved correctly.\n"
                  : "[sandbox] FAIL: LOGIN.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 4 - Server & admin UI: KLOG.ELF --selftest.
     * Proves structured kernel logging query, telemetry, severity and facility
     * filtering, incremental streaming, real-time keyword search, and
     * zero-syscall BufferSurface32 rasterization from ring 3. */
    static const char* const klog_app_all[] = {"KLOG.ELF", "--selftest"};
    int pid_klog_app = sys_exec_trusted("KLOG.ELF", (const char**)klog_app_all, 2);
    int code_klog_app = (pid_klog_app >= 0) ? sys_wait(pid_klog_app) : -1;
    sys_write(pid_klog_app >= 0 && code_klog_app == 0
                  ? "[sandbox] PASS: KLOG.ELF --selftest - structured log query, "
                    "telemetry, filtering, search, and inspector UI all behaved correctly.\n"
                  : "[sandbox] FAIL: KLOG.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 5 - Power-user & system apps: ACTMON.ELF --selftest.
     * Proves system activity metrics, physical memory telemetry, process
     * table queries, column sorting (CPU/Memory/PID), filtering, and
     * zero-syscall BufferSurface32 rasterization from ring 3. */
    static const char* const actmon_all[] = {"ACTMON.ELF", "--selftest"};
    int pid_actmon = sys_exec_trusted("ACTMON.ELF", (const char**)actmon_all, 2);
    int code_actmon = (pid_actmon >= 0) ? sys_wait(pid_actmon) : -1;
    sys_write(pid_actmon >= 0 && code_actmon == 0
                  ? "[sandbox] PASS: ACTMON.ELF --selftest - system activity monitoring, "
                    "CPU/memory telemetry, process table query, and UI presentation all behaved correctly.\n"
                  : "[sandbox] FAIL: ACTMON.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 6 - First impressions: INSTALL.ELF --selftest.
     * Proves wizard step machine, target disk selection, partition layout
     * dry-run calculation (SYS_DISK_INSTALL), user account credential synthesis
     * (PBKDF2-HMAC-SHA256), config generation, and zero-syscall BufferSurface32
     * graphical rasterization from ring 3. */
    static const char* const installer_all[] = {"INSTALL.ELF", "--selftest"};
    int pid_installer = sys_exec_trusted("INSTALL.ELF", (const char**)installer_all, 2);
    int code_installer = (pid_installer >= 0) ? sys_wait(pid_installer) : -1;
    sys_write(pid_installer >= 0 && code_installer == 0
                  ? "[sandbox] PASS: INSTALL.ELF --selftest - graphical installer wizard, "
                    "disk partitioning engine, account generation, and UI presentation all behaved correctly.\n"
                  : "[sandbox] FAIL: INSTALL.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 6 - First impressions: CRASHREP.ELF --selftest.
     * Proves crash report detection, reading /CRASHDMP.TXT and /CRASHDMP.DAT,
     * register context analysis, stack backtrace decoding, multi-tab state transitions
     * (Overview, Registers, Call Stack, Raw Report), SYS_CRASHDUMP_CTL (80) query & clear,
     * and zero-syscall BufferSurface32 graphical rasterization from ring 3. */
    static const char* const crashrep_all[] = {"CRASHREP.ELF", "--selftest"};
    int pid_crashrep = sys_exec_trusted("CRASHREP.ELF", (const char**)crashrep_all, 2);
    int code_crashrep = (pid_crashrep >= 0) ? sys_wait(pid_crashrep) : -1;
    sys_write(pid_crashrep >= 0 && code_crashrep == 0
                  ? "[sandbox] PASS: CRASHREP.ELF --selftest - crash report detection, "
                    "file parsing, register analysis, stack backtrace, and UI presentation all behaved correctly.\n"
                  : "[sandbox] FAIL: CRASHREP.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Month 6 - First impressions: FIRSTRUN.ELF --selftest.
     * Proves first-run setup wizard state machine transitions, hardware detection,
     * machine identity & hostname configuration (SYSTEM.CFG), administrator account
     * creation with PBKDF2-HMAC-SHA256 (USERS.CFG), network configuration & packet
     * filter policy (FIREWALL.CFG), visual preferences & accessibility (ACCESS.CFG),
     * and zero-syscall BufferSurface32 graphical rasterization from ring 3. */
    static const char* const firstrun_all[] = {"FIRSTRUN.ELF", "--selftest"};
    int pid_firstrun = sys_exec_trusted("FIRSTRUN.ELF", (const char**)firstrun_all, 2);
    int code_firstrun = (pid_firstrun >= 0) ? sys_wait(pid_firstrun) : -1;
    sys_write(pid_firstrun >= 0 && code_firstrun == 0
                  ? "[sandbox] PASS: FIRSTRUN.ELF --selftest - first-run setup wizard, "
                    "account creation, network setup, and preferences all behaved correctly.\n"
                  : "[sandbox] FAIL: FIRSTRUN.ELF --selftest reported at least "
                    "one failing case.\n");

    /* Phase 87: mandatory access control. MACTEST.ELF is the unconfined root
     * and policy administrator; it starts MACJAIL.ELF (and its other names),
     * each confined by its own NAME.MAC profile, and checks what the kernel
     * did to them: a root process with every capability refused each syscall,
     * file, peer and program its profile does not list; no escape by exec;
     * fails closed; the profile-stack limit; and last the one-way freeze. It
     * is started TRUSTED so it inherits this task's capabilities to delegate
     * them to the jail - the whole point is that the jail has every
     * capability and only the profile holds it. It runs LAST: the freeze at
     * its end cannot be undone until reboot. */
    static const char* const mac_all[] = {"MACTEST.ELF"};
    int pid_mac = sys_exec_trusted("MACTEST.ELF", (char**)mac_all, 1);
    int code_mac = (pid_mac >= 0) ? sys_wait(pid_mac) : -1;
    sys_write(pid_mac >= 0 && code_mac == 0
                  ? "[sandbox] PASS: MACTEST.ELF (mandatory access control "
                    "conformance).\n"
                  : "[sandbox] FAIL: MACTEST.ELF - the mandatory access "
                    "control conformance suite did not pass.\n");

    sys_exit(0);

    for (;;) { }
}
