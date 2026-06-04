use cfg_if::cfg_if;
use std::path::Path;

/// Link creation strategy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStrategy {
    HardLink,
    Symlink,
    Junction,
}

cfg_if! {
    if #[cfg(target_os = "macos")] {
        pub const PLATFORM: &str = "macos";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::HardLink;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    } else if #[cfg(target_os = "linux")] {
        pub const PLATFORM: &str = "linux";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::HardLink;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    } else if #[cfg(target_os = "windows")] {
        pub const PLATFORM: &str = "windows";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::Junction;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    } else {
        pub const PLATFORM: &str = "unknown";
        pub const DEFAULT_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
        pub const FALLBACK_STRATEGY: LinkStrategy = LinkStrategy::Symlink;
    }
}

/// Check if two paths are on the same filesystem partition
pub fn is_same_partition(path1: &Path, path2: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(m1), Ok(m2)) = (std::fs::metadata(path1), std::fs::metadata(path2)) {
            return m1.dev() == m2.dev();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path1, path2);
    }
    true // assume same partition on non-unix
}

/// Determine the link strategy ONCE for an entire dependency (source_dir → target_dir).
/// Cross-partition is the only case that ALWAYS fails for hardlinks, so we check it here.
/// All other hardlink failures (EDR, exotic FS, permissions) are handled by the per-file
/// fallback in create_single_link, which logs the actual error and falls back to symlink.
pub fn determine_strategy(source_dir: &Path, target_dir: &Path) -> LinkStrategy {
    match DEFAULT_STRATEGY {
        LinkStrategy::HardLink => {
            if is_same_partition(source_dir, target_dir) {
                LinkStrategy::HardLink
            } else {
                FALLBACK_STRATEGY
            }
        }
        other => other,
    }
}

/// Select the best link strategy for a specific file.
/// Uses the pre-determined base strategy; only overrides for directories.
pub fn select_strategy_for_file(source: &Path, base_strategy: LinkStrategy) -> LinkStrategy {
    // Directories always use symlinks (hardlinks don't work on dirs)
    if source.is_dir() {
        return LinkStrategy::Symlink;
    }
    base_strategy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform_constant() {
        #[cfg(target_os = "macos")]
        assert_eq!(PLATFORM, "macos");
        #[cfg(target_os = "linux")]
        assert_eq!(PLATFORM, "linux");
        #[cfg(target_os = "windows")]
        assert_eq!(PLATFORM, "windows");
    }

    #[test]
    fn test_default_strategy_matches_platform() {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(DEFAULT_STRATEGY, LinkStrategy::HardLink);
        #[cfg(target_os = "windows")]
        assert_eq!(DEFAULT_STRATEGY, LinkStrategy::Junction);
    }

    #[test]
    fn test_fallback_strategy_is_symlink() {
        assert_eq!(FALLBACK_STRATEGY, LinkStrategy::Symlink);
    }

    #[test]
    fn test_select_strategy_directory_uses_symlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let sub_dir = dir.path().join("subdir");
        std::fs::create_dir_all(&sub_dir).expect("Failed to create subdir");

        // Directory sources always get Symlink regardless of base strategy
        let strategy = select_strategy_for_file(&sub_dir, LinkStrategy::HardLink);
        assert_eq!(strategy, LinkStrategy::Symlink);
    }

    #[test]
    fn test_determine_strategy_same_partition_prefers_hardlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let source_dir = dir.path().join("source");
        let target_dir = dir.path().join("target");
        std::fs::create_dir_all(&source_dir).expect("Failed to create source dir");
        std::fs::create_dir_all(&target_dir).expect("Failed to create target dir");

        let strategy = determine_strategy(&source_dir, &target_dir);
        // On same partition with hardlink support, should be HardLink
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(strategy, LinkStrategy::HardLink);
        #[cfg(target_os = "windows")]
        assert_eq!(strategy, LinkStrategy::Junction);
    }

    #[test]
    fn test_select_strategy_for_file_passes_through_base() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").expect("Failed to write test file");

        // For files, select_strategy_for_file returns the base strategy as-is
        assert_eq!(select_strategy_for_file(&file_path, LinkStrategy::HardLink), LinkStrategy::HardLink);
        assert_eq!(select_strategy_for_file(&file_path, LinkStrategy::Symlink), LinkStrategy::Symlink);
    }

    #[test]
    fn test_is_same_partition_same_dir() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        assert!(is_same_partition(dir.path(), dir.path()));
    }

    #[test]
    fn test_link_strategy_equality() {
        assert_eq!(LinkStrategy::HardLink, LinkStrategy::HardLink);
        assert_ne!(LinkStrategy::HardLink, LinkStrategy::Symlink);
        assert_ne!(LinkStrategy::Symlink, LinkStrategy::Junction);
    }
}
