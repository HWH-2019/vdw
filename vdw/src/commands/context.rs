use anyhow::Result;
use serde::Serialize;
use std::path::{Path, PathBuf};

use crate::config::parser;
use crate::context::{file_list, filter};
use crate::types::config::VDWConfig;

#[derive(Debug, Serialize)]
struct ContextPreviewReport {
    workspace_root: PathBuf,
    workspace_name: String,
    total_files: usize,
    repositories: Vec<RepositoryContext>,
}

#[derive(Debug, Serialize)]
struct RepositoryContext {
    repo: String,
    path: String,
    linked: bool,
    files: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct ContextStatsReport {
    workspace_root: PathBuf,
    workspace_name: String,
    #[serde(flatten)]
    stats: file_list::ContextStats,
}

/// Preview all files in the AI context
pub fn preview(json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = parser::load_config(&cwd)?;
    let report = build_preview_report(&config, &cwd)?;

    if json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    println!("AI Context Preview");
    println!("==================\n");

    for repository in &report.repositories {
        println!("📦 {} ({})", repository.repo, repository.path);

        if !repository.linked {
            println!("   ⚠️  Path not linked yet. Run 'vdw link' first.\n");
            continue;
        }

        if repository.files.is_empty() {
            println!("   (no files match scope)\n");
        } else {
            for file in &repository.files {
                println!("   {}", file.display());
            }
            println!("   --- {} file(s)\n", repository.files.len());
        }
    }

    Ok(())
}

fn build_preview_report(config: &VDWConfig, workspace_root: &Path) -> Result<ContextPreviewReport> {
    let mut repositories = Vec::with_capacity(config.dependencies.len());
    let mut total_files = 0;

    for dep in &config.dependencies {
        let dep_path = workspace_root.join(&dep.path);
        let linked = dep_path.exists();
        let files = if linked {
            filter::filter_files(&dep_path, &dep.scope)?
                .into_iter()
                .map(|file| Path::new(&dep.path).join(file))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        total_files += files.len();
        repositories.push(RepositoryContext {
            repo: dep.dep_id().to_string(),
            path: dep.path.clone(),
            linked,
            files,
        });
    }

    Ok(ContextPreviewReport {
        workspace_root: workspace_root.to_path_buf(),
        workspace_name: config.workspace.name.clone(),
        total_files,
        repositories,
    })
}

/// Show context statistics
pub fn stats(json: bool) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let config = parser::load_config(&cwd)?;

    let summary = file_list::generate_context_summary(&config, &cwd)?;

    if json {
        let report = ContextStatsReport {
            workspace_root: cwd,
            workspace_name: config.workspace.name,
            stats: summary,
        };
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    println!("AI Context Statistics");
    println!("=====================\n");
    println!("Total files in context: {}", summary.total_files);
    println!("Total size: {}\n", format_size(summary.total_size_bytes));

    println!("{:<40} {:>8} {:>10}", "DEPENDENCY", "FILES", "SIZE");
    println!("{}", "-".repeat(60));

    for dep in &summary.per_dependency {
        println!(
            "{:<40} {:>8} {:>10}",
            truncate(&dep.path, 38),
            dep.file_count,
            format_size(dep.size_bytes),
        );
    }

    Ok(())
}

fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("...{}", &s[s.len() - max_len + 3..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::config::{Dependency, Scope, WorkspaceMeta};
    use std::fs;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 2 + 512 * 1024), "2.5 MB");
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_exact() {
        assert_eq!(truncate("12345", 5), "12345");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("a_very_long_path_string", 10);
        assert_eq!(result, "..._string");
        assert_eq!(result.len(), 10);
    }

    #[test]
    fn preview_report_uses_workspace_relative_paths_and_marks_missing_repos() {
        let workspace = tempfile::tempdir().expect("workspace");
        let linked_path = workspace.path().join("packages/linked/src");
        fs::create_dir_all(&linked_path).expect("linked path");
        fs::write(linked_path.join("lib.rs"), "pub fn linked() {}").expect("source file");
        fs::write(linked_path.join("lib.test.rs"), "test").expect("excluded file");

        let config = VDWConfig {
            workspace: WorkspaceMeta {
                name: "context-test".to_string(),
                version: "1.0".to_string(),
            },
            dependencies: vec![
                Dependency {
                    remote: "https://example.com/linked.git".to_string(),
                    origin: String::new(),
                    local: String::new(),
                    repo: String::new(),
                    path: "packages/linked".to_string(),
                    branch: Some("main".to_string()),
                    tag: None,
                    commit: None,
                    scope: Scope {
                        include: vec!["src/**/*.rs".to_string()],
                        exclude: vec!["**/*.test.rs".to_string()],
                    },
                    editable: true,
                },
                Dependency {
                    remote: "https://example.com/missing.git".to_string(),
                    origin: String::new(),
                    local: String::new(),
                    repo: String::new(),
                    path: "packages/missing".to_string(),
                    branch: Some("main".to_string()),
                    tag: None,
                    commit: None,
                    scope: Scope {
                        include: vec!["**/*".to_string()],
                        exclude: vec![],
                    },
                    editable: false,
                },
            ],
        };

        let report = build_preview_report(&config, workspace.path()).expect("preview report");

        assert_eq!(report.total_files, 1);
        assert!(report.repositories[0].linked);
        assert_eq!(
            report.repositories[0].files,
            vec![PathBuf::from("packages/linked/src/lib.rs")]
        );
        assert!(!report.repositories[1].linked);
        assert!(report.repositories[1].files.is_empty());
    }
}
