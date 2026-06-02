use anyhow::Result;

use crate::config::{parser, validator, env_resolver};

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

    // TODO: Check store exists, links are valid
    println!("  Store check: not yet implemented");
    println!("  Links check: not yet implemented");

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
                println!("    - {} -> {}", dep.repo, dep.path);
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
