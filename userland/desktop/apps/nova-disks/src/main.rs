//! userland/desktop/apps/nova-disks/src/main.rs
//!
//! # NovaOS Disks & Storage CLI Runner (nova-disks / Stratum)
//!
//! Sovereign storage topology, partition manager, SMART diagnostics, and benchmark runner.

use std::env;
use std::process::exit;

use nova_disks::{run_nova_disks_selftest, StratumStudio};

fn print_usage() {
    println!("NovaOS Sovereign Disks Studio (nova-disks / Stratum) v1.0.0");
    println!("Synthesizing Ubuntu Disks (gnome-disk-utility) Volume Architecture");
    println!();
    println!("Usage:");
    println!("  nova-disks                        Display storage topology & volume status");
    println!("  nova-disks --selftest             Execute autonomous qualification test suite");
    println!("  nova-disks --list                 List detected storage devices, models, and sizes");
    println!("  nova-disks --partitions           Display partition map and filesystem mount points");
    println!("  nova-disks --smart                Display SMART health diagnostic attributes & thermals");
    println!("  nova-disks --benchmark            Run disk read/write throughput and access time benchmark");
    println!("  nova-disks --fstab                Generate static /etc/fstab filesystem configuration");
    println!("  nova-disks --info                 Print storage architecture subsystems");
    println!("  nova-disks --version              Print version information");
    println!("  nova-disks --help                 Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. Topology: Scans NVMe 2.0, SATA SSD, HDD, optical CD/DVD and virtual loop devices");
    println!("  2. Volumes Bar: Proportional visual partition map matching Ubuntu Disks");
    println!("  3. Partitioning: Format (Ext4, FAT32, Ext2, NTFS, exFAT), resize, label & UUID edit");
    println!("  4. Mounting: Dynamic SYS_MOUNT / SYS_UMOUNT and /media mount point generation");
    println!("  5. Diagnostics: Real-time SMART attributes, disk temperature, and self-tests");
    println!("  6. Benchmarking: Multi-sample transfer rate (MB/s) and latency (ms) sample curves");
    println!("  7. Image Tools: Raw disk image (.img/.iso) creation and loopback attachment");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let mut studio = StratumStudio::new();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_usage();
                exit(0);
            }
            "--selftest" | "-t" => {
                println!("[*] Launching NovaOS Disks Autonomous Qualification Suite...");
                let ok = run_nova_disks_selftest();
                if ok {
                    println!("[PASS] NovaOS Disks (nova-disks / Stratum) passed all qualification checks.");
                    exit(0);
                } else {
                    eprintln!("[FAIL] One or more nova-disks tests failed.");
                    exit(1);
                }
            }
            "--list" | "-l" => {
                println!("=== NovaOS Storage Topology & Detected Devices ===");
                for (i, d) in studio.drives.iter().enumerate() {
                    println!("  [{}] {:<20} {:<36} {:<14} {}", i, d.name, d.model, d.device_node, d.formatted_size());
                    println!("      Assessment: {} | Partitioning: {}", d.formatted_assessment(), d.partition_table.as_str());
                }
                exit(0);
            }
            "--partitions" | "-p" => {
                let drive = studio.selected_drive();
                println!("=== Volume Partitions for {} ({}) ===", drive.name, drive.device_node);
                for p in &drive.partitions {
                    let mnt = p.mount_point.as_deref().unwrap_or("[Unmounted]");
                    println!("  * {:<16} {:<12} {:<10} {:<14} {}", p.name, p.device_node, p.fs_type.short_name(), p.formatted_size(), mnt);
                    if !p.uuid.is_empty() {
                        println!("    UUID: {} | Type: {}", p.uuid, p.partition_type_label);
                    }
                }
                exit(0);
            }
            "--smart" | "-s" => {
                println!("=== S.M.A.R.T. Health Telemetry & Diagnostic Attributes ===");
                println!("Assessment:   {}", studio.smart.overall_assessment);
                println!("Power Cycles: {} | Power-On Hours: {}", studio.smart.power_cycles, studio.smart.power_on_hours);
                for attr in &studio.smart.attributes {
                    println!("  ID {:<3} {:<28} Raw: {:<6} Worst: {:<4} Status: {:?}", attr.id, attr.name, attr.raw_value, attr.worst, attr.status);
                }
                exit(0);
            }
            "--benchmark" | "-b" => {
                println!("=== Running Storage Transfer Rate & Latency Benchmark ===");
                studio.run_benchmark();
                println!("Read Average Rate:   {:.1} MB/s (Max: {:.1} MB/s, Min: {:.1} MB/s)",
                    studio.benchmark.read_avg_mbps,
                    studio.benchmark.read_max_mbps,
                    studio.benchmark.read_min_mbps
                );
                println!("Write Average Rate:  {:.1} MB/s", studio.benchmark.write_avg_mbps);
                println!("Average Access Time: {:.3} ms (Sample count: {})", studio.benchmark.access_time_ms, studio.benchmark.sample_count);
                exit(0);
            }
            "--fstab" => {
                let storage = nova_disks::StratumStorage::new("/etc");
                println!("{}", storage.generate_fstab(&studio));
                exit(0);
            }
            "--info" | "-i" => {
                println!("NovaOS Disks & Storage Subsystems:");
                println!("  * Topology:    NVMe PCIe 2.0, SATA 6 Gb/s, ATA/IDE, and CD/DVD Optical");
                println!("  * Volumes:     Proportional partition rendering with active root flags (*)");
                println!("  * Operations:  Mount, unmount, format, resize, and sector integrity check");
                println!("  * Diagnostics: SMART thermal monitoring, power cycles, and self-tests");
                println!("  * Benchmark:   Sequential read/write throughput and access time latency curves");
                println!("  * Engine:      Pure 2D software double-buffered vector rasterizer (zero-syscall)");
                exit(0);
            }
            "--version" | "-v" => {
                println!("NovaOS Disks (Stratum) 1.0.0");
                println!("Sovereign Disks, Partition Manager & Benchmark Studio");
                exit(0);
            }
            _ => {}
        }
    }

    println!("============================================================");
    println!("     NOVAOS DISKS (STRATUM) - SOVEREIGN DISK STUDIO         ");
    println!("============================================================");
    println!("Status:      Active (Pure-Rust Cross-Platform & NovaOS Ring-3)");
    println!("Devices:     {} storage devices detected", studio.drives.len());
    println!("Selected:    {} ({})", studio.selected_drive().name, studio.selected_drive().device_node);
    println!("Partitions:  {} partitions mapped", studio.selected_drive().partitions.len());
    println!("SMART:       {}", studio.smart.overall_assessment);
    println!("Commands:    --selftest, --list, --partitions, --smart, --benchmark, --fstab, --info");
    println!("Run with --selftest to execute the autonomous qualification suite.");
}
