use std::path::{Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use crate::context::glob_matcher;
use crate::types::config::Scope;

/// Filter files in a directory according to scope rules.
/// Returns a sorted list of matching file paths relative to `base_path`.
pub fn filter_files(base_path: &Path, scope: &Scope) -> Result<Vec<PathBuf>> {
    if !base_path.exists() {
        return Ok(Vec::new());
    }

    let mut matched_files = Vec::new();

    for entry in WalkDir::new(base_path)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }

        let rel_path = match entry.path().strip_prefix(base_path) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

        if glob_matcher::matches_scope(&rel_str, &scope.include, &scope.exclude) {
            matched_files.push(rel_path.to_path_buf());
        }
    }

    matched_files.sort();
    Ok(matched_files)
}

/// Get total size in bytes of the given files relative to base_path
pub fn total_size(base_path: &Path, files: &[PathBuf]) -> u64 {
    files
        .iter()
        .filter_map(|f| std::fs::metadata(base_path.join(f)).ok())
        .map(|m| m.len())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_test_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();

        // Create directory structure:
        //   src/main.rs
        //   src/lib.rs
        //   src/mod.rs
        //   src/test_main.rs
        //   src/components/Button.tsx
        //   src/components/Button.test.tsx
        //   README.md
        //   Cargo.toml
        //   tests/integration_test.rs
        //   target/debug/output.o

        let src = dir.path().join("src");
        let components = src.join("components");
        let target_debug = dir.path().join("target/debug");
        let tests_dir = dir.path().join("tests");

        fs::create_dir_all(&components).unwrap();
        fs::create_dir_all(&target_debug).unwrap();
        fs::create_dir_all(&tests_dir).unwrap();

        fs::write(src.join("main.rs"), "fn main() {}").unwrap();
        fs::write(src.join("lib.rs"), "pub fn lib() {}").unwrap();
        fs::write(src.join("mod.rs"), "mod foo;").unwrap();
        fs::write(src.join("test_main.rs"), "#[test] fn test() {}").unwrap();
        fs::write(components.join("Button.tsx"), "<Button />").unwrap();
        fs::write(components.join("Button.test.tsx"), "test Button").unwrap();
        fs::write(dir.path().join("README.md"), "# Test").unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        fs::write(tests_dir.join("integration_test.rs"), "integration").unwrap();
        fs::write(target_debug.join("output.o"), "binary").unwrap();

        dir
    }

    #[test]
    fn test_filter_files_rs_only() {
        let dir = setup_test_dir();
        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let files = filter_files(dir.path(), &scope).unwrap();
        let file_names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();

        assert!(file_names.iter().any(|f| f == "src/main.rs"));
        assert!(file_names.iter().any(|f| f == "src/lib.rs"));
        assert!(file_names.iter().any(|f| f == "src/mod.rs"));
        assert!(file_names.iter().any(|f| f == "src/test_main.rs"));
        assert!(!file_names.iter().any(|f| f.contains("Button")));
        assert!(!file_names.iter().any(|f| f == "README.md"));
    }

    #[test]
    fn test_filter_files_with_exclude() {
        let dir = setup_test_dir();
        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec!["src/**/test_*.rs".to_string()],
        };

        let files = filter_files(dir.path(), &scope).unwrap();
        let file_names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();

        assert!(file_names.iter().any(|f| f == "src/main.rs"));
        assert!(file_names.iter().any(|f| f == "src/lib.rs"));
        assert!(!file_names.iter().any(|f| f.contains("test_main")));
    }

    #[test]
    fn test_filter_files_tsx_exclude_tests() {
        let dir = setup_test_dir();
        let scope = Scope {
            include: vec!["src/**/*.tsx".to_string()],
            exclude: vec!["**/*.test.tsx".to_string()],
        };

        let files = filter_files(dir.path(), &scope).unwrap();
        let file_names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();

        assert!(file_names.iter().any(|f| f.contains("Button.tsx") && !f.contains("test")));
        assert!(!file_names.iter().any(|f| f.contains("test.tsx")));
    }

    #[test]
    fn test_filter_files_nonexistent_path() {
        let scope = Scope {
            include: vec!["**/*.rs".to_string()],
            exclude: vec![],
        };

        let files = filter_files(Path::new("/nonexistent/path"), &scope).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_filter_files_empty_scope() {
        let dir = setup_test_dir();
        let scope = Scope {
            include: vec![],
            exclude: vec![],
        };

        let files = filter_files(dir.path(), &scope).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_filter_files_sorted_output() {
        let dir = setup_test_dir();
        let scope = Scope {
            include: vec!["**/*.rs".to_string()],
            exclude: vec![],
        };

        let files = filter_files(dir.path(), &scope).unwrap();
        let sorted: Vec<_> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
        let mut manual_sort = sorted.clone();
        manual_sort.sort();
        assert_eq!(sorted, manual_sort);
    }

    #[test]
    fn test_total_size() {
        let dir = setup_test_dir();
        let scope = Scope {
            include: vec!["src/**/*.rs".to_string()],
            exclude: vec![],
        };

        let files = filter_files(dir.path(), &scope).unwrap();
        let size = total_size(dir.path(), &files);
        assert!(size > 0);
    }

    #[test]
    fn test_total_size_empty() {
        let dir = setup_test_dir();
        let files: Vec<PathBuf> = vec![];
        let size = total_size(dir.path(), &files);
        assert_eq!(size, 0);
    }
}
