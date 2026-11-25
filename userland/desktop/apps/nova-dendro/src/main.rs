//! NovaOS Disk Usage Analyzer CLI & Launcher.

use std::env;
use std::process;
use nova_dendro::{
    create_default_devices, create_default_home_tree,
    DiskReportGenerator, DiskScanner, SunburstLayoutEngine, TreemapLayoutEngine,
};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.contains(&String::from("--selftest")) {
        println!("[*] Launching NovaOS Disk Usage Analyzer Autonomous Qualification Suite...");
        if nova_dendro::run_nova_dendro_selftest() {
            println!("[PASS] NovaOS Disk Usage Analyzer (nova-dendro / Dendro) passed all qualification checks.");
            process::exit(0);
        } else {
            eprintln!("[FAIL] NovaOS Disk Usage Analyzer selftest failed!");
            process::exit(1);
        }
    }

    if args.contains(&String::from("--devices")) || args.contains(&String::from("-d")) {
        let devices = create_default_devices();
        println!("NovaOS Storage Volumes & Mounts ({} detected):", devices.len());
        println!("{:<18} {:<24} {:<12} {:<12} {:<10}",
            "Volume Name", "Mount Point", "Capacity", "Available", "Status");
        println!("{}", "-".repeat(76));
        for dev in devices {
            let status = if dev.is_mounted {
                format!("{:.1}% used", dev.used_percent)
            } else {
                "Unmounted".to_string()
            };
            println!("{:<18} {:<24} {:<12} {:<12} {:<10}",
                dev.name, dev.mount_point, dev.format_total(), dev.format_available(), status);
        }
        return;
    }

    if args.contains(&String::from("--rings")) || args.contains(&String::from("-r")) {
        let tree = create_default_home_tree();
        let engine = SunburstLayoutEngine::new(36.0, 28.0, 3);
        let sectors = engine.compute_layout(&tree);
        println!("NovaOS Sunburst Radial Rings ({} sectors):", sectors.len());
        for sec in sectors.iter().take(12) {
            println!(" * L{} [{:.0}..{:.0} px] Arc: {:>5.1}°..{:>5.1}° | {:<16} ({})",
                sec.depth, sec.inner_r, sec.outer_r,
                sec.start_angle_rad.to_degrees(), sec.end_angle_rad.to_degrees(),
                sec.name, nova_dendro::format_bytes(sec.size_bytes));
        }
        return;
    }

    if args.contains(&String::from("--treemap")) || args.contains(&String::from("-t")) {
        let tree = create_default_home_tree();
        let engine = TreemapLayoutEngine::new(2, 2.0);
        let rects = engine.compute_layout(&tree, 0.0, 0.0, 600.0, 400.0);
        println!("NovaOS Squarified Treemap ({} partitions):", rects.len());
        for r in rects.iter().take(10) {
            println!(" * L{} ({:.0}, {:.0}, {:.0}x{:.0}) | {:<16} ({})",
                r.depth, r.x, r.y, r.w, r.h, r.name, nova_dendro::format_bytes(r.size_bytes));
        }
        return;
    }

    if let Some(pos) = args.iter().position(|a| a == "--scan") {
        if let Some(path) = args.get(pos + 1) {
            println!("Scanning path: {}", path);
            let scanner = DiskScanner::new(4);
            let root = scanner.scan_path(path);
            println!("Root: {} (Size: {}, Items: {})", root.name, root.format_size(), root.items_count);
            for child in root.children.iter().take(10) {
                println!("  {:>3.0}%  {:<20}  {}", child.percentage_of_parent, child.name, child.format_size());
            }
            return;
        }
    }

    if let Some(pos) = args.iter().position(|a| a == "--report") {
        if let Some(path) = args.get(pos + 1) {
            let scanner = DiskScanner::new(3);
            let root = scanner.scan_path(path);
            println!("{}", DiskReportGenerator::to_json(&root));
            return;
        }
    }

    if args.contains(&String::from("--version")) || args.contains(&String::from("-v")) {
        println!("Dendro Sovereign Disk Usage Analyzer v1.0.0 (NovaOS)");
        return;
    }

    println!("Dendro Disk Usage Analyzer (NovaOS Desktop Edition)");
    let devices = create_default_devices();
    println!("Detected {} storage locations. Run with --devices, --rings, or --scan <dir>.", devices.len());
}
