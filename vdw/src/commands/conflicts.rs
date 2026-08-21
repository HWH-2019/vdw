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

    std::fs::create_dir_all(&backup_root)?;
    for conflict in &selected_conflicts {
        let relative_path = workspace_relative_path(workspace_root, &conflict.target)?;
        backup_file_if_exists(
            &conflict.source,
            &backup_root.join("store").join(&relative_path),
        )?;
        backup_file_if_exists(
            &conflict.target,
            &backup_root.join("workspace").join(&relative_path),
        )?;
    }

    let apply_result = (|| -> Result<WorkspaceState> {
        for conflict in &selected_conflicts {
            match (conflict.reason, accepted) {
                (engine::LinkConflictReason::TargetMissing, ConflictResolutionSide::Store) => {
                    hardlink::create_hardlink(&conflict.source, &conflict.target).with_context(
                        || format!("Failed to restore {} from Store", conflict.target.display()),
                    )?;
                }
                (engine::LinkConflictReason::TargetMissing, ConflictResolutionSide::Workspace) => {
                    remove_path_if_exists(&conflict.source).with_context(|| {
                        format!(
                            "Failed to accept workspace deletion for {}",
                            conflict.target.display()
                        )
                    })?;
                }
                (engine::LinkConflictReason::SourceMissing, ConflictResolutionSide::Store) => {
                    remove_path_if_exists(&conflict.target).with_context(|| {
                        format!(
                            "Failed to accept Store deletion for {}",
                            conflict.target.display()
                        )
                    })?;
                }
                (engine::LinkConflictReason::SourceMissing, ConflictResolutionSide::Workspace) => {
                    copy_file(
                        &conflict.target,
                        &conflict.source,
                        "accept workspace content",
                    )?;
                    hardlink::create_hardlink(&conflict.source, &conflict.target).with_context(
                        || format!("Failed to rebuild hardlink {}", conflict.target.display()),
                    )?;
                }
                (_, ConflictResolutionSide::Workspace) => {
                    copy_file(
                        &conflict.target,
                        &conflict.source,
                        "accept workspace content",
                    )?;
                    hardlink::create_hardlink(&conflict.source, &conflict.target).with_context(
                        || format!("Failed to rebuild hardlink {}", conflict.target.display()),
                    )?;
                }
                (_, ConflictResolutionSide::Store) => {
                    hardlink::create_hardlink(&conflict.source, &conflict.target).with_context(
                        || format!("Failed to rebuild hardlink {}", conflict.target.display()),
                    )?;
                }
            }
        }

        let mut new_state = original_state.clone();
        for entry in &mut new_state.links {
            let relative_path = match workspace_relative_path(workspace_root, &entry.target) {
                Ok(path) => path,
                Err(_) => continue,
            };
            if selected_relative_paths.contains(&relative_path) {
                let identity = engine::get_file_inode(&entry.source);
                entry.source_inode = identity.map(|(inode, _)| inode);
                entry.source_volume = identity.map(|(_, volume)| volume);
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

fn backup_file_if_exists(source: &Path, backup: &Path) -> Result<()> {
    if std::fs::symlink_metadata(source).is_err() {
        return Ok(());
    }
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

fn copy_file(source: &Path, target: &Path, operation: &str) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, target).with_context(|| {
        format!(
            "Failed to {} from {} to {}",
            operation,
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else {
        std::fs::remove_file(path)?;
    }
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

        if let Err(error) = remove_path_if_exists(&conflict.target) {
            errors.push(format!(
                "clear workspace {}: {}",
                conflict.target.display(),
                error
            ));
        }
        if let Err(error) = remove_path_if_exists(&conflict.source) {
            errors.push(format!(
                "clear Store {}: {}",
                conflict.source.display(),
                error
            ));
        }
        if store_backup.exists() {
            if let Err(error) = copy_file(&store_backup, &conflict.source, "restore Store backup") {
                errors.push(error.to_string());
            }
        }
        if workspace_backup.exists() {
            if let Err(error) = copy_file(
                &workspace_backup,
                &conflict.target,
                "restore workspace backup",
            ) {
                errors.push(error.to_string());
            }
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

    #[test]
    fn resolve_missing_workspace_target_accepts_store_or_workspace_deletion() {
        for accepted in [
            ConflictResolutionSide::Store,
            ConflictResolutionSide::Workspace,
        ] {
            let workspace = tempfile::tempdir().expect("workspace");
            let store = tempfile::tempdir().expect("store");
            let source = store.path().join("file.txt");
            let target = workspace.path().join("packages/repo/file.txt");
            std::fs::write(&source, "store file").expect("source");
            conflict_state(workspace.path(), &source, &target)
                .save()
                .expect("state");

            resolve_in_workspace(workspace.path(), None, accepted, &[]).expect("resolve");

            if accepted == ConflictResolutionSide::Store {
                assert_eq!(
                    engine::get_file_inode(&source),
                    engine::get_file_inode(&target)
                );
            } else {
                assert!(!source.exists());
                assert!(!target.exists());
            }
        }
    }

    #[test]
    fn resolve_missing_store_source_accepts_store_deletion_or_workspace() {
        for accepted in [
            ConflictResolutionSide::Store,
            ConflictResolutionSide::Workspace,
        ] {
            let workspace = tempfile::tempdir().expect("workspace");
            let store = tempfile::tempdir().expect("store");
            let source = store.path().join("file.txt");
            let target = workspace.path().join("packages/repo/file.txt");
            std::fs::create_dir_all(target.parent().unwrap()).expect("target parent");
            std::fs::write(&target, "workspace file").expect("target");
            conflict_state(workspace.path(), &source, &target)
                .save()
                .expect("state");

            resolve_in_workspace(workspace.path(), None, accepted, &[]).expect("resolve");

            if accepted == ConflictResolutionSide::Store {
                assert!(!source.exists());
                assert!(!target.exists());
            } else {
                assert_eq!(
                    engine::get_file_inode(&source),
                    engine::get_file_inode(&target)
                );
                assert_eq!(std::fs::read_to_string(source).unwrap(), "workspace file");
            }
        }
    }
}
