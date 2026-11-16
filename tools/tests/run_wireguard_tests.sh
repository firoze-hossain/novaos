#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Native In-Kernel WireGuard VPN
#
# Validates modern cryptographic VPN mechanisms in pure Rust (#![no_std]):
# 1. Pure Rust Cryptographic Primitives:
#    - Curve25519 / X25519 ECDH Montgomery ladder (RFC 7748).
#    - ChaCha20-Poly1305 AEAD authenticated encryption/decryption (RFC 7539 / RFC 8439).
#    - BLAKE2s-256 hash, HMAC-BLAKE2s, and HKDF-style KDF1/KDF2/KDF3 (RFC 7693).
#    - TAI64N 12-byte monotonic timestamping.
# 2. Noise Protocol Framework (Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s):
#    - Handshake Initiation (148 bytes) & Response (92 bytes).
#    - Symmetric key derivation for transport data.
#    - 128-packet anti-replay sliding window.
# 3. Cryptokey Routing Subsystem:
#    - AllowedIPs CIDR table with Longest Prefix Match (LPM) on egress.
#    - Reverse-path source address check on ingress.
# 4. Virtual Device & Automatic Roaming:
#    - Dynamic endpoint roaming updates on authentic packets.
#    - Traffic accounting (tx_bytes, rx_bytes).
# 5. Userland Management CLI (novactl wg):
#    - novactl wg genkey, pubkey, set, show.
#
# Usage:
#   ./tools/tests/run_wireguard_tests.sh      (or: make wireguard-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/net/wireguard/ host tests (Native In-Kernel WireGuard VPN) ---"

    # 1. Standalone kernel WireGuard test harness
    ROOT_DIR="$(pwd)"
    cat << EOF > "$OUT/test_harness.rs"
#![allow(dead_code)]
#[path = "$ROOT_DIR/kernel/src/net/wireguard/mod.rs"]
mod wireguard;

// Mock spinlock for host test
pub mod spinlock {
    pub struct SpinLock<T>(std::sync::Mutex<T>);
    impl<T> SpinLock<T> {
        pub const fn new(val: T) -> Self {
            SpinLock(std::sync::Mutex::new(val))
        }
        pub fn lock(&self) -> std::sync::MutexGuard<'_, T> {
            self.0.lock().unwrap()
        }
    }
}

// Mock ip_send for host test
#[no_mangle]
pub extern "C" fn ip_send(_dest_ip: u32, _protocol: u8, _payload: *const u8, _payload_len: u16) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wireguard_full_selftest() {
        assert!(wireguard::run_wireguard_selftest(), "WireGuard self-test failed");
    }

    #[test]
    fn test_wireguard_replay_window_extended() {
        let mut w = wireguard::ReplayWindow::new();
        assert!(w.check_and_update(100));
        assert!(w.check_and_update(101));
        assert!(!w.check_and_update(100));
        assert!(!w.check_and_update(101));
        assert!(w.check_and_update(20)); // within 128
        assert!(!w.check_and_update(20));
        assert!(w.check_and_update(228)); // jump
        assert!(!w.check_and_update(20)); // now too old
    }

    #[test]
    fn test_wireguard_routing_lpm() {
        let mut rt = wireguard::CryptokeyRoutingTable::new();
        rt.add_route(0x0A000000, 8, 1);
        rt.add_route(0x0A010000, 16, 2);
        rt.add_route(0x0A010101, 32, 3);
        assert_eq!(rt.lookup_peer(0x0A010101), Some(3));
        assert_eq!(rt.lookup_peer(0x0A010102), Some(2));
        assert_eq!(rt.lookup_peer(0x0A020101), Some(1));
        assert_eq!(rt.lookup_peer(0xC0A80101), None);
    }
}
EOF

    rustc --edition 2021 --test -o "$OUT/wireguard_test" "$OUT/test_harness.rs"
    "$OUT/wireguard_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/wireguard_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/net/wireguard/ crypto, noise, and routing tests succeeded."

    # 2. novactl WireGuard CLI tests
    echo "--- userland/server/novactl wireguard CLI verification ---"
    cd userland/server/novactl
    cargo test --quiet
    cargo run --quiet -- --selftest >/dev/null
    cd ../../

    echo "[PASS] All Native In-Kernel WireGuard VPN Tunneling Engine tests passed successfully."
else
    echo "rustc not found; skipping WireGuard host tests."
fi
