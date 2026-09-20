use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use walkdir::WalkDir;

use crate::config::{env_resolver, parser, validator};
use crate::link::engine;
use crate::lockfile::{LockedDependency, WorkspaceLock, LOCK_FILE_NAME};
use crate::store::editable_locks::{acquire_editable_lock, release_editable_lock};
use crate::store::{manager as store_manager, object_pool};
use crate::types::workspace::{LinkEntry, WorkspaceState};
use crate::worktree::manager as worktree_manager;

pub fn execute(repo_filter: Option<&str>, watch: bool, frozen: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // 0. On Windows, check symlink privilege and guide user if missing
    #[cfg(windows)]
    {
        if !crate::link::platform::try_enable_symlink_privilege() {
            let dev_mode = crate::link::platform::is_developer_mode_enabled();
            if !dev_mode {
                eprintln!(
                    "⚠️  Windows symlink/junction creation requires elevated privileges.\n\
                       Enable Developer Mode (no admin needed) or run as Administrator:\n\
                       \n\
                       Option 1: Settings → Privacy & security → For developers → Developer Mode\n\
                       Option 2: Run this terminal as Administrator\n"
                );
            } else {
                eprintln!(
                    "⚠️  Developer Mode is enabled but symlink privilege could not be acquired.\n\
                       Try running this terminal as Administrator.\n"
                );
            }
        }
    }

    // 1. Load and validate config
    let mut declared_config = parser::load_config(&cwd)?;
    let normalized_legacy_sources = normalize_declarative_sources(&mut declared_config);
    if normalized_legacy_sources && frozen {
        bail!(
            ".vdwrc contains legacy managed worktree paths; run `vdw link` once to migrate the configuration before using --frozen"
        );
    }
    let mut config = declared_config.clone();
    env_resolver::resolve_env_vars(&mut config)?;
    if let Err(errors) = validator::validate_config(&config) {
        for e in &errors {
            eprintln!("  Config error: {}", e);
        }
        return Err(anyhow::anyhow!(
            "Configuration validation failed with {} error(s)",
            errors.len()
        ));
    }
    if normalized_legacy_sources {
        parser::save_config(&cwd, &declared_config)?;
        println!("Normalized legacy managed paths in .vdwrc.");
    }

    // 2. Ensure store is initialized
    let store_path = store_manager::get_store_path()?;
    if !store_path.exists() {
        store_manager::init_store()?;
    }

    let mut workspace_lock = if frozen {
        WorkspaceLock::load_required(&cwd)?
    } else {
        WorkspaceLock::load(&cwd)?.unwrap_or_else(WorkspaceLock::empty)
    };
    preflight_lock(
        &config.dependencies,
        &workspace_lock,
        &store_path,
        repo_filter,
        frozen,
    )?;
    let mut lock_changed = false;
    if !frozen && repo_filter.is_none() {
        let previous_len = workspace_lock.dependencies.len();
        workspace_lock.retain_paths(config.dependencies.iter().map(|dep| dep.path.as_str()));
        lock_changed = workspace_lock.dependencies.len() != previous_len;
    }

    // 3. Process each dependency (optionally filtered by repo URL)
    // Load existing state to preserve other repos' link entries
    let mut state = WorkspaceState::load(&cwd)?;
    let original_state = state.clone();

    let previous_links: Vec<_> = if let Some(filter) = repo_filter {
        state
            .links
            .iter()
            .filter(|entry| entry.dependency_repo == filter)
            .cloned()
            .collect()
    } else {
        state.links.clone()
    };
    let editable_dep_ids = config
        .dependencies
        .iter()
        .filter(|dep| dep.editable)
        .map(|dep| dep.dep_id().to_string())
        .collect::<HashSet<_>>();
    let previous_readonly_links = previous_links
        .iter()
        .filter(|entry| !editable_dep_ids.contains(&entry.dependency_repo))
        .cloned()
        .collect::<Vec<_>>();

    engine::verify_links_for_removal(&previous_readonly_links)?;
    if let Err(error) = engine::remove_links(&previous_readonly_links) {
        let rollback_result = engine::restore_links(&previous_readonly_links);
        return match rollback_result {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "Additionally failed to restore partially removed links: {}",
                rollback_error
            ))),
        };
    }

    // If re-linking specific repo(s), remove their old entries first
    // (the files were already removed by unlink, but stale entries may linger)
    if let Some(filter) = repo_filter {
        state.links.retain(|l| l.dependency_repo != filter);
    } else {
        // Full link: replace all entries (re-linking everything from scratch)
        state.links.clear();
    }
    let mut linked_count = 0;
    let mut editable_count = 0;
    let mut readonly_count = 0;
    let mut created_links = Vec::new();
    let mut editable_migrations = Vec::new();
    let mut editable_locks_to_acquire = Vec::new();

    macro_rules! try_link_step {
        ($operation:expr) => {
            match $operation {
                Ok(value) => value,
                Err(error) => {
                    let rollback_result = rollback_link_transaction(
                        &created_links,
                        &previous_readonly_links,
                        &editable_migrations,
                    );
                    return match rollback_result {
                        Ok(()) => Err(error.into()),
                        Err(rollback_error) => Err(anyhow::Error::from(error).context(format!(
                            "Additionally failed to restore the previous workspace links: {}",
                            rollback_error
                        ))),
                    };
                }
            }
        };
    }

    for dep in &config.dependencies {
        // Skip if repo_filter is specified and doesn't match
        if let Some(filter) = repo_filter {
            if dep.dep_id() != filter {
                continue;
            }
        }

        let dep_id = dep.dep_id().to_string();
        println!("Processing: {} ...", dep_id);

        let (repo_dir, resolved_commit, new_lock_entry) = try_link_step!(resolve_link_commit(
            dep,
            &workspace_lock,
            &store_path,
            frozen,
        ));
        if let Some(entry) = new_lock_entry {
            workspace_lock.upsert(entry);
            lock_changed = true;
        }

        let bare_git_path = repo_dir.join("bare.git");
        let target_dir = cwd.join(&dep.path);

        if dep.editable {
            let branch = dep
                .branch
                .as_deref()
                .or(dep.tag.as_deref())
                .or(dep.commit.as_deref())
                .unwrap_or("main")
                .to_string();
            let dep_links = previous_links
                .iter()
                .filter(|entry| entry.dependency_repo == dep_id)
                .cloned()
                .collect::<Vec<_>>();
            let migration = try_link_step!(prepare_editable_worktree(
                &cwd,
                &store_path,
                &bare_git_path,
                dep,
                &branch,
                &target_dir,
                &dep_links,
            ));
            editable_migrations.push(migration);
            try_link_step!(ensure_editable_contains_locked_commit(
                &target_dir,
                resolved_commit,
            ));
            editable_locks_to_acquire.push((dep_id.clone(), branch, target_dir.clone()));
            editable_count += 1;
            println!("  Editable worktree: {}", target_dir.display());
        } else {
            let checkouts_dir = repo_dir.join("checkouts");
            let checkout_dir = try_link_step!(checkout_from_bare(
                &bare_git_path,
                &checkouts_dir,
                resolved_commit,
            ));
            let entries = try_link_step!(engine::create_dependency_links(
                &checkout_dir,
                &target_dir,
                &dep.scope,
                &dep_id,
            ));

            linked_count += entries.len();
            created_links.extend(entries.iter().cloned());
            state.links.extend(entries);
            readonly_count += 1;
        }
    }

    // 4. Save workspace state
    if let Err(error) = state.save() {
        let rollback_result = rollback_link_transaction(
            &created_links,
            &previous_readonly_links,
            &editable_migrations,
        );
        let _ = original_state.save();
        return match rollback_result {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "Additionally failed to restore the previous workspace links: {}",
                rollback_error
            ))),
        };
    }

    // 5. Publish the lock only after workspace materialization succeeds.
    if lock_changed && !frozen {
        if let Err(error) = workspace_lock.save(&cwd) {
            let rollback_result = rollback_link_transaction(
                &created_links,
                &previous_readonly_links,
                &editable_migrations,
            );
            let _ = original_state.save();
            return match rollback_result {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(error.context(format!(
                    "Additionally failed to restore the previous workspace links: {}",
                    rollback_error
                ))),
            };
        }
    }

    for migration in &editable_migrations {
        if let Err(error) = migration.finalize() {
            tracing::warn!("Failed to trim migration backup: {}", error);
        }
    }
    for (dep_id, branch, target_dir) in editable_locks_to_acquire {
        if let Err(error) = acquire_editable_lock(&dep_id, &branch, &cwd, &target_dir) {
            tracing::warn!(
                "Failed to acquire editable lock for {} branch '{}': {}",
                dep_id,
                branch,
                error
            );
        }
    }
    // 6. Print summary
    println!("\n✅ Workspace linked successfully!");
    println!("   {} file(s) linked", linked_count);
    println!(
        "   {} editable, {} read-only dependencies",
        editable_count, readonly_count
    );
    if frozen {
        println!("   frozen from {}", LOCK_FILE_NAME);
    } else if lock_changed {
        println!("   {} updated", LOCK_FILE_NAME);
    }

    // 7. Show tips on Windows
    #[cfg(windows)]
    {
        println!();
        println!("💡 提示: 使用 `vdw link --watch` 实时监控 HardLink 断链");
        println!("   检测断链: vdw doctor --check-links");
        println!("   修复: vdw link --repo <url>");
        println!("   建议: 关闭 VS Code 原子保存 \"files.atomicSave\": false");
    }

    // 8. Start watch mode if requested
    if watch {
        println!("\n🔍 Starting watch mode for broken link detection...");
        crate::link::watcher::start_watch(&cwd)?;
    }

    Ok(())
}

fn normalize_declarative_sources(config: &mut crate::types::config::VDWConfig) -> bool {
    let mut changed = false;
    for dependency in &mut config.dependencies {
        if !dependency.remote.is_empty() {
            if !dependency.local.is_empty() {
                dependency.local.clear();
                changed = true;
            }
            if !dependency.origin.is_empty() {
                dependency.origin.clear();
                changed = true;
            }
        } else if !dependency.origin.is_empty() {
            dependency.local = std::mem::take(&mut dependency.origin);
            changed = true;
        }
    }
    changed
}

fn preflight_lock(
    dependencies: &[crate::types::config::Dependency],
    workspace_lock: &WorkspaceLock,
    store_path: &Path,
    repo_filter: Option<&str>,
    frozen: bool,
) -> Result<()> {
    if !frozen {
        return Ok(());
    }

    if repo_filter.is_none() && workspace_lock.dependencies.len() != dependencies.len() {
        bail!(
            "{} does not contain the same dependency set as .vdwrc; run `vdw sync --update-lock`",
            LOCK_FILE_NAME
        );
    }

    for dependency in dependencies {
        if repo_filter.is_some_and(|filter| dependency.dep_id() != filter) {
            continue;
        }
        let entry = workspace_lock
            .entry_for_path(&dependency.path)
            .with_context(|| {
                format!(
                    "{} has no entry for '{}'; run `vdw sync --update-lock`",
                    LOCK_FILE_NAME, dependency.path
                )
            })?;
        if !entry.matches(dependency) {
            bail!(
                "{} entry for '{}' no longer matches .vdwrc; run `vdw sync --update-lock`",
                LOCK_FILE_NAME,
                dependency.path
            );
        }
        let oid = git2::Oid::from_str(&entry.resolved_commit).with_context(|| {
            format!(
                "{} contains an invalid commit for '{}'",
                LOCK_FILE_NAME, dependency.path
            )
        })?;
        let bare_git_path =
            object_pool::repo_store_dir(dependency.dep_id(), store_path).join("bare.git");
        let repo = git2::Repository::open_bare(&bare_git_path).with_context(|| {
            format!(
                "Frozen dependency '{}' is unavailable in the Store; run `vdw sync --update-lock` first",
                dependency.path
            )
        })?;
        repo.find_commit(oid).with_context(|| {
            format!(
                "Frozen commit {} for '{}' is unavailable in the Store",
                oid, dependency.path
            )
        })?;
    }
    Ok(())
}

fn resolve_link_commit(
    dependency: &crate::types::config::Dependency,
    workspace_lock: &WorkspaceLock,
    store_path: &Path,
    frozen: bool,
) -> Result<(PathBuf, git2::Oid, Option<LockedDependency>)> {
    if let Some(entry) = workspace_lock.matching_entry(dependency) {
        let repo_dir = object_pool::repo_store_dir(dependency.dep_id(), store_path);
        let bare_git_path = repo_dir.join("bare.git");
        if !bare_git_path.exists() {
            if frozen {
                bail!(
                    "Frozen dependency '{}' is unavailable in the Store",
                    dependency.path
                );
            }
            object_pool::clone_to_store(
                dependency.dep_id(),
                store_path,
                dependency.branch.as_deref(),
                None,
            )?;
        }

        let oid = git2::Oid::from_str(&entry.resolved_commit).with_context(|| {
            format!(
                "{} contains an invalid commit for '{}'",
                LOCK_FILE_NAME, dependency.path
            )
        })?;
        let repo = git2::Repository::open_bare(&bare_git_path)?;
        if repo.find_commit(oid).is_err() {
            if frozen {
                bail!(
                    "Frozen commit {} for '{}' is unavailable in the Store",
                    oid,
                    dependency.path
                );
            }
            object_pool::refresh_dependency_revision(&bare_git_path, dependency)?;
            repo.find_commit(oid).with_context(|| {
                format!(
                    "Locked commit {} for '{}' could not be recovered; inspect the remote history before updating the lock",
                    oid, dependency.path
                )
            })?;
        }
        return Ok((repo_dir, oid, None));
    }

    if frozen {
        if workspace_lock.entry_for_path(&dependency.path).is_some() {
            bail!(
                "{} entry for '{}' no longer matches .vdwrc; run `vdw sync --update-lock`",
                LOCK_FILE_NAME,
                dependency.path
            );
        }
        bail!(
            "{} has no entry for '{}'; run `vdw sync --update-lock`",
            LOCK_FILE_NAME,
            dependency.path
        );
    }

    let repo_dir = object_pool::clone_to_store(
        dependency.dep_id(),
        store_path,
        dependency.branch.as_deref(),
        None,
    )?;
    let resolved_commit =
        object_pool::refresh_dependency_revision(&repo_dir.join("bare.git"), dependency)?;
    let entry = LockedDependency::from_dependency(dependency, resolved_commit.to_string())?;
    Ok((repo_dir, resolved_commit, Some(entry)))
}

fn ensure_editable_contains_locked_commit(
    worktree_path: &Path,
    locked_commit: git2::Oid,
) -> Result<()> {
    let repo = git2::Repository::open(worktree_path).with_context(|| {
        format!(
            "Cannot inspect editable worktree {}",
            worktree_path.display()
        )
    })?;
    let head = repo.head()?.peel_to_commit()?.id();
    if head == locked_commit || repo.graph_descendant_of(head, locked_commit)? {
        return Ok(());
    }
    bail!(
        "Editable worktree {} is at {}, which is not based on locked commit {}. Reconcile the branch before linking.",
        worktree_path.display(),
        head,
        locked_commit
    )
}

#[derive(Debug)]
enum EditableMigration {
    Existing,
    Created {
        bare_repo: PathBuf,
        target: PathBuf,
    },
    Moved {
        bare_repo: PathBuf,
        legacy: PathBuf,
        target: PathBuf,
        staging: Option<PathBuf>,
        overlay_paths: Vec<PathBuf>,
    },
}

impl EditableMigration {
    fn rollback(&self) -> Result<()> {
        match self {
            Self::Existing => Ok(()),
            Self::Created { bare_repo, target } => {
                worktree_manager::remove_worktree_at(bare_repo, target, true)
            }
            Self::Moved {
                bare_repo,
                legacy,
                target,
                staging,
                overlay_paths,
            } => {
                for relative in overlay_paths.iter().rev() {
                    remove_path_if_exists(&target.join(relative))?;
                }
                worktree_manager::move_worktree(bare_repo, target, legacy)?;
                if let Some(staging) = staging {
                    if let Some(parent) = target.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::rename(staging, target).with_context(|| {
                        format!(
                            "Failed to restore legacy projection from {}",
                            staging.display()
                        )
                    })?;
                }
                Ok(())
            }
        }
    }

    fn finalize(&self) -> Result<()> {
        let Self::Moved {
            staging: Some(staging),
            overlay_paths,
            ..
        } = self
        else {
            return Ok(());
        };

        let preserved = overlay_paths.iter().cloned().collect::<HashSet<_>>();
        let mut entries = WalkDir::new(staging)
            .min_depth(1)
            .into_iter()
            .filter_map(|entry| entry.ok())
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.depth()));
        for entry in entries {
            let relative = entry.path().strip_prefix(staging)?;
            if entry.file_type().is_dir() {
                if std::fs::read_dir(entry.path())?.next().is_none() {
                    std::fs::remove_dir(entry.path())?;
                }
            } else if !preserved.contains(relative) {
                std::fs::remove_file(entry.path())?;
            }
        }

        if staging.exists() && std::fs::read_dir(staging)?.next().is_none() {
            std::fs::remove_dir(staging)?;
        } else if staging.exists() {
            println!(
                "  Preserved workspace-only migration backup: {}",
                staging.display()
            );
        }
        Ok(())
    }
}

fn prepare_editable_worktree(
    workspace_root: &Path,
    store_path: &Path,
    bare_repo: &Path,
    dep: &crate::types::config::Dependency,
    branch: &str,
    target: &Path,
    legacy_links: &[LinkEntry],
) -> Result<EditableMigration> {
    if target.join(".git").exists() {
        ensure_worktree_uses_store(target, bare_repo)?;
        return Ok(EditableMigration::Existing);
    }

    let repo_dir = bare_repo
        .parent()
        .context("Store bare repository has no parent directory")?;
    let legacy = find_legacy_worktree(
        store_path,
        repo_dir,
        bare_repo,
        dep,
        branch,
        target,
        legacy_links,
    )?;
    let Some(legacy) = legacy else {
        worktree_manager::create_worktree_at(bare_repo, branch, target)?;
        return Ok(EditableMigration::Created {
            bare_repo: bare_repo.to_path_buf(),
            target: target.to_path_buf(),
        });
    };

    ensure_projection_conflicts_resolved(dep.dep_id(), legacy_links)?;
    let overlay_paths = validate_workspace_overlay(target, &legacy, legacy_links)?;
    let staging = if target.exists() {
        let staging = workspace_root
            .join(".vdw")
            .join("migration-backups")
            .join(format!(
                "{}-{}",
                Utc::now().format("%Y%m%dT%H%M%S%.fZ"),
                std::process::id()
            ))
            .join("workspace")
            .join(&dep.path);
        if let Some(parent) = staging.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(target, &staging).with_context(|| {
            format!(
                "Failed to stage legacy workspace projection {}",
                target.display()
            )
        })?;
        Some(staging)
    } else {
        None
    };

    if let Err(error) = worktree_manager::move_worktree(bare_repo, &legacy, target) {
        if let Some(staging) = &staging {
            let _ = std::fs::rename(staging, target);
        }
        return Err(error.context("Failed to migrate the editable worktree into the workspace"));
    }

    let migration = EditableMigration::Moved {
        bare_repo: bare_repo.to_path_buf(),
        legacy,
        target: target.to_path_buf(),
        staging,
        overlay_paths,
    };
    if let Err(error) = copy_workspace_overlay(&migration) {
        let rollback_error = migration.rollback().err();
        return match rollback_error {
            Some(rollback_error) => Err(error.context(format!(
                "Editable migration failed and rollback was incomplete: {}",
                rollback_error
            ))),
            None => {
                Err(error.context("Editable migration failed; the old projection was restored"))
            }
        };
    }

    Ok(migration)
}

fn ensure_worktree_uses_store(worktree: &Path, bare_repo: &Path) -> Result<()> {
    let repo = git2::Repository::open(worktree)
        .with_context(|| format!("Invalid editable Git worktree: {}", worktree.display()))?;
    let actual = std::fs::canonicalize(repo.path())
        .with_context(|| format!("Cannot resolve Git directory for {}", worktree.display()))?;
    let expected = std::fs::canonicalize(bare_repo)
        .with_context(|| format!("Cannot resolve Store repository {}", bare_repo.display()))?;
    if actual != expected && !actual.starts_with(expected.join("worktrees")) {
        bail!(
            "Refusing to adopt editable directory {}: its Git directory is {}, expected a worktree of {}",
            worktree.display(),
            actual.display(),
            expected.display()
        );
    }
    Ok(())
}

fn find_legacy_worktree(
    store_path: &Path,
    repo_dir: &Path,
    bare_repo: &Path,
    dep: &crate::types::config::Dependency,
    branch: &str,
    target: &Path,
    legacy_links: &[LinkEntry],
) -> Result<Option<PathBuf>> {
    if !dep.local.is_empty() {
        let local = PathBuf::from(&dep.local);
        if local.starts_with(store_path) && local != target && local.join(".git").exists() {
            return Ok(Some(local));
        }
    }

    for entry in legacy_links {
        let discover_from = entry.source.parent().unwrap_or(&entry.source);
        if let Ok(repo) = git2::Repository::discover(discover_from) {
            if let Some(workdir) = repo.workdir() {
                if workdir != target && workdir.starts_with(store_path) {
                    return Ok(Some(workdir.to_path_buf()));
                }
            }
        }
    }

    let expected_name = branch.replace('/', "-");
    if bare_repo.exists() {
        for worktree in worktree_manager::list_worktrees(bare_repo)? {
            if worktree.path != target
                && (worktree.branch == branch || worktree.branch == expected_name)
            {
                return Ok(Some(worktree.path));
            }
        }
    }

    for candidate in [
        repo_dir.join("worktrees").join(&expected_name),
        repo_dir.join("worktrees").join(branch),
    ] {
        if candidate != target && candidate.join(".git").exists() {
            return Ok(Some(candidate));
        }
    }
    Ok(None)
}

fn ensure_projection_conflicts_resolved(dep_id: &str, entries: &[LinkEntry]) -> Result<()> {
    let audit = engine::audit_links(entries);
    if audit.conflicts.is_empty() {
        return Ok(());
    }
    let preview = audit
        .conflicts
        .iter()
        .take(10)
        .map(|conflict| {
            format!(
                "{}: {}",
                conflict.target.display(),
                conflict.reason.description()
            )
        })
        .collect::<Vec<_>>()
        .join("\n  - ");
    bail!(
        "Cannot migrate editable dependency '{}' while {} legacy projection conflict(s) remain:\n  - {}\n\
         Run `vdw conflicts list --repo {}` to inspect the Store and workspace files, then use \
         `vdw conflicts resolve --repo {} --accept store|workspace` (optionally with --path).",
        dep_id,
        audit.conflicts.len(),
        preview,
        dep_id,
        dep_id
    )
}

fn validate_workspace_overlay(
    target: &Path,
    legacy_worktree: &Path,
    legacy_links: &[LinkEntry],
) -> Result<Vec<PathBuf>> {
    if !target.exists() {
        return Ok(Vec::new());
    }
    if !target.is_dir() {
        bail!(
            "Editable workspace target is not a directory: {}",
            target.display()
        );
    }

    let tracked = legacy_links
        .iter()
        .filter_map(|entry| entry.target.strip_prefix(target).ok())
        .map(Path::to_path_buf)
        .collect::<HashSet<_>>();
    let mut overlay = Vec::new();
    for entry in WalkDir::new(target)
        .min_depth(1)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_dir() {
            continue;
        }
        let relative = entry.path().strip_prefix(target)?.to_path_buf();
        if tracked.contains(&relative) {
            continue;
        }
        let store_file = legacy_worktree.join(&relative);
        if !store_file.exists() {
            if !entry.file_type().is_file() {
                bail!(
                    "Cannot migrate unsupported workspace-only file type: {}",
                    entry.path().display()
                );
            }
            overlay.push(relative);
            continue;
        }
        if entry.file_type().is_file()
            && engine::regular_files_are_equivalent(&store_file, entry.path()).unwrap_or(false)
        {
            continue;
        }
        bail!(
            "Workspace-only path conflicts with the full editable worktree: {}. Move or reconcile this file before retrying `vdw link`.",
            entry.path().display()
        );
    }
    Ok(overlay)
}

fn copy_workspace_overlay(migration: &EditableMigration) -> Result<()> {
    let EditableMigration::Moved {
        target,
        staging: Some(staging),
        overlay_paths,
        ..
    } = migration
    else {
        return Ok(());
    };
    for relative in overlay_paths {
        let source = staging.join(relative);
        let destination = target.join(relative);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &destination).with_context(|| {
            format!(
                "Failed to preserve workspace-only file {}",
                source.display()
            )
        })?;
    }
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn rollback_link_transaction(
    created_links: &[LinkEntry],
    previous_links: &[LinkEntry],
    migrations: &[EditableMigration],
) -> Result<()> {
    let mut errors = Vec::new();
    if let Err(error) = engine::verify_links_for_removal(created_links)
        .and_then(|_| engine::remove_links(created_links))
    {
        errors.push(format!("new read-only links: {}", error));
    }
    for migration in migrations.iter().rev() {
        if let Err(error) = migration.rollback() {
            errors.push(format!("editable migration: {}", error));
        }
    }
    if let Err(error) = engine::restore_links(previous_links) {
        errors.push(format!("previous read-only links: {}", error));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        bail!(errors.join("; "))
    }
}

/// Checkout files from a bare repo into an immutable, commit-keyed directory.
///
/// Resolving the declared revision before choosing the directory prevents two
/// workspaces that use different revisions of the same repository from sharing
/// stale files.
fn checkout_from_bare(
    bare_git_path: &Path,
    checkouts_dir: &Path,
    commit_id: git2::Oid,
) -> Result<PathBuf> {
    let repo = git2::Repository::open_bare(bare_git_path)
        .with_context(|| format!("Cannot open bare repo at {}", bare_git_path.display()))?;
    let commit = repo.find_commit(commit_id).with_context(|| {
        format!(
            "Cannot find locked commit {} in {}",
            commit_id,
            bare_git_path.display()
        )
    })?;
    let checkout_dir = checkouts_dir.join(commit_id.to_string());

    if checkout_dir.exists() && std::fs::read_dir(&checkout_dir)?.next().is_some() {
        make_dir_readonly_recursive(&checkout_dir)?;
        return Ok(checkout_dir);
    }

    std::fs::create_dir_all(checkouts_dir)?;
    if checkout_dir.exists() {
        std::fs::remove_dir(&checkout_dir).with_context(|| {
            format!(
                "Cannot replace incomplete read-only checkout {}",
                checkout_dir.display()
            )
        })?;
    }

    let staging_dir = checkouts_dir.join(format!(
        ".tmp-{}-{}-{}",
        commit_id,
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    std::fs::create_dir(&staging_dir)?;

    let tree = commit.tree()?;
    if let Err(error) = write_tree_to_dir(&repo, &tree, &staging_dir) {
        let _ = remove_checkout_staging(&staging_dir);
        return Err(error.context(format!("Failed to materialize locked commit {}", commit_id)));
    }

    match std::fs::rename(&staging_dir, &checkout_dir) {
        Ok(()) => Ok(checkout_dir),
        Err(_) if checkout_dir.exists() && std::fs::read_dir(&checkout_dir)?.next().is_some() => {
            remove_checkout_staging(&staging_dir)?;
            make_dir_readonly_recursive(&checkout_dir)?;
            Ok(checkout_dir)
        }
        Err(error) => {
            let _ = remove_checkout_staging(&staging_dir);
            Err(error).with_context(|| {
                format!(
                    "Cannot publish read-only checkout for locked commit {}",
                    commit_id
                )
            })
        }
    }
}

fn remove_checkout_staging(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    for entry in WalkDir::new(path)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if entry.file_type().is_file() {
            let mut permissions = std::fs::metadata(entry.path())?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            std::fs::set_permissions(entry.path(), permissions)?;
        }
    }
    std::fs::remove_dir_all(path)?;
    Ok(())
}

/// Recursively write a git tree to a directory.
/// Files are written as read-only (0o444) to enforce the read-only contract
/// at the filesystem level, preventing accidental edits that would affect
/// all workspaces sharing the same checkout via hardlinks.
fn write_tree_to_dir(repo: &git2::Repository, tree: &git2::Tree, dir: &Path) -> Result<()> {
    for entry in tree.iter() {
        let name = entry.name().unwrap_or("unknown");
        let entry_path = dir.join(name);

        match entry.kind() {
            Some(git2::ObjectType::Blob) => {
                let blob = repo.find_blob(entry.id())?;
                if let Some(parent) = entry_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&entry_path, blob.content())?;
                // Set file read-only to prevent accidental edits in shared checkout
                set_readonly(&entry_path)?;
            }
            Some(git2::ObjectType::Tree) => {
                std::fs::create_dir_all(&entry_path)?;
                let subtree = repo.find_tree(entry.id())?;
                write_tree_to_dir(repo, &subtree, &entry_path)?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Set a file to read-only at the filesystem level.
/// On Unix: removes all write bits (mode 0o444).
/// On Windows: sets the read-only attribute.
fn set_readonly(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o444);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    {
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

/// Recursively set all files in a directory to read-only.
/// Used to enforce read-only mode for existing checkouts from older VDW versions.
fn make_dir_readonly_recursive(dir: &Path) -> Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if entry.file_type().is_file() {
            // Skip if already read-only
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let metadata = match std::fs::metadata(entry.path()) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if metadata.permissions().mode() & 0o222 == 0 {
                    continue; // already read-only
                }
            }
            if let Err(e) = set_readonly(entry.path()) {
                tracing::warn!("Failed to set read-only on {:?}: {}", entry.path(), e);
            }
        }
    }
    Ok(())
}

pub fn unlink(repo_filter: Option<&str>) -> Result<()> {
    unlink_internal(repo_filter, false)
}

pub(crate) fn unlink_force(repo_filter: Option<&str>) -> Result<()> {
    unlink_internal(repo_filter, true)
}

fn unlink_internal(repo_filter: Option<&str>, force_editable: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let state = WorkspaceState::load(&cwd)?;
    let mut config = parser::load_config(&cwd)?;
    env_resolver::resolve_env_vars(&mut config)?;
    if let Err(errors) = validator::validate_config(&config) {
        bail!(
            "Refusing to unlink an invalid workspace configuration: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let store_path = store_manager::get_store_path()?;

    let links_to_remove: Vec<_> = if let Some(repo) = repo_filter {
        state
            .links
            .iter()
            .filter(|l| l.dependency_repo == repo)
            .cloned()
            .collect()
    } else {
        state.links.clone()
    };

    let mut worktrees_to_remove = Vec::new();
    for dep in config
        .dependencies
        .iter()
        .filter(|dep| dep.editable && repo_filter.map(|repo| dep.dep_id() == repo).unwrap_or(true))
    {
        let Some(worktree) = managed_worktree_for_unlink(&cwd, &store_path, dep, &state)? else {
            continue;
        };
        let bare_repo = bare_repo_for_managed_worktree(&worktree, &store_path)?;
        if !force_editable {
            let status = crate::worktree::check_dirty(&worktree).with_context(|| {
                format!("Failed to inspect editable worktree {}", worktree.display())
            })?;
            if status.is_dirty() {
                bail!(
                    "Refusing to unlink editable worktree with uncommitted changes: {} \
                     ({} modified, {} added, {} deleted, {} untracked). \
                     Commit or stash the changes first.",
                    worktree.display(),
                    status.modified,
                    status.added,
                    status.deleted,
                    status.untracked
                );
            }
        }
        worktrees_to_remove.push((
            dep.dep_id().to_string(),
            dep.branch.as_deref().unwrap_or("main").to_string(),
            worktree,
            bare_repo,
        ));
    }

    if links_to_remove.is_empty() && worktrees_to_remove.is_empty() {
        println!("No managed links or editable worktrees to remove.");
        return Ok(());
    }

    // Verify ownership before deleting anything. Atomic-save editors can turn a
    // hardlink into an independent file that must be preserved.
    engine::verify_links_for_removal(&links_to_remove)?;

    if !links_to_remove.is_empty() {
        println!("Removing {} link(s)...", links_to_remove.len());
    }
    if let Err(error) = engine::remove_links(&links_to_remove) {
        let rollback_result = engine::restore_links(&links_to_remove);
        return match rollback_result {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "Additionally failed to restore partially removed links: {}",
                rollback_error
            ))),
        };
    }

    // Clean up empty directories
    for link in &links_to_remove {
        if let Some(parent) = link.target.parent() {
            let _ = engine::cleanup_empty_dirs(parent, &cwd);
        }
    }

    let mut new_state = state.clone();
    if let Some(repo) = repo_filter {
        // Remove only the matching repo's links
        new_state.links.retain(|l| l.dependency_repo != repo);
    } else {
        // Full unlink: clear all state
        new_state = WorkspaceState::new(cwd.clone());
    }
    if let Err(error) = new_state.save() {
        let rollback_result = engine::restore_links(&links_to_remove);
        let _ = state.save();
        return match rollback_result {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "Additionally failed to restore removed links: {}",
                rollback_error
            ))),
        };
    }

    for (dep_id, branch, worktree, bare_repo) in &worktrees_to_remove {
        if let Err(error) =
            worktree_manager::remove_worktree_at(bare_repo, worktree, force_editable)
        {
            bail!(
                "Workspace metadata was unlinked, but editable worktree {} could not be removed: {}",
                worktree.display(),
                error
            );
        }
        if let Err(e) = release_editable_lock(dep_id, branch, &cwd) {
            tracing::warn!(
                "Failed to release editable lock for {} branch '{}': {}",
                dep_id,
                branch,
                e
            );
        }
    }

    println!(
        "✅ {} link(s) and {} editable worktree(s) removed.",
        links_to_remove.len(),
        worktrees_to_remove.len()
    );
    Ok(())
}

/// Refresh read-only projections and migrate legacy editable projections.
/// Native editable worktrees stay in place, including dirty and untracked files.
pub fn relink(_allow_dirty: bool) -> Result<()> {
    println!("Relinking workspace without replacing native editable worktrees...");
    execute(None, false, false)?;
    println!("✅ Workspace relink completed.");
    Ok(())
}

fn managed_worktree_for_unlink(
    workspace_root: &Path,
    store_path: &Path,
    dep: &crate::types::config::Dependency,
    state: &WorkspaceState,
) -> Result<Option<PathBuf>> {
    let target = workspace_root.join(&dep.path);
    if target.join(".git").exists() {
        return Ok(Some(target));
    }

    let matching_links: Vec<_> = state
        .links
        .iter()
        .filter(|entry| entry.dependency_repo == dep.dep_id())
        .collect();

    for entry in &matching_links {
        let discover_from = entry.source.parent().unwrap_or(&entry.source);
        if let Ok(repo) = git2::Repository::discover(discover_from) {
            if let Some(workdir) = repo.workdir() {
                if workdir.starts_with(store_path) {
                    return Ok(Some(workdir.to_path_buf()));
                }
            }
        }
    }

    if !dep.local.is_empty() {
        let local = PathBuf::from(&dep.local);
        if local.starts_with(store_path) && local.join(".git").exists() {
            return Ok(Some(local));
        }
    }

    if !matching_links.is_empty() {
        bail!(
            "Cannot resolve the managed worktree for editable dependency '{}'",
            dep.dep_id()
        );
    }

    Ok(None)
}

fn bare_repo_for_managed_worktree(worktree: &Path, store_path: &Path) -> Result<PathBuf> {
    let repo = git2::Repository::open(worktree)
        .with_context(|| format!("Invalid managed worktree {}", worktree.display()))?;
    let git_dir = std::fs::canonicalize(repo.path())?;
    let canonical_store = std::fs::canonicalize(store_path)?;
    if !git_dir.starts_with(&canonical_store) {
        bail!(
            "Refusing to remove non-VDW Git worktree: {}",
            worktree.display()
        );
    }
    let worktrees_dir = git_dir.parent().context("Missing worktrees directory")?;
    if worktrees_dir.file_name().and_then(|name| name.to_str()) != Some("worktrees") {
        bail!(
            "Cannot resolve Store repository for worktree {}",
            worktree.display()
        );
    }
    let bare_repo = worktrees_dir
        .parent()
        .context("Missing bare repository directory")?
        .to_path_buf();
    git2::Repository::open_bare(&bare_repo).with_context(|| {
        format!(
            "Resolved Store path is not a bare repository: {}",
            bare_repo.display()
        )
    })?;
    Ok(bare_repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::{Dependency, Scope};
    use std::fs;

    fn editable_dependency(remote: &str, local: &str, origin: &str) -> Dependency {
        Dependency {
            remote: remote.to_string(),
            origin: origin.to_string(),
            local: local.to_string(),
            repo: String::new(),
            path: "packages/repo".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["**/*".to_string()],
                exclude: vec![],
            },
            editable: true,
        }
    }

    #[test]
    fn normalize_remote_dependency_removes_legacy_runtime_paths() {
        let mut config = crate::types::config::VDWConfig {
            workspace: crate::types::config::WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![editable_dependency(
                "https://github.com/example/repo.git",
                "/store/objects/repo/worktrees/main",
                "/projects/repo",
            )],
        };

        assert!(normalize_declarative_sources(&mut config));
        assert!(config.dependencies[0].local.is_empty());
        assert!(config.dependencies[0].origin.is_empty());
    }

    #[test]
    fn normalize_local_dependency_restores_declarative_source() {
        let mut config = crate::types::config::VDWConfig {
            workspace: crate::types::config::WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![editable_dependency(
                "",
                "/store/objects/repo/worktrees/main",
                "/projects/repo",
            )],
        };

        assert!(normalize_declarative_sources(&mut config));
        assert_eq!(config.dependencies[0].local, "/projects/repo");
        assert!(config.dependencies[0].origin.is_empty());
    }

    fn create_clean_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let repo = git2::Repository::init(dir.path()).expect("Failed to initialize repository");
        fs::write(dir.path().join("tracked.txt"), "clean").expect("Failed to write tracked file");
        let mut index = repo.index().expect("Failed to open index");
        index
            .add_path(Path::new("tracked.txt"))
            .expect("Failed to add tracked file");
        index.write().expect("Failed to persist index");
        let tree_id = index.write_tree().expect("Failed to write tree");
        let tree = repo.find_tree(tree_id).expect("Failed to find tree");
        let signature = git2::Signature::now("VDW Test", "vdw@example.com")
            .expect("Failed to create signature");
        repo.commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .expect("Failed to create commit");
        drop(tree);
        drop(repo);
        dir
    }

    #[test]
    fn test_legacy_editable_projection_migrates_to_native_worktree() {
        let source_repo = create_clean_git_repo();
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let repo_dir = store.path().join("objects/repo");
        let bare_repo = repo_dir.join("bare.git");
        fs::create_dir_all(&repo_dir).expect("repo dir");
        git2::build::RepoBuilder::new()
            .bare(true)
            .clone(source_repo.path().to_str().unwrap(), &bare_repo)
            .expect("bare clone");
        let bare = git2::Repository::open_bare(&bare_repo).expect("bare");
        let branch = crate::worktree::git_ops::list_branches(&bare)
            .expect("branches")
            .into_iter()
            .find(|branch| branch == "main" || branch == "master")
            .expect("default branch");
        let legacy = repo_dir.join("worktrees").join(&branch);
        worktree_manager::create_worktree_at(&bare_repo, &branch, &legacy)
            .expect("legacy worktree");

        let target = workspace.path().join("packages/repo");
        let scope = Scope {
            include: vec!["**/*".to_string()],
            exclude: vec![],
        };
        let links = engine::create_dependency_links(&legacy, &target, &scope, "repo")
            .expect("legacy projection");
        fs::write(legacy.join("tracked.txt"), "dirty but preserved").expect("dirty edit");
        fs::write(legacy.join("store-only.txt"), "store only").expect("store untracked");
        fs::write(target.join("workspace-only.txt"), "workspace only")
            .expect("workspace untracked");
        let dep = editable_dependency("repo", &legacy.to_string_lossy(), "");

        let migration = prepare_editable_worktree(
            workspace.path(),
            store.path(),
            &bare_repo,
            &dep,
            &branch,
            &target,
            &links,
        )
        .expect("migration");

        assert!(!legacy.exists());
        assert!(target.join(".git").exists());
        assert_eq!(
            fs::read_to_string(target.join("tracked.txt")).unwrap(),
            "dirty but preserved"
        );
        assert_eq!(
            fs::read_to_string(target.join("store-only.txt")).unwrap(),
            "store only"
        );
        assert_eq!(
            fs::read_to_string(target.join("workspace-only.txt")).unwrap(),
            "workspace only"
        );
        ensure_worktree_uses_store(&target, &bare_repo).expect("store ownership");
        migration.finalize().expect("finalize migration");
    }

    /// Test that unlink correctly removes links created by the link engine
    #[test]
    fn test_unlink_removes_created_links() {
        // Set up source directory with files
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

        // Create source files
        let src = source_dir.path().join("src");
        fs::create_dir_all(&src).expect("Failed to create src dir");
        fs::write(src.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
        fs::write(src.join("lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");

        let target_dir = workspace_dir.path().join("libs/repo");

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        // Create links
        let entries = engine::create_dependency_links(
            source_dir.path(),
            &target_dir,
            &scope,
            "https://github.com/test/unlink-test.git",
        )
        .expect("Failed to create links");

        assert_eq!(entries.len(), 2, "Should have 2 linked files");
        assert!(
            target_dir.join("src/main.rs").exists(),
            "main.rs should exist after linking"
        );
        assert!(
            target_dir.join("src/lib.rs").exists(),
            "lib.rs should exist after linking"
        );

        // Save workspace state
        let mut state = WorkspaceState::new(workspace_dir.path().to_path_buf());
        state.links = entries;
        state.save().expect("Failed to save state");

        // Simulate unlink: load state, remove links, clean up
        let loaded_state =
            WorkspaceState::load(workspace_dir.path()).expect("Failed to load state");
        assert_eq!(loaded_state.links.len(), 2, "Should have 2 links in state");

        engine::remove_links(&loaded_state.links).expect("Failed to remove links");

        // Clean up empty dirs
        for dep_path in loaded_state.links.iter().map(|l| &l.target) {
            if let Some(parent) = dep_path.parent() {
                let _ = engine::cleanup_empty_dirs(parent, workspace_dir.path());
            }
        }

        // Verify files are removed
        assert!(
            !target_dir.join("src/main.rs").exists(),
            "main.rs should be removed after unlink"
        );
        assert!(
            !target_dir.join("src/lib.rs").exists(),
            "lib.rs should be removed after unlink"
        );

        // Clear state (as unlink does)
        let new_state = WorkspaceState::new(workspace_dir.path().to_path_buf());
        new_state.save().expect("Failed to clear state");

        // Verify state is cleared
        let final_state =
            WorkspaceState::load(workspace_dir.path()).expect("Failed to load final state");
        assert!(
            final_state.links.is_empty(),
            "Links should be empty after unlink"
        );
    }

    /// Test unlink with empty state (no links)
    #[test]
    fn test_unlink_empty_state() {
        let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

        let state = WorkspaceState::load(workspace_dir.path()).expect("Failed to load state");
        assert!(state.links.is_empty(), "New workspace should have no links");
    }

    /// Test unlink with mixed link types
    #[test]
    fn test_unlink_mixed_link_types() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

        // Create multiple source files
        fs::create_dir_all(source_dir.path().join("src")).expect("Failed to create src dir");
        fs::write(source_dir.path().join("src/main.rs"), "fn main() {}")
            .expect("Failed to write main.rs");
        fs::write(source_dir.path().join("README.md"), "# Test")
            .expect("Failed to write README.md");

        let target_dir = workspace_dir.path().join("libs/repo");

        let scope = Scope {
            include: vec!["**/*".to_string()],
            exclude: vec![],
        };

        // Create links
        let entries = engine::create_dependency_links(
            source_dir.path(),
            &target_dir,
            &scope,
            "https://github.com/test/mixed-test.git",
        )
        .expect("Failed to create links");

        assert!(entries.len() >= 2, "Should have at least 2 linked files");

        // Save state
        let mut state = WorkspaceState::new(workspace_dir.path().to_path_buf());
        state.links = entries;
        state.save().expect("Failed to save state");

        // Remove all links
        let loaded_state =
            WorkspaceState::load(workspace_dir.path()).expect("Failed to load state");
        engine::remove_links(&loaded_state.links).expect("Failed to remove links");

        // Verify all targets are removed
        for entry in &loaded_state.links {
            assert!(
                !entry.target.exists(),
                "Target {:?} should be removed",
                entry.target
            );
        }
    }

    fn create_bare_repo_with_revisions() -> (tempfile::TempDir, git2::Oid, git2::Oid) {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let bare_path = dir.path().join("bare.git");
        let repo = git2::Repository::init_bare(&bare_path).expect("Failed to initialize bare repo");
        let signature = git2::Signature::now("VDW Test", "vdw@example.com")
            .expect("Failed to create signature");

        let main_blob = repo.blob(b"main").expect("Failed to create main blob");
        let mut main_builder = repo
            .treebuilder(None)
            .expect("Failed to create tree builder");
        main_builder
            .insert("tracked.txt", main_blob, 0o100644)
            .expect("Failed to insert main blob");
        let main_tree_id = main_builder.write().expect("Failed to write main tree");
        let main_tree = repo
            .find_tree(main_tree_id)
            .expect("Failed to find main tree");
        let main_id = repo
            .commit(
                Some("refs/heads/main"),
                &signature,
                &signature,
                "main",
                &main_tree,
                &[],
            )
            .expect("Failed to create main commit");
        let main_commit = repo
            .find_commit(main_id)
            .expect("Failed to find main commit");

        let feature_blob = repo
            .blob(b"feature")
            .expect("Failed to create feature blob");
        let mut feature_builder = repo
            .treebuilder(None)
            .expect("Failed to create feature tree builder");
        feature_builder
            .insert("tracked.txt", feature_blob, 0o100644)
            .expect("Failed to insert feature blob");
        let feature_tree_id = feature_builder
            .write()
            .expect("Failed to write feature tree");
        let feature_tree = repo
            .find_tree(feature_tree_id)
            .expect("Failed to find feature tree");
        let feature_id = repo
            .commit(
                Some("refs/heads/feature"),
                &signature,
                &signature,
                "feature",
                &feature_tree,
                &[&main_commit],
            )
            .expect("Failed to create feature commit");
        repo.reference("refs/tags/v1", main_id, false, "test tag")
            .expect("Failed to create tag");

        drop(feature_tree);
        drop(feature_builder);
        drop(main_commit);
        drop(main_tree);
        drop(main_builder);
        drop(repo);
        (dir, main_id, feature_id)
    }

    /// Different declared revisions of one repository must never reuse a checkout.
    #[test]
    fn test_readonly_checkouts_are_isolated_by_resolved_commit() {
        let (dir, main_id, feature_id) = create_bare_repo_with_revisions();
        let bare_git_path = dir.path().join("bare.git");
        let checkouts_dir = dir.path().join("checkouts");

        let main_checkout = checkout_from_bare(&bare_git_path, &checkouts_dir, main_id)
            .expect("Failed to checkout main");
        let feature_checkout = checkout_from_bare(&bare_git_path, &checkouts_dir, feature_id)
            .expect("Failed to checkout feature");

        assert_eq!(main_checkout, checkouts_dir.join(main_id.to_string()));
        assert_eq!(feature_checkout, checkouts_dir.join(feature_id.to_string()));
        assert_ne!(main_checkout, feature_checkout);
        assert_eq!(
            fs::read_to_string(main_checkout.join("tracked.txt")).unwrap(),
            "main"
        );
        assert_eq!(
            fs::read_to_string(feature_checkout.join("tracked.txt")).unwrap(),
            "feature"
        );

        let reused_main = checkout_from_bare(&bare_git_path, &checkouts_dir, main_id)
            .expect("Failed to reuse tagged main commit");
        assert_eq!(reused_main, main_checkout);
    }

    #[test]
    fn test_readonly_checkout_rejects_unknown_revision() {
        let (dir, _, _) = create_bare_repo_with_revisions();
        let missing = git2::Oid::from_str(&"f".repeat(40)).unwrap();
        let error = checkout_from_bare(
            &dir.path().join("bare.git"),
            &dir.path().join("checkouts"),
            missing,
        )
        .expect_err("Unknown commit should fail");

        assert!(error.to_string().contains("Cannot find locked commit"));
    }
}
