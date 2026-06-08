# Installation & Setup Guide

<cite>
**Referenced Files in This Document**
- [README.md](file://vdw/README.md)
- [install.sh](file://vdw/install.sh)
- [Makefile](file://vdw/Makefile)
- [Cargo.toml](file://vdw/Cargo.toml)
- [main.rs](file://vdw/src/main.rs)
- [init.rs](file://vdw/src/commands/init.rs)
- [config.rs](file://vdw/src/types/config.rs)
- [.github/workflows/release.yml](file://.github/workflows/release.yml)
- [package.json](file://vdw-vscode/package.json)
- [README.md](file://vdw-vscode/README.md)
</cite>

## Update Summary
**Changes Made**
- Enhanced installation methods section with comprehensive CLI installation steps from the new README.md
- Added detailed VS Code extension installation and setup instructions
- Expanded initial workspace configuration with complete initialization and repository addition workflows
- Updated troubleshooting section with new common issues and solutions
- Improved platform-specific setup with enhanced macOS, Linux, and Windows installation guidance
- Added comprehensive usage examples for all major VDW commands

## Table of Contents
1. [Introduction](#introduction)
2. [Prerequisites](#prerequisites)
3. [Installation Methods](#installation-methods)
4. [Platform-Specific Setup](#platform-specific-setup)
5. [Post-Installation Verification](#post-installation-verification)
6. [VS Code Extension Setup](#vs-code-extension-setup)
7. [Initial Workspace Configuration](#initial-workspace-configuration)
8. [Troubleshooting Common Issues](#troubleshooting-common-issues)
9. [Development Environment Setup](#development-environment-setup)
10. [Complete Setup Workflow](#complete-setup-workflow)

## Introduction

VDW (Virtual Dev Workspace) is a cross-repository development middleware that allows developers to seamlessly work with multiple Git repositories as a single workspace. This guide provides comprehensive installation and setup instructions for the VDW CLI and its VS Code extension.

VDW offers intelligent dependency management, smart linking capabilities, and AI context filtering to enhance developer productivity in multi-repository environments. The system supports hardlinks, symlinks, and junctions depending on your operating system and filesystem capabilities.

## Prerequisites

Before installing VDW, ensure your system meets the following requirements:

### System Requirements
- **Operating Systems**: macOS, Linux, or Windows (Git Bash/WSL)
- **Architecture Support**: x86_64, arm64/aarch64
- **Rust Toolchain**: Required for building from source
- **Git**: Essential for repository operations
- **Shell Access**: Bash-compatible shell for installation scripts

### Supported Platforms
| Operating System | Architectures | Link Strategy |
|------------------|---------------|---------------|
| macOS | arm64, x86_64 | HardLink → Symlink |
| Linux | x86_64, aarch64 | HardLink → Symlink |
| Windows | x86_64 (Git Bash/WSL) | Junction → Symlink |

**Section sources**
- [README.md:333-340](file://vdw/README.md#L333-L340)
- [install.sh:71-86](file://vdw/install.sh#L71-L86)

## Installation Methods

VDW provides multiple installation approaches to accommodate different user needs and environments.

### Method 1: One-Command Installation (Recommended)

The simplest approach uses the built-in installer script:

```bash
cd vdw && make install
```

Or directly using the installer:

```bash
./install.sh
```

This method automatically:
- Detects your platform and architecture
- Checks for required prerequisites
- Builds from source if Rust is available
- Handles cross-platform compatibility
- Updates VS Code plugin binary

### Method 2: Quick Installation for Developers

If you already have Rust installed:

```bash
make install-quick
```

This method:
- Builds the release binary directly
- Copies to `~/.local/bin`
- Clears macOS EDR quarantine attributes
- Updates local VS Code plugin copy

### Method 3: Manual Cargo Installation

For advanced users or CI environments:

```bash
cargo install --path .
```

### Method 4: Pre-built Binary Installation

Once the project is published, you can download pre-built binaries:

```bash
# Example for Linux x86_64
curl -sfL https://github.com/YOUR_ORG/vdw/releases/latest/download/vdw-x86_64-unknown-linux-gnu -o /usr/local/bin/vdw
chmod +x /usr/local/bin/vdw
```

**Section sources**
- [README.md:56-70](file://vdw/README.md#L56-L70)
- [install.sh:107-179](file://vdw/install.sh#L107-L179)
- [Makefile:11-31](file://vdw/Makefile#L11-L31)

## Platform-Specific Setup

### macOS Installation

The installer automatically handles macOS-specific considerations:

1. **EDR Quarantine Removal**: Automatic clearing of Alibaba EDR and Apple quarantine attributes
2. **Architecture Detection**: Supports both Intel and Apple Silicon (x86_64, arm64)
3. **Default Installation Path**: `~/.local/bin/vdw`

Common macOS issues and solutions:

```bash
# If encountering SIGKILL (exit code 137) with release binary
xattr -d com.alibaba.security.edr.antivirus.cloudquery /path/to/vdw
```

### Linux Installation

Linux installation follows standard procedures:

1. **Architecture Support**: x86_64 and aarch64
2. **Hardlink Preference**: Uses hardlinks when filesystem supports them
3. **Installation Directory**: `~/.local/bin/` by default

### Windows (Git Bash/WSL) Installation

Windows support through Git Bash and WSL:

1. **Junction Support**: Uses Windows junctions for directory linking
2. **Executable Naming**: Installs as `vdw.exe`
3. **Path Configuration**: Adds to user profile PATH

**Section sources**
- [install.sh:165-170](file://vdw/install.sh#L165-L170)
- [install.sh:201-210](file://vdw/install.sh#L201-L210)
- [README.md:333-340](file://vdw/README.md#L333-L340)

## Post-Installation Verification

After installation, verify your setup with these commands:

### Basic Verification

```bash
# Check version
vdw --version

# Display help
vdw --help

# Verify PATH accessibility
which vdw
```

### Installation Script Verification

The installer performs automatic verification:

```bash
# The script checks:
# 1. Binary existence and executability
# 2. PATH configuration
# 3. Version reporting
# 4. Shell configuration suggestions
```

### Manual PATH Configuration

If the installer reports PATH issues:

```bash
# Add to ~/.bashrc or ~/.zshrc
export PATH="$PATH:$HOME/.local/bin"

# Reload shell configuration
source ~/.bashrc  # or source ~/.zshrc
```

**Section sources**
- [install.sh:242-271](file://vdw/install.sh#L242-L271)
- [README.md:71-76](file://vdw/README.md#L71-L76)

## VS Code Extension Setup

The VDW VS Code extension provides a graphical interface for workspace management.

### Extension Installation

1. **Prerequisites**: Ensure VDW CLI is installed first
2. **Extension Activation**: Automatically activates when `.vdwrc` is detected
3. **Binary Detection**: Extension looks for CLI in this order:
   - `vdw.cliPath` setting
   - `VDW_DEBUG=true` (development mode)
   - `./vdw/vdw` (local workspace binary)
   - System PATH

### Extension Features

| Feature | Description |
|---------|-------------|
| Dependency Tree View | Visual representation of linked/unlinked repositories |
| Add Repo Wizard | Interactive dialog for adding repositories |
| One-Click Link/Unlink | Direct operations from tree view |
| Status Bar | Real-time dependency and file statistics |
| AI Context Filtering | Scope-based file exposure for LLMs |

### Extension Configuration

Key settings available in VS Code:

```json
{
  "vdw.cliPath": "vdw",
  "vdw.autoOpenGitRepositories": true,
  "vdw.autoOpenDelay": 500
}
```

**Section sources**
- [README.md:320-331](file://vdw/README.md#L320-L331)
- [README.md:108-118](file://vdw-vscode/README.md#L108-L116)
- [package.json:150-169](file://vdw-vscode/package.json#L150-L169)

## Initial Workspace Configuration

### Creating Your First Workspace

1. **Initialize Workspace**:
```bash
cd my-project
vdw init
```

2. **Create .vdwrc Template**:
The init command generates a basic `.vdwrc` file with workspace metadata and empty dependencies array.

3. **Add Your First Repository**:
```bash
# HTTPS repository
vdw repo add https://github.com/org/ui-lib.git --path libs/ui-lib --branch main

# SSH repository
vdw repo add git@github.com:org/api-service.git --path services/api --branch develop

# Editable mode (worktree)
vdw repo add git@github.com:org/shared.git --path packages/shared --branch master --editable
```

### Understanding .vdwrc Structure

The configuration file uses YAML format:

```yaml
workspace:
  name: my-workspace
  version: "1.0"

dependencies:
  - remote: "https://github.com/org/ui-lib.git"
    path: libs/ui-lib
    branch: main
    scope:
      include:
        - "src/components/**"
        - "src/hooks/**"
      exclude:
        - "**/*.test.ts"
        - "**/node_modules/**"
    editable: false
```

### Dependency Configuration Fields

| Field | Required | Description |
|-------|----------|-------------|
| `remote` | ✅ | Git URL (https://, git@, or local path) |
| `path` | ✅ | Where to link in your workspace |
| `branch`/`tag`/`commit` | ✅ (one required) | Which version to track |
| `scope.include` | ✅ | Glob patterns for files to expose |
| `scope.exclude` | ❌ | Glob patterns to filter out |
| `editable` | ❌ | Enable git worktree (default: false) |

**Section sources**
- [README.md:96-120](file://vdw/README.md#L96-L120)
- [init.rs:5-29](file://vdw/src/commands/init.rs#L5-L29)
- [config.rs:19-47](file://vdw/src/types/config.rs#L19-L47)

## Troubleshooting Common Issues

### Installation Problems

**Issue**: Rust toolchain not found
```bash
# Solution: Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**Issue**: Git not found
```bash
# Solution: Install Git for your platform
# macOS: brew install git
# Ubuntu: sudo apt install git
# Windows: choco install git
```

**Issue**: PATH not configured
```bash
# Solution: Add to shell profile
echo 'export PATH="$PATH:$HOME/.local/bin"' >> ~/.bashrc
source ~/.bashrc
```

### macOS-Specific Issues

**Issue**: SIGKILL (exit code 137) with release binary
```bash
# Solution: Clear EDR quarantine
xattr -d com.alibaba.security.edr.antivirus.cloudquery /path/to/vdw
```

**Issue**: Gatekeeper warnings
```bash
# Solution: Clear quarantine attribute
xattr -d com.apple.quarantine /path/to/vdw
```

### VS Code Extension Issues

**Issue**: Extension not detecting CLI
```bash
# Solution: Configure vdw.cliPath setting
{
  "vdw.cliPath": "/absolute/path/to/vdw"
}
```

**Issue**: Extension not activating
```bash
# Solution: Ensure .vdwrc exists in workspace root
# The extension activates on workspaceContains:.vdwrc
```

### Linking Problems

**Issue**: Permission denied when linking
```bash
# Solution: Check filesystem permissions
# Ensure write access to workspace directory
```

**Issue**: Cross-filesystem linking limitations
```bash
# Solution: Understand platform limitations
# Windows: Uses junctions for directories, symlinks for files
# macOS/Linux: Prefers hardlinks, falls back to symlinks
```

**Section sources**
- [install.sh:113-126](file://vdw/install.sh#L113-L126)
- [install.sh:261-270](file://vdw/install.sh#L261-L270)
- [README.md:333-340](file://vdw/README.md#L333-L340)

## Development Environment Setup

### Building from Source

For development or contribution:

```bash
# Clone the repository
git clone https://github.com/vdw/vdw.git
cd vdw

# Build release version
cargo build --release

# Run tests
cargo test
cargo test -- --ignored  # Include integration tests
```

### Development Dependencies

The project uses modern Rust tooling:

```toml
[dependencies]
clap = { version = "4.4", features = ["derive", "env"] }
dialoguer = "0.11"
serde = { version = "1.0", features = ["derive"] }
serde_yaml = "0.9"
git2 = { version = "0.18", features = ["vendored-openssl"] }
tokio = { version = "1.35", features = ["full"] }
```

### Testing Setup

```bash
# Unit tests
cargo test

# Integration tests
cargo test -- --ignored

# Benchmark tests
cargo bench
```

### CI/CD Pipeline

The repository includes GitHub Actions for automated releases:

```yaml
name: Release
on:
  push:
    tags:
      - 'v*'
```

The pipeline builds binaries for multiple platforms and creates GitHub releases with checksums.

**Section sources**
- [README.md:349-363](file://vdw/README.md#L349-L363)
- [Cargo.toml:12-31](file://vdw/Cargo.toml#L12-L31)
- [.github/workflows/release.yml:1-92](file://.github/workflows/release.yml#L1-L92)

## Complete Setup Workflow

Follow this step-by-step workflow to complete your VDW installation:

### Step 1: System Preparation
1. Install Rust toolchain
2. Install Git
3. Verify system requirements

### Step 2: Install VDW CLI
```bash
cd vdw
make install  # Recommended
# or
./install.sh  # Alternative
```

### Step 3: Verify Installation
```bash
vdw --version
vdw --help
```

### Step 4: Configure VS Code Extension
1. Install VDW VS Code extension
2. Ensure CLI is accessible to extension
3. Verify extension activation

### Step 5: Create Your First Workspace
```bash
mkdir my-project && cd my-project
vdw init
# Edit .vdwrc to add dependencies
vdw link
```

### Step 6: Test Your Setup
```bash
vdw doctor    # Diagnose workspace
vdw validate  # Validate configuration
vdw context preview  # Check AI context
```

### Step 7: Continuous Integration
For automated deployments, use the provided CI/CD pipeline or adapt it for your needs.

This comprehensive installation and setup guide ensures you have a fully functional VDW environment ready for cross-repository development workflows.