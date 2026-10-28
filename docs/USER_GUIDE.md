# NovaOS End-User Guide & Reference Manual

Welcome to **NovaOS**, a modern, preemptive, multitasking 32-bit x86 operating system pairing a rock-solid monolithic kernel with a memory-safe, `#![no_std]` Rust ring-3 userland and unified graphical desktop environment.

---

## 1. System Overview & Architecture

NovaOS provides an out-of-the-box graphical desktop computing environment engineered for safety, speed, and deterministic reliability:

- **Unified Design System (`ui-rs`)**: Every userland application renders directly onto 32-bit XRGB pixel buffers ([`BufferSurface32`](file:///home/firoze/projects/others/novaos/userland/ui-rs/src/surface.rs)) using anti-aliased bitmap typography, standardized color themes, and zero per-pixel system call overhead.
- **Strict Userland Safety**: All graphical applications run strictly in unprivileged ring-3 mode under a 16KB stack ceiling, placing all persistent state and render framebuffers in static BSS storage to eliminate stack overflows.
- **Dual Display Pipeline**: Supports hardware-accelerated linear VESA/VBE framebuffers (1024x768x32bpp) and VirtIO-GPU 2D host resource pipelines.
- **Security & Integrity**: Process isolation with mandatory access control (MAC), stateful packet filtering firewall, salted RFC 8018 PBKDF2-HMAC-SHA256 user credential hashing, and atomic A/B dual-slot operating system updates.

---

## 2. Installation & First-Run Setup

### 2.1 Graphical Installer (`INSTALL.ELF`)
If installing NovaOS to bare metal or a virtual disk, the Graphical Installer walks you through a 6-step guided wizard:
1. **Welcome & Hardware Audit**: Automatically scans hardware capabilities (CPU cores, physical memory, block devices `/dev/ata0`, `/dev/vda`, `/dev/nvme0n1`, and video display).
2. **Disk Selection & Partitioning**: Choose your destination drive and select an automated partition preset (`Standard` 64/32/4/1 MB, `Minimal` 32/16/2/1 MB, or `Workstation` 128/64/8/2 MB). The partitioning engine automatically provisions Partition 1 (FAT32 EFI/System), Partition 2 (Ext2 Rootfs), Partition 3 (FAT32 Write-Ahead Journal), and Partition 4 (Raw Crash Dump Area).
3. **Account & System Identity**: Configure machine hostname (`SYSTEM.CFG`), administrator username, and password.
4. **Pre-Install Summary**: Inspect partition geometry and safety checklist before proceeding.
5. **Live Installation Engine**: Formats file systems, writes core system binaries, and provisions bootloaders.
6. **Installation Complete**: Offers immediate system reboot into your newly installed NovaOS environment.

### 2.2 First-Run Setup Wizard (`FIRSTRUN.ELF` / `SETUP.ELF`)
Upon first booting an initialized system, the First-Run Setup Wizard greets you to complete personalization:
- **System Identity**: Set or verify your computer hostname and select the default init manager (`NOVAINIT.ELF`).
- **Administrator Setup**: Configures root administrative privileges (UID 0 / GID 0) with PBKDF2-HMAC-SHA256 password hashing.
- **Network Configuration**: Select DHCP auto-configuration or static IPv4 settings (IP, Netmask, Gateway, DNS), and choose stateful firewall policy presets.
- **Visual Preferences & Accessibility**: Select color theme (Dark, Light, HighContrast) and typography scale (100% to 200%).
- **Deployment**: Progressively and atomically commits configuration files (`SYSTEM.CFG`, `USERS.CFG`, `FIREWALL.CFG`, `ACCESS.CFG`) before launching into your chosen session.

---

## 3. Desktop Environment & Window Manager (`WM.ELF`)

NovaOS features a complete windowing environment driven by the `WM.ELF` compositor:

```
+--------------------------------------------------------------------------------+
| [NovaOS]  File  Edit  View  Window  Help                     12:34 PM [Sound]  |  <- Top Menu Bar
+--------------------------------------------------------------------------------+
|                                                                                |
|   +--------------------------+          +----------------------------------+   |
|   | Terminal - /home/firoze  |          | Text Editor - NOTES.TXT          |   |
|   +--------------------------+          +----------------------------------+   |
|   | $ cat /etc/hostname      |          | Welcome to NovaOS!               |   |
|   | novaos                   |          | Pure Rust #![no_std] userland.   |   |
|   | $ _                      |          |                                  |   |
|   +--------------------------+          +----------------------------------+   |
|                                                                                |
+--------------------------------------------------------------------------------+
| [Start] | [Files] [Terminal] [Editor] [Store] [Settings] [ActMon] | Taskbar  |  <- Bottom Dock
+--------------------------------------------------------------------------------+
```

### 3.1 Window Controls & Chrome
Every window features standardized chrome:
- **Titlebar**: Displays the application icon, window title, and active focus status. Dragging moves the window smoothly across the 1024x768 desktop.
- **Minimize (`-`)**: Collapses the window into the bottom taskbar.
- **Maximize / Restore (`[]`)**: Toggles the window between full desktop workspace geometry and its previous floating position.
- **Close (`x`)**: Gracefully terminates the application process.

### 3.2 Advanced Window Snapping & Tiling
Dragging a window to screen perimeters activates smart snapping overlays:
- **Left Edge**: Snaps window to the left half of the display (`512x708`).
- **Right Edge**: Snaps window to the right half of the display (`512x708`).
- **Top Edge**: Snaps window to full maximize workspace (`1024x708`).
- **Corners**: Snaps window into quadrant quadrants (`TopLeft`, `TopRight`, `BottomLeft`, `BottomRight`).
- **Tile Modes**: Access automated tiling from the **Window** menu:
  - *Tile Side-by-Side*: Splits the two most recent windows into left and right halves.
  - *Tile 2x2 Grid*: Automatically tiles active windows into a 4-quadrant layout.
  - *Cascade*: Arranges all open windows in an overlapping cascading stack.

### 3.3 Top Menu Bar & Bottom Dock
- **Top Menu Bar (`y = 0..24`)**: Persistent system menu offering Apple/System menu actions, active application menus, battery/network telemetry, clock, and sound volume indicator.
- **Bottom Dock / Taskbar (`y = 732..768`)**: Launches pinned applications, displays running tasks with active status indicators, and provides one-click window switching.

---

## 4. Accessibility & Display Preferences

NovaOS treats accessibility as a foundational, non-optional standard:

### 4.1 Typography & Font Scaling
Choose between 4 dynamic font scaling tiers in **Settings > Display & Accessibility**:
- **Normal (100%)**: Default 8x12 typography ideal for standard viewing.
- **Medium (125%)**: Scaled font metrics with increased line height for compact displays.
- **Large (150%)**: Enlarged character advances and glyphs for comfortable reading.
- **Extra Large (200%)**: Double-scale rendering for low-vision users.

All window titlebars, menubars, and application text automatically reflow and scale without clipping.

### 4.2 WCAG 2.1 AAA High-Contrast Theme
The `HighContrast` theme delivers maximum legibility:
- Background: Pure solid black (`#000000`).
- Text & Window Chrome: Crisp solid white (`#FFFFFF`).
- Focus Indicators: Vibrant high-visibility yellow (`#FFFF00`).
- Window Outlines: Reinforced 2px solid borders; window drop shadows disabled.

### 4.3 High-Visibility Cursors
Choose from custom cursor themes:
- **Classic White**: Standard white cursor with dark border.
- **Amber / Retro**: High-visibility amber arrow.
- **High Contrast**: Solid bright yellow (`#FFFF00`) arrow with thick pitch-black border.

Preferences are persistently stored in the 64-byte binary `ACCESS.CFG` (`NVAC` magic) on Partition 1 with CRC checksum validation.

---

## 5. Core Applications Guide

### 5.1 File Manager (`FILES.ELF`)
- **Navigation**: Interactive path breadcrumbs with one-click folder jumping.
- **Directory Listing**: Displays file names, file sizes, permissions, and modification timestamps.
- **File Preview**: Monospace sidebar previewing text, configuration files, and ELF binaries.
- **File Operations**: Open files in designated applications, create directories, and safely delete items.

### 5.2 Terminal Emulator (`TERM.ELF`)
- **VT100 ANSI Emulation**: Full ANSI escape code support (colors, cursor positioning, clear screen).
- **Interactive Line Editing**: History navigation (`Up`/`Down`), cursor movement (`Left`/`Right`, `Home`/`End`), and tab completion.
- **Shell Features**: Full pipeline support (`cat FILE | grep PATTERN`), I/O redirection (`> FILE`, `>> FILE`), and background job execution.

### 5.3 Monospace Text Editor (`EDIT.ELF`)
- **Editing Buffer**: Clean text editing buffer with line numbering and syntax highlighting.
- **Navigation**: Arrow key navigation, word jumping, page scrolling, and line wrap.
- **File Management**: Open existing files via file picker or create new documents; Ctrl+S quick save.

### 5.4 Software Store (`STORE.ELF`)
- **Repository Browser**: Browses packages from `/REPO/PACKAGES.LST` categorized into Utilities, System, Development, and Games.
- **Package Details**: Displays package version, size, dependencies, description, and cryptographic signatures.
- **Actions**: One-click Install, Update, and Remove with real-time download and verification status.

### 5.5 Settings Application (`SETTINGS.ELF`)
A centralized control panel organized into specialized modules:
- **Display & Accessibility**: Font scaling (100-200%), color themes (Dark/Light/HighContrast), and cursor styles.
- **Service Manager**: Inspect running services, view restart policies (`always`, `on-failure`), and control `NOVAINIT.ELF` daemons.
- **Device Manager**: Interactive hardware tree listing CPU, PCI devices, network adapters, and storage controllers.
- **Firewall & Security**: Configure stateful packet filtering rules, default policies (`drop` vs `accept`), and review connection tracking tables.
- **Software Updates**: Atomic OS update dashboard and A/B boot-slot telemetry (see Section 6).

---

## 6. System Administration & Diagnostics

### 6.1 Activity Monitor (`ACTMON.ELF`)
- **Live Performance Meters**: Real-time graphs for CPU core utilization and physical memory consumption.
- **Process Table**: Lists active tasks, PID, parent PID, user ID, CPU usage, memory footprint, priority, and state.
- **Process Management**: Kill (`SIGKILL`), Terminate (`SIGTERM`), and adjust execution priority (renice).

### 6.2 Structured Kernel Log Viewer (`KLOG.ELF`)
- **Syslog Query**: Direct ring-buffer query displaying kernel timestamps and log messages.
- **Severity Filtering**: Filter by log levels: `EMERG`, `ALERT`, `CRIT`, `ERR`, `WARNING`, `NOTICE`, `INFO`, and `DEBUG`.
- **Search & Export**: Monospace search with highlight filtering and log file export.

### 6.3 Problem Reporter & Crash Dump Recovery (`CRASHREP.ELF`)
In the event of an unrecoverable system panic:
- NovaOS automatically captures a hardware snapshot into Partition 4 (crash dump partition) and exports `/CRASHDMP.TXT` and `/CRASHDMP.DAT`.
- On reboot, the graphical crash reporter automatically appears:
  - **Overview**: Displays panic reason, faulting EIP address, and CR2 memory access address.
  - **Registers**: Full snapshot of registers (`EAX`, `EBX`, `ECX`, `EDX`, `ESI`, `EDI`, `EBP`, `ESP`, `EIP`, `EFLAGS`).
  - **Call Stack**: Decoded stack frame backtrace identifying the exact module that triggered the fault.
  - **Toolbar**: `[Clear Record]` clears the pending crash status via `SYS_CRASHDUMP_CTL` (80); `[Reboot]` safely reboots the system.

### 6.4 Atomic Dual-Slot OS Updates
NovaOS supports zero-downtime, fully recoverable atomic operating system updates:
- **Dual Boot-Slots**: Slot A (`/boot/novaos.bin`) and Slot B (`/boot/novaos_b.bin`).
- **Signature Verification**: Updates (`OSUPDATE.PKG`) require valid cryptographic HMAC-SHA256 signatures matching the trusted system key.
- **Watchdog & Auto-Rollback**: On applying an update, the bootloader boots the staging slot in `Testing` mode. If the system fails to confirm health within 3 boot attempts, the hardware watchdog automatically rolls back to the known-good slot.

---

## 7. Keyboard Shortcuts & Reference

| Shortcut | Action | Scope |
|:---|:---|:---|
| `Alt + F4` | Close active window | Desktop |
| `Alt + Tab` | Switch between open windows | Desktop |
| `Win + Left` | Snap active window to left half | Window Manager |
| `Win + Right` | Snap active window to right half | Window Manager |
| `Win + Up` | Maximize active window | Window Manager |
| `Win + Down` | Restore / minimize active window | Window Manager |
| `Ctrl + Alt + T` | Open Terminal | Global |
| `Ctrl + S` | Save current file | Text Editor |
| `Ctrl + Q` | Quit application | General |
| `Ctrl + C` | Send interrupt signal | Terminal |
| `Ctrl + L` | Clear terminal screen | Terminal |

---

## 8. Summary of System Configuration Files

| File | Magic / Type | Description |
|:---|:---|:---|
| `SYSTEM.CFG` | `NVCF` (81 bytes) | Machine identity, hostname, primary user, and init target. |
| `USERS.CFG` | `NVUS` (716 bytes) | User records with UID/GID, 16-byte salt, and PBKDF2-HMAC-SHA256 hashes. |
| `ACCESS.CFG` | `NVAC` (64 bytes) | Visual theme, typography scale, cursor style, and 16-bit CRC checksum. |
| `FIREWALL.CFG` | Text | Stateful packet filter policies, incoming/outgoing rules, and helper configs. |
| `SERVICES.CFG` | Text | Service definitions, execution binaries, dependencies, and restart policies. |
