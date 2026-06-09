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

## 🚀 Quick Start

### 1. Install the CLI

```bash
cd vdw && make install-quick
```

> See [vdw/README.md](vdw/README.md) for full installation options including the install script and pre-built binaries.

### 2. Install the VS Code Extension

```bash
cd vdw-vscode && npm install && npm run compile && npm run package
code --install-extension vdw-vscode-*.vsix --force
```

### 3. Initialize a Workspace

```bash
cd your-project
vdw init
```

### 4. Add a Repository

```bash
# Remote repo via SSH
vdw repo add git@github.com:org/api-service.git --path services/api --branch main

# Local repo (editable)
vdw repo add /path/to/local/repo --path packages/shared --branch feature/dev --editable
```

### 5. Link Files

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
cd vdw-vscode && npm install && npm run compile

# Package VSIX
npm run package
```

---

## 📄 License

Apache 2.0
