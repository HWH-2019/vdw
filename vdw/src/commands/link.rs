use std::path::Path;

use anyhow::Result;

use crate::config::{env_resolver, parser, validator};
use crate::link::engine;
use crate::store::{manager as store_manager, object_pool};
use crate::types::workspace::WorkspaceState;
use crate::worktree::manager as worktree_manager;

pub fn execute(repo_filter: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // 1. Load and validate config
    let mut config = parser::load_config(&cwd)?;
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
    let mut state = WorkspaceState::load(&cwd).unwrap_or_else(|_| WorkspaceState::new(cwd.clone()));

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
        let repo_dir = object_pool::clone_to_store(
            &dep_id,
            &store_path,
            dep.branch.as_deref(),
            None, // no reference for MVP
        )?;

        let bare_git_path = repo_dir.join("bare.git");
        let target_dir = cwd.join(&dep.path);

        // b. Determine source directory
        let source_dir = if dep.editable {
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

            if let Some(wt_path) = existing_worktree {
                println!("  Using existing worktree: {}", wt_path.display());
                editable_count += 1;
                wt_path
            } else {
                // Create worktree for editable deps
                let branch = dep
                    .branch
                    .as_deref()
                    .or(dep.tag.as_deref())
                    .or(dep.commit.as_deref())
                    .unwrap_or("main");

                let worktree_base = repo_dir.join("worktrees");
                let wt_path =
                    worktree_manager::create_worktree(&bare_git_path, branch, &worktree_base)?;

                // Write back worktree path to dep.local
                // For local-only deps, preserve the original source path in dep.origin
                if dep.remote.is_empty() && dep.origin.is_empty() {
                    dep.origin = dep.local.clone();
                }
                dep.local = wt_path.to_string_lossy().to_string();

                editable_count += 1;
                wt_path
            }
        } else {
            // For read-only deps, checkout from bare repo to a checkout dir inside the store
            let checkout_dir = repo_dir.join("checkout");
            checkout_from_bare(&bare_git_path, &checkout_dir, dep)?;
            readonly_count += 1;
            checkout_dir
        };

        // c. Create links from source to workspace
        if source_dir.exists() {
            let entries =
                engine::create_dependency_links(&source_dir, &target_dir, &dep.scope, &dep_id)?;

            linked_count += entries.len();
            state.links.extend(entries);
        } else {
            println!("  ⚠️  Source not available (network fetch may have failed)");
        }
    }

    // 4. Save updated config with local paths filled in
    parser::save_config(&cwd, &config)?;

    // 5. Save workspace state
    state.save()?;

    // 6. Print summary
    println!("\n✅ Workspace linked successfully!");
    println!("   {} file(s) linked", linked_count);
    println!(
        "   {} editable, {} read-only dependencies",
        editable_count, readonly_count
    );

    Ok(())
}

/// Checkout files from a bare repo to a directory
/// For read-only mode: we need to extract the working tree from the bare repo
fn checkout_from_bare(
    bare_git_path: &Path,
    checkout_dir: &Path,
    dep: &crate::types::config::Dependency,
) -> Result<()> {
    // If checkout already exists and is non-empty, skip
    if checkout_dir.exists() && std::fs::read_dir(checkout_dir)?.next().is_some() {
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
            tracing::warn!("Cannot open bare repo at {}: {}", bare_git_path.display(), e);
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
            tracing::warn!(
                "Cannot find branch/commit '{}' in bare repo: {}",
                branch,
                e
            );
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

/// Recursively write a git tree to a directory
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

pub fn unlink(repo_filter: Option<&str>) -> Result<()> {
    let cwd = std::env::current_dir()?;

    // Load workspace state
    let state = WorkspaceState::load(&cwd)?;

    if state.links.is_empty() {
        println!("No links to remove.");
        return Ok(());
    }

    // Filter links by repo if specified
    let links_to_remove: Vec<_> = if let Some(repo) = repo_filter {
        state.links.iter()
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

    // Remove links
    println!("Removing {} link(s)...", links_to_remove.len());
    engine::remove_links(&links_to_remove)?;

    // Clean up empty directories
    for link in &links_to_remove {
        if let Some(parent) = link.target.parent() {
            let _ = engine::cleanup_empty_dirs(parent, &cwd);
        }
    }

    // Update state: remove only the entries for the unlinked repo(s)
    let mut new_state = state;
    if let Some(repo) = repo_filter {
        // Remove only the matching repo's links
        new_state.links.retain(|l| l.dependency_repo != repo);
    } else {
        // Full unlink: clear all state
        new_state = WorkspaceState::new(cwd.clone());
    }
    new_state.save()?;

    // Clear dep.local in .vdwrc for unlinked dependencies.
    // dep.local holds the VDW-managed worktree path (set by `link`), so it must be cleared.
    // But for OLD-format local-only deps (no origin, no remote), dep.local is the original
    // source path and serves as identity — must NOT be cleared.
    let mut config = parser::load_config(&cwd)?;
    let dep_ids_to_clear: Vec<String> = if let Some(repo) = repo_filter {
        vec![repo.to_string()]
    } else {
        // Full unlink: clear local for all editable deps that have a managed worktree
        config.dependencies.iter()
            .filter(|d| d.editable && (!d.remote.is_empty() || !d.origin.is_empty()))
            .map(|d| d.dep_id().to_string())
            .collect()
    };
    for dep in &mut config.dependencies {
        // Old-format local-only deps: dep.local IS the identity, don't clear
        if dep.remote.is_empty() && dep.origin.is_empty() {
            continue;
        }
        let dep_id = dep.dep_id().to_string();
        if dep_ids_to_clear.contains(&dep_id) && dep.editable {
            dep.local = String::new();
        }
    }
    parser::save_config(&cwd, &config)?;

    println!("✅ {} link(s) removed.", links_to_remove.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::Scope;
    use std::fs;

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
        let loaded_state = WorkspaceState::load(workspace_dir.path()).expect("Failed to load state");
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
        assert!(
            state.links.is_empty(),
            "New workspace should have no links"
        );
    }

    /// Test unlink with mixed link types
    #[test]
    fn test_unlink_mixed_link_types() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

        // Create multiple source files
        fs::create_dir_all(source_dir.path().join("src")).expect("Failed to create src dir");
        fs::write(
            source_dir.path().join("src/main.rs"),
            "fn main() {}",
        )
        .expect("Failed to write main.rs");
        fs::write(
            source_dir.path().join("README.md"),
            "# Test",
        )
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

        assert!(
            entries.len() >= 2,
            "Should have at least 2 linked files"
        );

        // Save state
        let mut state = WorkspaceState::new(workspace_dir.path().to_path_buf());
        state.links = entries;
        state.save().expect("Failed to save state");

        // Remove all links
        let loaded_state = WorkspaceState::load(workspace_dir.path()).expect("Failed to load state");
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
        fs::write(checkout_dir.join("existing.txt"), "content")
            .expect("Failed to write file");

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
        assert!(result.is_ok(), "Should succeed when checkout already exists");

        // Existing file should still be there
        assert!(
            checkout_dir.join("existing.txt").exists(),
            "Existing file should not be touched"
        );
    }
}
