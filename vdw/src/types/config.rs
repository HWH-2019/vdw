use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// VDW workspace configuration root
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VDWConfig {
    pub workspace: WorkspaceMeta,
    pub dependencies: Vec<Dependency>,
}

/// Workspace metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceMeta {
    pub name: String,
    pub version: String,
}

/// Dependency repository config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub repo: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub scope: Scope,
    #[serde(default = "default_editable")]
    pub editable: bool,
}

fn default_editable() -> bool {
    false
}

/// Scope config (controls which files are exposed to AI)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Scope {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

impl Dependency {
    /// Validate dependency configuration
    pub fn validate(&self) -> Result<(), String> {
        // Check repo URL format
        // Support: git@, https://, http://, or local file paths
        let is_valid_url = self.repo.starts_with("git@") 
            || self.repo.starts_with("https://") 
            || self.repo.starts_with("http://")
            || self.repo.starts_with("/")  // Absolute path
            || self.repo.starts_with("./") // Relative path
            || self.repo.starts_with("../"); // Parent path
        
        if !is_valid_url {
            return Err(format!(
                "Invalid repo URL '{}': must start with 'git@', 'https://', 'http://', or be a file path",
                self.repo
            ));
        }

        // Check branch/tag/commit mutual exclusion - exactly one must be set
        let version_specifiers = [&self.branch, &self.tag, &self.commit]
            .iter()
            .filter(|opt| opt.is_some())
            .count();

        if version_specifiers == 0 {
            return Err(format!(
                "Dependency '{}' must specify exactly one of: branch, tag, or commit",
                self.repo
            ));
        }
        if version_specifiers > 1 {
            return Err(format!(
                "Dependency '{}' must specify only one of: branch, tag, or commit (found {})",
                self.repo, version_specifiers
            ));
        }

        // Check scope.include must be non-empty
        if self.scope.include.is_empty() {
            return Err(format!(
                "Dependency '{}' must have at least one scope.include pattern",
                self.repo
            ));
        }

        Ok(())
    }

    /// Generate a short identifier from the repo URL
    pub fn repo_id(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.repo.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result)
            .chars()
            .take(8)
            .collect()
    }
}

impl Scope {
    /// Check if a file path matches the scope rules
    pub fn matches(&self, file_path: &str) -> bool {
        // File must match at least one include pattern
        let included = self.include.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(file_path))
                .unwrap_or(false)
        });

        if !included {
            return false;
        }

        // File must not match any exclude pattern
        let excluded = self.exclude.iter().any(|pattern| {
            glob::Pattern::new(pattern)
                .map(|p| p.matches(file_path))
                .unwrap_or(false)
        });

        !excluded
    }
}

impl fmt::Display for Dependency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.repo, self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dependency() -> Dependency {
        Dependency {
            repo: "https://github.com/example/repo.git".to_string(),
            path: "libs/repo".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["src/**/*.rs".to_string()],
                exclude: vec![],
            },
            editable: false,
        }
    }

    #[test]
    fn test_parse_valid_yaml_config() {
        let yaml = r#"
workspace:
  name: test-workspace
  version: "1.0"
dependencies:
  - repo: "https://github.com/example/repo.git"
    path: libs/repo
    branch: main
    scope:
      include:
        - "src/**/*.rs"
"#;
        let config: VDWConfig = serde_yaml::from_str(yaml).expect("Failed to parse YAML");
        assert_eq!(config.workspace.name, "test-workspace");
        assert_eq!(config.workspace.version, "1.0");
        assert_eq!(config.dependencies.len(), 1);
        assert_eq!(config.dependencies[0].repo, "https://github.com/example/repo.git");
        assert_eq!(config.dependencies[0].branch, Some("main".to_string()));
        assert!(!config.dependencies[0].editable);
    }

    #[test]
    fn test_dependency_validate_invalid_url() {
        let mut dep = sample_dependency();
        dep.repo = "invalid-url".to_string();
        let result = dep.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with"));
    }

    #[test]
    fn test_dependency_validate_multiple_version_specifiers() {
        let mut dep = sample_dependency();
        dep.tag = Some("v1.0".to_string());
        let result = dep.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("only one of"));
    }

    #[test]
    fn test_dependency_validate_no_version_specifier() {
        let mut dep = sample_dependency();
        dep.branch = None;
        let result = dep.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("exactly one of"));
    }

    #[test]
    fn test_dependency_validate_valid() {
        let dep = sample_dependency();
        assert!(dep.validate().is_ok());
    }

    #[test]
    fn test_dependency_validate_git_at_url() {
        let mut dep = sample_dependency();
        dep.repo = "git@github.com:example/repo.git".to_string();
        assert!(dep.validate().is_ok());
    }

    #[test]
    fn test_dependency_validate_empty_scope_include() {
        let mut dep = sample_dependency();
        dep.scope.include = vec![];
        let result = dep.validate();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("scope.include"));
    }

    #[test]
    fn test_repo_id() {
        let dep = sample_dependency();
        let id = dep.repo_id();
        assert_eq!(id.len(), 8);
        // Same repo should produce same id
        let id2 = dep.repo_id();
        assert_eq!(id, id2);
        // Different repo should produce different id
        let mut dep2 = sample_dependency();
        dep2.repo = "https://github.com/other/repo.git".to_string();
        let id3 = dep2.repo_id();
        assert_ne!(id, id3);
    }

    #[test]
    fn test_scope_matches_include_only() {
        let scope = Scope {
            include: vec!["src/**/*.rs".to_string(), "lib/**/*.rs".to_string()],
            exclude: vec![],
        };
        assert!(scope.matches("src/main.rs"));
        assert!(scope.matches("src/foo/bar.rs"));
        assert!(scope.matches("lib/utils.rs"));
        assert!(!scope.matches("tests/test.rs"));
        assert!(!scope.matches("README.md"));
    }

    #[test]
    fn test_scope_matches_include_and_exclude() {
        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec!["src/**/mod.rs".to_string()],
        };
        assert!(scope.matches("src/main.rs"));
        assert!(!scope.matches("src/foo/mod.rs"));
        assert!(!scope.matches("src/mod.rs"));
    }

    #[test]
    fn test_scope_matches_empty_include() {
        let scope = Scope {
            include: vec![],
            exclude: vec![],
        };
        assert!(!scope.matches("any/file.rs"));
    }

    #[test]
    fn test_default_editable_is_false() {
        assert!(!default_editable());
    }

    #[test]
    fn test_scope_default() {
        let scope = Scope::default();
        assert!(scope.include.is_empty());
        assert!(scope.exclude.is_empty());
    }
}
