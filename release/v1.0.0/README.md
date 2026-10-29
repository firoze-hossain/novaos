# NovaOS v1.0.0 Production Release

This directory contains the official cryptographic checksums and build manifests for NovaOS v1.0.0.

## Building the Release Artifacts
To reproduce or build the release binaries locally:
```bash
make release-signoff
```

This generates:
- `novaos.iso` (Hybrid bootable ISO for CD-ROM or USB BIOS hard drive)
- `novaos-desktop.img` (Desktop Edition partition image with full 13-app GUI suite)
- `novaos-server.img` (Server Edition headless partition image)
- `novaos.bin` (Shared kernel binary)

## Integrity Verification
```bash
sha256sum -c SHA256SUMS
```
See [RELEASE_SIGNOFF_V1.0.md](../../docs/RELEASE_SIGNOFF_V1.0.md) for full qualification details.
