# NovaOS 2.0: Production Graphical Installer & Setup Wizard (nova-installer)

## 1. Executive Summary

NovaOS 2.0 Desktop Edition delivers a modern, intuitive, and beginner-friendly graphical installation and setup wizard (`nova-installer`) written in 100% pure safe Rust (`userland/desktop/apps/nova-installer`).

Designed to match the elegance, responsiveness, and visual polish of the Windows 11 Out-of-Box Experience (OOBE) and Ubuntu Ubiquity, `nova-installer` provides:
- **Intelligent Drive Partitioning Wizard**: Guided automatic partitioning ("Erase entire disk and install NovaOS") and custom manual GPT partitioning with visual multi-colored bar gauges, 1MB alignment verification, and overlap checks.
- **Enterprise Storage Formatting**: Formats target partitions with modern GUID Partition Tables (GPT), FAT32 EFI System Partitions (`/boot/efi`), and high-performance NovaFS Copy-on-Write B-Tree rootfs (`/`) and home (`/home`).
- **User Account & Security Configuration**: Interactive account setup, real-time password strength meter, cryptographic password hashing via PBKDF2-HMAC-SHA256 (RFC 8018) with constant-time verification, and automatic sudoers droplet generation.
- **Timezone & Regional Localization**: Global timezone database with instant substring search, UTC offset math, and simulated local time previews.
- **Desktop Personalization & Live Preview**: Real-time theme mode selection (Dark Acrylic / Mica vs Light Minimal Cupertino), accent color customization (Nova Blue, Obsidian Purple, Emerald Green, Sunset Coral, Cyberpunk Cyan), and interactive miniature desktop rendering.
- **Animated Slide Showcase**: Rotates 5 architectural slides highlighting pure Rust memory safety, NovaFS snapshots, 60 FPS Wayland graphics, container isolation, and the NovaOS Developer SDK while base packages are unpacked.
- **UEFI 2.9 Bootloader Deployment**: Installs `/EFI/BOOT/BOOTX64.EFI` and generates `loader.cfg` with GPT partition UUID pointers.

---

## 2. Architecture & Wizard State Pipeline

```
+-----------------------------------------------------------------------------------+
|                           NovaOS 2.0 Desktop Installer Window                      |
|                 (1024x700 Retained-Mode BufferSurface via nova-ui)                |
+-----------------------------------------------------------------------------------+
                                         |
                                         v
+-----------------------------------------------------------------------------------+
|                        9-Step Setup Wizard State Machine                          |
+-----------------------------------------------------------------------------------+
  [Step 0] Welcome & Architecture Overview (64-Bit Safe Rust, NovaFS, Wayland)
     |
     v
  [Step 1] Pre-Flight Qualification Checks (CPU Cores, RAM >= 2GB, UEFI 2.9, Storage)
     |
     v
  [Step 2] Storage & Partitioning Wizard
     |---> Mode A: Guided Automatic Partitioning (ESP + NovaFS Root + Home + Swap)
     |---> Mode B: Custom Manual GPT Partitioning (Add/Delete/Resize, Mount Points)
     |---> Visual Proportional Multi-Colored Partition Bar Gauge (1MB Alignment Checks)
     |
     v
  [Step 3] Timezone & Regional Localization (Interactive Search, UTC Math, Clock Preview)
     |
     v
  [Step 4] User Account & Security Configuration
     |---> Username & Hostname POSIX/RFC-1123 Validation
     |---> Real-Time Password Strength Meter (Very Weak -> Strong)
     |---> PBKDF2-HMAC-SHA256 Password Hash (4096 Iterations, Constant-Time ct_eq)
     |---> Sudoers Privilege Droplet (/etc/sudoers.d/90-<user>)
     |
     v
  [Step 5] Desktop Personalization & Live Preview
     |---> Dark Acrylic Mica vs Light Minimal Cupertino Mode
     |---> 5 Accent Colors (Nova Blue, Obsidian Purple, Emerald, Coral, Cyan)
     |---> Live Miniature Desktop Canvas (Top Menu Bar, Wallpaper, Centered Dock, Snap)
     |
     v
  [Step 6] Summary Blueprint & Confirmation (Review Target Disk, Format Authorization)
     |
     v
  [Step 7] Installation Pipeline Engine
     |---> 1. Write Protective MBR & Primary/Backup GPT Headers (CRC32 Checksums)
     |---> 2. Format EFI System Partition (FAT32, /boot/efi)
     |---> 3. Format NovaFS RootFS (NOVAFS01 Superblock, Transaction ID, B-Tree Roots)
     |---> 4. Unpack Base Packages, System Binaries, and Desktop Applications
     |---> 5. Install UEFI 2.9 Bootloader (/EFI/BOOT/BOOTX64.EFI & loader.cfg)
     |---> 6. Provision /etc/passwd, /etc/shadow, /etc/hostname, /etc/hosts, /etc/fstab
     |---> 7. Synthesize Desktop Personalization (/etc/novaos/desktop.json)
     |---> Live Animated Slide Showcase (5 Rotating Architectural Slides + Log Feed)
     |
     v
  [Step 8] Installation Complete & Reboot Prompt (Success Badge, Metrics, One-Click Restart)
```

---

## 3. Storage Partitioning & GPT Specification

`nova-installer` implements strict GPT partitioning standards conforming to UEFI 2.9 specifications:

1. **Protective MBR**:
   - Single MBR partition entry at offset `0x01BE` with partition type `0xEE` spanning the whole disk to prevent legacy MBR partitioning tools from corrupting GPT headers.

2. **Primary & Backup GPT Headers**:
   - Signature: `EFI PART` (`0x5452415020494645`).
   - Primary header located at LBA 1; backup header located at `LastLBA` (`total_sectors - 1`).
   - First usable LBA: `2048` (1 MiB boundary alignment).
   - Last usable LBA: `total_sectors - 34`.
   - IEEE 802.3 CRC32 checksums for both partition entry array and header.

3. **Standard Guided Layout**:
   | Partition | Label | Filesystem | Mount Point | Default Size | GUID |
   |:---|:---|:---|:---|:---|:---|
   | Part 1 | `EFI System Partition` | FAT32 | `/boot/efi` | 512 MiB | `c12a7328-f81f-11d2-ba4b-00a0c93ec93b` |
   | Part 2 | `NovaOS System Root` | NovaFS | `/` | 40% (min 20GB) | `4f68bce3-e8cd-4db1-96e7-fbcaf984b709` |
   | Part 3 | `NovaOS User Data` | NovaFS | `/home` | Remainder | `ebd0a0a2-b9e5-4433-87c0-68b6b72699c7` |
   | Part 4 | `Swap & CrashDump` | Swap | `swap` | 4096 MiB | `0657fd6d-a4ab-43c4-84e5-0933c84b4f4f` |

---

## 4. User Security & Password Derivation

- **Algorithm**: PBKDF2-HMAC-SHA256 (RFC 8018 / NIST SP 800-132).
- **Salt**: 16-byte cryptographically derived pseudo-random salt.
- **Iterations**: 4096 rounds of HMAC-SHA256.
- **Output**: 32-byte digest stored in `/etc/shadow` using standard modular crypt format:
  ```text
  novauser:$pbkdf2$<hex_salt>$<hex_hash>:19700:0:99999:7:::
  ```
- **Privilege Separation**: Non-root accounts receive UID 1000, primary GID 1000, default shell `/bin/novash`, and sudo permissions in `/etc/sudoers.d/90-<username>`.

---

## 5. Desktop Personalization & Live Preview

The installer generates `/etc/novaos/desktop.json` which is read by `nova-compositor` and `nova-shell` on first boot:

```json
{
  "version": 2,
  "theme": "dark_mica",
  "accent_color": "#0078D4",
  "accent_name": "Nova Blue",
  "dock": {
    "position": "bottom_center",
    "magnification": true,
    "autohide": false
  },
  "display": {
    "scale_percent": 100,
    "snap_layouts": true,
    "acrylic_blur": true
  }
}
```

---

## 6. CLI Usage & Autonomous Qualification

- `nova-installer --selftest` / `nova-installer -t`: Executes all 8 internal qualification test suites and exits with code 0 on pass.
- `nova-installer --dry-run`: Runs installer wizard in simulation mode without physical disk writes.
- `nova-installer --help`: Displays command line usage.
- `make test-installer`: Executes the 14-check dedicated qualification test suite.
