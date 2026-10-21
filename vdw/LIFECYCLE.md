# Workspace lifecycle (0.10.0)

Building or installing does not enable cleanup. Automation and AI are disabled by default. CLI, extension and Skill use the same persistent plan/report contract.

## Archive and restore

```bash
vdw lifecycle capabilities
vdw workspace list --json
vdw workspace archive <workspace-id-or-exact-path>... --preview --json
vdw lifecycle plan adjust <plan-id> --exclude-workspace <id> --json
vdw lifecycle plan execute <reviewed-plan-id> --confirm --json
vdw lifecycle operation status <operation-id> --json
vdw workspace restore <archive-id> --preview --json
vdw lifecycle plan execute <restore-plan-id> --confirm --json
```

Plans last 30 minutes. Adjustment supersedes the old plan; re-execution returns the recorded operation. Execute rechecks policy, activity, Git ownership, HEAD, dirty state and material fingerprints.

Any staged, unstaged, conflicted or untracked repository content skips the **whole workspace**. Unknown ignored files are protected; only configured disposable directories (node_modules, target, __pycache__ by default) may be removed with a clean worktree. No force, reset, automatic commit or stash is used. Read-only content/metadata/ownership changes also prevent removal.

Every scoped workspace appears in results, including preflight skips. The summary distinguishes archived, not archived, skipped, failed and unfinished, with reasons and dirty counts. Partial execution is never called safe skipping. AI/cloud state does not change local archive counts. Zero candidates and all-skipped runs are explicit.

Configuration, branches, committed but unpushed work, the Store, original project files and other workspaces remain. Only managed worktrees and verified projections are removed. Archives never expire automatically. Restore uses actual archived editable HEADs and locked read-only commits without fetching; choose `--target /absolute/path` or `--branch <new-restore-branch>` when needed. Existing notes are not overwritten, and original branches are not reset.

## Materials, memory and Git backup

Default location: `~/.vdw/archives/<workspace-id>/<archive-id>`. config/ preserves declarations; workspace-files/ preserves original layout, hidden project files, empty directories and symlink metadata. Scope excludes packages, all actual managed dependency roots, Git databases and configured caches. Symlinks are not traversed.

memory/catalog.md, sources.jsonl and explicit-notes.json form the deterministic catalog, hashes and existing decision/task excerpts. They do not infer testing, release or business acceptance. Optional AI generations under derived/ai/ preserve provenance/checksums without changing base snapshots or project notes. Local sensitive materials and AI input have separate scopes.

Default store_refs pins required commits under refs/vdw/archives and adds verified archives to Store reference accounting. It requires the original Store. Archive corruption makes reference inventory incomplete; Store GC remains plan-only.

`workspace archive <id> --preview --git-backup portable_bundle --measure-bundle --json` measures a full selected-history backup in a separate temporary repository without changing source refs. Bundles add compressed history on top of the retained Store; separate archives duplicate common history. archive.max_bundle_bytes limits each bundle. Incremental chains are not enabled. LFS payloads, submodules and shallow/missing objects are not advertised as complete portable backups.

## Settings, protection, activity and automation

```bash
vdw config show --effective --workspace <id> --json
vdw config set idle-days 30 --preview --json
vdw config set idle-days 60 --workspace <id> --preview --json
vdw config set workspace-auto <id> false --preview --json
vdw config set retain-until 2026-12-01T00:00:00+08:00 --workspace <id> --preview --json
vdw config set retain-until none --workspace <id> --preview --json
vdw workspace protect add <id> --reason "long-lived workspace"
vdw workspace protect remove <id> --preview --json
vdw workspace protect list --json
vdw workspace touch <id> --source terminal
vdw lifecycle scan --preview --json
vdw lifecycle auto enable --preview --json
vdw lifecycle plan execute <enable-plan-id> --confirm --json
vdw lifecycle run --json
vdw lifecycle auto disable
```

Settings changes use preview/confirm. Protection covers every archive entry point; removal starts a fresh grace period. The only dirty policy is skip_workspace. Effective settings show global/workspace sources.

Defaults: 30 idle days, 72 hours grace, 24-hour scan interval. Enable is standing authorization for the global policy over registered current and future workspaces, subject to overrides and protection. macOS enable explicitly installs launchd with the current CLI path; disable unloads/removes it. Other schedulers use `auto enable --no-scheduler --preview` then lifecycle run.

A v1 registry is backed up and receives a fresh observation period; historical last_seen is not reliable usage history. Mutation, touch, observed source changes and IDE edits record activity; status/polling do not. IDE leases last 180 seconds and renew every 60 seconds. The IDE releases its own lease for confirmed archive and refuses to archive its unsaved editors. Unintegrated tools should use touch, leases or protection; Git cannot observe unsaved external buffers.

## Storage migration and recovery

```bash
vdw config set archive-root /absolute/new/location --preview --json
vdw lifecycle plan execute <config-plan-id> --confirm --json
# Optional background worker:
vdw lifecycle plan execute <config-plan-id> --confirm --background --json
vdw lifecycle operation watch <operation-id> --json
vdw lifecycle operation pause <operation-id>
vdw lifecycle operation continue <operation-id> --json
vdw lifecycle operation cancel <operation-id>
vdw lifecycle operation resume <archive-or-restore-operation-id> --preview --json
```

Location confirmation automatically migrates scoped history. New archives use the new root; old copies remain until copy, inventory/hash verification and durable index switching succeed. Only redundant source copies are removed. Progress includes phase, archive/file counts, copied/verified bytes and current file. watch --json emits successive snapshots.

One archive is copied at a time. Partial files are verified before reuse; interrupted copies restart at file boundaries. Explicit pause/cancel remain effective. Running migrations resume on the next VDW invocation, with per-operation worker leases preventing duplicate execution. Completed switches are not rolled back on cancellation. Recovery plans retain checkpoints/attempt history and never discard new dirty content.

## Optional AI and reserved integrations

Edit the lifecycle object from config show, then use `config set lifecycle-file <json-file> --preview --json` and confirm. memory.ai configures enabled, provider (command/http), trigger (after_archive/manual), model, input patterns, timeout and limits. Default input excludes credentials, keys and environment files.

The command adapter uses a fixed argv array, substituting {request_file}/{response_file} without a shell; absent placeholders append --input/--output. Input contains sources with path/hash/text and omitted paths. Output requires summary, evidence_refs with path/hash, and improvements with their own evidence_refs. Invalid references, structure, timeout and oversized output fail independently of the base archive. Manual generation uses `workspace memory summarize <archive-id> --preview --json`.

The HTTP adapter uses curl to POST the same generic contract to endpoint (HTTPS or loopback HTTP). credential_env references an environment variable; credentials are not stored in archives or passed in argv. This is an adapter contract, not automatic vendor API compatibility. Provenance does not guarantee semantic correctness.

lifecycle/providers.rs defines manifest, memory, blob, receipt verification and trigger-source interfaces. KBase/messages are reserved and return unsupported; no connector, listener or business-system mutation is enabled.

Operation exit codes: 0 completed (including reported safe skips), 2 failure, 3 unfinished/paused/cancelled, 4 stale archive plan. Usage/configuration/read errors use 1 (clap parsing may use 2). Execution JSON stays on stdout for nonzero exits; diagnostics use stderr.
