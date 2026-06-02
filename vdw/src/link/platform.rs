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

/// Check if hardlinks are supported at a path (tries creating a test link)
pub fn supports_hardlinks(path: &Path) -> bool {
    let test_file = path.join(".vdw_hl_test");
    let test_link = path.join(".vdw_hl_test_link");

    if std::fs::write(&test_file, "test").is_err() {
        return false;
    }

    let supported = std::fs::hard_link(&test_file, &test_link).is_ok();

    let _ = std::fs::remove_file(&test_file);
    let _ = std::fs::remove_file(&test_link);

    supported
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

/// Select the best link strategy given source and target paths
pub fn select_strategy(source: &Path, target_dir: &Path) -> LinkStrategy {
    // For directories, always use symlinks (hardlinks don't work on dirs)
    if source.is_dir() {
        return LinkStrategy::Symlink;
    }

    match DEFAULT_STRATEGY {
        LinkStrategy::HardLink => {
            if is_same_partition(source, target_dir) && supports_hardlinks(target_dir) {
                LinkStrategy::HardLink
            } else {
                FALLBACK_STRATEGY
            }
        }
        other => other,
    }
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

        let strategy = select_strategy(&sub_dir, dir.path());
        assert_eq!(strategy, LinkStrategy::Symlink);
    }

    #[test]
    fn test_select_strategy_file_prefers_hardlink() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello").expect("Failed to write test file");

        let strategy = select_strategy(&file_path, dir.path());
        // On same partition with hardlink support, should be HardLink
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        assert_eq!(strategy, LinkStrategy::HardLink);
        #[cfg(target_os = "windows")]
        assert_eq!(strategy, LinkStrategy::Junction);
    }

    #[test]
    fn test_supports_hardlinks_on_tempdir() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        // On a normal filesystem, hardlinks should be supported
        #[cfg(unix)]
        assert!(supports_hardlinks(dir.path()));
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
