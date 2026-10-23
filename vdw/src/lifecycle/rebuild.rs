//! Read-only reconstruction evidence. This report is deliberately not a lifecycle Plan:
//! it has no plan ID, execution route, registration, locks, fetch or materialization.
use super::{io::*, materials, types::*};
use crate::{
    commands::status::{self, TopologyState},
    config::{parser, validator},
    lockfile::WorkspaceLock,
    store::{
        manager, object_pool,
        registry::{self, WorkspaceRegistration},
    },
    types::config::Dependency,
};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize)]
pub struct Preview {
    pub schema_version: u32,
    pub kind: &'static str,
    pub executable: bool,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub auto_archive_enabled: bool,
    pub summary: BTreeMap<String, usize>,
    pub workspaces: Vec<WorkspacePreview>,
}
#[derive(Debug, Serialize)]
pub struct WorkspacePreview {
    pub workspace_id: String,
    pub name: String,
    pub path: PathBuf,
    pub registered: bool,
    pub classification: String,
    pub reasons: Vec<Reason>,
    pub repositories: Vec<RepositoryPreview>,
    pub materials: Vec<Material>,
    pub workspace_dirty: Option<DirtyCounts>,
}
#[derive(Debug, Serialize)]
pub struct Reason {
    pub code: String,
    pub message: String,
}
fn reason(code: &str, message: impl Into<String>) -> Reason {
    Reason {
        code: code.into(),
        message: message.into(),
    }
}
#[derive(Debug, Serialize)]
pub struct Evidence {
    pub source: String,
    pub commit: String,
    /// Candidates are never promoted to an editable worktree's original HEAD.
    pub authority: String,
    pub commit_available: bool,
    pub tree_available: bool,
}
#[derive(Debug, Serialize)]
pub struct RepositoryPreview {
    pub repo: String,
    pub path: String,
    pub editable: bool,
    pub bare: PathBuf,
    pub topology: String,
    pub topology_message: Option<String>,
    pub declared_revision: String,
    pub original_commit: Option<String>,
    pub dirty: Option<DirtyCounts>,
    pub evidence: Vec<Evidence>,
    pub history_truncated: bool,
    pub gitlinks: Vec<String>,
    pub blockers: Vec<Reason>,
}

pub fn preview(selectors: &[String], search_history: bool) -> Result<Preview> {
    let home = manager::get_vdw_home()?;
    let config = manager::load_global_config()?;
    let store = absolute(&manager::get_store_path()?)?;
    let registry = registry::load_workspace_registry_at(&home)?;
    let protections: BTreeMap<String, Protection> =
        read_or(&home.join("lifecycle/protection.json"))?;
    let leases: Vec<Lease> = read_or(&home.join("lifecycle/leases.json"))?;
    let mut selected = vec![];
    if selectors.is_empty() {
        selected.extend(registry.workspaces.iter().cloned().map(|w| (w, true)));
    } else {
        for selector in selectors {
            let path = std::env::current_dir()?.join(selector);
            let matches = registry
                .workspaces
                .iter()
                .filter(|w| w.workspace_id == *selector || w.name == *selector || w.path == path)
                .collect::<Vec<_>>();
            match matches.as_slice() {
                [w] => selected.push(((*w).clone(), true)),
                [] => {
                    let path = absolute(&path)?;
                    let mut ws = WorkspaceRegistration::new(path.clone(), chrono::Utc::now());
                    ws.workspace_id = format!(
                        "unregistered-{}",
                        &hash(path.to_string_lossy().as_bytes())[..24]
                    );
                    selected.push((ws, false));
                }
                _ => bail!("Ambiguous workspace selector: {selector}"),
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut workspaces = vec![];
    for (ws, registered) in selected {
        if !seen.insert(ws.path.clone()) {
            continue;
        }
        let mut row = inspect(
            &ws,
            &store,
            &config.lifecycle.archive.materials.disposable_ignored,
            search_history,
        );
        row.registered = registered;
        if ws.lifecycle_state != "active" && ws.lifecycle_state != "candidate" {
            row.reasons.push(reason(
                "lifecycle_state",
                format!("Workspace state is {}", ws.lifecycle_state),
            ));
            row.classification = "skipped".into();
        }
        if protections
            .get(&ws.workspace_id)
            .map(|p| p.expires_at.map(|e| e > chrono::Utc::now()).unwrap_or(true))
            .unwrap_or(false)
        {
            row.reasons
                .push(reason("protected", "Workspace is on the protection list"));
            row.classification = "skipped".into();
        }
        if ws.current_operation_id.is_some()
            || leases
                .iter()
                .any(|l| l.workspace_id == ws.workspace_id && l.expires_at > chrono::Utc::now())
        {
            row.reasons.push(reason(
                "workspace_busy",
                "Workspace has an operation or active lease",
            ));
            row.classification = "skipped".into();
        }
        workspaces.push(row);
    }
    // Investigate broken worktrees first, without losing dirty-workspace skip results.
    workspaces.sort_by_key(|w| {
        (
            !w.repositories
                .iter()
                .any(|r| r.topology == "invalid_worktree"),
            w.name.clone(),
        )
    });
    let mut summary = BTreeMap::from([
        ("rebuildable".into(), 0),
        ("skipped".into(), 0),
        ("blocked".into(), 0),
    ]);
    for w in &workspaces {
        *summary.entry(w.classification.clone()).or_default() += 1;
    }
    Ok(Preview {
        schema_version: 1,
        kind: "rebuild_preview",
        executable: false,
        generated_at: chrono::Utc::now(),
        auto_archive_enabled: config.lifecycle.auto.enabled,
        summary,
        workspaces,
    })
}

fn inspect(
    ws: &WorkspaceRegistration,
    store: &Path,
    disposable: &[String],
    search_history: bool,
) -> WorkspacePreview {
    let mut row = WorkspacePreview {
        workspace_id: ws.workspace_id.clone(),
        name: ws.name.clone(),
        path: ws.path.clone(),
        registered: true,
        classification: "blocked".into(),
        reasons: vec![],
        repositories: vec![],
        materials: vec![],
        workspace_dirty: None,
    };
    if let Err(err) = inspect_inner(ws, store, disposable, search_history, &mut row) {
        row.reasons
            .push(reason("inspection_failed", format!("{err:#}")));
    }
    let dirty = row
        .workspace_dirty
        .as_ref()
        .map(DirtyCounts::dirty)
        .unwrap_or(false)
        || row
            .repositories
            .iter()
            .filter_map(|r| r.dirty.as_ref())
            .any(DirtyCounts::dirty);
    let unknown = row
        .workspace_dirty
        .as_ref()
        .map(|d| d.unknown_ignored > 0)
        .unwrap_or(false)
        || row
            .repositories
            .iter()
            .filter_map(|r| r.dirty.as_ref())
            .any(|d| d.unknown_ignored > 0);
    let blocked =
        !row.reasons.is_empty() || row.repositories.iter().any(|r| !r.blockers.is_empty());
    if dirty {
        row.classification = "skipped".into();
        row.reasons.push(reason(
            "dirty_workspace",
            "Uncommitted changes skip this entire workspace; other blockers remain listed",
        ));
    } else if row.reasons.is_empty() && row.repositories.iter().all(|r| r.topology == "ready") {
        row.classification = "skipped".into();
        row.reasons.push(reason(
            "healthy_no_rebuild_needed",
            "Existing native worktrees and projections are healthy; preserve them. Repository reconstruction prerequisites remain listed for future review",
        ));
    } else if blocked {
        row.classification = "blocked".into();
    } else if unknown {
        row.classification = "skipped".into();
        row.reasons.push(reason(
            "unknown_ignored",
            "Ignored files outside disposable directories require review",
        ));
    } else {
        row.classification = "rebuildable".into();
    }
    row
}

fn inspect_inner(
    ws: &WorkspaceRegistration,
    store: &Path,
    disposable: &[String],
    search_history: bool,
    row: &mut WorkspacePreview,
) -> Result<()> {
    let root = absolute(&ws.path)?;
    if fs::symlink_metadata(&root)?.file_type().is_symlink() {
        bail!("Symlink workspace root");
    }
    for name in [".vdwrc", ".vdwrc.yml", ".vdw.lock", ".vdw/state.json"] {
        confined(&root, name)?;
    }
    let cfg = parser::load_config(&root)?;
    validator::validate_config(&cfg)
        .map_err(|errors| anyhow::anyhow!("Invalid workspace configuration: {errors:?}"))?;
    let mut roots = vec![];
    for dep in &cfg.dependencies {
        let path = safe_relative(&dep.path)?;
        if roots
            .iter()
            .any(|other: &PathBuf| path.starts_with(other) || other.starts_with(&path))
        {
            bail!("Dependency paths overlap after normalization: {}", dep.path);
        }
        roots.push(path);
    }
    let topology = status::inspect_workspace(&root, store)?;
    let lock = match WorkspaceLock::load(&root) {
        Ok(lock) => lock,
        Err(err) => {
            row.reasons.push(reason("invalid_lock", format!("{err:#}")));
            None
        }
    };
    for (dep, state) in cfg.dependencies.iter().zip(&topology.repositories) {
        let mut repo = RepositoryPreview {
            repo: dep.dep_id().into(),
            path: dep.path.clone(),
            editable: dep.editable,
            bare: object_pool::repo_store_dir(dep.dep_id(), store).join("bare.git"),
            topology: serde_json::to_value(state.topology_state)?
                .as_str()
                .unwrap_or("unknown")
                .into(),
            topology_message: state.message.clone(),
            declared_revision: state.declared_revision.clone(),
            original_commit: None,
            dirty: None,
            evidence: vec![],
            history_truncated: false,
            gitlinks: vec![],
            blockers: vec![],
        };
        if let Err(err) = inspect_repo(
            &root,
            dep,
            state.topology_state,
            lock.as_ref(),
            disposable,
            search_history,
            &mut repo,
        ) {
            repo.blockers
                .push(reason("repository_inspection_failed", format!("{err:#}")));
        }
        row.repositories.push(repo);
    }
    // Preserve all owned files, including custom dependency layouts and extra packages/
    // content. Only declared dependency roots and regenerated runtime state are excluded.
    row.materials = materials::collect(
        &root,
        &cfg.dependencies
            .iter()
            .map(|d| d.path.clone())
            .collect::<Vec<_>>(),
        &MaterialConfig {
            exclude: vec![],
            ..MaterialConfig::default()
        },
    )?;
    if root.join(".git").exists() {
        row.workspace_dirty = Some(dirty(&root, disposable, true)?);
    }
    Ok(())
}

fn inspect_repo(
    root: &Path,
    dep: &Dependency,
    topology: TopologyState,
    lock: Option<&WorkspaceLock>,
    disposable: &[String],
    search_history: bool,
    row: &mut RepositoryPreview,
) -> Result<()> {
    let path = confined(root, &dep.path)?;
    // The topology inspector calls vanished tracked links conflicts. An entirely
    // absent projection can be rebuilt from its exact lock without overwriting files.
    let topology = if !dep.editable && !path.exists() {
        row.topology = "missing".into();
        TopologyState::Missing
    } else {
        topology
    };
    let bare = git2::Repository::open_bare(&row.bare).context("Store repository is unavailable")?;
    if let Some(entry) = lock.and_then(|l| l.matching_entry(dep)) {
        evidence(
            row,
            &bare,
            ".vdw.lock",
            &entry.resolved_commit,
            if dep.editable {
                "baseline"
            } else {
                "exact_projection"
            },
        );
        if !dep.editable {
            row.original_commit = Some(entry.resolved_commit.clone());
        }
    }
    if dep.editable {
        if topology == TopologyState::Ready {
            let repo = git2::Repository::open(&path)?;
            let head = repo.head()?.peel_to_commit()?.id().to_string();
            evidence(row, &bare, "worktree:HEAD", &head, "exact_head");
            row.original_commit = Some(head);
            row.dirty = Some(dirty(&path, disposable, false)?);
        } else {
            row.blockers.push(reason("original_head_unproven", "Original editable HEAD cannot be read; lock baselines, refs and historical commits are candidates only"));
            row.blockers.push(reason("dirty_state_unknown", "Missing or unowned Git metadata prevents proving that the original workspace is clean"));
            if let Some(branch) = &dep.branch {
                for name in [
                    format!("refs/heads/{branch}"),
                    format!("refs/remotes/origin/{branch}"),
                ] {
                    if let Ok(commit) = bare.find_reference(&name).and_then(|r| r.peel_to_commit())
                    {
                        evidence(row, &bare, &name, &commit.id().to_string(), "candidate");
                    }
                    if let Ok(log) = bare.reflog(&name) {
                        for e in log.iter().take(8) {
                            evidence(
                                row,
                                &bare,
                                &format!("reflog:{name}"),
                                &e.id_new().to_string(),
                                "candidate",
                            );
                        }
                    }
                }
                if search_history {
                    // An argument (never shell text), and fixed strings: branch names cannot inject options or regexes.
                    let output = Command::new("git")
                        .arg("--no-optional-locks")
                        .arg("--git-dir")
                        .arg(&row.bare)
                        .args([
                            "log",
                            "--all",
                            "--reflog",
                            "--format=%H",
                            "--max-count=41",
                            "--fixed-strings",
                        ])
                        .arg(format!("--grep={branch}"))
                        .env("GIT_NO_LAZY_FETCH", "1")
                        .output()?;
                    if !output.status.success() {
                        bail!(
                            "Local history search failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    let hits = String::from_utf8(output.stdout)?;
                    row.history_truncated = hits.lines().count() > 40;
                    for oid in hits.lines().take(40) {
                        evidence(row, &bare, "history:branch_message", oid, "candidate");
                    }
                }
            }
        }
    } else {
        if row.original_commit.is_none() {
            row.blockers.push(reason(
                "projection_commit_unknown",
                "Read-only projection requires a matching exact lock entry",
            ));
        }
        if topology != TopologyState::Ready && topology != TopologyState::Missing {
            row.blockers.push(reason(
                "projection_conflict",
                "Existing projection contains drift or conflicts; preserve and review its files",
            ));
        }
        if topology == TopologyState::Missing
            && path.exists()
            && fs::read_dir(&path)?.next().is_some()
        {
            row.blockers.push(reason(
                "projection_target_nonempty",
                "Missing projection has existing files that require review",
            ));
        }
    }
    if let Some(head) = row.original_commit.clone() {
        let commit = bare
            .find_commit(git2::Oid::from_str(&head)?)
            .context("Original commit object is unavailable in Store")?;
        let tree = commit
            .tree()
            .context("Original commit tree is unavailable")?;
        let odb = bare.odb()?;
        let mut missing = false;
        tree.walk(git2::TreeWalkMode::PreOrder, |prefix, entry| {
            if entry.filemode() == 0o160000 {
                row.gitlinks.push(format!(
                    "{}{}@{}",
                    prefix,
                    entry.name().unwrap_or("<non-UTF8>"),
                    entry.id()
                ));
            } else if !odb.exists(entry.id()) {
                missing = true;
            }
            git2::TreeWalkResult::Ok
        })?;
        if missing {
            row.blockers.push(reason(
                "objects_missing",
                "Original tree has unavailable objects; preview does not fetch",
            ));
        }
        // Submodules need an explicit preservation/materialization policy in a future executor.
        if !row.gitlinks.is_empty() {
            row.blockers.push(reason("gitlink_requires_review", "Gitlinks remain at the original commit; rebuilding does not repair their configuration"));
        }
    }
    Ok(())
}

fn evidence(
    row: &mut RepositoryPreview,
    bare: &git2::Repository,
    source: &str,
    oid: &str,
    authority: &str,
) {
    if row
        .evidence
        .iter()
        .any(|e| e.source == source && e.commit == oid)
    {
        return;
    }
    let commit = git2::Oid::from_str(oid)
        .ok()
        .and_then(|oid| bare.find_commit(oid).ok());
    row.evidence.push(Evidence {
        source: source.into(),
        commit: oid.into(),
        authority: authority.into(),
        commit_available: commit.is_some(),
        tree_available: commit.and_then(|c| c.tree().ok()).is_some(),
    });
}

fn dirty(path: &Path, disposable: &[String], workspace: bool) -> Result<DirtyCounts> {
    let repo = git2::Repository::open(path)?;
    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(true)
        .update_index(false);
    let statuses = repo.statuses(Some(&mut options))?;
    let mut d = DirtyCounts::default();
    for entry in statuses.iter() {
        let path = entry.path().context("Non-UTF8 Git path requires review")?;
        if workspace
            && [".vdw/identity.json", ".vdw/state.json", ".vdw/archive.json"].contains(&path)
        {
            continue;
        }
        let s = entry.status();
        if s.contains(git2::Status::IGNORED) {
            if !path
                .trim_end_matches('/')
                .split('/')
                .any(|p| disposable.iter().any(|v| p == v))
            {
                d.unknown_ignored += 1;
            }
            continue;
        }
        if s.contains(git2::Status::CONFLICTED) {
            d.conflicts += 1;
        }
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            d.staged += 1;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            d.unstaged += 1;
        }
        if s.contains(git2::Status::WT_NEW) {
            d.untracked += 1;
        }
    }
    Ok(d)
}

pub fn print(report: &Preview, json: bool) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(report)?);
        return Ok(());
    }
    println!(
        "Reconstruction preview only (not executable): {:?}",
        report.summary
    );
    for ws in &report.workspaces {
        println!("{}  {}  {}", ws.classification, ws.name, ws.path.display());
        for r in &ws.reasons {
            println!("  {}: {}", r.code, r.message);
        }
        for repo in &ws.repositories {
            println!(
                "  {} [{}] original={}",
                repo.path,
                repo.topology,
                repo.original_commit.as_deref().unwrap_or("UNPROVEN")
            );
            for r in &repo.blockers {
                println!("    {}: {}", r.code, r.message);
            }
            for e in &repo.evidence {
                println!("    {} {} ({})", e.commit, e.source, e.authority);
            }
        }
    }
    Ok(())
}
