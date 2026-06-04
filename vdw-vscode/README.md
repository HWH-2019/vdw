# 🏗️ VDW for VS Code

> **Multi-repo dependency management, right in your IDE.**  
> Manage cross-repository dependencies, link files into your workspace, and feed AI context to your coding assistant — all from VS Code.

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| 📂 **Dependency Tree View** | See all your repos at a glance — linked vs unlinked, editable vs read-only |
| ➕ **Add Repo Wizard** | Interactive dialog to add remote or local repos with scope configuration |
| 🔗 **One-Click Link / Unlink** | Link or unlink individual repos directly from the tree |
| 🔄 **Relink All** | Unlink all + re-link with updated `.vdwrc` config in one command |
| 🩺 **Status Bar** | Real-time dependency count and file context size |
| 🤖 **AI Context** | Filter files per dependency scope — only expose what matters to LLMs |
| 📝 **Auto-Open Editable Repos** | Automatically registers git worktrees in VS Code Source Control |
| ⚡ **Config Watcher** | Auto-refresh when `.vdwrc` changes |

---

## 🖼️ What It Looks Like

### Activity Bar & Tree View
The VDW icon in the activity bar opens the **Dependencies** panel:

```
📦 VDW Workspace
├── ui-lib@main          ✅ Linked  [可编辑]
│   ├── 📁 ./packages/ui-lib
│   ├── 🔍 src/**/*.ts
│   └── ✅ 已链接 (editable)
├── api-service@develop   ⚪ Not linked [只读]
│   ├── 📁 ./services/api
│   ├── 🔍 src/**/*.rs
│   └── ⚪ 未链接 (read-only)
└── protos@v2.0.0        ✅ Linked  [只读]
```

- **Green link icon** → repo is linked and files exist in workspace
- **Gray link icon** → repo is configured but not yet linked
- **Inline actions**: ⛓️ Link / 🔗 Unlink / 🗑️ Remove
- **Tooltip**: full repo URL, branch, scope, editable status

### Status Bar
```
🔗 VDW: 3 deps, 1,247 files
```
Shows dependency count and total files in AI context. Click to open command palette.

---

## 🚀 Getting Started

### Prerequisites
- **[VDW CLI](../vdw)** — the Rust CLI that powers all operations
- **VS Code** 1.85+
- A workspace with a `.vdwrc` file

### Install the CLI

```bash
# From the vdw repo directory
cd vdw && make install

# Or globally
cargo install --path .
```

See the [VDW CLI README](../vdw/README.md) for detailed installation options.

### Activate the Extension

1. Open a folder containing a `.vdwrc` file
2. The extension activates automatically (triggered by `workspaceContains:.vdwrc`)
3. Look for the 🔗 VDW icon in the activity bar

> **No `.vdwrc` yet?** Run `VDW: Initialize Workspace` from the command palette, or use `vdw init` in the terminal.

---

## 🎮 Commands

All commands are accessible via the **Command Palette** (`Cmd+Shift+P` / `Ctrl+Shift+P`):

| Command | Tree View | Description |
|---------|-----------|-------------|
| `VDW: Add Repository` | ➕ | Interactive wizard to add a remote or local repo |
| `VDW: Initialize Workspace` | — | Create a `.vdwrc` in the current folder |
| `VDW: Relink allRepo` | 🔄 | Unlink all + re-link with updated config |
| `VDW: Validate Configuration` | — | Check `.vdwrc` for errors |
| `VDW: Refresh Context` | 🔃 | Rebuild the file context and update stats |
| `Link Repository` | ⛓️ (inline) | Link a specific unlinked repo |
| `Unlink Repository` | 🔗 (inline) | Unlink a specific linked repo |
| `Remove Repository` | 🗑️ (context menu) | Unlink + remove from `.vdwrc` |

---

## ⚙️ Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `vdw.cliPath` | `vdw` | Path to the VDW CLI binary (auto-detected if not set) |
| `vdw.autoOpenGitRepositories` | `true` | Auto-open editable repos in Source Control |
| `vdw.autoOpenDelay` | `500` | Delay (ms) between opening each Git repo |

### CLI Binary Detection Order

The extension finds the `vdw` binary in this order:

1. **`vdw.cliPath` setting** — explicit path if configured
2. **`VDW_DEBUG=true`** — uses `target/debug/vdw` for development
3. **`./vdw/vdw`** — local binary in the workspace repo
4. **System `PATH`** — any `vdw` found globally

---

## 📋 .vdwrc Configuration

The extension reads the `.vdwrc` YAML file at your workspace root:

```yaml
workspace:
  name: my-workspace
  version: "1.0"

dependencies:
  - repo: "https://github.com/org/ui-lib.git"
    path: libs/ui-lib
    branch: main
    scope:
      include:
        - "src/components/**"
      exclude:
        - "**/*.test.ts"
    editable: false
```

Each dependency appears as a tree item with its link status, scope filters, and version info.

---

## 🔧 How It Works

```
┌───────────────────────────────────────┐
│          VS Code Extension            │
│                                       │
│  TreeView ◄── ConfigService ◄── .vdwrc│
│     │                                 │
│     ├── VDWCliService ──► vdw link    │
│     ├── ContextService ──► file list  │
│     ├── AutoOpenService ─► Source Ctrl│
│     └── StatusBar ──► dep count       │
└───────────┬───────────────────────────┘
            │ spawn
┌───────────▼───────────────────────────┐
│        VDW CLI (Rust binary)          │
│   (link, unlink, validate, init)      │
└───────────────────────────────────────┘
```

The extension is a **thin UI layer** over the VDW Rust CLI. Every operation (link, unlink, validate, init) spawns the CLI binary and parses its output. The extension's job is to:

- Read and cache the `.vdwrc` configuration
- Determine which repos are linked (by checking if paths exist)
- Present an interactive tree view with link/unlink actions
- Auto-open Git worktrees in VS Code Source Control
- Track file context statistics for AI assistance

---

## 🧑‍💻 Development

```bash
# Install dependencies
npm install

# Compile TypeScript
npm run compile

# Watch mode
npm run watch

# Generate the marketplace icon (PNG from SVG)
npm run generate-icon

# Run tests
npm test
```

### Debug Mode

Set `VDW_DEBUG=true` to use the debug build of the Rust CLI:

```bash
# In VS Code terminal or .env
VDW_DEBUG=true
```

The extension will use `../vdw/target/debug/vdw` instead of the system binary.

---

## 📦 Publishing

```bash
# Package as VSIX (ready for manual installation)
npm run package

# Or publish to marketplace (requires publisher auth)
npm run publish
```

**Prerequisites for publishing:**
1. Install `vsce`: `npm install -g @vscode/vsce`
2. Generate the PNG icon: `npm run generate-icon`
3. Create a publisher on the [VS Code Marketplace](https://marketplace.visualstudio.com/manage)
4. Update `publisher` in `package.json`
5. Update the repository URL in `package.json` to point to the real repo
6. Run `npm run package` to create a `.vsix` file

The `scripts/` folder is excluded from the VSIX via `.vscodeignore`.

---

## 🗺️ Project Structure

```
vdw-vscode/
├── src/
│   ├── commands/          # Command handlers (link, unlink, add, etc.)
│   ├── providers/         # TreeView, FileProvider, StatusBar
│   ├── services/          # CLI, Config, Context, AutoOpen
│   ├── types/             # TypeScript interfaces
│   ├── utils/             # Logger
│   └── extension.ts       # Entry point & activation
├── resources/icons/       # SVG + PNG icons
├── scripts/               # Icon generation utility
├── package.json
└── tsconfig.json
```

---

## 📄 License

MIT
