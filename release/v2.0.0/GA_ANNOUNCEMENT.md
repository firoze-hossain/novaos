# 🚀 Announcing NovaOS 2.0 "Prometheus" General Availability (GA)

**November 19, 2026** — The NovaOS Foundation and Core Engineering Team are thrilled to announce the official **General Availability (GA) of NovaOS 2.0 ("Prometheus")**.

NovaOS 2.0 represents the complete fulfillment of our ambitious 6-month enterprise build plan: an independent, modern 64-bit operating system re-engineered from the ground up in **100% pure safe Rust**.

---

## What is NovaOS 2.0?

NovaOS 2.0 unites the raw hardware execution speed of a monolithic kernel with the rigorous security, memory safety, and capability isolation typically found in microkernels.

### 🌟 Highlights at a Glance
- **100% Safe Pure Rust Kernel Subsystems**: PML4 4-level paging, preemptive tickless SMP scheduler, NovaFS CoW filesystem, `nova_uring` async I/O, and in-kernel WireGuard VPN.
- **Modern Hardware First**: Native UEFI 2.9 bootloader with GPT partitioning, NVMe 2.0 PCIe storage controller, and xHCI USB 3.2 host controller.
- **Synthesized Desktop Workstation**: `nova-compositor` Wayland display server, Windows 11-style centered dynamic dock with hover magnification, 8-zone snap layouts & tiling grid, macOS-style persistent unified top menu bar, Spotlight-style instant search, Action Center, and Fluent Acrylic/Cupertino materials.
- **Enterprise Cloud Server**: Headless server edition booting in < 1.8 seconds with < 32 MiB RAM footprint, automated Cloud-Init provisioning, `novad` systemd-compatible service orchestrator, pure Rust container runtime (`novactl`) with cgroups v2, OpenSSH server (`novasshd`), stateful firewall (`novafw`), and Prometheus exporter (`novamon`).
- **Resilient Enterprise Storage**: Software RAID (RAID 0, 1, 10), volume pooling, automated unattended updates with atomic A/B slot rollback, and emergency system recovery (`nova-recovery`).
- **Developer First**: Official `novaos-sdk` pure Rust framework, `cargo-nova` CLI developer tool, and rapid < 1-second host simulator runner.

---

## Dual Distribution Editions

NovaOS 2.0 is distributed as two production images built on a single certified kernel core:

1. **NovaOS 2.0 Desktop Edition (`novaos-desktop-2.0.iso`)**:
   Full graphical workstation ISO bootable on both legacy BIOS and modern UEFI hardware. Includes live desktop testing, graphical installer wizard (`nova-installer`), and the complete native desktop application suite.
2. **NovaOS 2.0 Server Edition (`novaos-server-2.0.iso`)**:
   Minimalist headless cloud image optimized for hypervisors (Proxmox, OpenStack, KVM, VMware) and bare-metal edge servers with out-of-band serial console redirection (`ttyS0`, 115200 baud).

---

## Getting Started

Visit our official documentation portal on RozeHub or download the release deliverables:
- **Documentation Portal**: [https://rozehub.internal/docs/novaos](https://rozehub.internal/docs/novaos)
- **Source Code & GitHub Repository**: [https://github.com/firoze-hossain/novaos](https://github.com/firoze-hossain/novaos)
- **Release Deliverables Directory**: `release/v2.0.0/`

```bash
# Verify release cryptographic signatures
cd release/v2.0.0
sha256sum -c SHA256SUMS
gpg --verify SHA256SUMS.asc SHA256SUMS
```

*Welcome to the future of safe, predictable, high-performance systems software.*
