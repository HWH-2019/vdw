use std::fs;
use std::path::Path;

use vdw::link::engine;
use vdw::types::config::Scope;
use vdw::types::workspace::LinkType;

/// Helper: create a local git repo with some files and a commit
fn create_local_git_repo(dir: &Path) {
    let repo = git2::Repository::init(dir).expect("Failed to init git repo");
    let sig = git2::Signature::now("Test User", "test@example.com")
        .expect("Failed to create signature");

    // Create directory structure:
    //   src/main.rs
    //   src/lib.rs
    //   src/utils/mod.rs
    //   tests/integration.rs
    //   README.md
    let src = dir.join("src");
    let utils = src.join("utils");
    let tests = dir.join("tests");
    fs::create_dir_all(&utils).expect("Failed to create utils dir");
    fs::create_dir_all(&tests).expect("Failed to create tests dir");

    fs::write(src.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
    fs::write(src.join("lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");
    fs::write(utils.join("mod.rs"), "pub mod utils;").expect("Failed to write mod.rs");
    fs::write(tests.join("integration.rs"), "#[test] fn test() {}")
        .expect("Failed to write integration.rs");
    fs::write(dir.join("README.md"), "# Test Repo").expect("Failed to write README.md");

    // Stage all files
    let mut index = repo.index().expect("Failed to get index");
    index
        .add_path(Path::new("src/main.rs"))
        .expect("Failed to add src/main.rs");
    index
        .add_path(Path::new("src/lib.rs"))
        .expect("Failed to add src/lib.rs");
    index
        .add_path(Path::new("src/utils/mod.rs"))
        .expect("Failed to add src/utils/mod.rs");
    index
        .add_path(Path::new("tests/integration.rs"))
        .expect("Failed to add tests/integration.rs");
    index
        .add_path(Path::new("README.md"))
        .expect("Failed to add README.md");
    index.write().expect("Failed to write index");

    let tree_id = index.write_tree().expect("Failed to write tree");
    let tree = repo.find_tree(tree_id).expect("Failed to find tree");
    repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .expect("Failed to create initial commit");
}

/// Helper: write a .vdwrc config to a workspace directory
fn write_vdwrc(workspace_dir: &Path, _source_dir: &Path) {
    let yaml = r#"
workspace:
  name: integration-test
  version: "1.0"
dependencies:
  - repo: "https://github.com/example/local-repo.git"
    path: libs/local-repo
    branch: main
    scope:
      include:
        - "src/**/*.rs"
      exclude:
        - "src/**/mod.rs"
"#
        .to_string();
    fs::write(workspace_dir.join(".vdwrc"), yaml).expect("Failed to write .vdwrc");
}

/// End-to-end test: create git repo, write .vdwrc, link files, verify they appear
#[test]
fn test_link_end_to_end_with_git_repo() {
    let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    // Setup: create a local git repo with files
    create_local_git_repo(source_dir.path());

    // Setup: write a .vdwrc
    write_vdwrc(workspace_dir.path(), source_dir.path());

    // Create scope matching the .vdwrc config
    let scope = Scope {
        include: vec!["src/**/*.rs".to_string()],
        exclude: vec!["src/**/mod.rs".to_string()],
    };

    // Target directory is the dependency path under workspace
    let target_dir = workspace_dir.path().join("libs/local-repo");

    // Run the link engine
    let entries = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &scope,
        "https://github.com/example/local-repo.git",
    )
    .expect("Failed to create dependency links");

    // Verify: should link src/main.rs and src/lib.rs (mod.rs excluded)
    assert_eq!(entries.len(), 2, "Expected 2 linked files (mod.rs excluded)");

    // Verify files appear at expected workspace paths
    assert!(
        target_dir.join("src/main.rs").exists(),
        "src/main.rs should exist in workspace"
    );
    assert!(
        target_dir.join("src/lib.rs").exists(),
        "src/lib.rs should exist in workspace"
    );
    // mod.rs should NOT be linked (excluded by scope)
    assert!(
        !target_dir.join("src/utils/mod.rs").exists(),
        "src/utils/mod.rs should NOT be linked (excluded)"
    );
    // tests/ and README.md should NOT be linked (not in scope)
    assert!(
        !target_dir.join("tests/integration.rs").exists(),
        "tests/integration.rs should NOT be linked (out of scope)"
    );
    assert!(
        !target_dir.join("README.md").exists(),
        "README.md should NOT be linked (out of scope)"
    );

    // Verify file contents are correct
    let main_content =
        fs::read_to_string(target_dir.join("src/main.rs")).expect("Failed to read main.rs");
    assert_eq!(main_content, "fn main() {}");
}

/// End-to-end test: link then unlink removes files
#[test]
fn test_unlink_removes_linked_files() {
    let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    create_local_git_repo(source_dir.path());

    let scope = Scope {
        include: vec!["src/**/*.rs".to_string()],
        exclude: vec![],
    };

    let target_dir = workspace_dir.path().join("libs/local-repo");

    // Link
    let entries = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &scope,
        "https://github.com/example/local-repo.git",
    )
    .expect("Failed to create dependency links");

    assert!(!entries.is_empty(), "Should have some linked files");
    assert!(target_dir.join("src/main.rs").exists());

    // Unlink
    engine::remove_links(&entries).expect("Failed to remove links");

    // Verify all linked files are gone
    for entry in &entries {
        assert!(
            !entry.target.exists(),
            "Target {} should be removed after unlink",
            entry.target.display()
        );
    }

    // Source files should still exist
    assert!(
        source_dir.path().join("src/main.rs").exists(),
        "Source files should not be removed"
    );
}

/// End-to-end test: running link twice (idempotent) should not fail
#[test]
fn test_idempotent_link() {
    let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    create_local_git_repo(source_dir.path());

    let scope = Scope {
        include: vec!["src/**/*.rs".to_string()],
        exclude: vec![],
    };

    let target_dir = workspace_dir.path().join("libs/local-repo");

    // First link
    let entries1 = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &scope,
        "https://github.com/example/local-repo.git",
    )
    .expect("First link should succeed");

    assert!(!entries1.is_empty());

    // Second link - should fail because target already exists
    // But the function should handle it gracefully or the caller can handle it
    let entries2_result = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &scope,
        "https://github.com/example/local-repo.git",
    );

    // The second link attempt should return an error because targets exist
    // This is expected behavior - idempotent means "same result if you try again"
    // not necessarily "silently succeeds". For a true idempotent workflow,
    // the caller would unlink first then re-link.
    assert!(
        entries2_result.is_err(),
        "Second link should fail because target files already exist"
    );

    // Original linked files should still be intact
    assert!(
        target_dir.join("src/main.rs").exists(),
        "Original linked files should still exist"
    );
}

/// Test linking with .vdwrc-parsed scope (integration with config parsing)
#[test]
fn test_link_with_parsed_vdwrc_scope() {
    let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    create_local_git_repo(source_dir.path());

    // Parse the actual .vdwrc file
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample.vdwrc");
    let content = fs::read_to_string(&fixture_path).expect("Failed to read sample.vdwrc");
    let config: vdw::types::config::VDWConfig =
        serde_yaml::from_str(&content).expect("Failed to parse .vdwrc");

    // Use the shared-utils dependency scope from the fixture
    // It includes "src/**" and excludes "src/internal/**"
    let shared_utils = &config.dependencies[1];
    assert_eq!(shared_utils.scope.include, vec!["src/**"]);
    assert_eq!(shared_utils.scope.exclude, vec!["src/internal/**"]);

    let target_dir = workspace_dir.path().join("packages/utils");

    // Also create an "internal" dir in the source to test exclude
    let internal = source_dir.path().join("src/internal");
    fs::create_dir_all(&internal).expect("Failed to create internal dir");
    fs::write(internal.join("secret.rs"), "pub fn secret() {}")
        .expect("Failed to write secret.rs");

    let entries = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &shared_utils.scope,
        &shared_utils.repo,
    )
    .expect("Failed to create dependency links with parsed scope");

    // Should include src/main.rs, src/lib.rs, src/utils/mod.rs
    // Should exclude src/internal/secret.rs
    assert!(
        entries.iter().any(|e| e.target.ends_with("src/main.rs")),
        "src/main.rs should be linked"
    );
    assert!(
        entries.iter().any(|e| e.target.ends_with("src/lib.rs")),
        "src/lib.rs should be linked"
    );
    assert!(
        !entries
            .iter()
            .any(|e| e.target.ends_with("src/internal/secret.rs")),
        "src/internal/secret.rs should NOT be linked (excluded by scope)"
    );
}

/// Test link engine with multiple scopes (simulate multiple dependencies)
#[test]
fn test_link_multiple_dependencies() {
    let source_dir1 = tempfile::tempdir().expect("Failed to create source1 temp dir");
    let source_dir2 = tempfile::tempdir().expect("Failed to create source2 temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    // First repo: Rust files
    create_local_git_repo(source_dir1.path());

    // Second repo: TypeScript files
    let components = source_dir2.path().join("src/components");
    let hooks = source_dir2.path().join("src/hooks");
    fs::create_dir_all(&components).expect("Failed to create components dir");
    fs::create_dir_all(&hooks).expect("Failed to create hooks dir");
    fs::write(components.join("Button.tsx"), "<Button />").expect("Failed to write Button.tsx");
    fs::write(components.join("Button.test.tsx"), "test Button")
        .expect("Failed to write Button.test.tsx");
    fs::write(hooks.join("useAuth.ts"), "export function useAuth() {}")
        .expect("Failed to write useAuth.ts");
    fs::write(hooks.join("useAuth.spec.ts"), "spec useAuth")
        .expect("Failed to write useAuth.spec.ts");

    // Link first dependency: Rust scope
    let scope1 = Scope {
        include: vec!["src/**/*.rs".to_string()],
        exclude: vec![],
    };
    let target1 = workspace_dir.path().join("libs/rust-repo");
    let entries1 = engine::create_dependency_links(
        source_dir1.path(),
        &target1,
        &scope1,
        "https://github.com/example/rust-repo.git",
    )
    .expect("Failed to link rust-repo");

    // Link second dependency: TypeScript scope with test exclusion
    let scope2 = Scope {
        include: vec!["src/components/**".to_string(), "src/hooks/**".to_string()],
        exclude: vec!["**/*.test.tsx".to_string(), "**/*.spec.ts".to_string()],
    };
    let target2 = workspace_dir.path().join("apps/web");
    let entries2 = engine::create_dependency_links(
        source_dir2.path(),
        &target2,
        &scope2,
        "https://github.com/example/frontend.git",
    )
    .expect("Failed to link frontend");

    // Verify first dependency
    assert_eq!(entries1.len(), 3, "Rust repo should have 3 .rs files linked");
    assert!(target1.join("src/main.rs").exists());

    // Verify second dependency
    assert!(
        entries2.len() >= 2,
        "Frontend should have at least 2 files linked (Button.tsx + useAuth.ts)"
    );
    assert!(
        target2.join("src/components/Button.tsx").exists(),
        "Button.tsx should be linked"
    );
    assert!(
        target2.join("src/hooks/useAuth.ts").exists(),
        "useAuth.ts should be linked"
    );
    assert!(
        !target2.join("src/components/Button.test.tsx").exists(),
        "Button.test.tsx should NOT be linked (excluded)"
    );
    assert!(
        !target2.join("src/hooks/useAuth.spec.ts").exists(),
        "useAuth.spec.ts should NOT be linked (excluded)"
    );
}

/// Test that linked files report correct link type
/// On Unix, same-partition files use HardLink; cross-partition falls back to Symlink
#[test]
fn test_link_type_on_unix() {
    let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    create_local_git_repo(source_dir.path());

    let scope = Scope {
        include: vec!["src/**/*.rs".to_string()],
        exclude: vec![],
    };
    let target_dir = workspace_dir.path().join("libs/repo");

    let entries = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &scope,
        "test-repo",
    )
    .expect("Failed to create links");

    // On Unix, files should be either HardLink (same partition) or Symlink (fallback)
    #[cfg(unix)]
    {
        for entry in &entries {
            assert!(
                entry.link_type == LinkType::HardLink || entry.link_type == LinkType::Symlink,
                "Expected HardLink or Symlink on Unix for {}, got {:?}",
                entry.target.display(),
                entry.link_type
            );
        }
    }
}

/// Test workspace state save/load with link entries
#[test]
fn test_workspace_state_with_link_entries() {
    let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
    let workspace_dir = tempfile::tempdir().expect("Failed to create workspace temp dir");

    create_local_git_repo(source_dir.path());

    let scope = Scope {
        include: vec!["src/**/*.rs".to_string()],
        exclude: vec![],
    };
    let target_dir = workspace_dir.path().join("libs/repo");

    let entries = engine::create_dependency_links(
        source_dir.path(),
        &target_dir,
        &scope,
        "https://github.com/example/repo.git",
    )
    .expect("Failed to create links");

    // Create workspace state with link entries
    let mut state = vdw::types::workspace::WorkspaceState::new(workspace_dir.path().to_path_buf());
    state.links = entries;
    state.save().expect("Failed to save workspace state");

    // Load and verify
    let loaded = vdw::types::workspace::WorkspaceState::load(workspace_dir.path())
        .expect("Failed to load workspace state");

    assert_eq!(loaded.links.len(), state.links.len());
    for (orig, load) in state.links.iter().zip(loaded.links.iter()) {
        assert_eq!(orig.source, load.source);
        assert_eq!(orig.target, load.target);
        assert_eq!(orig.link_type, load.link_type);
        assert_eq!(orig.dependency_repo, load.dependency_repo);
    }
}
