use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use tracing::{info, warn};

use super::manager::get_vdw_home;
use crate::types::config::{EditableLock, EditableLockRegistry};

/// Get the path to the global editable locks file
fn get_locks_path() -> Result<PathBuf> {
    Ok(get_vdw_home()?.join("editable_locks.json"))
}

/// Load the global editable lock registry. Returns empty registry if file doesn't exist or is corrupted.
pub fn load_editable_locks() -> Result<EditableLockRegistry> {
    let path = get_locks_path()?;
    if !path.exists() {
        return Ok(EditableLockRegistry::new());
    }

    let content = fs::read_to_string(&path).context("Failed to read editable_locks.json")?;

    match serde_json::from_str::<EditableLockRegistry>(&content) {
        Ok(registry) => Ok(registry),
        Err(e) => {
            warn!("Failed to parse editable_locks.json, rebuilding: {}", e);
            Ok(EditableLockRegistry::new())
        }
    }
}

/// Save the editable lock registry to disk with file locking for concurrency safety.
pub fn save_editable_locks(registry: &EditableLockRegistry) -> Result<()> {
    let path = get_locks_path()?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
        .context("Failed to open editable_locks.json for writing")?;

    // Acquire exclusive file lock
    file.lock_exclusive()
        .context("Failed to acquire file lock on editable_locks.json")?;

    let content = serde_json::to_string_pretty(registry)
        .context("Failed to serialize editable lock registry")?;

    // Write content
    (&file)
        .write_all(content.as_bytes())
        .context("Failed to write editable_locks.json")?;

    // Lock is released when file is dropped
    Ok(())
}

/// Check if a repo+branch is already locked as editable by another workspace.
/// Returns Some(lock) if conflict found, None otherwise.
/// Automatically cleans up stale locks (where worktree_path no longer exists).
pub fn check_editable_lock(
    repo_id: &str,
    branch: &str,
    current_workspace: &Path,
) -> Result<Option<EditableLock>> {
    let mut registry = load_editable_locks()?;
    let current_ws = current_workspace.to_string_lossy().to_string();
    let mut stale_indices = Vec::new();
    let mut conflict: Option<EditableLock> = None;

    for (i, lock) in registry.locks.iter().enumerate() {
        if lock.repo_id == repo_id && lock.branch == branch {
            // Same workspace doesn't count as conflict (allows relink)
            if lock.workspace_path == current_ws {
                continue;
            }
            // Check worktree_path as primary validity judge
            if !Path::new(&lock.worktree_path).exists() {
                // Stale lock, mark for cleanup
                stale_indices.push(i);
                continue;
            }
            conflict = Some(lock.clone());
        }
    }

    // Clean up stale locks if any found
    if !stale_indices.is_empty() {
        for i in stale_indices.into_iter().rev() {
            let removed = registry.locks.remove(i);
            info!(
                "Cleaned up stale editable lock: {} branch '{}'",
                removed.repo_id, removed.branch
            );
        }
        save_editable_locks(&registry)?;
    }

    Ok(conflict)
}

/// Acquire an editable lock for the given repo+branch in the current workspace.
pub fn acquire_editable_lock(
    repo_id: &str,
    branch: &str,
    workspace_path: &Path,
    worktree_path: &Path,
) -> Result<()> {
    let mut registry = load_editable_locks()?;

    let ws = workspace_path.to_string_lossy().to_string();
    let wt = worktree_path.to_string_lossy().to_string();

    // Check if lock already exists for this workspace (update it)
    if let Some(existing) = registry
        .locks
        .iter_mut()
        .find(|l| l.repo_id == repo_id && l.branch == branch && l.workspace_path == ws)
    {
        existing.worktree_path = wt;
        existing.created_at = chrono::Utc::now().to_rfc3339();
    } else {
        registry.locks.push(EditableLock {
            repo_id: repo_id.to_string(),
            branch: branch.to_string(),
            workspace_path: ws,
            worktree_path: wt,
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    save_editable_locks(&registry)?;
    info!(
        "Acquired editable lock: {} branch '{}' for workspace {:?}",
        repo_id, branch, workspace_path
    );
    Ok(())
}

/// Release the editable lock for the given repo+branch held by the current workspace.
pub fn release_editable_lock(repo_id: &str, branch: &str, workspace_path: &Path) -> Result<()> {
    let mut registry = load_editable_locks()?;
    let ws = workspace_path.to_string_lossy().to_string();

    let before_len = registry.locks.len();
    registry
        .locks
        .retain(|l| !(l.repo_id == repo_id && l.branch == branch && l.workspace_path == ws));

    if registry.locks.len() < before_len {
        save_editable_locks(&registry)?;
        info!(
            "Released editable lock: {} branch '{}' for workspace {:?}",
            repo_id, branch, workspace_path
        );
    }

    Ok(())
}

/// Clean up all stale locks where worktree_path no longer exists.
pub fn cleanup_stale_locks() -> Result<Vec<EditableLock>> {
    let mut registry = load_editable_locks()?;
    let mut stale = Vec::new();

    registry.locks.retain(|lock| {
        if !Path::new(&lock.worktree_path).exists() {
            stale.push(lock.clone());
            false
        } else {
            true
        }
    });

    if !stale.is_empty() {
        save_editable_locks(&registry)?;
        for lock in &stale {
            info!(
                "Cleaned stale lock: {} branch '{}'",
                lock.repo_id, lock.branch
            );
        }
    }

    Ok(stale)
}
