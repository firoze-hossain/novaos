//! sdk/novaos-sdk/src/lib.rs
//!
//! # Official NovaOS Developer SDK & Application Framework (novaos-sdk)
//!
//! High-productivity, pure Rust development framework for building native NovaOS
//! graphical (`.app` bundles) and command-line applications.
//!
//! ## Subsystems:
//! - `app`: Application lifecycle, window configuration, and reactive event loops.
//! - `ui`: Retained-mode declarative UI toolkit with Flexbox layout, vector rasterization, and acrylic themes.
//! - `ipc`: Inter-Process Communication channels, pub/sub event bus, and shared memory.
//! - `fs`: Capability-safe sandboxed filesystem access and standard paths.
//! - `sys`: System telemetry, process inspection, and desktop notifications.
//! - `emulator`: Host-side window simulator for rapid <1s iteration.
//! - `prelude`: Convenient glob re-exports.

pub mod app;
pub mod emulator;
pub mod fs;
pub mod ipc;
pub mod prelude;
pub mod sys;
pub mod ui;

/// SDK version constant.
pub const SDK_VERSION: &str = "2.0.0";

/// Master self-test suite validating all 7 SDK architectural quadrants.
pub fn run_sdk_selftest() -> bool {
    use prelude::*;

    // 1. App Builder & Lifecycle
    let mut app = App::builder("com.novaos.test", "TestApp")
        .window_size(400, 300)
        .theme(ThemeMode::Dark)
        .build();

    if app.config.width != 400 || app.config.height != 300 {
        return false;
    }

    // 2. UI Layout & View Hierarchy
    let main_view = app.add_view("main_card", Rect::new(20, 20, 360, 260));
    let btn = Button::new("Submit").width(120).height(36).accent(true);
    let btn_node = btn.to_layout_node(10);
    if btn_node.style.height != Dimension::Points(36) {
        return false;
    }

    // 3. Reactive State Management
    let mut counter = State::new(0);
    counter.subscribe(main_view);
    counter.set(42, &mut app.scene);
    if counter.get() != 42 || !app.scene.get_node(main_view).unwrap().dirty_paint {
        return false;
    }

    // 4. Sandboxed Filesystem & Security Boundaries
    let tmp_root = std::env::temp_dir().join("novaos_sdk_test_fs");
    let _ = std::fs::create_dir_all(&tmp_root);
    let sfs = SandboxedFs::new(&tmp_root);

    // Verify valid write and read
    if sfs.write_file("config.json", b"{\"theme\":\"dark\"}").is_err() {
        return false;
    }
    if sfs.read_to_string("config.json").unwrap() != "{\"theme\":\"dark\"}" {
        return false;
    }

    // Verify traversal rejection
    if sfs.resolve("../../etc/passwd").is_ok() {
        return false; // Must reject path traversal
    }
    let _ = std::fs::remove_dir_all(&tmp_root);

    // 5. IPC Channels & Pub/Sub
    let mut chan = MessageChannel::new(100);
    if chan.send(101, 1, b"ping").is_err() {
        return false;
    }
    if chan.pending_outbox() != 1 {
        return false;
    }
    let out = chan.take_outbox();
    if out.len() != 1 || out[0].payload_as_str().unwrap() != "ping" {
        return false;
    }

    let mut bus = EventBus::new();
    let received = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let r_clone = received.clone();
    bus.subscribe("network.online", move |_| {
        r_clone.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    bus.publish("network.online", b"eth0");
    if !received.load(std::sync::atomic::Ordering::SeqCst) {
        return false;
    }

    // 6. System & Process Telemetry
    let sys = SystemInfo::query();
    if sys.os_name != "NovaOS" || sys.total_memory_mb == 0 {
        return false;
    }
    let proc = ProcessInfo::current();
    if proc.pid == 0 {
        return false;
    }

    // 7. Simulated Window Rendering & Export
    app.step_frame();
    if app.window.frame_count != 1 {
        return false;
    }
    let ppm = app.window.export_ppm();
    if !ppm.starts_with("P3\n400 300\n255\n") {
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sdk_full_selftest() {
        assert!(run_sdk_selftest(), "NovaOS SDK full self-test passed");
    }
}
