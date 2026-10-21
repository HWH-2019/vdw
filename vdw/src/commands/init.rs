use std::path::Path;

use anyhow::{bail, Result};

use crate::store::manager;

pub fn execute(name: String, template: Option<String>) -> Result<()> {
    validate_workspace_name(&name)?;
    let workspace_root = manager::get_workspace_root()?;
    let workspace_path = workspace_root.join(&name);
    crate::lifecycle::engine::ensure_active(&workspace_path)?;
    prepare_workspace_directory(&workspace_path)?;

    manager::init_store()?;
    let created = create_workspace_config(&workspace_path, &name, template.as_deref())?;
    crate::store::registry::register_workspace(&workspace_path)?;
    if !created {
        println!(
            "\u{26a0}\u{fe0f}  .vdwrc already exists in {}.",
            workspace_path.display()
        );
        println!("  Change to that directory and run 'vdw validate' to check configuration.");
        return Ok(());
    }

    let store_path = manager::get_store_path()?;
    println!("\u{2705} Store initialized at: {}", store_path.display());
    println!(
        "\u{2705} Created workspace at: {}",
        workspace_path.display()
    );
    println!("\nNext steps:");
    println!("  1. Change directory to {}", workspace_path.display());
    println!("  2. Edit .vdwrc to add your dependencies");
    println!("  3. Run 'vdw link' to create workspace links");

    Ok(())
}

fn validate_workspace_name(name: &str) -> Result<()> {
    let path = Path::new(name);
    let mut components = path.components();
    let first = components.next();
    if name.trim().is_empty()
        || name != name.trim()
        || path.is_absolute()
        || components.next().is_some()
        || !matches!(first, Some(std::path::Component::Normal(_)))
    {
        bail!("Workspace name must be a single directory name without path separators");
    }
    Ok(())
}

fn prepare_workspace_directory(workspace_path: &Path) -> Result<()> {
    if workspace_path.exists() {
        if !workspace_path.is_dir() {
            bail!(
                "Workspace path '{}' exists and is not a directory",
                workspace_path.display()
            );
        }
        let has_config =
            workspace_path.join(".vdwrc").exists() || workspace_path.join(".vdwrc.yml").exists();
        if !has_config && std::fs::read_dir(workspace_path)?.next().is_some() {
            bail!(
                "Workspace path '{}' already exists and is not empty",
                workspace_path.display()
            );
        }
        return Ok(());
    }
    std::fs::create_dir(workspace_path)?;
    Ok(())
}

fn create_workspace_config(
    workspace_path: &Path,
    name: &str,
    template: Option<&str>,
) -> Result<bool> {
    let vdwrc_path = workspace_path.join(".vdwrc");
    if vdwrc_path.exists() || workspace_path.join(".vdwrc.yml").exists() {
        return Ok(false);
    }
    std::fs::write(vdwrc_path, get_template(name, template))?;
    Ok(true)
}

fn get_template(name: &str, template_name: Option<&str>) -> String {
    match template_name {
        Some("fullstack") => fullstack_template(name),
        _ => default_template(name),
    }
}

fn default_template(name: &str) -> String {
    let name = serde_json::to_string(name).expect("serializing a string cannot fail");
    format!(
        r#"workspace:
  name: {name}
  version: "1.0"
dependencies: []
"#
    )
}

fn fullstack_template(name: &str) -> String {
    let name = serde_json::to_string(name).expect("serializing a string cannot fail");
    format!(
        r#"workspace:
  name: {name}
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_template_is_valid_yaml() {
        let content = default_template("my-workspace");
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
        let content = fullstack_template("my-fullstack-workspace");
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
        let content = get_template("my-workspace", None);
        assert!(content.contains("my-workspace"));
    }

    #[test]
    fn test_get_template_fullstack() {
        let content = get_template("my-fullstack-workspace", Some("fullstack"));
        assert!(content.contains("my-fullstack-workspace"));
    }

    #[test]
    fn test_get_template_unknown_returns_default() {
        let content = get_template("my-workspace", Some("unknown"));
        assert!(content.contains("my-workspace"));
    }

    #[test]
    fn test_create_workspace_config_creates_vdwrc() {
        let dir = tempfile::tempdir().unwrap();
        let created = create_workspace_config(dir.path(), "project-one", None).unwrap();

        assert!(created);
        assert!(dir.path().join(".vdwrc").exists());
        let content = std::fs::read_to_string(dir.path().join(".vdwrc")).unwrap();
        assert!(content.contains("name: \"project-one\""));
    }

    #[test]
    fn test_create_workspace_config_preserves_existing_vdwrc() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".vdwrc"), "existing").unwrap();

        let created = create_workspace_config(dir.path(), "project-one", None).unwrap();

        assert!(!created);
        let content = std::fs::read_to_string(dir.path().join(".vdwrc")).unwrap();
        assert_eq!(content, "existing");
    }

    #[test]
    fn test_create_workspace_config_preserves_existing_vdwrc_yml() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".vdwrc.yml"), "existing").unwrap();

        let created = create_workspace_config(dir.path(), "project-one", None).unwrap();

        assert!(!created);
        assert!(!dir.path().join(".vdwrc").exists());
    }

    #[test]
    fn test_workspace_name_must_be_one_directory_component() {
        assert!(validate_workspace_name("project-one").is_ok());
        assert!(validate_workspace_name("").is_err());
        assert!(validate_workspace_name("../project-one").is_err());
        assert!(validate_workspace_name("team/project-one").is_err());
        assert!(validate_workspace_name(" project-one").is_err());
    }

    #[test]
    fn test_prepare_workspace_directory_rejects_nonempty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("project-one");
        std::fs::create_dir(&workspace).unwrap();
        std::fs::write(workspace.join("existing.txt"), "existing").unwrap();

        assert!(prepare_workspace_directory(&workspace).is_err());
    }
}
