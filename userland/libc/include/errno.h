#ifndef NOVA_ERRNO_H
#define NOVA_ERRNO_H

/* errno - one plain global, since NovaOS processes are single-threaded
 * (no threads exist, so a thread-local buys nothing). It is defined
 * exactly once, in stdlib.c - the one libc object every userland
 * program (C and Rust alike) already links - and set by the libc
 * functions that can fail (malloc, fopen, setenv, ...), never cleared
 * by successful calls, exactly as the C standard specifies.
 *
 * The numeric values are the standard POSIX/Linux ones, not
 * NovaOS-specific numbers, so code ported from elsewhere that prints
 * or compares raw errno values behaves as its author expected.
 *
 * Honest limit worth knowing: this kernel's file syscalls report every
 * failure as the single value -1 (see kernel/arch/x86/cpu/syscall.c),
 * so libc infers the most specific errno the evidence supports rather
 * than reading one back. In practice that means: a missing file is
 * reliably ENOENT, but "the kernel refused" is reported as EACCES even
 * when the true cause was a full disk or a full open-file table. See
 * userland/libc/stdio.c's fopen() comment for the exact inference. */
extern int errno;

#define EPERM         1   /* operation not permitted */
#define ENOENT        2   /* no such file or directory */
#define ESRCH         3   /* no such process */
#define EINTR         4   /* interrupted */
#define EIO           5   /* I/O error */
#define ENXIO         6   /* no such device or address */
#define E2BIG         7   /* argument list too long */
#define ENOEXEC       8   /* exec format error */
#define EBADF         9   /* bad file descriptor / stream */
#define ECHILD       10   /* no child processes */
#define EAGAIN       11   /* try again */
#define ENOMEM       12   /* out of memory */
#define EACCES       13   /* permission denied */
#define EFAULT       14   /* bad address */
#define EBUSY        16   /* resource busy */
#define EEXIST       17   /* file exists */
#define ENODEV       19   /* no such device */
#define ENOTDIR      20   /* not a directory */
#define EISDIR       21   /* is a directory */
#define EINVAL       22   /* invalid argument */
#define ENFILE       23   /* too many open files in the system */
#define EMFILE       24   /* too many open files in this process */
#define EFBIG        27   /* file too large */
#define ENOSPC       28   /* no space left on device */
#define ESPIPE       29   /* illegal seek (e.g. on stdin/stdout) */
#define EROFS        30   /* read-only filesystem */
#define EPIPE        32   /* broken pipe */
#define EDOM         33   /* math argument out of domain */
#define ERANGE       34   /* result out of range */
#define ENAMETOOLONG 36   /* file name too long */
#define ENOSYS       38   /* function not implemented */
#define ENOTEMPTY    39   /* directory not empty */
#define EOVERFLOW    75   /* value too large for the data type */

#endif
