#!/usr/bin/env bash
# ==============================================================================
#  NovaOS Dual-Edition Automated Release Packaging Pipeline (v2.0)
# ==============================================================================
#
# Generates production-grade release media for NovaOS 2.0:
# 1. novaos-desktop-2.0.iso : Hybrid UEFI/BIOS bootable installer containing the
#                             full Wayland compositor, Fluent/macOS shell, and desktop app suite.
# 2. novaos-server-2.0.iso  : Optimized headless server image with SSH, container engine,
#                             conntrack firewall, Prometheus agent, and cloud-init bootstrap.
# 3. novaos-desktop-2.0.img : Partitioned GPT hybrid raw disk image (ESP + SYS + DATA + JNL + CRASH).
# 4. novaos-server-2.0.img  : Partitioned GPT hybrid raw disk image (ESP + SYS + DATA + JNL + CRASH).
# 5. Cryptographic Signatures: SHA256SUMS, individual digests, and GPG detached signatures.
# 6. Release Manifest: release_manifest.json generated via pure Rust novarelease engine.
#
# Usage:
#   ./tools/release/build_release_images.sh [--release-dir DIR] [--version VER] [--skip-build] [--fast]
# ==============================================================================

set -eu
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO_ROOT"

RELEASE_DIR="release/v2.0"
VERSION="2.0.0"
SKIP_BUILD=0
FAST_MODE=0
CLEAN_DIR=0
GPG_KEY_EMAIL="release@novaos.org"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --release-dir)
            RELEASE_DIR="$2"
            shift 2
            ;;
        --release-dir=*)
            RELEASE_DIR="${1#*=}"
            shift
            ;;
        --version)
            VERSION="$2"
            shift 2
            ;;
        --version=*)
            VERSION="${1#*=}"
            shift
            ;;
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        --fast)
            FAST_MODE=1
            shift
            ;;
        --clean)
            CLEAN_DIR=1
            shift
            ;;
        --help|-h)
            echo "NovaOS 2.0 Release Packaging Pipeline"
            echo "Usage: $0 [options]"
            echo "Options:"
            echo "  --release-dir <DIR>  Output directory (default: release/v2.0)"
            echo "  --version <VER>      Release version string (default: 2.0.0)"
            echo "  --skip-build         Use existing binaries without rebuilding"
            echo "  --fast               Fast build mode (reuses caches where possible)"
            echo "  --clean              Clean release directory before generating"
            echo "  --help               Display this help text"
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            exit 1
            ;;
    esac
done

DATE_STAMP="$(date -u '+%Y-%m-%d %H:%M:%S UTC')"
echo "================================================================================"
echo "          NovaOS 2.0 Production Dual-Edition Release Packaging Pipeline         "
echo "================================================================================"
echo "Version:     $VERSION"
echo "Target Dir:  $RELEASE_DIR"
echo "Timestamp:   $DATE_STAMP"
echo "Host Kernel: $(uname -s) $(uname -m) - $(uname -r)"
echo "================================================================================"

STEP=0
step() {
    STEP=$((STEP + 1))
    echo ""
    echo ">>> [Phase $STEP] $1"
    echo "--------------------------------------------------------------------------------"
}

# ------------------------------------------------------------------------------
# Phase 1: Toolchain & Prerequisite Sanity Checks
# ------------------------------------------------------------------------------
step "Verifying Toolchain & Build Prerequisites"

REQ_TOOLS=(gcc ld nasm xorriso grub-mkrescue parted mformat mcopy mkfs.ext2 debugfs python3 gpg sha256sum cargo rustc)
for tool in "${REQ_TOOLS[@]}"; do
    if ! command -v "$tool" &>/dev/null; then
        echo "❌ Error: Required tool '$tool' not found in PATH." >&2
        exit 1
    fi
done
echo "✅ All required host tools present: ${REQ_TOOLS[*]}"

# ------------------------------------------------------------------------------
# Phase 2: Compile Rust novarelease Engine & System Binaries
# ------------------------------------------------------------------------------
step "Building Rust Release Engine (novarelease) and Core Binaries"

echo "Compiling tools/release (novarelease)..."
cargo build --release --manifest-path tools/release/Cargo.toml
NOVARELEASE_BIN="tools/release/target/release/novarelease"

if [[ $SKIP_BUILD -eq 0 ]]; then
    echo "Compiling core desktop kernel (novaos.bin)..."
    make build/novaos.bin

    echo "Compiling dedicated headless server kernel (novaos-server.bin)..."
    make kernel-server

    echo "Ensuring Desktop test disk is up-to-date..."
    make disk-desktop

    echo "Ensuring Server test disk is up-to-date..."
    make disk-server
else
    echo "Skipping rebuild of core binaries (--skip-build specified)."
fi

# Verify source binaries exist
for req_bin in build/novaos.bin build/novaos-server.bin disk.img disk-server.img; do
    if [[ ! -f "$req_bin" ]]; then
        echo "❌ Error: Required artifact '$req_bin' does not exist." >&2
        exit 1
    fi
done
echo "✅ Core binaries and test disk images verified."

# ------------------------------------------------------------------------------
# Phase 3: Setup Release & Staging Directories
# ------------------------------------------------------------------------------
step "Preparing Release Directory Structure"

if [[ $CLEAN_DIR -eq 1 && -d "$RELEASE_DIR" ]]; then
    echo "Cleaning old release directory ($RELEASE_DIR)..."
    rm -rf "$RELEASE_DIR"
fi

mkdir -p "$RELEASE_DIR"
mkdir -p "$RELEASE_DIR/keys"

TMP_DIR="$(mktemp -d)"
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

DESKTOP_ISO_STAGING="$TMP_DIR/desktop_iso"
SERVER_ISO_STAGING="$TMP_DIR/server_iso"
mkdir -p "$DESKTOP_ISO_STAGING/boot/grub"
mkdir -p "$DESKTOP_ISO_STAGING/desktop/packages"
mkdir -p "$DESKTOP_ISO_STAGING/desktop/themes"
mkdir -p "$SERVER_ISO_STAGING/boot/grub"
mkdir -p "$SERVER_ISO_STAGING/cloud-init"
mkdir -p "$SERVER_ISO_STAGING/etc/novad"

# ------------------------------------------------------------------------------
# Phase 4: Build Desktop Hybrid UEFI/BIOS ISO (novaos-desktop-2.0.iso)
# ------------------------------------------------------------------------------
step "Building Desktop Edition Hybrid ISO (novaos-desktop-2.0.iso)"

cp build/novaos.bin "$DESKTOP_ISO_STAGING/boot/novaos.bin"
cp build/novaos.bin "$DESKTOP_ISO_STAGING/boot/novaos_b.bin"

# Desktop GRUB configuration
cat << 'EOF' > "$DESKTOP_ISO_STAGING/boot/grub/grub.cfg"
set timeout=5
set default=0

# Fluent dark color scheme styling
set menu_color_normal=light-gray/black
set menu_color_highlight=white/dark-gray

menuentry "NovaOS 2.0 Desktop Edition (Wayland Compositor & Fluent Desktop)" {
    multiboot /boot/novaos.bin edition=desktop vga=std
    boot
}

menuentry "NovaOS 2.0 Desktop Edition (Slot B - Alternate / Staged Update)" {
    multiboot /boot/novaos_b.bin edition=desktop vga=std
    boot
}

menuentry "NovaOS 2.0 Graphical Live Installer (nova-installer)" {
    multiboot /boot/novaos.bin edition=desktop mode=installer vga=std
    boot
}

menuentry "NovaOS 2.0 Desktop (Safe Graphics Mode - 1024x768 VESA)" {
    multiboot /boot/novaos.bin edition=desktop nomodeset vga=792
    boot
}
EOF

# Desktop userland payload
cp tools/fixtures/SYSTEM.CFG "$DESKTOP_ISO_STAGING/desktop/SYSTEM.CFG"
if [[ -f tools/fixtures/COMPOSITOR.ELF ]]; then
    cp tools/fixtures/COMPOSITOR.ELF "$DESKTOP_ISO_STAGING/desktop/COMPOSITOR.ELF"
fi
for pkg in tools/fixtures/*.PKG; do
    [[ -f "$pkg" ]] && cp "$pkg" "$DESKTOP_ISO_STAGING/desktop/packages/"
done

cat << EOF > "$DESKTOP_ISO_STAGING/VERSION.TXT"
NovaOS Desktop Edition v$VERSION
Build Date: $DATE_STAMP
Architecture: x86 (IA-32) Multiboot / Hybrid UEFI-BIOS
Target: High-Performance Graphical Workstation & Development Environment
Features: Wayland Compositor, Fluent 2.0 Dark UI, macOS-style Dock, 3D VirtIO-GPU, Sound Blaster 16
EOF

cat << 'EOF' > "$DESKTOP_ISO_STAGING/README.TXT"
NovaOS 2.0 Desktop Edition Live & Installation Media
====================================================
This hybrid ISO image boots on both legacy BIOS and modern UEFI hardware.

Boot Options:
- Press Enter to launch the live desktop workstation with full compositor.
- Select "Graphical Live Installer" to partition your drive and install NovaOS.
- Select "Safe Graphics Mode" if running on legacy or unaccelerated hypervisors.
EOF

cat << 'EOF' > "$DESKTOP_ISO_STAGING/INSTALL.TXT"
NovaOS Desktop Installation Quickstart:
1. Boot the ISO via USB drive or hypervisor virtual CD-ROM.
2. At the desktop prompt or installer wizard, select your target NVMe or SATA disk.
3. The installer creates a standard GPT layout with EFI System Partition (ESP),
   NovaFS/FAT32 System partition, and ext2 root volume.
4. Reboot and remove the installation media.
EOF

DESKTOP_ISO_OUT="$RELEASE_DIR/novaos-desktop-2.0.iso"
echo "Creating hybrid desktop ISO with grub-mkrescue..."
grub-mkrescue -o "$DESKTOP_ISO_OUT" "$DESKTOP_ISO_STAGING" 2>/dev/null

DESKTOP_ISO_SZ=$(wc -c < "$DESKTOP_ISO_OUT")
echo "✅ Desktop Hybrid ISO generated: $DESKTOP_ISO_OUT ($DESKTOP_ISO_SZ bytes)"

# Validate El Torito dual boot on generated desktop ISO
"$NOVARELEASE_BIN" inspect-eltorito "$DESKTOP_ISO_OUT"

# ------------------------------------------------------------------------------
# Phase 5: Build Server Hybrid UEFI/BIOS ISO (novaos-server-2.0.iso)
# ------------------------------------------------------------------------------
step "Building Server Edition Hybrid ISO (novaos-server-2.0.iso)"

cp build/novaos-server.bin "$SERVER_ISO_STAGING/boot/novaos.bin"
cp build/novaos-server.bin "$SERVER_ISO_STAGING/boot/novaos_b.bin"

# Server GRUB configuration with headless serial console support
cat << 'EOF' > "$SERVER_ISO_STAGING/boot/grub/grub.cfg"
set timeout=5
set default=0

# Configure dual console (Serial ttyS0 at 115200 baud + VGA display)
serial --unit=0 --speed=115200 --word=8 --parity=no --stop=1
terminal_input serial console
terminal_output serial console

menuentry "NovaOS 2.0 Server Edition (Headless Cloud & Microservices)" {
    multiboot /boot/novaos.bin edition=server console=ttyS0,115200
    boot
}

menuentry "NovaOS 2.0 Server Edition (Slot B - Alternate / Staged Update)" {
    multiboot /boot/novaos_b.bin edition=server console=ttyS0,115200
    boot
}

menuentry "NovaOS 2.0 Server Automated Install & Cloud-Init Bootstrap" {
    multiboot /boot/novaos.bin edition=server autoinstall ds=nocloud-net console=ttyS0,115200
    boot
}

menuentry "NovaOS 2.0 Server Emergency Rescue Shell" {
    multiboot /boot/novaos.bin edition=server single rescue console=ttyS0,115200
    boot
}
EOF

# Server cloud-init configuration templates
cat << 'EOF' > "$SERVER_ISO_STAGING/cloud-init/meta-data"
instance-id: novaserver-node-01
local-hostname: novaserver-node1
cloud-name: novaos-cloud
region: enterprise-datacenter-1
EOF

cat << 'EOF' > "$SERVER_ISO_STAGING/cloud-init/user-data"
#cloud-config
version: 2.0
manage_etc_hosts: true
users:
  - name: novaadmin
    gecos: NovaOS Administrator
    groups: [wheel, sudo, novactl]
    shell: /bin/novash
    ssh_authorized_keys:
      - ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIExampleNovaOSServerKeyAdmin2026 enterprise@novaos.org
write_files:
  - path: /etc/novafw.conf
    permissions: '0640'
    content: |
      [firewall]
      default_inbound = DROP
      default_outbound = ACCEPT
      allow tcp port 22
      allow tcp port 9100
      allow udp port 51820
runcmd:
  - [ novactl, service, enable, novasshd ]
  - [ novactl, service, enable, novamon ]
  - [ novactl, service, start, novad ]
EOF

cat << 'EOF' > "$SERVER_ISO_STAGING/cloud-init/network-config"
version: 2
ethernets:
  eth0:
    dhcp4: true
    dhcp6: false
EOF

# System service definitions for novad
cat << 'EOF' > "$SERVER_ISO_STAGING/etc/novad/novasshd.service"
[Unit]
Description=Pure Rust OpenSSH-Compatible SSH-2 Server Daemon
After=network.target novafw.service

[Service]
Type=simple
ExecStart=/bin/novasshd --port 22 --config /etc/ssh/sshd_config
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

cat << 'EOF' > "$SERVER_ISO_STAGING/etc/novad/novamon.service"
[Unit]
Description=Prometheus Telemetry Exporter & Health Metrics Agent
After=network.target

[Service]
Type=simple
ExecStart=/bin/novamon --port 9100 --metrics-path /metrics
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF

cat << 'EOF' > "$SERVER_ISO_STAGING/etc/novad/novafw.service"
[Unit]
Description=Stateful Packet-Filter Firewall & Conntrack Engine
Before=network.target

[Service]
Type=oneshot
ExecStart=/bin/novafw load /etc/novafw.conf
RemainAfterExit=yes

[Install]
WantedBy=network.target
EOF

cat << EOF > "$SERVER_ISO_STAGING/VERSION.TXT"
NovaOS Server Edition v$VERSION
Build Date: $DATE_STAMP
Architecture: x86 (IA-32) Multiboot / Dedicated Headless Server Kernel
Target: Enterprise Cloud Infrastructure, Bare-Metal Servers & Microservices
Features: novad init orchestrator, novasshd SSH server, WireGuard kernel engine,
          nftables-style conntrack firewall, Prometheus metrics exporter, software RAID
EOF

cat << 'EOF' > "$SERVER_ISO_STAGING/README.TXT"
NovaOS 2.0 Server Edition
=========================
Optimized for high-density microservices, container virtualization, and cloud hypervisors.
Features serial console redirection (ttyS0, 115200 baud) for out-of-band management.
EOF

SERVER_ISO_OUT="$RELEASE_DIR/novaos-server-2.0.iso"
echo "Creating hybrid server ISO with grub-mkrescue..."
grub-mkrescue -o "$SERVER_ISO_OUT" "$SERVER_ISO_STAGING" 2>/dev/null

SERVER_ISO_SZ=$(wc -c < "$SERVER_ISO_OUT")
echo "✅ Server Hybrid ISO generated: $SERVER_ISO_OUT ($SERVER_ISO_SZ bytes)"

# Validate El Torito dual boot on generated server ISO
"$NOVARELEASE_BIN" inspect-eltorito "$SERVER_ISO_OUT"

# ------------------------------------------------------------------------------
# Helper Function: Build Partitioned GPT Hybrid Disk Image
# ------------------------------------------------------------------------------
build_gpt_disk_image() {
    local TARGET_IMG="$1"
    local EDITION_TYPE="$2"
    echo "Assembling partitioned GPT disk image for $EDITION_TYPE: $TARGET_IMG..."

    local ESP_MB=32
    local SYS_MB=64
    local DATA_MB=32
    local JNL_MB=4
    local CRASH_MB=1
    local TOTAL_MB=$((1 + ESP_MB + SYS_MB + DATA_MB + JNL_MB + CRASH_MB + 1)) # 135MB

    local DISK_TMP="$TMP_DIR/gpt_$EDITION_TYPE"
    mkdir -p "$DISK_TMP"

    # 1. Partition 1: EFI System Partition (ESP) formatted FAT32
    local ESP_IMG="$DISK_TMP/esp.img"
    dd if=/dev/zero of="$ESP_IMG" bs=1M count=$ESP_MB status=none
    mformat -i "$ESP_IMG" -F ::
    mmd -i "$ESP_IMG" ::/EFI
    mmd -i "$ESP_IMG" ::/EFI/BOOT
    mmd -i "$ESP_IMG" ::/EFI/NOVAOS

    cat << EOF > "$DISK_TMP/grub_esp.cfg"
set timeout=3
set default=0
menuentry "NovaOS 2.0 ($EDITION_TYPE - UEFI)" {
    search --no-floppy --set=root --label NOVAOS_SYS
    multiboot /boot/novaos.bin edition=$EDITION_TYPE
    boot
}
EOF
    mcopy -i "$ESP_IMG" "$DISK_TMP/grub_esp.cfg" "::/EFI/NOVAOS/GRUB.CFG"
    mcopy -i "$ESP_IMG" "$DISK_TMP/grub_esp.cfg" "::/EFI/BOOT/BOOT.CFG"

    # 2. Partition 2: NovaOS System Partition (FAT32)
    local SYS_IMG="$DISK_TMP/sys.img"
    dd if=/dev/zero of="$SYS_IMG" bs=1M count=$SYS_MB status=none
    mformat -i "$SYS_IMG" -F ::

    # Copy files according to edition
    if [[ "$EDITION_TYPE" == "server" ]]; then
        echo "NovaOS Server Edition (x86_32 headless)" > "$DISK_TMP/EDITION.TXT"
        mcopy -i "$SYS_IMG" "$DISK_TMP/EDITION.TXT" "::EDITION.TXT"
        mcopy -i "$SYS_IMG" "$SERVER_ISO_STAGING/cloud-init/user-data" "::USERDATA.CFG"
        mcopy -i "$SYS_IMG" "$SERVER_ISO_STAGING/VERSION.TXT" "::VERSION.TXT"
    else
        echo "NovaOS Desktop Edition (x86_32 GUI)" > "$DISK_TMP/EDITION.TXT"
        mcopy -i "$SYS_IMG" "$DISK_TMP/EDITION.TXT" "::EDITION.TXT"
        mcopy -i "$SYS_IMG" tools/fixtures/SYSTEM.CFG "::SYSTEM.CFG"
        mcopy -i "$SYS_IMG" "$DESKTOP_ISO_STAGING/VERSION.TXT" "::VERSION.TXT"
        if [[ -f tools/fixtures/COMPOSITOR.ELF ]]; then
            mcopy -i "$SYS_IMG" tools/fixtures/COMPOSITOR.ELF "::COMPOSITOR.ELF"
        fi
    fi
    mcopy -i "$SYS_IMG" tools/fixtures/HELLO.TXT "::HELLO.TXT"
    if [[ -f tools/fixtures/HELLO.ELF ]]; then
        mcopy -i "$SYS_IMG" tools/fixtures/HELLO.ELF "::HELLO.ELF"
    fi

    # 3. Partition 3: ext2 Root Partition
    local DATA_IMG="$DISK_TMP/data.img"
    dd if=/dev/zero of="$DATA_IMG" bs=1M count=$DATA_MB status=none
    mkfs.ext2 -q -F "$DATA_IMG"
    for f in tools/fixtures/ext2root/*; do
        if [[ -f "$f" ]]; then
            local bname="$(basename "$f")"
            debugfs -w -R "write $f $bname" "$DATA_IMG" >/dev/null 2>&1 || true
        fi
    done

    # 4. Assemble Whole Disk with GPT Table
    dd if=/dev/zero of="$TARGET_IMG" bs=1M count=$TOTAL_MB status=none
    parted -s "$TARGET_IMG" mklabel gpt >/dev/null 2>&1

    # Define GPT Partitions:
    # 1: ESP (1MiB -> 33MiB)
    parted -s "$TARGET_IMG" mkpart primary fat32 1MiB $((1 + ESP_MB))MiB >/dev/null 2>&1
    parted -s "$TARGET_IMG" set 1 esp on >/dev/null 2>&1
    parted -s "$TARGET_IMG" name 1 ESP >/dev/null 2>&1

    # 2: NOVAOS_SYS (33MiB -> 97MiB)
    parted -s "$TARGET_IMG" mkpart primary fat32 $((1 + ESP_MB))MiB $((1 + ESP_MB + SYS_MB))MiB >/dev/null 2>&1
    parted -s "$TARGET_IMG" name 2 NOVAOS_SYS >/dev/null 2>&1

    # 3: NOVAOS_DATA (97MiB -> 129MiB)
    parted -s "$TARGET_IMG" mkpart primary ext2 $((1 + ESP_MB + SYS_MB))MiB $((1 + ESP_MB + SYS_MB + DATA_MB))MiB >/dev/null 2>&1
    parted -s "$TARGET_IMG" name 3 NOVAOS_DATA >/dev/null 2>&1

    # 4: NOVAOS_JNL (129MiB -> 133MiB)
    parted -s "$TARGET_IMG" mkpart primary ext2 $((1 + ESP_MB + SYS_MB + DATA_MB))MiB $((1 + ESP_MB + SYS_MB + DATA_MB + JNL_MB))MiB >/dev/null 2>&1
    parted -s "$TARGET_IMG" name 4 NOVAOS_JNL >/dev/null 2>&1

    # 5: NOVAOS_CRASH (133MiB -> 134MiB)
    parted -s "$TARGET_IMG" mkpart primary ext2 $((1 + ESP_MB + SYS_MB + DATA_MB + JNL_MB))MiB $((1 + ESP_MB + SYS_MB + DATA_MB + JNL_MB + CRASH_MB))MiB >/dev/null 2>&1
    parted -s "$TARGET_IMG" name 5 NOVAOS_CRASH >/dev/null 2>&1

    # Read back exact sector offsets from parted
    local P1_START=$(parted -s "$TARGET_IMG" unit s print | awk '/^ 1/ {gsub("s","",$2); print $2}')
    local P2_START=$(parted -s "$TARGET_IMG" unit s print | awk '/^ 2/ {gsub("s","",$2); print $2}')
    local P3_START=$(parted -s "$TARGET_IMG" unit s print | awk '/^ 3/ {gsub("s","",$2); print $2}')

    dd if="$ESP_IMG" of="$TARGET_IMG" bs=512 seek="$P1_START" conv=notrunc status=none
    dd if="$SYS_IMG" of="$TARGET_IMG" bs=512 seek="$P2_START" conv=notrunc status=none
    dd if="$DATA_IMG" of="$TARGET_IMG" bs=512 seek="$P3_START" conv=notrunc status=none

    echo "✅ GPT disk image created: $TARGET_IMG (ESP at sector $P1_START, SYS at sector $P2_START, DATA at sector $P3_START)"
}

# ------------------------------------------------------------------------------
# Phase 6: Build Desktop GPT Disk Image (novaos-desktop-2.0.img)
# ------------------------------------------------------------------------------
step "Building Desktop GPT Hybrid Disk Image (novaos-desktop-2.0.img)"
DESKTOP_IMG_OUT="$RELEASE_DIR/novaos-desktop-2.0.img"
build_gpt_disk_image "$DESKTOP_IMG_OUT" "desktop"
"$NOVARELEASE_BIN" inspect-gpt "$DESKTOP_IMG_OUT"

# ------------------------------------------------------------------------------
# Phase 7: Build Server GPT Disk Image (novaos-server-2.0.img)
# ------------------------------------------------------------------------------
step "Building Server GPT Hybrid Disk Image (novaos-server-2.0.img)"
SERVER_IMG_OUT="$RELEASE_DIR/novaos-server-2.0.img"
build_gpt_disk_image "$SERVER_IMG_OUT" "server"
"$NOVARELEASE_BIN" inspect-gpt "$SERVER_IMG_OUT"

# ------------------------------------------------------------------------------
# Phase 8: Package Standalone Kernel Binaries & Generate Manifest
# ------------------------------------------------------------------------------
step "Packaging Kernel Binaries and Generating Release Manifest"

cp build/novaos.bin "$RELEASE_DIR/novaos.bin"
cp build/novaos-server.bin "$RELEASE_DIR/novaos-server.bin"

MANIFEST_PATH="$RELEASE_DIR/release_manifest.json"
echo "Generating JSON release manifest with novarelease..."
"$NOVARELEASE_BIN" generate-manifest --dir "$RELEASE_DIR" --version "$VERSION" --out "$MANIFEST_PATH"

# ------------------------------------------------------------------------------
# Phase 9: Cryptographic Checksums & GPG Signatures
# ------------------------------------------------------------------------------
step "Generating Cryptographic Checksums and GPG Detached Signatures"

cd "$RELEASE_DIR"

echo "Calculating SHA-256 digests..."
sha256sum novaos-desktop-2.0.iso > novaos-desktop-2.0.iso.sha256
sha256sum novaos-server-2.0.iso > novaos-server-2.0.iso.sha256
sha256sum novaos-desktop-2.0.img > novaos-desktop-2.0.img.sha256
sha256sum novaos-server-2.0.img > novaos-server-2.0.img.sha256
sha256sum novaos.bin > novaos.bin.sha256
sha256sum novaos-server.bin > novaos-server.bin.sha256
sha256sum release_manifest.json > release_manifest.json.sha256

sha256sum novaos-desktop-2.0.iso novaos-server-2.0.iso novaos-desktop-2.0.img novaos-server-2.0.img novaos.bin novaos-server.bin release_manifest.json > SHA256SUMS

echo "Verifying SHA256SUMS file..."
sha256sum -c SHA256SUMS >/dev/null
echo "✅ SHA-256 checksums verified."

# GPG Signing
GPG_HOMEDIR="$REPO_ROOT/$RELEASE_DIR/keys/gnupg"
mkdir -p "$GPG_HOMEDIR"
chmod 700 "$GPG_HOMEDIR"

# Check if release key already exists in dedicated keyring; if not, generate dedicated ed25519 release key
if ! gpg --homedir "$GPG_HOMEDIR" --list-keys "$GPG_KEY_EMAIL" &>/dev/null; then
    echo "Generating dedicated enterprise release signing key ($GPG_KEY_EMAIL)..."
    BATCH_FILE="$TMP_DIR/genkey.batch"
    cat << EOF > "$BATCH_FILE"
%no-protection
Key-Type: EDDSA
Key-Curve: ed25519
Key-Usage: sign
Name-Real: NovaOS Release Engineering
Name-Email: $GPG_KEY_EMAIL
Expire-Date: 0
%commit
EOF
    gpg --homedir "$GPG_HOMEDIR" --batch --generate-key "$BATCH_FILE" 2>/dev/null
fi

echo "Signing SHA256SUMS with GPG release key..."
gpg --homedir "$GPG_HOMEDIR" --batch --yes --detach-sign --armor --output SHA256SUMS.asc SHA256SUMS
gpg --homedir "$GPG_HOMEDIR" --batch --yes --detach-sign --output SHA256SUMS.sig SHA256SUMS

echo "Signing individual release images..."
gpg --homedir "$GPG_HOMEDIR" --batch --yes --detach-sign --output novaos-desktop-2.0.iso.sig novaos-desktop-2.0.iso
gpg --homedir "$GPG_HOMEDIR" --batch --yes --detach-sign --output novaos-server-2.0.iso.sig novaos-server-2.0.iso
gpg --homedir "$GPG_HOMEDIR" --batch --yes --detach-sign --output novaos-desktop-2.0.img.sig novaos-desktop-2.0.img
gpg --homedir "$GPG_HOMEDIR" --batch --yes --detach-sign --output novaos-server-2.0.img.sig novaos-server-2.0.img

echo "Exporting ASCII-armored release public key..."
gpg --homedir "$GPG_HOMEDIR" --armor --export "$GPG_KEY_EMAIL" > "novaos-release-2.0-key.asc"

echo "Verifying GPG signatures against exported key..."
gpg --homedir "$GPG_HOMEDIR" --verify SHA256SUMS.asc SHA256SUMS 2>/dev/null
gpg --homedir "$GPG_HOMEDIR" --verify SHA256SUMS.sig SHA256SUMS 2>/dev/null
gpg --homedir "$GPG_HOMEDIR" --verify novaos-desktop-2.0.iso.sig novaos-desktop-2.0.iso 2>/dev/null
gpg --homedir "$GPG_HOMEDIR" --verify novaos-server-2.0.iso.sig novaos-server-2.0.iso 2>/dev/null
echo "✅ All GPG detached signatures verified."

cd "$REPO_ROOT"

# ------------------------------------------------------------------------------
# Phase 10: Full Dual-Edition Audit
# ------------------------------------------------------------------------------
step "Executing Full Dual-Edition Release Media Audit"
"$NOVARELEASE_BIN" audit --dir "$RELEASE_DIR"

echo ""
echo "================================================================================"
echo "   🎉 NOVAOS 2.0 DUAL-EDITION RELEASE PACKAGING COMPLETE AND CERTIFIED!       "
echo "================================================================================"
echo "Artifact Directory: $RELEASE_DIR"
echo ""
echo "Release Deliverables:"
printf "  %-28s  %12s  %s\n" "Filename" "Size" "Format / Target"
echo "  ----------------------------------------------------------------------------"
printf "  %-28s  %12s  %s\n" "novaos-desktop-2.0.iso" "$(du -h "$RELEASE_DIR/novaos-desktop-2.0.iso" | awk '{print $1}')" "Hybrid UEFI/BIOS Desktop Installer ISO"
printf "  %-28s  %12s  %s\n" "novaos-server-2.0.iso"  "$(du -h "$RELEASE_DIR/novaos-server-2.0.iso"  | awk '{print $1}')" "Hybrid UEFI/BIOS Headless Server ISO"
printf "  %-28s  %12s  %s\n" "novaos-desktop-2.0.img" "$(du -h "$RELEASE_DIR/novaos-desktop-2.0.img" | awk '{print $1}')" "GPT Partitioned Desktop Disk Image"
printf "  %-28s  %12s  %s\n" "novaos-server-2.0.img"  "$(du -h "$RELEASE_DIR/novaos-server-2.0.img"  | awk '{print $1}')" "GPT Partitioned Headless Server Disk Image"
printf "  %-28s  %12s  %s\n" "novaos.bin"              "$(du -h "$RELEASE_DIR/novaos.bin"              | awk '{print $1}')" "Desktop Multiboot Kernel ELF Binary"
printf "  %-28s  %12s  %s\n" "novaos-server.bin"       "$(du -h "$RELEASE_DIR/novaos-server.bin"       | awk '{print $1}')" "Server Multiboot Kernel ELF Binary"
printf "  %-28s  %12s  %s\n" "release_manifest.json"   "$(du -h "$RELEASE_DIR/release_manifest.json"   | awk '{print $1}')" "Cryptographic Release Manifest"
printf "  %-28s  %12s  %s\n" "SHA256SUMS"              "$(du -h "$RELEASE_DIR/SHA256SUMS"              | awk '{print $1}')" "Cryptographic SHA-256 Digest Matrix"
printf "  %-28s  %12s  %s\n" "SHA256SUMS.asc"          "$(du -h "$RELEASE_DIR/SHA256SUMS.asc"          | awk '{print $1}')" "Detached GPG Signature (ASCII-Armored)"
printf "  %-28s  %12s  %s\n" "novaos-release-2.0-key.asc" "$(du -h "$RELEASE_DIR/novaos-release-2.0-key.asc" | awk '{print $1}')" "GPG Release Engineering Public Key"
echo "================================================================================"
exit 0
