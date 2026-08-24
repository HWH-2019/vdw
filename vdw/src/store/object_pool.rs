use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::store::alternates;
use crate::types::store::StoreMetadata;

/// Compute a unique hash for a repository URL (first 8 hex chars of SHA256)
pub fn compute_repo_hash(repo_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_url.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)[..8].to_string()
}

/// Resolve the stable Store object directory for a repository identifier.
pub fn repo_store_dir(repo_url: &str, store_path: &Path) -> PathBuf {
    let repo_hash = compute_repo_hash(repo_url);
    let repo_name = extract_repo_name(repo_url);
    store_path.join(format!("objects/{}-{}", repo_name, repo_hash))
}

/// Clone a repository to the Content Store as a bare repo
/// Returns the path to the bare repo in the store
pub fn clone_to_store(
    repo_url: &str,
    store_path: &Path,
    branch: Option<&str>,
    reference_path: Option<&Path>,
) -> Result<PathBuf> {
    let repo_dir = repo_store_dir(repo_url, store_path);
    let bare_git_path = repo_dir.join("bare.git");

    // If already exists, ensure the requested branch is fetched, then return
    if bare_git_path.exists() {
        tracing::info!("Repository already exists in store: {}", repo_url);

        // If a specific branch is requested and not yet fetched, fetch it
        if let Some(b) = branch {
            let remote_ref = bare_git_path.join("refs/remotes/origin").join(b);
            if !remote_ref.exists() {
                tracing::info!("Branch '{}' not yet fetched for existing store entry, fetching...", b);
                let repo = git2::Repository::open_bare(&bare_git_path)?;
                if let Err(e) = fetch_with_timeout(&repo, branch, 90) {
                    tracing::warn!("Failed to fetch branch '{}' for existing store: {}", b, e);
                }
            }
        }

        // Fix HEAD for repos cloned with older versions (git2 defaults to master)
        fix_bare_repo_head(&bare_git_path, branch);
        return Ok(repo_dir);
    }

    // Create the directory
    std::fs::create_dir_all(&repo_dir)?;

    // Initialize bare repository using git2
    let repo = git2::Repository::init_bare(&bare_git_path)?;

    // Add remote "origin"
    repo.remote("origin", repo_url)?;

    // If reference path provided, setup alternates
    if let Some(ref_path) = reference_path {
        alternates::setup_alternates(&bare_git_path, ref_path)?;
    }

    // Fetch from remote with timeout (prevents hanging on network/SSH issues)
    let fetch_result = fetch_with_timeout(&repo, branch, 90);

    // Calculate size and determine commit hash after fetching
    let size = calculate_dir_size(&bare_git_path);
    let commit_hash = get_remote_head(&repo, branch).unwrap_or_default();

    // After fetch: validate that the repo actually has content
    match &fetch_result {
        Ok(()) => {
            tracing::info!("Successfully fetched repository: {}", repo_url);
        }
        Err(e) => {
            // If fetch failed AND we got no remote tracking refs, return a clear error
            if commit_hash.is_empty() {
                anyhow::bail!(
                    "Failed to clone repository '{}': {}\n\
                     Please check:\n\
                     - Network connectivity\n\
                     - SSH key authentication (if using git@ URL)\n\
                     - Repository URL is correct and accessible",
                    repo_url, e
                );
            }
            // If we have content despite fetch failure (e.g. from alternates), just warn
            tracing::warn!(
                "Fetch warning (repo has content from alternates): {}",
                e
            );
        }
    }

    // Save metadata with actual values
    let metadata = StoreMetadata {
        repo_url: repo_url.to_string(),
        commit_hash,
        branch: branch.map(String::from),
        tag: None,
        reference_count: 1,
        last_accessed: Utc::now(),
        created_at: Utc::now(),
        size_bytes: size,
        alternates_configured: reference_path.is_some(),
    };
    metadata.save(&repo_dir)?;

    // Fix HEAD: git2::Repository::init_bare() defaults HEAD to refs/heads/master,
    // but the repo may use a different default branch (e.g. main).
    // After fetch, update HEAD to point to the fetched branch so that
    // `git worktree add` and other operations work correctly.
    fix_bare_repo_head(&bare_git_path, branch);

    tracing::info!("Initialized store entry for: {}", repo_url);
    Ok(repo_dir)
}

/// Get the remote tracking HEAD commit hash after fetching
fn get_remote_head(repo: &git2::Repository, branch: Option<&str>) -> Option<String> {
    // If a specific branch was requested, check its remote tracking branch
    if let Some(b) = branch {
        let ref_name = format!("refs/remotes/origin/{}", b);
        if let Ok(reference) = repo.find_reference(&ref_name) {
            if let Some(oid) = reference.target() {
                return Some(oid.to_string());
            }
        }
        return None;
    }

    // No specific branch: try common default branch names
    for candidate in &[
        "refs/remotes/origin/main",
        "refs/remotes/origin/master",
        "refs/remotes/origin/HEAD",
    ] {
        if let Ok(reference) = repo.find_reference(candidate) {
            if let Some(oid) = reference.target() {
                return Some(oid.to_string());
            }
        }
    }
    None
}

/// Fix the bare repo's HEAD to point to the correct branch.
/// git2::Repository::init_bare() defaults HEAD to refs/heads/master,
/// but the repo may use a different default branch (e.g. main).
/// This causes `git worktree add` to fail with "invalid reference: HEAD".
fn fix_bare_repo_head(bare_git_path: &Path, branch: Option<&str>) {
    let head_path = bare_git_path.join("HEAD");

    // Determine the correct branch to point HEAD at
    let target_branch = if let Some(b) = branch {
        // Only use this branch if its ref actually exists
        let remote_ref = bare_git_path.join("refs/remotes/origin").join(b);
        let local_ref = bare_git_path.join("refs/heads").join(b);
        if !remote_ref.exists() && !local_ref.exists() {
            tracing::warn!(
                "Branch '{}' not found in bare repo refs, skipping HEAD update",
                b
            );
            return;
        }
        b.to_string()
    } else {
        // Auto-detect: check which remote tracking refs exist
        let refs_dir = bare_git_path.join("refs").join("remotes").join("origin");
        if refs_dir.join("main").exists() {
            "main".to_string()
        } else if refs_dir.join("master").exists() {
            "master".to_string()
        } else {
            // Can't detect, leave HEAD as-is
            return;
        }
    };

    // Write HEAD to point to the correct branch
    let head_content = format!("ref: refs/heads/{}\n", target_branch);
    if let Err(e) = std::fs::write(&head_path, head_content) {
        tracing::warn!("Failed to fix bare repo HEAD: {}", e);
    } else {
        tracing::info!("Fixed bare repo HEAD to refs/heads/{}", target_branch);
    }

    // Also ensure the local branch ref exists (copy from remote tracking ref)
    let remote_ref = bare_git_path.join("refs/remotes/origin").join(&target_branch);
    let local_ref = bare_git_path.join("refs/heads").join(&target_branch);
    if remote_ref.exists() && !local_ref.exists() {
        if let Some(parent) = local_ref.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(&remote_ref, &local_ref) {
            tracing::warn!("Failed to create local branch ref from remote: {}", e);
        }
    }
}

/// Calculate the total size of a directory recursively
fn calculate_dir_size(path: &Path) -> u64 {
    fn walk(dir: &Path, total: &mut u64) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        walk(&path, total);
                    } else if ft.is_file() || ft.is_symlink() {
                        if let Ok(meta) = std::fs::metadata(&path) {
                            *total += meta.len();
                        }
                    }
                }
            }
        }
    }
    let mut total = 0;
    walk(path, &mut total);
    total
}

/// Fetch from remote origin using system `git fetch` (delegates SSH auth to system OpenSSH).
/// This avoids libgit2/libssh2 transport compatibility issues with certain Git servers.
fn fetch_remote(repo: &git2::Repository, branch: Option<&str>) -> Result<()> {
    let bare_git_path = repo.path().to_path_buf();

    // Check if origin is a local path (not a URL)
    let origin_url = get_origin_url(repo);
    let is_local_origin = origin_url
        .as_deref()
        .map(|u| u.starts_with('/') || u.starts_with('.') || u.starts_with('~'))
        .unwrap_or(false);

    let mut cmd = std::process::Command::new("git");
    cmd.args(["fetch", "origin"]);

    if let Some(b) = branch {
        cmd.arg(format!("+refs/heads/{}:refs/remotes/origin/{}", b, b));
    } else {
        cmd.arg("+refs/heads/*:refs/remotes/origin/*");
    }

    // Use hardened SSH options: BatchMode=yes (fail fast, no prompts), ConnectTimeout (network limit),
    // StrictHostKeyChecking=accept-new (auto-accept new hosts, same as normal git)
    cmd.env(
        "GIT_SSH_COMMAND",
        "ssh -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new",
    );
    cmd.current_dir(&bare_git_path);

    tracing::info!(
        "Fetching from remote using system git (branch: {:?}, cwd: {})",
        branch,
        bare_git_path.display()
    );
    eprintln!("[GIT FETCH] {} origin {}", if branch.is_some() { "+refs/heads/<branch>:refs/remotes/origin/<branch>" } else { "+refs/heads/*:refs/remotes/origin/*" }, bare_git_path.display());

    let output = cmd
        .output()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!(
                    "git command not found. Please install Git and ensure it's in your PATH."
                )
            } else {
                anyhow::anyhow!("Failed to execute git fetch: {}", e)
            }
        })?;

    if output.status.success() {
        tracing::info!("git fetch completed successfully");
        eprintln!("[GIT FETCH] SUCCESS");

        // For local origins, also fetch remote tracking branches (refs/remotes/origin/*).
        // A local repo's remote tracking branches are NOT fetched by default,
        // but they may be the only source of branches the user wants.
        if is_local_origin {
            fetch_local_origin_remote_refs(&bare_git_path, branch);
        }

        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git fetch failed: {}", stderr.trim());
        eprintln!("[GIT FETCH] FAILED: {}", stderr.trim());

        // If fetching a specific branch failed with "couldn't find remote ref",
        // the branch may only exist as a remote tracking branch in the source repo.
        // Fall back to fetching all branches.
        if branch.is_some() && stderr.contains("couldn't find remote ref") {
            tracing::info!("Branch not found as refs/heads, falling back to fetch all branches");
            eprintln!("[GIT FETCH] Retrying with all branches...");
            return fetch_remote(repo, None);
        }

        Err(anyhow::anyhow!(
            "git fetch failed: {}",
            stderr.trim()
        ))
    }
}

/// Get the origin remote URL from a repository
fn get_origin_url(repo: &git2::Repository) -> Option<String> {
    repo.find_remote("origin")
        .ok()
        .and_then(|r| r.url().map(String::from))
}

/// For local origins, also fetch refs/remotes/origin/* from the source repo.
/// This ensures branches that only exist as remote tracking branches in the source
/// are also available in our bare repo.
fn fetch_local_origin_remote_refs(bare_git_path: &Path, branch: Option<&str>) {
    let mut cmd = std::process::Command::new("git");
    cmd.args(["fetch", "origin"]);

    if let Some(b) = branch {
        // Try fetching the specific branch from the source's remote tracking refs
        cmd.arg(format!("+refs/remotes/origin/{}:refs/remotes/origin/{}", b, b));
    } else {
        // Fetch all remote tracking branches from the source
        cmd.arg("+refs/remotes/origin/*:refs/remotes/origin/*");
    }

    cmd.env(
        "GIT_SSH_COMMAND",
        "ssh -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new",
    );
    cmd.current_dir(bare_git_path);

    tracing::info!("Fetching remote tracking branches from local origin");
    match cmd.output() {
        Ok(output) if output.status.success() => {
            tracing::info!("Fetched remote tracking branches from local origin");
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!("Fetch remote tracking branches skipped: {}", stderr.trim());
        }
        Err(e) => {
            tracing::debug!("Fetch remote tracking branches failed (non-fatal): {}", e);
        }
    }
}

/// Fetch from remote with a timeout to prevent hanging on network/SSH issues
fn fetch_with_timeout(repo: &git2::Repository, branch: Option<&str>, timeout_secs: u64) -> Result<()> {
    let bare_git_path = repo.path().to_path_buf();
    let branch_owned = branch.map(|s| s.to_string());

    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        // Re-open the bare repo in the spawned thread
        let repo = match git2::Repository::open_bare(&bare_git_path) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(anyhow::anyhow!("Failed to open bare repo: {}", e)));
                return;
            }
        };

        tracing::debug!("Fetch thread started (timeout: {}s)", timeout_secs);
        let result = fetch_remote(&repo, branch_owned.as_deref());
        let _ = tx.send(result);
    });

    match rx.recv_timeout(Duration::from_secs(timeout_secs)) {
        Ok(result) => {
            tracing::debug!("Fetch completed within timeout");
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            anyhow::bail!(
                "Operation timed out after {} seconds.\n\
                 Please check:\n\
                 - Network connectivity (can you reach the Git server?)\n\
                 - SSH key is loaded (ssh-add -l)\n\
                 - Repository URL is correct",
                timeout_secs
            )
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("Fetch thread disconnected unexpectedly")
        }
    }
}

/// Extract repo name from URL (e.g., "frontend" from "git@github.com:org/frontend.git")
fn extract_repo_name(repo_url: &str) -> String {
    repo_url
        .rsplit('/')
        .next()
        .unwrap_or("repo")
        .trim_end_matches(".git")
        .to_string()
}

/// List all repos in the store
pub fn list_store_repos(store_path: &Path) -> Result<Vec<(String, StoreMetadata)>> {
    let objects_dir = store_path.join("objects");
    if !objects_dir.exists() {
        return Ok(vec![]);
    }

    let mut repos = vec![];
    for entry in std::fs::read_dir(&objects_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let metadata_path = entry.path().join("metadata.json");
            if metadata_path.exists() {
                if let Ok(metadata) = StoreMetadata::load(&entry.path()) {
                    repos.push((entry.file_name().to_string_lossy().to_string(), metadata));
                }
            }
        }
    }

    Ok(repos)
}

/// Remove a repo from the store
pub fn remove_from_store(store_path: &Path, repo_url: &str) -> Result<()> {
    let repo_hash = compute_repo_hash(repo_url);
    let repo_name = extract_repo_name(repo_url);
    let repo_dir = store_path.join(format!("objects/{}-{}", repo_name, repo_hash));

    if repo_dir.exists() {
        std::fs::remove_dir_all(&repo_dir)?;
        tracing::info!("Removed from store: {}", repo_url);
    } else {
        tracing::warn!("Repository not found in store: {}", repo_url);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Create a local source repo with an initial commit for testing
    fn create_source_repo() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let repo = git2::Repository::init(dir.path()).expect("Failed to init repo");

        let sig = git2::Signature::now("Test", "test@test.com").expect("Failed to create sig");

        // Create a file and commit
        fs::write(dir.path().join("README.md"), "# Test Repo").expect("Failed to write README");

        let mut index = repo.index().expect("Failed to get index");
        index.add_path(std::path::Path::new("README.md")).expect("Failed to add path");
        index.write().expect("Failed to write index");

        let tree_id = index.write_tree().expect("Failed to write tree");
        let tree = repo.find_tree(tree_id).expect("Failed to find tree");

        repo.commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .expect("Failed to create commit");

        let url = format!("file://{}", dir.path().display());
        (dir, url)
    }

    #[test]
    fn test_compute_repo_hash_deterministic() {
        let hash1 = compute_repo_hash("https://github.com/org/repo.git");
        let hash2 = compute_repo_hash("https://github.com/org/repo.git");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 8);
    }

    #[test]
    fn test_compute_repo_hash_different_urls() {
        let hash1 = compute_repo_hash("https://github.com/org/repo1.git");
        let hash2 = compute_repo_hash("https://github.com/org/repo2.git");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_compute_repo_hash_format() {
        let hash = compute_repo_hash("https://github.com/org/repo.git");
        // Should be 8 hex characters
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_extract_repo_name_https() {
        assert_eq!(
            extract_repo_name("https://github.com/org/frontend.git"),
            "frontend"
        );
    }

    #[test]
    fn test_extract_repo_name_ssh() {
        assert_eq!(
            extract_repo_name("git@github.com:org/frontend.git"),
            "frontend"
        );
    }

    #[test]
    fn test_extract_repo_name_no_git_suffix() {
        assert_eq!(
            extract_repo_name("https://github.com/org/frontend"),
            "frontend"
        );
    }

    #[test]
    fn test_extract_repo_name_simple() {
        assert_eq!(extract_repo_name("repo"), "repo");
    }

    #[test]
    fn test_clone_to_store_creates_structure() {
        let (_source_dir, source_url) = create_source_repo();

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let result = clone_to_store(&source_url, &store_path, None, None);
        assert!(result.is_ok(), "clone_to_store should succeed with local repo: {:?}", result.err());

        let repo_dir = result.unwrap();
        assert!(repo_dir.exists());
        assert!(repo_dir.join("bare.git").exists());
        assert!(repo_dir.join("metadata.json").exists());

        // Verify metadata has actual values
        let metadata = StoreMetadata::load(&repo_dir).unwrap();
        assert_eq!(metadata.repo_url, source_url);
        assert!(!metadata.commit_hash.is_empty(), "commit_hash should be populated after fetch");
        assert!(metadata.size_bytes > 0, "size_bytes should be > 0 after fetch");
        assert!(!metadata.alternates_configured);
    }

    #[test]
    fn test_clone_to_store_idempotent() {
        let (_source_dir, source_url) = create_source_repo();

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let result1 = clone_to_store(&source_url, &store_path, None, None).unwrap();
        let result2 = clone_to_store(&source_url, &store_path, None, None).unwrap();

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_clone_to_store_with_alternates() {
        let (_source_dir, source_url) = create_source_repo();

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let reference_path = dir.path().join("reference.git");
        // Init reference as a bare repo with objects directory
        let _ref_repo = git2::Repository::init_bare(&reference_path).unwrap();

        let result = clone_to_store(&source_url, &store_path, None, Some(&reference_path));

        assert!(result.is_ok(), "clone_to_store with alternates should succeed: {:?}", result.err());
        let repo_dir = result.unwrap();
        let bare_git_path = repo_dir.join("bare.git");
        let alternates_file = bare_git_path.join("objects/info/alternates");
        assert!(alternates_file.exists());

        // Verify metadata has alternates_configured
        let metadata = StoreMetadata::load(&repo_dir).unwrap();
        assert!(metadata.alternates_configured);
    }

    #[test]
    fn test_clone_to_store_fails_on_unreachable() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repo_url = "https://github.com/nonexistent-org-12345/repo.git";
        let result = clone_to_store(repo_url, &store_path, None, None);

        assert!(result.is_err(), "Should fail on unreachable repository");
        let err = format!("{}", result.err().unwrap());
        assert!(err.contains("Failed to clone repository"), "Error should mention cloning failure");
    }

    #[test]
    fn test_list_store_repos_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repos = list_store_repos(&store_path).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn test_list_store_repos_with_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        // Manually create a repo entry with metadata
        let repo_dir = store_path.join("objects/test-abc12345");
        fs::create_dir_all(&repo_dir).unwrap();
        let metadata = StoreMetadata {
            repo_url: "https://github.com/test/repo.git".to_string(),
            commit_hash: "abc123".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            reference_count: 1,
            last_accessed: Utc::now(),
            created_at: Utc::now(),
            size_bytes: 1024,
            alternates_configured: false,
        };
        metadata.save(&repo_dir).unwrap();

        let repos = list_store_repos(&store_path).unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].0, "test-abc12345");
        assert_eq!(repos[0].1.repo_url, "https://github.com/test/repo.git");
    }

    #[test]
    fn test_remove_from_store() {
        let (_source_dir, source_url) = create_source_repo();

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repo_dir = clone_to_store(&source_url, &store_path, None, None).unwrap();
        assert!(repo_dir.exists());

        remove_from_store(&store_path, &source_url).unwrap();
        assert!(!repo_dir.exists());
    }

    #[test]
    fn test_remove_from_store_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        // Should not fail on nonexistent repo
        let result = remove_from_store(&store_path, "https://github.com/test/nonexistent.git");
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_remote_head_with_branch() {
        let (_source_dir, source_url) = create_source_repo();

        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repo_dir = clone_to_store(&source_url, &store_path, None, None).unwrap();
        let bare_git_path = repo_dir.join("bare.git");
        let repo = git2::Repository::open_bare(&bare_git_path).unwrap();

        let hash = get_remote_head(&repo, None);
        assert!(hash.is_some(), "Should find a remote head commit");
        assert!(!hash.unwrap().is_empty());
    }

    #[test]
    fn test_calculate_dir_size_returns_zero_for_empty() {
        let dir = tempfile::tempdir().unwrap();
        let size = calculate_dir_size(dir.path());
        assert_eq!(size, 0);
    }

    #[test]
    fn test_calculate_dir_size_returns_positive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("file.txt"), "content").unwrap();
        let size = calculate_dir_size(dir.path());
        assert!(size > 0);
    }

    /// Integration test: clone a real GitLab repository using SSH.
    /// Requires SSH access to gitlab.alibaba-inc.com with proper key loaded.
    /// Run with: cargo test -- --ignored test_clone_gitlab_repo
    #[test]
    #[ignore]
    fn test_clone_gitlab_repo() {
        let repo_url = "git@gitlab.alibaba-inc.com:eleme-domPoi/alsc.sales_aoi_service.git";
        let branch = Some("master");

        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).expect("Failed to create objects dir");

        eprintln!("\n=== Integration test: cloning real GitLab repo ===");
        eprintln!("URL:    {}", repo_url);
        eprintln!("Branch: {:?}", branch);
        eprintln!("Store:  {}", store_path.display());
        eprintln!("SSH_AUTH_SOCK: {:?}", std::env::var("SSH_AUTH_SOCK").ok());
        eprintln!("---");

        let result = clone_to_store(repo_url, &store_path, branch, None);

        assert!(result.is_ok(), "Failed to clone real GitLab repo: {:?}", result.err());

        let repo_dir = result.unwrap();
        let metadata = StoreMetadata::load(&repo_dir).expect("Failed to load metadata");

        eprintln!("---");
        eprintln!("Clone succeeded!");
        eprintln!("  Commit hash: {}", metadata.commit_hash);
        eprintln!("  Size:        {} bytes ({:.2} MB)", metadata.size_bytes, metadata.size_bytes as f64 / 1_048_576.0);
        eprintln!("  Branch:      {:?}", metadata.branch);
        eprintln!("  Repo URL:    {}", metadata.repo_url);

        assert!(!metadata.commit_hash.is_empty(), "commit_hash should be populated after cloning");
        assert!(metadata.size_bytes > 0, "size_bytes should be > 0 after cloning");
        assert_eq!(metadata.repo_url, repo_url, "repo_url should match the original URL");

        // Verify the bare repo has a local branch created (via fetch refspec)
        let bare_git_path = repo_dir.join("bare.git");
        let bare_repo = git2::Repository::open_bare(&bare_git_path)
            .expect("Failed to open bare repo for verification");
        let remote_ref = format!("refs/remotes/origin/{}", branch.unwrap());
        assert!(
            bare_repo.find_reference(&remote_ref).is_ok(),
            "Remote tracking ref '{}' should exist after fetch",
            remote_ref
        );

        eprintln!("=== Integration test passed ===\n");
    }
}
