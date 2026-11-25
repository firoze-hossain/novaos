//! userland/desktop/apps/nova-updates/src/main.rs
//!
//! # NovaOS Software Sources & Updates CLI Runner (nova-updates / Nexus)
//!
//! Sovereign package sources, mirror benchmark, driver manager, and update runner.

use std::env;
use std::process::exit;

use nova_updates::{run_nova_updates_selftest, NexusStudio};

fn print_usage() {
    println!("NovaOS Software & Updates Studio (nova-updates / Nexus) v1.0.0");
    println!("Synthesizing Ubuntu Software & Updates (7 tabs) & Software Updater");
    println!();
    println!("Usage:");
    println!("  nova-updates                      Display software sources & update telemetry");
    println!("  nova-updates --selftest           Execute autonomous qualification test suite");
    println!("  nova-updates --check              Check for pending software updates");
    println!("  nova-updates --mirrors            List configured mirrors and latency benchmarks");
    println!("  nova-updates --policy             Display automated update frequency and policies");
    println!("  nova-updates --drivers            List detected hardware devices and active DKMS drivers");
    println!("  nova-updates --keys               Display trusted cryptographic signing keys");
    println!("  nova-updates --sources            Export /etc/apt/sources.list and PPA configurations");
    println!("  nova-updates --info               Print updates architecture subsystems");
    println!("  nova-updates --version            Print version information");
    println!("  nova-updates --help               Display this reference manual");
    println!();
    println!("Subsystems:");
    println!("  1. Ubuntu Software: Official repository channels (main, universe, restricted, multiverse)");
    println!("  2. Mirrors: Multi-continental mirror database with dynamic latency speed benchmark");
    println!("  3. Other Software: 3rd-party PPAs, external deb/rpm feeds, deb822 export");
    println!("  4. Updates Policy: Automated checking schedules, unattended security patches, LTS upgrades");
    println!("  5. Authentication: GPG & Ed25519 signing keys trust store with cryptographic fingerprints");
    println!("  6. Additional Drivers: Hardware scan and proprietary DKMS driver alternative switching");
    println!("  7. Developer Options: Pre-release proposed channel and debug transaction logging");
    println!("  8. Software Updater: Interactive update dialog with 191.9 MB payload and 5-stage pipeline");
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let studio = NexusStudio::new();

    if args.len() > 1 {
        match args[1].as_str() {
            "--help" | "-h" => {
                print_usage();
                exit(0);
            }
            "--selftest" | "-t" => {
                println!("[*] Launching NovaOS Updates Autonomous Qualification Suite...");
                let ok = run_nova_updates_selftest();
                if ok {
                    println!("[PASS] NovaOS Updates (nova-updates / Nexus) passed all qualification checks.");
                    exit(0);
                } else {
                    eprintln!("[FAIL] One or more nova-updates tests failed.");
                    exit(1);
                }
            }
            "--check" => {
                println!("=== NovaOS Software Updater: Checking for Updates ===");
                println!("Status:   {}", studio.status_message);
                println!("Payload:  {}", studio.updater.total_download_display());
                for pkg in &studio.updater.pending_packages {
                    let mb = pkg.size_bytes as f64 / (1024.0 * 1024.0);
                    println!("  * {:<24} {:<16} -> {:<18} [{:.1} MB]", pkg.name, pkg.current_version, pkg.new_version, mb);
                }
                exit(0);
            }
            "--mirrors" => {
                println!("=== NovaOS Mirror Database & Latency Status ===");
                for m in &studio.mirrors.mirrors {
                    println!("  [{:<14}] {:<38} {:<6} {}", m.country, m.name, m.protocol.as_str(), m.url);
                }
                exit(0);
            }
            "--policy" => {
                println!("=== Automated Update Schedules & Security Policies ===");
                println!("Check Frequency:      {}", studio.policy.check_frequency.as_str());
                println!("Security Updates:     {}", studio.policy.security_updates.as_str());
                println!("Other Updates:        {}", studio.policy.other_updates.as_str());
                println!("Upgrade Track:        {}", studio.policy.release_upgrade.as_str());
                println!("NovaOS Pro / ESM:     Active until {}", studio.policy.esm_pro_expiry);
                exit(0);
            }
            "--drivers" => {
                println!("=== Hardware Devices & Additional Drivers ===");
                for dev in &studio.drivers.devices {
                    println!("\nDevice: {} - {}", dev.vendor, dev.model);
                    for drv in &dev.drivers {
                        println!("  [{}] {} ({}) [proprietary: {}]", if drv.active { "X" } else { " " }, drv.title, drv.version, drv.is_proprietary);
                    }
                }
                exit(0);
            }
            "--keys" => {
                println!("=== Cryptographic Signing Keys Trust Store ===");
                for k in &studio.auth_keys.keys {
                    println!("  Key ID:      {}", k.key_id);
                    println!("  Owner:       {}", k.owner_name);
                    println!("  Algorithm:   {} | Created: {}", k.algorithm.as_str(), k.created_date);
                    println!("  Fingerprint: {}\n", k.fingerprint);
                }
                exit(0);
            }
            "--sources" => {
                println!("{}", studio.channels.export_sources_list());
                println!("{}", studio.feeds.export_all_sources());
                exit(0);
            }
            "--info" | "-i" => {
                println!("NovaOS Software & Updates Subsystems:");
                println!("  * Channels:    Ubuntu & NovaOS Core (main, universe, restricted, multiverse)");
                println!("  * Mirrors:     Multi-continental mirror latency test and geo-routing");
                println!("  * PPAs:        Third-party package feeds with deb822 serialization");
                println!("  * Policies:    Automated check schedules & unattended security upgrades");
                println!("  * Trust Store: GPG & Ed25519 signing keys with fingerprint computation");
                println!("  * Drivers:     DKMS alternative driver switching (NVIDIA, Wi-Fi, Microcode)");
                println!("  * Updater:     Interactive Software Updater with 5-stage transaction pipeline");
                exit(0);
            }
            "--version" | "-v" => {
                println!("NovaOS Software & Updates (Nexus) 1.0.0");
                println!("Sovereign Software Sources, Mirror & Update Policy Studio");
                exit(0);
            }
            _ => {}
        }
    }

    println!("============================================================");
    println!(" NOVAOS SOFTWARE & UPDATES (NEXUS) - SOVEREIGN UPDATE STUDIO");
    println!("============================================================");
    println!("Status:      Active (Pure-Rust Cross-Platform & NovaOS Ring-3)");
    println!("Mirror:      {}", studio.mirrors.get_selected_mirror().name);
    println!("Channels:    5 channels (main, universe, restricted, multiverse, source)");
    println!("Feeds:       {} third-party repositories and PPAs configured", studio.feeds.feeds.len());
    println!("Keys:        {} trusted GPG & Ed25519 signing keys", studio.auth_keys.keys.len());
    println!("Pending:     {}", studio.updater.total_download_display());
    println!("Commands:    --selftest, --check, --mirrors, --policy, --drivers, --keys, --sources");
    println!("Run with --selftest to execute the autonomous qualification suite.");
}
