#ifndef NOVASYS_H
#define NOVASYS_H

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

#endif
