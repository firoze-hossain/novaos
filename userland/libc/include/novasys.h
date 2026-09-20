#ifndef NOVASYS_H
#define NOVASYS_H

#include <stdbool.h>

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
int sys_exec(const char* path, char** argv, int argc);
int sys_wait(int pid);
void* sys_sbrk(int increment);
int sys_fork(void);
int sys_read_key(void);
int sys_list_files(char* buf, int buf_size);
int sys_rtc_read(nova_rtc_time_t* out);
int sys_lspci(char* buf, int buf_size);
int sys_beep(void);
int sys_write_file(const char* filename, const void* data, unsigned int size);
int sys_delete_file(const char* filename);
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

#endif
