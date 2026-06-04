use anyhow::Result;

use crate::utils::errors::ConfigError;
use crate::types::config::VDWConfig;

/// Resolve environment variables in config values
/// Supports ${VAR_NAME} syntax in remote URLs, local paths, and link paths
pub fn resolve_env_vars(config: &mut VDWConfig) -> Result<()> {
    for dep in &mut config.dependencies {
        dep.remote = resolve_string(&dep.remote)?;
        dep.local = resolve_string(&dep.local)?;
        dep.path = resolve_string(&dep.path)?;
    }
    Ok(())
}

/// Resolve ${VAR_NAME} patterns in a string
fn resolve_string(s: &str) -> Result<String> {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '$' && chars.peek() == Some(&'{') {
            chars.next(); // consume '{'
            let mut var_name = String::new();
            let mut found_end = false;

            for inner_ch in chars.by_ref() {
                if inner_ch == '}' {
                    found_end = true;
                    break;
                }
                var_name.push(inner_ch);
            }

            if !found_end {
                anyhow::bail!(
                    "Unclosed environment variable reference: ${{{}}}",
                    var_name
                );
            }

            if var_name.is_empty() {
                anyhow::bail!("Empty environment variable reference: ${{}}");
            }

            let value = std::env::var(&var_name).map_err(|_| {
                ConfigError::EnvVarNotFound(var_name.clone())
            })?;
            result.push_str(&value);
        } else {
            result.push(ch);
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::{Dependency, Scope, WorkspaceMeta};

    #[test]
    fn test_resolve_string_no_vars() {
        let result = resolve_string("https://github.com/example/repo.git").unwrap();
        assert_eq!(result, "https://github.com/example/repo.git");
    }

    #[test]
    fn test_resolve_string_with_var() {
        std::env::set_var("VDW_TEST_HOST", "github.com");
        let result = resolve_string("https://${VDW_TEST_HOST}/example/repo.git").unwrap();
        assert_eq!(result, "https://github.com/example/repo.git");
        std::env::remove_var("VDW_TEST_HOST");
    }

    #[test]
    fn test_resolve_string_missing_var() {
        let result = resolve_string("https://${VDW_NONEXISTENT_VAR_12345}/repo.git");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_string_unclosed_brace() {
        let result = resolve_string("https://${VDW_UNCLOSED/repo.git");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_string_empty_var_name() {
        let result = resolve_string("https://${}/repo.git");
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_env_vars() {
        std::env::set_var("VDW_TEST_REPO", "https://github.com/test/repo.git");
        std::env::set_var("VDW_TEST_PATH", "libs/test");

        let mut config = VDWConfig {
            workspace: WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![Dependency {
                remote: "${VDW_TEST_REPO}".to_string(),
                origin: String::new(),
                local: String::new(),
                repo: String::new(),
                path: "${VDW_TEST_PATH}".to_string(),
                branch: Some("main".to_string()),
                tag: None,
                commit: None,
                scope: Scope {
                    include: vec!["src/**/*.rs".to_string()],
                    exclude: vec![],
                },
                editable: false,
            }],
        };

        resolve_env_vars(&mut config).unwrap();
        assert_eq!(config.dependencies[0].remote, "https://github.com/test/repo.git");
        assert_eq!(config.dependencies[0].path, "libs/test");

        std::env::remove_var("VDW_TEST_REPO");
        std::env::remove_var("VDW_TEST_PATH");
    }

    #[test]
    fn test_resolve_string_multiple_vars() {
        std::env::set_var("VDW_PROTO", "https");
        std::env::set_var("VDW_HOST2", "gitlab.com");
        let result = resolve_string("${VDW_PROTO}://${VDW_HOST2}/repo.git").unwrap();
        assert_eq!(result, "https://gitlab.com/repo.git");
        std::env::remove_var("VDW_PROTO");
        std::env::remove_var("VDW_HOST2");
    }
}
