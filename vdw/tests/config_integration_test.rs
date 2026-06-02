use std::fs;

use vdw::config::parser;
use vdw::config::validator;
use vdw::config::env_resolver;
use vdw::types::config::VDWConfig;

/// Load the sample fixture file and parse it end-to-end
#[test]
fn test_load_fixture_all_fields_parsed() {
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample.vdwrc");

    let content = fs::read_to_string(&fixture_path)
        .expect("Failed to read sample.vdwrc fixture");

    let config: VDWConfig = parser::parse_yaml(&content).expect("Failed to parse fixture YAML");

    // Verify workspace metadata
    assert_eq!(config.workspace.name, "fullstack-app");
    assert_eq!(config.workspace.version, "1.0");

    // Verify we have 2 dependencies
    assert_eq!(config.dependencies.len(), 2);

    // Verify first dependency (frontend)
    let frontend = &config.dependencies[0];
    assert_eq!(frontend.repo, "https://github.com/example/frontend.git");
    assert_eq!(frontend.path, "./apps/web");
    assert_eq!(frontend.branch, Some("main".to_string()));
    assert_eq!(frontend.tag, None);
    assert_eq!(frontend.commit, None);
    assert!(frontend.editable);
    assert_eq!(frontend.scope.include.len(), 2);
    assert_eq!(frontend.scope.include[0], "src/components/**");
    assert_eq!(frontend.scope.include[1], "src/hooks/**");
    assert_eq!(frontend.scope.exclude.len(), 2);
    assert_eq!(frontend.scope.exclude[0], "**/*.test.ts");
    assert_eq!(frontend.scope.exclude[1], "**/*.spec.ts");

    // Verify second dependency (shared-utils)
    let shared_utils = &config.dependencies[1];
    assert_eq!(shared_utils.repo, "https://github.com/example/shared-utils.git");
    assert_eq!(shared_utils.path, "./packages/utils");
    assert_eq!(shared_utils.branch, None);
    assert_eq!(shared_utils.tag, Some("v2.1.0".to_string()));
    assert_eq!(shared_utils.commit, None);
    assert!(!shared_utils.editable);
    assert_eq!(shared_utils.scope.include.len(), 1);
    assert_eq!(shared_utils.scope.include[0], "src/**");
    assert_eq!(shared_utils.scope.exclude.len(), 1);
    assert_eq!(shared_utils.scope.exclude[0], "src/internal/**");
}

/// Validate the fixture config passes all validation checks
#[test]
fn test_fixture_config_validates() {
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample.vdwrc");

    let content = fs::read_to_string(&fixture_path)
        .expect("Failed to read sample.vdwrc fixture");

    let config: VDWConfig = parser::parse_yaml(&content).expect("Failed to parse fixture YAML");

    // Validate the entire config
    let result = validator::validate_config(&config);
    assert!(result.is_ok(), "Fixture config should pass validation");

    // Also validate each dependency individually
    for dep in &config.dependencies {
        assert!(dep.validate().is_ok(), "Dependency '{}' should be valid", dep.repo);
    }
}

/// Test that env var resolution works with a parsed config
#[test]
fn test_env_var_resolution_integration() {
    std::env::set_var("VDW_ITEST_REPO_URL", "https://github.com/example/resolved.git");
    std::env::set_var("VDW_ITEST_REPO_PATH", "./libs/resolved");

    let yaml = r#"
workspace:
  name: env-test
  version: "1.0"
dependencies:
  - repo: ${VDW_ITEST_REPO_URL}
    path: ${VDW_ITEST_REPO_PATH}
    branch: main
    scope:
      include:
        - "src/**"
"#;

    let mut config: VDWConfig = parser::parse_yaml(yaml).expect("Failed to parse YAML with env vars");

    // Before resolution, raw values should contain ${...}
    assert!(config.dependencies[0].repo.contains("${"));
    assert!(config.dependencies[0].path.contains("${"));

    // Resolve env vars
    env_resolver::resolve_env_vars(&mut config).expect("Failed to resolve env vars");

    // After resolution, values should be substituted
    assert_eq!(config.dependencies[0].repo, "https://github.com/example/resolved.git");
    assert_eq!(config.dependencies[0].path, "./libs/resolved");

    // Config should still validate after resolution
    let result = validator::validate_config(&config);
    assert!(result.is_ok(), "Config should validate after env resolution");

    std::env::remove_var("VDW_ITEST_REPO_URL");
    std::env::remove_var("VDW_ITEST_REPO_PATH");
}

/// Test env var resolution failure with missing variable
#[test]
fn test_env_var_resolution_missing_var_fails() {
    let yaml = r#"
workspace:
  name: env-test
  version: "1.0"
dependencies:
  - repo: ${VDW_ITEST_DEFINITELY_MISSING_12345}
    path: ./libs/test
    branch: main
    scope:
      include:
        - "src/**"
"#;

    let mut config: VDWConfig = parser::parse_yaml(yaml).expect("Failed to parse YAML");

    let result = env_resolver::resolve_env_vars(&mut config);
    assert!(result.is_err(), "Should fail when env var is missing");
}

/// Test scope matching with fixture config patterns
#[test]
fn test_fixture_scope_matching() {
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample.vdwrc");

    let content = fs::read_to_string(&fixture_path)
        .expect("Failed to read sample.vdwrc fixture");

    let config: VDWConfig = parser::parse_yaml(&content).expect("Failed to parse fixture YAML");

    let frontend = &config.dependencies[0];

    // Files that should be included
    assert!(frontend.scope.matches("src/components/Button.tsx"));
    assert!(frontend.scope.matches("src/hooks/useAuth.ts"));
    assert!(frontend.scope.matches("src/components/Nested/Dialog.tsx"));

    // Files that should be excluded (test/spec files)
    assert!(!frontend.scope.matches("src/components/Button.test.ts"));
    assert!(!frontend.scope.matches("src/hooks/useAuth.spec.ts"));

    // Files that should not match include patterns
    assert!(!frontend.scope.matches("src/utils/helpers.ts"));
    assert!(!frontend.scope.matches("README.md"));

    // Test shared-utils scope
    let shared_utils = &config.dependencies[1];

    // Should be included
    assert!(shared_utils.scope.matches("src/index.ts"));
    assert!(shared_utils.scope.matches("src/lib/parse.ts"));

    // Should be excluded (internal)
    assert!(!shared_utils.scope.matches("src/internal/core.ts"));
}

/// Test load_config from a directory with the fixture
#[test]
fn test_load_config_from_directory_with_fixture() {
    let dir = tempfile::tempdir().unwrap();

    // Copy fixture content as .vdwrc in the temp dir
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample.vdwrc");

    let content = fs::read_to_string(&fixture_path)
        .expect("Failed to read sample.vdwrc fixture");

    fs::write(dir.path().join(".vdwrc"), &content).unwrap();

    let config = parser::load_config(dir.path()).expect("Failed to load config from directory");

    assert_eq!(config.workspace.name, "fullstack-app");
    assert_eq!(config.dependencies.len(), 2);
}
