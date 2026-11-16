#!/bin/sh
# Month 3 - Server & Containers: host-side tests for Enterprise Cloud TCP Stack
#
# Validates enterprise cloud TCP algorithms and mechanisms in pure Rust (#![no_std]):
# 1. RFC 8312 CUBIC Congestion Control:
#    - Slow Start initial window (10 MSS) and exponential growth.
#    - Fast Recovery entry with RFC 8312 beta = 0.7 window reduction.
#    - Pure integer arithmetic cube root calculation (integer_cbrt) without FPU.
#    - Congestion avoidance target calculation with concave and convex curves.
# 2. BBR (Bottleneck Bandwidth and RTT) Congestion Control:
#    - 4-phase state machine: Startup -> Drain -> ProbeBW -> ProbeRTT.
#    - 8-phase pacing gain cycle [1.25, 0.75, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0].
#    - Dynamic Bottleneck Bandwidth (BtlBw) filter and RTprop round-trip tracking.
#    - Bandwidth-Delay Product (BDP) target window calculation and pacing rate calculation.
# 3. RFC 2018 Selective Acknowledgments (SACK):
#    - Option Kind 4 (SACK-Permitted) handshake negotiation.
#    - Option Kind 5 (SACK Blocks) serialization for out-of-order received segments.
#    - Out-of-order block merging, sorting, and cumulative ACK pruning.
#    - SackScoreboard tracking of in-flight segments and selective fast retransmit of loss holes.
# 4. RFC 7323 TCP Window Scaling:
#    - Option Kind 3 (Window Scale) 3-byte negotiation during SYN / SYN-ACK.
#    - Shift factors 0..14 supported (up to 1GB effective window).
#    - 16-bit to 32-bit window decoding and encoding.
# 5. RFC 4987 TCP SYN Cookies:
#    - Stateless SYN-ACK generation during SYN flood DDoS pressure.
#    - 24-bit cryptographic hash mixing, 5-bit time quantum, 3-bit standard MSS table index.
#    - Cryptographic cookie verification and MSS reconstruction.
#    - Forgery detection and expiration rejection.
# 6. RFC 1122 TCP Keepalive Timers:
#    - Configurable idle time, probe interval, and max probe count.
#    - Probe segment generation with seq = snd_una - 1.
#    - Unresponsive peer detection and timeout teardown.
#
# Usage:
#   ./tools/tests/run_enterprise_tcp_tests.sh      (or: make enterprise-tcp-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/net/tcp/ host tests (Enterprise Cloud TCP Stack: Cubic, BBR, SACK, WS, SYN Cookies, Keepalive) ---"
    rustc --edition 2021 --test -o "$OUT/enterprise_tcp_test" kernel/src/net/tcp/mod.rs
    "$OUT/enterprise_tcp_test" 2>&1 | grep -E "^test result|FAILED|panicked|failed" || true
    "$OUT/enterprise_tcp_test" >/dev/null 2>&1
    echo "[PASS] All kernel/src/net/tcp/ tests succeeded."

    echo "[PASS] All Enterprise Cloud TCP Stack tests passed successfully."
else
    echo "rustc not found; skipping Enterprise TCP host tests."
fi
