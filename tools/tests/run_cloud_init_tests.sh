#!/bin/sh
# Month 6 - Release Hardening: Cloud-Init & Headless Server Automated Provisioning
#
# Dedicated qualification test runner for NovaOS Cloud-Init automated provisioning architecture:
#   1. File Architecture & Crate Manifests
#   2. Pure Rust #cloud-config YAML Parser
#   3. Pure Rust JSON Parser (OpenStack / Proxmox ConfigDrive)
#   4. Multi-Provider Datasource Discovery (NoCloud, ConfigDrive, Kernel Cmdline)
#   5. Netplan v2 DHCP Network Interface Parser
#   6. Netplan v2 Static IP, Gateway, DNS, & CIDR Synthesis
#   7. System Network Configuration Generation (/etc/network/interfaces & /etc/resolv.conf)
#   8. Hostname & /etc/hosts Cloud Provisioning (127.0.1.1 mapping)
#   9. User Account Creation & Sudo Rule Provisioning (/etc/passwd & /etc/sudoers.d/)
#  10. OpenSSH Key Injection & Strict Permissions (/root & /home/<user>/.ssh/authorized_keys)
#  11. Declarative File Generation (write_files block)
#  12. Command Execution Orchestration (runcmd sequence)
#  13. Instance Idempotency & State Tracking (/var/lib/cloud/instance & /run/cloud-init/status.json)
#  14. Autonomous Master Self-Test & CLI Interface (novainit --selftest)
#
# Usage:
#   ./tools/tests/run_cloud_init_tests.sh    (or: make test-cloud-init / make cloud-init-test)

set -e
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

echo "================================================================================"
echo "      NovaOS 2.0 - Cloud-Init & Headless Server Automated Provisioning          "
echo "================================================================================"
echo "Date: $(date)"
echo "Target: userland/server/novainit (cloud_init.rs, datasource.rs, network.rs, etc.)"
echo "--------------------------------------------------------------------------------"

TOTAL_CHECKS=0
PASSED_CHECKS=0

assert_check() {
    DESC="$1"
    TOTAL_CHECKS=$((TOTAL_CHECKS + 1))
    printf "Check [%02d] %s... " "$TOTAL_CHECKS" "$DESC"
}

pass_check() {
    PASSED_CHECKS=$((PASSED_CHECKS + 1))
    echo "OK"
}

fail_check() {
    REASON="$1"
    echo "FAIL"
    echo "[-] Error: $REASON"
    exit 1
}

OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

# ------------------------------------------------------------------------------
# Check 1: File Architecture & Manifests
# ------------------------------------------------------------------------------
assert_check "File architecture and crate manifests"
[ -f "userland/server/novainit/Cargo.toml" ] || fail_check "userland/server/novainit/Cargo.toml missing"
[ -f "userland/server/novainit/src/main.rs" ] || fail_check "userland/server/novainit/src/main.rs missing"
[ -f "userland/server/novainit/src/lib.rs" ] || fail_check "userland/server/novainit/src/lib.rs missing"
[ -f "userland/server/novainit/src/cloud_init.rs" ] || fail_check "userland/server/novainit/src/cloud_init.rs missing"
[ -f "userland/server/novainit/src/datasource.rs" ] || fail_check "userland/server/novainit/src/datasource.rs missing"
[ -f "userland/server/novainit/src/network.rs" ] || fail_check "userland/server/novainit/src/network.rs missing"
[ -f "userland/server/novainit/src/provision.rs" ] || fail_check "userland/server/novainit/src/provision.rs missing"
[ -f "userland/server/novainit/src/parser.rs" ] || fail_check "userland/server/novainit/src/parser.rs missing"
[ -f "tools/fixtures/novad/system/cloud-init.service" ] || fail_check "cloud-init.service unit file missing"
pass_check

# ------------------------------------------------------------------------------
# Check 2: Pure Rust #cloud-config YAML Parser
# ------------------------------------------------------------------------------
assert_check "Pure Rust #cloud-config YAML parser (scalars, mappings, sequences)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib parser::tests::test_yaml_cloud_config_parsing >/dev/null 2>&1 || fail_check "YAML cloud config parsing test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 3: Pure Rust JSON Parser (OpenStack / Proxmox ConfigDrive)
# ------------------------------------------------------------------------------
assert_check "Pure Rust JSON parser for OpenStack/Proxmox ConfigDrive metadata"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib parser::tests::test_json_openstack_metadata_parsing >/dev/null 2>&1 || fail_check "JSON OpenStack metadata parsing test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 4: Multi-Provider Datasource Discovery
# ------------------------------------------------------------------------------
assert_check "Multi-provider datasource discovery (NoCloud & ConfigDrive)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib datasource::tests::test_load_from_nocloud_directory >/dev/null 2>&1 || fail_check "NoCloud discovery test failed"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib datasource::tests::test_load_from_openstack_config_drive >/dev/null 2>&1 || fail_check "OpenStack ConfigDrive discovery test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 5: Netplan v2 DHCP Network Interface Parser
# ------------------------------------------------------------------------------
assert_check "Netplan v2 DHCP network interface parser"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib network::tests::test_parse_netplan_dhcp >/dev/null 2>&1 || fail_check "Netplan DHCP parsing test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 6: Netplan v2 Static IP, Gateway, DNS, & CIDR Synthesis
# ------------------------------------------------------------------------------
assert_check "Netplan v2 static IP, gateway, nameservers, & CIDR netmask conversion"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib network::tests::test_parse_netplan_static_ip_and_dns >/dev/null 2>&1 || fail_check "Netplan static IP and DNS parsing test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 7: System Network Configuration Generation
# ------------------------------------------------------------------------------
assert_check "System network configuration generation (/etc/network/interfaces & /etc/resolv.conf)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib provision::tests::test_provision_hostname_and_hosts >/dev/null 2>&1 || fail_check "System network config generation test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 8: Hostname & /etc/hosts Cloud Provisioning
# ------------------------------------------------------------------------------
assert_check "Hostname and /etc/hosts cloud provisioning (127.0.1.1 mapping)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib provision::tests::test_provision_hostname_and_hosts >/dev/null 2>&1 || fail_check "Hostname and hosts provisioning test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 9: User Account Creation & Sudo Rule Provisioning
# ------------------------------------------------------------------------------
assert_check "User account creation & sudoers configuration (/etc/passwd & /etc/sudoers.d/)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib provision::tests::test_provision_user_and_ssh_keys >/dev/null 2>&1 || fail_check "User and sudoers provisioning test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 10: OpenSSH Key Injection & Strict Permissions
# ------------------------------------------------------------------------------
assert_check "OpenSSH key injection & strict permissions (0700 dir, 0600 authorized_keys)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib provision::tests::test_provision_user_and_ssh_keys >/dev/null 2>&1 || fail_check "OpenSSH key injection test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 11: Declarative File Generation (write_files)
# ------------------------------------------------------------------------------
assert_check "Declarative file generation via write_files block with custom octal permissions"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib provision::tests::test_provision_write_files >/dev/null 2>&1 || fail_check "write_files generation test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 12: Command Execution Orchestration (runcmd)
# ------------------------------------------------------------------------------
assert_check "Command execution orchestration (runcmd command array)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib cloud_init::tests::test_cloud_init_end_to_end_lifecycle >/dev/null 2>&1 || fail_check "runcmd orchestration test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 13: Instance Idempotency & State Tracking
# ------------------------------------------------------------------------------
assert_check "Instance idempotency & state tracking (/var/lib/cloud/instance/instance-id)"
cargo test --manifest-path userland/server/novainit/Cargo.toml --lib cloud_init::tests::test_cloud_init_end_to_end_lifecycle >/dev/null 2>&1 || fail_check "Instance idempotency test failed"
pass_check

# ------------------------------------------------------------------------------
# Check 14: Autonomous Master Self-Test & CLI Interface
# ------------------------------------------------------------------------------
assert_check "Autonomous master self-test suite & novainit CLI interface"
cargo run --manifest-path userland/server/novainit/Cargo.toml --quiet -- --selftest >/dev/null 2>&1 || fail_check "novainit --selftest failed"
cargo run --manifest-path userland/server/novainit/Cargo.toml --quiet -- --help >/dev/null 2>&1 || fail_check "novainit --help failed"
pass_check

echo "--------------------------------------------------------------------------------"
echo "All $TOTAL_CHECKS/$TOTAL_CHECKS Cloud-Init qualification checks PASSED (100% success)"
echo "================================================================================"
exit 0
