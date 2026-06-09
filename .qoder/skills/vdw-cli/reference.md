# VDW CLI Command Reference

Full command syntax and options for every `vdw` subcommand.

---

## `vdw init`

Initialize a new VDW workspace. Creates `.vdwrc` in the current directory.

```bash
vdw init                    # Default template
vdw init --template fullstack   # Pre-filled examples
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
| `origin` | ❌ | Original source path preserved when `local` overwritten by worktree |
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

Remove a dependency from the workspace. Unlinks files, deletes worktree, and removes from `.vdwrc`.

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

Create hardlinks/symlinks from the content store into the workspace.

```bash
vdw link                    # Link all dependencies
vdw link --repo git@github.com:org/ui-lib.git   # Link single repo
```

Walks each dependency's source files, applies scope filters, creates links at configured paths.

**Link strategies** (auto-selected):

| Condition | Strategy | Fallback |
|-----------|----------|----------|
| Same filesystem | HardLink | Symlink |
| Cross-filesystem | Symlink | — |
| Windows (directory) | Junction | Symlink |
| Windows (file) | Symlink | — |

---

## `vdw unlink`

Remove all hardlinks/symlinks from the workspace. Store and `.vdwrc` are preserved.

```bash
vdw unlink                  # Unlink all
vdw unlink --repo git@github.com:org/ui-lib.git   # Unlink single repo
```

---

## `vdw validate`

Validate `.vdwrc` syntax and consistency.

```bash
vdw validate
```

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
```

---

## `vdw context stats`

Show file count and total size per dependency in the AI context.

```bash
vdw context stats
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
