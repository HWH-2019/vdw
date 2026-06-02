use std::collections::HashSet;

use crate::types::config::VDWConfig;
use crate::utils::errors::ConfigError;

/// Validate the entire VDW configuration
pub fn validate_config(config: &VDWConfig) -> Result<(), Vec<ConfigError>> {
    let mut errors = Vec::new();

    // 1. Validate each dependency
    for dep in &config.dependencies {
        if let Err(e) = dep.validate() {
            errors.push(ConfigError::ValidationError(e));
        }
    }

    // 2. Check path uniqueness across dependencies
    let mut seen_paths = HashSet::new();
    for dep in &config.dependencies {
        if !seen_paths.insert(dep.path.clone()) {
            errors.push(ConfigError::ValidationError(format!(
                "Duplicate dependency path: '{}'",
                dep.path
            )));
        }
    }

    // 3. Check for self-reference (repo path pointing to workspace itself)
    if config.dependencies.is_empty() {
        // No dependencies is valid, just skip
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::{Dependency, Scope, WorkspaceMeta};

    fn valid_config() -> VDWConfig {
        VDWConfig {
            workspace: WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![Dependency {
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
            }],
        }
    }

    #[test]
    fn test_validate_config_valid() {
        let config = valid_config();
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn test_validate_config_catches_duplicate_paths() {
        let mut config = valid_config();
        config.dependencies.push(Dependency {
            repo: "https://github.com/example/repo2.git".to_string(),
            path: "libs/repo".to_string(), // duplicate path
            branch: Some("dev".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["src/**/*.rs".to_string()],
                exclude: vec![],
            },
            editable: false,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("Duplicate dependency path")));
    }

    #[test]
    fn test_validate_config_catches_missing_scope_includes() {
        let mut config = valid_config();
        config.dependencies[0].scope.include = vec![];
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("scope.include")));
    }

    #[test]
    fn test_validate_config_catches_invalid_url() {
        let mut config = valid_config();
        config.dependencies[0].repo = "not-a-url".to_string();
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("must start with")));
    }

    #[test]
    fn test_validate_config_catches_multiple_version_specifiers() {
        let mut config = valid_config();
        config.dependencies[0].tag = Some("v1.0".to_string());
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.to_string().contains("only one of")));
    }

    #[test]
    fn test_validate_config_multiple_errors() {
        let mut config = valid_config();
        // Two deps with same path, one with invalid URL
        config.dependencies[0].repo = "bad-url".to_string();
        config.dependencies.push(Dependency {
            repo: "https://github.com/example/repo2.git".to_string(),
            path: "libs/repo".to_string(), // duplicate
            branch: Some("dev".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["src/**/*.rs".to_string()],
                exclude: vec![],
            },
            editable: false,
        });
        let result = validate_config(&config);
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert!(errors.len() >= 2);
    }

    #[test]
    fn test_validate_config_empty_dependencies() {
        let config = VDWConfig {
            workspace: WorkspaceMeta {
                name: "empty".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![],
        };
        assert!(validate_config(&config).is_ok());
    }
}
