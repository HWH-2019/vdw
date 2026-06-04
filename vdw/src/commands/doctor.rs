use anyhow::Result;

use crate::config::{parser, validator, env_resolver};
use crate::store::manager as store_manager;
use crate::types::workspace::WorkspaceState;

pub fn execute() -> Result<()> {
    println!("Running workspace diagnostics...");

    let cwd = std::env::current_dir()?;

    // Check .vdwrc exists
    match parser::load_config(&cwd) {
        Ok(config) => {
            println!("  .vdwrc: found");
            println!("  Workspace: {}", config.workspace.name);
            println!("  Dependencies: {}", config.dependencies.len());
        }
        Err(e) => {
            println!("  .vdwrc: NOT found ({})", e);
        }
    }

    // Check store exists
    match store_manager::get_store_path() {
        Ok(store_path) => {
            if store_path.exists() {
                println!("  Store: initialized at {}", store_path.display());
            } else {
                println!("  Store: NOT initialized (run 'vdw link' to initialize)");
            }
        }
        Err(e) => {
            println!("  Store: error ({})", e);
        }
    }

    // Check workspace state and link health
    match WorkspaceState::load(&cwd) {
        Ok(state) => {
            if !state.links.is_empty() {
                let mut broken = 0;
                for entry in &state.links {
                    if !entry.target.exists() {
                        broken += 1;
                    }
                }
                if broken > 0 {
                    println!("⚠️  {} broken link(s) detected. Run 'vdw link' to repair.", broken);
                } else {
                    println!("✅ All {} links are valid.", state.links.len());
                }
            } else {
                println!("ℹ️  No links created yet. Run 'vdw link' to create links.");
            }
        }
        Err(e) => {
            println!("  Links check: error ({})", e);
        }
    }

    Ok(())
}

pub fn validate() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let mut config = parser::load_config(&cwd)?;
    env_resolver::resolve_env_vars(&mut config)?;

    match validator::validate_config(&config) {
        Ok(()) => {
            println!("✅ Configuration is valid!");
            println!("  Workspace: {}", config.workspace.name);
            println!("  Dependencies: {}", config.dependencies.len());
            for dep in &config.dependencies {
                println!("    - {} -> {}", dep, dep.path);
            }
            Ok(())
        }
        Err(errors) => {
            println!("❌ Configuration has {} error(s):", errors.len());
            for err in &errors {
                println!("  - {}", err);
            }
            Err(anyhow::anyhow!("Configuration validation failed"))
        }
    }
}
