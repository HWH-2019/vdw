#!/usr/bin/env bash
set -euo pipefail

# VDW macOS DMG Builder
# Creates a macOS DMG containing the vdw CLI binary.

_HELP_TEXT="<arch>"
_HELP_DESC="Build a macOS DMG for the VDW CLI.

Arguments:
  arch  Target architecture: arm64 or x86_64

Examples:
  $(basename "$0") arm64
  $(basename "$0") x86_64

Prerequisites:
  - macOS (Darwin)
  - hdiutil (included with macOS)"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

# ─── Guard: macOS only ────────────────────────────────────────────────────────
if [[ "$(uname -s)" != "Darwin" ]]; then
    log_error "DMG packaging is only supported on macOS."
    exit 1
fi

if ! command -v hdiutil &>/dev/null; then
    log_error "hdiutil not found. This tool is included with macOS."
    exit 1
fi

# ─── Validate Arguments ───────────────────────────────────────────────────────
if [[ $# -lt 1 ]]; then
    log_error "Missing required argument: arch"
    echo ""
    show_help
    exit 1
fi

ARCH="$1"

case "${ARCH}" in
    arm64|x86_64) ;;
    *)
        log_error "Invalid architecture '${ARCH}'. Must be one of: arm64, x86_64"
        exit 1
        ;;
esac

# ─── Build CLI binary first ──────────────────────────────────────────────────
log_info "Building vdw CLI binary via package_vdw.sh..."
"${SCRIPT_DIR}/package_vdw.sh" darwin "${ARCH}"

# ─── Version ──────────────────────────────────────────────────────────────────
VERSION="$(get_vdw_version)"
if [[ -z "${VERSION}" ]]; then
    log_error "Could not determine vdw version from Cargo.toml"
    exit 1
fi
log_info "VDW CLI version: ${VERSION}"

# ─── Prepare DMG contents ────────────────────────────────────────────────────
DMG_ROOT="/tmp/vdw_dmg_root"
ARTIFACT_NAME="$(get_artifact_name vdw darwin "${ARCH}" "${VERSION}" dmg)"
RAW_DMG_NAME="vdw_darwin_${ARCH}_${VERSION}_raw.dmg"
ARTIFACT_PATH="${PROJECT_ROOT}/dist/${ARTIFACT_NAME}"

ensure_dist_dir

cleanup() {
    log_info "Cleaning up temporary files..."
    rm -rf "${DMG_ROOT}"
    rm -f "/tmp/${RAW_DMG_NAME}"
}
trap cleanup EXIT

rm -rf "${DMG_ROOT}"
mkdir -p "${DMG_ROOT}"

# Copy the binary
cp "${PROJECT_ROOT}/vdw/target/release/vdw" "${DMG_ROOT}/vdw"
chmod +x "${DMG_ROOT}/vdw"

# Create a symlink to indicate install location
ln -s /usr/local/bin "${DMG_ROOT}/Install Here"

# ─── Create DMG ───────────────────────────────────────────────────────────────
log_info "Creating DMG..."
hdiutil create -srcfolder "${DMG_ROOT}" -volname "VDW ${VERSION}" \
    -fs HFS+ -format UDRW "/tmp/${RAW_DMG_NAME}"

log_info "Compressing DMG..."
hdiutil convert "/tmp/${RAW_DMG_NAME}" -format UDZO -o "${ARTIFACT_PATH}"

log_info "DMG created: ${ARTIFACT_PATH}"

# ─── Output ───────────────────────────────────────────────────────────────────
echo "${ARTIFACT_PATH}"
