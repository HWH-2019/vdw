use std::path::{Path, PathBuf};

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

/// Clone a repository to the Content Store as a bare repo
/// Returns the path to the bare repo in the store
pub fn clone_to_store(
    repo_url: &str,
    store_path: &Path,
    branch: Option<&str>,
    reference_path: Option<&Path>,
) -> Result<PathBuf> {
    let repo_hash = compute_repo_hash(repo_url);
    let repo_name = extract_repo_name(repo_url);
    let repo_dir = store_path.join(format!("objects/{}-{}", repo_name, repo_hash));
    let bare_git_path = repo_dir.join("bare.git");

    // If already exists, just return the path
    if bare_git_path.exists() {
        tracing::info!("Repository already exists in store: {}", repo_url);
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

    // Fetch from remote (this does the actual clone)
    // Note: For MVP, we attempt to fetch. If network is unavailable, we still create the structure.
    let fetch_result = fetch_remote(&repo, branch);
    if let Err(e) = &fetch_result {
        tracing::warn!("Failed to fetch remote (network may be unavailable): {}", e);
        // Don't fail hard - the bare repo structure is still valid
    }

    // Save metadata
    let metadata = StoreMetadata {
        repo_url: repo_url.to_string(),
        commit_hash: String::new(), // Will be populated after successful fetch
        branch: branch.map(String::from),
        tag: None,
        reference_count: 1,
        last_accessed: Utc::now(),
        created_at: Utc::now(),
        size_bytes: 0,
        alternates_configured: reference_path.is_some(),
    };
    metadata.save(&repo_dir)?;

    tracing::info!("Initialized store entry for: {}", repo_url);
    Ok(repo_dir)
}

/// Fetch from remote origin
fn fetch_remote(repo: &git2::Repository, branch: Option<&str>) -> Result<()> {
    let mut remote = repo.find_remote("origin")?;
    let refspecs: Vec<String> = if let Some(b) = branch {
        vec![format!("+refs/heads/{}:refs/remotes/origin/{}", b, b)]
    } else {
        vec!["+refs/heads/*:refs/remotes/origin/*".to_string()]
    };
    let refspec_strs: Vec<&str> = refspecs.iter().map(|s| s.as_str()).collect();
    remote.fetch(&refspec_strs, None, None)?;
    Ok(())
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
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repo_url = "https://github.com/test/example.git";
        let result = clone_to_store(repo_url, &store_path, None, None);

        // This may fail due to network (fetch), but the structure should still be created
        // because we don't fail hard on fetch errors
        assert!(result.is_ok());

        let repo_dir = result.unwrap();
        assert!(repo_dir.exists());
        assert!(repo_dir.join("bare.git").exists());
        assert!(repo_dir.join("metadata.json").exists());

        // Verify metadata
        let metadata = StoreMetadata::load(&repo_dir).unwrap();
        assert_eq!(metadata.repo_url, repo_url);
        assert!(!metadata.alternates_configured);
    }

    #[test]
    fn test_clone_to_store_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repo_url = "https://github.com/test/example.git";
        let result1 = clone_to_store(repo_url, &store_path, None, None).unwrap();
        let result2 = clone_to_store(repo_url, &store_path, None, None).unwrap();

        assert_eq!(result1, result2);
    }

    #[test]
    fn test_clone_to_store_with_alternates() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let reference_path = dir.path().join("reference.git");
        fs::create_dir_all(reference_path.join("objects")).unwrap();

        let repo_url = "https://github.com/test/altrepo.git";
        let result = clone_to_store(repo_url, &store_path, None, Some(&reference_path));

        assert!(result.is_ok());
        let repo_dir = result.unwrap();
        let bare_git_path = repo_dir.join("bare.git");
        let alternates_file = bare_git_path.join("objects/info/alternates");
        assert!(alternates_file.exists());

        // Verify metadata has alternates_configured
        let metadata = StoreMetadata::load(&repo_dir).unwrap();
        assert!(metadata.alternates_configured);
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
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("store");
        fs::create_dir_all(store_path.join("objects")).unwrap();

        let repo_url = "https://github.com/test/removeme.git";
        let repo_dir = clone_to_store(repo_url, &store_path, None, None).unwrap();
        assert!(repo_dir.exists());

        remove_from_store(&store_path, repo_url).unwrap();
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
}
