/*
 * syscall.c - the actual int 0x80 wrappers (see novasys.h)
 */
#include "novasys.h"

int sys_write(const char* str) {
    int result = SYS_WRITE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(str)
                       : "memory", "cc");
    return result;
}

void sys_exit(int code) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_EXIT), "b"(code));
    __builtin_unreachable(); /* SYS_EXIT never returns */
}

void sys_yield(void) {
    __asm__ volatile ("int $0x80" : : "a"(SYS_YIELD) : "memory", "cc");
}

int sys_open(const char* filename) {
    int result = SYS_OPEN;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(filename)
                       : "memory", "cc");
    return result;
}

int sys_read(int handle, void* buf, int max_len) {
    int result = SYS_READ;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(buf), "d"(max_len)
                       : "memory", "cc");
    return result;
}

void sys_close(int handle) {
    __asm__ volatile ("int $0x80"
                       :
                       : "a"(SYS_CLOSE), "b"(handle)
                       : "memory", "cc");
}

int sys_spawn(void) {
    int result = SYS_SPAWN;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

/* Phase 73: the process's own environment, set once by crt0.asm from
 * the envp the kernel built on the initial stack and thereafter owned
 * by stdlib.c's setenv()/getenv() family. Defined in stdlib.c - the
 * one libc object every userland program already links - and only
 * *read* here. */
extern char** environ;

/* Phase 73: SYS_EXEC_ENV takes the child's environment in ESI, a fourth
 * register argument the original SYS_EXEC never had. That is exactly
 * why this is a new syscall number instead of an extra argument on the
 * old one: a raw `int 0x80` caller that predates this (kernel/task/
 * sandbox_demo.c's inline wrapper, say) never sets ESI, so its value
 * would be whatever the caller's last computation left in it - the
 * kernel would dereference garbage as an environment pointer. A
 * separate number leaves every existing caller exactly as it was. */
int sys_exec_env(const char* path, char** argv, int argc, char** envp) {
    int result = SYS_EXEC_ENV;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc), "S"(envp)
                       : "memory", "cc");
    return result;
}

/* The everyday form: the child inherits this process's environment,
 * as with any Unix exec/spawn. Every libc-linked program - C, and Rust
 * through its FFI to this same function - gets that behaviour without
 * a source change. A NULL `environ` (a program that never went through
 * crt0.asm, say) simply passes an empty environment - the kernel
 * treats a NULL envp identically to an empty one. */
int sys_exec(const char* path, char** argv, int argc) {
    return sys_exec_env(path, argv, argc, environ);
}

int sys_wait(int pid) {
    int result = SYS_WAIT;
    __asm__ volatile ("int $0x80" : "+a"(result) : "b"(pid) : "memory", "cc");
    return result;
}

/* Phase 71: see SYS_WAIT_NONBLOCK's own doc comment (novasys.h) for
 * the full contract - returns -2 (no such process), -1 (exists, still
 * running), or 0 (terminated, with *out_exit_code written). */
int sys_wait_nonblock(int pid, int* out_exit_code) {
    int result = SYS_WAIT_NONBLOCK;
    __asm__ volatile ("int $0x80" : "+a"(result) : "b"(pid), "c"(out_exit_code) : "memory", "cc");
    return result;
}

void* sys_sbrk(int increment) {
    int result = SYS_SBRK;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(increment)
                       : "memory", "cc");
    return (void*)result;
}

int sys_fork(void) {
    int result = SYS_FORK;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

int sys_read_key(void) {
    int result = SYS_READ_KEY;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

int sys_list_files(char* buf, int buf_size) {
    int result = SYS_LIST_FILES;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(buf), "c"(buf_size)
                       : "memory", "cc");
    return result;
}

int sys_rtc_read(nova_rtc_time_t* out) {
    int result = SYS_RTC_READ;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(out)
                       : "memory", "cc");
    return result;
}

int sys_lspci(char* buf, int buf_size) {
    int result = SYS_LSPCI;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(buf), "c"(buf_size)
                       : "memory", "cc");
    return result;
}

int sys_beep(void) {
    int result = SYS_BEEP;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

int sys_write_file(const char* filename, const void* data,
                    unsigned int size) {
    int result = SYS_WRITE_FILE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(filename), "c"(data), "d"(size)
                       : "memory", "cc");
    return result;
}

int sys_delete_file(const char* filename) {
    int result = SYS_DELETE_FILE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(filename)
                       : "memory", "cc");
    return result;
}

void sys_gfx_enter(void) {
    int eax = SYS_GFX_ENTER;
    __asm__ volatile ("int $0x80" : "+a"(eax) : : "memory", "cc");
}

void sys_gfx_exit(void) {
    int eax = SYS_GFX_EXIT;
    __asm__ volatile ("int $0x80" : "+a"(eax) : : "memory", "cc");
}

void sys_gfx_put_pixel(int x, int y, unsigned char color) {
    int eax = SYS_GFX_PUT_PIXEL;
    __asm__ volatile ("int $0x80"
                       : "+a"(eax)
                       : "b"(x), "c"(y), "d"((int)color)
                       : "memory", "cc");
}

void sys_gfx_fill_rect(int x, int y, int w, int h, unsigned char color) {
    int params[5] = {x, y, w, h, (int)color};
    int eax = SYS_GFX_FILL_RECT;
    __asm__ volatile ("int $0x80"
                       : "+a"(eax)
                       : "b"(params)
                       : "memory", "cc");
}

int sys_mouse_read(nova_mouse_state_t* out) {
    int result = SYS_MOUSE_READ;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(out)
                       : "memory", "cc");
    return result;
}

void sys_ping_start(unsigned int dest_ip) {
    int eax = SYS_PING_START;
    __asm__ volatile ("int $0x80"
                       : "+a"(eax)
                       : "b"(dest_ip)
                       : "memory", "cc");
}

int sys_ping_poll(unsigned int* out_rtt) {
    int result = SYS_PING_POLL;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(out_rtt)
                       : "memory", "cc");
    return result;
}

int sys_pipe(int out_handles[2]) {
    int result = SYS_PIPE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(out_handles)
                       : "memory", "cc");
    return result;
}

int sys_write_handle(int handle, const void* buf, int len) {
    int result = SYS_WRITE_HANDLE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(buf), "d"(len)
                       : "memory", "cc");
    return result;
}

int sys_login(const char* username, const char* password) {
    int result = SYS_LOGIN;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(username), "c"(password)
                       : "memory", "cc");
    return result;
}

unsigned int sys_getuid(void) {
    unsigned int result = SYS_GETUID;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       :
                       : "memory", "cc");
    return result;
}

int sys_sudo(const char* password) {
    int result = SYS_SUDO;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(password)
                       : "memory", "cc");
    return result;
}

int sys_shutdown(void) {
    int result = SYS_SHUTDOWN;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

int sys_socket(void) {
    int result = SYS_SOCKET;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

int sys_bind(int handle, unsigned short port) {
    int result = SYS_BIND;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"((int)port)
                       : "memory", "cc");
    return result;
}

int sys_listen(int handle, int backlog) {
    int result = SYS_LISTEN;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(backlog)
                       : "memory", "cc");
    return result;
}

int sys_accept(int handle) {
    int result = SYS_ACCEPT;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle)
                       : "memory", "cc");
    return result;
}

int sys_connect(int handle, unsigned int dest_ip, unsigned short dest_port) {
    int result = SYS_CONNECT;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(dest_ip), "d"((int)dest_port)
                       : "memory", "cc");
    return result;
}

int sys_exec_trusted_env(const char* path, char** argv, int argc,
                          char** envp) {
    int result = SYS_EXEC_TRUSTED_ENV;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc), "S"(envp)
                       : "memory", "cc");
    return result;
}

int sys_exec_trusted(const char* path, char** argv, int argc) {
    return sys_exec_trusted_env(path, argv, argc, environ);
}

int sys_dns_resolve(const char* hostname, unsigned int* out_ip) {
    int result = SYS_DNS_RESOLVE;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(hostname), "c"(out_ip)
                       : "memory", "cc");
    return result;
}

int sys_tftp_fetch(unsigned int server_ip, const char* remote_filename,
                    const char* local_filename) {
    int result = SYS_TFTP_FETCH;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(server_ip), "c"(remote_filename),
                         "d"(local_filename)
                       : "memory", "cc");
    return result;
}

int sys_socket_udp(void) {
    int result = SYS_SOCKET_UDP;
    __asm__ volatile ("int $0x80" : "+a"(result) : : "memory", "cc");
    return result;
}

int sys_sendto(int handle, const nova_udp_addr_t* addr, const void* buf,
               unsigned int len) {
    int result = SYS_SENDTO;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(addr), "d"(buf), "S"(len)
                       : "memory", "cc");
    return result;
}

int sys_recvfrom(int handle, void* buf, unsigned int max_len,
                  nova_udp_addr_t* out_addr) {
    int result = SYS_RECVFROM;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(handle), "c"(buf), "d"(max_len), "S"(out_addr)
                       : "memory", "cc");
    return result;
}

/* Phase 81: framebuffer API wrappers. One-argument int 0x80 calls whose
 * result (0, or a negative errno) is returned as-is. "memory" clobber
 * because the kernel reads and/or writes the structs these point at. */
static int fb_call1(int number, unsigned int arg) {
    int result = number;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(arg)
                       : "memory", "cc");
    return result;
}

int sys_fb_info(nova_fb_info_t* out) {
    return fb_call1(SYS_FB_INFO, (unsigned int)out);
}

int sys_fb_acquire(unsigned int backend) {
    return fb_call1(SYS_FB_ACQUIRE, backend);
}

int sys_fb_release(void) {
    return fb_call1(SYS_FB_RELEASE, 0);
}

int sys_fb_create(nova_fb_create_t* req) {
    return fb_call1(SYS_FB_CREATE, (unsigned int)req);
}

int sys_fb_destroy(unsigned int handle) {
    return fb_call1(SYS_FB_DESTROY, handle);
}

int sys_fb_present(const nova_fb_present_t* req) {
    return fb_call1(SYS_FB_PRESENT, (unsigned int)req);
}

int sys_fb_readback(const nova_fb_readback_t* req) {
    return fb_call1(SYS_FB_READBACK, (unsigned int)req);
}

/* Phase 83: shared-memory wrappers. Same one-argument int 0x80 shape as
 * the framebuffer calls above: the kernel reads and/or writes the struct
 * the argument points at, hence the "memory" clobber. */
int sys_shm_create(nova_shm_create_t* req) {
    return fb_call1(NOVA_SYS_SHM_CREATE, (unsigned int)req);
}

int sys_shm_grant(const nova_shm_grant_t* req) {
    return fb_call1(NOVA_SYS_SHM_GRANT, (unsigned int)req);
}

int sys_shm_map(nova_shm_map_t* req) {
    return fb_call1(NOVA_SYS_SHM_MAP, (unsigned int)req);
}

int sys_shm_unmap(unsigned int addr) {
    return fb_call1(NOVA_SYS_SHM_UNMAP, addr);
}

int sys_shm_destroy(unsigned int handle) {
    return fb_call1(NOVA_SYS_SHM_DESTROY, handle);
}

int sys_shm_info(nova_shm_info_t* req) {
    return fb_call1(NOVA_SYS_SHM_INFO, (unsigned int)req);
}
