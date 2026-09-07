use std::fs;
use std::process::Command;

use serde_json::Value;

fn context_workspace() -> tempfile::TempDir {
    let workspace = tempfile::tempdir().expect("workspace");
    fs::create_dir_all(workspace.path().join("packages/service/src")).expect("source directory");
    fs::write(
        workspace.path().join("packages/service/src/lib.rs"),
        "pub fn service() {}",
    )
    .expect("source file");
    fs::write(
        workspace.path().join("packages/service/README.md"),
        "excluded",
    )
    .expect("excluded file");
    fs::write(
        workspace.path().join(".vdwrc"),
        r#"workspace:
  name: context-cli-test
  version: "1.0"
dependencies:
  - remote: "https://example.com/org/service.git"
    path: packages/service
    branch: main
    editable: true
    scope:
      include:
        - "src/**/*.rs"
"#,
    )
    .expect("workspace config");
    workspace
}

fn run_context(workspace: &tempfile::TempDir, command: &str) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_vdw"))
        .args(["context", command, "--json"])
        .current_dir(workspace.path())
        .output()
        .expect("run vdw context command");

    assert!(
        output.status.success(),
        "context command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("JSON-only stdout")
}

#[test]
fn context_preview_json_is_workspace_relative() {
    let workspace = context_workspace();
    let report = run_context(&workspace, "preview");

    assert_eq!(report["workspace_name"], "context-cli-test");
    assert_eq!(report["total_files"], 1);
    assert_eq!(report["repositories"][0]["linked"], true);
    assert_eq!(
        report["repositories"][0]["files"][0],
        "packages/service/src/lib.rs"
    );
}

#[test]
fn context_stats_json_reports_counts_and_bytes() {
    let workspace = context_workspace();
    let report = run_context(&workspace, "stats");

    assert_eq!(report["workspace_name"], "context-cli-test");
    assert_eq!(report["total_files"], 1);
    assert!(report["total_size_bytes"].as_u64().unwrap_or_default() > 0);
    assert_eq!(report["per_dependency"][0]["file_count"], 1);
}
