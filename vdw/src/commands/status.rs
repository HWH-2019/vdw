use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::parser;
use crate::link::engine;
use crate::store::{manager as store_manager, object_pool};
use crate::types::config::{Dependency, VDWConfig};
use crate::types::workspace::{LinkEntry, WorkspaceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopologyState {
    Ready,
    Missing,
    InvalidWorktree,
    Unowned,
    Conflict,
    Detached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendedAction {
    None,
    Link,
    Relink,
    ResolveConflicts,
}

#[derive(Debug, Clone, Serialize)]
pub struct RepositoryTopologyStatus {
    pub repo: String,
    pub path: String,
    pub full_path: PathBuf,
    pub editable: bool,
    pub declared_revision: String,
    pub topology_state: TopologyState,
    pub recommended_action: RecommendedAction,
    pub tracked_link_count: usize,
    pub conflict_count: usize,
    pub missing_count: usize,
    pub detached_equivalent_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceTopologyStatus {
    pub workspace_root: PathBuf,
    pub workspace_name: String,
    pub healthy: bool,
    pub issue_count: usize,
    pub conflict_count: usize,
    pub repositories: Vec<RepositoryTopologyStatus>,
}

pub fn show(json: bool) -> Result<()> {
    let workspace_root = std::env::current_dir()?;
    let store_path = store_manager::get_store_path()?;
    let report = inspect_workspace(&workspace_root, &store_path)?;

    if json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    println!(
        "Workspace: {} ({})",
        report.workspace_name,
        report.workspace_root.display()
    );
    println!(
        "{:<32} {:<18} {:<10} PATH",
        "REPOSITORY", "REVISION", "TOPOLOGY"
    );
    println!("{}", "-".repeat(88));
    for repository in &report.repositories {
        println!(
            "{:<32} {:<18} {:<10} {}",
            truncate(&repository_name(&repository.repo), 30),
            truncate(&repository.declared_revision, 16),
            topology_label(repository.topology_state),
            repository.path,
        );
        if let Some(message) = &repository.message {
            println!("  {}", message);
        }
    }
    println!();
    if report.healthy {
        println!(
            "All {} repositories have healthy VDW topology.",
            report.repositories.len()
        );
    } else {
        println!(
            "{} repository issue(s), {} link conflict(s).",
            report.issue_count, report.conflict_count
        );
        println!("Use `vdw status --json` for structured diagnostics.");
    }
    Ok(())
}

pub fn inspect_workspace(
    workspace_root: &Path,
    store_path: &Path,
) -> Result<WorkspaceTopologyStatus> {
    let config = parser::load_config(workspace_root)?;
    let state = WorkspaceState::load(workspace_root)?;
    inspect(&config, &state, workspace_root, store_path)
}

fn inspect(
    config: &VDWConfig,
    state: &WorkspaceState,
    workspace_root: &Path,
    store_path: &Path,
) -> Result<WorkspaceTopologyStatus> {
    let repositories = config
        .dependencies
        .iter()
        .map(|dependency| inspect_dependency(dependency, state, workspace_root, store_path))
        .collect::<Vec<_>>();
    let issue_count = repositories
        .iter()
        .filter(|repository| repository.topology_state != TopologyState::Ready)
        .count();
    let conflict_count = repositories
        .iter()
        .map(|repository| repository.conflict_count)
        .sum();

    Ok(WorkspaceTopologyStatus {
        workspace_root: workspace_root.to_path_buf(),
        workspace_name: config.workspace.name.clone(),
        healthy: issue_count == 0,
        issue_count,
        conflict_count,
        repositories,
    })
}

fn inspect_dependency(
    dependency: &Dependency,
    state: &WorkspaceState,
    workspace_root: &Path,
    store_path: &Path,
) -> RepositoryTopologyStatus {
    let full_path = workspace_root.join(&dependency.path);
    let mut status = RepositoryTopologyStatus {
        repo: dependency.dep_id().to_string(),
        path: dependency.path.clone(),
        full_path: full_path.clone(),
        editable: dependency.editable,
        declared_revision: declared_revision(dependency),
        topology_state: TopologyState::Ready,
        recommended_action: RecommendedAction::None,
        tracked_link_count: 0,
        conflict_count: 0,
        missing_count: 0,
        detached_equivalent_count: 0,
        message: None,
    };

    if dependency.editable {
        inspect_editable(dependency, store_path, &full_path, &mut status);
    } else {
        inspect_read_only(dependency, state, &full_path, &mut status);
    }
    status
}

fn inspect_editable(
    dependency: &Dependency,
    store_path: &Path,
    full_path: &Path,
    status: &mut RepositoryTopologyStatus,
) {
    if !full_path.exists() {
        set_issue(
            status,
            TopologyState::Missing,
            RecommendedAction::Link,
            "Editable worktree path does not exist".to_string(),
        );
        return;
    }
    if !full_path.is_dir() {
        set_issue(
            status,
            TopologyState::InvalidWorktree,
            RecommendedAction::Relink,
            "Editable dependency path is not a directory".to_string(),
        );
        return;
    }

    let repository = match git2::Repository::open(full_path) {
        Ok(repository) => repository,
        Err(error) => {
            set_issue(
                status,
                TopologyState::InvalidWorktree,
                RecommendedAction::Relink,
                format!("Path is not a Git worktree: {}", error.message()),
            );
            return;
        }
    };

    let expected_bare =
        object_pool::repo_store_dir(dependency.dep_id(), store_path).join("bare.git");
    let actual_git_dir = canonicalize_existing(repository.path());
    let expected_git_dir = canonicalize_existing(&expected_bare);
    match (actual_git_dir, expected_git_dir) {
        (Ok(actual), Ok(expected))
            if actual == expected || actual.starts_with(expected.join("worktrees")) => {}
        (Ok(actual), Ok(expected)) => set_issue(
            status,
            TopologyState::Unowned,
            RecommendedAction::Relink,
            format!(
                "Git directory is {}, expected a VDW Store worktree of {}",
                actual.display(),
                expected.display()
            ),
        ),
        (Ok(_), Err(_)) => set_issue(
            status,
            TopologyState::Unowned,
            RecommendedAction::Relink,
            format!(
                "VDW Store repository is missing: {}",
                expected_bare.display()
            ),
        ),
        (Err(error), _) => set_issue(
            status,
            TopologyState::InvalidWorktree,
            RecommendedAction::Relink,
            format!("Cannot resolve Git directory: {}", error),
        ),
    }
}

fn inspect_read_only(
    dependency: &Dependency,
    state: &WorkspaceState,
    full_path: &Path,
    status: &mut RepositoryTopologyStatus,
) {
    let entries = state
        .links
        .iter()
        .filter(|entry| {
            entry.dependency_repo == dependency.dep_id() && entry.target.starts_with(full_path)
        })
        .cloned()
        .collect::<Vec<LinkEntry>>();
    status.tracked_link_count = entries.len();

    if entries.is_empty() {
        let (topology_state, action, message) = if full_path.exists() {
            (
                TopologyState::Unowned,
                RecommendedAction::Relink,
                "Read-only projection exists but has no VDW ownership records",
            )
        } else {
            (
                TopologyState::Missing,
                RecommendedAction::Link,
                "Read-only projection has not been materialized",
            )
        };
        set_issue(status, topology_state, action, message.to_string());
        return;
    }

    let audit = engine::audit_links(&entries);
    status.conflict_count = audit.conflicts.len();
    status.missing_count = audit.missing_count;
    status.detached_equivalent_count = audit.detached_equivalent_count;

    if !audit.conflicts.is_empty() {
        set_issue(
            status,
            TopologyState::Conflict,
            RecommendedAction::ResolveConflicts,
            format!(
                "{} tracked target(s) differ between Store and workspace",
                audit.conflicts.len()
            ),
        );
    } else if audit.missing_count > 0 {
        set_issue(
            status,
            TopologyState::Missing,
            RecommendedAction::Relink,
            format!("{} tracked link target(s) are missing", audit.missing_count),
        );
    } else if audit.detached_equivalent_count > 0 {
        set_issue(
            status,
            TopologyState::Detached,
            RecommendedAction::Relink,
            format!(
                "{} target(s) have identical content but are no longer VDW links",
                audit.detached_equivalent_count
            ),
        );
    }
}

fn set_issue(
    status: &mut RepositoryTopologyStatus,
    topology_state: TopologyState,
    recommended_action: RecommendedAction,
    message: String,
) {
    status.topology_state = topology_state;
    status.recommended_action = recommended_action;
    status.message = Some(message);
}

fn declared_revision(dependency: &Dependency) -> String {
    dependency
        .branch
        .as_deref()
        .or(dependency.tag.as_deref())
        .or(dependency.commit.as_deref())
        .unwrap_or("main")
        .to_string()
}

fn canonicalize_existing(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).with_context(|| format!("Cannot canonicalize {}", path.display()))
}

fn topology_label(state: TopologyState) -> &'static str {
    match state {
        TopologyState::Ready => "ready",
        TopologyState::Missing => "missing",
        TopologyState::InvalidWorktree => "invalid",
        TopologyState::Unowned => "unowned",
        TopologyState::Conflict => "conflict",
        TopologyState::Detached => "detached",
    }
}

fn repository_name(repo: &str) -> String {
    repo.trim_end_matches(['/', '\\'])
        .rsplit(['/', ':', '\\'])
        .next()
        .unwrap_or(repo)
        .trim_end_matches(".git")
        .to_string()
}

fn truncate(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut result = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::hardlink;
    use crate::types::config::{Scope, WorkspaceMeta};
    use crate::types::workspace::LinkType;

    fn dependency(path: &str, editable: bool) -> Dependency {
        Dependency {
            remote: "https://example.com/org/repo.git".to_string(),
            origin: String::new(),
            local: String::new(),
            repo: String::new(),
            path: path.to_string(),
            branch: Some("main".to_string()),
            tag: None,
            commit: None,
            scope: Scope {
                include: vec!["**/*".to_string()],
                exclude: vec![],
            },
            editable,
        }
    }

    fn config(dependencies: Vec<Dependency>) -> VDWConfig {
        VDWConfig {
            workspace: WorkspaceMeta {
                name: "test-workspace".to_string(),
                version: "1.0".to_string(),
            },
            dependencies,
        }
    }

    #[test]
    fn reports_missing_editable_worktree_without_persisting_runtime_state() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let state = WorkspaceState::new(workspace.path().to_path_buf());

        let report = inspect(
            &config(vec![dependency("packages/repo", true)]),
            &state,
            workspace.path(),
            store.path(),
        )
        .expect("status");

        assert!(!report.healthy);
        assert_eq!(report.issue_count, 1);
        assert_eq!(
            report.repositories[0].topology_state,
            TopologyState::Missing
        );
        assert_eq!(
            report.repositories[0].recommended_action,
            RecommendedAction::Link
        );
        assert!(!workspace.path().join(".vdw/state.json").exists());
    }

    #[test]
    fn reports_unowned_editable_git_directory() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let target = workspace.path().join("packages/repo");
        std::fs::create_dir_all(&target).expect("target");
        git2::Repository::init(&target).expect("git repo");

        let report = inspect(
            &config(vec![dependency("packages/repo", true)]),
            &WorkspaceState::new(workspace.path().to_path_buf()),
            workspace.path(),
            store.path(),
        )
        .expect("status");

        assert_eq!(
            report.repositories[0].topology_state,
            TopologyState::Unowned
        );
        assert_eq!(
            report.repositories[0].recommended_action,
            RecommendedAction::Relink
        );
    }

    #[test]
    fn aggregates_read_only_link_conflicts() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let source = store.path().join("file.txt");
        let target = workspace.path().join("packages/repo/file.txt");
        std::fs::create_dir_all(target.parent().unwrap()).expect("target parent");
        std::fs::write(&source, "store").expect("source");
        std::fs::write(&target, "workspace").expect("target");
        let mut state = WorkspaceState::new(workspace.path().to_path_buf());
        state.links.push(LinkEntry {
            source,
            target,
            link_type: LinkType::HardLink,
            dependency_repo: dependency("packages/repo", false).dep_id().to_string(),
            source_inode: None,
            source_volume: None,
        });

        let report = inspect(
            &config(vec![dependency("packages/repo", false)]),
            &state,
            workspace.path(),
            store.path(),
        )
        .expect("status");

        assert_eq!(report.conflict_count, 1);
        assert_eq!(
            report.repositories[0].topology_state,
            TopologyState::Conflict
        );
        assert_eq!(
            report.repositories[0].recommended_action,
            RecommendedAction::ResolveConflicts
        );
    }

    #[test]
    fn reports_owned_read_only_projection_as_ready() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let source = store.path().join("file.txt");
        let target = workspace.path().join("packages/repo/file.txt");
        std::fs::write(&source, "same").expect("source");
        hardlink::create_hardlink(&source, &target).expect("hardlink");
        let mut state = WorkspaceState::new(workspace.path().to_path_buf());
        state.links.push(LinkEntry {
            source,
            target,
            link_type: LinkType::HardLink,
            dependency_repo: dependency("packages/repo", false).dep_id().to_string(),
            source_inode: None,
            source_volume: None,
        });

        let report = inspect(
            &config(vec![dependency("packages/repo", false)]),
            &state,
            workspace.path(),
            store.path(),
        )
        .expect("status");

        assert!(report.healthy);
        assert_eq!(report.repositories[0].topology_state, TopologyState::Ready);
        assert_eq!(report.repositories[0].tracked_link_count, 1);
    }

    #[test]
    fn scopes_same_repository_link_records_to_each_dependency_path() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let source = store.path().join("file.txt");
        let target = workspace.path().join("packages/first/file.txt");
        std::fs::write(&source, "same").expect("source");
        hardlink::create_hardlink(&source, &target).expect("hardlink");
        let mut state = WorkspaceState::new(workspace.path().to_path_buf());
        state.links.push(LinkEntry {
            source,
            target,
            link_type: LinkType::HardLink,
            dependency_repo: dependency("packages/first", false).dep_id().to_string(),
            source_inode: None,
            source_volume: None,
        });

        let report = inspect(
            &config(vec![
                dependency("packages/first", false),
                dependency("packages/second", false),
            ]),
            &state,
            workspace.path(),
            store.path(),
        )
        .expect("status");

        assert_eq!(report.repositories[0].topology_state, TopologyState::Ready);
        assert_eq!(report.repositories[0].tracked_link_count, 1);
        assert_eq!(
            report.repositories[1].topology_state,
            TopologyState::Missing
        );
        assert_eq!(report.repositories[1].tracked_link_count, 0);
    }
}
