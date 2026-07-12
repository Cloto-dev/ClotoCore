#!/bin/bash
# ============================================================
# Cloto Quick Installer
# Resolves a release through the signed update manifest and
# installs the pre-built kernel binary.
#
# Usage:
#   bash install.sh
#
# Environment variables:
#   CLOTO_PREFIX   Install directory (default: /opt/cloto)
#   CLOTO_CHANNEL  Release channel: stable | current | experimental
#                  (default: stable)
#   CLOTO_VERSION  Pin an exact version (overrides CLOTO_CHANNEL)
#   CLOTO_SERVICE  Set to "true" to register as systemd service
# ============================================================
set -euo pipefail

REPO="Cloto-dev/ClotoCore"
INSTALL_DIR="${CLOTO_PREFIX:-/opt/cloto}"
CHANNEL="${CLOTO_CHANNEL:-stable}"
VERSION="${CLOTO_VERSION:-}"
SETUP_SERVICE="${CLOTO_SERVICE:-false}"
MANIFEST_URL="https://github.com/${REPO}/releases/download/updater-feed/manifest.json"

RED='\033[0;31m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[1;33m'
NC='\033[0m'

error() { echo -e "${RED}Error: $1${NC}" >&2; exit 1; }

# --- Preconditions ---
# python3 parses the release manifest. ClotoCore itself requires Python for
# MCP servers, so this adds no new dependency to the target machine.
command -v python3 &>/dev/null || error "python3 is required (it parses the release manifest)"

case "$CHANNEL" in
    stable|current|experimental) ;;
    *) error "Invalid CLOTO_CHANNEL: '$CHANNEL'. Expected stable, current, or experimental" ;;
esac

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
echo -e "${CYAN}Cloto Installer${NC}"
echo "  Platform: ${PLATFORM}"

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

# --- Resolve version + artifact via the signed update manifest (§5.2) ---
# The manifest indexes every release (version, tier, per-platform kernel
# URL + sha256) and the channel → version mapping. No GitHub API call, so
# no unauthenticated rate limit.
if [[ -n "$VERSION" && "$VERSION" != "latest" ]]; then
    echo "  Version:  ${VERSION} (pinned via CLOTO_VERSION)"
else
    VERSION=""
    echo "  Channel:  ${CHANNEL}"
fi

echo "  Fetching release manifest..."
curl -fsSL -o "${TMPDIR}/manifest.json" "$MANIFEST_URL" \
    || error "Failed to fetch the release manifest from ${MANIFEST_URL}"

# stdout carries "version url sha256"; resolution errors go to stderr and
# reach the terminal directly.
RESOLVED="$(python3 - "${TMPDIR}/manifest.json" "$CHANNEL" "${VERSION#v}" "$PLATFORM" <<'PY'
import json
import sys

manifest_path, channel, pin, platform = sys.argv[1:5]
with open(manifest_path) as f:
    manifest = json.load(f)

if pin:
    version = pin
else:
    version = manifest.get("channels", {}).get(channel)
    if not version:
        sys.exit(f"channel '{channel}' has no resolvable version in the manifest")

release = next(
    (r for r in manifest.get("releases", []) if r.get("version") == version), None
)
if release is None:
    sys.exit(f"version {version} not found in the release manifest")

kernel = release.get("kernel") or {}
asset = kernel.get(platform)
if not asset:
    available = ", ".join(sorted(kernel)) or "none"
    sys.exit(f"no kernel build for '{platform}' in v{version} (available: {available})")

print(version, asset["url"], asset["sha256"])
PY
)" || error "Version resolution failed (see message above)"

read -r VERSION_NUM URL SHA256 <<< "$RESOLVED"

# Validate version format (semver)
if ! [[ "$VERSION_NUM" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
    error "Invalid version format: '$VERSION_NUM'. Expected semver (e.g., 1.2.3)"
fi

echo "  Version:  v${VERSION_NUM}"

# --- Experimental warning (docs/RELEASE_PIPELINE_DESIGN.md §6) ---
# Fires on an explicit experimental channel request or a pre-release version pin.
if [[ "$CHANNEL" == "experimental" || "$VERSION_NUM" == *-* ]]; then
    echo ""
    echo -e "${YELLOW}WARNING: installing an EXPERIMENTAL (pre-release) build.${NC}"
    echo -e "${YELLOW}Experimental releases are opt-in only, with no guarantees.${NC}"
    echo -e "${YELLOW}Fixes ship in the next pre-release.${NC}"
fi

# --- Download ---
ARCHIVE="$(basename "$URL")"

echo ""
echo -e "${CYAN}Downloading ${ARCHIVE}...${NC}"
curl -fSL --progress-bar -o "${TMPDIR}/${ARCHIVE}" "$URL" \
    || error "Download failed. Check that version v${VERSION_NUM} exists at https://github.com/${REPO}/releases"

# --- Verify checksum against the manifest (mandatory) ---
echo "Verifying checksum..."
cd "$TMPDIR"
if command -v sha256sum &>/dev/null; then
    echo "${SHA256}  ${ARCHIVE}" | sha256sum -c - || error "Checksum verification failed"
else
    echo "${SHA256}  ${ARCHIVE}" | shasum -a 256 -c - || error "Checksum verification failed"
fi
cd - > /dev/null

# --- Extract ---
echo "Extracting..."
tar xzf "${TMPDIR}/${ARCHIVE}" -C "${TMPDIR}"

# --- Install via the binary's self-install command ---
EXTRACTED_DIR="${TMPDIR}/${ARCHIVE%.tar.gz}"

if [[ ! -f "${EXTRACTED_DIR}/clotocore" ]]; then
    error "Binary not found in archive"
fi

chmod +x "${EXTRACTED_DIR}/clotocore"

echo ""
echo -e "${CYAN}Installing to ${INSTALL_DIR}...${NC}"

# M-20: Use array to prevent word-splitting issues with paths containing spaces
INSTALL_ARGS=(install --prefix "${INSTALL_DIR}")
[[ "$SETUP_SERVICE" == "true" ]] && INSTALL_ARGS+=("--service")

# The binary's install command handles: file placement, .env generation,
# Python setup, and optional systemd service registration.
sudo "${EXTRACTED_DIR}/clotocore" "${INSTALL_ARGS[@]}"

echo ""
echo -e "${GREEN}ClotoCore v${VERSION_NUM} installed successfully.${NC}"
echo ""
echo -e "  Binary:    ${CYAN}${INSTALL_DIR}/clotocore${NC}"
echo -e "  Dashboard: ${CYAN}http://localhost:8081${NC}"
echo -e "  Manage:    ${CYAN}${INSTALL_DIR}/clotocore service start|stop|status${NC}"
echo -e "  Uninstall: ${CYAN}${INSTALL_DIR}/clotocore uninstall${NC}"
echo ""
