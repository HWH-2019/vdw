#!/usr/bin/env bash
# =============================================================================
# VDW Installer — One-command install for Virtual Dev Workspace
# Supports: macOS (arm64/x86_64), Linux (arm64/x86_64), Windows (Git Bash/WSL)
# =============================================================================
set -euo pipefail

REPO_NAME="vdw"
BINARY_NAME="vdw"
# ── Customize these for your project ──────────────────────────────────
# Set REPO_URL to enable git-based install
REPO_URL=""
# Set DOWNLOAD_BASE to enable pre-built binary download (requires CI release workflow)
DOWNLOAD_BASE=""
# Example: DOWNLOAD_BASE="https://github.com/your-org/${REPO_NAME}/releases/latest/download"
# ─────────────────────────────────────────────────────────────────────

# ---------------------------------------------------------------------------
# Help
# ---------------------------------------------------------------------------
usage() {
    cat <<EOF
VDW Installer — Virtual Dev Workspace CLI

Usage:
  $(basename "$0") [OPTIONS]

Options:
  --dir <PATH>     Install to custom directory (default: see below)
  --no-verify      Skip post-install verification
  --version        Print installer version
  -h, --help       Show this help

Default install locations:
  macOS/Linux      \$HOME/.local/bin/vdw
  Windows          \$USERPROFILE\\.local\\bin\\vdw.exe

Environment:
  INSTALL_DIR      Override install directory
  VDW_LOCAL        Path to local repo for source build (auto-detected)

Prerequisites:
  - Rust toolchain (rustup/cargo): curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  - Git
EOF
    exit 0
}

# ---------------------------------------------------------------------------
# Color helpers (disabled when not in terminal)
# ---------------------------------------------------------------------------
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    CYAN='\033[0;36m'
    NC='\033[0m' # No Color
else
    RED=''; GREEN=''; YELLOW=''; CYAN=''; NC=''
fi

info()  { echo -e "${CYAN}INFO${NC}  $1"; }
ok()    { echo -e "${GREEN}OK${NC}    $1"; }
warn()  { echo -e "${YELLOW}WARN${NC}  $1"; }
err()   { echo -e "${RED}ERROR${NC} $1"; }
die()   { err "$1"; exit 1; }

# ---------------------------------------------------------------------------
# Platform detection
# ---------------------------------------------------------------------------
detect_os() {
    case "$(uname -s)" in
        Darwin)  echo "macos"  ;;
        Linux)   echo "linux"  ;;
        CYGWIN*|MINGW*|MSYS*) echo "windows" ;;
        *)       die "Unsupported OS: $(uname -s). Only macOS, Linux, and Windows (Git Bash/WSL) are supported." ;;
    esac
}

detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64"  ;;
        aarch64|arm64) echo "aarch64" ;;
        *)             die "Unsupported architecture: $(uname -m)" ;;
    esac
}

detect_platform() {
    echo "$(detect_os)-$(detect_arch)"
}

# ---------------------------------------------------------------------------
# Install directory detection
# ---------------------------------------------------------------------------
default_install_dir() {
    local os
    os=$(detect_os)
    case "$os" in
        macos|linux) echo "${HOME}/.local/bin" ;;
        windows)     echo "${USERPROFILE}\\.local\\bin" ;;
    esac
}

# ---------------------------------------------------------------------------
# Install method 1: Build from source via cargo
# ---------------------------------------------------------------------------
install_from_source() {
    local src_dir="$1"
    local install_root="$2"

    info "Checking prerequisites..."

    if ! command -v cargo &>/dev/null; then
        err "Rust toolchain (cargo) not found."
        echo ""
        echo "  Install it with:"
        echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        echo ""
        echo "  Then restart your terminal and re-run this installer."
        return 1
    fi

    if ! command -v git &>/dev/null; then
        err "Git not found. Please install Git and try again."
        return 1
    fi

    # Determine source directory
    local build_dir
    if [ -n "$src_dir" ] && [ -f "${src_dir}/Cargo.toml" ]; then
        build_dir="$src_dir"
        info "Building from local source: ${build_dir}"
    elif [ -n "$REPO_URL" ]; then
        local tmp_dir
        tmp_dir=$(mktemp -d)
        info "Cloning ${REPO_URL}..."
        git clone --depth 1 "$REPO_URL" "${tmp_dir}/${REPO_NAME}"
        build_dir="${tmp_dir}/${REPO_NAME}"
    else
        # Try to find Cargo.toml relative to this script
        local script_dir
        script_dir="$(cd "$(dirname "$0")" && pwd)"
        if [ -f "${script_dir}/Cargo.toml" ]; then
            build_dir="$script_dir"
            info "Building from local source (script-relative): ${build_dir}"
        else
            err "No local source found and no repository URL configured."
            echo "  Either run this script from within the vdw repo directory,"
            echo "  or set REPO_URL in the script once the repo is published."
            return 1
        fi
    fi

    info "Building ${BINARY_NAME} (release mode)..."
    (cd "$build_dir" && cargo build --release --quiet 2>&1)
    ok "Build complete"

    # Install binary
    mkdir -p "$install_root"
    cp "${build_dir}/target/release/${BINARY_NAME}" "${install_root}/${BINARY_NAME}"
    if [ "$(detect_os)" = "windows" ]; then
        mv "${install_root}/${BINARY_NAME}" "${install_root}/${BINARY_NAME}.exe" 2>/dev/null || true
    fi

    # macOS: clear EDR quarantine attribute (known issue with Alibaba EDR)
    if [ "$(detect_os)" = "macos" ]; then
        xattr -d com.alibaba.security.edr.antivirus.cloudquery "${install_root}/${BINARY_NAME}" 2>/dev/null || true
        # Also clear generic quarantine
        xattr -d com.apple.quarantine "${install_root}/${BINARY_NAME}" 2>/dev/null || true
    fi

    return 0
}

# ---------------------------------------------------------------------------
# Install method 2: Download pre-built binary from GitHub releases
# (ready for when CI is set up — currently a placeholder)
# ---------------------------------------------------------------------------
install_prebuilt() {
    local platform="$1"
    local install_root="$2"

    if [ -z "$DOWNLOAD_BASE" ]; then
        warn "Pre-built binary download is not yet configured."
        echo ""
        echo "  Set DOWNLOAD_BASE in this script once you publish the repo."
        echo "  Example: DOWNLOAD_BASE=\"https://github.com/ORG/${REPO_NAME}/releases/latest/download\""
        echo ""
        echo "  Then set up CI: push a tag to trigger release workflow."
        echo "  See: .github/workflows/release.yml"
        echo ""
        return 1
    fi

    # Map platform to release asset filename
    local asset_file
    case "$platform" in
        macos-aarch64) asset_file="${BINARY_NAME}-aarch64-apple-darwin" ;;
        macos-x86_64)  asset_file="${BINARY_NAME}-x86_64-apple-darwin" ;;
        linux-x86_64)  asset_file="${BINARY_NAME}-x86_64-unknown-linux-gnu" ;;
        linux-aarch64) asset_file="${BINARY_NAME}-aarch64-unknown-linux-gnu" ;;
        windows-x86_64) asset_file="${BINARY_NAME}-x86_64-pc-windows-msvc.exe" ;;
        *) die "No pre-built binary for ${platform}" ;;
    esac

    local url="${DOWNLOAD_BASE}/${asset_file}"
    local bin_path="${install_root}/${BINARY_NAME}"
    [ "$platform" = "windows-*" ] && bin_path="${install_root}/${BINARY_NAME}.exe"

    info "Downloading ${BINARY_NAME} for ${platform}..."
    mkdir -p "$install_root"

    if command -v curl &>/dev/null; then
        curl -sfL "$url" -o "$bin_path"
    elif command -v wget &>/dev/null; then
        wget -q "$url" -O "$bin_path"
    else
        die "Neither curl nor wget found. Install one of them to download binaries."
    fi

    chmod +x "$bin_path"

    # macOS: clear EDR quarantine
    if [ "$(detect_os)" = "macos" ]; then
        xattr -d com.alibaba.security.edr.antivirus.cloudquery "$bin_path" 2>/dev/null || true
        xattr -d com.apple.quarantine "$bin_path" 2>/dev/null || true
    fi

    ok "Downloaded ${BINARY_NAME}"
    return 0
}

# ---------------------------------------------------------------------------
# Post-install: verify, PATH check
# ---------------------------------------------------------------------------
verify_install() {
    local install_root="$1"
    local bin_path="${install_root}/${BINARY_NAME}"
    [ "$(detect_os)" = "windows" ] && bin_path="${install_root}/${BINARY_NAME}.exe"

    if [ ! -f "$bin_path" ]; then
        die "Binary not found at ${bin_path}"
    fi

    if [ ! -x "$bin_path" ]; then
        chmod +x "$bin_path"
    fi

    ok "Installed: ${bin_path}"
    echo ""
    if command -v "${BINARY_NAME}" &>/dev/null; then
        "${BINARY_NAME}" --version
        ok "${BINARY_NAME} is accessible from your PATH"
    else
        warn "${install_root} is NOT in your PATH."
        echo ""
        echo "  Add it to your shell config:"
        echo ""
        echo "    # ~/.zshrc or ~/.bashrc"
        echo "    export PATH=\"\$PATH:${install_root}\""
        echo ""
        echo "  Then reload: source ~/.zshrc"
        echo "  Or run directly: ${bin_path} --help"
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
main() {
    local custom_dir=""
    local skip_verify=false

    # Parse args
    while [ $# -gt 0 ]; do
        case "$1" in
            --dir) shift; custom_dir="$1" ;;
            --no-verify) skip_verify=true ;;
            --version) echo "${BINARY_NAME} installer v0.1.0"; exit 0 ;;
            -h|--help) usage ;;
            *) die "Unknown option: $1. Use --help for usage." ;;
        esac
        shift
    done

    echo ""
    echo "  ╔══════════════════════════════════════════╗"
    echo "  ║     VDW — Virtual Dev Workspace          ║"
    echo "  ║     One-command Installer                ║"
    echo "  ╚══════════════════════════════════════════╝"
    echo ""

    # Detect platform
    local platform
    platform=$(detect_platform)
    info "Detected: ${platform}"

    # Determine install directory
    local install_root
    if [ -n "$custom_dir" ]; then
        install_root="$custom_dir"
    elif [ -n "${INSTALL_DIR:-}" ]; then
        install_root="$INSTALL_DIR"
    else
        install_root=$(default_install_dir)
    fi
    info "Install target: ${install_root}"
    echo ""

    # Auto-detect local source
    local src_dir="${VDW_LOCAL:-}"
    if [ -z "$src_dir" ]; then
        local script_dir
        script_dir="$(cd "$(dirname "$0")" && pwd)"
        if [ -f "${script_dir}/Cargo.toml" ]; then
            src_dir="$script_dir"
        fi
    fi

    # Try source build first; fall back to pre-built
    if ! install_from_source "$src_dir" "$install_root"; then
        install_prebuilt "$platform" "$install_root" || {
            echo ""
            err "Installation failed."
            echo "  Options:"
            echo "    1. Install Rust: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
            echo "    2. Then run: cargo install --path ."
            exit 1
        }
    fi

    echo ""
    if [ "$skip_verify" != true ]; then
        verify_install "$install_root"
    fi

    echo ""
    ok "Installation complete!"
    echo ""
    echo "  Quick start:"
    echo "    vdw init           Initialize a workspace"
    echo "    vdw repo add <url> Add a repository"
    echo "    vdw link           Create dependency links"
    echo "    vdw doctor         Check workspace health"
    echo ""
}

main "$@"
