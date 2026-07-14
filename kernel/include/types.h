#ifndef TYPES_H
#define TYPES_H

// Integer types
typedef unsigned char  uint8_t;
typedef unsigned short uint16_t;
typedef unsigned int   uint32_t;
typedef unsigned long  uint64_t;

typedef signed char    int8_t;
typedef signed short   int16_t;
typedef signed int     int32_t;
typedef signed long    int64_t;

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