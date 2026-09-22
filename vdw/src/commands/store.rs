use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use walkdir::WalkDir;

use crate::config::{env_resolver, parser, validator};
use crate::lockfile::WorkspaceLock;
use crate::store::manager::{self, GlobalConfig};
use crate::store::object_pool;
use crate::store::registry::{self, WorkspaceRegistration};
use crate::types::store::StoreMetadata;
use crate::worktree::manager as worktree_manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegisteredWorkspaceState {
    Ready,
    Missing,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegisteredWorkspaceStatus {
    pub path: PathBuf,
    pub last_seen: DateTime<Utc>,
    pub state: RegisteredWorkspaceState,
    pub config_reference_count: usize,
    pub lock_reference_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreObjectState {
    Referenced,
    RetainedRecent,
    GcCandidate,
    Unverified,
    Invalid,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreObjectStatus {
    pub name: String,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    pub size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_accessed: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_days: Option<i64>,
    pub config_references: Vec<PathBuf>,
    pub lock_references: Vec<PathBuf>,
    pub worktree_paths: Vec<PathBuf>,
    pub state: StoreObjectState,
    pub gc_candidate: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreSummary {
    pub object_count: usize,
    pub total_size_bytes: u64,
    pub referenced_count: usize,
    pub gc_candidate_count: usize,
    pub invalid_count: usize,
    pub max_store_size_bytes: u64,
    pub exceeds_configured_size: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct StoreStatusReport {
    pub store_path: PathBuf,
    pub registry_path: PathBuf,
    pub workspace_registry_present: bool,
    pub reference_inventory_complete: bool,
    pub gc_threshold_days: u32,
    pub registered_workspaces: Vec<RegisteredWorkspaceStatus>,
    pub objects: Vec<StoreObjectStatus>,
    pub summary: StoreSummary,
}

#[derive(Debug, Serialize)]
struct StoreGcDryRunReport {
    mode: &'static str,
    deletion_performed: bool,
    store_path: PathBuf,
    reference_inventory_complete: bool,
    gc_threshold_days: u32,
    reclaimable_size_bytes: u64,
    candidates: Vec<StoreObjectStatus>,
}

pub fn status(json: bool) -> Result<()> {
    let vdw_home = manager::get_vdw_home()?;
    let store_path = manager::get_store_path()?;
    let config = manager::load_global_config()?;
    let report = build_store_status(&vdw_home, &store_path, &config, Utc::now())?;

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_status(&report);
    }
    Ok(())
}

pub fn gc_dry_run(dry_run: bool, json: bool) -> Result<()> {
    if !dry_run {
        bail!(
            "Store GC is plan-only in v0.9.1; pass --dry-run to inspect eligible objects without deleting anything"
        );
    }

    let vdw_home = manager::get_vdw_home()?;
    let store_path = manager::get_store_path()?;
    let config = manager::load_global_config()?;
    let status = build_store_status(&vdw_home, &store_path, &config, Utc::now())?;
    let candidates = status
        .objects
        .iter()
        .filter(|object| object.gc_candidate)
        .cloned()
        .collect::<Vec<_>>();
    let report = StoreGcDryRunReport {
        mode: "dry_run",
        deletion_performed: false,
        store_path: status.store_path,
        reference_inventory_complete: status.reference_inventory_complete,
        gc_threshold_days: status.gc_threshold_days,
        reclaimable_size_bytes: candidates.iter().map(|object| object.size_bytes).sum(),
        candidates,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Store GC dry run: {}", report.store_path.display());
        if !report.reference_inventory_complete {
            println!(
                "No objects are eligible because the workspace reference inventory is incomplete."
            );
        } else if report.candidates.is_empty() {
            println!(
                "No unreferenced objects exceeded the {}-day retention threshold.",
                report.gc_threshold_days
            );
        } else {
            for candidate in &report.candidates {
                println!(
                    "- {} ({} bytes, {} days old)",
                    candidate.name,
                    candidate.size_bytes,
                    candidate.age_days.unwrap_or_default()
                );
            }
            println!(
                "Would reclaim {} bytes. No files were deleted.",
                report.reclaimable_size_bytes
            );
        }
    }
    Ok(())
}

pub fn build_store_status(
    vdw_home: &Path,
    store_path: &Path,
    config: &GlobalConfig,
    now: DateTime<Utc>,
) -> Result<StoreStatusReport> {
    let registry_path = vdw_home.join("workspaces.json");
    let workspace_registry_present = registry_path.exists();
    let registry = registry::load_workspace_registry_at(vdw_home)?;
    let mut config_references: HashMap<String, BTreeSet<PathBuf>> = HashMap::new();
    let mut lock_references: HashMap<String, BTreeSet<PathBuf>> = HashMap::new();
    let mut registered_workspaces = Vec::with_capacity(registry.workspaces.len());

    for registration in registry.workspaces {
        registered_workspaces.push(inspect_workspace(
            &registration,
            &mut config_references,
            &mut lock_references,
        ));
    }
    let reference_inventory_complete = workspace_registry_present
        && registered_workspaces
            .iter()
            .all(|workspace| workspace.state == RegisteredWorkspaceState::Ready);

    let mut objects = inspect_store_objects(
        store_path,
        config.gc_threshold_days,
        now,
        reference_inventory_complete,
        &config_references,
        &lock_references,
    )?;
    objects.sort_by(|left, right| left.name.cmp(&right.name));

    let total_size_bytes = objects.iter().map(|object| object.size_bytes).sum();
    let max_store_size_bytes = config.max_store_size_gb.saturating_mul(1024 * 1024 * 1024);
    let summary = StoreSummary {
        object_count: objects.len(),
        total_size_bytes,
        referenced_count: objects
            .iter()
            .filter(|object| object.state == StoreObjectState::Referenced)
            .count(),
        gc_candidate_count: objects.iter().filter(|object| object.gc_candidate).count(),
        invalid_count: objects
            .iter()
            .filter(|object| object.state == StoreObjectState::Invalid)
            .count(),
        max_store_size_bytes,
        exceeds_configured_size: total_size_bytes > max_store_size_bytes,
    };

    Ok(StoreStatusReport {
        store_path: store_path.to_path_buf(),
        registry_path,
        workspace_registry_present,
        reference_inventory_complete,
        gc_threshold_days: config.gc_threshold_days,
        registered_workspaces,
        objects,
        summary,
    })
}

fn inspect_workspace(
    registration: &WorkspaceRegistration,
    config_references: &mut HashMap<String, BTreeSet<PathBuf>>,
    lock_references: &mut HashMap<String, BTreeSet<PathBuf>>,
) -> RegisteredWorkspaceStatus {
    if !registration.path.exists() {
        return RegisteredWorkspaceStatus {
            path: registration.path.clone(),
            last_seen: registration.last_seen,
            state: RegisteredWorkspaceState::Missing,
            config_reference_count: 0,
            lock_reference_count: 0,
            message: Some("registered workspace path is unavailable".to_string()),
        };
    }

    let mut messages = Vec::new();
    let config_reference_count =
        match parser::load_config(&registration.path).and_then(|mut config| {
            env_resolver::resolve_env_vars(&mut config)?;
            if let Err(errors) = validator::validate_config(&config) {
                bail!(
                    "configuration validation failed: {}",
                    errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
            Ok(config)
        }) {
            Ok(config) => {
                for dependency in &config.dependencies {
                    config_references
                        .entry(dependency.dep_id().to_string())
                        .or_default()
                        .insert(registration.path.clone());
                }
                config.dependencies.len()
            }
            Err(error) => {
                messages.push(format!("configuration: {}", error));
                0
            }
        };

    let lock_reference_count = match WorkspaceLock::load(&registration.path) {
        Ok(Some(workspace_lock)) => {
            for dependency in &workspace_lock.dependencies {
                lock_references
                    .entry(dependency.repo.clone())
                    .or_default()
                    .insert(registration.path.clone());
            }
            workspace_lock.dependencies.len()
        }
        Ok(None) => 0,
        Err(error) => {
            messages.push(format!("lock file: {}", error));
            0
        }
    };

    RegisteredWorkspaceStatus {
        path: registration.path.clone(),
        last_seen: registration.last_seen,
        state: if messages.is_empty() {
            RegisteredWorkspaceState::Ready
        } else {
            RegisteredWorkspaceState::Invalid
        },
        config_reference_count,
        lock_reference_count,
        message: (!messages.is_empty()).then(|| messages.join("; ")),
    }
}

fn inspect_store_objects(
    store_path: &Path,
    gc_threshold_days: u32,
    now: DateTime<Utc>,
    reference_inventory_complete: bool,
    config_references: &HashMap<String, BTreeSet<PathBuf>>,
    lock_references: &HashMap<String, BTreeSet<PathBuf>>,
) -> Result<Vec<StoreObjectStatus>> {
    let objects_path = store_path.join("objects");
    if !objects_path.exists() {
        return Ok(Vec::new());
    }

    let mut objects = Vec::new();
    for entry in fs::read_dir(&objects_path)
        .with_context(|| format!("Failed to read Store objects at {}", objects_path.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        objects.push(inspect_store_object(
            store_path,
            &entry.path(),
            gc_threshold_days,
            now,
            reference_inventory_complete,
            config_references,
            lock_references,
        ));
    }
    Ok(objects)
}

fn inspect_store_object(
    store_path: &Path,
    object_path: &Path,
    gc_threshold_days: u32,
    now: DateTime<Utc>,
    reference_inventory_complete: bool,
    config_references: &HashMap<String, BTreeSet<PathBuf>>,
    lock_references: &HashMap<String, BTreeSet<PathBuf>>,
) -> StoreObjectStatus {
    let name = object_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| object_path.display().to_string());
    let size_bytes = directory_size(object_path);
    let metadata = match StoreMetadata::load(object_path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return invalid_object(
                name,
                object_path,
                size_bytes,
                format!("metadata is unavailable or invalid: {}", error),
            )
        }
    };
    let repo = metadata.repo_url.clone();
    if repo.is_empty() || object_pool::repo_store_dir(&repo, store_path) != object_path {
        return invalid_object(
            name,
            object_path,
            size_bytes,
            "metadata repository identifier does not match the Store object path".to_string(),
        );
    }
    let config_references = config_references
        .get(&repo)
        .map(|paths| paths.iter().cloned().collect())
        .unwrap_or_default();
    let lock_references = lock_references
        .get(&repo)
        .map(|paths| paths.iter().cloned().collect())
        .unwrap_or_default();
    let bare_path = object_path.join("bare.git");
    let worktree_paths = match worktree_manager::list_worktrees(&bare_path) {
        Ok(worktrees) => worktrees
            .into_iter()
            .map(|worktree| worktree.path)
            .collect::<Vec<_>>(),
        Err(error) => {
            return StoreObjectStatus {
                name,
                path: object_path.to_path_buf(),
                repo: Some(repo),
                size_bytes,
                last_accessed: Some(metadata.last_accessed),
                age_days: Some(age_days(now, metadata.last_accessed)),
                config_references,
                lock_references,
                worktree_paths: Vec::new(),
                state: StoreObjectState::Invalid,
                gc_candidate: false,
                reason: format!("bare repository could not be inspected: {}", error),
            }
        }
    };
    let age_days = age_days(now, metadata.last_accessed);
    let has_references =
        !config_references.is_empty() || !lock_references.is_empty() || !worktree_paths.is_empty();
    let (state, gc_candidate, reason) = if has_references {
        (
            StoreObjectState::Referenced,
            false,
            "protected by workspace configuration, lock file, or Git worktree".to_string(),
        )
    } else if !reference_inventory_complete {
        (
            StoreObjectState::Unverified,
            false,
            "workspace reference inventory is incomplete".to_string(),
        )
    } else if age_days < i64::from(gc_threshold_days) {
        (
            StoreObjectState::RetainedRecent,
            false,
            format!("within the {}-day retention window", gc_threshold_days),
        )
    } else {
        (
            StoreObjectState::GcCandidate,
            true,
            format!(
                "no registered config, lock, or worktree references and older than {} days",
                gc_threshold_days
            ),
        )
    };

    StoreObjectStatus {
        name,
        path: object_path.to_path_buf(),
        repo: Some(repo),
        size_bytes,
        last_accessed: Some(metadata.last_accessed),
        age_days: Some(age_days),
        config_references,
        lock_references,
        worktree_paths,
        state,
        gc_candidate,
        reason,
    }
}

fn invalid_object(
    name: String,
    object_path: &Path,
    size_bytes: u64,
    reason: String,
) -> StoreObjectStatus {
    StoreObjectStatus {
        name,
        path: object_path.to_path_buf(),
        repo: None,
        size_bytes,
        last_accessed: None,
        age_days: None,
        config_references: Vec::new(),
        lock_references: Vec::new(),
        worktree_paths: Vec::new(),
        state: StoreObjectState::Invalid,
        gc_candidate: false,
        reason,
    }
}

fn age_days(now: DateTime<Utc>, last_accessed: DateTime<Utc>) -> i64 {
    now.signed_duration_since(last_accessed).num_days().max(0)
}

fn directory_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

fn print_status(report: &StoreStatusReport) {
    println!("Store: {}", report.store_path.display());
    println!(
        "Objects: {} ({} bytes), referenced: {}, GC candidates: {}, invalid: {}",
        report.summary.object_count,
        report.summary.total_size_bytes,
        report.summary.referenced_count,
        report.summary.gc_candidate_count,
        report.summary.invalid_count
    );
    println!(
        "Workspace registry: {} workspace(s), inventory {}",
        report.registered_workspaces.len(),
        if report.reference_inventory_complete {
            "complete"
        } else {
            "incomplete"
        }
    );
    for workspace in &report.registered_workspaces {
        println!("- {:?}: {}", workspace.state, workspace.path.display());
    }
    if report.summary.exceeds_configured_size {
        println!(
            "Warning: Store exceeds the configured {} byte limit.",
            report.summary.max_store_size_bytes
        );
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    fn write_workspace(workspace: &Path, config_repo: &str, lock_repo: &str) {
        let config = format!(
            r#"workspace:
  name: store-test
  version: "1.0"
dependencies:
  - remote: "{}"
    path: packages/config-repo
    branch: main
    scope:
      include:
        - "**/*"
"#,
            config_repo
        );
        fs::write(workspace.join(".vdwrc"), config).unwrap();
        let lock = format!(
            r#"version: 1
dependencies:
  - repo: "{}"
    path: packages/lock-repo
    editable: false
    revision:
      kind: branch
      value: main
    resolved_commit: "1111111111111111111111111111111111111111"
"#,
            lock_repo
        );
        fs::write(workspace.join(".vdw.lock"), lock).unwrap();
    }

    fn create_store_object(store: &Path, repo: &str, last_accessed: DateTime<Utc>) -> PathBuf {
        let object = object_pool::repo_store_dir(repo, store);
        fs::create_dir_all(&object).unwrap();
        git2::Repository::init_bare(object.join("bare.git")).unwrap();
        StoreMetadata {
            repo_url: repo.to_string(),
            commit_hash: "1111111111111111111111111111111111111111".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            reference_count: 0,
            last_accessed,
            created_at: last_accessed,
            size_bytes: 0,
            alternates_configured: false,
        }
        .save(&object)
        .unwrap();
        object
    }

    fn write_empty_workspace(workspace: &Path) {
        fs::write(
            workspace.join(".vdwrc"),
            "workspace:\n  name: empty\n  version: '1.0'\ndependencies: []\n",
        )
        .unwrap();
    }

    #[test]
    fn status_protects_references_and_only_marks_old_unreferenced_objects() {
        let root = tempfile::tempdir().unwrap();
        let vdw_home = root.path().join("home");
        let store = root.path().join("store");
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let config_repo = "https://example.com/config.git";
        let lock_repo = "https://example.com/lock.git";
        let old_repo = "https://example.com/old.git";
        let recent_repo = "https://example.com/recent.git";
        write_workspace(&workspace, config_repo, lock_repo);
        registry::register_workspace_at(&vdw_home, &workspace, Utc::now()).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();
        create_store_object(&store, config_repo, now - Duration::days(90));
        create_store_object(&store, lock_repo, now - Duration::days(90));
        create_store_object(&store, old_repo, now - Duration::days(31));
        create_store_object(&store, recent_repo, now - Duration::days(29));
        let malformed = store.join("objects/malformed");
        fs::create_dir_all(&malformed).unwrap();

        let report = build_store_status(&vdw_home, &store, &GlobalConfig::default(), now).unwrap();

        assert!(report.reference_inventory_complete);
        assert_eq!(report.summary.object_count, 5);
        assert_eq!(report.summary.referenced_count, 2);
        assert_eq!(report.summary.gc_candidate_count, 1);
        assert_eq!(report.summary.invalid_count, 1);
        let candidate = report
            .objects
            .iter()
            .find(|object| object.gc_candidate)
            .unwrap();
        assert_eq!(candidate.repo.as_deref(), Some(old_repo));
        assert_eq!(candidate.age_days, Some(31));
        assert!(malformed.exists(), "status must never delete Store objects");
    }

    #[test]
    fn missing_registry_prevents_gc_candidates() {
        let root = tempfile::tempdir().unwrap();
        let vdw_home = root.path().join("home");
        let store = root.path().join("store");
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();
        create_store_object(
            &store,
            "https://example.com/old.git",
            now - Duration::days(365),
        );

        let report = build_store_status(&vdw_home, &store, &GlobalConfig::default(), now).unwrap();

        assert!(!report.workspace_registry_present);
        assert!(!report.reference_inventory_complete);
        assert_eq!(report.summary.gc_candidate_count, 0);
        assert_eq!(report.objects[0].state, StoreObjectState::Unverified);
    }

    #[test]
    fn unavailable_registered_workspace_prevents_gc_candidates() {
        let root = tempfile::tempdir().unwrap();
        let vdw_home = root.path().join("home");
        let store = root.path().join("store");
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        registry::register_workspace_at(&vdw_home, &workspace, Utc::now()).unwrap();
        fs::remove_dir(&workspace).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();
        create_store_object(
            &store,
            "https://example.com/old.git",
            now - Duration::days(365),
        );

        let report = build_store_status(&vdw_home, &store, &GlobalConfig::default(), now).unwrap();

        assert!(!report.reference_inventory_complete);
        assert_eq!(report.summary.gc_candidate_count, 0);
        assert_eq!(
            report.registered_workspaces[0].state,
            RegisteredWorkspaceState::Missing
        );
    }

    #[test]
    fn registered_git_worktree_protects_an_old_object() {
        let root = tempfile::tempdir().unwrap();
        let vdw_home = root.path().join("home");
        let store = root.path().join("store");
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write_empty_workspace(&workspace);
        registry::register_workspace_at(&vdw_home, &workspace, Utc::now()).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();
        let object = create_store_object(
            &store,
            "https://example.com/worktree.git",
            now - Duration::days(365),
        );
        let bare = git2::Repository::open_bare(object.join("bare.git")).unwrap();
        let blob = bare.blob(b"protected").unwrap();
        let mut builder = bare.treebuilder(None).unwrap();
        builder.insert("README.md", blob, 0o100644).unwrap();
        let tree = bare.find_tree(builder.write().unwrap()).unwrap();
        let signature = git2::Signature::now("VDW Test", "vdw@example.com").unwrap();
        bare.commit(
            Some("refs/heads/main"),
            &signature,
            &signature,
            "initial",
            &tree,
            &[],
        )
        .unwrap();
        bare.set_head("refs/heads/main").unwrap();
        let worktrees = root.path().join("worktrees");
        worktree_manager::create_worktree(&object.join("bare.git"), "main", &worktrees).unwrap();

        let report = build_store_status(&vdw_home, &store, &GlobalConfig::default(), now).unwrap();

        assert_eq!(report.summary.gc_candidate_count, 0);
        assert_eq!(report.objects[0].state, StoreObjectState::Referenced);
        assert_eq!(report.objects[0].worktree_paths.len(), 1);
    }

    #[test]
    fn mismatched_metadata_path_is_never_a_gc_candidate() {
        let root = tempfile::tempdir().unwrap();
        let vdw_home = root.path().join("home");
        let store = root.path().join("store");
        let workspace = root.path().join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        write_empty_workspace(&workspace);
        registry::register_workspace_at(&vdw_home, &workspace, Utc::now()).unwrap();
        let object = store.join("objects/wrong-name");
        fs::create_dir_all(&object).unwrap();
        git2::Repository::init_bare(object.join("bare.git")).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 9, 0, 0, 0).unwrap();
        StoreMetadata {
            repo_url: "https://example.com/other.git".to_string(),
            commit_hash: "1111111111111111111111111111111111111111".to_string(),
            branch: Some("main".to_string()),
            tag: None,
            reference_count: 0,
            last_accessed: now - Duration::days(365),
            created_at: now - Duration::days(365),
            size_bytes: 0,
            alternates_configured: false,
        }
        .save(&object)
        .unwrap();

        let report = build_store_status(&vdw_home, &store, &GlobalConfig::default(), now).unwrap();

        assert_eq!(report.summary.invalid_count, 1);
        assert!(!report.objects[0].gc_candidate);
    }
}
