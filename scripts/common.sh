#!/usr/bin/env bash
set -euo pipefail

# VDW Packaging - Common Utilities
# Shared functions for all packaging scripts.

# ─── Colors ────────────────────────────────────────────────────────────────────
if [[ -t 1 ]]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    NC='\033[0m' # No Color
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

# ─── Logging ───────────────────────────────────────────────────────────────────
log_info()  { echo -e "${GREEN}[INFO]${NC}  $*"; }
log_warn()  { echo -e "${YELLOW}[WARN]${NC}  $*" >&2; }
log_error() { echo -e "${RED}[ERROR]${NC} $*" >&2; }

# ─── Project Root ─────────────────────────────────────────────────────────────
# Auto-detect: parent directory of scripts/
PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PROJECT_ROOT

# ─── Version Extraction ───────────────────────────────────────────────────────
get_vdw_version() {
    grep '^version' "${PROJECT_ROOT}/vdw/Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/'
}

get_vscode_version() {
    sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\(.*\)".*/\1/p' "${PROJECT_ROOT}/vdw-vscode/package.json" | head -1
}

get_skill_version() {
    # The Skill is released in lockstep with the CLI and uses the CLI version.
    get_vdw_version
}

# ─── Artifact Naming ──────────────────────────────────────────────────────────
# Usage: get_artifact_name <name> <os> <arch> <version> <ext>
# Example: get_artifact_name vdw darwin arm64 0.5.0 tar.gz
#   => vdw_darwin_arm64_0.5.0.tar.gz
get_artifact_name() {
    local name="${1:-}" os="${2:-}" arch="${3:-}" version="${4:-}" ext="${5:-}"
    if [[ -z "${name}" ]] || [[ -z "${os}" ]] || [[ -z "${arch}" ]] || [[ -z "${version}" ]] || [[ -z "${ext}" ]]; then
        log_error "get_artifact_name requires 5 arguments: name os arch version ext"
        return 1
    fi
    echo "${name}_${os}_${arch}_${version}.${ext}"
}

# ─── Dist Directory ───────────────────────────────────────────────────────────
ensure_dist_dir() {
    mkdir -p "${PROJECT_ROOT}/dist"
}

# ─── Help Helper ──────────────────────────────────────────────────────────────
show_help() {
    local script_name
    script_name="$(basename "$0")"
    if [[ -n "${_HELP_TEXT:-}" ]]; then
        echo "Usage: ${script_name} ${_HELP_TEXT}"
    else
        echo "Usage: ${script_name}"
    fi
    if [[ -n "${_HELP_DESC:-}" ]]; then
        echo ""
        echo "${_HELP_DESC}"
    fi
}

# Check for --help / -h early
if [[ "${1:-}" == "--help" ]] || [[ "${1:-}" == "-h" ]]; then
    show_help
    exit 0
fi
