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
    let workspace_root = temp.path().join("workspaces");
    let workspace = workspace_root.join("workspace");
    let custom_store = temp.path().join("shared-store");
    fs::create_dir_all(home.join(".vdw")).unwrap();
    fs::create_dir_all(&workspace_root).unwrap();
    fs::create_dir_all(&custom_store).unwrap();
    let config = serde_json::json!({
        "store_path": custom_store,
        "max_store_size_gb": 1,
        "gc_threshold_days": 30,
        "workspace_root": workspace_root
    });
    fs::write(
        home.join(".vdw/config.json"),
        serde_json::to_vec_pretty(&config).unwrap(),
    )
    .unwrap();

    let init = run_vdw(&home, temp.path(), &["init", "workspace"]);
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
    let workspace_root = temp.path().join("workspaces");
    let workspace = workspace_root.join("workspace");
    fs::create_dir_all(&home).unwrap();
    let workspace_root_arg = workspace_root.to_string_lossy();
    assert_success(&run_vdw(
        &home,
        temp.path(),
        &["config", "set", "workspace-root", &workspace_root_arg],
    ));
    assert_success(&run_vdw(&home, temp.path(), &["init", "workspace"]));
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

#[test]
fn init_requires_workspace_root_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    fs::create_dir_all(&home).unwrap();

    let init = run_vdw(&home, temp.path(), &["init", "workspace"]);

    assert!(!init.status.success());
    assert!(String::from_utf8_lossy(&init.stderr).contains("vdw config set workspace-root <path>"));
    assert!(!home.join(".vdw/workspaces/workspace").exists());
}

#[test]
fn config_set_workspace_root_controls_workspace_creation() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    let workspace_root = temp.path().join("custom-workspaces");
    fs::create_dir_all(&home).unwrap();
    let workspace_root_arg = workspace_root.to_string_lossy();

    let configure = run_vdw(
        &home,
        temp.path(),
        &["config", "set", "workspace-root", &workspace_root_arg],
    );
    assert_success(&configure);
    let init = run_vdw(&home, temp.path(), &["init", "project-one"]);
    assert_success(&init);

    let config_path = workspace_root.join("project-one/.vdwrc");
    assert!(config_path.is_file());
    let workspace_config = fs::read_to_string(config_path).unwrap();
    assert!(workspace_config.contains("name: \"project-one\""));

    let global_config: serde_json::Value =
        serde_json::from_slice(&fs::read(home.join(".vdw/config.json")).unwrap()).unwrap();
    assert_eq!(
        global_config["workspace_root"],
        fs::canonicalize(&workspace_root)
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
}
