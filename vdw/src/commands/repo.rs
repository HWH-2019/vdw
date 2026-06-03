use anyhow::Result;
use std::path::PathBuf;

use crate::config::parser;
use crate::store::{get_store_path, init_store, object_pool};
use crate::types::config::{Dependency, Scope, VDWConfig};

/// Add a repository to the workspace
/// This will:
/// 1. Clone the repository to the Content Store
/// 2. Update the .vdwrc configuration file
pub fn add(url: String, path: Option<String>, branch: Option<String>, editable: bool) -> Result<()> {
    // Ensure the store is initialized
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
    update_config(&url, path, branch, editable)?;

    Ok(())
}

/// Update the .vdwrc configuration file to add the new dependency
fn update_config(
    url: &str,
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

    // Check if dependency already exists
    if config.dependencies.iter().any(|dep| dep.repo == url) {
        println!("\x1b[33m\x1b[0m Repository already exists in configuration");
        return Ok(());
    }

    // Create new dependency
    let dependency = Dependency {
        repo: url.to_string(),
        path: path.unwrap_or_else(|| {
            // Auto-generate path from repo name
            let repo_name = url.split('/').last().unwrap_or("repo").replace(".git", "");
            format!("./packages/{}", repo_name)
        }),
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

/// Remove a repository from the Content Store
pub fn remove(name: String) -> Result<()> {
    let store_path = ensure_store()?;

    // Try to remove by URL first (if it looks like a URL)
    if name.contains("://") || name.contains('@') {
        object_pool::remove_from_store(&store_path, &name)?;
        println!("\x1b[32m✓\x1b[0m Repository removed from store: {}", name);
        return Ok(());
    }

    // Otherwise, search by directory name in the store
    let repos = object_pool::list_store_repos(&store_path)?;
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
            anyhow::bail!("No repository found matching '{}'. Use `vdw repo list` to see available repositories.", name);
        }
        1 => {
            let (_, metadata) = &matched[0];
            object_pool::remove_from_store(&store_path, &metadata.repo_url)?;
            println!(
                "\x1b[32m✓\x1b[0m Repository removed from store: {}",
                metadata.repo_url
            );
        }
        _ => {
            println!("\x1b[33mMultiple repositories match '{}':\x1b[0m", name);
            for (dir_name, metadata) in matched {
                println!("  {} -> {}", dir_name, metadata.repo_url);
            }
            println!("\nPlease provide a more specific name or use the full URL.");
        }
    }

    Ok(())
}

/// List all repositories in the Content Store
pub fn list() -> Result<()> {
    let store_path = ensure_store()?;

    let repos = object_pool::list_store_repos(&store_path)?;

    if repos.is_empty() {
        println!("No repositories in the store. Use `vdw repo add <url>` to add one.");
        return Ok(());
    }

    // Print table header
    println!(
        "{:<30} {:<12} {:<10} {:<8} URL",
        "NAME", "BRANCH", "REFS", "ALT"
    );
    println!("{}", "-".repeat(80));

    for (dir_name, metadata) in &repos {
        let branch = metadata
            .branch
            .as_deref()
            .unwrap_or("(default)");
        let alt = if metadata.alternates_configured {
            "yes"
        } else {
            "no"
        };

        // Truncate long names
        let display_name = if dir_name.len() > 28 {
            format!("{}…", &dir_name[..25])
        } else {
            dir_name.clone()
        };

        println!(
            "{:<30} {:<12} {:<10} {:<8} {}",
            display_name, branch, metadata.reference_count, alt, metadata.repo_url
        );
    }

    println!();
    println!("Total: {} repository(ies)", repos.len());

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
