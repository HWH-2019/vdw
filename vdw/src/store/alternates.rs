use std::fs;
use std::path::Path;

use anyhow::Result;

/// Configure Git alternates to make a repo reference a shared object pool
pub fn setup_alternates(repo_git_path: &Path, reference_path: &Path) -> Result<()> {
    let alternates_file = repo_git_path.join("objects/info/alternates");
    let alternates_dir = alternates_file.parent().unwrap();

    fs::create_dir_all(alternates_dir)?;

    let reference_objects_path = reference_path.join("objects");
    fs::write(
        &alternates_file,
        reference_objects_path.to_string_lossy().as_ref(),
    )?;

    tracing::info!(
        "Configured alternates: {} -> {}",
        repo_git_path.display(),
        reference_objects_path.display()
    );

    Ok(())
}

/// Remove Git alternates configuration
pub fn remove_alternates(repo_git_path: &Path) -> Result<()> {
    let alternates_file = repo_git_path.join("objects/info/alternates");

    if alternates_file.exists() {
        fs::remove_file(&alternates_file)?;
        tracing::info!("Removed alternates: {}", repo_git_path.display());
    }

    Ok(())
}

/// Validate alternates paths are still valid
pub fn validate_alternates(repo_git_path: &Path) -> Result<bool> {
    let alternates_file = repo_git_path.join("objects/info/alternates");

    if !alternates_file.exists() {
        return Ok(true); // No alternates is valid
    }

    let content = fs::read_to_string(&alternates_file)?;
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let path = std::path::PathBuf::from(line);
        if !path.exists() || !path.is_dir() {
            return Ok(false);
        }
    }

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_alternates_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");
        let reference_path = dir.path().join("reference.git");

        fs::create_dir_all(&repo_git_path).unwrap();
        fs::create_dir_all(reference_path.join("objects")).unwrap();

        setup_alternates(&repo_git_path, &reference_path).unwrap();

        let alternates_file = repo_git_path.join("objects/info/alternates");
        assert!(alternates_file.exists());

        let content = fs::read_to_string(alternates_file).unwrap();
        let expected = reference_path.join("objects").to_string_lossy().to_string();
        assert_eq!(content, expected);
    }

    #[test]
    fn test_setup_alternates_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");
        let reference_path = dir.path().join("reference.git");

        fs::create_dir_all(&repo_git_path).unwrap();
        fs::create_dir_all(reference_path.join("objects")).unwrap();

        setup_alternates(&repo_git_path, &reference_path).unwrap();

        let other_reference = dir.path().join("other.git");
        fs::create_dir_all(other_reference.join("objects")).unwrap();

        setup_alternates(&repo_git_path, &other_reference).unwrap();

        let alternates_file = repo_git_path.join("objects/info/alternates");
        let content = fs::read_to_string(alternates_file).unwrap();
        let expected = other_reference.join("objects").to_string_lossy().to_string();
        assert_eq!(content, expected);
    }

    #[test]
    fn test_remove_alternates_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");

        fs::create_dir_all(repo_git_path.join("objects/info")).unwrap();
        fs::write(
            repo_git_path.join("objects/info/alternates"),
            "/some/path/objects",
        )
        .unwrap();

        remove_alternates(&repo_git_path).unwrap();

        let alternates_file = repo_git_path.join("objects/info/alternates");
        assert!(!alternates_file.exists());
    }

    #[test]
    fn test_remove_alternates_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");
        fs::create_dir_all(&repo_git_path).unwrap();

        // Should not fail on missing file
        remove_alternates(&repo_git_path).unwrap();
    }

    #[test]
    fn test_validate_alternates_no_file() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");
        fs::create_dir_all(&repo_git_path).unwrap();

        // No alternates file means valid
        assert!(validate_alternates(&repo_git_path).unwrap());
    }

    #[test]
    fn test_validate_alternates_valid_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");
        let reference_path = dir.path().join("reference.git");

        fs::create_dir_all(&repo_git_path).unwrap();
        fs::create_dir_all(reference_path.join("objects")).unwrap();

        setup_alternates(&repo_git_path, &reference_path).unwrap();

        assert!(validate_alternates(&repo_git_path).unwrap());
    }

    #[test]
    fn test_validate_alternates_invalid_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");

        // Create alternates file pointing to a non-existent path
        fs::create_dir_all(repo_git_path.join("objects/info")).unwrap();
        fs::write(
            repo_git_path.join("objects/info/alternates"),
            "/nonexistent/path/objects",
        )
        .unwrap();

        assert!(!validate_alternates(&repo_git_path).unwrap());
    }

    #[test]
    fn test_validate_alternates_with_comments_and_empty_lines() {
        let dir = tempfile::tempdir().unwrap();
        let repo_git_path = dir.path().join("repo.git");
        let reference_path = dir.path().join("reference.git");

        fs::create_dir_all(&repo_git_path).unwrap();
        fs::create_dir_all(reference_path.join("objects")).unwrap();

        // Write alternates with comments and empty lines
        fs::create_dir_all(repo_git_path.join("objects/info")).unwrap();
        let ref_obj_path = reference_path.join("objects").to_string_lossy().to_string();
        let content = format!("# comment\n\n{}\n", ref_obj_path);
        fs::write(repo_git_path.join("objects/info/alternates"), content).unwrap();

        assert!(validate_alternates(&repo_git_path).unwrap());
    }
}
