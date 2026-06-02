use std::path::Path;
use anyhow::Result;

/// Create a symlink from source to target
pub fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    if target.exists() || target.symlink_metadata().is_ok() {
        return Err(anyhow::anyhow!("Target already exists: {}", target.display()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    #[cfg(unix)]
    std::os::unix::fs::symlink(source, target)?;

    #[cfg(windows)]
    {
        if source.is_dir() {
            std::os::windows::fs::symlink_dir(source, target)?;
        } else {
            std::os::windows::fs::symlink_file(source, target)?;
        }
    }

    Ok(())
}

/// Remove a symlink
pub fn remove_symlink(target: &Path) -> Result<()> {
    if target.symlink_metadata().is_ok() {
        #[cfg(unix)]
        std::fs::remove_file(target)?;

        #[cfg(windows)]
        {
            if target.is_dir() {
                std::fs::remove_dir(target)?;
            } else {
                std::fs::remove_file(target)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_symlink_file() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("link.txt");

        std::fs::write(&source, "symlink content").expect("Failed to write source");

        create_symlink(&source, &target).expect("Failed to create symlink");

        assert!(target.exists());
        let content = std::fs::read_to_string(&target).expect("Failed to read target");
        assert_eq!(content, "symlink content");
    }

    #[test]
    fn test_create_symlink_creates_parent_dirs() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("sub/nested/link.txt");

        std::fs::write(&source, "nested symlink").expect("Failed to write source");

        create_symlink(&source, &target).expect("Failed to create symlink");

        assert!(target.exists());
        let content = std::fs::read_to_string(&target).expect("Failed to read target");
        assert_eq!(content, "nested symlink");
    }

    #[test]
    fn test_create_symlink_target_exists_fails() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("link.txt");

        std::fs::write(&source, "source").expect("Failed to write source");
        std::fs::write(&target, "existing").expect("Failed to write target");

        let result = create_symlink(&source, &target);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_remove_symlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("link.txt");

        std::fs::write(&source, "hello").expect("Failed to write source");
        create_symlink(&source, &target).expect("Failed to create symlink");
        assert!(target.exists());

        remove_symlink(&target).expect("Failed to remove symlink");
        assert!(!target.exists());
        // Source should still exist
        assert!(source.exists());
    }

    #[test]
    fn test_remove_symlink_nonexistent_is_ok() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let target = dir.path().join("nonexistent.txt");
        assert!(remove_symlink(&target).is_ok());
    }

    #[test]
    fn test_symlink_is_symbolic_link() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("link.txt");

        std::fs::write(&source, "content").expect("Failed to write source");
        create_symlink(&source, &target).expect("Failed to create symlink");

        let metadata = target.symlink_metadata().expect("Failed to get metadata");
        assert!(metadata.file_type().is_symlink());
    }
}
