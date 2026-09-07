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

int sys_exec(const char* path, char** argv, int argc) {
    int result = SYS_EXEC;
    __asm__ volatile ("int $0x80"
                       : "+a"(result)
                       : "b"(path), "c"(argv), "d"(argc)
                       : "memory", "cc");
    return result;
}

int sys_wait(int pid) {
    int result = SYS_WAIT;
    __asm__ volatile ("int $0x80" : "+a"(result) : "b"(pid) : "memory", "cc");
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
