//! userland/desktop/apps/nova-clock/src/main.rs
//!
//! # NovaOS Clock CLI & Self-Test Qualification Runner
//!
//! Provides the command-line interface, telemetry status printer,
//! and standalone self-test runner for `nova-clock`.

use std::env;
use std::process::exit;

use nova_clock::model::ClockHub;
use nova_clock::run_nova_clock_selftest;

fn print_usage() {
    println!("NovaOS Sovereign Clock (nova-clock / Chronos) v1.0.0");
    println!("Synthesizing Apple macOS World Map/Analog Cards & Ubuntu Yaru Timers/Stopwatch");
    println!();
    println!("Usage:");
    println!("  nova-clock                       Display active timekeeping telemetry");
    println!("  nova-clock --status              Print detailed status of all 4 clock subsystems");
    println!("  nova-clock --selftest            Execute autonomous qualification test suite");
    println!("  nova-clock --help                Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. World Clock: Solar terminator day/night equirectangular map & 24 global cities");
    println!("  2. Alarms: Weekday/weekend bitmasks, animated switches & 9-minute snooze");
    println!("  3. Stopwatch: Centisecond precision timer with fastest/slowest lap telemetry");
    println!("  4. Timers: Quick Start grid (1m..1h) & dynamic countdown progress bar");
}

fn print_status() {
    let mut hub = ClockHub::default();
    hub.sync_rtc(2026, 11, 24, 18, 0, 0, 360);

    println!("=== NOVAOS CLOCK (CHRONOS) TELEMETRY ===");
    println!("Local Base Time (Dhaka BST): {}", hub.world_mgr.city_time(&hub.world_mgr.pinned[0].city).format_hms());
    println!("Pinned World Cities ({}):", hub.world_mgr.pinned.len());
    for p in &hub.world_mgr.pinned {
        let t = hub.world_mgr.city_time(&p.city);
        let day_str = if hub.world_mgr.is_daytime(&p.city) { "DAY" } else { "NIGHT" };
        println!("  • {:<14} ({}) {:02}:{:02}:{:02} [{}]", p.city.name, p.city.tz_code, t.hour, t.minute, t.second, day_str);
    }
    println!("Alarms: {} active", hub.alarm_mgr.alarms.len());
    println!("Stopwatch State: {:?}", hub.stopwatch.state);
    println!("Timer State: {:?}", hub.timer.state);
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
                println!("[*] Launching NovaOS Clock Autonomous Qualification Suite...");
                let ok = run_nova_clock_selftest();
                if ok {
                    println!("[PASS] NovaOS Clock self-test passed (100% checks verified).");
                    exit(0);
                } else {
                    eprintln!("[FAIL] NovaOS Clock self-test encountered validation errors.");
                    exit(1);
                }
            }
            "--status" | "-s" => {
                print_status();
                exit(0);
            }
            _ => {
                eprintln!("Unknown argument '{}'. Use --help for usage.", args[1]);
                exit(1);
            }
        }
    }

    print_status();
}
