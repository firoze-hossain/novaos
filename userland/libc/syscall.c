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
