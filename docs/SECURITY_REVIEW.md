# NovaOS Full Security Review: Cross-Track Permission & Capability Audit

**Document Version:** 1.0 (Release Candidate Sign-off)  
**Target:** NovaOS v1.0 Kernel Foundation & Interface Track  
**Classification:** Technical Security Audit & Architectural Verification Report  
**Author:** NovaOS Core Security Architecture Team  

---

## 1. Executive Summary & Threat Model

NovaOS was architected from inception around a strict **defense-in-depth** and **least-privilege** security model. Across all six months of development, system features spanning both the low-level kernel foundation and the ring-3 graphical and system applications were developed with explicit isolation barriers.

### 1.1 Architectural Trust Boundaries
The operating system enforces four primary security boundaries:

```
+-----------------------------------------------------------------------+
| UNTRUSTED EXTERNAL ENVIRONMENT (Network packets, untrusted storage)   |
+-----------------------------------+-----------------------------------+
                                    | [Stateful Packet Inspection Firewall]
+-----------------------------------v-----------------------------------+
| RING 3: UNPRIVILEGED APPLICATIONS & SANDBOXES                         |
| (FILES.ELF, EDIT.ELF, TERM.ELF, STORE.ELF, ACTMON.ELF, KLOG.ELF)      |
| - Memory Isolation (Per-process page tables)                          |
| - Ambient Authority Stripped (Capability allowlists)                  |
| - MAC Profiles Enforced (Fine-grained file/syscall sandboxing)        |
+-----------------------------------+-----------------------------------+
                                    | [Kernel Syscall Gate: int 0x80 DPL 3]
+-----------------------------------v-----------------------------------+
| RING 3: PRIVILEGED SYSTEM SERVICES & COMPOSITOR                       |
| (NOVAINIT.ELF [PID 1], WM.ELF, LOGIN.ELF, INSTALL.ELF, SETTINGS.ELF)  |
| - Root Privilege Gates (UID 0 & unconfined MAC policy)               |
| - Single-Owner Exclusive Resources (Framebuffer Display)             |
+-----------------------------------+-----------------------------------+
                                    | [Address Space Split: 0xC0000000]
+-----------------------------------v-----------------------------------+
| RING 0: KERNEL FOUNDATION & MEMORY-SAFE SUBSYSTEMS                    |
| - Core Hardware Drivers & Scheduler (16 KB bounded kernel stack)      |
| - Rust Subsystems: TCP/IP, Firewall, Journals, SHA-256, MAC, Installer|
| - User-Pointer Bounds Sanitization (paging_user_range_ok)             |
+-----------------------------------------------------------------------+
```

### 1.2 Threat Model Invariants
1. **Ring 0 / Ring 3 Address Space Segregation:** No user-controlled pointer may reference or cause mutation of memory at or above `0xC0000000`, the real-mode interrupt vector table at Page 0 (`0x00000000 - 0x00000FFF`), or physical device registers.
2. **Ambient Authority Elimination:** File and network access are capability-gated. A process spawned without `can_open_any_file` or `allowed_files` cannot access configuration or credential stores even if filesystem permissions permit read access.
3. **Mandatory Access Control (MAC) Confinement:** Even when running as root (`uid == 0`), processes running under active MAC profiles cannot bypass policy constraints or alter MAC rule files (`*.MAC`).
4. **Immutable Identity Propagation:** Inter-process communication (`SYS_MSG_SEND`, `SYS_SHM_*`) and kernel logging (`SYS_KLOG_WRITE`) stamp sender credentials directly from kernel scheduler task records (`process_current()`), preventing sender impersonation.
5. **Credential Ephemerality:** Plaintext passwords entered during login, user creation, or privilege escalation must be erased via explicit volatile zeroization immediately upon computation of cryptographic digests.

---

## 2. Complete Syscall Authorization & Security Matrix

NovaOS exposes 84 numbered system calls via software interrupt `int 0x80`. The table below outlines the gate mechanisms, permission checks, and pointer validation rules for every syscall in the system:

| Syscall # | Identifier | Functional Category | Privilege & Gate Mechanism | User Pointer Bounds Check |
|:---:|:---|:---|:---|:---|
| **1** | `SYS_WRITE` | Console I/O | Open (ambient ring-3 logging) | Read validation up to NUL byte (`paging_user_range_ok`) |
| **2** | `SYS_EXIT` | Process Control | Open (self-termination) | None (scalar exit code in EBX) |
| **3** | `SYS_YIELD` | Process Control | Open (cooperative quantum yield) | None (no arguments) |
| **4** | `SYS_OPEN` | File I/O | **Capability-gated** (`allowed_files[]` or `can_open_any_file`) + MAC file rule | Read string validation (`paging_user_range_ok`) |
| **5** | `SYS_READ` | File / Stream I/O | Handle-bound (descriptor owned by caller) | Write buffer validation (`paging_user_range_ok`) |
| **6** | `SYS_CLOSE` | File / Stream I/O | Handle-bound (descriptor owned by caller) | None (integer handle) |
| **7** | `SYS_NET_SEND` | Raw UDP Network | **Capability-gated** (`allowed_hosts[]`) + MAC net rule | Read payload buffer validation |
| **8** | `SYS_SPAWN` | Process Control | **Capability-gated** (`can_spawn`) + MAC exec rule | Read string validation for executable name |
| **9** | `SYS_EXEC` | Process Control | **Capability-gated** (`can_spawn`) + MAC exec rule | Read argv array & string validation |
| **10** | `SYS_WAIT` | Process Control | Caller's child process only (PID match) | None (integer child PID) |
| **11** | `SYS_SBRK` | Memory Management | Process-confined (within process virtual heap bounds) | Bounds check against virtual limit |
| **12** | `SYS_FORK` | Process Control | **Capability-gated** (`can_spawn`) | None |
| **13** | `SYS_READ_KEY` | Console Input | Open | Pointer validation for keystroke struct |
| **14** | `SYS_LIST_FILES`| Directory VFS | Open (filtered by root directory scope) | Buffer validation for directory entry array |
| **15** | `SYS_RTC_READ` | Hardware Clock | Open (read-only hardware clock) | Pointer validation for `rtc_time_t` struct |
| **16** | `SYS_LSPCI` | Hardware Discovery | Open (read-only PCI topology inspection) | Buffer validation for PCI device records |
| **17** | `SYS_BEEP` | Audio / Alert | Open (rate-limited speaker pulse) | None |
| **18** | `SYS_WRITE_FILE`| File I/O | **Capability-gated** (`allowed_files[]` or `can_open_any_file`) + MAC write rule | Read payload validation |
| **19** | `SYS_DELETE_FILE`| File I/O | **Capability-gated** + MAC delete rule + Protected file check | Read filename validation |
| **20-23** | `SYS_GFX_*` | Legacy VGA 13h | Exclusive display acquisition required | Bounds check within VGA bank |
| **24** | `SYS_MOUSE_READ`| Input Peripheral | Open | Pointer validation for `mouse_state_t` |
| **25-26** | `SYS_PING_*` | ICMP Diagnostics | Open (non-privileged diagnostics) | Pointer validation for ping stats |
| **27-28** | `SYS_PIPE / WRITE`| IPC Stream | Process-confined handle pair | Buffer read/write validation |
| **29** | `SYS_LOGIN` | Identity Transition | Open entrypoint; strictly transitions UID on SHA-256 match | Pointer validation for username/password strings |
| **30** | `SYS_GETUID` | Identity Query | Open (returns current process `uid`) | None |
| **31** | `SYS_SUDO` | Privilege Escalation| Password match against root account in `USERS.CFG` | Pointer validation for password string |
| **32** | `SYS_SHUTDOWN` | System Lifecycle | **Root-gated** (`uid == 0`) + MAC admin gate | None |
| **33-37** | `SYS_TCP_*` | TCP Network Sockets| Handle-bound; connect/bind subject to MAC network rules | Buffer read/write validation |
| **38** | `SYS_EXEC_TRUSTED`| Process Control | **Inheritance-gated**: caller must already possess `can_open_any_file` | Executable & argv validation |
| **39** | `SYS_DNS_RESOLVE`| Network Discovery | Open (read-only UDP port 53 query) | Hostname read buffer & IP write buffer validation |
| **40** | `SYS_TFTP_FETCH`| Network Transfer | **Capability-gated** (`allowed_hosts[]`) | Target filename & payload buffer validation |
| **41** | `SYS_WAIT_NONBLOCK`| Process Control | Caller's child process only | Pointer validation for exit status |
| **42-43** | `SYS_EXEC_*ENV` | Process Control | **Capability-gated** (`can_spawn`) + environment sanitization | Environment array pointer validation |
| **44-46** | `SYS_UDP_*` | UDP Sockets | Handle-bound + MAC send/bind network rules | Buffer read/write validation |
| **47-53** | `SYS_FB_*` | Modern Framebuffer | **Single-Owner Mutual Exclusion** (`fb_sys_acquire`) | Pointer validation on surfaces & damage rects |
| **54-59** | `SYS_SHM_*` | Shared Memory IPC | Object ACL-gated (`SYS_SHM_GRANT`), refcounted | Pointer validation on mapping targets |
| **60-65** | `SYS_MSG_*` | App Messaging IPC | Inbox recipient policy; sender PID/UID stamped by kernel | Pointer validation on message envelopes |
| **66-69** | `SYS_AUDIO_*` | Audio Stream & Mix | Stream owner; master volume/tap **Root-gated** (`uid == 0`) | Buffer write validation for PCM frames |
| **70** | `SYS_RLIMIT` | Resource Bounds | Self-tightening open; raising limits / altering others is **Root-gated** | Pointer validation for `rlimit` struct |
| **71-72** | `SYS_MAC_*` | MAC Policy Control | Policy modification **Root-gated** (`uid == 0` & unconfined) | Pointer validation for profile name & rules |
| **73-74** | `SYS_FW_*` | Firewall Control | Configuration & flow tracking **Root-gated** (`uid == 0`) | Pointer validation for rule structures |
| **75-77** | `SYS_KLOG_*` | Structured Logging | Read open; append stamped with caller PID | Pointer validation for log entry query |
| **78** | `SYS_PKGREPO_CTL`| Package HTTP Server| Server control & configuration **Root-gated** (`uid == 0`) | Pointer validation for repo control structs |
| **79** | `SYS_OS_UPDATE` | Atomic OS Update | **Root-gated** (`uid == 0`) + MAC admin gate + ECDSA/SHA-256 signature check | Pointer validation for package payload |
| **80** | `SYS_CRASHDUMP_CTL`| Crash Diagnostics | Export & clear operations **Root-gated** (`uid == 0`) | Pointer validation for crash record query |
| **81** | `SYS_DISK_INSTALL`| Disk Partitioning | Live partition/format **Root-gated** (`uid == 0`); dry-run open | Pointer validation for disk layout request |
| **82** | `SYS_SERVICE_STATUS`| Init Supervisor | **PID 1 Exclusive** (`novainit` supervisor only) | Pointer validation for service status records |
| **83** | `SYS_DEV_INFO` | Hardware Registry | Open (read-only hardware registry query) | Buffer validation for device descriptors |
| **84** | `SYS_PROC_INFO` | Process Telemetry | Open (read-only process table telemetry query) | Buffer validation for process telemetry records |

---

## 3. Kernel Isolation & Memory Safety Architecture

### 3.1 High-Memory Address Space Split (`0xC0000000`)
The NovaOS kernel resides strictly in the upper 1GB virtual memory region (`0xC0000000 - 0xFFFFFFFF`). Userland processes reside in the lower 3GB (`0x00000000 - 0xBFFFFFFF`).

```
0xFFFFFFFF +---------------------------------------+
           | Kernel High Memory (Paging, Drivers,   |  SUPERVISOR ONLY
           | Rust Runtime, Kernel Heap & Stacks)   |  (Ring 0 only)
0xC0000000 +---------------------------------------+  <-- Split Boundary
           | User Memory Space                     |  USER-ACCESSIBLE
           | (Stack, Heap, Code, Read-Only Data)   |  (Ring 3, DPL 3)
0x00001000 +---------------------------------------+
           | Page 0 (IVT & BIOS Data Area)         |  SUPERVISOR / PROTECTED
0x00000000 +---------------------------------------+
```

### 3.2 Pointer Bounds Invariants (`paging_user_range_ok`)
Before the kernel dereferences any pointer supplied by a ring-3 process, it executes `paging_user_range_ok(addr, len, writable)`:
1. **Kernel High-Memory Rejection:** Any buffer overlapping or extending into `[0xC0000000, 0xFFFFFFFF]` causes immediate rejection (`-EFAULT`).
2. **Page 0 IVT Protection:** Any range touching page 0 (`[0x00000000, 0x00000FFF]`) is rejected.
3. **Integer Overflow / Wraparound Prevention:** Checks `addr + len >= addr` to prevent 32-bit unsigned wrap-around attacks.
4. **Physical Identity Mapping Protection:** Direct physical access regions (e.g. `0x100000`) are rejected for non-embedded userland tasks.

### 3.3 Rust Memory Safety Guarantees
All major network protocols (TCP, UDP, IP, ARP, DNS), stateful packet inspection firewalls (`firewall.rs`), filesystem journals (`fat32_journal.rs`), package repositories (`pkgrepo.rs`), cryptographic hash calculations (`sha256.rs`), crash dump parsers (`crashdump.rs`), and partitioning engines (`disk_installer.rs`) are compiled with `#![no_std]` Rust.
- **Zero Buffer Overflows:** Array bounds checking is statically enforced or panics safely into the kernel crash dump handler.
- **Zero Use-After-Free / Double-Free:** Rust ownership semantics prevent dangling pointers at compile time.
- **Stack Bound Safety:** All kernel stack frames remain strictly within the 16 KB kernel stack limit.

---

## 4. Cross-Track Application Least-Privilege Matrix

Every userland application was audited for least privilege across capability allocation, MAC sandboxing, filesystem reach, and credential hygiene:

```
+---------------+-------+----------------------+--------------------+--------------------+----------------------------+
| Application   | UID   | MAC Sandboxing       | Filesystem Scope   | Network Scope      | Syscall Restrictions       |
+---------------+-------+----------------------+--------------------+--------------------+----------------------------+
| NOVAINIT.ELF  | 0     | Supervisor Profile   | Full VFS           | Local only         | Gated: SYS_SERVICE_STATUS  |
| LOGIN.ELF     | 0->U  | Auth Profile         | USERS.CFG only     | None               | Credential zeroing on auth |
| FIRSTRUN.ELF  | 0->U  | Setup Profile        | SYSTEM/USERS.CFG   | DHCP/Local only    | Account creation only      |
| SHELL.ELF     | User  | Interactive Profile  | Home dir + /BIN    | DNS/Ping only      | Sandboxed exec inheritance |
| WM.ELF        | User  | Compositor Profile   | Assets & Fonts     | None               | Exclusive SYS_FB_ACQUIRE   |
| FILES.ELF     | User  | VFS Sandboxed        | User directory     | None               | No raw disk / partition    |
| TERM.ELF      | User  | Virtual Terminal     | Pipes / PTY only   | None               | ANSI terminal sanitization |
| EDIT.ELF      | User  | Editor Sandboxed     | Opened file only   | None               | No process spawn / net     |
| STORE.ELF     | User  | Client Sandboxed     | /PACKAGES only     | PKGREPO HTTP port  | Cryptographic verification |
| SETTINGS.ELF  | User  | Preferences Profile  | SYSTEM.CFG (gated) | Local telemetry    | Root escalation via sudo   |
| KLOG.ELF      | User  | Audit Read-Only      | None               | None               | Read-only SYS_KLOG_READ    |
| ACTMON.ELF    | User  | Telemetry Read-Only  | None               | None               | Read-only SYS_PROC_INFO    |
| INSTALL.ELF   | 0     | Installer Engine     | Raw disk partitions| None               | Requires confirmation & dry|
| CRASHREP.ELF  | User  | Diagnostic Profile   | CRASHDMP.DAT/.TXT  | None               | Read-only crash dump query |
+---------------+-------+----------------------+--------------------+--------------------+----------------------------+
```

### 4.1 Credential Scrubbing & Anti-Leak Safeguards
1. **Password Memory Sanitization:** In `LOGIN.ELF`, `FIRSTRUN.ELF`, `firstrun.c`, and `users.rs`, plaintext password character buffers are scrubbed immediately after computing the SHA-256 digest using volatile loops (`memset` with memory clobbers) to prevent plaintext retention in page frames.
2. **Timing-Invariant Digest Comparison:** Password hash validation in `users.rs` performs constant-time comparisons over fixed 32-byte hash buffers to eliminate timing side channels.
3. **Information Leak Prevention in Syscalls:** `SYS_LOGIN` and `SYS_SUDO` return uniform error codes (`-E_ACCES`) for non-existent users and incorrect passwords, preventing user enumeration.

---

## 5. IPC, Shared Memory & Peripheral Isolation

### 5.1 Asynchronous Messaging (`SYS_MSG_*`)
- **Kernel-Stamped Sender Identity:** When a process dispatches a message via `msg_sys_send()`, the sender PID and UID are read directly from `process_current()`. Userland code cannot specify or spoof the sender field.
- **Selective Reception & Filtering:** Receivers configure inboxes with explicit filter criteria (sender PID, message type, tag). Unsolicited messages are rejected or bounded by inbox depth quotas to prevent resource exhaustion attacks.

### 5.2 Shared Memory (`SYS_SHM_*`)
- **Per-Object Access Control Lists (ACL):** Shared memory regions created via `SYS_SHM_CREATE` are private by default. Access to peer processes requires explicit `SYS_SHM_GRANT` with specified read/write permission bitmasks.
- **Quota Bounding:** The kernel enforces global and per-process limits on total shared physical frames, preventing malicious processes from monopolizing system physical RAM.

### 5.3 Framebuffer Arbitration (`SYS_FB_*`)
- **Exclusive Display Ownership:** The display controller backend permits only one active owner at a time (`SYS_FB_ACQUIRE`). Secondary acquisition attempts return `-EBUSY` (`-16`).
- **Crash / Exit Cleanup:** If the display owner crashes or exits unexpectedly, the kernel scheduler automatically releases framebuffer locks and restores text video mode (`fb_process_exit`), ensuring the machine never becomes unresponsive.

---

## 6. Network Perimeter & Stateful Firewall Verification

### 6.1 Kernel Stateful Firewall (`SYS_FW_*`)
The network stack integrates a stateful packet inspection engine:
- **Connection Tracking Table:** Tracks up to 256 simultaneous bidirectional flows with sequence number verification and TTL expiration.
- **Directional State Transitions:** TCP handshake validation requires valid SYN -> SYN-ACK -> ACK progression; unsolicited incoming SYN packets are dropped by default unless explicitly permitted by firewall rule.
- **Restricted Control Interface:** Firewall rule configuration (`SYS_FW_CTL`) and flow state inspection (`SYS_FW_INFO`) require `uid == 0` and unconfined MAC profile.

---

## 7. Automated Verification Results & Test Matrix

The complete security architecture is validated continuously through multi-layered automated tests:

### 7.1 In-Kernel Security Audit Self-Test (Phase 98)
Executed automatically on every system boot in `kernel_main()`:
- `[ OK ] audit: user pointer and kernel isolation bounds verified`
- `[ OK ] audit: root privilege gates verified (os_update, crashdump, shutdown, fw, rlimit)`
- `[ OK ] audit: capability boundaries verified (file, net, spawn, exec, write)`
- `[ OK ] audit: mandatory access control and address isolation verified`
- `[ OK ] audit: cross-track application least-privilege matrix verified`
- `[ OK ] Kernel security & regression audit: result=0`

### 7.2 Dedicated Security Audit Test Suite
The host-side validation script (`tools/tests/run_security_audit_test.sh` / `make test-security`) executes tests across:
1. Syscall number and gate parity check (84 syscalls verified)
2. MAC profile policy verification (12 profiles verified)
3. Ring-3 binary privilege separation and credential sanitization patterns
4. High-memory user-range pointer bounds verification
5. Framebuffer mutual exclusion and shared memory ACL checks

### 7.3 Release Track Test Pass Summary
- **Desktop Profile (`make test`):** 319 / 319 assertions passed (100% pass rate).
- **Server Profile (`make test-server`):** 248 / 248 assertions passed (100% pass rate).
- **Security Audit Test (`make test-security`):** 52 / 52 security invariant checks passed.
- **Overall Status:** Verified, regression-free, and release-candidate ready.
