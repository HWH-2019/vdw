use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use super::manager::get_vdw_home;

pub const WORKSPACE_REGISTRY_VERSION: u32 = 2;
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
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default = "active")]
    pub lifecycle_state: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub observation_started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_active_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub activity_source: String,
    #[serde(default)]
    pub activity_revision: u64,
    #[serde(default)]
    pub latest_archive_id: Option<String>,
    #[serde(default)]
    pub current_operation_id: Option<String>,
    #[serde(default)]
    pub candidate_since: Option<DateTime<Utc>>,
    #[serde(default)]
    pub observed_fingerprint: Option<String>,
}
fn active() -> String {
    "active".into()
}
impl WorkspaceRegistration {
    pub fn new(path: PathBuf, now: DateTime<Utc>) -> Self {
        let name = crate::config::parser::load_config(&path)
            .map(|c| c.workspace.name)
            .unwrap_or_else(|_| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
        Self {
            path,
            last_seen: now,
            workspace_id: crate::lifecycle::io::id("ws"),
            name,
            lifecycle_state: active(),
            created_at: Some(now),
            observation_started_at: Some(now),
            last_active_at: Some(now),
            activity_source: "registration".into(),
            activity_revision: 1,
            latest_archive_id: None,
            current_operation_id: None,
            candidate_since: None,
            observed_fingerprint: None,
        }
    }
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
    let mut registry: WorkspaceRegistry = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse workspace registry at {}", path.display()))?;
    if registry.version != 1 && registry.version != WORKSPACE_REGISTRY_VERSION {
        bail!(
            "Unsupported workspace registry version {}; expected {}",
            registry.version,
            WORKSPACE_REGISTRY_VERSION
        );
    }
    if registry.version == 1 {
        let now = Utc::now();
        for ws in &mut registry.workspaces {
            let mut migrated = WorkspaceRegistration::new(ws.path.clone(), now);
            migrated.workspace_id = format!(
                "ws-{}",
                &crate::lifecycle::io::hash(ws.path.to_string_lossy().as_bytes())[..24]
            );
            migrated.last_seen = ws.last_seen;
            migrated.activity_source = "legacy_observation".into();
            *ws = migrated;
        }
        registry.version = WORKSPACE_REGISTRY_VERSION;
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
        if existing.lifecycle_state == "active" {
            existing.last_active_at = Some(last_seen);
            existing.activity_revision += 1;
            existing.activity_source = "vdw_mutation".into();
            existing.candidate_since = None;
        }
    } else {
        registry
            .workspaces
            .push(WorkspaceRegistration::new(workspace_root, last_seen));
    }
    registry
        .workspaces
        .sort_by(|left, right| left.path.cmp(&right.path));
    save_workspace_registry_at(vdw_home, &registry)
}

pub fn save_workspace_registry_at(vdw_home: &Path, registry: &WorkspaceRegistry) -> Result<()> {
    if registry.version != WORKSPACE_REGISTRY_VERSION {
        bail!(
            "Refusing to write workspace registry version {}; expected {}",
            registry.version,
            WORKSPACE_REGISTRY_VERSION
        );
    }

    crate::lifecycle::io::save(&vdw_home.join(WORKSPACE_REGISTRY_FILE), registry)
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
            r#"{"version":999,"workspaces":[]}"#,
        )
        .unwrap();

        assert!(load_workspace_registry_at(home.path()).is_err());
    }
}
