#!/bin/sh
# Month 1 - Foundation: host-side tests for Pure Rust NVMe 2.0 PCIe Storage Controller Driver
# 1. PCI class enumeration matching 0x010802 (Storage, NVM, NVM Express).
# 2. 64-bit BAR0 MMIO register space mapping (CAP, VS, CC, CSTS, AQA, ASQ, ACQ).
# 3. Doorbell offset calculations across varying doorbell strides.
# 4. 64-byte SQE and 16-byte CQE layout and size verifications.
# 5. Zero-copy DMA: Physical Region Page (PRP) mode calculations.
# 6. NVMe 2.0 Scatter Gather List (SGL) descriptor layout and parsing.
# 7. Circular DMA submission and completion queue ring buffers with phase tag inversion.
# 8. Identify Controller & Namespace data structure parsing (NVMe 2.0, 512B/4096B LBA).
# 9. MSI-X interrupt vector table mapping and queue steering.
# 10. VFS BlockDevice trait read/write operations and 2.5+ GB/s throughput benchmarking.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_nvme_tests.sh      (or: make nvme2-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/drivers/storage/nvme.rs host tests (NVMe 2.0 PCIe Driver) ---"
    rustc --edition 2021 --test -o "$OUT/nvme_test" kernel/src/drivers/storage/nvme.rs
    "$OUT/nvme_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/nvme_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/storage/nvme.rs tests succeeded."
    echo "[PASS] All NVMe 2.0 PCIe Storage Controller Driver host tests succeeded."
else
    echo "nvme2-test: rustc not found - skipping the Rust suite"
fi
