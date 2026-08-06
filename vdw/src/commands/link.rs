use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::config::{env_resolver, parser, validator};
use crate::link::engine;
use crate::store::editable_locks::{acquire_editable_lock, release_editable_lock};
use crate::store::{manager as store_manager, object_pool};
use crate::types::workspace::WorkspaceState;
use crate::worktree::manager as worktree_manager;

/// Record the VDW-managed worktree path on the dependency.
///
/// This must run for both newly-created and reused worktrees. In particular,
/// `unlink` clears `dep.local`, so a subsequent relink that discovers the
/// conventional worktree path must restore it before saving `.vdwrc`.
fn record_managed_worktree(dep: &mut crate::types::config::Dependency, worktree_path: &Path) {
    // For local-only dependencies, preserve the original checkout before
    // replacing `local` with the VDW-managed worktree path.
    if dep.remote.is_empty() && dep.origin.is_empty() && !dep.local.is_empty() {
        dep.origin = dep.local.clone();
    }
    dep.local = worktree_path.to_string_lossy().to_string();
}

pub fn execute(repo_filter: Option<&str>, watch: bool) -> Result<()> {
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
    let mut config = parser::load_config(&cwd)?;
    let original_config = config.clone();
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

    // 2. Ensure store is initialized
    let store_path = store_manager::get_store_path()?;
    if !store_path.exists() {
        store_manager::init_store()?;
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
    engine::verify_links_for_removal(&previous_links)?;
    if let Err(error) = engine::remove_links(&previous_links) {
        let rollback_result = engine::restore_links(&previous_links);
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

    macro_rules! try_link_step {
        ($operation:expr) => {
            match $operation {
                Ok(value) => value,
                Err(error) => {
                    let rollback_result = rollback_link_files(&created_links, &previous_links);
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

    for dep in &mut config.dependencies {
        // Skip if repo_filter is specified and doesn't match
        if let Some(filter) = repo_filter {
            if dep.dep_id() != filter {
                continue;
            }
        }

        let dep_id = dep.dep_id().to_string();
        println!("Processing: {} ...", dep_id);

        // a. Ensure repo is in store
        let repo_dir = try_link_step!(object_pool::clone_to_store(
            &dep_id,
            &store_path,
            dep.branch.as_deref(),
            None, // no reference for MVP
        ));

        let bare_git_path = repo_dir.join("bare.git");
        let target_dir = cwd.join(&dep.path);

        // b. Determine source directory
        let source_dir = if dep.editable {
            let branch = dep
                .branch
                .as_deref()
                .or(dep.tag.as_deref())
                .or(dep.commit.as_deref())
                .unwrap_or("main")
                .to_string();

            // Check if dep.local is already a VDW-managed worktree path (inside the store).
            // Only reuse it if it's managed — original source paths must be replaced
            // with worktrees so dep.local consistently points to a managed location.
            let existing_worktree = if !dep.local.is_empty() {
                let local_path = Path::new(&dep.local);
                let is_managed = local_path.starts_with(&store_path);
                if is_managed && local_path.exists() && local_path.join(".git").exists() {
                    Some(local_path.to_path_buf())
                } else {
                    None
                }
            } else {
                None
            };

            // If dep.local is empty, also scan the store for existing worktrees
            // at the conventional path {repo_dir}/worktrees/{branch}/.
            // This handles re-link after unlink (where dep.local was cleared).
            let existing_worktree = if existing_worktree.is_none() {
                let conventional_wt = repo_dir.join("worktrees").join(&branch);
                if conventional_wt.exists() && conventional_wt.join(".git").exists() {
                    println!(
                        "  Found existing worktree in store: {}",
                        conventional_wt.display()
                    );
                    Some(conventional_wt)
                } else {
                    None
                }
            } else {
                existing_worktree
            };

            if let Some(wt_path) = existing_worktree {
                println!("  Using existing worktree: {}", wt_path.display());
                record_managed_worktree(dep, &wt_path);
                if let Err(e) = acquire_editable_lock(&dep_id, &branch, &cwd, &wt_path) {
                    tracing::warn!(
                        "Failed to acquire editable lock for {} branch '{}': {}",
                        dep_id,
                        branch,
                        e
                    );
                }
                editable_count += 1;
                wt_path
            } else {
                // Create worktree for editable deps
                let worktree_base = repo_dir.join("worktrees");
                let wt_path = try_link_step!(worktree_manager::create_worktree(
                    &bare_git_path,
                    &branch,
                    &worktree_base,
                ));

                record_managed_worktree(dep, &wt_path);
                if let Err(e) = acquire_editable_lock(&dep_id, &branch, &cwd, &wt_path) {
                    tracing::warn!(
                        "Failed to acquire editable lock for {} branch '{}': {}",
                        dep_id,
                        branch,
                        e
                    );
                }

                editable_count += 1;
                wt_path
            }
        } else {
            // For read-only deps, checkout from bare repo to a checkout dir inside the store
            let checkout_dir = repo_dir.join("checkout");
            try_link_step!(checkout_from_bare(&bare_git_path, &checkout_dir, dep));
            readonly_count += 1;
            checkout_dir
        };

        // c. Create links from source to workspace
        if source_dir.exists() {
            let entries = try_link_step!(engine::create_dependency_links(
                &source_dir,
                &target_dir,
                &dep.scope,
                &dep_id,
            ));

            linked_count += entries.len();
            created_links.extend(entries.iter().cloned());
            state.links.extend(entries);
        } else {
            println!("  ⚠️  Source not available (network fetch may have failed)");
        }
    }

    // 4. Save updated config with local paths filled in
    if let Err(error) = parser::save_config(&cwd, &config) {
        let rollback_result = rollback_link_files(&created_links, &previous_links);
        let _ = parser::save_config(&cwd, &original_config);
        return match rollback_result {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "Additionally failed to restore the previous workspace links: {}",
                rollback_error
            ))),
        };
    }

    // 5. Save workspace state
    if let Err(error) = state.save() {
        let rollback_result = rollback_link_files(&created_links, &previous_links);
        let _ = parser::save_config(&cwd, &original_config);
        let _ = original_state.save();
        return match rollback_result {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(error.context(format!(
                "Additionally failed to restore the previous workspace links: {}",
                rollback_error
            ))),
        };
    }

    // 6. Print summary
    println!("\n✅ Workspace linked successfully!");
    println!("   {} file(s) linked", linked_count);
    println!(
        "   {} editable, {} read-only dependencies",
        editable_count, readonly_count
    );

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

fn rollback_link_files(
    created_links: &[crate::types::workspace::LinkEntry],
    previous_links: &[crate::types::workspace::LinkEntry],
) -> Result<()> {
    engine::verify_links_for_removal(created_links)?;
    engine::remove_links(created_links)?;
    engine::restore_links(previous_links)?;
    Ok(())
}

/// Checkout files from a bare repo to a directory
/// For read-only mode: we need to extract the working tree from the bare repo
fn checkout_from_bare(
    bare_git_path: &Path,
    checkout_dir: &Path,
    dep: &crate::types::config::Dependency,
) -> Result<()> {
    // If checkout already exists and is non-empty, ensure files are read-only
    // (handles upgrade from older VDW versions that didn't set read-only)
    if checkout_dir.exists() && std::fs::read_dir(checkout_dir)?.next().is_some() {
        make_dir_readonly_recursive(checkout_dir)?;
        return Ok(());
    }

    std::fs::create_dir_all(checkout_dir)?;

    let branch = dep
        .branch
        .as_deref()
        .or(dep.tag.as_deref())
        .or(dep.commit.as_deref())
        .unwrap_or("main");

    // Use git2 to checkout the tree into checkout_dir
    let repo = match git2::Repository::open_bare(bare_git_path) {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                "Cannot open bare repo at {}: {}",
                bare_git_path.display(),
                e
            );
            return Ok(());
        }
    };

    // Try to find the branch/commit reference and get the tree
    let tree = find_tree_for_ref(&repo, branch);

    match tree {
        Ok(tree) => {
            write_tree_to_dir(&repo, &tree, checkout_dir)?;
        }
        Err(e) => {
            // If we can't find the branch, it might not have been fetched
            tracing::warn!("Cannot find branch/commit '{}' in bare repo: {}", branch, e);
        }
    }

    Ok(())
}

/// Find the git tree for a given branch/tag/commit reference
fn find_tree_for_ref<'a>(repo: &'a git2::Repository, reference: &str) -> Result<git2::Tree<'a>> {
    // Try local branch first
    if let Ok(branch_ref) = repo.find_branch(reference, git2::BranchType::Local) {
        let commit = branch_ref.get().peel_to_commit()?;
        return Ok(commit.tree()?);
    }

    // Try remote branch (origin/<reference>)
    let remote_name = format!("origin/{}", reference);
    if let Ok(branch_ref) = repo.find_branch(&remote_name, git2::BranchType::Remote) {
        let commit = branch_ref.get().peel_to_commit()?;
        return Ok(commit.tree()?);
    }

    // Try as a commit OID
    if let Ok(oid) = git2::Oid::from_str(reference) {
        let commit = repo.find_commit(oid)?;
        return Ok(commit.tree()?);
    }

    Err(anyhow::anyhow!(
        "Cannot find reference '{}' in repository",
        reference
    ))
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
    unlink_internal(repo_filter, true)
}

fn unlink_internal(repo_filter: Option<&str>, clear_managed_local: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // Load workspace state
    let state = WorkspaceState::load(&cwd)?;

    if state.links.is_empty() {
        println!("No links to remove.");
        return Ok(());
    }

    // Filter links by repo if specified
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

    if links_to_remove.is_empty() {
        println!("No links found for the specified repository.");
        return Ok(());
    }

    // Verify ownership before deleting anything. Atomic-save editors can turn a
    // hardlink into an independent file that must be preserved.
    engine::verify_links_for_removal(&links_to_remove)?;

    // Remove links
    println!("Removing {} link(s)...", links_to_remove.len());
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

    // Update state: remove only the entries for the unlinked repo(s)
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

    if !clear_managed_local {
        println!("✅ {} link(s) removed.", links_to_remove.len());
        return Ok(());
    }

    // Clear dep.local in .vdwrc for unlinked dependencies.
    // dep.local holds the VDW-managed worktree path (set by `link`), so it must be cleared.
    // But for OLD-format local-only deps (no origin, no remote), dep.local is the original
    // source path and serves as identity — must NOT be cleared.
    let mut config = match parser::load_config(&cwd) {
        Ok(config) => config,
        Err(error) => {
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
    };
    let dep_ids_to_clear: Vec<String> = if let Some(repo) = repo_filter {
        vec![repo.to_string()]
    } else {
        // Full unlink: clear local for all editable deps that have a managed worktree
        config
            .dependencies
            .iter()
            .filter(|d| d.editable && (!d.remote.is_empty() || !d.origin.is_empty()))
            .map(|d| d.dep_id().to_string())
            .collect()
    };
    let mut locks_to_release = Vec::new();
    for dep in &mut config.dependencies {
        // Old-format local-only deps: dep.local IS the identity, don't clear
        if dep.remote.is_empty() && dep.origin.is_empty() {
            continue;
        }
        let dep_id = dep.dep_id().to_string();
        if dep_ids_to_clear.contains(&dep_id) && dep.editable {
            let branch = dep.branch.as_deref().unwrap_or("main");
            locks_to_release.push((dep_id, branch.to_string()));
            dep.local = String::new();
        }
    }
    if let Err(error) = parser::save_config(&cwd, &config) {
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

    for (dep_id, branch) in locks_to_release {
        if let Err(e) = release_editable_lock(&dep_id, &branch, &cwd) {
            tracing::warn!(
                "Failed to release editable lock for {} branch '{}': {}",
                dep_id,
                branch,
                e
            );
        }
    }

    println!("✅ {} link(s) removed.", links_to_remove.len());
    Ok(())
}

/// Rebuild every workspace link as one CLI transaction.
///
/// Unlike the old extension-side `unlink` + `link` sequence, this keeps
/// `.vdwrc` stable until the replacement links are ready and restores the
/// previous state if either phase fails.
pub fn relink(allow_dirty: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let original_config = parser::load_config(&cwd)?;
    let original_state = WorkspaceState::load(&cwd)?;

    engine::verify_links_for_removal(&original_state.links)?;
    ensure_editable_worktrees_clean(&original_config, &original_state, allow_dirty)?;

    println!("Relinking workspace as a single transaction...");
    let relink_result = (|| -> Result<()> {
        unlink_internal(None, false)?;
        execute(None, false)
    })();

    if let Err(error) = relink_result {
        let rollback_result = restore_relink_snapshot(&cwd, &original_config, &original_state);
        return match rollback_result {
            Ok(()) => Err(error.context("Relink failed; the previous workspace was restored")),
            Err(rollback_error) => Err(error.context(format!(
                "Relink failed and rollback was incomplete: {}",
                rollback_error
            ))),
        };
    }

    println!("✅ Workspace relink transaction completed.");
    Ok(())
}

fn restore_relink_snapshot(
    cwd: &Path,
    config: &crate::types::config::VDWConfig,
    state: &WorkspaceState,
) -> Result<()> {
    let mut rollback_errors = Vec::new();

    if let Err(error) = engine::restore_links(&state.links) {
        rollback_errors.push(format!("links: {}", error));
    }
    if let Err(error) = state.save() {
        rollback_errors.push(format!("state.json: {}", error));
    }
    if let Err(error) = parser::save_config(cwd, config) {
        rollback_errors.push(format!(".vdwrc: {}", error));
    }

    if !rollback_errors.is_empty() {
        bail!(rollback_errors.join("; "));
    }
    Ok(())
}

fn ensure_editable_worktrees_clean(
    config: &crate::types::config::VDWConfig,
    state: &WorkspaceState,
    allow_dirty: bool,
) -> Result<()> {
    if allow_dirty {
        return Ok(());
    }

    let mut checked = HashSet::new();
    let mut dirty_worktrees = Vec::new();

    for dep in config.dependencies.iter().filter(|dep| dep.editable) {
        let worktree = find_managed_worktree(dep, state)?;
        let Some(worktree) = worktree else {
            continue;
        };
        let normalized = std::fs::canonicalize(&worktree).unwrap_or(worktree.clone());
        if !checked.insert(normalized) {
            continue;
        }

        let status = crate::worktree::check_dirty(&worktree).with_context(|| {
            format!(
                "Failed to inspect editable worktree for '{}': {}",
                dep.dep_id(),
                worktree.display()
            )
        })?;
        if status.is_dirty() {
            dirty_worktrees.push(format!(
                "{} ({} changes: {} modified, {} added, {} deleted, {} untracked)",
                worktree.display(),
                status.total(),
                status.modified,
                status.added,
                status.deleted,
                status.untracked
            ));
        }
    }

    if !dirty_worktrees.is_empty() {
        bail!(
            "Refusing to relink editable worktrees with uncommitted changes:\n  - {}\n\
             Commit or stash the changes first. Use --allow-dirty only when preserving them is intentional.",
            dirty_worktrees.join("\n  - ")
        );
    }

    Ok(())
}

fn find_managed_worktree(
    dep: &crate::types::config::Dependency,
    state: &WorkspaceState,
) -> Result<Option<PathBuf>> {
    let matching_links: Vec<_> = state
        .links
        .iter()
        .filter(|entry| entry.dependency_repo == dep.dep_id())
        .collect();

    for entry in &matching_links {
        let discover_from = entry.source.parent().unwrap_or(&entry.source);
        if let Ok(repo) = git2::Repository::discover(discover_from) {
            if let Some(workdir) = repo.workdir() {
                return Ok(Some(workdir.to_path_buf()));
            }
        }
    }

    if !dep.local.is_empty() {
        let local = PathBuf::from(&dep.local);
        if local.join(".git").exists() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::{Dependency, Scope, VDWConfig, WorkspaceMeta};
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
    fn test_record_managed_worktree_restores_reused_remote_path() {
        let mut dep = editable_dependency("https://github.com/example/repo.git", "", "");
        let worktree = Path::new("/store/objects/repo/worktrees/main");

        record_managed_worktree(&mut dep, worktree);

        assert_eq!(dep.local, worktree.to_string_lossy());
        assert!(dep.origin.is_empty());
    }

    #[test]
    fn test_record_managed_worktree_preserves_local_origin() {
        let mut dep = editable_dependency("", "/projects/repo", "");
        let worktree = Path::new("/store/objects/repo/worktrees/main");

        record_managed_worktree(&mut dep, worktree);

        assert_eq!(dep.origin, "/projects/repo");
        assert_eq!(dep.local, worktree.to_string_lossy());
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
    fn test_relink_preflight_rejects_dirty_editable_worktree() {
        let repo_dir = create_clean_git_repo();
        let dep = editable_dependency(
            "https://github.com/example/repo.git",
            &repo_dir.path().to_string_lossy(),
            "",
        );
        let config = VDWConfig {
            workspace: WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![dep],
        };
        let state = WorkspaceState::new(repo_dir.path().join("workspace"));

        ensure_editable_worktrees_clean(&config, &state, false)
            .expect("Clean worktree should pass preflight");

        fs::write(repo_dir.path().join("untracked.txt"), "dirty")
            .expect("Failed to create dirty file");
        let error = ensure_editable_worktrees_clean(&config, &state, false)
            .expect_err("Dirty worktree should block relink");
        assert!(error.to_string().contains("uncommitted changes"));
        ensure_editable_worktrees_clean(&config, &state, true)
            .expect("--allow-dirty should bypass the preflight");
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

    /// Test that checkout_from_bare skips when checkout already exists
    #[test]
    fn test_checkout_from_bare_skips_existing() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let checkout_dir = dir.path().join("checkout");
        fs::create_dir_all(&checkout_dir).expect("Failed to create checkout dir");

        // Put a file in checkout to make it non-empty
        fs::write(checkout_dir.join("existing.txt"), "content").expect("Failed to write file");

        // Create a minimal dep config
        let dep = crate::types::config::Dependency {
            remote: "https://github.com/test/repo.git".to_string(),
            origin: String::new(),
            local: String::new(),
            repo: String::new(),
            path: "libs/repo".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope::default(),
            editable: false,
        };

        // Should skip (no bare.git exists, but that's ok because it skips before opening)
        let bare_git_path = dir.path().join("bare.git");
        let result = checkout_from_bare(&bare_git_path, &checkout_dir, &dep);
        assert!(
            result.is_ok(),
            "Should succeed when checkout already exists"
        );

        // Existing file should still be there
        assert!(
            checkout_dir.join("existing.txt").exists(),
            "Existing file should not be touched"
        );
    }
}
