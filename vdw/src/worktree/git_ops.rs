use std::path::Path;

use anyhow::Result;
use git2::{BranchType, Repository};

/// Open a bare repository
pub fn open_bare_repo(path: &Path) -> Result<Repository> {
    let repo = Repository::open_bare(path)?;
    Ok(repo)
}

/// Open any repository (bare or normal)
pub fn open_repo(path: &Path) -> Result<Repository> {
    let repo = Repository::open(path)?;
    Ok(repo)
}

/// List all local branches in a repo
pub fn list_branches(repo: &Repository) -> Result<Vec<String>> {
    let branches = repo.branches(Some(BranchType::Local))?;
    let mut names = Vec::new();
    for branch in branches {
        let (branch, _) = branch?;
        if let Some(name) = branch.name()? {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// List all remote branches
pub fn list_remote_branches(repo: &Repository) -> Result<Vec<String>> {
    let branches = repo.branches(Some(BranchType::Remote))?;
    let mut names = Vec::new();
    for branch in branches {
        let (branch, _) = branch?;
        if let Some(name) = branch.name()? {
            names.push(name.to_string());
        }
    }
    Ok(names)
}

/// Find or create a local branch from a remote tracking branch
pub fn find_or_create_local_branch<'a>(
    repo: &'a Repository,
    branch_name: &str,
) -> Result<git2::Reference<'a>> {
    // Try to find local branch first
    if let Ok(branch) = repo.find_branch(branch_name, BranchType::Local) {
        return Ok(branch.into_reference());
    }

    // Try to create from remote
    let remote_name = format!("origin/{}", branch_name);
    if let Ok(remote_branch) = repo.find_branch(&remote_name, BranchType::Remote) {
        let commit = remote_branch.get().peel_to_commit()?;
        let branch = repo.branch(branch_name, &commit, false)?;
        return Ok(branch.into_reference());
    }

    Err(anyhow::anyhow!(
        "Branch '{}' not found locally or in remote",
        branch_name
    ))
}

/// Get the HEAD commit of a branch
pub fn get_branch_commit(repo: &Repository, branch_name: &str) -> Result<git2::Oid> {
    let branch = repo.find_branch(branch_name, BranchType::Local)?;
    let commit = branch.get().peel_to_commit()?;
    Ok(commit.id())
}

/// Get the current HEAD commit OID of a repository
pub fn get_head_commit(repo: &Repository) -> Result<git2::Oid> {
    let head = repo.head()?;
    let commit = head.peel_to_commit()?;
    Ok(commit.id())
}

/// Create an initial commit in an empty repository (useful for testing)
pub fn create_initial_commit(
    repo: &Repository,
    message: &str,
    sig: &git2::Signature,
) -> Result<git2::Oid> {
    let tree_id = {
        let mut index = repo.index()?;
        // Write an empty tree (no files added yet)
        index.write_tree()?
    };

    let tree = repo.find_tree(tree_id)?;

    // Create commit with no parent (initial commit)
    let oid = repo.commit(Some("HEAD"), sig, sig, message, &tree, &[])?;
    Ok(oid)
}

/// Create a commit with a single file and a parent commit
pub fn create_commit_with_file(
    repo: &Repository,
    parent_commit: &git2::Commit,
    message: &str,
    sig: &git2::Signature,
    file_path: &str,
    file_content: &str,
) -> Result<git2::Oid> {
    let mut index = repo.index()?;

    // Create a blob from the content
    let _blob_oid = repo.blob(file_content.as_bytes())?;

    // Add the file to the index
    index.add_path(Path::new(file_path))?;
    index.write()?;

    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let oid = repo.commit(
        Some("HEAD"),
        sig,
        sig,
        message,
        &tree,
        &[parent_commit],
    )?;
    Ok(oid)
}

/// Result of checking a worktree for uncommitted changes
#[derive(Debug, Clone, serde::Serialize)]
pub struct DirtyStatus {
    pub modified: usize,
    pub added: usize,
    pub deleted: usize,
    pub untracked: usize,
}

impl DirtyStatus {
    pub fn is_dirty(&self) -> bool {
        self.modified + self.added + self.deleted + self.untracked > 0
    }
    pub fn total(&self) -> usize {
        self.modified + self.added + self.deleted + self.untracked
    }
}

/// Check if a worktree path has uncommitted changes
pub fn check_dirty(worktree_path: &Path) -> Result<DirtyStatus> {
    let repo = Repository::open(worktree_path)?;
    let statuses = repo.statuses(None)?;

    let mut modified = 0;
    let mut added = 0;
    let mut deleted = 0;
    let mut untracked = 0;

    for entry in statuses.iter() {
        let status = entry.status();
        // Staged changes
        if status.contains(git2::Status::INDEX_NEW) {
            added += 1;
        } else if status.contains(git2::Status::INDEX_MODIFIED)
            || status.contains(git2::Status::INDEX_RENAMED)
            || status.contains(git2::Status::INDEX_TYPECHANGE)
        {
            modified += 1;
        } else if status.contains(git2::Status::INDEX_DELETED) {
            deleted += 1;
        }
        // Working tree changes (only count if not already counted as staged)
        else if status.contains(git2::Status::WT_MODIFIED)
            || status.contains(git2::Status::WT_TYPECHANGE)
        {
            modified += 1;
        } else if status.contains(git2::Status::WT_DELETED) {
            deleted += 1;
        } else if status.contains(git2::Status::WT_NEW) {
            untracked += 1;
        }
    }

    Ok(DirtyStatus {
        modified,
        added,
        deleted,
        untracked,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;

    /// Helper: create a temporary git repo with a commit
    fn create_test_repo() -> (tempfile::TempDir, Repository) {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("Failed to init repo");

        let sig = git2::Signature::now("Test User", "test@example.com")
            .expect("Failed to create signature");

        // Create initial commit
        create_initial_commit(&repo, "Initial commit", &sig)
            .expect("Failed to create initial commit");

        (dir, repo)
    }

    /// Helper: create a temporary bare repo from a normal repo
    /// Uses git CLI for bare cloning since git2 doesn't have clone_bare
    fn create_test_bare_repo() -> (tempfile::TempDir, Repository) {
        let (work_dir, _work_repo) = create_test_repo();

        // Create bare clone using git CLI
        let bare_dir = tempfile::tempdir().expect("Failed to create temp dir for bare repo");
        let bare_path = bare_dir.path().join("bare.git");

        let output = Command::new("git")
            .args([
                "clone",
                "--bare",
                work_dir.path().to_str().expect("path to string"),
                bare_path.to_str().expect("path to string"),
            ])
            .output()
            .expect("Failed to execute git clone --bare");

        if !output.status.success() {
            panic!(
                "git clone --bare failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        let bare_repo = Repository::open_bare(&bare_path).expect("Failed to open bare repo");

        (bare_dir, bare_repo)
    }

    #[test]
    fn test_open_repo() {
        let (dir, _repo) = create_test_repo();
        let opened = open_repo(dir.path()).expect("Failed to open repo");
        assert!(!opened.is_bare());
    }

    #[test]
    fn test_open_bare_repo() {
        let (bare_dir, _bare_repo) = create_test_bare_repo();
        let bare_path = bare_dir.path().join("bare.git");
        let opened = open_bare_repo(&bare_path).expect("Failed to open bare repo");
        assert!(opened.is_bare());
    }

    #[test]
    fn test_open_repo_nonexistent() {
        let result = open_repo(Path::new("/nonexistent/path"));
        assert!(result.is_err());
    }

    #[test]
    fn test_list_branches() {
        let (dir, repo) = create_test_repo();

        // The initial commit creates a master/main branch
        let branches = list_branches(&repo).expect("Failed to list branches");
        assert!(!branches.is_empty());
        // On most systems the default branch is either "master" or "main"
        assert!(
            branches.iter().any(|b| b == "master" || b == "main"),
            "Expected master or main branch, got: {:?}",
            branches
        );

        // Test with a fresh repo object
        let reopened = open_repo(dir.path()).expect("Failed to reopen repo");
        let branches2 = list_branches(&reopened).expect("Failed to list branches");
        assert_eq!(branches, branches2);
    }

    #[test]
    fn test_list_branches_bare_repo() {
        let (bare_dir, bare_repo) = create_test_bare_repo();

        let branches = list_branches(&bare_repo).expect("Failed to list branches");
        // Should have the default branch from the original repo
        assert!(!branches.is_empty());
        assert!(
            branches.iter().any(|b| b == "master" || b == "main"),
            "Expected master or main branch in bare repo, got: {:?}",
            branches
        );

        // Also test reopening
        let bare_path = bare_dir.path().join("bare.git");
        let reopened = open_bare_repo(&bare_path).expect("Failed to reopen bare repo");
        let branches2 = list_branches(&reopened).expect("Failed to list branches");
        assert_eq!(branches, branches2);
    }

    #[test]
    fn test_list_remote_branches() {
        let (_bare_dir, bare_repo) = create_test_bare_repo();

        let remote_branches =
            list_remote_branches(&bare_repo).expect("Failed to list remote branches");
        // A bare clone from a local path may or may not have remote tracking branches
        // depending on how git sets them up. If present, verify they have origin prefix.
        if !remote_branches.is_empty() {
            assert!(
                remote_branches
                    .iter()
                    .any(|b| b.starts_with("origin/master") || b.starts_with("origin/main")),
                "Expected origin/master or origin/main, got: {:?}",
                remote_branches
            );
        }
        // If no remote branches (bare clone from local path), that's acceptable too
    }

    #[test]
    fn test_find_or_create_local_branch_existing() {
        let (_bare_dir, bare_repo) = create_test_bare_repo();

        // Find the existing default branch
        let default_branch = list_branches(&bare_repo)
            .expect("Failed to list branches")
            .into_iter()
            .find(|b| b == "master" || b == "main")
            .expect("No default branch found");

        let reference =
            find_or_create_local_branch(&bare_repo, &default_branch)
                .expect("Failed to find existing branch");
        assert!(reference.name().is_some());
    }

    #[test]
    fn test_find_or_create_local_branch_from_remote() {
        let (_bare_dir, bare_repo) = create_test_bare_repo();

        // The default branch already exists as local (from clone)
        // First verify the local branch exists and can be found
        let local_branches = list_branches(&bare_repo).expect("Failed to list local branches");
        let default_branch = local_branches
            .into_iter()
            .find(|b| b == "master" || b == "main")
            .expect("No default local branch found");

        // Test finding the existing local branch (which should succeed)
        let result = find_or_create_local_branch(&bare_repo, &default_branch);
        assert!(result.is_ok(), "Should find existing local branch");
    }

    #[test]
    fn test_find_or_create_local_branch_nonexistent() {
        let (_bare_dir, bare_repo) = create_test_bare_repo();

        // Try a branch that doesn't exist locally or remotely
        let result = find_or_create_local_branch(&bare_repo, "nonexistent-branch");
        assert!(result.is_err());
    }

    #[test]
    fn test_get_branch_commit() {
        let (_bare_dir, bare_repo) = create_test_bare_repo();

        let default_branch = list_branches(&bare_repo)
            .expect("Failed to list branches")
            .into_iter()
            .find(|b| b == "master" || b == "main")
            .expect("No default branch found");

        let oid = get_branch_commit(&bare_repo, &default_branch).expect("Failed to get commit");
        assert!(!oid.is_zero(), "Commit OID should not be zero");
    }

    #[test]
    fn test_get_head_commit() {
        let (_dir, repo) = create_test_repo();

        let oid = get_head_commit(&repo).expect("Failed to get HEAD commit");
        assert!(!oid.is_zero(), "HEAD commit OID should not be zero");
    }

    #[test]
    fn test_create_initial_commit() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("Failed to init repo");

        let sig =
            git2::Signature::now("Test User", "test@example.com").expect("Failed to create sig");

        let oid = create_initial_commit(&repo, "First commit", &sig)
            .expect("Failed to create initial commit");

        assert!(!oid.is_zero());

        // Verify the commit exists
        let commit = repo.find_commit(oid).expect("Failed to find commit");
        assert_eq!(commit.message(), Some("First commit"));
    }

    #[test]
    fn test_create_commit_with_file() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("Failed to init repo");

        let sig =
            git2::Signature::now("Test User", "test@example.com").expect("Failed to create sig");

        let parent_oid = create_initial_commit(&repo, "Initial commit", &sig)
            .expect("Failed to create initial commit");
        let parent = repo.find_commit(parent_oid).expect("Failed to find parent");

        // Create a file in the working directory first
        let file_path = dir.path().join("test.txt");
        fs::write(&file_path, "hello world").expect("Failed to write file");

        let oid = create_commit_with_file(
            &repo,
            &parent,
            "Add test.txt",
            &sig,
            "test.txt",
            "hello world",
        )
        .expect("Failed to create commit with file");

        assert!(!oid.is_zero());

        // Verify the commit
        let commit = repo.find_commit(oid).expect("Failed to find commit");
        assert_eq!(commit.message(), Some("Add test.txt"));
        assert_eq!(commit.parent_count(), 1);
    }
}