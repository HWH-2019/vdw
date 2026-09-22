#!/usr/bin/env bash
set -euo pipefail

# Packages the vdw-cli AI agent Skill with its top-level directory preserved.

_HELP_TEXT=""
_HELP_DESC="Package the vdw-cli AI agent Skill as a versioned tar.gz artifact.

The artifact contains a top-level vdw-cli/ directory and is written to dist/."

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

SKILL_DIR="${PROJECT_ROOT}/vdw-cli"
VERSION="$(get_skill_version)"

for required_file in SKILL.md reference.md scripts/open-workspace.sh scripts/install-bundled-cli.sh; do
    if [[ ! -f "${SKILL_DIR}/${required_file}" ]]; then
        log_error "Missing required Skill file: vdw-cli/${required_file}"
        exit 1
    fi
done

ensure_dist_dir

ARTIFACT_NAME="vdw_cli_skill_${VERSION}.tar.gz"
ARTIFACT_PATH="${PROJECT_ROOT}/dist/${ARTIFACT_NAME}"

log_info "Packaging vdw-cli Skill ${VERSION}..."
tar -czf "${ARTIFACT_PATH}" -C "${PROJECT_ROOT}" vdw-cli

log_info "Skill package created: ${ARTIFACT_PATH}"
echo "${ARTIFACT_PATH}"
