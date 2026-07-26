#ifndef TYPES_H
#define TYPES_H

// Integer types
typedef unsigned char  uint8_t;
typedef unsigned short uint16_t;
typedef unsigned int   uint32_t;
/* `unsigned long` is only 32 bits under -m32 GCC (i386 ABI) - `long
 * long` is the type that's actually 64 bits regardless of -m32/-m64.
 * This bit us for real: the Multiboot memory-map entries GRUB hands
 * us have genuine 64-bit addr/len fields (see multiboot.h), and with
 * the old `unsigned long` typedef every field after the first in that
 * packed struct was silently misaligned. */
typedef unsigned long long uint64_t;

typedef signed char    int8_t;
typedef signed short   int16_t;
typedef signed int     int32_t;
typedef signed long long int64_t;

// Boolean
typedef enum { false = 0, true = 1 } bool;

// Size type
typedef unsigned int size_t;

// NULL pointer
#define NULL ((void*)0)

// Maximum values
#define UINT32_MAX 0xFFFFFFFF
#define INT32_MAX  0x7FFFFFFF

#endif