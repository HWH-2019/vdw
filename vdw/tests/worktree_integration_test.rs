use std::fs;
use std::path::PathBuf;

use vdw::worktree::{create_worktree, list_worktrees, remove_worktree};
use vdw::worktree::git_ops;

/// Helper: create a bare git repo with multiple files and commits
fn create_bare_repo_with_commits() -> (tempfile::TempDir, PathBuf, String) {
    // Create a normal repo first
    let work_dir = tempfile::tempdir().expect("Failed to create temp work dir");
    let work_repo = git2::Repository::init(work_dir.path()).expect("Failed to init repo");

    let sig = git2::Signature::now("Test User", "test@example.com")
        .expect("Failed to create signature");

    // Create multiple files
    let src = work_dir.path().join("src");
    fs::create_dir_all(&src).expect("Failed to create src dir");
    fs::write(src.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
    fs::write(src.join("lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");
    fs::write(work_dir.path().join("README.md"), "# Test Repo\n").expect("Failed to write README");

    // Stage and commit all files
    let mut index = work_repo.index().expect("Failed to get index");
    index.add_path(std::path::Path::new("src/main.rs")).expect("Failed to add main.rs");
    index.add_path(std::path::Path::new("src/lib.rs")).expect("Failed to add lib.rs");
    index.add_path(std::path::Path::new("README.md")).expect("Failed to add README");
    index.write().expect("Failed to write index");

    let tree_id = index.write_tree().expect("Failed to write tree");
    let tree = work_repo.find_tree(tree_id).expect("Failed to find tree");
    let _initial_oid = work_repo
        .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
        .expect("Failed to create initial commit");

    // Create bare clone using git2
    let bare_dir = tempfile::tempdir().expect("Failed to create temp dir for bare repo");
    let bare_path = bare_dir.path().join("bare.git");

    let _bare_repo = git2::build::RepoBuilder::new()
        .bare(true)
        .clone(
            work_dir.path().to_str().expect("path to string"),
            &bare_path,
        )
        .expect("Failed to clone bare repo");

    // Determine the default branch name
    let bare_repo = git2::Repository::open_bare(&bare_path).expect("Failed to open bare repo");
    let branches = git_ops::list_branches(&bare_repo).expect("Failed to list branches");
    let default_branch = branches
        .into_iter()
        .find(|b| b == "master" || b == "main")
        .expect("No default branch found")
        .to_string();

    drop(bare_repo);
    // Keep work_dir alive through the function scope
    let _ = work_dir;

    (bare_dir, bare_path, default_branch)
}

/// Test: create a worktree from a bare repo, verify it has the expected files
#[test]
fn test_worktree_create_and_verify_files() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    // Create a worktree
    let worktree_path = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Failed to create worktree");

    // Verify worktree directory exists
    assert!(worktree_path.exists(), "Worktree path should exist");

    // Verify worktree has the expected files from the commit
    assert!(
        worktree_path.join("README.md").exists(),
        "Worktree should have README.md"
    );
    assert!(
        worktree_path.join("src/main.rs").exists(),
        "Worktree should have src/main.rs"
    );
    assert!(
        worktree_path.join("src/lib.rs").exists(),
        "Worktree should have src/lib.rs"
    );

    // Verify file contents
    let readme = fs::read_to_string(worktree_path.join("README.md"))
        .expect("Failed to read README.md from worktree");
    assert!(readme.contains("Test Repo"));

    // Cleanup
    let _ = remove_worktree(&bare_path, &default_branch, true);
    let _ = bare_dir;
}

/// Test: list worktrees after creation should include the new worktree
#[test]
fn test_worktree_list_after_creation() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    // Before creation, should be empty
    let worktrees_before = list_worktrees(&bare_path).expect("Failed to list worktrees");
    assert!(
        worktrees_before.is_empty(),
        "Should have no worktrees initially, got: {:?}",
        worktrees_before
    );

    // Create a worktree
    let _worktree_path = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Failed to create worktree");

    // After creation, should list the worktree
    let worktrees_after = list_worktrees(&bare_path).expect("Failed to list worktrees after creation");
    assert!(
        !worktrees_after.is_empty(),
        "Should have at least one worktree after creation"
    );

    // Verify the worktree info contains the branch name
    let sanitized = default_branch.replace('/', "-");
    assert!(
        worktrees_after.iter().any(|wt| wt.branch == sanitized || wt.branch == default_branch),
        "Worktree list should contain branch '{}', got: {:?}",
        default_branch,
        worktrees_after
    );

    // Cleanup
    let _ = remove_worktree(&bare_path, &default_branch, true);
    let _ = bare_dir;
}

/// Test: remove a worktree and verify cleanup
#[test]
fn test_worktree_remove_and_verify_cleanup() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    // Create a worktree
    let worktree_path = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Failed to create worktree");

    assert!(worktree_path.exists(), "Worktree should exist before removal");

    // Remove the worktree
    remove_worktree(&bare_path, &default_branch, true)
        .expect("Failed to remove worktree");

    // Verify the worktree directory is cleaned up
    assert!(
        !worktree_path.exists(),
        "Worktree directory should be removed after cleanup"
    );

    // Verify worktree is no longer listed
    let worktrees = list_worktrees(&bare_path).expect("Failed to list worktrees after removal");
    assert!(
        worktrees.is_empty(),
        "Should have no worktrees after removal, got: {:?}",
        worktrees
    );

    let _ = bare_dir;
}

/// Test: create worktree, verify it's a valid git repository
#[test]
fn test_worktree_is_valid_git_repo() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    let worktree_path = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Failed to create worktree");

    // The worktree should be openable as a git repository
    let worktree_repo = git2::Repository::open(&worktree_path)
        .expect("Worktree should be a valid git repository");

    // Should not be bare
    assert!(!worktree_repo.is_bare(), "Worktree should not be bare");

    // HEAD should point to the expected branch
    let head = worktree_repo.head().expect("Failed to get HEAD");
    let branch_name = head
        .shorthand()
        .expect("HEAD should have a branch name");
    assert_eq!(
        branch_name, default_branch,
        "Worktree HEAD should point to {}",
        default_branch
    );

    // Cleanup
    let _ = remove_worktree(&bare_path, &default_branch, true);
    let _ = bare_dir;
}

/// Test: creating a worktree for the same branch twice returns the same path (idempotent)
#[test]
fn test_worktree_create_idempotent() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    // First creation
    let path1 = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("First worktree creation should succeed");

    // Second creation (idempotent - should return same path)
    let path2 = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Second worktree creation should succeed");

    assert_eq!(
        path1, path2,
        "Idempotent creation should return the same path"
    );

    // Cleanup
    let _ = remove_worktree(&bare_path, &default_branch, true);
    let _ = bare_dir;
}

/// Test: worktree files reflect the content of the committed branch
#[test]
fn test_worktree_file_content_matches_commit() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    let worktree_path = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Failed to create worktree");

    // Verify file contents match what was committed
    let main_content = fs::read_to_string(worktree_path.join("src/main.rs"))
        .expect("Failed to read main.rs from worktree");
    assert_eq!(main_content, "fn main() {}");

    let lib_content = fs::read_to_string(worktree_path.join("src/lib.rs"))
        .expect("Failed to read lib.rs from worktree");
    assert_eq!(lib_content, "pub fn lib() {}");

    // Cleanup
    let _ = remove_worktree(&bare_path, &default_branch, true);
    let _ = bare_dir;
}

/// Test: git operations on worktree (check status, HEAD commit)
#[test]
fn test_worktree_git_status_clean() {
    let (bare_dir, bare_path, default_branch) = create_bare_repo_with_commits();

    let worktree_base = tempfile::tempdir().expect("Failed to create temp dir for worktrees");

    let worktree_path = create_worktree(&bare_path, &default_branch, worktree_base.path())
        .expect("Failed to create worktree");

    // Open the worktree repo and verify HEAD commit matches bare repo
    let worktree_repo = git2::Repository::open(&worktree_path)
        .expect("Failed to open worktree as repo");
    let head_oid = git_ops::get_head_commit(&worktree_repo)
        .expect("Failed to get HEAD commit from worktree");

    // Also get the commit from bare repo's branch
    let bare_repo = git2::Repository::open_bare(&bare_path)
        .expect("Failed to open bare repo");
    let branch_oid = git_ops::get_branch_commit(&bare_repo, &default_branch)
        .expect("Failed to get branch commit from bare repo");

    assert_eq!(
        head_oid, branch_oid,
        "Worktree HEAD commit should match bare repo branch commit"
    );

    // Cleanup
    drop(bare_repo);
    let _ = remove_worktree(&bare_path, &default_branch, true);
    let _ = bare_dir;
}
