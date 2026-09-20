---
name: vdw-cli
description: Set up reproducible VDW development workspaces with vdw init, repo add, sync, lock, and frozen link. Use when preparing or refreshing a development workspace after requirements analysis, before coding begins. Triggered by user confirmation after development requirements are understood. Creates workspace directories, initializes .vdwrc, resolves .vdw.lock, materializes repository dependencies, and offers IDE launch.
---

# VDW CLI Workspace Setup

## Quick Start

After analyzing development requirements, ask: *"Ready to set up the VDW workspace for this task?"*

Then follow the **scenario chain** that matches the user's situation (see below). Do NOT auto-trigger — always ask for explicit confirmation first.

---

## Capability Catalog

Each step is a discrete, reusable operation. Scenarios chain them together.

| # | Capability | Command / Action |
|---|-----------|------------------|
| **A** | Gather repos from requirements | Scan task description for Git URLs, local paths, branch names, editable flags. Present summary table to user. Ask for missing info. |
| **B** | Init workspace | Ask user for workspace name and parent directory (default: `~/.vdw/workspaces/`). Folder name = workspace name. `mkdir -p {parent}/{name}` + `cd {path} && vdw init [--template fullstack]` |
| **C** | Add remote repo | `vdw repo add "{url}" --path ./packages/{name} --branch {branch} [--editable]` |
| **D** | Add local repo | `vdw repo add {/absolute/path} --path ./packages/{name} --branch {branch} [--editable]` |
| **E** | Link workspace | `cd {workspace} && vdw link --frozen [--repo "{url}"]` after lock resolution |
| **F** | Unlink | `vdw unlink [--repo "{url}"]` |
| **G** | Remove repo | `vdw repo remove {name}` |
| **H** | List repos | `vdw repo list` |
| **I** | Validate config | `vdw validate` |
| **J** | Check topology | `vdw status --json` |
| **K** | Doctor | `vdw doctor` |
| **L** | AI context preview | `vdw context preview --json` |
| **M** | AI context stats | `vdw context stats --json` |
| **N** | Worktree list | `vdw worktree list` |
| **O** | Worktree create | `vdw worktree create {repo} {branch}` |
| **P** | Worktree delete | `vdw worktree delete {repo} {branch} [--force]` |
| **Q** | Launch IDE | `bash {skill_dir}/scripts/open-workspace.sh {path}` |
| **R** | Inspect revision drift | `vdw sync --dry-run --json` |
| **S** | Update dependency lock | `vdw sync --update-lock [--json]` |

Full command syntax: [reference.md](reference.md)

---

## Scenario Chains

Determine the user's situation, then follow the corresponding chain:

| Scenario | When to Use | Chain |
|----------|-------------|-------|
| **全流程** | Workspace dir doesn't exist yet, setting up from scratch | **A → B → C/D → S → E → I → J → L/M → Q** |
| **工作空间构建** | `.vdwrc` exists but repos not linked | **A → R → S → E → I → J** |
| **添加单个仓库** | Adding a new dep to existing workspace | **H → A → C/D → R → S → E --repo → I → J** |
| **依赖更新** | Inspect and accept branch/tag movement | **R → confirm → S → E → J** |
| **移除仓库** | Removing a dep from workspace | **H → G** |
| **诊断排查** | Something is broken | **I → J → K** |
| **AI 上下文** | Review/preview AI context | **L → M** |
| **Worktree 管理** | Create/delete branches for editable repos | **N → O/P** |
| **清理空间** | Remove all links, keep config | **F → E** (if re-link desired) |

---

## Capability Details

### A — Gather Repositories

1. Scan the requirements/task description for:
   - Git remote URLs (`git@`, `https://...git`, `http://...git`)
   - Local paths mentioned as dependencies
   - Branch names in context
2. Determine `editable` flag: `true` if task requires modifying the repo
3. Present a summary table:

```
| Repo | URL/Path | Branch | Editable |
|------|----------|--------|----------|
| api-service | git@github.com:org/api.git | main | yes |
| ui-lib | /Users/dev/projects/ui-lib | feat-x | yes |
```

4. Ask user to confirm or provide missing URLs and branches. Never infer an unknown branch as `main`.

### B — Init Workspace

**Step 1: Ask the user** two questions before creating anything:
1. *"What name for this workspace?"* — this becomes both the folder name and `workspace.name` in `.vdwrc`
2. *"Where to create it?"* — parent directory, default: `~/.vdw/workspaces/`

Default location: `~/.vdw/workspaces/{workspace-name}/`. Folder name must match the workspace name.

**Step 2: Create and init** after user confirms:
```bash
mkdir -p {parent_dir}/{workspace_name}
cd {parent_dir}/{workspace_name} && vdw init
```

`.vdwrc` already exists → `vdw init` warns but doesn't error (acceptable).

### C/D — Add Repository

Repo name is derived from URL (last segment, `.git` stripped). Default path: `./packages/{repo_name}/`.

```bash
cd {workspace_path} && vdw repo add "{url}" --path "./packages/{name}" --branch "{branch}" [--editable]
```

**Editable flag**: `--editable` if modifying; omit for read-only reference. If unsure, ask.

`vdw repo add` updates the Store and `.vdwrc`; it does not materialize the configured workspace path. Resolve `.vdw.lock` with capability S, then run capability E.

**Editable lock conflict**: `--editable` may auto-downgrade to read-only if the same branch is already editable in another workspace. This is expected.

### E — Link Workspace

```bash
cd {workspace_path} && vdw link --frozen
```

Wait for completion. Verify output: `✅ Workspace linked successfully!` with correct editable/read-only count.

Then run `vdw validate` and `vdw status --json`. For every editable dependency, also read back its actual `git branch --show-current`, `git rev-parse HEAD`, and `git remote get-url origin` from the configured workspace path. Treat these values as authoritative; Store metadata alone is not proof of the linked branch or commit.

### R/S — Resolve Dependency Lock

Run `vdw sync --dry-run --json` first for an existing workspace. Present added, updated, config-changed, and removed dependencies; do not accept branch/tag movement without user confirmation. After confirmation, or immediately after adding dependencies to a new workspace, run `vdw sync --update-lock --json` and verify `.vdw.lock` records a full commit for every `.vdwrc` dependency. Then use frozen linking.

Dry-run may fetch Store refs but does not edit `.vdw.lock` or workspace projections. If an editable worktree does not contain a newly locked commit, stop and ask the user to reconcile that branch; never reset it.

### Q — Launch IDE

```bash
bash {skill_dir}/scripts/open-workspace.sh "{workspace_path}"
```

Auto-detects: Qoder → VS Code → Cursor → Trae → Windsurf → Finder.

---

## Important Notes

- **`vdw` in PATH** — if not found, try `~/.local/bin/vdw`
- **Branch required** — always pass the confirmed branch; stop and ask if it is unknown
- **Declarative config** — `.vdwrc` contains source intent only; never write managed worktree paths into `local` or `origin`
- **Reproducible linking** — prefer `vdw link --frozen` whenever `.vdw.lock` exists; use `vdw sync --dry-run --json` before changing pins
- **Preserve work** — before unlinking, removing, relinking, or moving a workspace, inspect `vdw status --json` and every editable repository's Git status
- **Cleanup on failure** — do not recursively delete the workspace. Diagnose with `vdw status`, `vdw conflicts list`, and `vdw doctor`; use VDW cleanup commands only after confirming their targets and dirty state
- **AI scope** — read `vdw context preview --json` before broad code discovery and use its workspace-relative `files` as the default read/search boundary. Expand beyond it only when the task requires related files, and report that expansion. This contract is not a filesystem security boundary for editable worktrees
- **Full CLI reference** — see [reference.md](reference.md) for all commands and options
