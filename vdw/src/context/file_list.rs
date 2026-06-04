use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use parking_lot::RwLock;

use crate::context::filter;
use crate::types::config::VDWConfig;

/// Context statistics for display
#[derive(Debug, Clone)]
pub struct ContextStats {
    pub total_files: usize,
    pub total_size_bytes: u64,
    pub per_dependency: Vec<DependencyStats>,
}

/// Statistics for a single dependency
#[derive(Debug, Clone)]
pub struct DependencyStats {
    pub repo: String,
    pub path: String,
    pub file_count: usize,
    pub size_bytes: u64,
}

/// Cached file list with lazy computation
pub struct FileListCache {
    cache: RwLock<HashMap<String, Vec<PathBuf>>>,
}

impl Default for FileListCache {
    fn default() -> Self {
        Self::new()
    }
}

impl FileListCache {
    pub fn new() -> Self {
        Self {
            cache: RwLock::new(HashMap::new()),
        }
    }

    /// Get file list for a dependency (cached).
    /// If the result is already in the cache, returns a clone.
    /// Otherwise, runs the `compute` closure, caches the result, and returns it.
    pub fn get_or_compute(
        &self,
        dep_id: &str,
        compute: impl FnOnce() -> Vec<PathBuf>,
    ) -> Vec<PathBuf> {
        // Fast path: read lock
        {
            let cache = self.cache.read();
            if let Some(files) = cache.get(dep_id) {
                return files.clone();
            }
        }

        // Slow path: compute and write
        let files = compute();
        let mut cache = self.cache.write();
        cache.insert(dep_id.to_string(), files.clone());
        files
    }

    /// Invalidate cache for a specific dependency
    pub fn invalidate(&self, dep_id: &str) {
        let mut cache = self.cache.write();
        cache.remove(dep_id);
    }

    /// Clear all cached data
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }
}

/// Generate context summary for the entire workspace
pub fn generate_context_summary(
    config: &VDWConfig,
    workspace_root: &Path,
) -> Result<ContextStats> {
    let mut total_files = 0;
    let mut total_size = 0u64;
    let mut per_dep = Vec::new();

    for dep in &config.dependencies {
        let dep_path = workspace_root.join(&dep.path);
        let files = filter::filter_files(&dep_path, &dep.scope)?;
        let size = filter::total_size(&dep_path, &files);

        total_files += files.len();
        total_size += size;

        per_dep.push(DependencyStats {
            repo: dep.dep_id().to_string(),
            path: dep.path.clone(),
            file_count: files.len(),
            size_bytes: size,
        });
    }

    Ok(ContextStats {
        total_files,
        total_size_bytes: total_size,
        per_dependency: per_dep,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::{Dependency, Scope, WorkspaceMeta};
    use std::fs;

    #[test]
    fn test_file_list_cache_new() {
        let cache = FileListCache::new();
        let result = cache.get_or_compute("test", || vec![PathBuf::from("a.rs")]);
        assert_eq!(result, vec![PathBuf::from("a.rs")]);
    }

    #[test]
    fn test_file_list_cache_returns_cached() {
        let cache = FileListCache::new();
        let mut call_count = 0;

        // First call computes
        let result1 = cache.get_or_compute("dep1", || {
            call_count += 1;
            vec![PathBuf::from("file.rs")]
        });
        assert_eq!(result1, vec![PathBuf::from("file.rs")]);
        assert_eq!(call_count, 1);

        // Second call returns cached value without recomputing
        let result2 = cache.get_or_compute("dep1", || {
            call_count += 1;
            vec![PathBuf::from("different.rs")]
        });
        assert_eq!(result2, vec![PathBuf::from("file.rs")]);
        assert_eq!(call_count, 1); // compute was not called again
    }

    #[test]
    fn test_file_list_cache_different_keys() {
        let cache = FileListCache::new();
        let mut call_count = 0;

        let r1 = cache.get_or_compute("dep1", || {
            call_count += 1;
            vec![PathBuf::from("file_1.rs")]
        });
        let r2 = cache.get_or_compute("dep2", || {
            call_count += 1;
            vec![PathBuf::from("file_2.rs")]
        });

        assert_eq!(r1, vec![PathBuf::from("file_1.rs")]);
        assert_eq!(r2, vec![PathBuf::from("file_2.rs")]);
        assert_eq!(call_count, 2);
    }

    #[test]
    fn test_file_list_cache_invalidate() {
        let cache = FileListCache::new();
        let mut call_count = 0;

        // Insert
        cache.get_or_compute("dep1", || {
            call_count += 1;
            vec![PathBuf::from("original.rs")]
        });
        assert_eq!(call_count, 1);

        // Invalidate
        cache.invalidate("dep1");

        // Should recompute
        let result = cache.get_or_compute("dep1", || {
            call_count += 1;
            vec![PathBuf::from("updated.rs")]
        });
        assert_eq!(result, vec![PathBuf::from("updated.rs")]);
        assert_eq!(call_count, 2);
    }

    #[test]
    fn test_file_list_cache_clear() {
        let cache = FileListCache::new();

        cache.get_or_compute("dep1", || vec![PathBuf::from("a.rs")]);
        cache.get_or_compute("dep2", || vec![PathBuf::from("b.rs")]);

        cache.clear();

        let mut call_count = 0;
        let result = cache.get_or_compute("dep1", || {
            call_count += 1;
            vec![PathBuf::from("recomputed.rs")]
        });
        assert_eq!(result, vec![PathBuf::from("recomputed.rs")]);
        assert_eq!(call_count, 1);
    }

    #[test]
    fn test_file_list_cache_invalidate_nonexistent() {
        let cache = FileListCache::new();
        // Should not panic
        cache.invalidate("nonexistent");
    }

    fn setup_workspace() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let lib_path = dir.path().join("libs/core");
        fs::create_dir_all(&lib_path).unwrap();
        fs::write(lib_path.join("main.rs"), "fn main() {}").unwrap();
        fs::write(lib_path.join("lib.rs"), "pub fn lib() {}").unwrap();
        fs::write(lib_path.join("test.rs"), "#[test] fn test() {}").unwrap();
        dir
    }

    fn make_config() -> VDWConfig {
        VDWConfig {
            workspace: WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![Dependency {
                remote: "https://github.com/example/core.git".to_string(),
                origin: String::new(),
                local: String::new(),
                repo: String::new(),
                path: "libs/core".to_string(),
                branch: Some("main".to_string()),
                tag: None,
                commit: None,
                scope: Scope {
                    include: vec!["**/*.rs".to_string()],
                    exclude: vec!["**/test.rs".to_string()],
                },
                editable: false,
            }],
        }
    }

    #[test]
    fn test_generate_context_summary() {
        let dir = setup_workspace();
        let config = make_config();

        let summary = generate_context_summary(&config, dir.path()).unwrap();

        assert_eq!(summary.total_files, 2); // main.rs and lib.rs (test.rs excluded)
        assert!(summary.total_size_bytes > 0);
        assert_eq!(summary.per_dependency.len(), 1);
        assert_eq!(summary.per_dependency[0].repo, "https://github.com/example/core.git");
        assert_eq!(summary.per_dependency[0].path, "libs/core");
        assert_eq!(summary.per_dependency[0].file_count, 2);
    }

    #[test]
    fn test_generate_context_summary_no_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let config = VDWConfig {
            workspace: WorkspaceMeta {
                name: "empty".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![],
        };

        let summary = generate_context_summary(&config, dir.path()).unwrap();
        assert_eq!(summary.total_files, 0);
        assert_eq!(summary.total_size_bytes, 0);
        assert!(summary.per_dependency.is_empty());
    }

    #[test]
    fn test_generate_context_summary_unlinked_path() {
        let dir = tempfile::tempdir().unwrap();
        let config = VDWConfig {
            workspace: WorkspaceMeta {
                name: "test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![Dependency {
                remote: "https://github.com/example/missing.git".to_string(),
                origin: String::new(),
                local: String::new(),
                repo: String::new(),
                path: "libs/missing".to_string(),
                branch: Some("main".to_string()),
                tag: None,
                commit: None,
                scope: Scope {
                    include: vec!["**/*.rs".to_string()],
                    exclude: vec![],
                },
                editable: false,
            }],
        };

        let summary = generate_context_summary(&config, dir.path()).unwrap();
        assert_eq!(summary.total_files, 0);
        assert_eq!(summary.per_dependency[0].file_count, 0);
    }
}
