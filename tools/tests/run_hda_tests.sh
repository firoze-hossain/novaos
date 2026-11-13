#!/bin/sh
# Month 5 - Desktop Experience: host-side tests for Intel High Definition Audio (HDA) Driver & Codec Subsystem
# 1. PCI class code compliance (0x040300) and MMIO register map validation.
# 2. Controller reset protocol (GCTL.CRST) and CORB/RIRB ring buffer setup.
# 3. Stream format encoding/decoding for 24-bit 96kHz, 24-bit 48kHz, 16-bit 44.1/48kHz PCM, 1..8 channels.
# 4. Buffer Descriptor List (BDL) cyclic DMA stream rings with period boundaries and IOC flags.
# 5. Codec verb command construction and response unpacking.
# 6. Audio Function Group (AFG) and Audio Widget topology discovery (DAC, ADC, Pin Complex, Mixer).
# 7. Pin configuration default register parsing (Connectivity, Location, Device type, Color, Jack sense).
# 8. External Amplifier Power Down (EAPD) and Headphone Amplifier activation.
# 9. Hardware volume and gain control with dB attenuation.
# 10. Playback and capture DMA circular buffer operations with Link Position in Buffer (LPIB).
# 11. Zero-latency stream delay query and buffer free space calculations.
# 12. VFS /dev/snd/pcm0p and /dev/snd/pcm0c character device interfaces for novasound daemon.
#
# No QEMU needed; executes in under two seconds.
#
#   ./tools/tests/run_hda_tests.sh      (or: make hda-test)
set -e
cd "$(dirname "$0")/../.."
OUT=$(mktemp -d)
trap 'rm -rf "$OUT"' EXIT

if command -v rustc >/dev/null 2>&1; then
    echo "--- kernel/src/drivers/audio/hda.rs host tests (Intel HDA Controller & Sound Subsystem) ---"
    rustc --edition 2021 --test -o "$OUT/hda_test" kernel/src/drivers/audio/hda.rs
    "$OUT/hda_test" --test-threads=1 2>&1 | grep -E "^test |^test result|FAILED|panicked|failed" || true
    "$OUT/hda_test" --test-threads=1 >/dev/null 2>&1
    echo "[PASS] All kernel/src/drivers/audio/hda.rs tests succeeded."

    echo "[PASS] All Intel HDA Driver & Sound Subsystem host tests succeeded."
else
    echo "hda-test: rustc not found - skipping the Rust suite"
fi
