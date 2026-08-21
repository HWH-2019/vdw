use anyhow::Result;

use crate::config::{env_resolver, parser, validator};
use crate::store::manager as store_manager;
use crate::types::workspace::WorkspaceState;

pub fn execute(check_links: bool) -> Result<()> {
    if check_links {
        let cwd = std::env::current_dir()?;
        match crate::link::watcher::check_links(&cwd) {
            Ok(0) => {
                eprintln!("✅ 所有 HardLink 连接正常");
                return Ok(());
            }
            Ok(count) => {
                eprintln!("⚠️ 发现 {} 个断链文件", count);
                return Ok(());
            }
            Err(e) => {
                // Error already printed by check_links, just return
                return Err(e);
            }
        }
    }

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
                let audit = crate::link::engine::audit_links(&state.links);
                if audit.missing_count > 0 {
                    println!(
                        "⚠️  {} broken link(s) detected. Run 'vdw link' to repair.",
                        audit.missing_count
                    );
                }
                if audit.detached_equivalent_count > 0 {
                    println!(
                        "⚠️  {} detached HardLink target(s) have identical content and can be rebuilt safely.",
                        audit.detached_equivalent_count
                    );
                }
                if !audit.conflicts.is_empty() {
                    println!(
                        "❌ {} link conflict(s) contain workspace changes. Run 'vdw conflicts list'.",
                        audit.conflicts.len()
                    );
                }
                if audit.missing_count == 0
                    && audit.detached_equivalent_count == 0
                    && audit.conflicts.is_empty()
                {
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
