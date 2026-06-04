use std::path::Path;

use anyhow::{Context, Result};

use crate::types::config::VDWConfig;

/// Load and parse .vdwrc from the given directory
pub fn load_config(workspace_dir: &Path) -> Result<VDWConfig> {
    let yaml_path = workspace_dir.join(".vdwrc");

    if yaml_path.exists() {
        let content =
            std::fs::read_to_string(&yaml_path).context("Failed to read .vdwrc file")?;
        let mut config = parse_yaml(&content).context("Failed to parse .vdwrc")?;
        migrate_legacy_repo_field(&mut config);
        return Ok(config);
    }

    // Also try .vdwrc.yml extension
    let yml_path = workspace_dir.join(".vdwrc.yml");
    if yml_path.exists() {
        let content =
            std::fs::read_to_string(&yml_path).context("Failed to read .vdwrc.yml file")?;
        let mut config = parse_yaml(&content).context("Failed to parse .vdwrc.yml")?;
        migrate_legacy_repo_field(&mut config);
        return Ok(config);
    }

    anyhow::bail!(
        "No .vdwrc configuration file found in {}",
        workspace_dir.display()
    )
}

/// Parse config from a YAML string
pub fn parse_yaml(content: &str) -> Result<VDWConfig> {
    let config: VDWConfig = serde_yaml::from_str(content).context("Failed to parse YAML config")?;
    Ok(config)
}

/// Save config back to .vdwrc in the given directory
pub fn save_config(workspace_dir: &Path, config: &VDWConfig) -> Result<()> {
    let yaml_path = workspace_dir.join(".vdwrc");
    let target_path = if yaml_path.exists() {
        yaml_path
    } else {
        workspace_dir.join(".vdwrc.yml")
    };

    let yaml_content = serde_yaml::to_string(config).context("Failed to serialize config to YAML")?;
    std::fs::write(&target_path, yaml_content)
        .context(format!("Failed to write config to {}", target_path.display()))?;
    Ok(())
}

/// Migrate legacy 'repo' field to 'remote'/'local' for backward compatibility
fn migrate_legacy_repo_field(config: &mut VDWConfig) {
    for dep in &mut config.dependencies {
        if dep.remote.is_empty() && dep.local.is_empty() && !dep.repo.is_empty() {
            if dep.repo.starts_with("git@")
                || dep.repo.starts_with("https://")
                || dep.repo.starts_with("http://")
            {
                dep.remote = dep.repo.clone();
            } else {
                dep.local = dep.repo.clone();
            }
            eprintln!(
                "[warn] Dependency uses deprecated 'repo' field. Please migrate to 'remote'/'local'. Run `vdw migrate-config`."
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_parse_yaml_valid() {
        let yaml = r#"
workspace:
  name: my-project
  version: "1.0"
dependencies:
  - remote: "https://github.com/example/repo.git"
    path: libs/repo
    branch: main
    scope:
      include:
        - "src/**/*.rs"
"#;
        let config = parse_yaml(yaml).expect("Failed to parse valid YAML");
        assert_eq!(config.workspace.name, "my-project");
        assert_eq!(config.dependencies.len(), 1);
    }

    #[test]
    fn test_parse_yaml_invalid() {
        let yaml = "not valid yaml {{{";
        assert!(parse_yaml(yaml).is_err());
    }

    #[test]
    fn test_load_config_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = load_config(dir.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_load_config_from_vdwrc() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
workspace:
  name: test
  version: "1.0"
dependencies: []
"#;
        fs::write(dir.path().join(".vdwrc"), yaml).unwrap();
        let config = load_config(dir.path()).expect("Failed to load config");
        assert_eq!(config.workspace.name, "test");
    }

    #[test]
    fn test_load_config_from_vdwrc_yml() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = r#"
workspace:
  name: test-yml
  version: "2.0"
dependencies: []
"#;
        fs::write(dir.path().join(".vdwrc.yml"), yaml).unwrap();
        let config = load_config(dir.path()).expect("Failed to load config");
        assert_eq!(config.workspace.name, "test-yml");
    }

    #[test]
    fn test_migrate_legacy_repo_remote_url() {
        let yaml = r#"
workspace:
  name: migrate-test
  version: "1.0"
dependencies:
  - repo: "git@github.com:example/repo.git"
    path: libs/repo
    branch: main
    scope:
      include:
        - "src/**"
"#;
        let mut config = parse_yaml(yaml).expect("Failed to parse YAML");
        migrate_legacy_repo_field(&mut config);
        assert_eq!(config.dependencies[0].remote, "git@github.com:example/repo.git");
        assert_eq!(config.dependencies[0].local, "");
    }

    #[test]
    fn test_migrate_legacy_repo_local_path() {
        let yaml = r#"
workspace:
  name: migrate-test
  version: "1.0"
dependencies:
  - repo: "/path/to/local/repo"
    path: libs/repo
    branch: main
    scope:
      include:
        - "src/**"
"#;
        let mut config = parse_yaml(yaml).expect("Failed to parse YAML");
        migrate_legacy_repo_field(&mut config);
        assert_eq!(config.dependencies[0].remote, "");
        assert_eq!(config.dependencies[0].local, "/path/to/local/repo");
    }

    #[test]
    fn test_migrate_legacy_repo_no_migration_when_new_fields_set() {
        let yaml = r#"
workspace:
  name: migrate-test
  version: "1.0"
dependencies:
  - remote: "git@github.com:example/repo.git"
    local: "/path/to/worktree"
    path: libs/repo
    branch: main
    scope:
      include:
        - "src/**"
"#;
        let mut config = parse_yaml(yaml).expect("Failed to parse YAML");
        migrate_legacy_repo_field(&mut config);
        // Should not change - remote/local already set
        assert_eq!(config.dependencies[0].remote, "git@github.com:example/repo.git");
        assert_eq!(config.dependencies[0].local, "/path/to/worktree");
    }
}
