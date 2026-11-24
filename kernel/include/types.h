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
#if !defined(__bool_true_false_are_defined) && !defined(bool)
#define bool _Bool
#define true 1
#define false 0
#define __bool_true_false_are_defined 1
#endif

// Size type
#ifndef _SIZE_T_DEFINED
typedef unsigned int size_t;
#define _SIZE_T_DEFINED
#endif
typedef uint32_t     uintptr_t;
typedef int32_t      intptr_t;

// NULL pointer
#ifndef NULL
#define NULL ((void*)0)
#endif

// Maximum values
#ifndef UINT32_MAX
#define UINT32_MAX 0xFFFFFFFF
#endif
#ifndef INT32_MAX
#define INT32_MAX  0x7FFFFFFF
#endif

#endif