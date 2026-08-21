# 🏗️ VDW — Virtual Dev Workspace

> **Cross-repo development middleware for the AI era**  
> Seamlessly stitch multiple repositories into one development workspace, then serve the context to your AI coding assistant.

[![Rust](https://img.shields.io/badge/Rust-🦀-orange?style=flat-square)](https://www.rust-lang.org)
[![VS Code](https://img.shields.io/badge/VS%20Code-Extension-blue?style=flat-square)](https://code.visualstudio.com)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=flat-square)](LICENSE)

---

## 🤔 What is VDW?

Modern development doesn't live in a single repo. VDW solves this by giving you:

1. 🗄️ A **content-addressable store** on your machine that deduplicates repos
2. 🔗 **Smart linking** — expose only the files you want via hardlinks / symlinks
3. 🤖 **AI context filtering** — generate a clean file list for LLM-based coding assistants
4. 📝 **Editable dependencies** — repos added with `--editable` use git worktrees so you can modify code with full git support

Think of it as **`npm link` but for any language**, combined with **AI-aware file scoping**.

---

## 📁 Project Structure

```
agent-vdm/
├── vdw/                    # 🦀 Rust CLI — core engine
├── vdw-vscode/             # 🧩 VS Code Extension — GUI frontend
├── vdw-test/               # 🧪 Test workspace with sample .vdwrc
├── ai_workspace/           # 📋 PRD & technical design documents
├── .github/workflows/      # ⚙️ CI/CD pipeline
└── DEBUGGING_GUIDE.md      # 🐛 Debugging guide
```

### Sub-projects

| Project | Description | Documentation |
|---------|-------------|---------------|
| 🦀 **[vdw](vdw/)** | Rust CLI — clone, link, worktree, context filtering | [vdw/README.md](vdw/README.md) |
| 🧩 **[vdw-vscode](vdw-vscode/)** | VS Code extension — tree view, add repo wizard, status bar | [vdw-vscode/README.md](vdw-vscode/README.md) |

### Design Documents

| Document | Description |
|----------|-------------|
| [VDW 需求文档 v1.0](ai_workspace/VDW虚拟工作空间_需求文档_v1.0.md) | Original requirements specification |
| [VDW 技术设计文档 v1.0](ai_workspace/VDW虚拟工作空间_技术设计文档_v1.0.md) | Technical architecture & design |
| [VDW 配置重构需求文档 v1.0](ai_workspace/VDW配置重构_需求文档_v1.0.md) | Config schema refactoring (remote/local separation) |
| [VDW 插件功能增强 v1.0](ai_workspace/VDW插件功能增强_需求文档_v1.0.md) | VS Code plugin feature enhancements |
| [VDW 打包与分发 v1.0](ai_workspace/VDW打包与分发_需求文档_v1.0.md) | Unified packaging naming, macOS .dmg installer, scripts/ directory |

---

## 📦 Installation

The CLI is required. The VS Code extension is optional, but recommended for repository management and Source Control integration.

Install the CLI and VSIX from the same release version whenever possible; newer extension commands may not exist in an older CLI.

### 1. Install the VDW CLI

#### Option A — Download a release (no Rust required)

Download the archive for your platform from [GitHub Releases](https://github.com/HWH-2019/vdw/releases):

| Platform | Release asset |
|----------|---------------|
| macOS Apple Silicon | `vdw_darwin_arm64_<version>.tar.gz` |
| macOS Intel | `vdw_darwin_x86_64_<version>.tar.gz` |
| Linux x86_64 | `vdw_linux_x86_64_<version>.tar.gz` |
| Linux ARM64 | `vdw_linux_arm64_<version>.tar.gz` |
| Windows x86_64 | `vdw_win32_x86_64_<version>.tar.gz` |

On macOS or Linux:

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

Add the install directory to your shell if necessary:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc  # use ~/.bashrc for Bash
source ~/.zshrc
vdw --version
```

On Windows, extract the archive, rename the executable to `vdw.exe`, move it to a directory on `PATH`, and verify with `vdw --version` in PowerShell or Git Bash.

#### Option B — Build and install from source

Requires Git and the [Rust toolchain](https://rustup.rs).

```bash
git clone https://github.com/HWH-2019/vdw.git vdw-src
cd vdw-src/vdw
make install
```

The installer builds a release binary, installs it to `~/.local/bin/vdw`, handles known macOS quarantine attributes, and verifies the result. Developers who already have all prerequisites can use `make install-quick` instead. For a custom directory:

```bash
./install.sh --dir /your/install/directory
```

Use the script on macOS, Linux, or WSL. For a native Windows source build, run `cargo install --path . --locked` inside `vdw-src/vdw`, or use the pre-built Windows archive.

See the [CLI installation guide](vdw/README.md#install-the-cli) for upgrades, uninstalling, and troubleshooting.

### 2. Install the VS Code / Qoder extension

#### Option A — Install the release VSIX

1. Download `vdw_vscode_<version>.vsix` from [GitHub Releases](https://github.com/HWH-2019/vdw/releases).
2. In VS Code or Qoder, open **Extensions**, choose **Install from VSIX...**, and select the downloaded file.
3. If the `code` command is available, VS Code can also install it from a terminal:

```bash
VERSION=0.6.2  # replace with the downloaded VSIX version
code --install-extension "./vdw_vscode_${VERSION}.vsix" --force
```

#### Option B — Package the extension from source

Requires Node.js 20+ and npm.

```bash
cd vdw-src/vdw-vscode
npm ci
npm run package
```

`npm run package` runs icon generation and TypeScript compilation, then creates `vdw-vscode-<version>.vsix` in the current directory. Install that file using **Install from VSIX...** or the `code --install-extension` command above.

The extension launches `vdw` from the IDE process environment. If the IDE cannot see your shell `PATH`, set an absolute path in Settings JSON:

```json
{
  "vdw.cliPath": "/Users/your-name/.local/bin/vdw"
}
```

Use `C:\\Users\\your-name\\.local\\bin\\vdw.exe` on Windows.

### 3. Verify the installation

```bash
vdw --version
vdw --help
```

Then open a folder containing `.vdwrc`, run **VDW: Validate Configuration** from the Command Palette, and check **View → Output → VDW** if the extension reports an error.

---

## 🚀 Quick Start

### 1. Initialize a Workspace

```bash
cd your-project
vdw init
```

### 2. Add a Repository

```bash
# Remote repo via SSH
vdw repo add git@github.com:org/api-service.git --path services/api --branch main

# Local repo (editable)
vdw repo add /path/to/local/repo --path packages/shared --branch feature/dev --editable
```

### 3. Link Files

```bash
vdw link
```

VDW walks each dependency's source directory, applies scope filters, and creates hardlinks (or symlinks) at the configured path.

---

## 📋 Configuration

A `.vdwrc` file (YAML) defines your workspace:

```yaml
workspace:
  name: my-monorepo
  version: "1.0"

dependencies:
  # Remote repository (Git URL)
  - remote: "git@github.com:org/ui-lib.git"
    path: libs/ui-lib
    branch: main
    scope:
      include:
        - "src/components/**"
        - "src/hooks/**"
      exclude:
        - "**/*.test.ts"
        - "**/node_modules/**"

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

### Dependency Fields

| Field | Required | Description |
|-------|----------|-------------|
| `remote` | ✅ | Git URL (`https://`, `git@`, or empty for local repos) |
| `local` | ✅ (if no remote) | Local repository path on disk |
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
                    └───────────┬─────────────────────────┘
                                │ link / unlink
                    ┌───────────▼─────────────────────────┐
                    │     VDW Content Store                │
                    │     ~/.vdw/store/objects/             │
                    └───────────┬─────────────────────────┘
                                │ clone / fetch
                    ┌───────────▼─────────────────────────┐
                    │      Remote / Local Repositories     │
                    └─────────────────────────────────────┘
```

**Link strategies (auto-selected):**

| Condition | Strategy | Fallback |
|-----------|----------|----------|
| Same filesystem (macOS/Linux) | 🔗 HardLink | Symlink |
| Cross-filesystem (macOS/Linux) | 🔗 Symlink | — |
| Windows (directory) | 🪟 Junction | Symlink |
| Windows (file) | 🪟 Symlink | — |

---

## 🖥️ VS Code Extension

The VS Code extension provides a visual GUI over the CLI:

- 📂 **Dependency Tree View** — see all repos at a glance
- ➕ **Add Repo Wizard** — interactive dialog with branch selection (fuzzy search)
- 🔗 **One-Click Link / Unlink** — inline actions on tree items
- 🔄 **Relink All** — unlink + re-link with updated config
- 📝 **Auto-Open Git Repos** — editable repos auto-register in Source Control
- ⚡ **Config Watcher** — auto-refresh when `.vdwrc` changes

See [vdw-vscode/README.md](vdw-vscode/README.md) for full documentation.

### 🤖 AI Agent Skill

VDW also ships with a **Qoder skill** ([vdw-cli](.qoder/skills/vdw-cli/SKILL.md)) that lets AI coding agents automatically set up workspaces:

- 🏁 Initialize `.vdwrc` after requirements analysis
- 📥 Add repos with `--editable` or read-only mode
- 🔗 Link dependencies and offer IDE launch

The skill follows a scenario-chain model — it asks for confirmation, then picks the right sequence of CLI commands based on your situation.

---

## 💻 Platform Support

| OS | Architectures | Link Strategy | Status |
|----|--------------|---------------|--------|
| 🍎 macOS | arm64 (M4), x86_64 | HardLink → Symlink | ✅ |
| 🐧 Linux | x86_64, aarch64 | HardLink → Symlink | ⏳ |
| 🪟 Windows | x86_64 (Git Bash / WSL) | Junction → Symlink | ⏳ |

---

## 🧑‍💻 Development

```bash
# Build CLI
cd vdw && cargo build --release

# Run CLI tests
cargo test

# Build VS Code extension
cd vdw-vscode && npm ci && npm run compile

# Package VSIX
npm run package
```

---

## 📄 License

Apache 2.0
