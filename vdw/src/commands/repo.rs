use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::commands::link;
use crate::config::parser;
use crate::store::editable_locks::{
    acquire_editable_lock, check_editable_lock, release_editable_lock,
};
use crate::store::{get_store_path, init_store, object_pool};
use crate::types::config::{Dependency, Scope, VDWConfig};

/// Check if the input looks like a remote URL (contains :// or @)
fn is_remote_url(input: &str) -> bool {
    input.contains("://") || input.contains('@')
}

/// Return a repository-like basename for URLs and local paths.
fn repo_basename(input: &str) -> &str {
    input
        .trim_end_matches(|c| c == '/' || c == '\\')
        .rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or("")
        .trim_end_matches(".git")
}

/// Match every identifier accepted by repository commands consistently.
fn dependency_matches(dep: &Dependency, name: &str) -> bool {
    if dep.remote == name || dep.origin == name || dep.local == name || dep.path == name {
        return true;
    }

    // Only treat a separator-free input as a short repository name. Full paths
    // must match exactly to avoid removing another repository with the same basename.
    if name.contains('/') || name.contains('\\') {
        return false;
    }

    let short_name = name.trim_end_matches(".git");
    [&dep.remote, &dep.origin, &dep.local]
        .into_iter()
        .any(|identifier| !identifier.is_empty() && repo_basename(identifier) == short_name)
}

/// Add a repository to the workspace
/// This will:
/// 1. Clone the repository to the Content Store (for remote URLs)
/// 2. Update the .vdwrc configuration file
pub fn add(
    url: String,
    path: Option<String>,
    branch: Option<String>,
    editable: bool,
) -> Result<()> {
    if is_remote_url(&url) {
        // Remote repo: clone to store, then update config
        let store_path = ensure_store()?;

        // Step 1: Clone repository to Content Store
        let repo_dir = object_pool::clone_to_store(&url, &store_path, branch.as_deref(), None)?;

        println!(
            "\x1b[32m✓\x1b[0m Repository added to store: {}",
            repo_dir.display()
        );
        println!("  URL: {}", url);
        if let Some(ref b) = branch {
            println!("  Branch: {}", b);
        }

        // Step 2: Update .vdwrc configuration
        update_config(&url, "", path, branch, editable)?;
    } else {
        // Local repo: skip clone, just add to config
        let local_path = std::fs::canonicalize(&url)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| url.clone());

        println!("\x1b[32m✓\x1b[0m Adding local repository: {}", local_path);

        update_config("", &local_path, path, branch, editable)?;
    }

    Ok(())
}

/// Update the .vdwrc configuration file to add the new dependency
fn update_config(
    remote: &str,
    local: &str,
    path: Option<String>,
    branch: Option<String>,
    editable: bool,
) -> Result<()> {
    // Find workspace root (directory containing .vdwrc)
    let workspace_dir = find_workspace_root()?;
    let config_path = workspace_dir.join(".vdwrc");

    // Load existing config
    let mut config = if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;
        parser::parse_yaml(&content)?
    } else {
        // Create default config
        VDWConfig {
            workspace: crate::types::config::WorkspaceMeta {
                name: "my-workspace".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![],
        }
    };

    // Duplicate detection: same repo + same branch
    let is_dup = config.dependencies.iter().any(|dep| {
        let same_repo = if !remote.is_empty() {
            dep.remote == remote
        } else if !local.is_empty() {
            dep.local == local || dep.origin == local
        } else {
            false
        };
        let same_branch = dep.branch == branch;
        same_repo && same_branch
    });
    if is_dup {
        println!("\x1b[33m\x1b[0m Repository already exists in configuration");
        return Ok(());
    }

    // Determine the source for path auto-generation
    let source = if !remote.is_empty() { remote } else { local };
    let repo_name = source
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .replace(".git", "");

    // Check editable lock conflict before creating dependency
    let mut editable = editable;
    let repo_id = if !remote.is_empty() {
        remote.to_string()
    } else {
        local.to_string()
    };
    let branch_name = branch.as_deref().unwrap_or("main").to_string();

    if editable {
        if let Ok(Some(conflict)) = check_editable_lock(&repo_id, &branch_name, &workspace_dir) {
            println!(
                "Warning: {} branch '{}' is already editable in workspace: {}. Downgrading to read-only mode.",
                repo_id, branch_name, conflict.workspace_path
            );
            editable = false;
        }
    }

    let dependency_path = path.unwrap_or_else(|| format!("./packages/{}", repo_name));

    // Create new dependency
    let dependency = Dependency {
        remote: remote.to_string(),
        origin: String::new(),
        local: local.to_string(),
        repo: String::new(),
        path: dependency_path.clone(),
        branch,
        tag: None,
        commit: None,
        scope: Scope {
            include: vec!["**/*".to_string()], // Default: all files
            exclude: vec![],
        },
        editable,
    };

    // Validate dependency
    if let Err(e) = dependency.validate() {
        anyhow::bail!("Invalid dependency configuration: {}", e);
    }

    // Add to dependencies list
    config.dependencies.push(dependency);

    // Save updated config
    let yaml_content = serde_yaml::to_string(&config)?;
    std::fs::write(&config_path, &yaml_content)?;

    // Acquire editable lock after successful config write
    if editable {
        let worktree_path = workspace_dir.join(&dependency_path);
        if let Err(e) =
            acquire_editable_lock(&repo_id, &branch_name, &workspace_dir, &worktree_path)
        {
            tracing::warn!("Failed to acquire editable lock: {}", e);
        }
    }

    println!(
        "\x1b[32m✓\x1b[0m Configuration updated: {}",
        config_path.display()
    );
    println!("  Dependencies: {} total", config.dependencies.len());

    Ok(())
}

/// Find the workspace root directory (containing .vdwrc)
fn find_workspace_root() -> Result<PathBuf> {
    let current_dir = std::env::current_dir()?;
    let mut dir = current_dir.as_path();

    loop {
        if dir.join(".vdwrc").exists() || dir.join(".vdwrc.yml").exists() {
            return Ok(dir.to_path_buf());
        }

        match dir.parent() {
            Some(parent) => dir = parent,
            None => {
                anyhow::bail!(
                    "No .vdwrc configuration file found in current directory or parents. \
                     Run `vdw init` first."
                )
            }
        }
    }
}

/// Resolve a user-provided name to the dep_id used in state.links.
/// dep_id priority: remote > origin > local
fn resolve_repo_id(name: &str) -> Option<String> {
    // If it's already a URL, that IS the dep_id
    if is_remote_url(name) {
        return Some(name.to_string());
    }

    // Otherwise, look up in .vdwrc to find the matching dep's dep_id
    let workspace_dir = find_workspace_root().ok()?;
    let config = parser::load_config(&workspace_dir).ok()?;

    config
        .dependencies
        .iter()
        .find(|dep| dependency_matches(dep, name))
        .map(|dep| dep.dep_id().to_string())
}

/// Remove a repository from the workspace
pub fn remove(name: String, force: bool) -> Result<()> {
    // Step 1: Resolve name to the dep_id used in state.links
    let repo_id = resolve_repo_id(&name);

    let mut force_remove = force;

    // Check for uncommitted changes in editable repos before removal
    if let Ok(workspace_dir) = find_workspace_root() {
        if let Ok(config) = parser::load_config(&workspace_dir) {
            for dep in &config.dependencies {
                if dependency_matches(dep, &name) && dep.editable {
                    // Determine worktree path
                    let workspace_target = workspace_dir.join(&dep.path);
                    let worktree_path = if workspace_target.join(".git").exists() {
                        workspace_target
                    } else if !dep.local.is_empty() {
                        let p = std::path::Path::new(&dep.local);
                        if p.is_absolute() {
                            p.to_path_buf()
                        } else {
                            workspace_dir.join(&dep.local)
                        }
                    } else {
                        continue;
                    };

                    if worktree_path.exists() {
                        match crate::worktree::check_dirty(&worktree_path) {
                            Ok(status) if status.is_dirty() && !force_remove => {
                                let branch_info = dep.branch.as_deref().unwrap_or("unknown");
                                if atty::is(atty::Stream::Stdin) {
                                    // Interactive terminal: show prompt
                                    println!("\n\x1b[33m⚠️  Repository \"{}\" (branch: {}) has uncommitted changes:\x1b[0m", name, branch_info);
                                    println!(
                                        "    {} modified, {} added, {} deleted, {} untracked\n",
                                        status.modified,
                                        status.added,
                                        status.deleted,
                                        status.untracked
                                    );

                                    let items = vec![
                                        "Discard changes and remove",
                                        "Keep worktree files (remove from config only)",
                                        "Cancel",
                                    ];
                                    let selection = dialoguer::Select::new()
                                        .with_prompt("What would you like to do?")
                                        .items(&items)
                                        .default(2)
                                        .interact()?;

                                    match selection {
                                        0 => force_remove = true,
                                        1 => {
                                            // Keep files: only remove from config
                                            remove_from_config(&name)?;
                                            println!("\x1b[32m✓\x1b[0m Removed from .vdwrc configuration (worktree files preserved)");
                                            return Ok(());
                                        }
                                        _ => {
                                            println!("Operation cancelled.");
                                            return Ok(());
                                        }
                                    }
                                } else {
                                    anyhow::bail!(
                                        "Repository '{}' (branch: {}) has {} uncommitted change(s). Re-run with --force only if discarding them is intentional.",
                                        name,
                                        branch_info,
                                        status.total()
                                    );
                                }
                            }
                            Ok(_) => { /* clean, continue */ }
                            Err(e) => {
                                tracing::warn!("Failed to check dirty status for {}: {}", name, e);
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    // Step 2: Unlink first (remove linked files + update state.json)
    if let Some(ref id) = repo_id {
        let result = if force_remove {
            link::unlink_force(Some(id.as_str()))
        } else {
            link::unlink(Some(id.as_str()))
        };
        result.with_context(|| format!("Failed to unlink repository '{}'", name))?;
    }

    // Step 3: Remove from .vdwrc configuration
    let config_removed = remove_from_config(&name)?;

    // Also try to remove from the Content Store (best-effort, non-blocking)
    if let Ok(store_path) = ensure_store() {
        if is_remote_url(&name) {
            // Direct URL match - remove from store
            if object_pool::remove_from_store(&store_path, &name).is_ok() {
                println!("\x1b[32m✓\x1b[0m Repository removed from store: {}", name);
            }
        } else {
            // Search by name in the store
            if let Ok(repos) = object_pool::list_store_repos(&store_path) {
                let matched: Vec<_> = repos
                    .iter()
                    .filter(|(dir_name, metadata)| {
                        dir_name.starts_with(&name)
                            || dir_name.contains(&name)
                            || metadata.repo_url.contains(&name)
                    })
                    .collect();

                match matched.len() {
                    0 => {
                        if !config_removed {
                            anyhow::bail!(
                                "No repository found matching '{}'. Use `vdw repo list` to see available repositories.",
                                name
                            );
                        }
                    }
                    1 => {
                        let (_, metadata) = &matched[0];
                        if object_pool::remove_from_store(&store_path, &metadata.repo_url).is_ok() {
                            println!(
                                "\x1b[32m✓\x1b[0m Repository removed from store: {}",
                                metadata.repo_url
                            );
                        }
                    }
                    _ => {
                        println!(
                            "\x1b[33mMultiple repositories match '{}' in store:\x1b[0m",
                            name
                        );
                        for (dir_name, metadata) in matched {
                            println!("  {} -> {}", dir_name, metadata.repo_url);
                        }
                        println!("\nPlease provide a more specific name or use the full URL.");
                    }
                }
            }
        }
    } else if !config_removed {
        anyhow::bail!(
            "No repository found matching '{}'. Use `vdw repo list` to see available repositories.",
            name
        );
    }

    Ok(())
}

/// Remove a dependency from .vdwrc configuration
fn remove_from_config(name: &str) -> Result<bool> {
    let workspace_dir = match find_workspace_root() {
        Ok(dir) => dir,
        Err(_) => return Ok(false),
    };
    let mut config = parser::load_config(&workspace_dir)?;

    let original_len = config.dependencies.len();

    // Find dependencies being removed to release their editable locks
    let deps_to_remove: Vec<_> = config
        .dependencies
        .iter()
        .filter(|dep| dependency_matches(dep, name))
        .cloned()
        .collect();

    config
        .dependencies
        .retain(|dep| !dependency_matches(dep, name));

    let removed = config.dependencies.len() < original_len;

    if removed {
        parser::save_config(&workspace_dir, &config)?;

        // Release editable locks only after the configuration update succeeds.
        for dep in &deps_to_remove {
            let dep_id = dep.dep_id().to_string();
            let branch = dep.branch.as_deref().unwrap_or("main");
            if let Err(e) = release_editable_lock(&dep_id, branch, &workspace_dir) {
                tracing::warn!(
                    "Failed to release editable lock for {} branch '{}': {}",
                    dep_id,
                    branch,
                    e
                );
            }
        }

        println!(
            "\x1b[32m✓\x1b[0m Repository removed from configuration: {}",
            name
        );
    }

    Ok(removed)
}

/// List all repositories in the workspace
pub fn list() -> Result<()> {
    let workspace_dir = find_workspace_root()?;
    let config_path = workspace_dir.join(".vdwrc");

    if !config_path.exists() {
        println!("No workspace configuration found. Run `vdw init` first.");
        return Ok(());
    }

    let content = std::fs::read_to_string(&config_path)?;
    let config = parser::parse_yaml(&content)?;

    if config.dependencies.is_empty() {
        println!("No repositories in the workspace. Use `vdw repo add <url>` to add one.");
        return Ok(());
    }

    // Print table header
    println!("{:<40} {:<12} {:<8} {}", "REPO", "BRANCH", "EDIT", "PATH");
    println!("{}", "-".repeat(80));

    for dep in &config.dependencies {
        // Show dep.remote as primary identifier, dep.local if remote is empty
        let source = dep.dep_id().to_string();
        let branch = dep
            .branch
            .as_deref()
            .or(dep.tag.as_deref())
            .or(dep.commit.as_deref())
            .unwrap_or("(none)");
        let editable = if dep.editable { "yes" } else { "no" };

        // Truncate long sources
        let display_source = if source.len() > 38 {
            format!("{}…", &source[..35])
        } else {
            source
        };

        println!(
            "{:<40} {:<12} {:<8} {}",
            display_source, branch, editable, dep.path
        );
    }

    println!();
    println!("Total: {} repository(ies)", config.dependencies.len());

    Ok(())
}

/// Show repository dirty status as JSON
pub fn status(name: String) -> Result<()> {
    let workspace_dir = find_workspace_root()?;
    let config = parser::load_config(&workspace_dir)?;

    // Find matching dependency
    let dep = config
        .dependencies
        .iter()
        .find(|dep| dependency_matches(dep, &name));

    let dep = match dep {
        Some(d) => d,
        None => anyhow::bail!("Repository '{}' not found in workspace configuration", name),
    };

    let branch = dep.branch.as_deref().unwrap_or("unknown").to_string();

    if !dep.editable {
        let output = serde_json::json!({
            "repo": name,
            "branch": branch,
            "editable": false,
            "dirty": false,
            "changes": null,
            "worktree_path": null,
        });
        println!("{}", serde_json::to_string(&output)?);
        return Ok(());
    }

    // Resolve worktree path from dep.local
    let worktree_path = if dep.local.is_empty() {
        None
    } else {
        let p = std::path::Path::new(&dep.local);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            workspace_dir.join(&dep.local)
        };
        Some(resolved)
    };

    match worktree_path {
        Some(ref wp) if wp.exists() => {
            let (dirty, changes) = match crate::worktree::check_dirty(wp) {
                Ok(ds) => (ds.is_dirty(), serde_json::to_value(&ds).ok()),
                Err(e) => {
                    tracing::warn!("Failed to check dirty status for {}: {}", name, e);
                    (false, None)
                }
            };
            let output = serde_json::json!({
                "repo": name,
                "branch": branch,
                "editable": true,
                "dirty": dirty,
                "changes": changes,
                "worktree_path": wp.to_string_lossy(),
            });
            println!("{}", serde_json::to_string(&output)?);
        }
        _ => {
            let output = serde_json::json!({
                "repo": name,
                "branch": branch,
                "editable": true,
                "dirty": false,
                "changes": null,
                "worktree_path": worktree_path.as_ref().map(|p| p.to_string_lossy()),
            });
            println!("{}", serde_json::to_string(&output)?);
        }
    }

    Ok(())
}

/// Ensure the store is initialized, returning the store path
fn ensure_store() -> Result<std::path::PathBuf> {
    let store_path = get_store_path()?;

    if !store_path.exists() {
        tracing::info!("Store not found, initializing...");
        init_store()?;
    }

    Ok(store_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dependency() -> Dependency {
        Dependency {
            remote: "git@github.com:example/service.git".to_string(),
            origin: "/repos/original-service".to_string(),
            local: "/store/worktrees/service-main".to_string(),
            repo: String::new(),
            path: "services/service".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope::default(),
            editable: true,
        }
    }

    #[test]
    fn dependency_match_accepts_all_supported_identifiers() {
        let dep = dependency();

        assert!(dependency_matches(&dep, &dep.remote));
        assert!(dependency_matches(&dep, &dep.origin));
        assert!(dependency_matches(&dep, &dep.local));
        assert!(dependency_matches(&dep, &dep.path));
        assert!(dependency_matches(&dep, "service"));
        assert!(dependency_matches(&dep, "service.git"));
        assert!(dependency_matches(&dep, "original-service"));
        assert!(dependency_matches(&dep, "service-main"));
    }

    #[test]
    fn dependency_match_requires_exact_full_paths() {
        let dep = dependency();

        assert!(!dependency_matches(&dep, "/another/service"));
        assert!(!dependency_matches(&dep, "services/other"));
        assert!(!dependency_matches(&dep, "unknown"));
    }
}
