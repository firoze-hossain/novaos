# NovaOS 2.0 Desktop User & Customization Guide

## 1. Overview of the NovaOS 2.0 Desktop

NovaOS 2.0 Desktop Edition delivers a modern, hardware-accelerated user experience engineered in 100% pure safe Rust. Powered by `nova-compositor` (a native Wayland-style display server) and the `nova-ui` retained-mode vector toolkit, NovaOS synthesizes the aesthetic refinement of macOS with the versatile window tiling ergonomics of Windows 11.

```
+---------------------------------------------------------------------------------------------------------+
| [NovaOS Logo]  File  Edit  View  Terminal  Help                [Active App]         [100%] [WiFi] 18:15 |
+---------------------------------------------------------------------------------------------------------+
|                                                                                                         |
|   +------------------------------------+             +------------------------------------+             |
|   | Terminal - nova-term               |             | File Browser - nova-files          |             |
|   +------------------------------------+             +------------------------------------+             |
|   | $ cargo nova build --release       |             | /home/firoze/projects/novaos       |             |
|   |    Compiling nova-app v0.1.0       |             | [ ] kernel/      [ ] userland/     |             |
|   |     Finished release [optimized]   |             | [ ] docs/        [ ] Cargo.toml    |             |
|   |                                    |             |                                    |             |
|   +------------------------------------+             +------------------------------------+             |
|                                                                                                         |
|                                                                                                         |
|                           +-----------------------------------------------+                             |
|                           |  [Files]  [Term]  [Edit]  [Monitor]  [Settings]   |                             |
|                           +-----------------------------------------------+                             |
+---------------------------------------------------------------------------------------------------------+
```

---

## 2. Desktop Shell Architecture

### 2.1 Unified Persistent Top Menu Bar
Positioned at the top of the display, the Top Menu Bar provides unified status indicators and global application menus:
- **Application Menu**: Reflects the currently active window's actions (File, Edit, View, Help). Menu actions are dispatched over lockless IPC to client applications.
- **System Indicator Tray**: Displays real-time status:
  - Network status (Ethernet / Wi-Fi signal strength and active IP).
  - Native WireGuard VPN indicator with active peer handshake status.
  - Master audio volume level with quick mute toggle.
  - System clock and calendar popup (`HH:MM`).

### 2.2 Centered Dynamic Taskbar & Application Dock
The dynamic application dock floats above the bottom margin with responsive acrylic blur:
- **Active Indicators**: Running applications display an illuminated dot beneath their icon.
- **Hover Animations**: Cursor proximity dynamically scales nearby icons with quadratic Bézier interpolation.
- **Application Pinning**: Users can customize default dock shortcuts by editing `~/.config/novaos/dock.json`.

### 2.3 Universal Instant Search (`nova-search`)
Activated via **`Super + Space`** or **`Ctrl + Space`**, Universal Search opens a centered search palette:
- **Fuzzy Application Launcher**: Type abbreviations (e.g., `term` for `nova-term`, `edit` for `nova-edit`) to launch applications instantly.
- **Embedded Calculator**: Evaluates arithmetic expressions and engineering equations directly in the query bar (e.g., `1024 * 768 * 4 / (1024 * 1024)` -> `3.0 MiB`).
- **Filesystem Search**: Rapidly indexes files and directories across `/home` and `/etc` using the kernel dentry cache.
- **System Commands**: Execute immediate system actions: `lock`, `logout`, `sleep`, `restart`, `shutdown`.

### 2.4 Action Center & Notification Flyout
Triggered by clicking the status tray or pressing **`Super + A`**:
- **Notifications Stack**: Groups system and application alerts with interactive dismiss and reply actions.
- **Quick Settings Toggles**:
  - `Wi-Fi` / `Ethernet`: Network interface control.
  - `VPN`: Toggle WireGuard tunnel.
  - `Night Light`: Adjusts color temperature from 6500K to 3400K to reduce blue light.
  - `Dark Mode`: Toggles between Acrylic Dark and Minimalist Light themes.
  - `Do Not Disturb`: Suppresses banner notifications during presentations and gaming.
- **Hardware Sliders**: Smooth continuous sliders for Volume and Display Backlight.

---

## 3. Window Management & 8-Zone Snap Layouts

NovaOS incorporates an advanced window management engine with automatic tiling and 8-zone edge snapping.

```
+-----------------------------------------------------------------------+
|  Top-Left Quarter (25%)  |   Top Half (50%)   |  Top-Right Qtr (25%)  |
+--------------------------+--------------------+-----------------------+
|                          |                    |                       |
|     Left Half (50%)      |   Full Screen      |    Right Half (50%)   |
|                          |     (Maximize)     |                       |
|                          |                    |                       |
+--------------------------+--------------------+-----------------------+
| Bottom-Left Quarter (25%)|  Bottom Half (50%) | Bottom-Right Qtr (25%)|
+--------------------------+--------------------+-----------------------+
```

### Keyboard Shortcut Matrix
| Shortcut | Action | Description |
| :--- | :--- | :--- |
| **`Super + Left`** | Snap Left | Resizes window to left 50% split |
| **`Super + Right`** | Snap Right | Resizes window to right 50% split |
| **`Super + Up`** | Maximize | Expands window to fill the entire desktop work area |
| **`Super + Down`** | Restore / Minimize | Restores tiled window to floating; second press minimizes |
| **`Super + Alt + Left`** | Snap Top-Left | Tiles window into top-left 25% quadrant |
| **`Super + Alt + Right`** | Snap Top-Right | Tiles window into top-right 25% quadrant |
| **`Super + Shift + Left`** | Snap Bottom-Left | Tiles window into bottom-left 25% quadrant |
| **`Super + Shift + Right`** | Snap Bottom-Right| Tiles window into bottom-right 25% quadrant |
| **`Super + 1` / `2` / `3`** | 3-Column Layout | Cycles active window through 50/25/25, 25/50/25, 25/25/50 columns |

### Interactive Mouse Snapping
Dragging any window title bar to screen edges triggers translucent blue snap preview guides:
- Drag to **Top Edge**: Full-screen maximize.
- Drag to **Left or Right Edge**: Vertical 50% split.
- Drag to **Any Corner**: 25% quadrant snap.

---

## 4. Personalization & Theming Engine

The visual design system supports two primary design paradigms:
1. **Fluent Acrylic Mica**: Multi-layer real-time Gaussian blur with subtle noise textures and luminescent window borders.
2. **Minimalist Cupertino**: Clean, high-contrast surfaces with crisp drop shadows and refined vector accents.

### Theme Configuration: `~/.config/novaos/theme.json`
```json
{
  "theme_mode": "dark",
  "accent_color": "#0078D4",
  "acrylic": {
    "enabled": true,
    "blur_radius": 24,
    "surface_opacity": 0.78,
    "noise_intensity": 0.04
  },
  "window": {
    "corner_radius": 12,
    "border_width": 1,
    "border_color": "#FFFFFF22",
    "shadow_radius": 16
  },
  "typography": {
    "font_family": "NovaSans",
    "font_size_base": 14,
    "subpixel_antialiasing": true
  },
  "wallpaper": {
    "path": "/usr/share/backgrounds/nova-aurora.png",
    "mode": "fill"
  }
}
```

### High-Resolution Display Scaling (HiDPI / Retina)
Configurable in **Settings -> Display**:
- **100% (1.0x)**: Standard 1080p displays.
- **125% (1.25x)**: High-density 1440p laptops.
- **150% (1.5x)**: Ultra-wide 3K displays.
- **200% (2.0x)**: 4K UHD and Apple Retina displays.

All system vector icons, fonts, and window borders scale proportionally using fixed-point coordinate mathematics, guaranteeing zero pixel blur or edge distortion.

---

## 5. Built-In Applications Suite

| Application | Command | Description |
| :--- | :--- | :--- |
| **Terminal** | `nova-term` | Hardware-accelerated GPU terminal with VT100 colors and split tabs |
| **File Manager** | `nova-files` | Tabbed file browser with instant search, preview, and snapshot recovery |
| **Text Editor** | `nova-edit` | Lightweight code editor with syntax highlighting for Rust, C, and Shell |
| **System Settings**| `nova-settings`| Centralized configuration for displays, network, audio, and accounts |
| **Activity Monitor**| `nova-monitor` | Real-time CPU, RAM, disk I/O graphs, and process management |
| **System Recovery**| `nova-recovery`| Emergency boot repair, snapshot rollback, and password reset |
| **Software Store** | `nova-store` | Graphical package installer connecting to the RozeHub marketplace |

---

## 6. Global Keyboard Shortcuts Cheat Sheet

| Shortcut | Function |
| :--- | :--- |
| **`Super + Space`** | Open Universal Instant Search (`nova-search`) |
| **`Super + T`** | Launch Terminal (`nova-term`) |
| **`Super + E`** | Launch File Manager (`nova-files`) |
| **`Super + I`** | Open System Settings (`nova-settings`) |
| **`Super + L`** | Lock Screen immediately |
| **`Super + A`** | Toggle Action Center & Quick Settings |
| **`Super + N`** | Toggle Notification Center |
| **`Alt + Tab`** | Fast Window Switcher with live thumbnails |
| **`Ctrl + Alt + Del`** | Open Emergency Task Manager / Power Menu |
| **`PrintScreen`** | Capture full-screen screenshot to `~/Pictures/Screenshots` |
| **`Super + Shift + S`** | Capture selected rectangular region screenshot |
