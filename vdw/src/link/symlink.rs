use std::path::Path;
use anyhow::Result;

/// Create a symlink from source to target
/// If target already exists (as symlink or regular file), it will be removed first (idempotent).
pub fn create_symlink(source: &Path, target: &Path) -> Result<()> {
    // Remove existing target first (idempotent: allows re-linking without errors)
    if target.symlink_metadata().is_ok() {
        std::fs::remove_file(target)?;
    } else if target.exists() {
        std::fs::remove_file(target)?;
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
    fn test_create_symlink_idempotent() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("link.txt");

        std::fs::write(&source, "source content").expect("Failed to write source");

        // First link succeeds
        create_symlink(&source, &target).expect("First link should succeed");
        assert!(target.exists());

        // Second link on same target should succeed (idempotent)
        let result = create_symlink(&source, &target);
        assert!(result.is_ok(), "Second link should succeed (idempotent): {:?}", result.err());
        assert!(target.exists());

        let content = std::fs::read_to_string(&target).expect("Failed to read target");
        assert_eq!(content, "source content");
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
