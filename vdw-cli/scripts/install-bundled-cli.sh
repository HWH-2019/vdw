#!/usr/bin/env bash
set -euo pipefail

VERSION="0.9.1"
EXPECTED_SHA256="1b40afbc79d5b350a926ef8516377afb0668b480572e7229bafd53d290f9264b"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SKILL_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ARCHIVE="${SKILL_DIR}/assets/vdw_darwin_arm64_${VERSION}.tar.gz"
INSTALL_DIR="${VDW_INSTALL_DIR:-${HOME}/.local/bin}"
INSTALL_PATH="${INSTALL_DIR}/vdw"

if [[ "$(uname -s)" != "Darwin" ]] || [[ "$(uname -m)" != "arm64" ]]; then
    echo "Bundled VDW CLI ${VERSION} supports macOS Apple Silicon only." >&2
    exit 2
fi

if [[ ! -f "${ARCHIVE}" ]]; then
    echo "Bundled CLI archive not found: ${ARCHIVE}" >&2
    echo "Install a matching VDW CLI separately, then run: vdw --version" >&2
    exit 3
fi

ACTUAL_SHA256="$(shasum -a 256 "${ARCHIVE}" | awk '{print $1}')"
if [[ "${ACTUAL_SHA256}" != "${EXPECTED_SHA256}" ]]; then
    echo "Bundled CLI checksum mismatch." >&2
    exit 4
fi

TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/vdw-install.XXXXXX")"
trap 'rm -rf "${TEMP_DIR}"' EXIT

tar -xzf "${ARCHIVE}" -C "${TEMP_DIR}"
if [[ ! -f "${TEMP_DIR}/vdw" ]]; then
    echo "Bundled CLI archive does not contain the vdw executable." >&2
    exit 5
fi

ACTUAL_VERSION="$("${TEMP_DIR}/vdw" --version)"
if [[ "${ACTUAL_VERSION}" != "vdw ${VERSION}" ]]; then
    echo "Bundled CLI version mismatch: expected 'vdw ${VERSION}', got '${ACTUAL_VERSION}'." >&2
    exit 6
fi

if [[ -e "${INSTALL_PATH}" ]]; then
    EXISTING_VERSION="$("${INSTALL_PATH}" --version 2>/dev/null || true)"
    if [[ "${EXISTING_VERSION}" == "vdw ${VERSION}" ]]; then
        echo "${EXISTING_VERSION} is already installed at ${INSTALL_PATH}"
        exit 0
    fi
    if [[ "${VDW_INSTALL_FORCE:-0}" != "1" ]]; then
        echo "Refusing to replace ${INSTALL_PATH} (${EXISTING_VERSION:-unknown version})." >&2
        echo "After explicit confirmation, rerun with VDW_INSTALL_FORCE=1." >&2
        exit 7
    fi
fi

mkdir -p "${INSTALL_DIR}"
install -m 0755 "${TEMP_DIR}/vdw" "${INSTALL_PATH}"

INSTALLED_VERSION="$("${INSTALL_PATH}" --version)"
if [[ "${INSTALLED_VERSION}" != "vdw ${VERSION}" ]]; then
    echo "Installed CLI verification failed: ${INSTALLED_VERSION}" >&2
    exit 8
fi

echo "Installed ${INSTALLED_VERSION} at ${INSTALL_PATH}"
