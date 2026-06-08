use anyhow::Result;
use std::path::{Path, PathBuf};

use crate::config::parser;
use crate::store::editable_locks::release_editable_lock;
use crate::store::{get_store_path, init_store, object_pool};
use crate::worktree::manager;

/// List all worktrees across all dependencies in the workspace
pub fn list() -> Result<()> {
    let workspace_dir = std::env::current_dir()?;
    let config = parser::load_config(&workspace_dir)?;
    let store_path = ensure_store()?;

    if config.dependencies.is_empty() {
        println!("No dependencies configured. Use `vdw repo add <url>` to add one.");
        return Ok(());
    }

    println!("VDW Worktrees:");
    println!("{}", "-".repeat(80));
    println!(
        "{:<25} {:<20} {:<10} PATH",
        "REPO", "BRANCH", "EDITABLE"
    );
    println!("{}", "-".repeat(80));

    let mut total_count = 0;

    for dep in &config.dependencies {
        let repo_url = dep.dep_id();
        let repo_hash = object_pool::compute_repo_hash(repo_url);
        let repo_name = extract_repo_name(repo_url);
        let repo_dir = store_path.join(format!("objects/{}-{}", repo_name, repo_hash));
        let bare_git_path = repo_dir.join("bare.git");

        if !bare_git_path.exists() {
            println!(
                "{:<25} {:<20} {:<10} (not in store)",
                truncate(repo_url, 24),
                "(n/a)",
                if dep.editable { "yes" } else { "no" }
            );
            continue;
        }

        // List worktrees for this repo
        let worktrees = manager::list_worktrees(&bare_git_path)?;

        if worktrees.is_empty() {
            println!(
                "{:<25} {:<20} {:<10} (no worktrees)",
                truncate(repo_url, 24),
                "(n/a)",
                if dep.editable { "yes" } else { "no" }
            );
        } else {
            for wt in &worktrees {
                println!(
                    "{:<25} {:<20} {:<10} {}",
                    truncate(repo_url, 24),
                    truncate(&wt.branch, 19),
                    if dep.editable { "yes" } else { "no" },
                    wt.path.display()
                );
                total_count += 1;
            }
        }
    }

    println!();
    println!("Total: {} worktree(s)", total_count);

    Ok(())
}

/// Create a worktree for a specific repo and branch
pub fn create(repo: String, branch: String) -> Result<()> {
    let store_path = ensure_store()?;

    // Find the bare repo in the store
    let bare_git_path = find_bare_repo_path(&store_path, &repo)?;

    // Determine the worktree base path (under the store's worktrees directory)
    let worktree_base = store_path.join("worktrees");

    let worktree_path = manager::create_worktree(&bare_git_path, &branch, &worktree_base)?;

    println!(
        "\x1b[32m✓\x1b[0m Created worktree for '{}' on branch '{}'",
        repo, branch
    );
    println!("  Path: {}", worktree_path.display());

    Ok(())
}

/// Delete a worktree for a specific repo and branch
pub fn delete(repo: String, branch: String, force: bool) -> Result<()> {
    let store_path = ensure_store()?;

    // Release editable lock before removing worktree
    let workspace_path = std::env::current_dir()?;
    if let Err(e) = release_editable_lock(&repo, &branch, &workspace_path) {
        tracing::warn!("Failed to release editable lock for {} branch '{}': {}", repo, branch, e);
    }

    // Find the bare repo in the store
    let bare_git_path = find_bare_repo_path(&store_path, &repo)?;

    manager::remove_worktree(&bare_git_path, &branch, force)?;

    println!(
        "\x1b[32m✓\x1b[0m Deleted worktree for '{}' on branch '{}'",
        repo, branch
    );

    Ok(())
}

/// Find the bare git path for a repo URL in the store
fn find_bare_repo_path(store_path: &Path, repo: &str) -> Result<PathBuf> {
    // Try matching by URL directly (compute hash)
    let repo_hash = object_pool::compute_repo_hash(repo);
    let repo_name = extract_repo_name(repo);
    let repo_dir = store_path.join(format!("objects/{}-{}", repo_name, repo_hash));
    let bare_git_path = repo_dir.join("bare.git");

    if bare_git_path.exists() {
        return Ok(bare_git_path);
    }

    // If not found by URL, try matching by name/partial URL
    let repos = object_pool::list_store_repos(store_path)?;
    let matched: Vec<_> = repos
        .iter()
        .filter(|(dir_name, metadata)| {
            dir_name.starts_with(repo)
                || dir_name.contains(repo)
                || metadata.repo_url.contains(repo)
        })
        .collect();

    match matched.len() {
        0 => {
            anyhow::bail!(
                "No repository found matching '{}'. Use `vdw repo add <url>` first.",
                repo
            );
        }
        1 => {
            let (_, metadata) = &matched[0];
            let hash = object_pool::compute_repo_hash(&metadata.repo_url);
            let name = extract_repo_name(&metadata.repo_url);
            let dir = store_path.join(format!("objects/{}-{}", name, hash));
            let bare_path = dir.join("bare.git");
            if bare_path.exists() {
                Ok(bare_path)
            } else {
                anyhow::bail!("Bare repository not found at {}", bare_path.display());
            }
        }
        _ => {
            println!("\x1b[33mMultiple repositories match '{}':\x1b[0m", repo);
            for (dir_name, metadata) in matched {
                println!("  {} -> {}", dir_name, metadata.repo_url);
            }
            println!("\nPlease provide a more specific name or use the full URL.");
            Err(anyhow::anyhow!("Multiple repositories match '{}'", repo))
        }
    }
}

/// Ensure the store is initialized, returning the store path
fn ensure_store() -> Result<PathBuf> {
    let store_path = get_store_path()?;

    if !store_path.exists() {
        tracing::info!("Store not found, initializing...");
        init_store()?;
    }

    Ok(store_path)
}

/// Extract repo name from URL (same logic as object_pool)
fn extract_repo_name(repo_url: &str) -> String {
    repo_url
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git")
        .to_string()
}

/// Truncate a string to max_len characters
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len - 1])
    }
}