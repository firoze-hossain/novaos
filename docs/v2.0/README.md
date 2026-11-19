# NovaOS 2.0 "Prometheus" Enterprise Technical Documentation

Welcome to the official technical documentation library for **NovaOS 2.0 ("Prometheus")**. This enterprise documentation suite provides complete architectural specifications, kernel manuals, system administration guides, and developer frameworks for operating system engineers, cloud administrators, and application developers.

```
                                  +---------------------------------------+
                                  |    NovaOS 2.0 Documentation Suite    |
                                  +---------------------------------------+
                                                      |
         +--------------------+-----------------------+-----------------------+--------------------+
         |                    |                       |                       |                    |
         v                    v                       v                       v                    v
+-----------------+  +-----------------+     +-----------------+     +-----------------+  +-----------------+
|  Architecture   |  | Kernel Internals|     |  Desktop User   |     |  Server Admin   |  |  Developer SDK  |
|  Specification  |  |  & Syscall ABI  |     | & Customization |     |  & Containers   |  | Reference Guide |
+-----------------+  +-----------------+     +-----------------+     +-----------------+  +-----------------+
| System overview |  | 64-bit dispatch |     | Shell & UI      |     | Cloud-init      |  | novaos-sdk      |
| Memory map      |  | Syscall table   |     | Snap layouts    |     | novad services  |  | cargo-nova CLI  |
| PML4 paging     |  | Calling ABI     |     | Fluent Acrylic  |     | Containers      |  | Vector widgets  |
| NovaFS & uring  |  | Wire structs    |     | Themes & HiDPI  |     | RAID & Updates  |  | .app bundles    |
+-----------------+  +-----------------+     +-----------------+     +-----------------+  +-----------------+
```

---

## Technical Manuals Index

### 1. [NovaOS Architecture Specification](ARCHITECTURE_SPECIFICATION.md)
*Deep system architectural overview and subsystem engineering blueprints:*
- Executive summary & microkernel vs hybrid architecture design philosophy.
- Complete 64-bit Long Mode virtual memory map ($256 \text{ TiB}$) and PML4 4-level paging hierarchy.
- Preemptive tickless SMP scheduler with Local APIC one-shot timers and work-stealing.
- NovaFS Copy-on-Write (CoW) B-tree enterprise filesystem with instantaneous $O(1)$ snapshots.
- Asynchronous zero-copy I/O ring engine (`nova_uring`).
- Hardware-accelerated graphics pipeline: DRM/KMS mode-setting, VirtIO-GPU 3D Virgl Mesa bridge, and DMA-BUF memory sharing.
- Dual-Edition architecture split: Desktop Edition vs Headless Server Edition.

### 2. [Kernel Internals & Syscall ABI Manual](KERNEL_INTERNALS_SYSCALL_ABI.md)
*Low-level ABI specification, calling conventions, and complete kernel interfaces:*
- Hardware 64-bit `syscall`/`sysret` dispatcher configuration via MSRs (`IA32_STAR`, `IA32_LSTAR`, `IA32_FMASK`, `IA32_KERNEL_GS_BASE`).
- Calling convention register allocation (`RAX`, `RDI`, `RSI`, `RDX`, `R10`, `R8`, `R9`) and error handling.
- Comprehensive System Call Table: Process, VFS, Memory, `nova_uring`, Shared Memory IPC, Framebuffer, Sockets, and MAC.
- Kernel wire formats and C structure definitions.
- POSIX-compliant `errno` failure matrix and capability-based authorization gates.

### 3. [Desktop User & Customization Guide](DESKTOP_USER_CUSTOMIZATION_GUIDE.md)
*End-user operations, desktop environment shell, and customization reference:*
- Pure Rust Wayland-style composited desktop environment (`nova-compositor`).
- macOS-style persistent unified Top Menu Bar with active application integration and system status tray.
- Windows 11-style centered dynamic Taskbar & Application Dock with running indicators and hover animations.
- macOS Spotlight-style Universal Instant Search (`nova-search`) with embedded calculator and fuzzy launcher.
- Windows Action Center & Notification Flyout with grouped alerts and Quick Settings toggles.
- 8-Zone Snap Layouts and automatic tiling grid shortcuts (`Super + Arrow`).
- Theming engine: Fluent Acrylic Mica vs Minimalist Cupertino, accent palettes, and HiDPI/Retina display scaling (100%–200%).
- Built-in application guide (`nova-term`, `nova-files`, `nova-edit`, `nova-settings`, `nova-monitor`, `nova-recovery`).

### 4. [Server Administration & Container Deployment Handbook](SERVER_ADMIN_CONTAINER_HANDBOOK.md)
*Enterprise server operations, cloud provisioning, and container infrastructure:*
- Headless Server Edition architecture: Sub-2-second boot, < 32 MiB baseline RAM footprint, NUMA scheduler tuning.
- Automated server provisioning via Cloud-Init (`novainit` ConfigDrive and NoCloud datasources).
- Systemd-compatible service orchestration (`novad`) and `novactl` service management CLI.
- Pure Rust Container Engine (`novactl container` / `novadock`) with 6-axis namespaces (PID, Net, Mount, IPC, UTS, User) and cgroups v2 resource governance.
- Enterprise cloud TCP stack with Google BBR and Cubic congestion control algorithms.
- Stateful packet-filter firewall (`novafw`) and in-kernel WireGuard VPN tunneling (`novavpn`).
- Enterprise multi-disk storage & software RAID (`NovaVolume` RAID 0, RAID 1, RAID 10) with hot-plug disk detection.
- Automated unattended updates with atomic A/B slot rollback and hardware watchdog safety (`novaupdate`).
- Prometheus telemetry and monitoring exporter (`novamon` on port 9100).

### 5. [Developer SDK Reference Guide](DEVELOPER_SDK_REFERENCE.md)
*Official application framework and developer tooling manual:*
- Overview of the pure Rust `novaos-sdk` framework and `cargo-nova` developer toolchain.
- Scaffolding new applications: `cargo nova new <app-name> --template gui|cli|service`.
- Application lifecycle management, `AppBuilder`, event dispatch, and frame rendering.
- Retained-mode vector UI toolkit, declarative `ui! { ... }` macros, and reactive state management (`State<T>`).
- Capability-safe sandboxed filesystem access (`SandboxedFs`) and standard application directories (`AppPaths`).
- High-bandwidth zero-copy shared memory channels (`ShmChannel`) and typed IPC message buses.
- Self-contained `.app` application bundle packaging and cryptographic integrity manifests (`Manifest.sha256`).
- Host simulator workflow (`cargo nova run --simulator`) for rapid < 1-second iteration on Linux and macOS.

---

## Documentation Verification & Quality Assurance

All architectural manuals, API tables, and code snippets are verified continuously against the NovaOS codebase:
```bash
# Execute the automated documentation qualification suite
make test-docs
```
