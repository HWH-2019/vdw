#!/usr/bin/env bash
set -euo pipefail

# VDW One-Click Packaging Script
# Detects the current platform and packages all VDW components.

_HELP_TEXT=""
_HELP_DESC="Package all VDW components for the current platform.

This script:
  - Detects the current OS and architecture
  - macOS: builds a DMG for the CLI
  - Linux: builds a tar.gz for the CLI
  - Always packages the VS Code extension as a VSIX
  - Always packages the vdw-cli AI agent Skill as a tar.gz
  - Checks version consistency between CLI and VSIX
  - Prints a summary of all generated artifacts

Examples:
  $(basename "$0")"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

# ─── Detect Platform ──────────────────────────────────────────────────────────
OS_NAME="$(uname -s)"
ARCH_RAW="$(uname -m)"

# Map architecture
case "${ARCH_RAW}" in
    aarch64|arm64)
        ARCH="arm64"
        ;;
    x86_64|amd64)
        ARCH="x86_64"
        ;;
    *)
        log_error "Unsupported architecture: ${ARCH_RAW}"
        exit 1
        ;;
esac

log_info "Detected platform: ${OS_NAME} / ${ARCH}"

# ─── Version Consistency Check ────────────────────────────────────────────────
VDW_VERSION="$(get_vdw_version)"
VSCODE_VERSION="$(get_vscode_version)"
SKILL_VERSION="$(get_skill_version)"

log_info "CLI version:    ${VDW_VERSION}"
log_info "VSIX version:   ${VSCODE_VERSION}"
log_info "Skill version:  ${SKILL_VERSION}"

if [[ "${VDW_VERSION}" != "${VSCODE_VERSION}" ]]; then
    log_warn "Version mismatch! CLI=${VDW_VERSION}, VSIX=${VSCODE_VERSION}"
    log_warn "Consider aligning versions before release."
fi

# ─── Package CLI ──────────────────────────────────────────────────────────────
CLI_ARTIFACT=""

case "${OS_NAME}" in
    Darwin)
        log_info "Packaging CLI as DMG for macOS..."
        CLI_ARTIFACT="$("${SCRIPT_DIR}/package_vdw_dmg.sh" "${ARCH}")"
        ;;
    Linux)
        log_info "Packaging CLI as tar.gz for Linux..."
        CLI_ARTIFACT="$("${SCRIPT_DIR}/package_vdw.sh" linux "${ARCH}")"
        ;;
    *)
        log_warn "Unsupported OS '${OS_NAME}' for CLI packaging. Skipping CLI."
        ;;
esac

# ─── Package VS Code Extension ────────────────────────────────────────────────
log_info "Packaging VS Code extension..."
VSIX_ARTIFACT="$("${SCRIPT_DIR}/package_vscode.sh")"

# ─── Package AI Agent Skill ──────────────────────────────────────────────────
log_info "Packaging vdw-cli AI agent Skill..."
SKILL_ARTIFACT="$("${SCRIPT_DIR}/package_skill.sh")"

# ─── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════"
echo "  VDW Packaging Summary"
echo "═══════════════════════════════════════════════════════"
echo "  Platform:    ${OS_NAME} / ${ARCH}"
echo "  CLI version: ${VDW_VERSION}"
echo "  VSIX version: ${VSCODE_VERSION}"
echo "  Skill version: ${SKILL_VERSION}"
echo "───────────────────────────────────────────────────────"
if [[ -n "${CLI_ARTIFACT}" ]]; then
    echo "  CLI:  ${CLI_ARTIFACT}"
fi
echo "  VSIX: ${VSIX_ARTIFACT}"
echo "  Skill: ${SKILL_ARTIFACT}"
echo "═══════════════════════════════════════════════════════"
echo ""
