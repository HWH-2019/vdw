use std::fs;

use vdw::store::{init_store_at, GlobalConfig};
use vdw::types::store::StoreMetadata;
use chrono::Utc;

/// Initialize store in a temp directory and verify directory structure
#[test]
fn test_init_store_creates_directory_structure() {
    let dir = tempfile::tempdir().unwrap();
    let vdw_home = init_store_at(dir.path()).expect("Failed to init store");

    // Verify all expected directories are created
    assert!(vdw_home.join("store/objects").exists(), "store/objects should exist");
    assert!(vdw_home.join("store/objects").is_dir(), "store/objects should be a directory");
    assert!(vdw_home.join("store/cache").exists(), "store/cache should exist");
    assert!(vdw_home.join("store/cache").is_dir(), "store/cache should be a directory");
    assert!(vdw_home.join("logs").exists(), "logs should exist");
    assert!(vdw_home.join("logs").is_dir(), "logs should be a directory");

    // Verify config.json is created
    let config_path = vdw_home.join("config.json");
    assert!(config_path.exists(), "config.json should exist");

    // Verify config.json content is valid
    let content = fs::read_to_string(&config_path).expect("Failed to read config.json");
    let config: GlobalConfig = serde_json::from_str(&content).expect("Failed to parse config.json");
    assert_eq!(config.store_path, "~/.vdw/store");
    assert_eq!(config.max_store_size_gb, 50);
    assert_eq!(config.gc_threshold_days, 30);
}

/// Test metadata save/load roundtrip
#[test]
fn test_metadata_roundtrip_integration() {
    let dir = tempfile::tempdir().unwrap();
    let _vdw_home = init_store_at(dir.path()).expect("Failed to init store");

    // Create a subdirectory for the repo metadata
    let repo_dir = dir.path().join("store/objects/frontend-abc12345");
    fs::create_dir_all(&repo_dir).expect("Failed to create repo dir");

    let original = StoreMetadata {
        repo_url: "https://github.com/example/frontend.git".to_string(),
        commit_hash: "a1b2c3d4e5f6".to_string(),
        branch: Some("main".to_string()),
        tag: None,
        reference_count: 1,
        last_accessed: Utc::now(),
        created_at: Utc::now(),
        size_bytes: 4096,
        alternates_configured: false,
    };

    // Save
    original.save(&repo_dir).expect("Failed to save metadata");
    assert!(repo_dir.join("metadata.json").exists(), "metadata.json should exist after save");

    // Load
    let loaded = StoreMetadata::load(&repo_dir).expect("Failed to load metadata");

    // Verify roundtrip
    assert_eq!(loaded.repo_url, original.repo_url);
    assert_eq!(loaded.commit_hash, original.commit_hash);
    assert_eq!(loaded.branch, original.branch);
    assert_eq!(loaded.tag, original.tag);
    assert_eq!(loaded.reference_count, original.reference_count);
    assert_eq!(loaded.size_bytes, original.size_bytes);
    assert_eq!(loaded.alternates_configured, original.alternates_configured);
}

/// Test that initializing the store twice is idempotent
#[test]
fn test_init_store_idempotent() {
    let dir = tempfile::tempdir().unwrap();

    // First init
    let home1 = init_store_at(dir.path()).expect("First init failed");
    assert!(home1.join("store/objects").exists());

    // Second init should not fail
    let home2 = init_store_at(dir.path()).expect("Second init failed");
    assert!(home2.join("store/objects").exists());

    // Same base path
    assert_eq!(home1, home2);
}

/// Test that existing config.json is preserved on re-init
#[test]
fn test_init_store_preserves_existing_config() {
    let dir = tempfile::tempdir().unwrap();

    // First init
    init_store_at(dir.path()).expect("First init failed");

    // Modify config.json
    let config_path = dir.path().join("config.json");
    let custom_config = r#"{
        "store_path": "/custom/store",
        "max_store_size_gb": 200,
        "gc_threshold_days": 90
    }"#;
    fs::write(&config_path, custom_config).expect("Failed to write custom config");

    // Second init should not overwrite
    init_store_at(dir.path()).expect("Second init failed");

    let content = fs::read_to_string(&config_path).expect("Failed to read config");
    let config: GlobalConfig = serde_json::from_str(&content).expect("Failed to parse config");
    assert_eq!(config.store_path, "/custom/store");
    assert_eq!(config.max_store_size_gb, 200);
    assert_eq!(config.gc_threshold_days, 90);
}

/// Test metadata increment/decrement reference count
#[test]
fn test_metadata_reference_count_operations() {
    let dir = tempfile::tempdir().unwrap();
    let repo_dir = dir.path().join("repo");
    fs::create_dir_all(&repo_dir).unwrap();

    let mut metadata = StoreMetadata {
        repo_url: "https://github.com/example/repo.git".to_string(),
        commit_hash: "abc123".to_string(),
        branch: Some("main".to_string()),
        tag: None,
        reference_count: 1,
        last_accessed: Utc::now(),
        created_at: Utc::now(),
        size_bytes: 1024,
        alternates_configured: true,
    };

    // Increment
    metadata.increment_reference();
    assert_eq!(metadata.reference_count, 2);

    // Save and reload
    metadata.save(&repo_dir).unwrap();
    let mut loaded = StoreMetadata::load(&repo_dir).unwrap();
    assert_eq!(loaded.reference_count, 2);

    // Decrement
    loaded.decrement_reference();
    assert_eq!(loaded.reference_count, 1);

    // Decrement to zero
    loaded.decrement_reference();
    assert_eq!(loaded.reference_count, 0);

    // Decrement below zero should stay at zero
    loaded.decrement_reference();
    assert_eq!(loaded.reference_count, 0);
}

/// Test creating multiple repo entries in the store
#[test]
fn test_multiple_repo_entries_in_store() {
    let dir = tempfile::tempdir().unwrap();
    init_store_at(dir.path()).expect("Failed to init store");

    let objects_dir = dir.path().join("store/objects");

    // Create two repo entries
    for (name, url) in [
        ("frontend-abc12345", "https://github.com/example/frontend.git"),
        ("backend-def67890", "https://github.com/example/backend.git"),
    ] {
        let repo_dir = objects_dir.join(name);
        fs::create_dir_all(&repo_dir).unwrap();

        let metadata = StoreMetadata {
            repo_url: url.to_string(),
            commit_hash: "deadbeef".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            reference_count: 1,
            last_accessed: Utc::now(),
            created_at: Utc::now(),
            size_bytes: 2048,
            alternates_configured: false,
        };
        metadata.save(&repo_dir).unwrap();
    }

    // Verify both exist
    assert!(objects_dir.join("frontend-abc12345/metadata.json").exists());
    assert!(objects_dir.join("backend-def67890/metadata.json").exists());

    // Load and verify
    let m1 = StoreMetadata::load(&objects_dir.join("frontend-abc12345")).unwrap();
    assert_eq!(m1.repo_url, "https://github.com/example/frontend.git");

    let m2 = StoreMetadata::load(&objects_dir.join("backend-def67890")).unwrap();
    assert_eq!(m2.repo_url, "https://github.com/example/backend.git");
}
