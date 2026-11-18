# NovaOS 2.0 Developer SDK & Userland Application Framework (`novaos-sdk`)

**Document Version:** 2.0 (Developer Release)  
**Target:** NovaOS 2.0 Application Developers & System Engineers  
**Classification:** Official Developer Documentation & API Architecture Manual  
**Author:** NovaOS Developer Platform Team <sdk@novaos.org>  

---

## 1. Executive Summary & Design Principles

The **NovaOS Developer SDK** (`novaos-sdk`) is the official native application development framework for NovaOS 2.0. Built entirely in pure, memory-safe Rust with zero external dependencies, `novaos-sdk` empowers developers to construct high-performance, aesthetically refined graphical applications (`.app` bundles) and server-grade command-line tools.

### 1.1 Core Architectural Principles
1. **Zero External Crate Dependencies:** The SDK is self-contained within the NovaOS ecosystem, linking directly to native system libraries (`nova-ui`, `nova-bundle`, `nova-io`, `nova-abi`).
2. **Retained-Mode Reactive UI:** High-frame-rate rendering engine powered by Flexbox layouts, Signed Distance Field (SDF) vector rasterization, acrylic blur filters, and subpixel antialiasing.
3. **Self-Contained Application Bundles (`.app`):** Applications package their metadata (`Info.json`), binaries, resources, and cryptographic manifests into unified directories, eliminating dependency conflicts and enabling drag-and-drop installation.
4. **Mandatory Capability Sandboxing:** Filesystem and system operations conform strictly to the NovaOS Common Criteria EAL4+ least-privilege security model.
5. **Ultra-Fast Host Simulation (<1s):** Develop and test applications directly on developer host machines (Linux, macOS) using the embedded software simulator before deploying to the OS.

---

## 2. CLI Tooling: `cargo-nova`

The `cargo-nova` utility is the primary CLI driver for project scaffolding, compilation, packaging, and simulation. It operates seamlessly as a Cargo plugin (`cargo nova <cmd>`) or as a standalone binary (`cargo-nova <cmd>`).

### 2.1 Command Reference

```
cargo nova <SUBCOMMAND> [OPTIONS]
```

| Subcommand | Arguments & Flags | Description |
|---|---|---|
| `new` | `<NAME> [--template <gui\|cli>] [--path <DIR>]` | Scaffolds a new project with Cargo manifest, `Info.json`, and starter source code. |
| `build` | `[--release] [--target <TRIPLE>]` | Compiles the project for the host simulator or native NovaOS target (`i686-novaos` / `x86_64-novaos`). |
| `package` | `[--release] [--out <DIR>]` | Assembles the compiled binary, assets, and `Info.json` into a verified `.app` bundle. |
| `run` | `[--simulator] [--emulator] [--release]` | Executes the app in the host simulator or boots the QEMU virtual machine. |
| `info` | None | Displays SDK version, toolchain environment, and framework settings. |
| `--selftest` | None | Runs automated end-to-end integration tests verifying scaffolding, compilation, packaging, and simulation. |

---

## 3. SDK Architecture & Module Overview

```
novaos-sdk/
├── app          # Application lifecycle, window builder, scene graph, State<T>
├── ui           # Retained-mode UI toolkit, Flexbox layout, vector shapes, theme
├── ipc          # MessageChannel, EventBus (pub/sub), ShmChannel (shared memory)
├── fs           # SandboxedFs (path traversal defense), AppPaths standard directories
├── sys          # SystemInfo, ProcessInfo, desktop notifications
├── emulator     # SimulatedWindow, InputSimulator, PPM visual buffer export
└── prelude      # Idiomatic glob import: use novaos_sdk::prelude::*;
```

---

## 4. Application Lifecycle & Reactive State

### 4.1 Creating an Application (`AppBuilder`)

Every GUI application defines an entry point using the fluent `App::builder` API:

```rust
use novaos_sdk::prelude::*;

fn main() {
    let mut app = App::builder("com.novaos.myapp", "My Application")
        .window_size(800, 600)
        .theme(ThemeMode::Dark)
        .accent(AccentPalette::FluentBlue)
        .resizable(true)
        .build();

    let main_view = app.add_view("root_panel", Rect::new(0, 0, 800, 600));

    // Run application loop
    app.step_frame();
}
```

### 4.2 Reactive State (`State<T>`)
State in `novaos-sdk` is reactive. Modifying state automatically flags subscribed scene nodes as dirty, requesting layout recomputation and repainting on the next frame:

```rust
// Initialize reactive state
let mut click_counter = State::new(0);

// Subscribe the UI card node to updates
click_counter.subscribe(main_view);

// Mutating state triggers damage invalidation
click_counter.set(click_counter.get() + 1, &mut app.scene);
```

---

## 5. Declarative UI & Layout Engine

`novaos-sdk` re-exports the full declarative widget and layout suite of `nova-ui`:

### 5.1 Flexbox Layout Macros
```rust
// Declarative column layout
let layout = col!(
    gap: 12,
    padding: 16,
    children: [
        button!("Save Document"),
        slider!(0.0, 100.0, 50.0),
        text_input!("Enter filename...")
    ]
);
```

### 5.2 Fluent Design System & Acrylic Materials
The toolkit adheres to modern desktop design aesthetics:
- **Windows 11 Fluent Acrylic / Mica:** Translucent panels with background Gaussian blur and depth box shadows.
- **macOS Typography & Geometry:** Clean corner radii (`CornerTokens::medium = 8px`), WCAG AA compliant contrast ratios (`meets_wcag_aa`), and high-DPI vector typography.

---

## 6. Inter-Process Communication (IPC)

NovaOS applications communicate through structured, capability-gated IPC primitives:

### 6.1 Typed Message Channels (`MessageChannel`)
Direct messaging built over `SYS_MSG_SEND` and `SYS_MSG_RECV`:

```rust
let mut channel = MessageChannel::new(current_pid);

// Send structured message
channel.send(receiver_pid, MSG_TYPE_DATA, b"payload_bytes")?;

// Receive message
if let Some(msg) = channel.recv() {
    println!("Received from {}: {:?}", msg.header.sender_pid, msg.payload);
}
```

### 6.2 Publish / Subscribe Event Bus (`EventBus`)
Asynchronous decoupled notifications across application components:

```rust
let mut bus = EventBus::new();

bus.subscribe("system.theme_changed", |payload| {
    println!("Theme changed event received: {:?}", payload);
});

bus.publish("system.theme_changed", b"Dark");
```

### 6.3 Shared Memory Buffers (`ShmChannel`)
Zero-copy high-throughput communication for multimedia or large data sets:

```rust
let mut shm = ShmChannel::create(1001, 1024 * 1024); // 1 MB
shm.write_at(0, b"framebuffer_data")?;
```

---

## 7. Sandboxed Filesystem Access

Applications execute in restricted sandboxes. Filesystem access is mediated by `SandboxedFs`:

```rust
let fs = SandboxedFs::new("/var/data/com.novaos.myapp");

// Write configuration
fs.write_file("settings.json", b"{\"volume\": 80}")?;

// Path traversal attempts are strictly prevented
assert!(fs.resolve("../../etc/shadow").is_err());
```

### 7.1 Standard System Directories (`AppPaths`)
- `AppPaths::app_data_dir("com.novaos.myapp")` -> `/var/data/com.novaos.myapp`
- `AppPaths::app_config_dir("com.novaos.myapp")` -> `/etc/com.novaos.myapp`
- `AppPaths::documents_dir()` -> `/home/user/Documents`
- `AppPaths::temp_dir()` -> `/tmp`

---

## 8. Self-Contained `.app` Bundle Format

Every NovaOS graphical application packages into a directory named `<ExecutableName>.app`:

```
Calculator.app/
├── Info.json           # Application manifest, permissions, and window settings
├── calculator          # 64-bit ELF binary (chmod +x)
├── Manifest.sha256     # Cryptographic integrity checksums
├── Assets/
│   └── icon.png        # Application launcher icon
└── Resources/          # Fonts, sounds, shaders, localization
```

### 8.1 `Info.json` Specification

```json
{
  "bundle_id": "com.novaos.calculator",
  "display_name": "Calculator",
  "version": "2.0.0",
  "executable": "calculator",
  "category": "Utilities",
  "icon": "Assets/icon.png",
  "permissions": [
    "ui"
  ],
  "window": {
    "width": 360,
    "height": 480,
    "resizable": false
  }
}
```

---

## 9. Host Simulator & Testing

Developers can test applications rapidly without needing to boot QEMU:

```bash
cargo nova run
```
The host simulator creates a `SimulatedWindow` with an in-memory `BufferSurface`, dispatches mouse/keyboard events via `InputSimulator`, and can export pixel-perfect Netpbm PPM snapshots (`app.window.export_ppm()`) for automated visual regression testing.

---

## 10. Sample Applications

The SDK repository includes two full reference applications:
- **`sdk/examples/calculator-app/`:** Retained-mode calculator GUI with responsive state, button grid, and arithmetic evaluation.
- **`sdk/examples/sysfetch-cli/`:** Fast, aesthetic system telemetry CLI tool displaying OS version, kernel information, memory, and security confinement status.
