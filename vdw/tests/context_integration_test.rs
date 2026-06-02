use std::fs;
use std::path::PathBuf;

use vdw::context::filter;
use vdw::context::file_list;
use vdw::context::glob_matcher;
use vdw::types::config::{Dependency, Scope, VDWConfig, WorkspaceMeta};

/// Helper: create a temp directory with a mix of file types
fn create_mixed_file_directory() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");

    let src = dir.path().join("src");
    let tests = dir.path().join("tests");
    let docs = dir.path().join("docs");

    fs::create_dir_all(&src).expect("Failed to create src dir");
    fs::create_dir_all(&tests).expect("Failed to create tests dir");
    fs::create_dir_all(&docs).expect("Failed to create docs dir");

    // Rust files
    fs::write(src.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
    fs::write(src.join("lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");
    fs::write(src.join("utils.rs"), "pub fn utils() {}").expect("Failed to write utils.rs");

    // TypeScript files
    fs::write(src.join("app.ts"), "export function app() {}").expect("Failed to write app.ts");
    fs::write(src.join("index.ts"), "export * from './app'").expect("Failed to write index.ts");

    // Test files
    fs::write(src.join("app.test.ts"), "test('app', () => {})").expect("Failed to write app.test.ts");
    fs::write(src.join("utils.test.ts"), "test('utils', () => {})").expect("Failed to write utils.test.ts");
    fs::write(tests.join("integration.test.ts"), "describe('integration', () => {})")
        .expect("Failed to write integration.test.ts");

    // Markdown files
    fs::write(dir.path().join("README.md"), "# Project").expect("Failed to write README.md");
    fs::write(docs.join("guide.md"), "# Guide").expect("Failed to write guide.md");
    fs::write(docs.join("api.md"), "# API Reference").expect("Failed to write api.md");

    dir
}

/// Test: filter_files with include *.rs returns only Rust files
#[test]
fn test_filter_files_rs_only() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec!["**/*.rs".to_string()],
        exclude: vec![],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");

    // Should include: src/main.rs, src/lib.rs, src/utils.rs
    let file_names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();

    assert!(file_names.iter().any(|f| f == "src/main.rs"), "Should include src/main.rs");
    assert!(file_names.iter().any(|f| f == "src/lib.rs"), "Should include src/lib.rs");
    assert!(file_names.iter().any(|f| f == "src/utils.rs"), "Should include src/utils.rs");

    // Should NOT include TypeScript or Markdown files
    assert!(!file_names.iter().any(|f| f.ends_with(".ts")), "Should not include .ts files");
    assert!(!file_names.iter().any(|f| f.ends_with(".md")), "Should not include .md files");
}

/// Test: filter_files with include *.ts, exclude *.test.ts
#[test]
fn test_filter_files_ts_exclude_tests() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec!["**/*.ts".to_string()],
        exclude: vec!["**/*.test.ts".to_string()],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");
    let file_names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();

    // Should include: src/app.ts, src/index.ts
    assert!(file_names.iter().any(|f| f == "src/app.ts"), "Should include src/app.ts");
    assert!(file_names.iter().any(|f| f == "src/index.ts"), "Should include src/index.ts");

    // Should NOT include test files
    assert!(
        !file_names.iter().any(|f| f.contains("test.ts")),
        "Should not include .test.ts files"
    );
}

/// Test: filter_files with include *.md returns only Markdown files
#[test]
fn test_filter_files_md_only() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec!["**/*.md".to_string()],
        exclude: vec![],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");
    let file_names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();

    assert_eq!(files.len(), 3, "Should have 3 markdown files");
    assert!(file_names.iter().any(|f| f == "README.md"), "Should include README.md");
    assert!(file_names.iter().any(|f| f == "docs/guide.md"), "Should include docs/guide.md");
    assert!(file_names.iter().any(|f| f == "docs/api.md"), "Should include docs/api.md");
}

/// Test: filter_files with multiple include patterns
#[test]
fn test_filter_files_multiple_includes() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec!["**/*.rs".to_string(), "**/*.md".to_string()],
        exclude: vec![],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");

    // Should include both .rs and .md files
    let rs_count = files.iter().filter(|f| f.to_string_lossy().ends_with(".rs")).count();
    let md_count = files.iter().filter(|f| f.to_string_lossy().ends_with(".md")).count();

    assert_eq!(rs_count, 3, "Should have 3 .rs files");
    assert_eq!(md_count, 3, "Should have 3 .md files");
    assert_eq!(files.len(), 6, "Should have 6 total files (3 .rs + 3 .md)");
}

/// Test: filter_files with empty scope returns nothing
#[test]
fn test_filter_files_empty_scope() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec![],
        exclude: vec![],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");
    assert!(files.is_empty(), "Empty scope should match no files");
}

/// Test: filter_files returns sorted results
#[test]
fn test_filter_files_returns_sorted() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec!["**/*.rs".to_string()],
        exclude: vec![],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");

    let file_strs: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
    let mut sorted = file_strs.clone();
    sorted.sort();
    assert_eq!(file_strs, sorted, "Filter results should be sorted");
}

/// Test: generate_context_summary with a sample VDWConfig
#[test]
fn test_generate_context_summary_with_vdwrc_config() {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");

    // Create the directory structure matching the VDWConfig dependencies
    let lib_path = dir.path().join("libs/core");
    let app_path = dir.path().join("apps/web");

    fs::create_dir_all(lib_path.join("src")).expect("Failed to create lib src dir");
    fs::create_dir_all(app_path.join("src/components")).expect("Failed to create app components dir");
    fs::create_dir_all(app_path.join("src/hooks")).expect("Failed to create app hooks dir");

    // Core library: Rust files
    fs::write(lib_path.join("src/main.rs"), "fn main() {}").expect("Failed to write main.rs");
    fs::write(lib_path.join("src/lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");
    fs::write(lib_path.join("src/test.rs"), "#[test] fn test() {}").expect("Failed to write test.rs");

    // Frontend app: TypeScript files
    fs::write(app_path.join("src/components/Button.tsx"), "<Button />").expect("Failed to write Button.tsx");
    fs::write(app_path.join("src/components/Button.test.tsx"), "test Button").expect("Failed to write test");
    fs::write(app_path.join("src/hooks/useAuth.ts"), "export function useAuth() {}")
        .expect("Failed to write useAuth.ts");
    fs::write(app_path.join("src/hooks/useAuth.spec.ts"), "spec useAuth").expect("Failed to write spec");

    let config = VDWConfig {
        workspace: WorkspaceMeta {
            name: "integration-test".to_string(),
            version: "1.0".to_string(),
        },
        dependencies: vec![
            Dependency {
                repo: "https://github.com/example/core.git".to_string(),
                path: "libs/core".to_string(),
                branch: Some("main".to_string()),
                tag: None,
                commit: None,
                scope: Scope {
                    include: vec!["src/**/*.rs".to_string()],
                    exclude: vec!["**/test.rs".to_string()],
                },
                editable: false,
            },
            Dependency {
                repo: "https://github.com/example/frontend.git".to_string(),
                path: "apps/web".to_string(),
                branch: Some("main".to_string()),
                tag: None,
                commit: None,
                scope: Scope {
                    include: vec!["src/components/**".to_string(), "src/hooks/**".to_string()],
                    exclude: vec!["**/*.test.tsx".to_string(), "**/*.spec.ts".to_string()],
                },
                editable: true,
            },
        ],
    };

    let summary = file_list::generate_context_summary(&config, dir.path())
        .expect("Failed to generate context summary");

    // Total: core has 2 files (main.rs + lib.rs, test.rs excluded),
    //        frontend has 2 files (Button.tsx + useAuth.ts, test/spec excluded)
    assert_eq!(summary.total_files, 4, "Expected 4 total files in summary");
    assert!(summary.total_size_bytes > 0, "Total size should be positive");

    // Check per-dependency stats
    assert_eq!(summary.per_dependency.len(), 2, "Should have 2 dependency stats");

    // Core dependency
    let core_stats = &summary.per_dependency[0];
    assert_eq!(core_stats.repo, "https://github.com/example/core.git");
    assert_eq!(core_stats.path, "libs/core");
    assert_eq!(core_stats.file_count, 2, "Core should have 2 files (test.rs excluded)");
    assert!(core_stats.size_bytes > 0);

    // Frontend dependency
    let frontend_stats = &summary.per_dependency[1];
    assert_eq!(frontend_stats.repo, "https://github.com/example/frontend.git");
    assert_eq!(frontend_stats.path, "apps/web");
    assert_eq!(frontend_stats.file_count, 2, "Frontend should have 2 files (test/spec excluded)");
    assert!(frontend_stats.size_bytes > 0);
}

/// Test: generate_context_summary with no dependencies returns empty stats
#[test]
fn test_generate_context_summary_no_dependencies() {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");

    let config = VDWConfig {
        workspace: WorkspaceMeta {
            name: "empty-workspace".to_string(),
            version: "1.0".to_string(),
        },
        dependencies: vec![],
    };

    let summary = file_list::generate_context_summary(&config, dir.path())
        .expect("Failed to generate context summary");

    assert_eq!(summary.total_files, 0);
    assert_eq!(summary.total_size_bytes, 0);
    assert!(summary.per_dependency.is_empty());
}

/// Test: generate_context_summary with missing dependency path returns zero counts
#[test]
fn test_generate_context_summary_missing_path() {
    let dir = tempfile::tempdir().expect("Failed to create temp dir");

    let config = VDWConfig {
        workspace: WorkspaceMeta {
            name: "test".to_string(),
            version: "1.0".to_string(),
        },
        dependencies: vec![Dependency {
            repo: "https://github.com/example/missing.git".to_string(),
            path: "libs/missing".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["**/*.rs".to_string()],
                exclude: vec![],
            },
            editable: false,
        }],
    };

    let summary = file_list::generate_context_summary(&config, dir.path())
        .expect("Failed to generate context summary");

    assert_eq!(summary.total_files, 0);
    assert_eq!(summary.per_dependency[0].file_count, 0);
    assert_eq!(summary.per_dependency[0].size_bytes, 0);
}

/// Test: glob_matcher integration with scope patterns from .vdwrc
#[test]
fn test_glob_matcher_with_vdwrc_patterns() {
    // Load the sample fixture and test its scope patterns
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample.vdwrc");
    let content = fs::read_to_string(&fixture_path).expect("Failed to read sample.vdwrc");
    let config: VDWConfig = serde_yaml::from_str(&content).expect("Failed to parse .vdwrc");

    // Test frontend scope (include: src/components/**, src/hooks/**; exclude: **/*.test.ts, **/*.spec.ts)
    let frontend = &config.dependencies[0];

    // Should match
    assert!(glob_matcher::matches_scope(
        "src/components/Button.tsx",
        &frontend.scope.include,
        &frontend.scope.exclude,
    ));
    assert!(glob_matcher::matches_scope(
        "src/hooks/useAuth.ts",
        &frontend.scope.include,
        &frontend.scope.exclude,
    ));

    // Should not match (excluded)
    assert!(!glob_matcher::matches_scope(
        "src/components/Button.test.ts",
        &frontend.scope.include,
        &frontend.scope.exclude,
    ));
    assert!(!glob_matcher::matches_scope(
        "src/hooks/useAuth.spec.ts",
        &frontend.scope.include,
        &frontend.scope.exclude,
    ));

    // Should not match (not in include)
    assert!(!glob_matcher::matches_scope(
        "src/utils/helpers.ts",
        &frontend.scope.include,
        &frontend.scope.exclude,
    ));

    // Test shared-utils scope (include: src/**; exclude: src/internal/**)
    let shared_utils = &config.dependencies[1];

    assert!(glob_matcher::matches_scope(
        "src/index.ts",
        &shared_utils.scope.include,
        &shared_utils.scope.exclude,
    ));
    assert!(!glob_matcher::matches_scope(
        "src/internal/core.ts",
        &shared_utils.scope.include,
        &shared_utils.scope.exclude,
    ));
}

/// Test: total_size returns correct aggregate file size
#[test]
fn test_total_size_aggregation() {
    let dir = create_mixed_file_directory();
    let scope = Scope {
        include: vec!["**/*.rs".to_string()],
        exclude: vec![],
    };

    let files = filter::filter_files(dir.path(), &scope).expect("Failed to filter files");
    let total = filter::total_size(dir.path(), &files);

    // Calculate expected size manually
    let mut expected_size = 0u64;
    for file in &files {
        let metadata = fs::metadata(dir.path().join(file)).expect("Failed to get metadata");
        expected_size += metadata.len();
    }

    assert_eq!(total, expected_size, "Total size should match sum of individual file sizes");
    assert!(total > 0, "Total size should be positive");
}

/// Test: FileListCache integration with real filter operations
#[test]
fn test_file_list_cache_with_real_filter() {
    let dir = create_mixed_file_directory();
    let cache = file_list::FileListCache::new();

    let scope = Scope {
        include: vec!["**/*.rs".to_string()],
        exclude: vec![],
    };

    let base_path = dir.path().to_path_buf();

    // First call: compute
    let result1 = cache.get_or_compute("rs-dep", || {
        filter::filter_files(&base_path, &scope).expect("Failed to filter files")
    });

    assert_eq!(result1.len(), 3, "Should have 3 .rs files");

    // Second call: cached
    let mut compute_count = 0;
    let result2 = cache.get_or_compute("rs-dep", || {
        compute_count += 1;
        vec![PathBuf::from("should_not_be_returned.rs")]
    });

    assert_eq!(result2.len(), 3, "Cached result should have 3 files");
    assert_eq!(compute_count, 0, "Should not recompute cached entry");

    // Invalidate and recompute
    cache.invalidate("rs-dep");

    let scope2 = Scope {
        include: vec!["**/*.ts".to_string()],
        exclude: vec!["**/*.test.ts".to_string()],
    };
    let result3 = cache.get_or_compute("rs-dep", || {
        filter::filter_files(&base_path, &scope2).expect("Failed to filter files")
    });

    assert!(result3.iter().any(|f| f.to_string_lossy().ends_with(".ts")));
    assert!(!result3.iter().any(|f| f.to_string_lossy().contains("test.ts")));
}
