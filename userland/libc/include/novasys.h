#ifndef NOVASYS_H
#define NOVASYS_H

#include <stdbool.h>
#include "nova_fb_abi.h"
#include "nova_shm_abi.h"
#include "nova_msg_abi.h"
#include "nova_audio_abi.h"
#include "nova_rlimit_abi.h"

/* Raw syscall numbers and wrappers - NovaOS's own convention (int
 * 0x80, EAX = number, EBX/ECX/EDX = up to three arguments), NOT
 * Linux's syscall ABI. Mirrors kernel/arch/x86/cpu/syscall.h exactly;
 * kept as a separate copy here rather than a shared header because
 * userland code is compiled completely separately from the kernel
 * (different toolchain flags, no access to kernel headers) - the same
 * reason libc headers are never literally the kernel's own headers on
 * a real OS either. */

#define SYS_WRITE    1
#define SYS_EXIT     2
#define SYS_YIELD    3
#define SYS_OPEN     4
#define SYS_READ     5
#define SYS_CLOSE    6
#define SYS_NET_SEND 7
#define SYS_SPAWN    8
#define SYS_EXEC     9
#define SYS_WAIT     10
#define SYS_SBRK     11
#define SYS_FORK     12
#define SYS_READ_KEY 13
#define SYS_LIST_FILES 14
#define SYS_RTC_READ 15
#define SYS_LSPCI 16
#define SYS_BEEP 17
#define SYS_WRITE_FILE 18
#define SYS_DELETE_FILE 19
#define SYS_GFX_ENTER 20
#define SYS_GFX_EXIT 21
#define SYS_GFX_PUT_PIXEL 22
#define SYS_GFX_FILL_RECT 23
#define SYS_MOUSE_READ 24
#define SYS_PING_START 25
#define SYS_PING_POLL 26
#define SYS_PIPE 27
#define SYS_WRITE_HANDLE 28
#define SYS_LOGIN 29
#define SYS_GETUID 30
#define SYS_SUDO 31
#define SYS_SHUTDOWN 32
#define SYS_SOCKET   33
#define SYS_BIND     34
#define SYS_LISTEN   35
#define SYS_ACCEPT   36
#define SYS_CONNECT  37
#define SYS_EXEC_TRUSTED 38
#define SYS_DNS_RESOLVE 39
#define SYS_TFTP_FETCH 40
#define SYS_WAIT_NONBLOCK 41
#define SYS_EXEC_ENV 42
#define SYS_EXEC_TRUSTED_ENV 43

/* Phase 73: the largest environment a process may hand to a child at
 * exec time - mirrors MAX_EXEC_ENV / MAX_EXEC_ENV_BYTES in
 * kernel/task/process.h exactly (kept as a separate copy for the same
 * reason every other constant in this header is: userland is built
 * without access to kernel headers). NOVA_ENV_MAX_BYTES counts every
 * "NAME=value" string plus its NUL terminator. libc's own setenv()/
 * putenv() refuse to grow the environment past these limits, so a
 * process can never build an environment its own children could not
 * inherit - see stdlib.c. */
#define NOVA_ENV_MAX_VARS  32
#define NOVA_ENV_MAX_BYTES 4096

/* Matches kernel/drivers/mouse/ps2mouse.h's mouse_state_t exactly
 * (verified with a standalone -m32 sizeof/offsetof check: 12 bytes,
 * dx@0/dy@4/left@8/right@9/middle@10 - neither side uses
 * __attribute__((packed)), so both need the same compiler's default
 * alignment rules, which -m32 on both sides guarantees). */
typedef struct {
    int dx;
    int dy;
    bool left_button;
    bool right_button;
    bool middle_button;
} nova_mouse_state_t;

/* Matches kernel/drivers/rtc/rtc.h's rtc_time_t exactly (same target,
 * same simple POD layout, no padding either side needs to worry
 * about) - SYS_RTC_READ writes the kernel's own struct directly into
 * whatever buffer this points at. */
typedef struct {
    unsigned short year;
    unsigned char month;
    unsigned char day;
    unsigned char hour;
    unsigned char minute;
    unsigned char second;
} nova_rtc_time_t;

int sys_write(const char* str);
void sys_exit(int code) __attribute__((noreturn));
void sys_yield(void);
int sys_open(const char* filename);
int sys_read(int handle, void* buf, int max_len);
void sys_close(int handle);
int sys_spawn(void);
/* Phase 73: sys_exec() and sys_exec_trusted() below now pass the
 * calling process's own `environ` to the new process (Unix-style
 * environment inheritance) by way of SYS_EXEC_ENV/SYS_EXEC_TRUSTED_ENV.
 * sys_exec_env()/sys_exec_trusted_env() are the explicit forms for a
 * caller that wants to hand over a different environment: `envp` is a
 * NULL-terminated array of "NAME=value" strings, or NULL for an empty
 * one. They return the new pid, or -1 on any failure - including an
 * environment over NOVA_ENV_MAX_VARS/NOVA_ENV_MAX_BYTES. The original
 * SYS_EXEC/SYS_EXEC_TRUSTED syscall numbers (which always give the
 * child an empty environment) still exist in the kernel, untouched,
 * for the raw-`int 0x80` callers that predate this. */
int sys_exec(const char* path, char** argv, int argc);
int sys_exec_env(const char* path, char** argv, int argc, char** envp);
int sys_exec_trusted_env(const char* path, char** argv, int argc,
                          char** envp);
int sys_wait(int pid);
int sys_wait_nonblock(int pid, int* out_exit_code);
void* sys_sbrk(int increment);
int sys_fork(void);
int sys_read_key(void);
int sys_list_files(char* buf, int buf_size);
int sys_rtc_read(nova_rtc_time_t* out);
int sys_lspci(char* buf, int buf_size);
int sys_beep(void);
int sys_write_file(const char* filename, const void* data, unsigned int size);
int sys_delete_file(const char* filename);
/* Phase 83: shared-memory IPC. Model, structs and limits are in
 * nova_shm_abi.h; novashm.h wraps these in a friendlier API plus the
 * frame-handoff protocol. Every call returns 0 / a non-negative result, or
 * a NEGATIVE errno (not -1 plus errno). */
/* Phase 84: app-to-app messaging. Model, structs and limits are in
 * nova_msg_abi.h; novamsg.h wraps these and adds waiting (nothing blocks
 * in the kernel). Every call returns 0 / a non-negative result, or a
 * NEGATIVE errno. */
/* Phase 85: audio mixing. Model, structs and limits are in nova_audio_abi.h;
 * novaaudio.h wraps these (and adds write_all(), tone generation). Every call
 * returns 0 / a non-negative result, or a NEGATIVE errno. */
/* Phase 86: per-process resource limits. Model, rules and the usage fields are
 * in nova_rlimit_abi.h. Returns 0, or a NEGATIVE errno. */
int sys_rlimit(nova_rlimit_t* req);

int sys_audio_open(nova_audio_open_t* req);
int sys_audio_write(nova_audio_write_t* req);
int sys_audio_ctl(nova_audio_ctl_t* req);
int sys_audio_close(const nova_audio_close_t* req);

int sys_msg_open(nova_msg_open_t* req);
int sys_msg_close(void);
int sys_msg_send(const nova_msg_send_t* req);
int sys_msg_recv(nova_msg_recv_t* req);
int sys_msg_service(nova_msg_service_t* req);
int sys_msg_ctl(nova_msg_ctl_t* req);

int sys_shm_create(nova_shm_create_t* req);
int sys_shm_grant(const nova_shm_grant_t* req);
int sys_shm_map(nova_shm_map_t* req);
int sys_shm_unmap(unsigned int addr);
int sys_shm_destroy(unsigned int handle);
int sys_shm_info(nova_shm_info_t* req);
int sys_fb_info(nova_fb_info_t* out);
int sys_fb_acquire(unsigned int backend);
int sys_fb_release(void);
int sys_fb_create(nova_fb_create_t* req);
int sys_fb_destroy(unsigned int handle);
int sys_fb_present(const nova_fb_present_t* req);
int sys_fb_readback(const nova_fb_readback_t* req);
void sys_gfx_enter(void);
void sys_gfx_exit(void);
void sys_gfx_put_pixel(int x, int y, unsigned char color);
void sys_gfx_fill_rect(int x, int y, int w, int h, unsigned char color);
int sys_mouse_read(nova_mouse_state_t* out);
void sys_ping_start(unsigned int dest_ip);
int sys_ping_poll(unsigned int* out_rtt);

/* Phase 36: kernel pipes. sys_pipe() fills out_handles[0]/[1] with a
 * fresh {read_handle, write_handle} pair, returning 0 on success or
 * -1 on failure. The read end is read with the existing sys_read()
 * and closed with the existing sys_close() - both already dispatch
 * correctly on a pipe handle (see kernel/arch/x86/cpu/syscall.c);
 * only writing needed a new syscall at all, since nothing before this
 * phase could write to a handle rather than a whole named file.
 * sys_write_handle() returns bytes actually written (may be less than
 * `len` - see kernel/rust/pipe.rs), or -1. */
int sys_pipe(int out_handles[2]);
int sys_write_handle(int handle, const void* buf, int len);

/* Phase 47: SYS_LOGIN/SYS_GETUID - see kernel/arch/x86/cpu/syscall.h's
 * own comment on each for the full argument/return contract.
 * sys_login() returns 0 on success (the calling process's own uid/gid
 * are updated kernel-side) or -1 on any credential mismatch.
 * sys_getuid() always succeeds, returning the calling process's
 * current uid. */
int sys_login(const char* username, const char* password);
unsigned int sys_getuid(void);

/* Phase 51: SYS_SUDO - see kernel/arch/x86/cpu/syscall.h's own
 * comment for the full contract. Returns 0 on success (the calling
 * process's own uid/gid become 0/0) or -1 on any failure - wrong
 * password, not in the admin group, or locked out, deliberately not
 * distinguished. */
int sys_sudo(const char* password);

/* Phase 55: SYS_SHUTDOWN - see kernel/arch/x86/cpu/syscall.h's own
 * comment for the full contract. On a real, working ACPI shutdown
 * this call does not return at all. If it does return, the machine
 * is still running - the return value is whatever
 * kernel/rust/acpi.rs's own rust_acpi_shutdown() reported: -1 no ACPI
 * present, -2 no usable FADT, -3 no _S5 package found in the DSDT,
 * -4/-5 this machine needed an ACPI-enable handshake that failed, -6
 * the real S5 write was issued but had no effect. */
int sys_shutdown(void);

/* Phase 58: a real Berkeley-sockets-style API - see
 * kernel/arch/x86/cpu/syscall.h's own comment on each syscall number
 * for the full contract; kernel/rust/tcp.rs implements the real TCP
 * state machine behind all of these.
 *
 * sys_socket() takes no arguments (this kernel only ever creates an
 * IPv4/TCP socket) and returns a handle, or -1. sys_bind()'s `port` is
 * host byte order; 0 auto-assigns an ephemeral port. sys_listen()'s
 * `backlog` is accepted for Berkeley-sockets shape compatibility but
 * currently unused (the real backlog is a small fixed size).
 * sys_accept() blocks until a connection completes its handshake and
 * returns a brand new handle for it, or -1. sys_connect()'s
 * `dest_ip`/`dest_port` match sys_ping_start()'s own convention
 * (dest_ip built with e.g. ip_make(), host byte order); blocks until
 * ESTABLISHED or failure, returning 0 or -1.
 *
 * Once connected, an existing socket handle is read with the existing
 * sys_read() (-2 return means "would block", not an error - more data
 * may still arrive), written with the existing sys_write_handle(), and
 * torn down with the existing sys_close() - all three already dispatch
 * correctly on a socket handle exactly the way they already do on a
 * pipe handle (see kernel/arch/x86/cpu/syscall.c); no new read/write/
 * close syscalls were needed for this phase. */
int sys_socket(void);
int sys_bind(int handle, unsigned short port);
int sys_listen(int handle, int backlog);
int sys_accept(int handle);
int sys_connect(int handle, unsigned int dest_ip, unsigned short dest_port);

/* Phase 59: identical to sys_exec() in every respect but one - see
 * kernel/arch/x86/cpu/syscall.h's own comment on SYS_EXEC_TRUSTED for
 * the full reasoning. Delegates the *calling* process's own "may open
 * any file" capability to the new process, instead of always granting
 * nothing. Harmless to call from an ordinary, unprivileged program
 * (delegating "false" is a no-op, identical to plain sys_exec()) -
 * only meaningfully different when the caller already has that
 * capability itself (in practice, only the interactive ring-3 shell). */
int sys_exec_trusted(const char* path, char** argv, int argc);

/* Phase 60: closes the release-readiness doc's own 2.1 row - see
 * kernel/arch/x86/cpu/syscall.h's own comments on SYS_DNS_RESOLVE/
 * SYS_TFTP_FETCH for the full reasoning. sys_dns_resolve() is
 * un-gated and safe for any program to call; sys_tftp_fetch() needs
 * the calling process's own can_open_any_file (it creates/overwrites
 * a local file), so - like sys_write_file()/sys_delete_file() - it
 * only actually succeeds when launched via sys_exec_trusted() rather
 * than plain sys_exec(). */
int sys_dns_resolve(const char* hostname, unsigned int* out_ip);
int sys_tftp_fetch(unsigned int server_ip, const char* remote_filename,
                    const char* local_filename);

/* Phase 78: real, multi-socket UDP - see kernel/rust/udp.rs's own
 * module comment for the full design. sys_socket_udp() is a new,
 * separate syscall from sys_socket() (TCP) rather than a type
 * argument added to it, because sys_socket()'s own existing callers
 * (and this file's own ABI, matching how every other syscall number
 * here is a small, stable integer a raw int-0x80 caller might also
 * use directly) never set a register for such an argument to land in
 * - see kernel/arch/x86/cpu/syscall.h's own comment on SYS_SOCKET_UDP
 * for the full reasoning, the identical hazard and the identical fix
 * Phase 74's SYS_EXEC_ENV/SYS_EXEC_TRUSTED_ENV already established. */
#define SYS_SOCKET_UDP 44
#define SYS_SENDTO 45
#define SYS_RECVFROM 46

/* Phase 81: the framebuffer graphics API. The model, structs, limits
 * and error conditions are all in nova_fb_abi.h (included above);
 * novagfx.h wraps these in a friendlier surface API with drawing
 * helpers. Every call returns 0 (or a non-negative result) on success
 * and a NEGATIVE errno on failure - NOT -1 plus errno. */
#define SYS_FB_INFO     47
#define SYS_FB_ACQUIRE  48
#define SYS_FB_RELEASE  49
#define SYS_FB_CREATE   50
#define SYS_FB_DESTROY  51
#define SYS_FB_PRESENT  52
#define SYS_FB_READBACK 53

/* The exact 6-byte wire layout kernel/arch/x86/cpu/syscall.c's own
 * SYS_SENDTO/SYS_RECVFROM read/write directly at a pointer passed in
 * a register - see that file's own nova_udp_addr_t, which this
 * mirrors byte-for-byte (kept in sync by comment and convention
 * rather than a shared header, this project's established convention
 * for a small FFI-boundary struct). `ip` is host byte order, matching
 * every other IP address this kernel passes around (ip_make(),
 * sys_connect()'s own dest_ip, ...). */
typedef struct {
    unsigned int ip;
    unsigned short port;
} nova_udp_addr_t;

/* sys_socket_udp()/sys_bind()/sys_connect() (the latter two already
 * declared above, reused unchanged - a UDP handle dispatches
 * correctly through them exactly as a TCP one already does) create
 * and configure a UDP socket. Once connected (sys_connect()), the
 * existing sys_read()/sys_write_handle()/sys_close() work against
 * that one fixed peer - no new syscalls needed for that case, the
 * same "reuse read/write/close" design SYS_SOCKET's own Phase 58
 * comment already established for TCP.
 *
 * sys_sendto()/sys_recvfrom() are the general, per-packet-addressed
 * case connect()+read/write cannot express at all: sys_sendto() sends
 * to `addr` regardless of whether the socket is connected (a
 * connected socket's own recorded peer is left untouched - real
 * sendto() semantics), returning the number of bytes sent or -1.
 * sys_recvfrom() is non-blocking - 0 if nothing has arrived yet (never
 * blocks the way sys_accept() does), the byte count copied (truncated
 * to `max_len` if the real datagram was larger - the honest, real UDP
 * "the rest is simply gone" contract) with `*out_addr` filled in with
 * who it was actually from, or -1. Both are UDP-only - calling either
 * on a TCP handle fails, use sys_read()/sys_write_handle() for that. */
int sys_socket_udp(void);
int sys_sendto(int handle, const nova_udp_addr_t* addr, const void* buf,
               unsigned int len);
int sys_recvfrom(int handle, void* buf, unsigned int max_len,
                  nova_udp_addr_t* out_addr);

#endif
