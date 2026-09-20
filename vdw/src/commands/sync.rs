use std::collections::HashSet;

use anyhow::{bail, Result};
use serde::Serialize;

use crate::config::{env_resolver, parser, validator};
use crate::lockfile::{LockedDependency, LockedRevision, WorkspaceLock, LOCK_FILE_NAME};
use crate::store::{manager as store_manager, object_pool};

#[derive(Debug, Serialize)]
struct SyncReport {
    lock_file: &'static str,
    mode: &'static str,
    changed: bool,
    dependencies: Vec<SyncDependencyReport>,
    removed_dependencies: Vec<LockedDependency>,
}

#[derive(Debug, Serialize)]
struct SyncDependencyReport {
    repo: String,
    path: String,
    editable: bool,
    revision: LockedRevision,
    locked_commit: Option<String>,
    resolved_commit: String,
    status: SyncStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SyncStatus {
    Added,
    Unchanged,
    Updated,
    ConfigChanged,
}

pub fn execute(dry_run: bool, update_lock: bool, json: bool) -> Result<()> {
    if dry_run && update_lock {
        bail!("--dry-run and --update-lock cannot be used together");
    }

    let workspace_root = std::env::current_dir()?;
    let mut config = parser::load_config(&workspace_root)?;
    env_resolver::resolve_env_vars(&mut config)?;
    if let Err(errors) = validator::validate_config(&config) {
        bail!(
            "Cannot sync an invalid workspace configuration: {}",
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
    }

    let store_path = store_manager::get_store_path()?;
    if !store_path.exists() {
        store_manager::init_store()?;
    }

    let previous_lock = WorkspaceLock::load(&workspace_root)?.unwrap_or_else(WorkspaceLock::empty);
    let mut next_lock = WorkspaceLock::empty();
    let mut reports = Vec::with_capacity(config.dependencies.len());
    let configured_paths = config
        .dependencies
        .iter()
        .map(|dependency| dependency.path.as_str())
        .collect::<HashSet<_>>();
    let removed_dependencies = previous_lock
        .dependencies
        .iter()
        .filter(|dependency| !configured_paths.contains(dependency.path.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    for dependency in &config.dependencies {
        let repo_dir = object_pool::clone_to_store(
            dependency.dep_id(),
            &store_path,
            dependency.branch.as_deref(),
            None,
        )?;
        let resolved_commit =
            object_pool::refresh_dependency_revision(&repo_dir.join("bare.git"), dependency)?
                .to_string();
        let locked = LockedDependency::from_dependency(dependency, resolved_commit.clone())?;
        let previous = previous_lock.entry_for_path(&dependency.path);
        let status = match previous {
            None => SyncStatus::Added,
            Some(previous) if !previous.matches(dependency) => SyncStatus::ConfigChanged,
            Some(previous) if previous.resolved_commit == resolved_commit => SyncStatus::Unchanged,
            Some(_) => SyncStatus::Updated,
        };

        reports.push(SyncDependencyReport {
            repo: dependency.dep_id().to_string(),
            path: dependency.path.clone(),
            editable: dependency.editable,
            revision: LockedRevision::from_dependency(dependency)?,
            locked_commit: previous.map(|entry| entry.resolved_commit.clone()),
            resolved_commit,
            status,
        });
        next_lock.upsert(locked);
    }

    let changed = reports
        .iter()
        .any(|report| report.status != SyncStatus::Unchanged)
        || !removed_dependencies.is_empty();
    if update_lock {
        next_lock.save(&workspace_root)?;
    }

    let report = SyncReport {
        lock_file: LOCK_FILE_NAME,
        mode: if update_lock {
            "update_lock"
        } else {
            "dry_run"
        },
        changed,
        dependencies: reports,
        removed_dependencies,
    };
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_human_report(&report, update_lock);
    }
    Ok(())
}

fn print_human_report(report: &SyncReport, updated: bool) {
    if report.dependencies.is_empty() {
        println!("No dependencies configured.");
    }
    for dependency in &report.dependencies {
        let old = dependency.locked_commit.as_deref().unwrap_or("(unlocked)");
        let marker = match dependency.status {
            SyncStatus::Added => "+",
            SyncStatus::Unchanged => "=",
            SyncStatus::Updated => "↑",
            SyncStatus::ConfigChanged => "!",
        };
        println!(
            "{} {} {} -> {}",
            marker, dependency.path, old, dependency.resolved_commit
        );
    }
    for dependency in &report.removed_dependencies {
        println!(
            "- {} {} -> (removed)",
            dependency.path, dependency.resolved_commit
        );
    }

    if updated {
        println!("✅ {} updated.", report.lock_file);
    } else if report.changed {
        println!("Lock changes detected. Run `vdw sync --update-lock` to accept them.");
    } else {
        println!("✅ {} is up to date.", report.lock_file);
    }
}
