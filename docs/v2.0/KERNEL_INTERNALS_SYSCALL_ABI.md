# NovaOS 2.0 Kernel Internals & Syscall ABI Manual

## 1. 64-Bit Fast System Call Architecture

NovaOS 2.0 implements hardware-accelerated 64-bit system calls utilizing the AMD64/Intel 64 `syscall` and `sysret` instructions, reducing boundary crossing latency to approximately 70 CPU clock cycles.

```
                      Userland Process (Ring 3)
                                 |
           Populate RAX (Syscall #) & Arguments (RDI, RSI, RDX, R10, R8, R9)
                                 |
                         Execute SYSCALL
                                 |
  +------------------------------v------------------------------+
  | Hardware Transition:                                         |
  | 1. RIP <- IA32_LSTAR (0xC0000082)                            |
  | 2. RCX <- Saved Userland RIP                                 |
  | 3. RFLAGS <- RFLAGS & ~IA32_FMASK (0xC0000084)               |
  | 4. R11 <- Saved Userland RFLAGS                              |
  | 5. CS / SS <- Loaded from IA32_STAR (Ring 0 Selectors)       |
  +------------------------------+------------------------------+
                                 |
                       Kernel Dispatcher (Ring 0)
                                 |
                     Execute SWAPGS Instruction
             (Loads per-CPU TSS & Kernel Stack pointer)
                                 |
              Validate Canonical Bounds & User Pointers
                                 |
               Evaluate Process Capability Bitmask & MAC
                                 |
              Dispatch to Subsystem Function Handler
                                 |
               RAX <- Return Value / Negative Errno
                                 |
                     Execute SWAPGS Instruction
                                 |
                         Execute SYSRET
                                 |
                      Userland Process Resumes
```

### Hardware Configuration Registers (MSRs)
At boot time, CPU initialization (`arch/x86/cpu.rs`) configures the following Model Specific Registers on each active core:
- **`IA32_STAR` (`0xC0000081`)**: Specifies target Code and Stack Segment selectors for kernel and user mode transitions.
- **`IA32_LSTAR` (`0xC0000082`)**: Programmed with the virtual entry address of `kernel_fast_syscall_entry`.
- **`IA32_FMASK` (`0xC0000084`)**: Mask set to `0x257FD5` to atomically clear `IF` (Interrupts), `DF` (Direction), `TF` (Trap), `IOPL` (I/O Privilege), and `AC` (Alignment Check) upon entry.
- **`IA32_KERNEL_GS_BASE` (`0xC0000102`)**: Stores the base address of the per-core `cpu_local_storage_t` structure.

---

## 2. Calling Convention & Register Allocation

NovaOS 2.0 strictly follows the System V AMD64 ABI kernel convention:

| Register | Role on Entry | Role on Exit | Preservation Status |
| :--- | :--- | :--- | :--- |
| **`RAX`** | System Call Number | Return value ($ \ge 0$) or `-errno` ($< 0$) | Scratch (Clobbered) |
| **`RDI`** | Argument 1 | Unchanged / Scratch | Scratch |
| **`RSI`** | Argument 2 | Unchanged / Scratch | Scratch |
| **`RDX`** | Argument 3 | Unchanged / Scratch | Scratch |
| **`R10`** | Argument 4 *(Substitutes RCX)* | Unchanged / Scratch | Scratch |
| **`R8`** | Argument 5 | Unchanged / Scratch | Scratch |
| **`R9`** | Argument 6 | Unchanged / Scratch | Scratch |
| **`RCX`** | Userland Return RIP *(Hardware)* | Scratch | Hardware Clobbered |
| **`R11`** | Userland Return RFLAGS *(Hardware)* | Scratch | Hardware Clobbered |
| **`RBX`, `RBP`, `R12`–`R15`, `RSP`** | Preserved | Preserved | Callee-Saved (Preserved) |

> [!IMPORTANT]
> The `RCX` register is overwritten by the CPU with the return instruction pointer during `syscall`. In accordance with standard 64-bit conventions, Argument 4 is passed in **`R10`**.

---

## 3. Comprehensive System Call Table

### 3.1 Process & Task Management
| Syscall # | Name | Signature | Capability Required | Description |
| :--- | :--- | :--- | :--- | :--- |
| **1** | `sys_exit` | `void sys_exit(int status)` | None | Terminates current process and wakes parent |
| **2** | `sys_fork` | `pid_t sys_fork(void)` | `CAN_EXEC` | Creates child process via Copy-on-Write |
| **7** | `sys_waitpid` | `pid_t sys_waitpid(pid_t pid, int *status, int options)` | None | Suspends caller until child state changes |
| **11** | `sys_exec` | `int sys_exec(const char *path, char *const argv[], char *const envp[])` | `CAN_EXEC` | Overlays current address space with ELF64 binary |
| **20** | `sys_getpid` | `pid_t sys_getpid(void)` | None | Returns process ID of caller |
| **37** | `sys_kill` | `int sys_kill(pid_t pid, int sig)` | None (Subject to PID owner) | Delivers asynchronous signal to process |
| **120** | `sys_clone` | `int sys_clone(unsigned long flags, void *child_stack)` | `CAN_EXEC` | Creates thread or sandboxed namespace process |
| **158** | `sys_yield` | `void sys_yield(void)` | None | Voluntarily relinquishes current CPU timeslice |
| **272** | `sys_unshare` | `int sys_unshare(unsigned long flags)` | Root / `CAN_HOST_ACCESS` | Detaches process execution context from shared namespaces |
| **308** | `sys_setns` | `int sys_setns(int fd, int nstype)` | Root / `CAN_HOST_ACCESS` | Attaches calling task to specified namespace |

### 3.2 File & VFS Subsystem
| Syscall # | Name | Signature | Capability Required | Description |
| :--- | :--- | :--- | :--- | :--- |
| **3** | `sys_read` | `ssize_t sys_read(int fd, void *buf, size_t count)` | `CAN_READ_FILES` | Reads bytes from open file descriptor |
| **4** | `sys_write` | `ssize_t sys_write(int fd, const void *buf, size_t count)` | `CAN_WRITE_FILES` | Writes bytes to open file descriptor |
| **5** | `sys_open` | `int sys_open(const char *path, int flags, mode_t mode)` | Subject to `flags` | Opens or creates file in concurrent VFS |
| **6** | `sys_close` | `int sys_close(int fd)` | None | Closes file descriptor and releases lock |
| **19** | `sys_lseek` | `off_t sys_lseek(int fd, off_t offset, int whence)` | None | Repositions read/write file offset |
| **106** | `sys_stat` | `int sys_stat(const char *path, struct stat *buf)` | `CAN_READ_FILES` | Retrieves metadata by filesystem path |
| **108** | `sys_fstat` | `int sys_fstat(int fd, struct stat *buf)` | None | Retrieves metadata by open descriptor |
| **141** | `sys_getdents` | `int sys_getdents(int fd, struct dirent *dirp, unsigned int count)` | `CAN_READ_FILES` | Reads directory entries from VFS dentry cache |
| **291** | `sys_inotify_init` | `int sys_inotify_init(void)` | `CAN_READ_FILES` | Allocates asynchronous inotify queue descriptor |
| **292** | `sys_inotify_add_watch`| `int sys_inotify_add_watch(int fd, const char *path, uint32_t mask)` | `CAN_READ_FILES` | Registers directory or file event watcher |

### 3.3 Memory Management
| Syscall # | Name | Signature | Capability Required | Description |
| :--- | :--- | :--- | :--- | :--- |
| **45** | `sys_brk` | `void *sys_brk(void *addr)` | None | Adjusts process heap break pointer |
| **90** | `sys_mmap` | `void *sys_mmap(void *addr, size_t length, int prot, int flags, int fd, off_t offset)` | None | Maps physical memory, files, or anonymous pages |
| **91** | `sys_munmap` | `int sys_munmap(void *addr, size_t length)` | None | Unmaps pages and returns frames to PMM |
| **125** | `sys_mprotect` | `int sys_mprotect(void *addr, size_t len, int prot)` | None | Updates PML4 protection flags (`PROT_NONE/READ/WRITE/EXEC`) |

### 3.4 Asynchronous I/O Ring Engine (`nova_uring`)
| Syscall # | Name | Signature | Capability Required | Description |
| :--- | :--- | :--- | :--- | :--- |
| **425** | `sys_uring_setup` | `int sys_uring_setup(uint32_t entries, struct nova_uring_params *p)` | None | Initializes SQ and CQ lockless ring buffers |
| **426** | `sys_uring_enter` | `int sys_uring_enter(int fd, uint32_t to_submit, uint32_t min_complete, uint32_t flags)` | None | Submits ring requests and awaits completions |
| **427** | `sys_uring_register` | `int sys_uring_register(int fd, unsigned int opcode, void *arg, unsigned int nr_args)` | None | Registers pre-mapped DMA buffers and file tables |

### 3.5 Inter-Process Communication & Shared Memory
| Syscall # | Name | Signature | Capability Required | Description |
| :--- | :--- | :--- | :--- | :--- |
| **300** | `sys_shm_create` | `int sys_shm_create(size_t size, uint32_t flags)` | None | Allocates physical frame group for shared memory |
| **301** | `sys_shm_map` | `void *sys_shm_map(int handle, int prot)` | None | Maps shared memory object into caller PML4 |
| **302** | `sys_shm_unmap` | `int sys_shm_unmap(int handle)` | None | Detaches shared memory mapping |
| **303** | `sys_shm_grant` | `int sys_shm_grant(int handle, pid_t target_pid, int rights)` | None | Grants read/write rights to another process |
| **304** | `sys_shm_revoke` | `int sys_shm_revoke(int handle, pid_t target_pid)` | None | Revokes previously granted access rights |
| **305** | `sys_shm_destroy` | `int sys_shm_destroy(int handle)` | None | Releases shared object when refcount reaches zero |

### 3.6 Hardware Framebuffer & Graphics Engine
| Syscall # | Name | Signature | Capability Required | Description |
| :--- | :--- | :--- | :--- | :--- |
| **310** | `sys_fb_info` | `int sys_fb_info(struct fb_info *info)` | None | Returns resolution, pitch, bpp, and backend type |
| **311** | `sys_fb_acquire` | `int sys_fb_acquire(uint32_t backend)` | `CAN_HOST_ACCESS` | Acquires exclusive ownership of primary display head |
| **312** | `sys_fb_release` | `int sys_fb_release(void)` | None | Releases display head back to text console |
| **313** | `sys_fb_surface_create`| `int sys_fb_surface_create(uint32_t w, uint32_t h, uint32_t fmt)` | None | Allocates offscreen 32-bit ARGB surface |
| **316** | `sys_fb_present` | `int sys_fb_present(int surface_handle)` | None | Presents surface contents to hardware display |
| **317** | `sys_fb_damage` | `int sys_fb_damage(int surface_handle, const struct fb_damage_rect *rect)` | None | Submits partial damage rectangle for zero-copy flush |

---

## 4. Kernel Wire Formats & Data Structures

### 4.1 `nova_uring` Structures
```c
struct nova_uring_sqe {
    uint8_t  opcode;      /* URING_OP_READ, URING_OP_WRITE, URING_OP_POLL, etc. */
    uint8_t  flags;       /* Ring submission flags */
    uint16_t ioprio;      /* I/O priority class */
    int32_t  fd;          /* File descriptor or socket */
    uint64_t addr;        /* Userland buffer address or registered buffer offset */
    uint32_t len;         /* Buffer length in bytes */
    uint64_t offset;      /* File offset */
    uint64_t user_data;   /* Arbitrary user identifier reflected in completion */
};

struct nova_uring_cqe {
    uint64_t user_data;   /* Copied directly from the matching SQE */
    int32_t  res;         /* Resulting byte count or negative errno */
    uint32_t flags;       /* Completion flags */
};

struct nova_uring_params {
    uint32_t sq_entries;  /* Count of submission queue entries */
    uint32_t cq_entries;  /* Count of completion queue entries */
    uint32_t flags;       /* SETUP_SQPOLL, SETUP_IOPOLL */
    uint32_t sq_off[8];   /* Array offsets for ring synchronization */
    uint32_t cq_off[8];
};
```

### 4.2 Framebuffer Damage Structure
```c
struct fb_damage_rect {
    uint32_t x;           /* Top-left X coordinate */
    uint32_t y;           /* Top-left Y coordinate */
    uint32_t width;       /* Rectangle width */
    uint32_t height;      /* Rectangle height */
};
```

---

## 5. POSIX Standard Error Codes

Syscalls return negative values in `RAX` on failure. The standard library (`novalibc`) stores `-RAX` into `errno` and returns `-1`.

| Symbolic Macro | Value | Description |
| :--- | :--- | :--- |
| **`EPERM`** | 1 | Operation not permitted / Capability denied |
| **`ENOENT`** | 2 | No such file or directory |
| **`ESRCH`** | 3 | No such process |
| **`EINTR`** | 4 | Interrupted system call |
| **`EIO`** | 5 | Hardware I/O error |
| **`EBADF`** | 9 | Bad file descriptor |
| **`EAGAIN`** | 11 | Resource temporarily unavailable / Try again |
| **`ENOMEM`** | 12 | Out of memory |
| **`EACCES`** | 13 | Permission denied |
| **`EFAULT`** | 14 | Bad address / User pointer bounds violation |
| **`EBUSY`** | 16 | Device or resource busy |
| **`EEXIST`** | 17 | File or resource already exists |
| **`EINVAL`** | 22 | Invalid argument |
| **`ENOSPC`** | 28 | No space left on device |
| **`EPIPE`** | 32 | Broken pipe |
| **`ECONNREFUSED`** | 111 | Connection refused |
| **`ETIMEDOUT`** | 110 | Connection timed out |
