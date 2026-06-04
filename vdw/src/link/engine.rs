use std::path::Path;
use anyhow::Result;
use walkdir::WalkDir;

use crate::link::{hardlink, junction, platform, symlink};
use crate::types::config::Scope;
use crate::types::workspace::{LinkEntry, LinkType};

/// Create links for a single dependency from source_dir to target_dir
/// Only links files that match the scope
pub fn create_dependency_links(
    source_dir: &Path,
    target_dir: &Path,
    scope: &Scope,
    dep_repo: &str,
) -> Result<Vec<LinkEntry>> {
    let mut entries = Vec::new();

    if !source_dir.exists() {
        return Err(anyhow::anyhow!(
            "Source directory does not exist: {}",
            source_dir.display()
        ));
    }

    // Determine link strategy ONCE for the entire dependency
    // (avoids per-file probing that causes random hardlink/symlink inconsistency)
    let base_strategy = platform::determine_strategy(source_dir, target_dir);

    // Walk the source directory and link files matching scope
    for entry in WalkDir::new(source_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let rel_path = entry.path().strip_prefix(source_dir)?;
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        // Check if file matches scope
        if !scope.matches(&rel_str) {
            continue;
        }

        let source_path = entry.path().to_path_buf();
        let target_path = target_dir.join(rel_path);

        // Select strategy for this specific file (usually base_strategy, but dirs get symlink)
        let strategy = platform::select_strategy_for_file(&source_path, base_strategy);
        let actual_link_type = create_single_link(&source_path, &target_path, strategy)?;

        entries.push(LinkEntry {
            source: source_path,
            target: target_path,
            link_type: actual_link_type,
            dependency_repo: dep_repo.to_string(),
        });
    }

    Ok(entries)
}

/// Create a single link using the given strategy.
/// Returns the ACTUAL LinkType used (may differ from strategy if hardlink fell back to symlink).
fn create_single_link(
    source: &Path,
    target: &Path,
    strategy: platform::LinkStrategy,
) -> Result<LinkType> {
    match strategy {
        platform::LinkStrategy::HardLink => {
            match hardlink::create_hardlink(source, target) {
                Ok(()) => Ok(LinkType::HardLink),
                Err(e) => {
                    // Hardlink failed — fall back to symlink and log the real error
                    tracing::warn!(
                        "Hardlink failed for {:?} -> {:?} ({}), falling back to symlink",
                        source, target, e
                    );
                    symlink::create_symlink(source, target)?;
                    Ok(LinkType::Symlink)
                }
            }
        }
        platform::LinkStrategy::Symlink => {
            symlink::create_symlink(source, target)?;
            Ok(LinkType::Symlink)
        }
        platform::LinkStrategy::Junction => {
            junction::create_junction(source, target)?;
            Ok(LinkType::Junction)
        }
    }
}

/// Remove all links tracked in the given entries list
pub fn remove_links(entries: &[LinkEntry]) -> Result<()> {
    for entry in entries {
        match entry.link_type {
            LinkType::HardLink => hardlink::remove_hardlink(&entry.target)?,
            LinkType::Symlink => symlink::remove_symlink(&entry.target)?,
            LinkType::Junction => junction::remove_junction(&entry.target)?,
        }
    }

    Ok(())
}

/// Remove empty parent directories up to (but not including) stop_at
pub fn cleanup_empty_dirs(path: &Path, stop_at: &Path) -> Result<()> {
    let mut current = path.to_path_buf();
    while current != stop_at && current.starts_with(stop_at) {
        if current.is_dir() && std::fs::read_dir(&current)?.next().is_none() {
            std::fs::remove_dir(&current)?;
        } else {
            break;
        }
        current = current
            .parent()
            .unwrap_or(stop_at)
            .to_path_buf();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_source_dir(dir: &std::path::Path) {
        // Create a directory structure:
        //   src/main.rs
        //   src/lib.rs
        //   src/utils/mod.rs
        //   tests/integration.rs
        //   README.md
        let src = dir.join("src");
        let utils = src.join("utils");
        let tests = dir.join("tests");
        std::fs::create_dir_all(&utils).expect("Failed to create utils dir");
        std::fs::create_dir_all(&tests).expect("Failed to create tests dir");

        std::fs::write(src.join("main.rs"), "fn main() {}").expect("Failed to write main.rs");
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}").expect("Failed to write lib.rs");
        std::fs::write(utils.join("mod.rs"), "pub mod utils;").expect("Failed to write mod.rs");
        std::fs::write(tests.join("integration.rs"), "#[test] fn test() {}")
            .expect("Failed to write integration.rs");
        std::fs::write(dir.join("README.md"), "# Test Repo").expect("Failed to write README.md");
    }

    #[test]
    fn test_create_dependency_links_with_scope() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "https://github.com/example/repo.git",
        )
        .expect("Failed to create dependency links");

        // Should link: src/main.rs, src/lib.rs, src/utils/mod.rs
        assert_eq!(entries.len(), 3);

        // All entries should belong to the same repo
        for entry in &entries {
            assert_eq!(entry.dependency_repo, "https://github.com/example/repo.git");
        }

        // Check files exist in target
        assert!(target_dir.path().join("src/main.rs").exists());
        assert!(target_dir.path().join("src/lib.rs").exists());
        assert!(target_dir.path().join("src/utils/mod.rs").exists());
        // These should NOT be linked (don't match scope)
        assert!(!target_dir.path().join("tests/integration.rs").exists());
        assert!(!target_dir.path().join("README.md").exists());
    }

    #[test]
    fn test_create_dependency_links_with_exclude() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec!["src/**/mod.rs".to_string()],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "https://github.com/example/repo.git",
        )
        .expect("Failed to create dependency links");

        // Should link: src/main.rs, src/lib.rs (mod.rs excluded)
        assert_eq!(entries.len(), 2);
        assert!(target_dir.path().join("src/main.rs").exists());
        assert!(target_dir.path().join("src/lib.rs").exists());
        assert!(!target_dir.path().join("src/utils/mod.rs").exists());
    }

    #[test]
    fn test_create_dependency_links_missing_source() {
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        let scope = Scope {
            include: vec!["**/*".to_string()],
            exclude: vec![],
        };

        let result = create_dependency_links(
            Path::new("/nonexistent/path"),
            target_dir.path(),
            &scope,
            "test",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not exist"));
    }

    #[test]
    fn test_remove_links() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "https://github.com/example/repo.git",
        )
        .expect("Failed to create links");

        // Verify files exist
        assert!(target_dir.path().join("src/main.rs").exists());

        // Remove all links
        remove_links(&entries).expect("Failed to remove links");

        // Verify files are gone
        assert!(!target_dir.path().join("src/main.rs").exists());
        assert!(!target_dir.path().join("src/lib.rs").exists());
        assert!(!target_dir.path().join("src/utils/mod.rs").exists());
    }

    #[test]
    fn test_cleanup_empty_dirs() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        // Create nested empty directories
        let nested = dir.path().join("a/b/c");
        std::fs::create_dir_all(&nested).expect("Failed to create nested dir");

        // Create a file in 'a' to prevent it from being removed
        std::fs::write(dir.path().join("a/keep.txt"), "keep").expect("Failed to write keep file");

        cleanup_empty_dirs(&nested, dir.path()).expect("Failed to cleanup");

        // 'c' and 'b' should be removed (empty), 'a' should stay (has keep.txt)
        assert!(!dir.path().join("a/b/c").exists());
        assert!(!dir.path().join("a/b").exists());
        assert!(dir.path().join("a").exists());
        assert!(dir.path().join("a/keep.txt").exists());
    }

    #[test]
    fn test_cleanup_empty_dirs_stops_at_boundary() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");

        let nested = dir.path().join("a/b");
        std::fs::create_dir_all(&nested).expect("Failed to create nested dir");

        // cleanup with stop_at = nested itself should be a no-op
        cleanup_empty_dirs(&nested, &nested).expect("Failed to cleanup");
        assert!(nested.exists());
    }

    #[test]
    fn test_create_dependency_links_empty_scope() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec![],
            exclude: vec![],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "test",
        )
        .expect("Failed to create dependency links");

        // Empty include means nothing matches
        assert!(entries.is_empty());
    }

    #[test]
    fn test_link_entry_has_correct_type() {
        let source_dir = tempfile::tempdir().expect("Failed to create source temp dir");
        let target_dir = tempfile::tempdir().expect("Failed to create target temp dir");

        setup_source_dir(source_dir.path());

        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let entries = create_dependency_links(
            source_dir.path(),
            target_dir.path(),
            &scope,
            "test",
        )
        .expect("Failed to create dependency links");

        // On macOS/Linux with same partition, should use HardLink
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            for entry in &entries {
                assert_eq!(entry.link_type, LinkType::HardLink);
            }
        }
    }
}
