use std::fs;
use std::process::Command;

use vdw::store::alternates;

/// Helper: create a minimal bare git repo using `git init --bare`
fn create_bare_git_repo(path: &std::path::Path) {
    fs::create_dir_all(path).expect("Failed to create dir for bare repo");
    let output = Command::new("git")
        .args(["init", "--bare"])
        .arg(path)
        .output()
        .expect("Failed to run git init --bare");
    if !output.status.success() {
        panic!(
            "git init --bare failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Setup alternates pointing to a temp bare repo and validate
#[test]
fn test_setup_and_validate_alternates() {
    let dir = tempfile::tempdir().unwrap();

    // Create a bare reference repo with actual git objects
    let reference_path = dir.path().join("reference.git");
    create_bare_git_repo(&reference_path);
    assert!(reference_path.join("objects").exists());

    // Create a repo that will use alternates
    let repo_git_path = dir.path().join("repo.git");
    create_bare_git_repo(&repo_git_path);

    // Setup alternates
    alternates::setup_alternates(&repo_git_path, &reference_path)
        .expect("Failed to setup alternates");

    // Verify alternates file exists
    let alternates_file = repo_git_path.join("objects/info/alternates");
    assert!(alternates_file.exists(), "alternates file should exist");

    // Verify content points to reference objects
    let content = fs::read_to_string(&alternates_file).expect("Failed to read alternates");
    let expected = reference_path.join("objects").to_string_lossy().to_string();
    assert_eq!(content, expected);

    // Validate alternates - should be valid
    let is_valid = alternates::validate_alternates(&repo_git_path)
        .expect("Failed to validate alternates");
    assert!(is_valid, "Alternates should be valid");
}

/// Remove alternates and verify cleanup
#[test]
fn test_remove_alternates_and_verify_cleanup() {
    let dir = tempfile::tempdir().unwrap();

    // Create repos
    let reference_path = dir.path().join("reference.git");
    create_bare_git_repo(&reference_path);

    let repo_git_path = dir.path().join("repo.git");
    create_bare_git_repo(&repo_git_path);

    // Setup alternates
    alternates::setup_alternates(&repo_git_path, &reference_path).unwrap();
    let alternates_file = repo_git_path.join("objects/info/alternates");
    assert!(alternates_file.exists(), "alternates file should exist after setup");

    // Remove alternates
    alternates::remove_alternates(&repo_git_path).expect("Failed to remove alternates");

    // Verify file is gone
    assert!(!alternates_file.exists(), "alternates file should be removed after cleanup");

    // Validate should still return true (no alternates is valid)
    let is_valid = alternates::validate_alternates(&repo_git_path)
        .expect("Failed to validate after removal");
    assert!(is_valid, "No alternates should be considered valid");
}

/// Test that validate returns false when alternates points to non-existent path
#[test]
fn test_validate_alternates_broken_reference() {
    let dir = tempfile::tempdir().unwrap();

    let repo_git_path = dir.path().join("repo.git");
    create_bare_git_repo(&repo_git_path);

    // Create a reference, setup alternates, then delete the reference
    let reference_path = dir.path().join("temp-reference.git");
    create_bare_git_repo(&reference_path);

    alternates::setup_alternates(&repo_git_path, &reference_path).unwrap();

    // Delete the reference repo objects directory to break the link
    fs::remove_dir_all(&reference_path).expect("Failed to remove reference repo");

    // Validate should return false
    let is_valid = alternates::validate_alternates(&repo_git_path)
        .expect("Failed to validate broken alternates");
    assert!(!is_valid, "Alternates with broken reference should be invalid");
}

/// Test re-setting alternates to a different reference
#[test]
fn test_reassign_alternates() {
    let dir = tempfile::tempdir().unwrap();

    // Create two reference repos
    let reference1_path = dir.path().join("reference1.git");
    let reference2_path = dir.path().join("reference2.git");
    create_bare_git_repo(&reference1_path);
    create_bare_git_repo(&reference2_path);

    let repo_git_path = dir.path().join("repo.git");
    create_bare_git_repo(&repo_git_path);

    // Setup with first reference
    alternates::setup_alternates(&repo_git_path, &reference1_path).unwrap();
    let content1 = fs::read_to_string(repo_git_path.join("objects/info/alternates")).unwrap();
    assert!(content1.contains("reference1"));

    // Reassign to second reference
    alternates::setup_alternates(&repo_git_path, &reference2_path).unwrap();
    let content2 = fs::read_to_string(repo_git_path.join("objects/info/alternates")).unwrap();
    assert!(content2.contains("reference2"));
    assert!(!content2.contains("reference1"));

    // Should still validate
    let is_valid = alternates::validate_alternates(&repo_git_path).unwrap();
    assert!(is_valid);
}

/// Test removing alternates when no alternates file exists
#[test]
fn test_remove_alternates_when_none_exists() {
    let dir = tempfile::tempdir().unwrap();

    let repo_git_path = dir.path().join("repo.git");
    create_bare_git_repo(&repo_git_path);

    // Should not fail
    alternates::remove_alternates(&repo_git_path).expect("Should not fail when no alternates file");

    // Validate should return true
    let is_valid = alternates::validate_alternates(&repo_git_path).unwrap();
    assert!(is_valid, "No alternates file is valid");
}

/// Test alternates with comments and empty lines
#[test]
fn test_validate_alternates_with_comments() {
    let dir = tempfile::tempdir().unwrap();

    let reference_path = dir.path().join("reference.git");
    create_bare_git_repo(&reference_path);

    let repo_git_path = dir.path().join("repo.git");
    create_bare_git_repo(&repo_git_path);

    // Manually write alternates with comments
    fs::create_dir_all(repo_git_path.join("objects/info")).unwrap();
    let ref_obj_path = reference_path.join("objects").to_string_lossy().to_string();
    let content = format!("# This is a comment\n\n{}\n# Another comment\n", ref_obj_path);
    fs::write(repo_git_path.join("objects/info/alternates"), content).unwrap();

    // Should validate successfully
    let is_valid = alternates::validate_alternates(&repo_git_path).unwrap();
    assert!(is_valid, "Alternates with comments should be valid");
}
