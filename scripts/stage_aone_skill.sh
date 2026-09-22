#!/usr/bin/env bash
set -euo pipefail

# Stages the vdw-cli Skill and the macOS Apple Silicon CLI for Aone publishing.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=common.sh
source "${SCRIPT_DIR}/common.sh"

VERSION="$(get_skill_version)"
SKILL_DIR="${PROJECT_ROOT}/vdw-cli"
CLI_ARCHIVE="${PROJECT_ROOT}/dist/vdw_darwin_arm64_${VERSION}.tar.gz"

if [[ ! -f "${CLI_ARCHIVE}" ]]; then
    log_error "Missing CLI artifact: ${CLI_ARCHIVE}"
    log_error "Run: make package-vdw"
    exit 1
fi

if ! grep -q "^[[:space:]]*version: \"${VERSION}\"$" "${SKILL_DIR}/SKILL.md"; then
    log_error "vdw-cli/SKILL.md metadata version does not match CLI ${VERSION}"
    exit 1
fi

STAGE_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vdw-aone-${VERSION}.XXXXXX")"
cp -R "${SKILL_DIR}/." "${STAGE_DIR}/"
mkdir -p "${STAGE_DIR}/assets"
cp "${CLI_ARCHIVE}" "${STAGE_DIR}/assets/"

# Aone's publisher requires a top-level version, while the standard Codex Skill
# validator accepts custom version information only under metadata.
awk -v version="${VERSION}" '
    NR == 1 { print; next }
    !inserted && /^name:/ {
        print
        print "version: " version
        inserted = 1
        next
    }
    { print }
' "${STAGE_DIR}/SKILL.md" > "${STAGE_DIR}/SKILL.md.aone"
mv "${STAGE_DIR}/SKILL.md.aone" "${STAGE_DIR}/SKILL.md"

log_info "Aone Skill staged at: ${STAGE_DIR}"
echo "${STAGE_DIR}"
