# NovaOS 2.0 Server Administration & Container Deployment Handbook

## 1. Overview of NovaOS 2.0 Server Edition

NovaOS 2.0 Server Edition is a high-density, headless cloud operating system engineered for enterprise virtualization, bare-metal data centers, and edge infrastructure. By eliminating GUI drivers and window compositing overhead, Server Edition boots in under 1.8 seconds, uses less than 32 MiB of baseline RAM, and dedicates all hardware throughput to compute, storage, and container workloads.

```
+---------------------------------------------------------------------------------------------------------+
|                                  NovaOS 2.0 Headless Server Platform                                    |
+---------------------------------------------------------------------------------------------------------+
| [novasshd] Pure Rust SSH-2 | [novamon] Prometheus Metrics:9100 | [novafw] Stateful Packet Firewall      |
+---------------------------------------------------------------------------------------------------------+
|                               Container Isolation & Governance Layer                                    |
|     cgroups v2 (CPU / Mem / PIDs)  |  6-Axis Namespaces (PID / Net / Mnt / IPC / UTS / User)           |
|     novadock / novactl Container Engine  |  Bridge Networking (nova-br0) & Port Forwarding              |
+---------------------------------------------------------------------------------------------------------+
|                                Service & Orchestration Framework (novad)                                |
|     Systemd-Style Unit File Parser  |  Dependency DAG Resolver  |  Automated Process Supervision        |
+---------------------------------------------------------------------------------------------------------+
|                                Enterprise Storage & Resiliency Subsystem                                |
|     NovaVolume Software RAID (0, 1, 10)  |  NovaFS CoW Snapshots  |  Automated A/B Update Rollback       |
+---------------------------------------------------------------------------------------------------------+
|                                  Kernel Cloud Profile & Networking                                      |
|     Enterprise TCP (Cubic & BBR)  |  Kernel WireGuard VPN  |  NUMA Work-Stealing Preemptive Scheduler   |
+---------------------------------------------------------------------------------------------------------+
```

---

## 2. Automated Server Provisioning with Cloud-Init (`novainit`)

NovaOS Server provides native `cloud-init` discovery on first boot, enabling instant deployment across OpenStack, Proxmox, AWS EC2, and VMware environments.

### Supported Datasources
- **ConfigDrive**: Scans ISO9660 or VFAT volumes mounted at `/dev/sr0` or labeled `config-2`.
- **NoCloud**: Reads metadata directly from `/var/lib/cloud/seed/user-data` and `meta-data`.
- **Network Metadata**: Queries cloud hypervisor link-local endpoints (`http://169.254.169.254/`).

### Configuration Example: `/var/lib/cloud/seed/user-data`
```yaml
#cloud-config
hostname: srv-edge-01.novaos.internal
fqdn: srv-edge-01.novaos.internal
manage_etc_hosts: true

users:
  - name: sysadmin
    gecos: System Administrator
    primary_group: admin
    groups: [sudo, wheel]
    shell: /bin/sh
    sudo: ALL=(ALL) NOPASSWD:ALL
    lock_passwd: false
    passwd: "$6$rounds=4096$salt$encryptedpasswordhash..."
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleAuthorizedKeySysadminOperator

network:
  version: 2
  ethernets:
    eth0:
      dhcp4: false
      addresses:
        - 192.168.10.50/24
      gateway4: 192.168.10.1
      nameservers:
        addresses: [1.1.1.1, 8.8.8.8]

write_files:
  - path: /etc/novad/system/api-worker.service
    permissions: '0644'
    content: |
      [Unit]
      Description=Edge API Worker Daemon
      After=network.target

      [Service]
      ExecStart=/usr/bin/api-worker --port 8080
      Restart=always
      MemoryMax=512M

runcmd:
  - novactl service enable api-worker
  - novactl service start api-worker
  - novactl firewall add --rule "ACCEPT tcp from any to any port 8080"
```

---

## 3. Service & Daemon Orchestration (`novad`)

NovaOS utilizes `novad`, a pure Rust service supervisor supporting systemd-compatible unit files.

### Service Unit Structure: `/etc/novad/system/example.service`
```ini
[Unit]
Description=High-Throughput Telemetry Agent
After=network.target storage.target
Requires=storage.target

[Service]
Type=simple
ExecStart=/usr/bin/telemetry-agent --config /etc/telemetry.json
ExecStop=/usr/bin/telemetry-agent --shutdown
Restart=always
RestartSec=5
User=daemon
Group=daemon
MemoryMax=256M
CPUWeight=100

[Install]
WantedBy=multi-user.target
```

### Service Management CLI (`novactl`)
```bash
# Start and stop services
novactl service start <unit-name>
novactl service stop <unit-name>
novactl service restart <unit-name>

# Check service status and health logs
novactl service status <unit-name>

# Enable or disable automated boot launch
novactl service enable <unit-name>
novactl service disable <unit-name>

# List all active and failed system services
novactl service list
```

---

## 4. Pure Rust Container Engine (`novactl container` / `novadock`)

NovaOS 2.0 embeds a native container runtime executing standard OCI container bundles using kernel isolation primitives:

### Isolation Architecture
1. **6-Axis Kernel Namespaces**:
   - `PID`: Isolated process table preventing containers from viewing or signaling external tasks.
   - `Mount`: Private filesystem root established via `pivot_root`.
   - `Net`: Independent network namespace with dedicated virtual ethernet (`veth`) interfaces.
   - `IPC`: Isolated POSIX message queues, semaphores, and shared memory handles.
   - `UTS`: Private container hostname.
   - `User`: Maps container root (UID 0) to an unprivileged host UID (e.g., UID 100000).
2. **Cgroups v2 Resource Governance**:
   - CPU throttling via `cpu.max` (bandwidth quotas).
   - Strict memory ceiling via `memory.max` with automated OOM termination.
   - Fork-bomb defense via `pids.max`.
   - Block I/O priority via `io.weight`.

### Container CLI Workflow
```bash
# Run an isolated container with resource limits and port mapping
novactl container run \
  --name web-app \
  --bundle /var/lib/containers/web-app \
  --cpus 2.0 \
  --memory 512M \
  --pids 64 \
  --port 80:8080

# List active containers
novactl container ps

# View container output logs
novactl container logs web-app

# Execute a shell command inside a running container namespace
novactl container exec web-app -- /bin/ls -la /app

# Stop and remove containers
novactl container stop web-app
novactl container rm web-app
```

---

## 5. Enterprise Networking, Firewall & WireGuard VPN

### 5.1 High-Performance TCP Stack
NovaOS integrates an enterprise cloud TCP stack with advanced congestion control:
- **Cubic**: High-throughput congestion control optimized for high-bandwidth, long-fat data center networks.
- **BBR (Bottleneck Bandwidth and RTT)**: Google BBR algorithm maximizes throughput and minimizes bufferbloat on variable-latency links.
- Selection via sysfs: `echo bbr > /sys/net/ipv4/tcp_congestion_control`.

### 5.2 Stateful Packet Firewall (`novafw`)
The `novafw` firewall filters IPv4 traffic with connection tracking (`conntrack`):
```bash
# Allow inbound SSH and HTTPS traffic
novactl firewall add --rule "ACCEPT tcp from any to any port 22"
novactl firewall add --rule "ACCEPT tcp from any to any port 443"

# Allow established and related connections
novactl firewall add --rule "ACCEPT state ESTABLISHED,RELATED"

# Block unauthorized inbound traffic
novactl firewall add --rule "DROP all from any to any"

# List active firewall rules
novactl firewall list
```

### 5.3 Native Kernel WireGuard VPN (`novavpn`)
NovaOS implements the Noise IK cryptographic protocol in pure Rust in ring-0:
```ini
# /etc/wireguard/wg0.conf
[Interface]
PrivateKey = aGVsbG93b3JsZHByaXZhdGVrZXlleGFtcGxlMTIzNDU2Nzg=
Address = 10.100.0.2/24
ListenPort = 51820

[Peer]
PublicKey = c2VydmVycHVibGlja2V5ZXhhbXBsZTEyMzQ1Njc4OTA=
Endpoint = 198.51.100.1:51820
AllowedIPs = 10.100.0.0/24
PersistentKeepalive = 25
```
```bash
# Manage WireGuard tunnel
novactl vpn up wg0
novactl vpn status wg0
novactl vpn down wg0
```

---

## 6. Enterprise Storage: Software RAID & Volume Pooling (`NovaVolume`)

The `NovaVolume` storage subsystem combines physical NVMe and SATA block devices into resilient virtual storage pools:

### Supported RAID Topologies
- **RAID 0 (Striping)**: Splits blocks across disks for maximum read/write performance.
- **RAID 1 (Mirroring)**: Full disk mirroring with zero data loss on single disk failure.
- **RAID 10 (Striped Mirrors)**: High-performance striping over mirrored pairs.

```bash
# Create a mirrored RAID 1 volume across two physical NVMe drives
novactl storage pool create --name data-pool --level raid1 --disks /dev/nvme0n1,/dev/nvme1n1

# Check RAID status and rebuild progression
novactl storage pool status data-pool

# Replace a failed disk and trigger background rebuild
novactl storage pool replace --pool data-pool --failed /dev/nvme0n1 --new /dev/nvme2n1
```

---

## 7. Automated Updates with Atomic A/B Rollback (`novaupdate`)

To ensure 99.999% uptime, NovaOS Server utilizes dual boot slots:
- **Slot A** (Active Rootfs).
- **Slot B** (Alternate Update Slot).

```
[Fetch Signed Update] --> [Stage in Slot B] --> [Update Bootloader Slot]
                                                        |
                                                        v
                                                  [Reboot System]
                                                        |
                              +-------------------------+-------------------------+
                              | Post-Boot Health Self-Tests                      |
                              v                                                   v
                         [TESTS PASS]                                       [TESTS FAIL]
                              |                                                   |
                      Commit Slot B (Active)                         Hardware Watchdog Reset
                                                                     Fallback to Slot A (Rollback)
```

### Administration CLI
```bash
# Check for updates and verify cryptographic signatures
novactl update check

# Stage update to alternate slot
novactl update stage --version 2.1.0

# Commit slot and schedule reboot
novactl update commit

# Manual rollback to previous slot
novactl update rollback
```

---

## 8. Prometheus Telemetry & Monitoring (`novamon`)

NovaOS embeds `novamon`, an integrated Prometheus exporter serving metrics over HTTP on port 9100 (`http://<server-ip>:9100/metrics`):

### Exported Metrics Samples
```promql
# CPU utilization per core
node_cpu_seconds_total{cpu="0",mode="idle"} 45210.12
node_cpu_seconds_total{cpu="0",mode="system"} 1284.50

# Memory allocation breakdown
node_memory_MemTotal_bytes 34359738368
node_memory_MemFree_bytes 28456128512
node_memory_Cached_bytes 4125890560

# NVMe disk IOPS and throughput
node_disk_read_bytes_total{device="nvme0n1"} 148920156160
node_disk_written_bytes_total{device="nvme0n1"} 84210954240

# Container resource consumption
novadock_container_cpu_usage_seconds{name="web-app"} 412.30
novadock_container_memory_bytes{name="web-app"} 245129000
```
