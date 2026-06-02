use std::path::Path;
use anyhow::Result;

/// Create a hard link from source to target
pub fn create_hardlink(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        return Err(anyhow::anyhow!("Target already exists: {}", target.display()));
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::hard_link(source, target)?;
    Ok(())
}

/// Remove a hard link (just removes the file at target)
pub fn remove_hardlink(target: &Path) -> Result<()> {
    if target.exists() {
        std::fs::remove_file(target)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_hardlink_basic() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");

        std::fs::write(&source, "hello world").expect("Failed to write source");

        create_hardlink(&source, &target).expect("Failed to create hardlink");

        assert!(target.exists());
        let content = std::fs::read_to_string(&target).expect("Failed to read target");
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_create_hardlink_creates_parent_dirs() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("sub/nested/target.txt");

        std::fs::write(&source, "nested content").expect("Failed to write source");

        create_hardlink(&source, &target).expect("Failed to create hardlink");

        assert!(target.exists());
        let content = std::fs::read_to_string(&target).expect("Failed to read target");
        assert_eq!(content, "nested content");
    }

    #[test]
    fn test_create_hardlink_target_exists_fails() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");

        std::fs::write(&source, "source content").expect("Failed to write source");
        std::fs::write(&target, "existing content").expect("Failed to write target");

        let result = create_hardlink(&source, &target);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn test_remove_hardlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");

        std::fs::write(&source, "hello").expect("Failed to write source");
        create_hardlink(&source, &target).expect("Failed to create hardlink");
        assert!(target.exists());

        remove_hardlink(&target).expect("Failed to remove hardlink");
        assert!(!target.exists());
        // Source should still exist
        assert!(source.exists());
    }

    #[test]
    fn test_remove_hardlink_nonexistent_is_ok() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let target = dir.path().join("nonexistent.txt");
        assert!(remove_hardlink(&target).is_ok());
    }

    #[test]
    fn test_hardlink_same_content_after_source_modification() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source = dir.path().join("source.txt");
        let target = dir.path().join("target.txt");

        std::fs::write(&source, "original").expect("Failed to write source");
        create_hardlink(&source, &target).expect("Failed to create hardlink");

        // Modify source - hardlink should see the change (same inode)
        std::fs::write(&source, "modified").expect("Failed to modify source");
        let content = std::fs::read_to_string(&target).expect("Failed to read target");
        assert_eq!(content, "modified");
    }
}
