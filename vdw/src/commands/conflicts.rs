use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::Utc;
use clap::ValueEnum;
use serde::Serialize;

use crate::link::{engine, hardlink};
use crate::types::workspace::{LinkEntry, LinkType, WorkspaceState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionSide {
    Store,
    Workspace,
}

impl ConflictResolutionSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Store => "store",
            Self::Workspace => "workspace",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictView {
    pub source: PathBuf,
    pub target: PathBuf,
    pub relative_path: PathBuf,
    pub link_type: LinkType,
    pub dependency_repo: String,
    pub reason: engine::LinkConflictReason,
    pub resolvable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictListReport {
    pub conflict_count: usize,
    pub missing_count: usize,
    pub detached_equivalent_count: usize,
    pub conflicts: Vec<ConflictView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConflictResolutionReport {
    pub resolved_count: usize,
    pub accepted: ConflictResolutionSide,
    pub backup_root: PathBuf,
}

pub fn list(repo_filter: Option<&str>, json: bool) -> Result<()> {
    let workspace_root = std::env::current_dir()?;
    let report = inspect(&workspace_root, repo_filter)?;

    if json {
        println!("{}", serde_json::to_string(&report)?);
        return Ok(());
    }

    if report.conflict_count == 0 {
        println!("No link conflicts found.");
        if report.detached_equivalent_count > 0 {
            println!(
                "{} detached target(s) have identical content and can be rebuilt safely.",
                report.detached_equivalent_count
            );
        }
        return Ok(());
    }

    println!("Found {} link conflict(s):", report.conflict_count);
    for conflict in &report.conflicts {
        println!(
            "  - {} ({:?})\n      Store: {}\n      Workspace: {}",
            conflict.relative_path.display(),
            conflict.reason,
            conflict.source.display(),
            conflict.target.display()
        );
    }
    println!();
    println!("Resolve all: vdw conflicts resolve --accept store|workspace");
    println!("Resolve selected: add --path <workspace-relative-path>");
    Ok(())
}

pub fn resolve(
    repo_filter: Option<&str>,
    accepted: ConflictResolutionSide,
    paths: &[PathBuf],
    json: bool,
) -> Result<()> {
    let workspace_root = std::env::current_dir()?;
    let report = resolve_in_workspace(&workspace_root, repo_filter, accepted, paths)?;

    if json {
        println!("{}", serde_json::to_string(&report)?);
    } else {
        println!(
            "Resolved {} conflict(s) by accepting {}.",
            report.resolved_count,
            accepted.as_str()
        );
        println!("Backups: {}", report.backup_root.display());
    }
    Ok(())
}

pub fn inspect(workspace_root: &Path, repo_filter: Option<&str>) -> Result<ConflictListReport> {
    let state = WorkspaceState::load(workspace_root)?;
    let entries = matching_entries(&state, repo_filter);
    let audit = engine::audit_links(&entries);
    let conflicts = audit
        .conflicts
        .into_iter()
        .map(|conflict| ConflictView {
            relative_path: workspace_relative_path(workspace_root, &conflict.target)
                .unwrap_or_else(|_| conflict.target.clone()),
            source: conflict.source,
            target: conflict.target,
            link_type: conflict.link_type,
            dependency_repo: conflict.dependency_repo,
            reason: conflict.reason,
            resolvable: conflict.resolvable,
        })
        .collect::<Vec<_>>();

    Ok(ConflictListReport {
        conflict_count: conflicts.len(),
        missing_count: audit.missing_count,
        detached_equivalent_count: audit.detached_equivalent_count,
        conflicts,
    })
}

pub fn resolve_in_workspace(
    workspace_root: &Path,
    repo_filter: Option<&str>,
    accepted: ConflictResolutionSide,
    paths: &[PathBuf],
) -> Result<ConflictResolutionReport> {
    let original_state = WorkspaceState::load(workspace_root)?;
    let entries = matching_entries(&original_state, repo_filter);
    let audit = engine::audit_links(&entries);
    let selected_paths = normalize_selected_paths(workspace_root, paths)?;

    let selected_conflicts = audit
        .conflicts
        .iter()
        .filter(|conflict| {
            if selected_paths.is_empty() {
                return true;
            }
            workspace_relative_path(workspace_root, &conflict.target)
                .map(|relative| selected_paths.contains(&relative))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    if selected_conflicts.is_empty() {
        if audit.conflicts.is_empty() {
            bail!("No link conflicts found");
        }
        bail!("None of the requested paths match a current link conflict");
    }

    if let Some(conflict) = selected_conflicts
        .iter()
        .find(|conflict| !conflict.resolvable || conflict.link_type != LinkType::HardLink)
    {
        bail!(
            "Conflict cannot be resolved automatically: {} ({:?})",
            conflict.target.display(),
            conflict.reason
        );
    }

    let selected_relative_paths = selected_conflicts
        .iter()
        .map(|conflict| workspace_relative_path(workspace_root, &conflict.target))
        .collect::<Result<HashSet<_>>>()?;

    if !selected_paths.is_empty() {
        let unmatched = selected_paths
            .difference(&selected_relative_paths)
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>();
        if !unmatched.is_empty() {
            bail!(
                "Requested path(s) are not current conflicts: {}",
                unmatched.join(", ")
            );
        }
    }

    let backup_root = workspace_root
        .join(".vdw")
        .join("conflict-backups")
        .join(format!(
            "{}-{}-{}",
            Utc::now().format("%Y%m%dT%H%M%S%.fZ"),
            std::process::id(),
            accepted.as_str()
        ));

    for conflict in &selected_conflicts {
        let relative_path = workspace_relative_path(workspace_root, &conflict.target)?;
        backup_file(
            &conflict.source,
            &backup_root.join("store").join(&relative_path),
        )?;
        backup_file(
            &conflict.target,
            &backup_root.join("workspace").join(&relative_path),
        )?;
    }

    let apply_result = (|| -> Result<WorkspaceState> {
        for conflict in &selected_conflicts {
            if accepted == ConflictResolutionSide::Workspace {
                std::fs::copy(&conflict.target, &conflict.source).with_context(|| {
                    format!(
                        "Failed to accept workspace content for {}",
                        conflict.target.display()
                    )
                })?;
            }
            hardlink::create_hardlink(&conflict.source, &conflict.target).with_context(|| {
                format!("Failed to rebuild hardlink {}", conflict.target.display())
            })?;
        }

        let mut new_state = original_state.clone();
        for entry in &mut new_state.links {
            let relative_path = match workspace_relative_path(workspace_root, &entry.target) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if selected_relative_paths.contains(&relative_path) {
                if let Some((inode, volume)) = engine::get_file_inode(&entry.source) {
                    entry.source_inode = Some(inode);
                    entry.source_volume = Some(volume);
                }
            }
        }
        new_state.save()?;
        Ok(new_state)
    })();

    if let Err(error) = apply_result {
        let rollback_error = rollback_from_backup(
            workspace_root,
            &backup_root,
            &selected_conflicts,
            &original_state,
        )
        .err();
        return match rollback_error {
            Some(rollback_error) => Err(error.context(format!(
                "Conflict resolution failed and rollback was incomplete: {}",
                rollback_error
            ))),
            None => Err(error.context("Conflict resolution failed; original files were restored")),
        };
    }

    Ok(ConflictResolutionReport {
        resolved_count: selected_conflicts.len(),
        accepted,
        backup_root,
    })
}

fn matching_entries(state: &WorkspaceState, repo_filter: Option<&str>) -> Vec<LinkEntry> {
    state
        .links
        .iter()
        .filter(|entry| {
            repo_filter
                .map(|repo| entry.dependency_repo == repo)
                .unwrap_or(true)
        })
        .cloned()
        .collect()
}

fn workspace_relative_path(workspace_root: &Path, target: &Path) -> Result<PathBuf> {
    let relative = target.strip_prefix(workspace_root).with_context(|| {
        format!(
            "Tracked target is outside the workspace: {}",
            target.display()
        )
    })?;
    normalize_relative_path(relative)
}

fn normalize_selected_paths(workspace_root: &Path, paths: &[PathBuf]) -> Result<HashSet<PathBuf>> {
    paths
        .iter()
        .map(|path| {
            let relative = if path.is_absolute() {
                path.strip_prefix(workspace_root)
                    .with_context(|| format!("Path is outside the workspace: {}", path.display()))?
            } else {
                path.as_path()
            };
            normalize_relative_path(relative)
        })
        .collect()
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(segment) => normalized.push(segment),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("Path must stay inside the workspace: {}", path.display())
            }
        }
    }
    Ok(normalized)
}

fn backup_file(source: &Path, backup: &Path) -> Result<()> {
    if let Some(parent) = backup.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, backup).with_context(|| {
        format!(
            "Failed to back up {} to {}",
            source.display(),
            backup.display()
        )
    })?;
    Ok(())
}

fn rollback_from_backup(
    workspace_root: &Path,
    backup_root: &Path,
    conflicts: &[&engine::LinkConflict],
    original_state: &WorkspaceState,
) -> Result<()> {
    let mut errors = Vec::new();
    for conflict in conflicts.iter().rev() {
        let relative_path = match workspace_relative_path(workspace_root, &conflict.target) {
            Ok(path) => path,
            Err(error) => {
                errors.push(error.to_string());
                continue;
            }
        };
        let store_backup = backup_root.join("store").join(&relative_path);
        let workspace_backup = backup_root.join("workspace").join(&relative_path);

        if let Err(error) = std::fs::copy(&store_backup, &conflict.source) {
            errors.push(format!(
                "restore Store {}: {}",
                conflict.source.display(),
                error
            ));
        }
        if let Err(error) = hardlink::remove_hardlink(&conflict.target) {
            errors.push(format!(
                "remove rebuilt target {}: {}",
                conflict.target.display(),
                error
            ));
        }
        if let Some(parent) = conflict.target.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                errors.push(format!(
                    "restore target directory {}: {}",
                    parent.display(),
                    error
                ));
                continue;
            }
        }
        if let Err(error) = std::fs::copy(&workspace_backup, &conflict.target) {
            errors.push(format!(
                "restore workspace {}: {}",
                conflict.target.display(),
                error
            ));
        }
    }

    if let Err(error) = original_state.save() {
        errors.push(format!("restore state.json: {}", error));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        bail!(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conflict_state(workspace_root: &Path, source: &Path, target: &Path) -> WorkspaceState {
        let mut state = WorkspaceState::new(workspace_root.to_path_buf());
        state.links.push(LinkEntry {
            source: source.to_path_buf(),
            target: target.to_path_buf(),
            link_type: LinkType::HardLink,
            dependency_repo: "repo".to_string(),
            source_inode: None,
            source_volume: None,
        });
        state
    }

    #[test]
    fn resolve_accepts_store_and_keeps_recoverable_backups() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let source = store.path().join("file.txt");
        let target = workspace.path().join("packages/repo/file.txt");
        std::fs::create_dir_all(target.parent().unwrap()).expect("target parent");
        std::fs::write(&source, "store edit").expect("source");
        std::fs::write(&target, "workspace edit").expect("target");
        conflict_state(workspace.path(), &source, &target)
            .save()
            .expect("state");

        let report =
            resolve_in_workspace(workspace.path(), None, ConflictResolutionSide::Store, &[])
                .expect("resolve");

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "store edit");
        assert_eq!(
            engine::get_file_inode(&source),
            engine::get_file_inode(&target)
        );
        assert_eq!(
            std::fs::read_to_string(report.backup_root.join("workspace/packages/repo/file.txt"))
                .unwrap(),
            "workspace edit"
        );
    }

    #[test]
    fn resolve_accepts_workspace_and_writes_it_back_to_store() {
        let workspace = tempfile::tempdir().expect("workspace");
        let store = tempfile::tempdir().expect("store");
        let source = store.path().join("file.txt");
        let target = workspace.path().join("packages/repo/file.txt");
        std::fs::create_dir_all(target.parent().unwrap()).expect("target parent");
        std::fs::write(&source, "store edit").expect("source");
        std::fs::write(&target, "workspace edit").expect("target");
        conflict_state(workspace.path(), &source, &target)
            .save()
            .expect("state");

        let report = resolve_in_workspace(
            workspace.path(),
            None,
            ConflictResolutionSide::Workspace,
            &[PathBuf::from("packages/repo/file.txt")],
        )
        .expect("resolve");

        assert_eq!(std::fs::read_to_string(&source).unwrap(), "workspace edit");
        assert_eq!(
            engine::get_file_inode(&source),
            engine::get_file_inode(&target)
        );
        assert_eq!(
            std::fs::read_to_string(report.backup_root.join("store/packages/repo/file.txt"))
                .unwrap(),
            "store edit"
        );
    }

    #[test]
    fn resolve_rejects_paths_outside_workspace() {
        let workspace = tempfile::tempdir().expect("workspace");
        let error = normalize_selected_paths(workspace.path(), &[PathBuf::from("../outside")])
            .expect_err("outside path must be rejected");
        assert!(error.to_string().contains("inside the workspace"));
    }
}
