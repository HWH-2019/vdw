# 🏗️ VDW — Virtual Dev Workspace

> **Cross-repo development middleware for the AI era**  
> Seamlessly stitch multiple repositories into one development workspace, then serve the context to your AI coding assistant.

[![Rust](https://img.shields.io/badge/Rust-🦀-orange?style=flat-square)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=flat-square)](LICENSE)

---

## 🤔 What is VDW?

Modern development doesn't live in a single repo. You have:

- A **frontend** app → `git@github.com:org/web-app.git`
- A **shared UI library** → `git@github.com:org/ui-kit.git`
- A **backend service** → `git@github.com:org/api-service.git`
- A **protobuf schema** repo → `git@github.com:org/protos.git`

You need them all in one place during development. But cloning them manually, keeping them in sync, and managing file paths is a **nightmare**. 😩

**VDW solves this.** It gives you:

1. 🗄️ A **content-addressable store** on your machine that deduplicates repos
2. 🔗 **Smart linking** — expose only the files you want via hardlinks / symlinks
3. 🤖 **AI context filtering** — generate a clean file list for LLM-based coding assistants
4. 📝 **Editable dependencies** — repos added with `--editable` use git worktrees so you can modify code with full git support

Think of it as **`npm link` but for any language**, combined with **AI-aware file scoping**.

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| 📦 **Content Store** | Bare git repos in `~/.vdw/store/objects/` — automatic dedup via SHA256 |
| 🔗 **Smart Linking** | Hardlinks on same filesystem, symlinks cross-filesystem (or junctions on Windows) |
| 🎯 **Scope Filtering** | Glob patterns to include/exclude files per dependency |
| ✏️ **Editable Mode** | Git worktrees for repos you need to modify |
| 🔍 **AI Context** | Generate filtered file lists for LLM-powered coding tools |
| 🌐 **SSH Auth** | Delegates to system OpenSSH — no key management needed |
| 🛡️ **Idempotent** | Run `vdw link` 100 times — same result every time |
| 🖥️ **VS Code Plugin** | GUI for adding/linking repos (see [vdw-vscode](../vdw-vscode)) |

---

## Install the CLI

### Requirements

- **Git** is required at runtime for repository fetch and worktree operations.
- Release archives do not require Rust.
- Source installation requires the **[Rust toolchain](https://rustup.rs)** (`cargo` and `rustc`).
- Supported environments are macOS, Linux, and Windows x86_64. On Windows, use PowerShell, Git Bash, or WSL as appropriate for your Git installation.

### Option A — Install a pre-built release

Download the matching archive from [GitHub Releases](https://github.com/HWH-2019/vdw/releases):

| Platform | Archive name | Binary inside |
|----------|--------------|---------------|
| macOS Apple Silicon | `vdw_darwin_arm64_<version>.tar.gz` | `vdw_darwin_arm64_<version>` |
| macOS Intel | `vdw_darwin_x86_64_<version>.tar.gz` | `vdw_darwin_x86_64_<version>` |
| Linux x86_64 | `vdw_linux_x86_64_<version>.tar.gz` | `vdw_linux_x86_64_<version>` |
| Linux ARM64 | `vdw_linux_arm64_<version>.tar.gz` | `vdw_linux_arm64_<version>` |
| Windows x86_64 | `vdw_win32_x86_64_<version>.tar.gz` | `vdw_win32_x86_64_<version>.exe` |

macOS/Linux example:

```bash
VERSION=0.6.2            # replace with the release you downloaded
OS=darwin                # darwin or linux
ARCH=arm64               # arm64 or x86_64
ASSET="vdw_${OS}_${ARCH}_${VERSION}"
tar -xzf "${ASSET}.tar.gz"
mkdir -p "$HOME/.local/bin"
mv "$ASSET" "$HOME/.local/bin/vdw"
chmod +x "$HOME/.local/bin/vdw"
```

Make sure `~/.local/bin` is on `PATH`:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc  # use ~/.bashrc for Bash
source ~/.zshrc
vdw --version
```

On Windows, extract the archive, rename the executable to `vdw.exe`, move it to a directory on `PATH`, and run `vdw --version`.

### Option B — Install from source

From the repository root:

```bash
cd vdw
make install
```

Or clone and install in one flow:

```bash
git clone https://github.com/HWH-2019/vdw.git vdw-src
cd vdw-src/vdw
./install.sh
```

`install.sh` is the recommended source installer on macOS, Linux, and WSL. For native Windows builds, use `cargo install --path . --locked` from PowerShell or install the pre-built Windows archive.

The installer:

- checks Git and Rust;
- builds `target/release/vdw`;
- installs to `~/.local/bin/vdw` by default;
- clears known macOS EDR/quarantine attributes; and
- verifies the installed binary.

Useful alternatives:

```bash
make install-quick                 # build and copy, with fewer checks
./install.sh --dir /opt/vdw/bin    # custom install directory
cargo install --path . --locked    # install to Cargo's bin directory
```

`install.sh` currently builds from local source; its pre-built download fallback is intentionally not configured. Use the GitHub release archives above when Rust is unavailable.

### Verify

```bash
which vdw        # Windows: where.exe vdw
vdw --version
vdw --help
```

If the command is missing but `~/.local/bin/vdw` exists, fix `PATH` or configure the VS Code extension with the absolute binary path.

### Upgrade or uninstall

To upgrade, install the newer archive over the existing binary or pull the source repository and run `make install` again.

```bash
make uninstall                       # when using the default source install
rm "$HOME/.local/bin/vdw"            # equivalent default-path removal
```

Removing the binary does not delete `~/.vdw/store` or any VDW workspace.

---

## 🚀 Quick Start

### 🏁 Start a workspace

```bash
cd my-project

# Initialize with a .vdwrc config file
vdw init

# Or with a fullstack template (pre-filled examples)
vdw init --template fullstack
```

### 📥 Add a repository

```bash
# Remote repo via SSH
vdw repo add git@github.com:org/api-service.git --path services/api --branch develop

# Remote repo via HTTPS
vdw repo add https://github.com/org/ui-lib.git --path libs/ui-lib --branch main

# Local repo (editable — creates a git worktree)
vdw repo add /path/to/local/repo --path packages/shared --branch main --editable

# Remote repo (editable — creates a git worktree so you can make changes)
vdw repo add git@github.com:org/shared.git --path packages/shared --branch master --editable
```

This will:
1. Clone the repo as a bare git repo into `~/.vdw/store/objects/`
2. Create a `metadata.json` with commit hash, size, branch info
3. Add an entry to your `.vdwrc` configuration

### 🔗 Link files into your workspace

```bash
# Link all dependencies
vdw link

# Link a specific repo only
vdw link --repo git@github.com:org/ui-lib.git
```

VDW walks each dependency's source directory, applies scope filters, and creates hardlinks (or symlinks) at the configured path. Git metadata (`.git` directories and worktree pointer files) is never projected. IDE integrations register the complete managed worktree instead, so scope filtering cannot appear as deleted Git files. Run it anytime to refresh.

### ❌ Unlink

```bash
# Unlink all
vdw unlink

# Unlink a specific repo
vdw unlink --repo git@github.com:org/ui-lib.git
```

If an editor has replaced a hardlink and the Store/workspace contents differ, VDW stops before deleting anything. Inspect and resolve the files first:

```bash
# List only real content/metadata conflicts
vdw conflicts list

# Accept one side for every conflict (both sides are backed up first)
vdw conflicts resolve --accept store
vdw conflicts resolve --accept workspace

# Resolve selected files only
vdw conflicts resolve --accept workspace --path packages/ui-lib/src/button.ts
```

Backups are retained under `.vdw/conflict-backups/`. Detached files with identical content are rebuilt automatically and do not require a decision.

### 🗑️ Remove a repo

```bash
# By URL
vdw repo remove https://github.com/org/ui-lib.git

# By name (fuzzy search)
vdw repo remove ui-lib
```

### 📋 List repositories

```bash
vdw repo list
```

```
REPO                                     BRANCH       EDIT     PATH
--------------------------------------------------------------------------------
git@gitlab.alibaba-inc.com:eleme-do…     main         yes      ./packages/alsc.sales_aoi_service
ui-lib-a1b2c3d4                          develop      no       ./libs/ui-lib

Total: 2 repository(ies)
```

### 🩺 Doctor (diagnose issues)

```bash
vdw doctor
```

### ✅ Validate config

```bash
vdw validate
```

### 🧠 AI context

```bash
# Preview files that will be included in AI context
vdw context preview

# Show statistics (file count, total size by dependency)
vdw context stats
```

### 🌿 Worktrees (for editable repos)

```bash
# Create a worktree for a specific branch
vdw worktree create ui-lib feature/new-button

# List worktrees
vdw worktree list

# Remove a worktree
vdw worktree delete ui-lib feature/new-button --force
```

---

## 📋 Configuration

A `.vdwrc` file (YAML) defines your workspace:

```yaml
workspace:
  name: my-monorepo
  version: "1.0"

dependencies:
  # Remote repository (Git URL)
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

  # Remote repository (SSH)
  - remote: "git@github.com:org/protos.git"
    path: packages/protos
    tag: v2.0.0
    scope:
      include:
        - "**/*.proto"

  # Local repository (path on disk)
  - remote: ''
    local: /path/to/local/api-service
    path: services/api
    branch: develop
    editable: true
    scope:
      include:
        - "src/**/*.rs"
```

### Dependency fields

| Field | Required | Description |
|-------|----------|-------------|
| `remote` | ✅ | Git URL (`https://`, `git@`, or empty for local repos) |
| `local` | ✅ (if no remote) | Local repository path on disk |
| `origin` | ❌ | Original source path (preserved when `local` is overwritten with worktree path) |
| `path` | ✅ | Where to link in your workspace |
| `branch` / `tag` / `commit` | ✅ (pick one) | Which version to track |
| `scope.include` | ✅ | Glob patterns for files to expose |
| `scope.exclude` | ❌ | Glob patterns to filter out |
| `editable` | ❌ | Enable git worktree (default: `false`) |

---

## ⚙️ How It Works

```
                    ┌─────────────────────────────────────┐
                    │         Your Workspace               │
                    │                                     │
                    │   packages/ui-lib/ (hardlinks)      │
                    │   services/api/    (worktree)        │
                    │   packages/protos/ (symlinks)        │
                    │                                     │
                    └───────────┬─────────────────────────┘
                                │ link / unlink
                    ┌───────────▼─────────────────────────┐
                    │     VDW Content Store                │
                    │     ~/.vdw/store/objects/             │
                    │                                     │
                    │   ui-lib-a1b2c3/   (bare.git)        │
                    │   api-svc-d4e5f6/  (bare.git)        │
                    │   protos-7890ab/   (bare.git)        │
                    └───────────┬─────────────────────────┘
                                │ clone / fetch
                    ┌───────────▼─────────────────────────┐
                    │      Remote Git Repositories         │
                    │                                     │
                    │   github.com/org/ui-lib.git          │
                    │   github.com/org/api-service.git     │
                    │   github.com/org/protos.git          │
                    └─────────────────────────────────────┘
```

**The flow:**

1. **`vdw repo add`** → clones a bare git repo into the content store, records commit hash + metadata
2. **`vdw link`** → walks each repo's files, matches scope filters, creates hardlinks/symlinks in your workspace
3. **`vdw unlink`** → removes all links (but keeps the store intact)
4. **Editable mode** → creates a git worktree from the bare repo so you can commit/push changes
5. **Local repos** → when origin is a local path, VDW also fetches remote-tracking branches (not just local branches)

### Link strategies (auto-selected)

| Condition | Strategy | Fallback |
|-----------|----------|----------|
| Same filesystem (macOS/Linux) | 🔗 HardLink | Symlink |
| Cross-filesystem (macOS/Linux) | 🔗 Symlink | — |
| Windows (directory) | 🪟 Junction | Symlink |
| Windows (file) | 🪟 Symlink | — |

Hardlinks are preferred because they take zero disk space and don't break when the source moves.

---

## 🖥️ VS Code Extension

VDW ships with a VS Code extension for visual management:

- ➕ **Add repos** from the command palette
- 🔗 **Link / Unlink** with one click
- 📋 **Browse** all dependency files in a custom tree view
- 🩺 **Status bar** shows workspace health

The extension invokes `vdw` from its Extension Host environment by default. If a GUI IDE does not inherit your shell `PATH`, set `vdw.cliPath` to the absolute binary path (for example, `~/.local/bin/vdw`) in VS Code/Qoder Settings JSON.

---

## 💻 Platform Support

| OS | Architectures | Link Strategy | Status |
|----|--------------|---------------|--------|
| 🍎 macOS | arm64 (M4), x86_64 | HardLink → Symlink | ✅ |
| 🐧 Linux | x86_64, aarch64 | HardLink → Symlink | ⏳ |
| 🪟 Windows | x86_64 (Git Bash / WSL) | Junction → Symlink | ⏳ |

> **macOS note**: If you encounter `SIGKILL` (exit code 137) with the release binary, clear the EDR quarantine:
> ```bash
> xattr -d com.alibaba.security.edr.antivirus.cloudquery /path/to/vdw
> ```
> The `install.sh` script handles this automatically.

---

## 🧑‍💻 Development

```bash
# Build
cargo build --release

# Run tests
cargo test

# Run all tests including integration
cargo test -- --ignored

# Benchmarks
cargo bench
```

### Project structure

```
vdw/
├── src/
│   ├── commands/       # CLI command implementations
│   ├── config/         # .vdwrc parsing + validation
│   ├── context/        # AI context file filtering
│   ├── link/           # Link engine (hardlink, symlink, junction)
│   ├── store/          # Content store (clone, fetch, metadata)
│   ├── types/          # Shared types
│   ├── utils/          # Utilities (git, fs, logging)
│   └── worktree/       # Git worktree management
├── tests/              # Integration tests
├── Cargo.toml
├── Makefile            # Build + install targets
└── install.sh          # Cross-platform installer
```

---

## 📄 License

Apache 2.0
