# NovaOS 2.0 Developer SDK Reference Guide

## 1. Overview of the NovaOS Developer SDK

The **NovaOS Developer SDK** (`novaos-sdk`) is the official pure Rust framework for constructing high-performance, native GUI and CLI applications for NovaOS. Designed with memory safety, zero-cost abstractions, and sub-millisecond responsiveness in mind, the SDK provides comprehensive bindings for window management, vector graphics rasterization, retained-mode UI widgets, capability-safe I/O, and typed IPC messaging.

```
+---------------------------------------------------------------------------------------------------------+
|                                    Developer Application Code                                           |
|                  (main.rs utilizing declarative UI macros, reactive state & IPC)                       |
+---------------------------------------------------------------------------------------------------------+
|                                       novaos-sdk Pure Rust Framework                                    |
|  +--------------------+  +--------------------+  +--------------------+  +--------------------+         |
|  |    app Module      |  |     ui Module      |  |  graphics Module   |  |     ipc Module     |         |
|  | - AppBuilder       |  | - Retained Graph   |  | - Vector Canvas    |  | - MessageChannel   |         |
|  | - Event Loop       |  | - Flexbox Layout   |  | - Acrylic Shaders  |  | - ShmChannel (Zero)|         |
|  | - Lifecycle Hooks  |  | - Reactive State   |  | - TrueType Fonts   |  | - EventBus Broadcast|        |
|  +--------------------+  +--------------------+  +--------------------+  +--------------------+         |
|  +--------------------------------------------+  +--------------------------------------------+         |
|  |             fs & sandboxing Module         |  |             system Module                  |         |
|  |  - SandboxedFs Capability Bounds           |  |  - SystemInfo & Hardware Telemetry         |         |
|  |  - AppPaths (Data, Config, Cache, Docs)    |  |  - Process Telemetry & Clipboard Access    |         |
|  +--------------------------------------------+  +--------------------------------------------+         |
+---------------------------------------------------------------------------------------------------------+
|                    cargo-nova Toolchain Driver (CLI, Packaging & Simulator)                             |
|         cargo nova new  |  cargo nova build  |  cargo nova package  |  cargo nova run                   |
+---------------------------------------------------------------------------------------------------------+
```

---

## 2. Installation & Toolchain Setup

The NovaOS SDK integrates directly with standard Rust toolchains (`rustc` and `cargo` $\ge$ 1.75).

### 2.1 Installing the `cargo-nova` Toolchain
```bash
# Install cargo-nova from the local SDK repository
cargo install --path sdk/cargo-nova

# Verify installation
cargo nova --version
# Expected: cargo-nova 2.0.0 (Prometheus Enterprise Edition)
```

### 2.2 Scaffolding a New Application
`cargo-nova` provides pre-configured production templates:
- `gui`: Standard retained-mode vector graphical application.
- `cli`: Command-line system utility with terminal styling.
- `service`: Headless background daemon orchestrated by `novad`.

```bash
# Create a new graphical application
cargo nova new system-monitor --template gui

# Project Directory Structure:
# system-monitor/
# |-- Cargo.toml             # Rust manifest with novaos-sdk dependency
# |-- Info.json              # Application bundle manifest and capability declarations
# |-- src/
# |   `-- main.rs            # Application entry point and state machine
# `-- Assets/
#     `-- icon.png           # Vector/PNG application icon
```

---

## 3. `cargo-nova` Command Reference

| Command | Arguments | Description |
| :--- | :--- | :--- |
| **`cargo nova new`** | `<name> [--template <gui\|cli\|service>]` | Scaffolds a new application using the selected template |
| **`cargo nova build`**| `[--release] [--target <triple>]` | Compiles application code against the NovaOS sysroot |
| **`cargo nova package`**| `[--release]` | Generates a verified `.app` bundle with `Info.json` and checksums |
| **`cargo nova run`** | `[--simulator]` | Launches application instantly inside the host desktop simulator |
| **`cargo nova test`** | `[--all]` | Executes unit and integration tests under mock kernel environments |
| **`cargo nova clean`**| None | Cleans local target build artifacts and package caches |

---

## 4. Core SDK Architecture & Modules

### 4.1 Application Lifecycle (`novaos_sdk::app`)
The `AppBuilder` orchestrates window creation, event dispatch, and frame rendering:

```rust
use novaos_sdk::app::{AppBuilder, Application, Context};
use novaos_sdk::graphics::Canvas;
use novaos_sdk::ui::Event;

struct DemoApp {
    counter: u32,
}

impl Application for DemoApp {
    fn init(&mut self, _ctx: &mut Context) {
        println!("DemoApp initialized");
    }

    fn handle_event(&mut self, event: Event, _ctx: &mut Context) {
        if let Event::KeyDown(key) = event {
            println!("Key pressed: {:?}", key);
        }
    }

    fn update(&mut self, _ctx: &mut Context) {
        // Periodic state updates (e.g., animations or telemetry)
    }

    fn render(&mut self, canvas: &mut Canvas) {
        canvas.clear(novaos_sdk::graphics::Color::from_hex("#1E1E1E"));
        canvas.draw_text("Hello, NovaOS 2.0!", 50.0, 50.0, 24.0, novaos_sdk::graphics::Color::WHITE);
    }
}

fn main() {
    AppBuilder::new()
        .title("Demo Application")
        .size(800, 600)
        .resizable(true)
        .run(DemoApp { counter: 0 });
}
```

### 4.2 Retained-Mode UI & Flexbox (`novaos_sdk::ui`)
`novaos-sdk` features a declarative UI macro `ui! { ... }` with automatic Flexbox layout calculation:

```rust
use novaos_sdk::ui::{Button, Container, FlexDirection, Label, State, Widget};

pub struct CounterView {
    count: State<i32>,
}

impl CounterView {
    pub fn new() -> Self {
        Self { count: State::new(0) }
    }

    pub fn build(&self) -> Box<dyn Widget> {
        let count_state = self.count.clone();
        
        Container::new()
            .direction(FlexDirection::Column)
            .padding(20.0)
            .gap(12.0)
            .child(Label::new(format!("Count: {}", self.count.get())).size(20.0))
            .child(
                Button::new("Increment")
                    .on_click(move || {
                        count_state.set(count_state.get() + 1);
                    })
            )
            .into_widget()
    }
}
```

### 4.3 High-Performance Graphics (`novaos_sdk::graphics`)
Vector graphics APIs provide anti-aliased primitive drawing with subpixel TrueType font rendering:
```rust
// Canvas drawing methods:
canvas.draw_rect(rect, paint);
canvas.draw_rounded_rect(rect, corner_radius, paint);
canvas.draw_circle(center_point, radius, paint);
canvas.draw_line(start_point, end_point, paint);
canvas.draw_acrylic_blur(rect, blur_radius, noise_opacity);
canvas.draw_icon(IconType::Settings, rect, color);
```

### 4.4 Capability-Safe Filesystem Access (`novaos_sdk::fs`)
To satisfy enterprise sandboxing, processes access the filesystem through `SandboxedFs`, which guarantees path traversal defense:
```rust
use novaos_sdk::fs::{AppPaths, SandboxedFs};

let data_dir = AppPaths::data_dir("my-app");
let fs = SandboxedFs::open(data_dir)?;

// Read and write within the sandboxed container
fs.write_file("config.json", b"{ \"volume\": 80 }")?;
let content = fs.read_to_string("config.json")?;
```

### 4.5 Inter-Process Communication (`novaos_sdk::ipc`)
Zero-copy shared memory channels (`ShmChannel`) allow 60 FPS video or high-frequency telemetry exchange:
```rust
use novaos_sdk::ipc::{MessageChannel, ShmChannel};

// Typed IPC channel:
let channel: MessageChannel<SystemEvent> = MessageChannel::connect("system.events")?;
channel.send(SystemEvent::UserLoggedIn("sysadmin".into()))?;

// Zero-copy triple buffer for streaming frames:
let shm = ShmChannel::create("camera_stream", 1920 * 1080 * 4)?;
let mut buffer = shm.acquire_write_buffer();
buffer.fill(0xFF);
shm.commit();
```

---

## 5. `.app` Application Bundle Specification

NovaOS packages userland applications into self-contained directory bundles ending in `.app`.

### Bundle Structure
```
Calculator.app/
|-- Info.json              # Application manifest and capability declaration
|-- NovaOS/
|   `-- calculator         # 64-bit ELF executable compiled for x86_64-unknown-novaos
|-- Assets/
|   |-- icon-16x16.png
|   |-- icon-32x32.png
|   |-- icon-64x64.png
|   `-- icon-128x128.png
|-- Resources/
|   |-- strings.json       # Internationalization localization strings
|   `-- fonts/
|       `-- JetBrainsMono.ttf
`-- Manifest.sha256        # SHA-256 cryptographic hashes for every bundle file
```

### Manifest Schema: `Info.json`
```json
{
  "id": "org.novaos.calculator",
  "name": "Calculator",
  "version": "2.0.0",
  "entry_point": "NovaOS/calculator",
  "category": "Utilities",
  "description": "Enterprise scientific and programmer calculator for NovaOS.",
  "author": "NovaOS Foundation",
  "capabilities": [
    "can_read_files"
  ],
  "display": {
    "default_width": 420,
    "default_height": 580,
    "min_width": 360,
    "min_height": 480,
    "resizable": true,
    "supports_hidpi": true
  }
}
```

---

## 6. Host Simulator Development Workflow

Developing directly on target hardware or QEMU can be time-consuming. The NovaOS SDK includes an instant host simulator (`cargo nova run --simulator`) that emulates kernel framebuffer presentation, window snapping, and event delivery on Linux and macOS:

```bash
# Rapid compile and test loop (< 1 second)
$ cargo nova run --simulator
[cargo-nova] Compiling system-monitor v2.0.0...
[cargo-nova] Packaging system-monitor.app...
[cargo-nova] Starting NovaOS Host Simulator (1024x768 @ 60 FPS)...
[simulator] Display head initialized (BufferSurface32).
[simulator] Window mapped: 'System Monitor' (PID 1024).
```

Changes made to UI layouts, shaders, or IPC logic can be inspected immediately with live keyboard and mouse input on the developer's workstation.
