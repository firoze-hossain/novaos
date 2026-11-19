# NovaOS 2.0 "Prometheus" Enterprise Architecture Specification

## 1. Executive Summary & Design Philosophy

NovaOS 2.0 ("Prometheus") is an enterprise-grade, high-performance operating system architected for mission-critical desktop workstations, cloud hypervisors, and edge servers. NovaOS unites the performance and raw hardware efficiency of a monolithic kernel with the security, determinism, and capability isolation typically associated with microkernels.

```
+---------------------------------------------------------------------------------------------------------+
|                                        NovaOS 2.0 Application Layer                                      |
|  [Desktop Apps: Files, Term, Edit]  [Container Runtimes: novactl/dock]  [Cloud Daemons: novad/sshd/mon] |
+---------------------------------------------------------------------------------------------------------+
|                                    Userland System Frameworks & SDK                                     |
|     novaos-sdk (Pure Rust)  |  nova-ui Scene Graph  |  novalibc (POSIX Compat)  |  Wayland Protocols    |
+---------------------------------------------------------------------------------------------------------+
|                                       Kernel Syscall ABI Boundary                                       |
|               Fast x86_64 Syscall/Sysret Dispatcher (MSR 0xC0000082) & Capability Gates                |
+---------------------------------------------------------------------------------------------------------+
|                                      NovaOS 2.0 Core Kernel Subsystems                                  |
|  +--------------------+  +--------------------+  +--------------------+  +--------------------+         |
|  |   Memory & MMU     |  |   SMP Scheduler    |  |  Storage & Files   |  |  Graphics & Input  |         |
|  |  PML4 Paging (4-L) |  | Preemptive Tickless|  | NovaFS B-Tree CoW  |  | DRM/KMS Mode-Set   |         |
|  |  Demand Paging/CoW |  | 5 Priority Classes |  | Lockless VFS Dcache|  | VirtIO-GPU 3D Virgl|         |
|  |  LZ4 Anonymous Swap|  | APIC One-Shot Timer|  | nova_uring Async IO|  | Wayland Compositor |         |
|  |  KASLR & NX Guard  |  | Work-Stealing SMP  |  | NovaVolume RAID0110|  | Zero-Copy DMA-BUF  |         |
|  +--------------------+  +--------------------+  +--------------------+  +--------------------+         |
|  +--------------------------------------------+  +--------------------------------------------+         |
|  |       Networking & Cloud Services          |  |         Security & Process Governance      |         |
|  |  Enterprise TCP (Cubic & BBR Congestion)   |  | Mandatory Access Control (MAC Engine)      |         |
|  |  Stateful Packet Firewall (novafw Conntrack|  | Cgroups v2 & 6-Axis Namespaces (PID/Net/...) |         |
|  |  Native In-Kernel WireGuard VPN (Noise IK) |  | FIPS 140-3 Zeroization & Stack Canaries   |         |
|  +--------------------------------------------+  +--------------------------------------------+         |
+---------------------------------------------------------------------------------------------------------+
|                                         Hardware Abstraction Layer                                      |
|    UEFI 2.9 GOP Bootloader | NVMe 2.0 PCIe | xHCI USB 3.2 | Intel HDA | RTL8139/VirtIO-Net | APIC/SMP   |
+---------------------------------------------------------------------------------------------------------+
```

### Core Design Principles
1. **Memory Safety by Architecture**: All high-complexity kernel subsystems (PML4 paging, NovaFS CoW, `nova_uring`, scheduling, container cgroups, WireGuard tunneling, and graphics rendering) are engineered in 100% pure safe Rust (`#![no_std]`).
2. **Deterministic Latency**: Preemptive tickless SMP scheduling driven by per-core Local APIC one-shot timers eliminates periodic timer tick interruptions during compute-bound workloads.
3. **Asynchronous Zero-Copy I/O**: High-bandwidth storage and network workloads leverage `nova_uring`, eliminating system call overhead through lockless submission and completion ring buffers.
4. **Defense in Depth**: Every userland process executes under least-privilege capability boundaries (`can_read_files`, `can_write_files`, `can_exec`, `can_network`, `can_host_access`, `can_raw_io`), verified by kernel gates and Mandatory Access Control (MAC).

---

## 2. Memory Architecture & Address Space Layout

NovaOS 2.0 operates exclusively in 64-bit Long Mode with a 4-level PML4 (Page Map Level 4) paging hierarchy. The address space provides 48 bits of virtual addressing ($256 \text{ TiB}$) split into equal lower-half userland and higher-half kernel domains.

### Virtual Memory Map

```
+----------------------------------+ 0xFFFFFFFFFFFFFFFF
| Upper System Guard Page          |
+----------------------------------+ 0xFFFFFFFF80000000
| Kernel Text, Data & BSS          | (Mapped to physical 1MB - 32MB)
+----------------------------------+ 0xFFFFC00000000000
| Dynamic vmalloc & Kernel Heap    | (Kernel heap expansion, slab/buddy)
+----------------------------------+ 0xFFFF888000000000
| Direct Physical Memory Mapping   | (Complete physical RAM 1:1 identity map)
+----------------------------------+ 0xFFFF800000000000  <-- Higher-Half Base
| Non-Canonical Address Space Hole | ($16 \text{ EiB} - 512 \text{ TiB}$)
+----------------------------------+ 0x00007FFFFFFFFFFF  <-- Userland Limit
| User Stack (Grows Downward)      | (Protected by non-executable guard page)
+----------------------------------+ 0x00007F0000000000
| Dynamic Libraries & mmap Region  | (ASLR randomized allocations)
+----------------------------------+ 0x0000000040000000
| User Heap (brk)                  | (Grows upward)
+----------------------------------+ 0x0000000000400000
| User Executable Code & Data      | (ELF load segments)
+----------------------------------+ 0x0000000000001000
| Zero-Page NULL Trap Guard        | (Unmapped; traps NULL pointer dereferences)
+----------------------------------+ 0x0000000000000000
```

### Page Translation Hierarchy
* **PML4 Table**: Root translation table indexed by bits [47:39]. Stored in control register `CR3`.
* **PDPT (Page Directory Pointer Table)**: Indexed by bits [38:30]. Supports 1 GiB huge pages when bit 7 (PS) is set.
* **Page Directory (PD)**: Indexed by bits [29:21]. Supports 2 MiB huge pages.
* **Page Table (PT)**: Indexed by bits [20:12]. Maps standard 4 KiB frames.
* **Hardware NX Bit**: Bit 63 enforces Execute-Disable on data, heap, and stack pages, preventing code injection attacks.

---

## 3. Kernel Subsystems

### 3.1 Preemptive Tickless SMP Scheduler
The scheduler manages up to 64 symmetric CPU cores using Local APIC one-shot timers:
- **Priority Classes**:
  1. `RealTime` (Priority 0–7): Guaranteed bounded dispatch for audio mixing and compositor page-flips.
  2. `System` (Priority 8–15): Essential system daemons (`novad`, `novasshd`).
  3. `High` (Priority 16–23): Interactive userland UI applications (`nova-shell`).
  4. `Normal` (Priority 24–31): Batch computations and user processes.
  5. `Idle` (Priority 32): Zero-power halt execution.
- **Work-Stealing Algorithm**: Cores with empty runqueues scan remote runqueues locklessly, migrating lower-priority tasks across NUMA nodes with cache-affinity preservation.

### 3.2 NovaFS Enterprise Filesystem
NovaFS is a native Copy-on-Write (CoW) B-tree filesystem:
- **Atomicity**: Updates allocate new extent blocks before updating B-tree parent nodes, committing transactions via an atomic generation advance in the superblock.
- **Snapshots & Subvolumes**: Instantaneous $O(1)$ snapshots preserve filesystem states without copying block data.
- **Data Integrity**: Superblock metadata is protected by IEEE 802.3 CRC32 checksums, while file data blocks verify SHA-256 digests on readback.

### 3.3 Asynchronous I/O Ring Engine (`nova_uring`)
Modeled after modern Linux `io_uring`, `nova_uring` bypasses traditional syscall overhead:
- **Submission Queue (SQ)**: Circular lockless array populated by ring-3 applications with I/O requests (`Read`, `Write`, `Fsync`, `Poll`).
- **Completion Queue (CQ)**: Circular array populated by kernel drivers with completion statuses and byte counts.
- **Kernel Polling (SQPOLL)**: Dedicated kernel thread drains the SQ autonomously, achieving zero system calls for high-frequency storage operations.

### 3.4 Hardware Graphics & Wayland Compositor
- **DRM/KMS Mode-Setting**: Kernel controls display timing, video modes, and hardware overlay planes via `/dev/dri/card0`.
- **VirtIO-GPU 3D Virgl**: Hardware-accelerated 3D command submission streams OpenGL/Vulkan primitives to host hypervisors.
- **DMA-BUF Zero-Copy**: Framebuffers share physical pages across client applications and `nova-compositor` without memory copies.

---

## 4. Dual-Edition Architecture Split

NovaOS 2.0 is distributed in two distinct editions sharing a single certified kernel core:

| Subsystem / Feature | NovaOS 2.0 Desktop Edition | NovaOS 2.0 Server Edition |
| :--- | :--- | :--- |
| **Primary Interface** | `nova-compositor` Wayland GUI | Headless Serial & SSH-2 Console |
| **Display Pipeline** | Full DRM/KMS, Mesa Virgl 3D, HiDPI | Text Console & Null Display Driver |
| **Desktop Shell** | Top Menu Bar, Centered Dock, Snap Grid | Disabled (Zero Memory Footprint) |
| **Service Orchestrator** | `novad` (Desktop UI Profile) | `novad` (Headless Cloud Profile) |
| **Automated Provisioning** | Graphical Installer (`nova-installer`) | Cloud-Init (`novainit` ConfigDrive/NoCloud) |
| **Container Hosting** | Development Sandboxes | Production `novactl` / `novadock` Pods |
| **Telemetry & Metrics** | Local Activity Monitor (`nova-monitor`) | Prometheus Exporter (`novamon` Port 9100) |
| **Secure Remote Shell** | Client Tools | OpenSSH Daemon (`novasshd`) |
| **Default Network Profile** | DHCP / Wi-Fi Network Manager | Static IP / LACP Bonding / WireGuard |

---

## 5. Security & Isolation Architecture

```
                  +----------------------------------------------+
                  |              Ring-3 User Process             |
                  +----------------------------------------------+
                                         |
                                         | Syscall Gate (MSR 0xC0000082)
                                         v
                  +----------------------------------------------+
                  |         Syscall Authorization Filter         |
                  +----------------------------------------------+
                                         |
                       +-----------------+-----------------+
                       |                                   |
                       v                                   v
        +-----------------------------+     +-----------------------------+
        |   Capability Mask Check     |     |  Mandatory Access Control   |
        |  - CAN_READ_FILES           |     |  - Enforce Security Profile |
        |  - CAN_WRITE_FILES          |     |  - Restrict File/Path Tree  |
        |  - CAN_EXECUTE              |     |  - Block Unauthorized Net   |
        |  - CAN_NETWORK              |     |  - Immutable Post-Freeze    |
        +-----------------------------+     +-----------------------------+
                       |                                   |
                       +-----------------+-----------------+
                                         | (Both Passed)
                                         v
                  +----------------------------------------------+
                  |           Kernel Subsystem Execution         |
                  +----------------------------------------------+
```

1. **Strict User Pointer Sanitization**: Kernel validates all userland memory buffers before dereferencing, ensuring pointers reside in canonical lower-half memory (`< 0x0000800000000000`) and do not wrap around the 64-bit integer limit.
2. **Mandatory Secret Zeroization**: Cryptographic credentials, private keys, and shadow password buffers enforce volatile memory clearing (`zeroize()`) immediately after use.
3. **Common Criteria & FIPS 140-3 Conformance**: AES, SHA-256, and HMAC primitives execute Known Answer Tests (KAT) at boot before system initialization completes.
