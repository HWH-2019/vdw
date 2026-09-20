use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use vdw::lockfile::WorkspaceLock;
use vdw::types::config::{Dependency, Scope, VDWConfig, WorkspaceMeta};

fn commit_version(repo_path: &Path, content: &str) -> git2::Oid {
    let repo = git2::Repository::open(repo_path)
        .or_else(|_| git2::Repository::init(repo_path))
        .expect("source repository");
    fs::write(repo_path.join("version.txt"), content).expect("version file");
    let mut index = repo.index().expect("source index");
    index
        .add_path(Path::new("version.txt"))
        .expect("stage version file");
    index.write().expect("write source index");
    let tree_id = index.write_tree().expect("source tree");
    let tree = repo.find_tree(tree_id).expect("find source tree");
    let signature = git2::Signature::now("VDW Test", "vdw@example.com").unwrap();
    let parent = repo
        .find_reference("refs/heads/main")
        .ok()
        .and_then(|reference| reference.peel_to_commit().ok());
    let parents = parent.iter().collect::<Vec<_>>();
    let commit = repo
        .commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            content,
            &tree,
            &parents,
        )
        .expect("source commit");
    repo.set_head("refs/heads/main").expect("source HEAD");
    commit
}

fn write_config(workspace: &Path, source: &Path) -> String {
    let config = VDWConfig {
        workspace: WorkspaceMeta {
            name: "sync-test".to_string(),
            version: "1.0".to_string(),
        },
        dependencies: vec![Dependency {
            remote: String::new(),
            origin: String::new(),
            local: source.to_string_lossy().into_owned(),
            repo: String::new(),
            path: "packages/source".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["**/*".to_string()],
                exclude: Vec::new(),
            },
            editable: false,
        }],
    };
    let content = serde_yaml::to_string(&config).expect("serialize config");
    fs::write(workspace.join(".vdwrc"), &content).expect("workspace config");
    content
}

fn run_vdw(home: &Path, workspace: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_vdw"))
        .args(args)
        .current_dir(workspace)
        .env("HOME", home)
        .output()
        .expect("run vdw")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "vdw failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sync_dry_run_and_frozen_link_keep_revisions_reproducible() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    let source = temp.path().join("source");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&source).unwrap();

    let first = commit_version(&source, "first");
    let original_config = write_config(&workspace, &source);

    let initial_sync = run_vdw(&home, &workspace, &["sync", "--update-lock", "--json"]);
    assert_success(&initial_sync);
    let initial_report: serde_json::Value =
        serde_json::from_slice(&initial_sync.stdout).expect("initial sync report");
    assert_eq!(initial_report["mode"], "update_lock");
    assert_eq!(initial_report["dependencies"][0]["status"], "added");
    let initial_lock_content = fs::read_to_string(workspace.join(".vdw.lock")).unwrap();
    let initial_lock = WorkspaceLock::load_required(&workspace).unwrap();
    assert_eq!(
        initial_lock.dependencies[0].resolved_commit,
        first.to_string()
    );

    let second = commit_version(&source, "second");
    let dry_run = run_vdw(&home, &workspace, &["sync", "--dry-run", "--json"]);
    assert_success(&dry_run);
    let dry_report: serde_json::Value =
        serde_json::from_slice(&dry_run.stdout).expect("dry-run sync report");
    assert_eq!(dry_report["mode"], "dry_run");
    assert_eq!(dry_report["dependencies"][0]["status"], "updated");
    assert_eq!(
        dry_report["dependencies"][0]["resolved_commit"],
        second.to_string()
    );
    assert_eq!(
        fs::read_to_string(workspace.join(".vdw.lock")).unwrap(),
        initial_lock_content
    );

    let frozen_old = run_vdw(&home, &workspace, &["link", "--frozen"]);
    assert_success(&frozen_old);
    assert_eq!(
        fs::read_to_string(workspace.join("packages/source/version.txt")).unwrap(),
        "first"
    );

    let update = run_vdw(&home, &workspace, &["sync", "--update-lock"]);
    assert_success(&update);
    let updated_lock = WorkspaceLock::load_required(&workspace).unwrap();
    assert_eq!(
        updated_lock.dependencies[0].resolved_commit,
        second.to_string()
    );

    let frozen_new = run_vdw(&home, &workspace, &["link", "--frozen"]);
    assert_success(&frozen_new);
    assert_eq!(
        fs::read_to_string(workspace.join("packages/source/version.txt")).unwrap(),
        "second"
    );
    assert_eq!(
        fs::read_to_string(workspace.join(".vdwrc")).unwrap(),
        original_config,
        "sync and link must not persist runtime paths in .vdwrc"
    );

    fs::write(
        workspace.join(".vdwrc"),
        "workspace:\n  name: sync-test\n  version: '1.0'\ndependencies: []\n",
    )
    .unwrap();
    let removed = run_vdw(&home, &workspace, &["sync", "--dry-run", "--json"]);
    assert_success(&removed);
    let removed_report: serde_json::Value =
        serde_json::from_slice(&removed.stdout).expect("removed dependency report");
    assert_eq!(
        removed_report["removed_dependencies"][0]["path"],
        "packages/source"
    );
    assert_eq!(
        WorkspaceLock::load_required(&workspace)
            .unwrap()
            .dependencies
            .len(),
        1,
        "dry-run must preserve removed lock entries"
    );

    let accept_removal = run_vdw(&home, &workspace, &["sync", "--update-lock"]);
    assert_success(&accept_removal);
    assert!(WorkspaceLock::load_required(&workspace)
        .unwrap()
        .dependencies
        .is_empty());
}

#[test]
fn frozen_link_without_lock_fails_before_touching_workspace_content() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    let source = temp.path().join("source");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(workspace.join("packages/source")).unwrap();
    fs::create_dir_all(&source).unwrap();
    commit_version(&source, "first");
    write_config(&workspace, &source);
    let sentinel = workspace.join("packages/source/keep.txt");
    fs::write(&sentinel, "keep").unwrap();

    let output = run_vdw(&home, &workspace, &["link", "--frozen"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(".vdw.lock is required"));
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "keep");
    assert!(!workspace.join(".vdw/state.json").exists());
}

#[test]
fn frozen_link_does_not_rewrite_legacy_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    let legacy_config = r#"workspace:
  name: legacy-test
  version: "1.0"
dependencies:
  - remote: "https://github.com/example/repo.git"
    local: "/tmp/legacy-managed-worktree"
    path: packages/repo
    branch: main
    editable: true
    scope:
      include:
        - "**/*"
"#;
    fs::write(workspace.join(".vdwrc"), legacy_config).unwrap();

    let output = run_vdw(&home, &workspace, &["link", "--frozen"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("legacy managed worktree paths"));
    assert_eq!(
        fs::read_to_string(workspace.join(".vdwrc")).unwrap(),
        legacy_config
    );
}
