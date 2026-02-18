#!/bin/bash
# ============================================================
# Exiv Quick Installer
# Downloads a pre-built binary from GitHub Releases and installs it.
#
# Usage:
#   bash install.sh
#
# Environment variables:
#   EXIV_PREFIX   Install directory (default: /opt/exiv)
#   EXIV_VERSION  Version to install (default: latest)
#   EXIV_SERVICE  Set to "true" to register as systemd service
# ============================================================
set -euo pipefail

REPO="Exiv-ai/Exiv"
INSTALL_DIR="${EXIV_PREFIX:-/opt/exiv}"
VERSION="${EXIV_VERSION:-latest}"
SETUP_SERVICE="${EXIV_SERVICE:-false}"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
NC='\033[0m'

error() { echo -e "${RED}Error: $1${NC}" >&2; exit 1; }

# --- Detect platform ---
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)
            case "$arch" in
                x86_64)       echo "linux-x64" ;;
                aarch64|arm64) echo "linux-arm64" ;;
                *) error "Unsupported architecture: $arch" ;;
            esac ;;
        Darwin)
            case "$arch" in
                x86_64)  echo "macos-x64" ;;
                arm64)   echo "macos-arm64" ;;
                *) error "Unsupported architecture: $arch" ;;
            esac ;;
        *)
            error "Unsupported OS: $os. Download the Windows build from https://github.com/${REPO}/releases" ;;
    esac
}

PLATFORM="$(detect_platform)"
echo -e "${CYAN}Exiv Installer${NC}"
echo "  Platform: ${PLATFORM}"

# --- Resolve version ---
if [[ "$VERSION" == "latest" ]]; then
    echo "  Resolving latest version..."
    VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d'"' -f4)" \
        || error "Failed to fetch latest release. Set EXIV_VERSION explicitly."
fi
VERSION_NUM="${VERSION#v}"

# Validate version format (semver)
if ! [[ "$VERSION_NUM" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    error "Invalid version format: '$VERSION_NUM'. Expected semver (e.g., 1.2.3)"
fi

echo "  Version:  v${VERSION_NUM}"

# --- Download ---
ARCHIVE="exiv-${VERSION_NUM}-${PLATFORM}.tar.gz"
URL="https://github.com/${REPO}/releases/download/v${VERSION_NUM}/${ARCHIVE}"
CHECKSUM_URL="${URL}.sha256"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

echo ""
echo -e "${CYAN}Downloading ${ARCHIVE}...${NC}"
curl -fSL --progress-bar -o "${TMPDIR}/${ARCHIVE}" "$URL" \
    || error "Download failed. Check that version v${VERSION_NUM} exists at https://github.com/${REPO}/releases"

curl -fsSL -o "${TMPDIR}/${ARCHIVE}.sha256" "$CHECKSUM_URL" 2>/dev/null || true

# --- Verify checksum ---
if [[ -s "${TMPDIR}/${ARCHIVE}.sha256" ]]; then
    echo "Verifying checksum..."
    cd "$TMPDIR"
    if command -v sha256sum &>/dev/null; then
        sha256sum -c "${ARCHIVE}.sha256" || error "Checksum verification failed"
    else
        shasum -a 256 -c "${ARCHIVE}.sha256" || error "Checksum verification failed"
    fi
    cd - > /dev/null
else
    echo "  (checksum file not available, skipping verification)"
fi

# --- Extract ---
echo "Extracting..."
tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}"

# --- Install via the binary's self-install command ---
EXTRACTED_DIR="${TMPDIR}/exiv-${VERSION_NUM}-${PLATFORM}"

if [[ ! -f "${EXTRACTED_DIR}/exiv_system" ]]; then
    error "Binary not found in archive"
fi

chmod +x "${EXTRACTED_DIR}/exiv_system"

echo ""
echo -e "${CYAN}Installing to ${INSTALL_DIR}...${NC}"

# M-20: Use array to prevent word-splitting issues with paths containing spaces
INSTALL_ARGS=(install --prefix "${INSTALL_DIR}")
[[ "$SETUP_SERVICE" == "true" ]] && INSTALL_ARGS+=("--service")

# The binary's install command handles: file placement, .env generation,
# Python setup, and optional systemd service registration.
sudo "${EXTRACTED_DIR}/exiv_system" "${INSTALL_ARGS[@]}"

echo ""
echo -e "${GREEN}Exiv v${VERSION_NUM} installed successfully.${NC}"
echo ""
echo -e "  Binary:    ${CYAN}${INSTALL_DIR}/exiv_system${NC}"
echo -e "  Dashboard: ${CYAN}http://localhost:8081${NC}"
echo -e "  Manage:    ${CYAN}${INSTALL_DIR}/exiv_system service start|stop|status${NC}"
echo -e "  Uninstall: ${CYAN}${INSTALL_DIR}/exiv_system uninstall${NC}"
echo ""
