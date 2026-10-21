# VDW CLI Command Reference

Full command syntax and options for every `vdw` subcommand.

---

## `vdw config`

Configure the parent directory used for every new workspace. This setting is required before the first `vdw init`; VDW does not select a default workspace location.

```bash
vdw config set workspace-root /absolute/path/to/workspaces
vdw config set workspace-root ~/workspaces
vdw config show
vdw config show --json
```

The command creates the selected directory, resolves it to an absolute path, and stores it as `workspace_root` in `~/.vdw/config.json`. Relative paths are rejected so workspace placement never depends on the current directory.

---

## `vdw init`

Initialize a named VDW workspace beneath the configured workspace root. The workspace name becomes both the directory name and `workspace.name` in `.vdwrc`.

```bash
vdw init my-workspace                         # Default template
vdw init my-fullstack --template fullstack   # Pre-filled examples
```

`.vdwrc` structure:
```yaml
workspace:
  name: my-workspace
  version: "1.0"

dependencies:
  - remote: "https://github.com/org/repo.git"   # or git@ for SSH
    local: ""                                    # set ONLY for local-only repos
    path: ./packages/repo-name
    branch: main                                 # or tag: v1.0, commit: abc123
    editable: false                              # true for git worktree
    scope:
      include:
        - "src/**/*.rs"
      exclude:
        - "**/*.test.rs"
```

| Field | Required | Description |
|-------|----------|-------------|
| `remote` | ✅ (if no local) | Git URL: `https://`, `git@`, or `http://` |
| `local` | ✅ (if no remote) | Absolute path to local git repo |
| `origin` | ❌ | Legacy migration field; new configurations must not set it |
| `path` | ✅ | Relative workspace path to link into |
| `branch` / `tag` / `commit` | ✅ (pick one) | Version specifier |
| `scope.include` | ✅ | Glob patterns for files to expose |
| `scope.exclude` | ❌ | Glob patterns to filter out |
| `editable` | ❌ | Default `false`; set `true` for git worktree mode |

---

## `vdw repo add`

Add a repository dependency to the workspace. Clones to `~/.vdw/store/objects/` and updates `.vdwrc`.

```bash
# Remote via SSH
vdw repo add git@github.com:org/api-service.git --path services/api --branch develop

# Remote via HTTPS
vdw repo add https://github.com/org/ui-lib.git --path libs/ui-lib --branch main

# Local repo (editable)
vdw repo add /path/to/local/repo --path packages/shared --branch main --editable

# Remote, editable
vdw repo add git@github.com:org/shared.git --path packages/shared --branch master --editable
```

| Option | Description |
|--------|-------------|
| `url` | Remote URL or local path (required, positional) |
| `--path` / `-p` | Workspace link path (default: `./packages/{repo_name}`) |
| `--branch` / `-b` | Git branch to track (required when version not specified) |
| `--editable` | Enable git worktree mode (default: false) |

**Editable lock behavior**: If the same repo+branch is already editable in another workspace, the command auto-downgrades to read-only with a warning message.

**Silent fetch failure**: Even with exit code 0, verify the fetch succeeded:
```bash
cat ~/.vdw/store/objects/{repo_name}-*/metadata.json
# commit_hash must be non-empty, size_bytes > 0
```

---

## `vdw repo remove`

Remove a dependency from the current workspace. It unlinks managed files, removes the current workspace's editable worktree, and updates `.vdwrc` and `.vdw.lock`. The shared Store entry is retained for reuse by other workspaces.

```bash
# By URL
vdw repo remove https://github.com/org/ui-lib.git

# By generated name (fuzzy)
vdw repo remove ui-lib
```

> Use `vdw repo list` to find the generated name if unsure.

---

## `vdw repo list`

Show all dependencies in the current workspace.

```bash
vdw repo list
```

Output:
```
REPO                                     BRANCH       EDIT     PATH
--------------------------------------------------------------------------------
git@gitlab.alibaba-inc.com:eleme-do…     main         yes      ./packages/alsc.sales_aoi_service
ui-lib-a1b2c3d4                          develop      no       ./libs/ui-lib

Total: 2 repository(ies)
```

---

## `vdw link`

Materialize dependencies from the shared Store into the workspace.

```bash
vdw link                    # Link all dependencies
vdw link --repo git@github.com:org/ui-lib.git   # Link single repo
vdw link --frozen           # Use only matching, locally available locked commits
```

Editable dependencies become complete native Git worktrees at their configured workspace path. Read-only dependencies resolve the declared branch/tag/commit into an immutable Store checkout at `checkouts/<commit-sha>`, apply scope filters, then create links at the configured path.

A normal link creates missing or config-stale `.vdw.lock` entries but preserves an existing matching pin. `--frozen` performs a complete preflight before changing projections, never fetches, and fails if the lock does not match `.vdwrc` or its commit is absent from the Store. Editable HEAD may contain local commits on top of the lock, but must include the locked commit in its history.

**Link strategies** (auto-selected):

| Condition | Strategy | Fallback |
|-----------|----------|----------|
| Same filesystem | HardLink | Symlink |
| Cross-filesystem | Symlink | — |
| Windows (directory) | Junction | Symlink |
| Windows (file) | Symlink | — |

---

## `vdw unlink`

Remove all hardlinks/symlinks from the workspace. Store, `.vdwrc`, and `.vdw.lock` are preserved.

```bash
vdw unlink                  # Unlink all
vdw unlink --repo git@github.com:org/ui-lib.git   # Unlink single repo
```

---

## `vdw sync`

Fetch declared revisions and compare their resolved commits with `.vdw.lock`.

```bash
vdw sync --dry-run --json   # report drift; do not change the lock
vdw sync --update-lock      # accept all current resolutions
```

Dry-run is the default when `--update-lock` is omitted. It may update the shared Store's fetched refs, but it never changes `.vdw.lock` or workspace projections. Review `added`, `updated`, `config_changed`, and removed entries before accepting changes. For an editable dependency, reconcile a worktree that is behind or unrelated to the newly locked commit before linking.

---

## `vdw validate`

Validate `.vdwrc` syntax and consistency.

```bash
vdw validate
```

---

## `vdw store status`

Inspect the effective shared Store path, registered workspace health, object disk usage, and live config/lock/worktree references.

```bash
vdw store status
vdw store status --json
```

VDW maintains `~/.vdw/workspaces.json` after `init`, repository config changes, successful `link`, and `sync --update-lock`. `reference_inventory_complete: false` means one or more references could not be verified, so no object is considered eligible for GC.

The effective Store path is read from `~/.vdw/config.json`. Absolute paths and `~/...` are supported; relative values are anchored at `~/.vdw`.

---

## `vdw store gc`

Generate a garbage-collection plan without deleting Store data.

```bash
vdw store gc --dry-run
vdw store gc --dry-run --json
```

In 0.9.1, `--dry-run` is required and `deletion_performed` is always `false`. A candidate must be older than `gc_threshold_days`, have valid metadata and a readable bare repository, and have no registered `.vdwrc`, `.vdw.lock`, or Git worktree references. Never remove Store object directories manually from an AI workflow.

---

## `vdw doctor`

Run diagnostics on the workspace — checks store health, link integrity, and config consistency.

```bash
vdw doctor
```

---

## `vdw context preview`

List files that would be included in AI context (scoped by `.vdwrc` include/exclude rules).

```bash
vdw context preview
vdw context preview --json   # stable machine-readable contract for AI agents
```

JSON output contains the workspace root/name, total file count, and one entry per repository with `linked` state and workspace-relative `files`.

---

## `vdw context stats`

Show file count and total size per dependency in the AI context.

```bash
vdw context stats
vdw context stats --json
```

---

## `vdw worktree`

Manage git worktrees for editable repositories.

```bash
# List all worktrees
vdw worktree list

# Create a worktree for a branch
vdw worktree create {repo_name} {branch}

# Delete a worktree
vdw worktree delete {repo_name} {branch}
vdw worktree delete {repo_name} {branch} --force
```


## Workspace lifecycle (0.10+)

- Inspect: `vdw lifecycle capabilities`, `vdw workspace list --json`.
- Preview archive: `vdw workspace archive <id>... --preview --json`.
- Preview restore: `vdw workspace restore <archive_id> [--target /path] [--branch new-name] --preview --json`.
- Adjust: `vdw lifecycle plan adjust <plan_id> --exclude-workspace <id> --json`.
- Confirm/execute: `vdw lifecycle plan execute <plan_id> --confirm --json`.
- Read complete results: `vdw lifecycle operation status <operation_id> --json`.
- Recover: `vdw lifecycle operation resume <operation_id> --preview --json`.
- Retention: `vdw config set idle-days 30 [--workspace <id>] --preview --json`.
- No-cleanup list: `vdw workspace protect add <id> --reason <text>`; removal uses `protect remove <id> --preview --json`, followed by plan confirmation.
- Location: `vdw config set archive-root /absolute/path --preview --json`; confirming automatically synchronizes history. Use plan execute --background for a worker, operation watch for progress, and operation pause/continue/cancel for migration control.
- Automation: `vdw lifecycle auto enable --preview --json`; confirm the policy plan. Use --no-scheduler with an external scheduler. Disable with `vdw lifecycle auto disable`.
- Effective settings: `vdw config show --effective --workspace <id> --json`.

Any dirty repository skips its whole workspace. Always return every scoped workspace and all archived/not-archived/skipped reasons. Never force deletion to bypass a skip. Base snapshots preserve materials outside packages/actual managed dependencies; optional AI is configured through a reviewed lifecycle-file plan. Cloud and message adapters remain reserved.
