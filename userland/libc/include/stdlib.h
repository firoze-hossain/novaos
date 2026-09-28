#ifndef NOVA_STDLIB_H
#define NOVA_STDLIB_H

#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

/* Allocator. Every returned pointer is 16-byte aligned. malloc(0)
 * returns a real, freeable minimal block. Failure returns NULL with
 * errno == ENOMEM (see errno.h). realloc(NULL, n) is malloc(n);
 * realloc(p, 0) frees p and returns NULL; if realloc fails the
 * original block is left valid and untouched. free() of a pointer the
 * allocator did not hand out, or of an already-freed one, is refused
 * with a console message instead of corrupting the heap. */
void* malloc(size_t size);
void free(void* ptr);
void* realloc(void* ptr, size_t size);
void* calloc(size_t nmemb, size_t size);

int atoi(const char* s);

/* Registers `fn` to run, last-registered first, when the program
 * calls exit() or returns from main(). Up to 32 handlers; returns 0,
 * or -1 if the table is full. stdio uses one of these slots to flush
 * open files. */
int atexit(void (*fn)(void));
void exit(int code) __attribute__((noreturn));

/* The process environment: a NULL-terminated array of "NAME=value"
 * strings, filled in by the kernel at exec time from whatever the
 * parent passed (see sys_exec() in novasys.h - children inherit their
 * parent's environment) and published here by crt0. A program started
 * by the kernel itself or by the shell, rather than by another
 * program, starts with an EMPTY environment: nothing sets defaults.
 *
 * getenv() returns a pointer to the value inside the environment (do
 * not modify or free it), or NULL if the name is unset or invalid.
 * setenv() copies name and value; with overwrite == 0 an existing
 * variable is left alone. unsetenv() succeeds whether or not the
 * variable existed. putenv() makes the caller's own string part of
 * the environment WITHOUT copying it (per POSIX) - it must outlive its
 * use; "NAME" with no '=' removes NAME. clearenv() empties the
 * environment.
 *
 * All of the mutators return 0 on success or -1 with errno set:
 * EINVAL for a NULL/empty name, a name containing '=', or a NULL
 * value; ENOMEM if out of memory OR if the change would push the
 * environment past NOVA_ENV_MAX_VARS (32) variables /
 * NOVA_ENV_MAX_BYTES (4096) bytes - the same limits the kernel applies
 * when handing an environment to a child, so a process can never build
 * one its own children could not inherit. */
extern char** environ;
char* getenv(const char* name);
int setenv(const char* name, const char* value, int overwrite);
int unsetenv(const char* name);
int putenv(char* string);
int clearenv(void);

#endif
