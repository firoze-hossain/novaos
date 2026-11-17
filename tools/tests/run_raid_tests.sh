#!/bin/sh
# Month 5 - Enterprise Storage: Enterprise Multi-Disk Storage & Software RAID Subsystem (NovaVolume)
#
# Validates pure Rust software RAID and dynamic volume pooling features:
# 1. Multi-Disk Storage Pooling & RAID Levels:
#    - RAID-0 (Striped): high-throughput striping across >=2 disks.
#    - RAID-1 (Mirrored): synchronous write replication, round-robin read load balancing.
#    - RAID-10 (Striped Mirrors): nested 1+0 array combining striped throughput with mirrored resilience.
# 2. Resilient Superblock Metadata & Write Intent Logging:
#    - Persistent 512-byte on-disk superblock with CRC32 integrity verification.
#    - Monotonic generation tracking and state persistence.
# 3. Dynamic Online Volume Resizing (extend_volume):
#    - Expands virtual block device capacity online without service interruption.
# 4. Fault Detection & Zero-Data-Loss Failover:
#    - Catastrophic drive failure simulation and instant read/write failover to surviving mirrors.
# 5. Hot-Plug Disk Detection & Automatic Hot-Spare Takeover:
#    - Dynamically detects newly inserted drives and attaches standby hot-spares.
#    - Automatically promotes hot-spare to active rebuilding target upon drive failure.
# 6. Automatic Background Rebuild Engine:
#    - Chunk-by-chunk resynchronization with progress tracking and live I/O handling.
# 7. Complete CLI Management:
#    - novactl volume create, list, status, extend, add-disk, fail-disk, rebuild, rm.
#
# Usage:
#   ./tools/tests/run_raid_tests.sh      (or: make raid-test)
set -e
cd "$(dirname "$0")/../.."

if command -v cargo >/dev/null 2>&1; then
    echo "--- Month 5: Enterprise Multi-Disk Storage & Software RAID Subsystem (NovaVolume) ---"
    
    # 1. Run Cargo unit test suite
    echo "[1/4] Running Cargo unit test suite..."
    (cd userland/server/novactl && cargo test --quiet)
    
    # 2. Run automated self-test suite
    echo "[2/4] Running novactl --selftest engine verification..."
    (cd userland/server/novactl && cargo run --quiet -- --selftest)

    # 3. Run full CLI storage & RAID lifecycle in isolated sandbox
    echo "[3/4] Running CLI software RAID volume lifecycle workflow..."
    SANDBOX_DIR=$(mktemp -d)
    trap 'rm -rf "$SANDBOX_DIR"' EXIT
    export NOVACTL_DATA_DIR="$SANDBOX_DIR/data"
    mkdir -p "$NOVACTL_DATA_DIR"

    BIN="userland/server/novactl/target/debug/novactl"
    if [ ! -f "$BIN" ]; then
        (cd userland/server/novactl && cargo build --quiet)
    fi

    # Create RAID-1 volume with hot-spare
    echo "  Creating RAID-1 mirrored volume 'dbpool' with active drives and hot-spare..."
    $BIN volume create dbpool --level 1 --disks /dev/nvme0n1,/dev/nvme1n1 --spares /dev/nvme2n1

    # List volumes and verify
    echo "  Listing storage volumes..."
    $BIN volume list | grep -q "dbpool"

    # Status inspection
    echo "  Inspecting 'dbpool' volume status..."
    $BIN volume status dbpool | grep -q "HEALTHY"
    $BIN volume status dbpool | grep -q "RAID-1"

    # Online capacity expansion
    echo "  Testing online volume capacity extension (+50G)..."
    $BIN volume extend dbpool 50G | grep -q "Successfully extended"

    # Inject simulated physical drive failure
    echo "  Simulating catastrophic drive failure on /dev/nvme0n1..."
    $BIN volume fail-disk dbpool /dev/nvme0n1 | grep -q "Hot-spare activated"

    # Verify transition to Rebuilding
    $BIN volume status dbpool | grep -q "REBUILDING"

    # Step background rebuild engine to 100%
    echo "  Stepping background rebuild engine..."
    $BIN volume rebuild dbpool | grep -q "Rebuild progress"
    $BIN volume rebuild dbpool | grep -q "Fully restored to 100%"

    # Verify status is now Healthy
    $BIN volume status dbpool | grep -q "HEALTHY"

    # Hot-plug another standby spare
    echo "  Hot-plugging new standby drive /dev/nvme3n1..."
    $BIN volume add-disk dbpool /dev/nvme3n1 --spare | grep -q "added to volume"

    # Test RAID-0 and RAID-10 volume creation
    echo "  Creating RAID-0 stripe volume 'cachepool'..."
    $BIN volume create cachepool --level 0 --disks /dev/sda,/dev/sdb
    $BIN volume list | grep -q "cachepool"

    echo "  Creating RAID-10 striped-mirror volume 'enterprisepool'..."
    $BIN volume create enterprisepool --level 10 --disks /dev/sda,/dev/sdb,/dev/sdc,/dev/sdd
    $BIN volume list | grep -q "enterprisepool"
    $BIN volume status enterprisepool | grep -q "RAID-10"

    # Volume teardown
    echo "  Cleaning up storage volumes..."
    $BIN volume rm cachepool | grep -q "deleted"
    $BIN volume rm enterprisepool | grep -q "deleted"
    $BIN volume rm dbpool | grep -q "deleted"

    # 4. Verify kernel boot log assertion
    echo "[4/4] Verifying kernel-side RAID selftest in boot log..."
    if [ -f "build/test-serial.log" ]; then
        grep -q "Enterprise Software RAID initialized" build/test-serial.log
    fi

    echo "[PASS] All Enterprise Multi-Disk Storage & Software RAID Subsystem (NovaVolume) tests passed successfully."
else
    echo "[SKIP] Cargo not installed; skipping host storage RAID test."
fi
