use anyhow::Result;

use crate::store::manager;

pub fn execute(template: Option<String>) -> Result<()> {
    let cwd = std::env::current_dir()?;
    manager::init_store()?;
    let created = create_workspace_config(&cwd, template.as_deref())?;
    crate::store::registry::register_workspace(&cwd)?;
    if !created {
        println!("\u{26a0}\u{fe0f}  .vdwrc already exists in this directory.");
        println!("  Use 'vdw validate' to check configuration.");
        return Ok(());
    }

    let store_path = manager::get_store_path()?;
    println!("\u{2705} Store initialized at: {}", store_path.display());
    println!("\u{2705} Created .vdwrc in {}", cwd.display());
    println!("\nNext steps:");
    println!("  1. Edit .vdwrc to add your dependencies");
    println!("  2. Run 'vdw link' to create workspace links");

    Ok(())
}

fn create_workspace_config(cwd: &std::path::Path, template: Option<&str>) -> Result<bool> {
    let vdwrc_path = cwd.join(".vdwrc");
    if vdwrc_path.exists() || cwd.join(".vdwrc.yml").exists() {
        return Ok(false);
    }
    std::fs::write(vdwrc_path, get_template(template))?;
    Ok(true)
}

fn get_template(template_name: Option<&str>) -> String {
    match template_name {
        Some("fullstack") => fullstack_template(),
        _ => default_template(),
    }
}

fn default_template() -> String {
    r#"workspace:
  name: my-workspace
  version: "1.0"
dependencies: []
"#
    .to_string()
}

fn fullstack_template() -> String {
    r#"workspace:
  name: my-fullstack-workspace
  version: "1.0"
dependencies:
  # Example: include a shared UI library
  # - repo: "https://github.com/example/ui-lib.git"
  #   path: libs/ui-lib
  #   branch: main
  #   scope:
  #     include:
  #       - "src/components/**"
  #       - "src/hooks/**"
  #     exclude:
  #       - "src/**/*.test.ts"
  #
  # Example: include a backend service
  # - repo: "https://github.com/example/api-service.git"
  #   path: services/api
  #   tag: v2.0.0
  #   scope:
  #     include:
  #       - "src/**/*.rs"
  #     exclude:
  #       - "src/**/mod.rs"
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_template_is_valid_yaml() {
        let content = default_template();
        let result = serde_yaml::from_str::<crate::types::config::VDWConfig>(&content);
        assert!(
            result.is_ok(),
            "Default template should produce valid YAML config"
        );

        let config = result.unwrap();
        assert_eq!(config.workspace.name, "my-workspace");
        assert_eq!(config.workspace.version, "1.0");
        assert!(config.dependencies.is_empty());
    }

    #[test]
    fn test_fullstack_template_is_valid_yaml() {
        let content = fullstack_template();
        let result = serde_yaml::from_str::<crate::types::config::VDWConfig>(&content);
        assert!(
            result.is_ok(),
            "Fullstack template should produce valid YAML config"
        );

        let config = result.unwrap();
        assert_eq!(config.workspace.name, "my-fullstack-workspace");
        assert!(config.dependencies.is_empty()); // all commented out
    }

    #[test]
    fn test_get_template_default() {
        let content = get_template(None);
        assert!(content.contains("my-workspace"));
    }

    #[test]
    fn test_get_template_fullstack() {
        let content = get_template(Some("fullstack"));
        assert!(content.contains("my-fullstack-workspace"));
    }

    #[test]
    fn test_get_template_unknown_returns_default() {
        let content = get_template(Some("unknown"));
        assert!(content.contains("my-workspace"));
    }

    #[test]
    fn test_create_workspace_config_creates_vdwrc() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_workspace_config(dir.path(), None).unwrap();

        assert!(created);
        assert!(dir.path().join(".vdwrc").exists());
    }

    #[test]
    fn test_create_workspace_config_preserves_existing_vdwrc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".vdwrc"), "existing").unwrap();

        let created = create_workspace_config(dir.path(), None).unwrap();

        assert!(!created);
        let content = std::fs::read_to_string(dir.path().join(".vdwrc")).unwrap();
        assert_eq!(content, "existing");
    }

    #[test]
    fn test_create_workspace_config_preserves_existing_vdwrc_yml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".vdwrc.yml"), "existing").unwrap();

        let created = create_workspace_config(dir.path(), None).unwrap();

        assert!(!created);
        assert!(!dir.path().join(".vdwrc").exists());
    }
}
