//! NovaOS Firmware Studio CLI / Entrypoint.

use std::env;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.contains(&String::from("--selftest")) {
        println!("[*] Launching NovaOS Firmware Autonomous Qualification Suite...");
        if nova_firmware::run_nova_firmware_selftest() {
            println!("[PASS] NovaOS Firmware (nova-firmware / Aegis) passed all qualification checks.");
            process::exit(0);
        } else {
            eprintln!("[FAIL] NovaOS Firmware selftest failed!");
            process::exit(1);
        }
    }

    let inv = nova_firmware::HardwareInventory::new();

    if args.contains(&String::from("--devices")) || args.contains(&String::from("--list")) {
        println!("NovaOS Firmware Inventory ({} devices):", inv.devices.len());
        for dev in &inv.devices {
            let update_status = if dev.has_update() { " [UPDATE AVAILABLE]" } else { " [OK]" };
            println!(" * {} ({}): Current={:?}{}",
                dev.name,
                dev.vendor,
                dev.current_version.as_deref().unwrap_or("none"),
                update_status
            );
        }
        return;
    }

    if args.contains(&String::from("--check")) {
        let pending: Vec<_> = inv.devices.iter().filter(|d| d.has_update()).collect();
        println!("Pending Firmware Updates: {} available", pending.len());
        for dev in pending {
            println!(" * {}: {:?} -> {:?}", dev.name, dev.current_version, dev.available_version);
        }
        return;
    }

    if args.contains(&String::from("--info")) {
        println!("System Hardware Firmware Components:");
        for dev in &inv.devices {
            println!("ID: {}", dev.id);
            println!("  Name: {}", dev.name);
            println!("  Vendor: {}", dev.vendor);
            println!("  GUIDs: {:?}", dev.guids);
            println!("  Checksum: {:?}", dev.checksum);
        }
        return;
    }

    if args.contains(&String::from("--version")) || args.contains(&String::from("-v")) {
        println!("NovaOS Firmware Studio v1.0.0 (Aegis Edition)");
        return;
    }

    println!("Aegis Firmware Studio (NovaOS Edition)");
    println!("Usage: nova-firmware [--selftest|--devices|--check|--info|--version]");
}
