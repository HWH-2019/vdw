use anyhow::Result;
use std::path::PathBuf;

use crate::commands::link;
use crate::config::parser;
use crate::store::{get_store_path, init_store, object_pool};
use crate::types::config::{Dependency, Scope, VDWConfig};

/// Check if the input looks like a remote URL (contains :// or @)
fn is_remote_url(input: &str) -> bool {
    input.contains("://") || input.contains('@')
}

/// Add a repository to the workspace
/// This will:
/// 1. Clone the repository to the Content Store (for remote URLs)
/// 2. Update the .vdwrc configuration file
pub fn add(url: String, path: Option<String>, branch: Option<String>, editable: bool) -> Result<()> {
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

    // Duplicate detection: compare by remote for remote repos, local for local repos
    if !remote.is_empty() {
        if config.dependencies.iter().any(|dep| dep.remote == remote) {
            println!("\x1b[33m\x1b[0m Repository already exists in configuration");
            return Ok(());
        }
    } else if !local.is_empty() {
        if config.dependencies.iter().any(|dep| dep.local == local) {
            println!("\x1b[33m\x1b[0m Repository already exists in configuration");
            return Ok(());
        }
    }

    // Determine the source for path auto-generation
    let source = if !remote.is_empty() { remote } else { local };
    let repo_name = source
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .replace(".git", "");

    // Create new dependency
    let dependency = Dependency {
        remote: remote.to_string(),
        origin: String::new(),
        local: local.to_string(),
        repo: String::new(),
        path: path.unwrap_or_else(|| format!("./packages/{}", repo_name)),
        branch,
        tag: None,
        commit: None,
        scope: Scope {
            include: vec!["**/*".to_string()],  // Default: all files
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

    println!("\x1b[32m✓\x1b[0m Configuration updated: {}", config_path.display());
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

    for dep in &config.dependencies {
        let dep_id = dep.dep_id().to_string();

        // Match by short name from remote URL
        let short_name = dep
            .remote
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".git");
        if !short_name.is_empty() && short_name == name {
            return Some(dep_id);
        }

        // Match by origin path (original source for local-only deps)
        if !dep.origin.is_empty() && dep.origin == name {
            return Some(dep_id);
        }

        // Match by exact local path
        if dep.local == name {
            return Some(dep_id);
        }

        // Match by local/origin path basename
        let base_path = if !dep.origin.is_empty() { &dep.origin } else { &dep.local };
        let basename = base_path.rsplit('/').next().unwrap_or("");
        if !basename.is_empty() && basename == name {
            return Some(dep_id);
        }
    }

    None
}

/// Remove a repository from the workspace
pub fn remove(name: String) -> Result<()> {
    // Step 1: Resolve name to the dep_id used in state.links
    // (this is dep.remote if non-empty, otherwise dep.local)
    let repo_id = resolve_repo_id(&name);

    // Step 2: Unlink first (remove linked files + update state.json)
    if let Some(ref id) = repo_id {
        if let Err(e) = link::unlink(Some(id.as_str())) {
            tracing::warn!("Unlink before remove failed (continuing): {}", e);
        }
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
                        println!("\x1b[33mMultiple repositories match '{}' in store:\x1b[0m", name);
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
    let config_path = workspace_dir.join(".vdwrc");

    if !config_path.exists() {
        return Ok(false);
    }

    let content = std::fs::read_to_string(&config_path)?;
    let mut config = parser::parse_yaml(&content)?;

    let original_len = config.dependencies.len();

    if is_remote_url(name) {
        // Match by remote URL
        config.dependencies.retain(|dep| dep.remote != name);
    } else {
        // Match by local path or repo short name
        config.dependencies.retain(|dep| {
            // Check exact local path match
            if dep.local == name {
                return false;
            }
            // Check short name from remote URL (e.g., "repo" from "git@github.com:org/repo.git")
            let short_name = dep
                .remote
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_end_matches(".git");
            if !short_name.is_empty() && short_name == name {
                return false;
            }
            // Check short name from local path basename
            let local_basename = dep.local.rsplit('/').next().unwrap_or("");
            if !local_basename.is_empty() && local_basename == name {
                return false;
            }
            true
        });
    }

    let removed = config.dependencies.len() < original_len;

    if removed {
        let yaml_content = serde_yaml::to_string(&config)?;
        std::fs::write(&config_path, &yaml_content)?;
        println!("\x1b[32m✓\x1b[0m Repository removed from configuration: {}", name);
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
    println!(
        "{:<40} {:<12} {:<8} {}",
        "REPO", "BRANCH", "EDIT", "PATH"
    );
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

/// Ensure the store is initialized, returning the store path
fn ensure_store() -> Result<std::path::PathBuf> {
    let store_path = get_store_path()?;

    if !store_path.exists() {
        tracing::info!("Store not found, initializing...");
        init_store()?;
    }

    Ok(store_path)
}
