use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Component, Path};

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
    /// Remote Git repository URL (e.g., git@github.com:org/repo.git)
    /// Empty string for local-only repos
    #[serde(default)]
    pub remote: String,

    /// Legacy migration field used by older VDW versions when `local` was
    /// overwritten with a managed worktree path. New configurations do not
    /// write this field.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub origin: String,

    /// Declarative source path for a local-only repository.
    /// Runtime worktree locations are derived from `path` and never persisted
    /// here.
    #[serde(default)]
    pub local: String,

    /// Legacy field for backward compatibility - mapped to remote/local during parsing
    #[serde(default, skip_serializing)]
    pub repo: String,

    /// Workspace link path (relative to workspace root)
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
    /// Get the canonical identifier for this dependency.
    /// Priority: remote > local > origin (legacy fallback)
    pub fn dep_id(&self) -> &str {
        if !self.remote.is_empty() {
            &self.remote
        } else if !self.local.is_empty() {
            &self.local
        } else {
            &self.origin
        }
    }

    /// Validate dependency configuration
    pub fn validate(&self) -> Result<(), String> {
        // Check that remote, origin, and local are not ALL empty
        if self.remote.is_empty() && self.origin.is_empty() && self.local.is_empty() {
            return Err(
                "Dependency must have at least one of 'remote', 'origin', or 'local' set"
                    .to_string(),
            );
        }

        // Validate remote format: must be git@, https://, http:// prefix, OR empty string
        if !self.remote.is_empty() {
            let is_valid_url = self.remote.starts_with("git@")
                || self.remote.starts_with("https://")
                || self.remote.starts_with("http://");

            if !is_valid_url {
                return Err(format!(
                    "Invalid remote URL '{}': must start with 'git@', 'https://', or 'http://'",
                    self.remote
                ));
            }
        }

        // Check branch/tag/commit mutual exclusion - exactly one must be set
        let version_specifiers = [&self.branch, &self.tag, &self.commit]
            .iter()
            .filter(|opt| opt.is_some())
            .count();

        let dep_id = self.dep_id();

        if version_specifiers == 0 {
            return Err(format!(
                "Dependency '{}' must specify exactly one of: branch, tag, or commit",
                dep_id
            ));
        }
        if version_specifiers > 1 {
            return Err(format!(
                "Dependency '{}' must specify only one of: branch, tag, or commit (found {})",
                dep_id, version_specifiers
            ));
        }

        let workspace_path = Path::new(&self.path);
        let mut has_normal_component = false;
        for component in workspace_path.components() {
            match component {
                Component::Normal(_) => has_normal_component = true,
                Component::CurDir => {}
                Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                    return Err(format!(
                        "Dependency '{}' path must stay inside the workspace: '{}'",
                        dep_id, self.path
                    ));
                }
            }
        }
        if !has_normal_component {
            return Err(format!(
                "Dependency '{}' path must name a subdirectory inside the workspace",
                dep_id
            ));
        }

        // Check scope.include must be non-empty
        if self.scope.include.is_empty() {
            return Err(format!(
                "Dependency '{}' must have at least one scope.include pattern",
                dep_id
            ));
        }

        Ok(())
    }

    /// Generate a short identifier from the repo URL or local path
    pub fn repo_id(&self) -> String {
        let source = self.dep_id();
        let mut hasher = Sha256::new();
        hasher.update(source.as_bytes());
        let result = hasher.finalize();
        format!("{:x}", result).chars().take(8).collect()
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
        write!(f, "{} -> {}", self.dep_id(), self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_dependency() -> Dependency {
        Dependency {
            remote: "https://github.com/example/repo.git".to_string(),
            origin: String::new(),
            local: String::new(),
            repo: String::new(),
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
  - remote: "https://github.com/example/repo.git"
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
        assert_eq!(
            config.dependencies[0].remote,
            "https://github.com/example/repo.git"
        );
        assert_eq!(config.dependencies[0].local, "");
        assert_eq!(config.dependencies[0].branch, Some("main".to_string()));
        assert!(!config.dependencies[0].editable);
    }

    #[test]
    fn test_dependency_validate_both_empty() {
        let mut dep = sample_dependency();
        dep.remote = String::new();
        dep.local = String::new();
        let result = dep.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("at least one of 'remote', 'origin', or 'local'"));
    }

    #[test]
    fn test_dependency_validate_invalid_remote_url() {
        let mut dep = sample_dependency();
        dep.remote = "invalid-url".to_string();
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
    fn test_dependency_validate_rejects_path_outside_workspace() {
        for path in ["../repo", "/tmp/repo", "."] {
            let mut dep = sample_dependency();
            dep.path = path.to_string();
            let error = dep.validate().expect_err("unsafe path must be rejected");
            assert!(error.contains("inside the workspace"));
        }
    }

    #[test]
    fn test_dependency_validate_valid() {
        let dep = sample_dependency();
        assert!(dep.validate().is_ok());
    }

    #[test]
    fn test_dependency_validate_git_at_url() {
        let mut dep = sample_dependency();
        dep.remote = "git@github.com:example/repo.git".to_string();
        assert!(dep.validate().is_ok());
    }

    #[test]
    fn test_dependency_validate_local_only() {
        let mut dep = sample_dependency();
        dep.remote = String::new();
        dep.local = "/path/to/local/repo".to_string();
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
    fn test_repo_id_from_remote() {
        let dep = sample_dependency();
        let id = dep.repo_id();
        assert_eq!(id.len(), 8);
        // Same remote should produce same id
        let id2 = dep.repo_id();
        assert_eq!(id, id2);
        // Different remote should produce different id
        let mut dep2 = sample_dependency();
        dep2.remote = "https://github.com/other/repo.git".to_string();
        let id3 = dep2.repo_id();
        assert_ne!(id, id3);
    }

    #[test]
    fn test_repo_id_from_local_when_no_remote() {
        let mut dep = sample_dependency();
        dep.remote = String::new();
        dep.local = "/path/to/local/repo".to_string();
        let id = dep.repo_id();
        assert_eq!(id.len(), 8);
        // Same local path should produce same id
        let id2 = dep.repo_id();
        assert_eq!(id, id2);
    }

    #[test]
    fn test_repo_id_prefers_declarative_local_over_legacy_origin() {
        let mut dep = sample_dependency();
        dep.remote.clear();
        dep.local = "/repos/current".to_string();
        dep.origin = "/repos/legacy".to_string();

        assert_eq!(dep.dep_id(), "/repos/current");
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

    #[test]
    fn test_editable_lock_registry_new() {
        let registry = EditableLockRegistry::new();
        assert_eq!(registry.version, 1);
        assert!(registry.locks.is_empty());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableLockRegistry {
    pub version: u32,
    pub locks: Vec<EditableLock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditableLock {
    pub repo_id: String,
    pub branch: String,
    pub workspace_path: String,
    pub worktree_path: String,
    pub created_at: String,
}

impl EditableLockRegistry {
    pub fn new() -> Self {
        Self {
            version: 1,
            locks: Vec::new(),
        }
    }
}
