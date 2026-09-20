use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use chrono::{Duration, Utc};
use vdw::store::object_pool;
use vdw::types::store::StoreMetadata;

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

fn create_old_store_object(store: &Path, repo: &str) -> std::path::PathBuf {
    let object = object_pool::repo_store_dir(repo, store);
    fs::create_dir_all(&object).unwrap();
    git2::Repository::init_bare(object.join("bare.git")).unwrap();
    let timestamp = Utc::now() - Duration::days(60);
    StoreMetadata {
        repo_url: repo.to_string(),
        commit_hash: "1111111111111111111111111111111111111111".to_string(),
        branch: Some("main".to_string()),
        tag: None,
        reference_count: 0,
        last_accessed: timestamp,
        created_at: timestamp,
        size_bytes: 0,
        alternates_configured: false,
    }
    .save(&object)
    .unwrap();
    object
}

#[test]
fn init_and_store_status_use_configured_store_path() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    let custom_store = temp.path().join("shared-store");
    fs::create_dir_all(home.join(".vdw")).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&custom_store).unwrap();
    let config = serde_json::json!({
        "store_path": custom_store,
        "max_store_size_gb": 1,
        "gc_threshold_days": 30
    });
    fs::write(
        home.join(".vdw/config.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    let init = run_vdw(&home, &workspace, &["init"]);
    assert_success(&init);
    assert!(custom_store.join("objects").is_dir());
    assert!(custom_store.join("cache").is_dir());
    assert!(home.join(".vdw/workspaces.json").is_file());

    let status = run_vdw(&home, &workspace, &["store", "status", "--json"]);
    assert_success(&status);
    let report: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(
        report["store_path"],
        custom_store.to_string_lossy().as_ref()
    );
    assert_eq!(report["workspace_registry_present"], true);
    assert_eq!(report["reference_inventory_complete"], true);
    assert_eq!(report["registered_workspaces"].as_array().unwrap().len(), 1);
}

#[test]
fn store_gc_requires_dry_run_and_never_deletes_candidates() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&workspace).unwrap();
    assert_success(&run_vdw(&home, &workspace, &["init"]));
    let store = home.join(".vdw/store");
    let object = create_old_store_object(&store, "https://example.com/old.git");

    let unsafe_request = run_vdw(&home, &workspace, &["store", "gc"]);
    assert!(!unsafe_request.status.success());
    assert!(String::from_utf8_lossy(&unsafe_request.stderr).contains("plan-only"));

    let dry_run = run_vdw(&home, &workspace, &["store", "gc", "--dry-run", "--json"]);
    assert_success(&dry_run);
    let report: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(report["mode"], "dry_run");
    assert_eq!(report["deletion_performed"], false);
    assert_eq!(report["candidates"].as_array().unwrap().len(), 1);
    assert!(object.exists(), "dry-run must not delete Store objects");
}
