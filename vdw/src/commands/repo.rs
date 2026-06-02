use anyhow::Result;

use crate::store::{get_store_path, init_store, object_pool};

/// Add a repository to the Content Store
pub fn add(url: String, _path: Option<String>, branch: Option<String>, _editable: bool) -> Result<()> {
    // Ensure the store is initialized
    let store_path = ensure_store()?;

    let repo_dir = object_pool::clone_to_store(&url, &store_path, branch.as_deref(), None)?;

    println!(
        "\x1b[32m✓\x1b[0m Repository added to store: {}",
        repo_dir.display()
    );
    println!("  URL: {}", url);
    if let Some(ref b) = branch {
        println!("  Branch: {}", b);
    }

    Ok(())
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
