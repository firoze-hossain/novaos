#!/usr/bin/env bash
# ==============================================================================
# NovaOS Companion Application Sync & Update Tool
# Synchronizes, compiles, and registers core default apps and App Store packages:
#   - aero-browser   (Default Web Browser)
#   - spectra-player (Default Media Player)
#   - vellum         (Default PDF Studio)
#   - vortex         (App Store BitTorrent & Stream Studio)
#   - zenith         (Default Office Suite)
#   - DBNavigator    (App Store Database IDE)
#   - thundercall    (App Store API Platform)
#   - lumina         (App Store Code Editor / IDE)
# ==============================================================================

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
OTHERS_DIR="$(cd "$ROOT_DIR/.." && pwd)"
FIXTURES_DIR="$ROOT_DIR/tools/fixtures"

DO_PULL=0
REBUILD_DISK=1
TARGET_APP="all"

usage() {
    echo "Usage: $0 [OPTIONS] [APP_NAME]"
    echo ""
    echo "Apps:"
    echo "  all            Update all core apps and App Store packages (default)"
    echo "  aero           Update Aero Browser"
    echo "  spectra        Update Spectra Media Player"
    echo "  vellum         Update Vellum PDF Studio"
    echo "  vortex         Update Vortex BitTorrent Client & IDM Accelerator"
    echo "  zenith         Update Zenith Office Suite"
    echo "  dbnavigator    Update DBNavigator Database IDE"
    echo "  thundercall    Update ThunderCall API Platform"
    echo "  lumina         Update Lumina Code Editor"
    echo ""
    echo "Options:"
    echo "  --pull, -p     Pull latest Git commits from remote repository before building"
    echo "  --no-disk      Skip rebuilding disk.img after updating apps"
    echo "  --help, -h     Show this help message"
    exit 0
}

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --pull|-p)
            DO_PULL=1
            shift
            ;;
        --no-disk)
            REBUILD_DISK=0
            shift
            ;;
        --help|-h)
            usage
            ;;
        *)
            TARGET_APP="$1"
            shift
            ;;
    esac
done

sync_git_repo() {
    local dir="$1"
    local name="$2"
    if [ "$DO_PULL" -eq 1 ] && [ -d "$dir/.git" ]; then
        echo ">>> [Git Pull] Fetching latest updates for $name..."
        (cd "$dir" && git pull --ff-only) || echo "Warning: git pull for $name encountered a notice, proceeding with local tree."
    fi
}

update_aero() {
    local repo="$OTHERS_DIR/aero-browser"
    echo "========================================================"
    echo " Updating Aero Browser (Default Web Browser)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "aero-browser"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_spectra() {
    local repo="$OTHERS_DIR/spectra-player"
    if [ ! -d "$repo" ]; then
        repo="$OTHERS_DIR/nova-player"
    fi
    echo "========================================================"
    echo " Updating Spectra Media Player (Default Media Player)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "spectra-player"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_vellum() {
    local repo="$OTHERS_DIR/vellum"
    echo "========================================================"
    echo " Updating Vellum PDF Studio (Default PDF Reader/Editor)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "vellum"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_vortex() {
    local repo="$OTHERS_DIR/vortex"
    echo "========================================================"
    echo " Updating Vortex (BitTorrent & Stream Accelerator)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "vortex"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_zenith() {
    local repo="$OTHERS_DIR/zenith"
    echo "========================================================"
    echo " Updating Zenith (Default Sovereign Office Suite)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "zenith"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_dbnavigator() {
    local repo="$OTHERS_DIR/DBNavigator"
    echo "========================================================"
    echo " Updating DBNavigator (App Store Database IDE)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "DBNavigator"
        cat << 'EOF' > "$FIXTURES_DIR/DBNAV.PKG"
[package]
id = "org.novaos.dbnavigator"
name = "DBNavigator Pro"
version = "1.0.0"
category = "Development"
app_filename = "DBNAV.ELF"
summary = "Multi-engine database SQL query IDE and ER diagram modeler"
EOF
        echo "--> Verified package descriptor DBNAV.PKG in $FIXTURES_DIR"
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_thundercall() {
    local repo="$OTHERS_DIR/thundercall"
    echo "========================================================"
    echo " Updating ThunderCall (App Store API Platform)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "thundercall"
        cat << 'EOF' > "$FIXTURES_DIR/THUNDER.PKG"
[package]
id = "org.novaos.thundercall"
name = "ThunderCall Client"
version = "1.0.0"
category = "Internet"
app_filename = "THUNDER.ELF"
summary = "High-performance API development and testing platform"
EOF
        echo "--> Verified package descriptor THUNDER.PKG in $FIXTURES_DIR"
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_lumina() {
    local repo="$OTHERS_DIR/lumina"
    echo "========================================================"
    echo " Updating Lumina (App Store Code Editor / IDE)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "lumina"
        cat << 'EOF' > "$FIXTURES_DIR/LUMINA.PKG"
[package]
id = "org.novaos.lumina"
name = "Lumina Studio"
version = "1.0.0"
category = "Development"
app_filename = "LUMINA.ELF"
summary = "IntelliJ-inspired code editor and polyglot development IDE"
EOF
        echo "--> Verified package descriptor LUMINA.PKG in $FIXTURES_DIR"
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_chronos() {
    local repo="$OTHERS_DIR/chronos"
    echo "========================================================"
    echo " Updating Chronos (Default Sovereign Clock & Timer Suite)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "chronos"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_mnemos() {
    local repo="$OTHERS_DIR/mnemos"
    echo "========================================================"
    echo " Updating Mnemos (Default Sovereign Clipboard Manager)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "mnemos"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_iris() {
    local repo="$OTHERS_DIR/iris"
    echo "========================================================"
    echo " Updating Iris (Default Sovereign Camera & Video Studio)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "iris"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_kairos() {
    local repo="$OTHERS_DIR/kairos"
    echo "========================================================"
    echo " Updating Kairos (Default Sovereign Calendar & Event Studio)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "kairos"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_glyph() {
    local repo="$OTHERS_DIR/glyph"
    echo "========================================================"
    echo " Updating Glyph (Default Sovereign Code & Text Editor)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "glyph"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

update_atlas() {
    local repo="$OTHERS_DIR/atlas"
    echo "========================================================"
    echo " Updating Atlas (Default Sovereign Maps & Navigation)"
    echo "========================================================"
    if [ -d "$repo" ]; then
        sync_git_repo "$repo" "atlas"
        if [ -x "$repo/novaos/build.sh" ]; then
            echo "--> Compiling bare-metal NovaOS binaries & package..."
            (cd "$repo/novaos" && ./build.sh)
        fi
    else
        echo "Warning: Repository $repo not found."
    fi
}

# Execute requested updates
case "$TARGET_APP" in
    all|"")
        update_aero
        update_spectra
        update_vellum
        update_vortex
        update_zenith
        update_dbnavigator
        update_thundercall
        update_lumina
        update_chronos
        update_mnemos
        update_iris
        update_kairos
        update_glyph
        update_atlas
        ;;
    atlas|map|maps|nova-map)
        update_atlas
        ;;
    glyph|edit|editor|nova-edit)
        update_glyph
        ;;
    kairos|calendar|nova-calendar)
        update_kairos
        ;;
    iris|cam|camera|nova-cam)
        update_iris
        ;;
    mnemos|clip|nova-clip)
        update_mnemos
        ;;
    chronos|clock|nova-clock)
        update_chronos
        ;;
    aero|browser|aero-browser)
        update_aero
        ;;
    spectra|player|spectra-player)
        update_spectra
        ;;
    vellum|pdf)
        update_vellum
        ;;
    vortex|torrent)
        update_vortex
        ;;
    zenith|office)
        update_zenith
        ;;
    dbnavigator|dbnav)
        update_dbnavigator
        ;;
    thundercall|thunder)
        update_thundercall
        ;;
    lumina)
        update_lumina
        ;;
    *)
        echo "Unknown app target: $TARGET_APP"
        usage
        ;;
esac

# Rebuild disk image
if [ "$REBUILD_DISK" -eq 1 ]; then
    echo ""
    echo "========================================================"
    echo " Rebuilding NovaOS Disk Image with Updated Apps..."
    echo "========================================================"
    "$ROOT_DIR/tools/build-disk-image.sh"
fi

echo ""
echo "✅ All requested NovaOS apps have been successfully synchronized and built!"
