# NovaOS 2.0: Cloud-Init & Headless Server Automated Provisioning

## 1. Executive Summary

NovaOS 2.0 Server Edition features native, enterprise-grade cloud-init automation written in 100% pure safe Rust (`userland/server/novainit`). Operating before standard network services and SSH daemons initialize, `novainit` inspects local hardware and hypervisor channels to automatically discover cloud metadata, network layouts, and `#cloud-config` manifests.

Without requiring manual intervention or interactive installers, NovaOS automatically configures static or DHCP network interfaces, sets DNS resolvers and search domains, creates non-root administrative users, installs OpenSSH authorized keys with strict POSIX permissions (`0700`/`0600`), configures `sudoers.d` access, generates arbitrary system files, executes declarative commands (`runcmd`), and tracks boot idempotency across reboots.

This subsystem provides zero-touch provisioning for OpenStack, Proxmox VE, KVM/QEMU, Terraform, Ansible, and Kubernetes bare-metal automation pipelines.

---

## 2. Architecture & Pipeline Overview

```
+-----------------------------------------------------------------------------------+
|                        Hypervisor / Cloud Infrastructure                          |
|    (OpenStack ConfigDrive, Proxmox Cloud-Init ISO, NoCloud Disk, or Cmdline)      |
+-----------------------------------------------------------------------------------+
                                         |
                                         | VIRTIO / IDE / CD-ROM / Kernel Cmdline
                                         v
+-----------------------------------------------------------------------------------+
|                  Datasource Discovery Engine (datasource.rs)                      |
|  - CLI Override: --seed <dir> or env NOVAINIT_SEED_DIR                            |
|  - Kernel Command Line: /proc/cmdline (ds=nocloud;s=..., novainit.seed=...)       |
|  - NoCloud Seeds: /var/lib/cloud/seed/nocloud, /var/lib/cloud/seed                |
|  - ConfigDrive: /media/configdrive, /mnt/configdrive (openstack/latest, ec2)      |
+-----------------------------------------------------------------------------------+
                                         |
                                         | Raw YAML / JSON Streams
                                         v
+-----------------------------------------------------------------------------------+
|                    Pure Rust Parsers (parser.rs & network.rs)                     |
|  - YamlParser: Recursive AST (Mappings, Sequences, Integers, Booleans, Blocks)    |
|  - JsonParser: JSON Object & Array Tokens (OpenStack meta_data.json)              |
|  - Netplan v2 Engine: DHCP4/DHCP6, Static IPs, Gateways, Nameservers, CIDR masks  |
+-----------------------------------------------------------------------------------+
                                         |
                                         | Strongly-Typed CloudConfig
                                         v
+-----------------------------------------------------------------------------------+
|                  Idempotency & Lifecycle Manager (cloud_init.rs)                  |
|  - Instance Tracking: /var/lib/cloud/instance/instance-id                         |
|  - Idempotent Check: If instance-id matches, skip execution without mutations     |
|  - Status Reporting: /run/cloud-init/status.json                                  |
+-----------------------------------------------------------------------------------+
                                         |
                                         | Staged Execution
                                         v
+-----------------------------------------------------------------------------------+
|                     System Provisioner Engine (provision.rs)                      |
|  1. Network: /etc/network/interfaces, /etc/resolv.conf                            |
|  2. Identity: /etc/hostname, /etc/hosts (127.0.1.1 <hostname>)                     |
|  3. Accounts: /etc/passwd (UID 1000), /home/<user>/.ssh/authorized_keys (0600)    |
|  4. Root Security: /root/.ssh/authorized_keys (0600, 0700 dir)                   |
|  5. Privileges: /etc/sudoers.d/90-cloud-init-<user> (mode 0440)                   |
|  6. Files: write_files (declarative paths, contents, octal permissions)           |
|  7. Commands: runcmd sequential command execution array                           |
+-----------------------------------------------------------------------------------+
                                         |
                                         | System Ready
                                         v
+-----------------------------------------------------------------------------------+
|                   NovaOS Services (novad / novasshd / novafw)                     |
|  - novad starts network.target and novasshd.service                               |
|  - System is immediately reachable via SSH using injected public keys             |
+-----------------------------------------------------------------------------------+
```

---

## 3. Supported Datasource Providers

The `DataSourceManager` (`datasource.rs`) implements multi-provider discovery in priority order:

1. **Explicit Seed Override**:
   - CLI argument: `novainit init --seed /path/to/seed`
   - Environment variable: `NOVAINIT_SEED_DIR=/mnt/seed`

2. **Kernel Command Line Parameters (`/proc/cmdline`)**:
   - `ds=nocloud;s=/path/to/seed/`
   - `novainit.seed=/path/to/seed/`
   - `ci.seed=/path/to/seed/`

3. **NoCloud Standard Filesystem Paths**:
   - `/var/lib/cloud/seed/nocloud/` (`meta-data`, `user-data`, `network-config`)
   - `/var/lib/cloud/seed/`
   - `/var/lib/cloud/instance/`
   - `/etc/cloud/seed/`

4. **ConfigDrive (OpenStack & Proxmox VE)**:
   - Mount points: `/media/configdrive`, `/mnt/configdrive`, `/var/lib/cloud/config-drive`
   - OpenStack layout: `openstack/latest/meta_data.json`, `user_data`, `network_data.json`
   - EC2 fallback layout: `ec2/latest/meta-data`, `user-data`

---

## 4. Declarative `#cloud-config` Schema

`novainit` processes standard `#cloud-config` YAML directives:

```yaml
#cloud-config
hostname: novanode-prod-01
fqdn: novanode-prod-01.infra.corp

ssh_authorized_keys:
  - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInw2BkW1+8n83fM5R ops-admin@corp

users:
  - name: novaadmin
    gecos: Enterprise Administrator
    shell: /bin/novash
    sudo: ALL=(ALL) NOPASSWD:ALL
    groups: [wheel, sudo, admin]
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIInw2BkW1+8n83fM5R novaadmin@corp

write_files:
  - path: /etc/motd
    content: "NovaOS Cloud Node Ready\n"
    permissions: "0644"
  - path: /etc/issue
    content: "NovaOS 2.0 Cloud Instance\n"
    permissions: "0644"

runcmd:
  - [ echo, "Provisioning completed" ]
```

---

## 5. Network Configuration (Netplan v2 & Interfaces v1)

`novainit` accepts modern Netplan v2 schemas via `network-config`:

```yaml
version: 2
ethernets:
  eth0:
    dhcp4: false
    dhcp6: false
    addresses:
      - 192.168.100.50/24
    gateway4: 192.168.100.1
    nameservers:
      addresses:
        - 1.1.1.1
        - 8.8.8.8
      search:
        - corp.internal
```

The engine converts this into `/etc/network/interfaces` and `/etc/resolv.conf`:

```
# Generated by NovaOS Cloud-Init
auto lo
iface lo inet loopback

auto eth0
iface eth0 inet static
    address 192.168.100.50
    netmask 255.255.255.0
    gateway 192.168.100.1
```

CIDR prefix-to-netmask conversion is calculated entirely in constant-space integer bit shifts without external dependencies.

---

## 6. Security Model & OpenSSH Injection

1. **Privilege Isolation**:
   - Accounts are created with standard non-root UIDs (starting at UID 1000).
   - Shell defaults to `/bin/novash` (or user-specified binary).
   - Account lines are appended to `/etc/passwd` cleanly with locked passwords (`x`).

2. **OpenSSH Key Security**:
   - Keys configured in `ssh_authorized_keys` are appended to `/root/.ssh/authorized_keys`.
   - Per-user keys are installed in `/home/<username>/.ssh/authorized_keys`.
   - On POSIX filesystems, permissions are strictly enforced:
     - `.ssh` directories: `0700` (`rwx------`)
     - `authorized_keys`: `0600` (`rw-------`)

3. **Sudo Access**:
   - Dedicated drop-in files are generated in `/etc/sudoers.d/90-cloud-init-<user>`.
   - Permissions are enforced to read-only `0440` (`r--r-----`).

---

## 7. Boot Idempotency & Lifecycle

- On successful completion, `novainit` writes the active instance ID to `/var/lib/cloud/instance/instance-id`.
- On subsequent boots, `novainit` compares the discovered instance ID against this stamp:
  - **Matched**: Logs that provisioning has already occurred and exits immediately with code 0.
  - **New ID**: Triggers fresh reprovisioning (e.g. when a cloned VM template boots with a new instance ID).
- Real-time provisioning status is published to `/run/cloud-init/status.json`:
  ```json
  {
    "status": "done",
    "instance_id": "novanode-prod-01",
    "timestamp": 1731950000,
    "datasource": "NoCloud",
    "errors": []
  }
  ```

---

## 8. CLI & Service Unit

### CLI Commands
- `novainit init [--local] [--seed <dir>] [--force]`: Execute cloud-init boot sequence.
- `novainit modules [--mode <config|final>]`: Execute individual configuration stages.
- `novainit status`: Inspect provisioning state and active instance ID.
- `novainit query <key>`: Query parsed metadata keys (e.g. `instance_id`, `hostname`).
- `novainit --selftest`: Run autonomous qualification tests (returns 0 on pass).

### Systemd / Novad Service Unit (`tools/fixtures/novad/system/cloud-init.service`)
```ini
[Unit]
Description=NovaOS Cloud-Init Automated Headless Provisioning Service
Before=network.target multi-user.target novasshd.service
After=local-fs.target
Requires=local-fs.target

[Service]
Type=oneshot
ExecStart=/bin/novainit init
RemainAfterExit=yes
StandardOutput=journal

[Install]
WantedBy=multi-user.target
```
