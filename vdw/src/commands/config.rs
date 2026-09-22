use std::path::PathBuf;

use anyhow::Result;

use crate::store::manager;

pub fn set_workspace_root(path: PathBuf) -> Result<()> {
    let configured_path = path.to_string_lossy();
    let workspace_root = manager::set_workspace_root(&configured_path)?;
    println!(
        "\u{2705} Workspace root configured at: {}",
        workspace_root.display()
    );
    Ok(())
}

pub fn show(json: bool) -> Result<()> {
    let config = manager::load_global_config()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&config)?);
        return Ok(());
    }

    println!("Store path: {}", manager::get_store_path()?.display());
    match config.workspace_root {
        Some(path) if !path.trim().is_empty() => println!("Workspace root: {path}"),
        _ => println!("Workspace root: <not configured>"),
    }
    Ok(())
}
