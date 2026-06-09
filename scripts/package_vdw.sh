#!/usr/bin/env bash
set -euo pipefail

# VDW CLI Packaging Script
# Builds and packages the vdw CLI binary for a target OS/architecture.

_HELP_TEXT="<os> <arch>"
_HELP_DESC="Package the VDW CLI binary.

Arguments:
  os    Target OS: darwin, linux, or win32
  arch  Target architecture: arm64 or x86_64

Examples:
  $(basename "$0") darwin arm64
  $(basename "$0") linux x86_64
  $(basename "$0") win32 x86_64"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

# ─── Validate Arguments ───────────────────────────────────────────────────────
if [[ $# -lt 2 ]]; then
    log_error "Missing required arguments."
    echo ""
    show_help
    exit 1
fi

OS="$1"
ARCH="$2"

# Validate OS
case "${OS}" in
    darwin|linux|win32) ;;
    *)
        log_error "Invalid OS '${OS}'. Must be one of: darwin, linux, win32"
        exit 1
        ;;
esac

# Validate ARCH
case "${ARCH}" in
    arm64|x86_64) ;;
    *)
        log_error "Invalid architecture '${ARCH}'. Must be one of: arm64, x86_64"
        exit 1
        ;;
esac

# ─── Version ──────────────────────────────────────────────────────────────────
VERSION="$(get_vdw_version)"
if [[ -z "${VERSION}" ]]; then
    log_error "Could not determine vdw version from Cargo.toml"
    exit 1
fi
log_info "VDW CLI version: ${VERSION}"

# ─── Build ────────────────────────────────────────────────────────────────────
log_info "Building vdw CLI (release)..."
cd "${PROJECT_ROOT}/vdw"
cargo build --release
log_info "Build complete."

# ─── Package ──────────────────────────────────────────────────────────────────
ensure_dist_dir

case "${OS}" in
    darwin|linux)
        ARTIFACT="$(get_artifact_name vdw "${OS}" "${ARCH}" "${VERSION}" tar.gz)"
        ARTIFACT_PATH="${PROJECT_ROOT}/dist/${ARTIFACT}"
        cd "${PROJECT_ROOT}/vdw/target/release"
        tar -czf "${ARTIFACT_PATH}" vdw
        log_info "Packaged: ${ARTIFACT_PATH}"
        ;;
    win32)
        log_warn "Cross-compilation for win32 may not be available on the current platform."
        log_warn "Ensure you have the appropriate Rust target installed (e.g., x86_64-pc-windows-msvc)."
        ARTIFACT="$(get_artifact_name vdw "${OS}" "${ARCH}" "${VERSION}" zip)"
        ARTIFACT_PATH="${PROJECT_ROOT}/dist/${ARTIFACT}"
        if command -v zip &>/dev/null; then
            cd "${PROJECT_ROOT}/vdw/target/release"
            zip -j "${ARTIFACT_PATH}" vdw.exe 2>/dev/null || {
                log_error "Could not find vdw.exe in target/release/. Cross-compilation may have failed."
                exit 1
            }
            log_info "Packaged: ${ARTIFACT_PATH}"
        else
            log_error "zip command not found. Install zip to package for win32."
            exit 1
        fi
        ;;
esac

# ─── Output ───────────────────────────────────────────────────────────────────
echo "${ARTIFACT_PATH}"
