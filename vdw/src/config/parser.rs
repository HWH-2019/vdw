use std::path::Path;

use anyhow::{Context, Result};

use crate::types::config::VDWConfig;

/// Load and parse .vdwrc from the given directory
pub fn load_config(workspace_dir: &Path) -> Result<VDWConfig> {
    let yaml_path = workspace_dir.join(".vdwrc");

    if yaml_path.exists() {
        let content =
            std::fs::read_to_string(&yaml_path).context("Failed to read .vdwrc file")?;
        return parse_yaml(&content).context("Failed to parse .vdwrc");
    }

    // Also try .vdwrc.yml extension
    let yml_path = workspace_dir.join(".vdwrc.yml");
    if yml_path.exists() {
        let content =
            std::fs::read_to_string(&yml_path).context("Failed to read .vdwrc.yml file")?;
        return parse_yaml(&content).context("Failed to parse .vdwrc.yml");
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
  - repo: "https://github.com/example/repo.git"
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
}
