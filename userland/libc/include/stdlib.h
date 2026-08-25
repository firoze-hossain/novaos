#ifndef NOVA_STDLIB_H
#define NOVA_STDLIB_H

#include <stddef.h>

void* malloc(size_t size);
void free(void* ptr);
int atoi(const char* s);
void exit(int code) __attribute__((noreturn));

#endif
