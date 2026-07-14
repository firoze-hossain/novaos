#ifndef STRING_H
#define STRING_H

#include "../include/types.h"

void* memcpy(void* dest, const void* src, size_t n);
void* memset(void* s, int c, size_t n);
size_t strlen(const char* s);
char* strcpy(char* dest, const char* src);
int strcmp(const char* s1, const char* s2);
char* strchr(const char* s, int c);
void itoa(int num, char* str, int base);
int isdigit(int c);
int isspace(int c);

#endif