use std::fs;
use std::process::Command;

use vdw::store::object_pool;

#[test]
fn repo_remove_retains_shared_store_entry() {
    let temp = tempfile::tempdir().expect("temp directory");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    let store = home.join(".vdw/store");
    let repo_url = "https://example.com/org/shared.git";

    fs::create_dir_all(&workspace).expect("workspace directory");
    fs::create_dir_all(store.join("objects")).expect("store directory");
    fs::write(
        workspace.join(".vdwrc"),
        format!(
            r#"workspace:
  name: test-workspace
  version: "1.0"
dependencies:
  - remote: "{repo_url}"
    path: packages/shared
    branch: main
    editable: false
    scope:
      include:
        - "**/*"
"#
        ),
    )
    .expect("workspace config");

    let store_entry = object_pool::repo_store_dir(repo_url, &store);
    fs::create_dir_all(&store_entry).expect("store entry");
    fs::write(store_entry.join("sentinel"), "shared").expect("store sentinel");

    let output = Command::new(env!("CARGO_BIN_EXE_vdw"))
        .args(["repo", "remove", repo_url])
        .current_dir(&workspace)
        .env("HOME", &home)
        .output()
        .expect("run vdw repo remove");

    assert!(
        output.status.success(),
        "repo remove failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(store_entry.join("sentinel").exists());

    let config = fs::read_to_string(workspace.join(".vdwrc")).expect("updated config");
    assert!(!config.contains(repo_url));
    assert!(String::from_utf8_lossy(&output.stdout).contains("Shared Store entry retained"));
}
