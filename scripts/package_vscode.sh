#!/usr/bin/env bash
set -euo pipefail

# VDW VS Code Extension Packaging Script
# Builds and packages the vdw-vscode extension as a VSIX.

_HELP_TEXT=""
_HELP_DESC="Package the VDW VS Code extension as a VSIX file.

This script runs 'npm run package' inside vdw-vscode/ and copies
the resulting VSIX to dist/ with a standardized name.

Examples:
  $(basename "$0")"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

# ─── Version ──────────────────────────────────────────────────────────────────
VERSION="$(get_vscode_version)"
if [[ -z "${VERSION}" ]]; then
    log_error "Could not determine vdw-vscode version from package.json"
    exit 1
fi
log_info "VDW VS Code extension version: ${VERSION}"

# ─── Build VSIX ───────────────────────────────────────────────────────────────
log_info "Packaging VS Code extension..."
cd "${PROJECT_ROOT}/vdw-vscode"
npm run package
log_info "VSIX packaging complete."

# ─── Locate and rename ────────────────────────────────────────────────────────
ensure_dist_dir

# Find the generated VSIX (e.g., vdw-vscode-0.5.0.vsix)
VSIX_FILE=""
for f in vdw-vscode-*.vsix; do
    if [[ -f "$f" ]]; then
        VSIX_FILE="$f"
        break
    fi
done

if [[ -z "${VSIX_FILE}" ]]; then
    log_error "Could not find generated VSIX file in vdw-vscode/"
    exit 1
fi

ARTIFACT_NAME="vdw_vscode_${VERSION}.vsix"
ARTIFACT_PATH="${PROJECT_ROOT}/dist/${ARTIFACT_NAME}"

cp "${VSIX_FILE}" "${ARTIFACT_PATH}"
log_info "Copied to: ${ARTIFACT_PATH}"

# ─── Output ───────────────────────────────────────────────────────────────────
echo "${ARTIFACT_PATH}"
