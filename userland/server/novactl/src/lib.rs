//! userland/server/novactl/src/lib.rs
//!
//! NovaOS 2.0 (Month 3 - Server & Containers): Container Runtime Engine Library
//!
//! Exposes the pure Rust container runtime API:
//!   - OCI Runtime Specification v1.0.2 parser & generator
//!   - Control Groups v2 resource limits controller (memory, cpu, pids, io)
//!   - Linux-grade Namespaces (PID, Mount, Net, IPC, UTS, User)
//!   - Virtual network bridge and IPAM engine
//!   - Image store and rootfs layer manager
//!   - Complete sub-50ms container lifecycle engine

pub mod cgroup;
pub mod container;
pub mod namespace;
pub mod network;
pub mod oci;
pub mod storage;
pub mod wireguard;
pub mod service;

pub use cgroup::{CgroupController, ContainerMetrics};
pub use container::{Container, ContainerConfig, ContainerEngine, ContainerStatus};
pub use namespace::{
    NamespaceConfig, CLONE_NEWCGROUP, CLONE_NEWIPC, CLONE_NEWNET, CLONE_NEWNS, CLONE_NEWPID,
    CLONE_NEWUSER, CLONE_NEWUTS,
};
pub use network::{ContainerNetworkConfig, NetworkManager, PortMapping};
pub use oci::{OciLinuxResources, OciMount, OciProcess, OciRoot, OciSpec};
pub use storage::{
    DiskHealth, ImageInfo, PhysicalDiskInfo, RaidLevel, StorageManager, VolumeManager,
    VolumeStatus, VolumeSuperblock,
};

use std::time::Instant;

/// Comprehensive end-to-end self-test for `novactl`.
///
/// Verifies:
///   1. OCI Spec parse and serialize round-trip.
///   2. Image store resolution and rootfs verification.
///   3. NetworkManager IPAM allocation and configuration.
///   4. Namespace chroot boundary protection against path traversal.
///   5. Control Groups v2 limit configuration and process attachment.
///   6. Container sub-50ms launch guarantee.
///   7. Container lifecycle (run, list, inspect, logs, stats, exec, stop, restart, rm).
pub fn run_novactl_selftest() -> bool {
    println!("============================================================");
    println!("     NovaOS Container Runtime Engine (novactl) Self-Test     ");
    println!("============================================================");

    // 1. Test OCI Spec serialization / deserialization
    print!("[TEST 1/8] OCI Spec v1.0.2 Serialization & Parsing... ");
    let mut spec = OciSpec::default();
    spec.hostname = "selftest-host".to_string();
    spec.process.args = vec!["/bin/sh".to_string(), "-c".to_string(), "uptime".to_string()];
    spec.linux_resources.memory_max_bytes = Some(256 * 1024 * 1024);
    spec.linux_resources.cpu_quota_usec = Some(50_000);
    spec.linux_resources.pids_max = Some(64);

    let json = spec.to_json();
    let parsed = match OciSpec::parse_json(&json) {
        Ok(s) => s,
        Err(e) => {
            println!("FAILED: {}", e);
            return false;
        }
    };
    assert_eq!(parsed.hostname, "selftest-host");
    assert_eq!(parsed.process.args, vec!["/bin/sh", "-c", "uptime"]);
    assert_eq!(parsed.linux_resources.memory_max_bytes, Some(256 * 1024 * 1024));
    assert_eq!(parsed.linux_resources.cpu_quota_usec, Some(50_000));
    assert_eq!(parsed.linux_resources.pids_max, Some(64));
    println!("PASSED");

    // 2. Test StorageManager and standard OCI image resolution
    print!("[TEST 2/8] StorageManager OCI Image Resolution... ");
    let storage = StorageManager::new();
    let images = storage.list_images();
    assert!(!images.is_empty(), "Image store should contain seeded images");
    let has_alpine = images.iter().any(|i| i.name.contains("alpine"));
    assert!(has_alpine, "Should find alpine image");

    let alpine_rootfs = match storage.resolve_image_rootfs("alpine:latest") {
        Ok(p) => p,
        Err(e) => {
            println!("FAILED: {}", e);
            return false;
        }
    };
    assert!(alpine_rootfs.exists(), "Resolved rootfs path must exist");
    println!("PASSED (found {} local images)", images.len());

    // 3. Test NetworkManager IPAM
    print!("[TEST 3/8] NetworkManager IPAM & veth pair generation... ");
    let mut net = NetworkManager::new();
    let ip1 = net.allocate_ip();
    let ip2 = net.allocate_ip();
    assert_eq!(ip1.to_string(), "10.88.0.2");
    assert_eq!(ip2.to_string(), "10.88.0.3");
    let net_cfg = net.setup_container_network("c1234567890a", "container1", &[(8080, 80)]);
    assert_eq!(net_cfg.veth_host, "vethc123456");
    assert_eq!(net_cfg.veth_container, "eth0");
    assert_eq!(net_cfg.gateway.to_string(), "10.88.0.1");
    assert_eq!(net_cfg.port_mappings.len(), 1);
    assert_eq!(net_cfg.port_mappings[0].host_port, 8080);
    assert_eq!(net_cfg.port_mappings[0].container_port, 80);
    let resolv = net.generate_resolv_conf();
    assert!(resolv.contains("nameserver 10.88.0.1"));
    let hosts = net.generate_hosts(&net_cfg);
    assert!(hosts.contains("container1"));
    println!("PASSED (bridge 10.88.0.1 -> allocated 10.88.0.2, 10.88.0.3)");

    // 4. Test Namespace chroot jail path traversal prevention
    print!("[TEST 4/8] Namespace Security & Path Traversal Prevention... ");
    let ns = NamespaceConfig::new("jail-test", &alpine_rootfs);
    assert!(
        (ns.clone_flags() & CLONE_NEWPID) != 0
            && (ns.clone_flags() & CLONE_NEWNS) != 0
            && (ns.clone_flags() & CLONE_NEWNET) != 0
            && (ns.clone_flags() & CLONE_NEWIPC) != 0
            && (ns.clone_flags() & CLONE_NEWUTS) != 0
            && (ns.clone_flags() & CLONE_NEWUSER) != 0,
        "All 6 namespaces must be enabled in clone_flags"
    );
    // Safe lookup
    let safe_res = ns.resolve_in_jail("/etc/os-release");
    assert!(safe_res.is_ok());
    // Escape attempt lookup
    let escape_res = ns.resolve_in_jail("../../../../../../../etc/shadow");
    assert!(escape_res.is_ok());
    let path = escape_res.unwrap();
    assert!(path.starts_with(&alpine_rootfs), "Path traversal must remain bounded within rootfs jail");
    println!("PASSED (6 Linux namespaces verified; jail boundary enforced)");

    // 5. Test CgroupController v2 resource governance
    print!("[TEST 5/8] Cgroups v2 Resource Governance... ");
    let cgroup = CgroupController::new("test-cgroup-001");
    let setup_res = cgroup.setup(
        Some(512 * 1024 * 1024),
        Some(50_000),
        100_000,
        Some(100),
        Some("8:0 rbps=10485760 wbps=10485760"),
    );
    assert!(setup_res.is_ok(), "Cgroup setup failed: {:?}", setup_res.err());
    assert!(cgroup.attach_pid(42).is_ok());
    let metrics = cgroup.read_metrics();
    assert_eq!(metrics.memory_max_bytes, 512 * 1024 * 1024);
    assert_eq!(metrics.pids_max, 100);
    assert!(cgroup.freeze().is_ok());
    assert!(cgroup.unfreeze().is_ok());
    assert!(cgroup.cleanup().is_ok());
    println!("PASSED (memory.max, cpu.max, pids.max, io.max, freeze/unfreeze)");

    // 6. Test Container Engine Sub-50ms Launch
    print!("[TEST 6/8] ContainerEngine Sub-50ms Launch Performance... ");
    let mut engine = ContainerEngine::new();
    let config = ContainerConfig {
        name: "perf-container".to_string(),
        image: "alpine:latest".to_string(),
        memory_limit_bytes: Some(256 * 1024 * 1024),
        cpu_quota_usec: Some(25_000),
        pids_limit: Some(32),
        ports: vec![(8080, 80)],
        ..Default::default()
    };

    let t0 = Instant::now();
    let container_id = match engine.run(config) {
        Ok(id) => id,
        Err(e) => {
            println!("FAILED: {}", e);
            return false;
        }
    };
    let launch_duration = t0.elapsed();
    assert!(
        launch_duration.as_millis() < 50,
        "Container launch took {:?}, exceeding 50ms requirement!",
        launch_duration
    );
    let container = engine.get(&container_id).expect("Container should be registered");
    assert_eq!(container.status, ContainerStatus::Running);
    assert!(container.pid.is_some());
    assert!(!container.ip_address.is_empty());
    println!("PASSED (Launched in {:?} < 50ms threshold)", launch_duration);

    // 7. Test Container Lifecycle & Operations (logs, exec, stats)
    print!("[TEST 7/8] Container Lifecycle Operations (ps, logs, exec, stats)... ");
    let list_running = engine.list(false);
    assert_eq!(list_running.len(), 1);
    assert_eq!(list_running[0].name, "perf-container");

    let logs = engine.logs("perf-container", None).unwrap();
    assert!(logs.contains("perf-container"));
    assert!(logs.contains("started"));

    let exec_out = engine.exec("perf-container", &["cat".to_string(), "/etc/hostname".to_string()]).unwrap();
    assert!(exec_out.contains("perf-container"));

    let stats = engine.stats("perf-container").unwrap();
    assert_eq!(stats.memory_max_bytes, 256 * 1024 * 1024);
    assert_eq!(stats.pids_max, 32);

    // Stop and restart
    assert!(engine.stop("perf-container").is_ok());
    assert_eq!(engine.get("perf-container").unwrap().status, ContainerStatus::Stopped);
    assert_eq!(engine.list(false).len(), 0);
    assert_eq!(engine.list(true).len(), 1);

    assert!(engine.start("perf-container").is_ok());
    assert_eq!(engine.get("perf-container").unwrap().status, ContainerStatus::Running);
    assert_eq!(engine.list(false).len(), 1);

    assert!(engine.restart("perf-container").is_ok());
    assert_eq!(engine.get("perf-container").unwrap().status, ContainerStatus::Running);
    println!("PASSED (ps, logs, exec, stats, stop, start, restart verified)");

    // 8. Test Clean Container Removal
    print!("[TEST 8/9] Container Teardown & Resource Deallocation... ");
    assert!(engine.stop("perf-container").is_ok());
    let rm_res = engine.rm("perf-container", false);
    assert!(rm_res.is_ok(), "Container removal failed: {:?}", rm_res.err());
    assert!(engine.get("perf-container").is_none());
    assert_eq!(engine.list(true).len(), 0);
    println!("PASSED");

    // 9. Test Native In-Kernel WireGuard VPN Configuration Subsystem (novactl wg)
    print!("[TEST 9/9] WireGuard VPN CLI (genkey, pubkey, set, show)... ");
    let privkey = wireguard::generate_private_key();
    let b64_priv = wireguard::base64_encode_32(&privkey);
    assert_eq!(b64_priv.len(), 44);
    let decoded_priv = wireguard::base64_decode_32(&b64_priv).expect("Base64 decode failed");
    assert_eq!(decoded_priv, privkey);

    let pubkey = wireguard::derive_public_key(&privkey);
    let b64_pub = wireguard::base64_encode_32(&pubkey);
    assert_eq!(b64_pub.len(), 44);

    let set_args = vec![
        "set".to_string(),
        "wg0".to_string(),
        "private-key".to_string(),
        b64_priv,
        "listen-port".to_string(),
        "51820".to_string(),
        "peer".to_string(),
        b64_pub,
        "endpoint".to_string(),
        "198.51.100.2:51820".to_string(),
        "allowed-ips".to_string(),
        "10.0.0.2/32, 192.168.1.0/24".to_string(),
        "persistent-keepalive".to_string(),
        "25".to_string(),
    ];
    assert!(wireguard::execute_wg_command(&set_args).is_ok());
    assert!(wireguard::execute_wg_command(&["show".to_string(), "wg0".to_string()]).is_ok());
    println!("PASSED");

    // 10. Service Orchestrator (novad) integration
    print!("[TEST 10/11] Service Orchestrator (novad) CLI Integration... ");
    let mut svc_mgr = novad::ServiceManager::new();
    assert!(svc_mgr.boot_target("default.target").is_ok());
    assert_eq!(svc_mgr.status_unit("novasshd.service").unwrap().state, novad::UnitState::Active);
    assert!(svc_mgr.stop_unit("novasshd.service").is_ok());
    assert_eq!(svc_mgr.status_unit("novasshd.service").unwrap().state, novad::UnitState::Inactive);
    println!("PASSED");

    // 11. Enterprise Software RAID & Dynamic Volume Pooling (NovaVolume)
    print!("[TEST 11/11] Enterprise Software RAID & Dynamic Volume Pooling (NovaVolume)... ");
    let mut vol_mgr = VolumeManager::new();
    let r1_vol = vol_mgr.create_volume(
        "selftest-r1",
        RaidLevel::Raid1,
        &["/dev/nvme0n1".to_string(), "/dev/nvme1n1".to_string()],
        &["/dev/nvme2n1".to_string()],
        Some(64),
    ).expect("Failed to create RAID-1 volume");

    assert_eq!(r1_vol.active_members, 2);
    assert_eq!(r1_vol.total_members, 3);
    assert_eq!(r1_vol.state, VolumeStatus::Healthy);

    // Online capacity expansion
    let initial_cap = r1_vol.volume_sectors;
    let ext_vol = vol_mgr.extend_volume("selftest-r1", 4096).expect("Failed to extend volume");
    assert_eq!(ext_vol.volume_sectors, initial_cap + 4096);
    assert!(ext_vol.generation > r1_vol.generation);

    // Drive failure injection -> automatic hot-spare takeover
    let failed_vol = vol_mgr.fail_disk("selftest-r1", "/dev/nvme0n1").expect("Failed to inject drive failure");
    assert_eq!(failed_vol.state, VolumeStatus::Rebuilding);
    assert_eq!(failed_vol.members[0].status, DiskHealth::Failed);
    assert_eq!(failed_vol.members[2].status, DiskHealth::Rebuilding);

    // Background rebuild progression to 100%
    let _ = vol_mgr.step_rebuild("selftest-r1");
    let rebuilt_vol = vol_mgr.step_rebuild("selftest-r1").expect("Failed to step rebuild");
    assert_eq!(rebuilt_vol.state, VolumeStatus::Healthy);
    assert_eq!(rebuilt_vol.rebuild_progress, 100.0);
    assert_eq!(rebuilt_vol.members[2].status, DiskHealth::Online);

    // RAID-10 volume creation
    let r10_vol = vol_mgr.create_volume(
        "selftest-r10",
        RaidLevel::Raid10,
        &["/dev/sda".to_string(), "/dev/sdb".to_string(), "/dev/sdc".to_string(), "/dev/sdd".to_string()],
        &[],
        None,
    ).expect("Failed to create RAID-10 volume");
    assert_eq!(r10_vol.active_members, 4);
    assert_eq!(r10_vol.state, VolumeStatus::Healthy);

    // CLI dispatch verification
    storage::handle_volume_cmd(&["list".to_string()]);
    storage::handle_volume_cmd(&["status".to_string(), "selftest-r1".to_string()]);

    // Teardown
    assert!(vol_mgr.delete_volume("selftest-r1").is_ok());
    assert!(vol_mgr.delete_volume("selftest-r10").is_ok());
    println!("PASSED");

    println!("============================================================");
    println!("  ALL 11/11 NOVACTL ENGINE, STORAGE, WIREGUARD & SERVICE CHECKS PASSED ");
    println!("============================================================");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_novactl_suite() {
        assert!(run_novactl_selftest());
    }
}
