//! userland/desktop/apps/nova-cam/src/main.rs
//!
//! # NovaOS Camera CLI & Self-Test Qualification Runner
//!
//! Provides the command-line interface, telemetry status printer,
//! and standalone self-test runner for `nova-cam`.

use std::env;
use std::process::exit;

use nova_cam::model::{create_platform_camera_engine, CameraHub};
use nova_cam::run_nova_cam_selftest;

fn print_usage() {
    println!("NovaOS Sovereign Camera (nova-cam / Iris) v1.0.0");
    println!("Synthesizing Ubuntu Camera (GNOME Snapshot) with Sovereign Ring-3 IPC");
    println!();
    println!("Usage:");
    println!("  nova-cam                         Display active camera viewfinder & telemetry");
    println!("  nova-cam --status                Print detailed status of camera subsystems");
    println!("  nova-cam --selftest              Execute autonomous qualification test suite");
    println!("  nova-cam --list-devices          List all detected built-in and external webcams");
    println!("  nova-cam --photo [FILE]          Capture high-resolution snapshot to PPM image");
    println!("  nova-cam --record [SECS]         Record video clip for specified duration");
    println!("  nova-cam --help                  Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. Hardware Probing: Automatic built-in laptop and external USB webcam discovery");
    println!("  2. Dynamic Resolution: 480p SD, 720p HD, 1080p FHD, and 4K UHD negotiation");
    println!("  3. Viewfinder Blitter: Aspect-ratio letterbox scaling with composition grids");
    println!("  4. Shutter Controls: Circular shutter button, countdown timers, and screen flash");
    println!("  5. Video Recording: Live session timer, duration tracking, and stream sink");
    println!("  6. Media Library: Integrated gallery management for pictures and videos");
}

fn print_status() {
    let hub = CameraHub::new();
    let backend = create_platform_camera_engine();

    println!("=== NOVAOS CAMERA (IRIS) TELEMETRY ===");
    println!("Active Backend:       {}", backend.backend_name());
    println!("Connected Devices:    {} detected", hub.registry.devices.len());
    if let Some(active) = hub.registry.active_device() {
        println!("Active Camera Device: {} ({})", active.name, active.kind.label());
        println!("Device Node:          {}", active.device_node);
    }
    println!("Active Resolution:    {} [{}]", hub.config.resolution.label(), hub.config.resolution.aspect_ratio_str());
    println!("Camera Mode:          {}", hub.config.mode.label());
    println!("Selfie Mirror Mode:   {}", if hub.config.mirror_mode { "ON" } else { "OFF" });
    println!("Composition Grid:     {}", hub.config.grid.label());
    println!("Timer Duration:       {}", hub.config.timer.label());
    println!("Media Gallery Count:  {} items", hub.gallery.items.len());
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_usage();
                exit(0);
            }
            "--selftest" => {
                println!("[*] Launching NovaOS Camera Autonomous Qualification Suite...");
                let ok = run_nova_cam_selftest();
                if ok {
                    println!("[PASS] NovaOS Camera (nova-cam / Iris) passed all qualification checks.");
                    exit(0);
                } else {
                    eprintln!("[FAIL] One or more nova-cam tests failed.");
                    exit(1);
                }
            }
            "--status" => {
                print_status();
                exit(0);
            }
            "--list-devices" => {
                let mut backend = create_platform_camera_engine();
                let devices = backend.probe_devices();
                println!("Detected Camera Hardware ({} total):", devices.len());
                for (i, dev) in devices.iter().enumerate() {
                    println!("  [{}] {} ({}) on {}", i + 1, dev.name, dev.kind.label(), dev.device_node);
                }
                exit(0);
            }
            _ => {}
        }
    }

    print_status();
}
