use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::manager::get_vdw_home;

pub const WORKSPACE_REGISTRY_VERSION: u32 = 1;
const WORKSPACE_REGISTRY_FILE: &str = "workspaces.json";
const WORKSPACE_REGISTRY_LOCK_FILE: &str = "workspaces.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRegistry {
    pub version: u32,
    pub workspaces: Vec<WorkspaceRegistration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRegistration {
    pub path: PathBuf,
    pub last_seen: DateTime<Utc>,
}

impl WorkspaceRegistry {
    pub fn empty() -> Self {
        Self {
            version: WORKSPACE_REGISTRY_VERSION,
            workspaces: Vec::new(),
        }
    }
}

pub fn load_workspace_registry() -> Result<WorkspaceRegistry> {
    load_workspace_registry_at(&get_vdw_home()?)
}

pub fn load_workspace_registry_at(vdw_home: &Path) -> Result<WorkspaceRegistry> {
    let path = vdw_home.join(WORKSPACE_REGISTRY_FILE);
    if !path.exists() {
        return Ok(WorkspaceRegistry::empty());
    }

    let content = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read workspace registry at {}", path.display()))?;
    let registry: WorkspaceRegistry = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse workspace registry at {}", path.display()))?;
    if registry.version != WORKSPACE_REGISTRY_VERSION {
        bail!(
            "Unsupported workspace registry version {}; expected {}",
            registry.version,
            WORKSPACE_REGISTRY_VERSION
        );
    }
    Ok(registry)
}

pub fn register_workspace(workspace_root: &Path) -> Result<()> {
    register_workspace_at(&get_vdw_home()?, workspace_root, Utc::now())
}

pub fn register_workspace_at(
    vdw_home: &Path,
    workspace_root: &Path,
    last_seen: DateTime<Utc>,
) -> Result<()> {
    fs::create_dir_all(vdw_home)
        .with_context(|| format!("Failed to create VDW home at {}", vdw_home.display()))?;
    let workspace_root = canonical_workspace_path(workspace_root)?;
    let lock_path = vdw_home.join(WORKSPACE_REGISTRY_LOCK_FILE);
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("Failed to open registry lock at {}", lock_path.display()))?;
    lock_file.lock_exclusive().with_context(|| {
        format!(
            "Failed to lock workspace registry at {}",
            lock_path.display()
        )
    })?;

    let mut registry = load_workspace_registry_at(vdw_home)?;
    if let Some(existing) = registry
        .workspaces
        .iter_mut()
        .find(|workspace| workspace.path == workspace_root)
    {
        existing.last_seen = last_seen;
    } else {
        registry.workspaces.push(WorkspaceRegistration {
            path: workspace_root,
            last_seen,
        });
    }
    registry
        .workspaces
        .sort_by(|left, right| left.path.cmp(&right.path));
    save_workspace_registry_at(vdw_home, &registry)
}

fn save_workspace_registry_at(vdw_home: &Path, registry: &WorkspaceRegistry) -> Result<()> {
    if registry.version != WORKSPACE_REGISTRY_VERSION {
        bail!(
            "Refusing to write workspace registry version {}; expected {}",
            registry.version,
            WORKSPACE_REGISTRY_VERSION
        );
    }

    let path = vdw_home.join(WORKSPACE_REGISTRY_FILE);
    let staging = vdw_home.join(format!(
        "{}.tmp-{}",
        WORKSPACE_REGISTRY_FILE,
        std::process::id()
    ));
    let content =
        serde_json::to_string_pretty(registry).context("Failed to serialize workspace registry")?;
    fs::write(&staging, content).with_context(|| {
        format!(
            "Failed to write workspace registry at {}",
            staging.display()
        )
    })?;

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path).with_context(|| {
            format!("Failed to replace workspace registry at {}", path.display())
        })?;
    }

    fs::rename(&staging, &path)
        .with_context(|| format!("Failed to publish workspace registry at {}", path.display()))?;
    Ok(())
}

fn canonical_workspace_path(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path)
        .with_context(|| format!("Cannot register missing workspace {}", path.display()))
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn missing_registry_is_empty() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            load_workspace_registry_at(home.path()).unwrap(),
            WorkspaceRegistry::empty()
        );
    }

    #[test]
    fn register_workspace_roundtrips_and_updates_last_seen() {
        let home = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let first = Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap();
        let second = Utc.with_ymd_and_hms(2026, 9, 2, 10, 0, 0).unwrap();

        register_workspace_at(home.path(), workspace.path(), first).unwrap();
        register_workspace_at(home.path(), workspace.path(), second).unwrap();

        let registry = load_workspace_registry_at(home.path()).unwrap();
        assert_eq!(registry.workspaces.len(), 1);
        assert_eq!(
            registry.workspaces[0].path,
            fs::canonicalize(workspace.path()).unwrap()
        );
        assert_eq!(registry.workspaces[0].last_seen, second);
    }

    #[test]
    fn register_workspace_sorts_paths() {
        let home = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let workspace_b = root.path().join("b");
        let workspace_a = root.path().join("a");
        fs::create_dir_all(&workspace_b).unwrap();
        fs::create_dir_all(&workspace_a).unwrap();

        register_workspace_at(home.path(), &workspace_b, Utc::now()).unwrap();
        register_workspace_at(home.path(), &workspace_a, Utc::now()).unwrap();

        let registry = load_workspace_registry_at(home.path()).unwrap();
        assert!(registry.workspaces[0].path.ends_with("a"));
        assert!(registry.workspaces[1].path.ends_with("b"));
    }

    #[test]
    fn register_workspace_rejects_missing_path() {
        let home = tempfile::tempdir().unwrap();
        let result = register_workspace_at(home.path(), &home.path().join("missing"), Utc::now());
        assert!(result.is_err());
    }

    #[test]
    fn load_rejects_unsupported_registry_version() {
        let home = tempfile::tempdir().unwrap();
        fs::write(
            home.path().join(WORKSPACE_REGISTRY_FILE),
            r#"{"version":2,"workspaces":[]}"#,
        )
        .unwrap();

        assert!(load_workspace_registry_at(home.path()).is_err());
    }
}
