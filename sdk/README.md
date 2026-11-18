# NovaOS 2.0 Developer SDK & Application Framework (`novaos-sdk`)

Welcome to the official **NovaOS Developer SDK** repository. This directory contains the complete tooling and software development kit for authoring native, high-performance applications for NovaOS 2.0 in pure Rust.

---

## Directory Overview

```
sdk/
├── Cargo.toml                  # Workspace root for SDK crates
├── README.md                   # This overview guide
├── novaos-sdk/                 # Core developer SDK library (app, ui, ipc, fs, sys, emulator)
├── cargo-nova/                 # Official CLI tool & project manager (cargo nova new/build/package/run)
├── templates/                  # Production application templates
│   ├── gui-app/                # Starter graphical app (Fluent Acrylic, Window, EventDispatcher)
│   └── cli-app/                # Starter command-line utility (SystemInfo, ProcessInfo)
└── examples/                   # Reference native applications
    ├── calculator-app/         # Retained-mode calculator GUI with responsive state & grid
    └── sysfetch-cli/           # Fast system telemetry & architecture inspection tool
```

---

## Quickstart

### 1. Install `cargo-nova` CLI
```bash
cargo install --path sdk/cargo-nova
```
Or use directly with cargo:
```bash
cargo nova --help
```

### 2. Create a New Project
```bash
# Create a new GUI desktop application
cargo nova new my-app --template gui

# Or create a command-line tool
cargo nova new my-tool --template cli
```

### 3. Run in Host Simulator
Develop and test with instant feedback (<1s compile & run):
```bash
cd my-app
cargo nova run
```

### 4. Package into Self-Contained `.app` Bundle
```bash
cargo nova package --release
```
This generates `build/my-app.app/` containing:
- `Info.json`: Application metadata, permissions, and window settings
- `my-app`: 64-bit ELF binary
- `Assets/icon.png`: Application icon
- `Resources/`: Private fonts, sound assets, and dictionaries
- `Manifest.sha256`: Cryptographic integrity manifest

---

## Core SDK Architecture (`novaos-sdk`)

| Module | Description |
|---|---|
| `novaos_sdk::app` | Application lifecycle, window builder, scene graph, and reactive `State<T>`. |
| `novaos_sdk::ui` | Retained-mode UI toolkit, Flexbox layout, vector shapes, and Acrylic themes. |
| `novaos_sdk::ipc` | Typed message channels, publish/subscribe `EventBus`, and `ShmChannel` buffers. |
| `novaos_sdk::fs` | Sandboxed filesystem operations, path normalization, and standard directory paths. |
| `novaos_sdk::sys` | `SystemInfo`, `ProcessInfo`, hardware telemetry, and desktop notifications. |
| `novaos_sdk::emulator` | `SimulatedWindow` rendering, input injection, and PPM buffer exports for testing. |
| `novaos_sdk::prelude` | Convenient glob re-exports for rapid application development. |

For full API documentation and architectural guide, see [`docs/DEVELOPER_SDK.md`](../docs/DEVELOPER_SDK.md).
