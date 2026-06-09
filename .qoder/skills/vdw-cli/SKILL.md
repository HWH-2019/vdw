---
name: vdw-cli
description: Set up VDW development workspaces with vdw init, repo add, and link. Use when preparing a development workspace after requirements analysis, before coding begins. Triggered by user confirmation after development requirements are understood. Creates workspace directories, initializes .vdwrc, adds repository dependencies, and offers IDE launch.
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
| **E** | Link workspace | `cd {workspace} && vdw link [--repo "{url}"]` |
| **F** | Unlink | `vdw unlink [--repo "{url}"]` |
| **G** | Remove repo | `vdw repo remove {name}` |
| **H** | List repos | `vdw repo list` |
| **I** | Validate config | `vdw validate` |
| **J** | Doctor | `vdw doctor` |
| **K** | AI context preview | `vdw context preview` |
| **L** | AI context stats | `vdw context stats` |
| **M** | Worktree list | `vdw worktree list` |
| **N** | Worktree create | `vdw worktree create {repo} {branch}` |
| **O** | Worktree delete | `vdw worktree delete {repo} {branch} [--force]` |
| **P** | Launch IDE | `bash {skill_dir}/scripts/open-workspace.sh {path}` |

Full command syntax: [reference.md](reference.md)

---

## Scenario Chains

Determine the user's situation, then follow the corresponding chain:

| Scenario | When to Use | Chain |
|----------|-------------|-------|
| **全流程** | Workspace dir doesn't exist yet, setting up from scratch | **A → B → C/D → P** |
| **工作空间构建** | `.vdwrc` exists but repos not linked | **A → C/D** |
| **添加单个仓库** | Adding a new dep to existing workspace | **H → A → C/D → E --repo** |
| **移除仓库** | Removing a dep from workspace | **H → G** |
| **诊断排查** | Something is broken | **I → J** |
| **AI 上下文** | Review/preview AI context | **K → L** |
| **Worktree 管理** | Create/delete branches for editable repos | **M → N/O** |
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

4. Ask user to confirm or provide missing URLs

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

**After `vdw repo add`**: check `~/.vdw/store/objects/{name}-*/metadata.json` — `commit_hash` must be non-empty and `size_bytes > 0`, otherwise fetch silently failed.

**Editable lock conflict**: `--editable` may auto-downgrade to read-only if the same branch is already editable in another workspace. This is expected.

### E — Link Workspace

```bash
cd {workspace_path} && vdw link
```

Wait for completion. Verify output: `✅ Workspace linked successfully!` with correct editable/read-only count.

### P — Launch IDE

```bash
bash {skill_dir}/scripts/open-workspace.sh "{workspace_path}"
```

Auto-detects: Qoder → VS Code → Cursor → Trae → Windsurf → Finder.

---

## Important Notes

- **`vdw` in PATH** — if not found, try `~/.local/bin/vdw`
- **Branch required** — always `--branch`, default `main` if unknown
- **Cleanup on failure** — `rm -rf` the workspace dir and retry
- **Full CLI reference** — see [reference.md](reference.md) for all commands and options
