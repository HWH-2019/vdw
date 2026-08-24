# 🏗️ VDW for VS Code

> **Multi-repo dependency management, right in your IDE.**  
> Manage cross-repository dependencies, link files into your workspace, and feed AI context to your coding assistant — all from VS Code.

---

## ✨ Features

| Feature | Description |
|---------|-------------|
| 📂 **Repository Status View** | Compact Source Control rows combining VDW topology with live Git status |
| ➕ **Add Repo Wizard** | Interactive dialog to add remote or local repos with branch selection and scope configuration |
| 🔗 **One-Click Link / Unlink** | Link or unlink individual repos directly from the tree |
| 🔄 **Reconcile Workspace** | Refresh read-only projections and migrate legacy editable repos without replacing native worktrees |
| 🩺 **Status Bar** | Quiet repository summary that highlights workspace issues and dirty repositories |
| 🤖 **AI Context** | Filter files per dependency scope — only expose what matters to LLMs |
| 📝 **Native Editable Repos** | Lets Source Control discover workspace-native worktrees, then performs a best-effort refresh |
| ⚡ **Config Watcher** | Auto-refresh when `.vdwrc` changes |

---

## 🖼️ What It Looks Like

### Source Control Repository View
VDW contributes a compact **VDW Repositories** view to the native Source Control sidebar:

```
VDW REPOSITORIES
├── ⚠ ui-lib         feature/workspace · 12 changes · ↑1
├── ◉ api-service    develop
└── ✕ protos         v2.0.0 · missing
```

- Healthy repositories stay quiet; missing/foreign worktrees, projection conflicts, detached links, branch drift, and repositories not loaded by SCM are highlighted.
- Repository names and branches are separate, so long branch names do not hide repository identity.
- **Inline actions**: open Source Control, inspect Store/workspace diffs, reconcile topology, open a terminal, or reveal the repository in Explorer.
- Link appears inline for a missing repository, conflict inspection for a divergent projection, and reconcile for repairable topology drift. Unlink and remove remain in menus.
- Path, source, mode, and non-default AI scope remain available in the tooltip without consuming tree rows.

### Status Bar
```
⚠ VDW: 1 issue
```
Click to open a keyboard-friendly repository picker. Selecting an editable repository opens Source Control; selecting a read-only projection reveals it in Explorer.

---

## 🚀 Getting Started

### Prerequisites

- **[VDW CLI](https://github.com/HWH-2019/vdw/tree/main/vdw)** — the Rust CLI that powers all operations
- **VS Code** 1.85+
- **Qoder** or another VS Code-compatible IDE can install the same VSIX
- Node.js is only required when packaging the extension from source

### Install the CLI

```bash
# From the repository root
cd vdw
make install

# Verify the executable
vdw --version
```

See the [VDW CLI installation guide](https://github.com/HWH-2019/vdw/blob/main/vdw/README.md#install-the-cli) for pre-built archives, source installation, PATH setup, upgrades, and uninstalling.

### Install the extension

Use the same release version for the CLI and VSIX whenever possible. The extension invokes CLI subcommands directly, so mixing a newer extension with an older CLI can produce missing-command errors.

#### Option A — Download the release VSIX

1. Download `vdw_vscode_<version>.vsix` from [GitHub Releases](https://github.com/HWH-2019/vdw/releases).
2. Open the **Extensions** view in VS Code or Qoder.
3. Open the Extensions view menu (`...`), choose **Install from VSIX...**, and select the file.
4. Reload the IDE window when prompted.

If the VS Code `code` command is installed, the equivalent terminal command is:

```bash
VERSION=0.7.1  # replace with the downloaded VSIX version
code --install-extension "./vdw_vscode_${VERSION}.vsix" --force
```

#### Option B — Package from source

Requires Node.js 20+ and npm:

```bash
git clone https://github.com/HWH-2019/vdw.git vdw-src
cd vdw-src/vdw-vscode
npm ci
npm run package
```

The `prepackage` hook generates the PNG icon and compiles TypeScript automatically. The result is `vdw-vscode-<version>.vsix` in `vdw-vscode/`; install it with **Install from VSIX...** or the `code` command above.

### Configure the CLI path

The extension runs the CLI inside the IDE Extension Host. GUI applications do not always inherit the same `PATH` as your terminal, so an explicit absolute path is the most reliable setup:

```json
{
  "vdw.cliPath": "/Users/your-name/.local/bin/vdw"
}
```

Windows example:

```json
{
  "vdw.cliPath": "C:\\Users\\your-name\\.local\\bin\\vdw.exe"
}
```

Open **Preferences: Open Settings (JSON)** from the Command Palette to set it. The default value is `vdw`, which requires the binary to be available on the Extension Host's `PATH`.

### Activate the Extension

1. Open a folder containing a `.vdwrc` file.
2. The extension activates automatically (`workspaceContains:.vdwrc` / `.vdwrc.yml`, with startup activation as a fallback).
3. Open **Source Control** and expand **VDW Repositories**.
4. Run **VDW: Validate Configuration** from the Command Palette.
5. If validation fails to launch, open **View → Output**, select **VDW**, and verify `vdw.cliPath`.

> **No `.vdwrc` yet?** Run `VDW: Initialize Workspace` from the command palette, or use `vdw init` in the terminal.

### Upgrade or uninstall

- Upgrade: install the newer VSIX with **Install from VSIX...** again, or rerun `code --install-extension "./vdw_vscode_0.7.1.vsix" --force` with the new filename.
- Uninstall: remove **VDW - Virtual Dev Workspace** from the Extensions view, or run:

```bash
code --uninstall-extension vdw-team.vdw-vscode
```

Uninstalling the extension does not remove the CLI, `~/.vdw/store`, `.vdwrc`, or linked workspace files.

---

## 🎮 Commands

When upgrading a legacy editable projection, unlinking, or removing a repository finds different Store and workspace versions, the extension can open the corresponding VS Code diff. Missing files are also conflicts because hardlinks cannot propagate independent create/delete/rename operations. Resolve files individually or batch-accept all Store/workspace versions; VDW backs up both sides under `.vdw/conflict-backups/` before applying the selection.

All commands are accessible via the **Command Palette** (`Cmd+Shift+P` / `Ctrl+Shift+P`):

| Command | Tree View | Description |
|---------|-----------|-------------|
| `VDW: Add Repository` | ➕ | Interactive wizard to add a remote or local repo |
| `VDW: Initialize Workspace` | — | Create a `.vdwrc` in the current folder |
| `VDW: Reconcile Workspace` | menu | Refresh read-only links and migrate legacy editable projections; keep native editable worktrees in place |
| `VDW: Validate Configuration` | — | Check `.vdwrc` for errors |
| `VDW: Refresh Workspace Status` | 🔃 | Refresh configuration, aggregate VDW topology, AI context, and live Git status without changing workspace files |
| `VDW: Open Repository` | status bar | Pick a repository and navigate to Source Control or Explorer |
| `Open in Source Control` | inline | Register and reveal an editable worktree in Source Control |
| `Open in Terminal` | inline | Start a terminal at the repository root |
| `Reveal in Explorer` | inline | Reveal the repository root in the native Explorer |
| `Link Repository` | inline when missing | Link a specific missing repository |
| `Inspect and Resolve VDW Conflicts` | inline on conflict | Compare Store/workspace files and accept either side individually or in bulk |
| `Unlink Repository` | context menu | Unlink a specific available repository |
| `Remove Repository` | 🗑️ (context menu) | Unlink + remove from `.vdwrc` |

---

## ⚙️ Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `vdw.cliPath` | `vdw` | CLI command or absolute binary path; the default requires `vdw` on the Extension Host's `PATH` |
| `vdw.autoOpenGitRepositories` | `true` | Best-effort refresh of native editable workspace repos in Source Control |
| `vdw.autoOpenDelay` | `500` | Deprecated compatibility setting; repository registration now waits for Git events |

### CLI Binary Resolution

The extension resolves the CLI in this order:

1. **`VDW_CLI_PATH`** — debugger-injected override used by this repository's launch configuration.
2. **`vdw.cliPath`** — defaults to `vdw`; set an absolute path for GUI IDE sessions that do not inherit your shell `PATH`.
3. Development fallbacks (`VDW_DEBUG=true` and a workspace-local `./vdw/vdw`) are used only when `vdw.cliPath` is empty.
4. **System `PATH`** — used when the resolved command is `vdw`.

---

## 📋 .vdwrc Configuration

The extension reads the `.vdwrc` YAML file at your workspace root:

```yaml
workspace:
  name: my-workspace
  version: "1.0"

dependencies:
  # Remote repository (Git URL)
  - remote: "https://github.com/org/ui-lib.git"
    path: libs/ui-lib
    branch: main
    scope:
      include:
        - "src/components/**"
      exclude:
        - "**/*.test.ts"
    editable: false

  # Local repository (path on disk)
  - remote: ''
    local: /path/to/local/repo
    path: packages/my-lib
    branch: feature/dev
    editable: true
    scope:
      include:
        - "src/**/*.ts"
```

Each dependency appears as one compact repository row. Dynamic Git and IDE status is kept in memory and is never written to `state.json`.

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

The extension is a **thin UI layer** over the VDW Rust CLI. Mutating operations remain CLI-owned. `vdw status --json` supplies all topology and ownership diagnostics in one invocation, while the built-in Git API supplies live branch, change, and ahead/behind data. Since VDW 0.7, editable dependencies are complete Git worktrees at `dependency.path`; `scope` filters AI context rather than the physical editable checkout. The extension's job is to:

- Read and cache the `.vdwrc` configuration
- Merge aggregate CLI topology with live Git state in memory
- Present an interactive tree view with link/unlink actions
- Let the built-in Git extension discover nested workspace repositories and request a public-API refresh after Git initializes
- Close a native worktree only when unlink/remove is about to delete it; relink leaves the repository registered
- Track file context statistics for AI assistance

Git change events reuse a short-lived topology cache; explicit refresh and link/relink operations force a new aggregate audit. No dynamic UI or Git status is written to `.vdw/state.json`.

---

## 🧑‍💻 Development

```bash
# Install exact locked dependencies
npm ci

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
│   ├── commands/          # Command handlers (addRepo, index)
│   ├── providers/         # workspaceTreeView, fileProvider, statusBar
│   ├── services/          # vdwCliService, configService, contextService, autoOpenService
│   ├── types/             # TypeScript interfaces
│   ├── utils/             # git (branch listing), logger
│   └── extension.ts       # Entry point & activation
├── resources/icons/       # SVG + PNG icons
├── scripts/               # Icon generation utility
├── package.json
└── tsconfig.json
```

---

## 📄 License

Apache 2.0
