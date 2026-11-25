//! NovaOS System Monitor CLI / Entrypoint.

use std::env;
use std::process;
use nova_monitor::{
    CpuTelemetry, DiskTelemetry, FileSystemManager, MemoryTelemetry,
    NetworkTelemetry, ProcessManager, PulseUiState, SystemTelemetrySnapshot,
};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.contains(&String::from("--selftest")) {
        println!("[*] Launching NovaOS System Monitor Autonomous Qualification Suite...");
        if nova_monitor::run_nova_monitor_selftest() {
            println!("[PASS] NovaOS System Monitor (nova-monitor / Pulse) passed all qualification checks.");
            process::exit(0);
        } else {
            eprintln!("[FAIL] NovaOS System Monitor selftest failed!");
            process::exit(1);
        }
    }

    if args.contains(&String::from("--processes")) || args.contains(&String::from("-p")) {
        let mut mgr = ProcessManager::new();
        mgr.populate_default_telemetry();
        println!("NovaOS System Processes ({} active):", mgr.processes().len());
        println!("{:<24} {:<10} {:>8} {:>8} {:>12} {:<10}",
            "Process Name", "User", "% CPU", "PID", "Memory", "Priority");
        println!("{}", "-".repeat(76));
        for p in mgr.processes() {
            println!("{:<24} {:<10} {:>7.2}% {:>8} {:>12} {:<10}",
                p.name, p.user, p.cpu_percent, p.pid, p.formatted_memory(), p.priority.as_str());
        }
        return;
    }

    if args.contains(&String::from("--resources")) || args.contains(&String::from("-r")) {
        let cpu = CpuTelemetry::new(4);
        let mem = MemoryTelemetry::new();
        let net = NetworkTelemetry::new();
        let disk = DiskTelemetry::new();

        println!("NovaOS Hardware & Resource Gauges:");
        println!("  CPU: Overall {:.1}% (4 Cores Active)", cpu.overall_percent);
        println!("  Memory: {:.2} GB used ({:.1}%) of {:.2} GB",
            mem.used_bytes as f64 / 1e9, mem.memory_used_percent(), mem.total_bytes as f64 / 1e9);
        println!("  Network Rx/Tx: {} / {}",
            nova_monitor::format_rate(net.recv_rate_bps), nova_monitor::format_rate(net.send_rate_bps));
        println!("  Disk R/W: {} / {}",
            nova_monitor::format_rate(disk.read_rate_bps), nova_monitor::format_rate(disk.write_rate_bps));
        return;
    }

    if args.contains(&String::from("--filesystems")) || args.contains(&String::from("-f")) {
        let fs_mgr = FileSystemManager::new();
        println!("NovaOS Storage Volume Mounts:");
        for fs in fs_mgr.entries() {
            println!(" * {} on {} ({}) Total={} Avail={} Used={:.1}%",
                fs.device, fs.directory, fs.fs_type, fs.formatted_total(), fs.formatted_available(), fs.used_percent());
        }
        return;
    }

    if args.contains(&String::from("--report")) {
        let mut p_mgr = ProcessManager::new();
        p_mgr.populate_default_telemetry();
        let cpu = CpuTelemetry::new(4);
        let mem = MemoryTelemetry::new();
        let net = NetworkTelemetry::new();
        let disk = DiskTelemetry::new();
        let fs_mgr = FileSystemManager::new();
        let snap = SystemTelemetrySnapshot::new(
            p_mgr.processes().to_vec(),
            cpu,
            mem,
            net,
            disk,
            fs_mgr.entries().to_vec(),
        );
        println!("{}", snap.to_json());
        return;
    }

    if args.contains(&String::from("--version")) || args.contains(&String::from("-v")) {
        println!("Pulse Sovereign System & Resource Monitor v1.0.0 (NovaOS)");
        return;
    }

    // Default overview
    println!("Pulse System & Resource Monitor (NovaOS Desktop Edition)");
    let mut state = PulseUiState::new();
    state.proc_mgr.populate_default_telemetry();
    println!("State initialized with {} processes and 3 storage mounts.", state.proc_mgr.processes().len());
}
