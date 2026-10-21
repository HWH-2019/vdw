use super::{io::*, materials, types::*};
use crate::{
    config::{env_resolver, parser, validator},
    lockfile::WorkspaceLock,
    store::{
        manager, object_pool,
        registry::{self, WorkspaceRegistration, WorkspaceRegistry},
    },
    types::workspace::WorkspaceState,
};
use anyhow::{bail, Context, Result};
use chrono::{Duration, Utc};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub struct Engine {
    pub background: bool,
    pub home: PathBuf,
    pub store: PathBuf,
    pub config: manager::GlobalConfig,
    pub registry: WorkspaceRegistry,
    pub policies: BTreeMap<String, Policy>,
    pub protections: BTreeMap<String, Protection>,
    pub archives: Vec<ArchiveRecord>,
    pub leases: Vec<Lease>,
    _guard: Guard,
}
impl Engine {
    pub fn open() -> Result<Self> {
        let home = absolute(&manager::get_vdw_home()?)?;
        let guard = Guard::acquire(&home)?;
        let config = manager::load_global_config()?;
        Self::validate_config(&config.lifecycle)?;
        let store = absolute(&manager::get_store_path()?)?;
        let registry = registry::load_workspace_registry_at(&home)?;
        let raw = home.join("workspaces.json");
        if raw.exists() && read::<serde_json::Value>(&raw)?["version"] == 1 {
            atomic_bytes(
                &home.join(format!("workspaces.v1-{}.json", id("backup"))),
                &fs::read(&raw)?,
            )?;
            registry::save_workspace_registry_at(&home, &registry)?;
        }
        let base = home.join("lifecycle");
        let e = Self {
            background: false,
            policies: read_or(&base.join("policies.json"))?,
            protections: read_or(&base.join("protection.json"))?,
            archives: read_or(&base.join("archives.json"))?,
            leases: read_or(&base.join("leases.json"))?,
            home,
            store,
            config,
            registry,
            _guard: guard,
        };
        Ok(e)
    }
    fn write_identity(&self, ws: &WorkspaceRegistration) -> Result<()> {
        let identity = confined(&ws.path, ".vdw/identity.json")?;
        if identity.exists() {
            let value: serde_json::Value = read(&identity)?;
            if value["workspace_id"] != ws.workspace_id {
                bail!("Workspace identity mismatch: {}", ws.path.display());
            }
        } else {
            save(
                &identity,
                &serde_json::json!({"workspace_id":ws.workspace_id}),
            )?;
        }
        Ok(())
    }
    pub fn validate_config(c: &LifecycleConfig) -> Result<()> {
        if c.version != 1
            || c.idle_days == 0
            || c.dirty_policy != "skip_workspace"
            || !c.archive.keep_workspace_skeleton
            || !c.archive.materials.exclude_managed_dependencies
            || !c.memory.index_required
        {
            bail!("Unsupported lifecycle configuration; dirty_policy must be skip_workspace, retain the skeleton and required index");
        }
        if !["store_refs", "portable_bundle"].contains(&c.archive.git_backup.as_str()) {
            bail!("Unsupported git_backup");
        }
        if c.auto.scan_interval_hours == 0
            || c.archive.migration.concurrency != 1
            || c.archive.migration.on_root_change != "auto"
            || !c.archive.migration.cleanup_source_after_verify
        {
            bail!("Invalid scheduler or migration policy");
        }
        if c.cloud.enabled {
            bail!("Cloud provider unsupported: only the reserved capability contract is available");
        }
        materials::validate_patterns(&c.archive.materials.include)?;
        materials::validate_patterns(&c.archive.materials.exclude)?;
        materials::validate_patterns(&c.memory.ai.input_include)?;
        materials::validate_patterns(&c.memory.ai.input_exclude)?;
        if c.memory.ai.enabled {
            if !["after_archive", "manual"].contains(&c.memory.ai.trigger.as_str()) {
                bail!("AI trigger must be after_archive or manual");
            }
            if ![Some("command"), Some("http")].contains(&c.memory.ai.provider.as_deref()) {
                bail!("AI provider must be command or http");
            }
            if c.memory.ai.timeout_seconds == 0
                || c.memory.ai.max_input_chars == 0
                || c.memory.ai.max_output_tokens == 0
                || c.memory.ai.timeout_seconds > 3600
                || c.memory.ai.max_input_chars > 50_000_000
                || c.memory.ai.max_output_tokens > 1_000_000
            {
                bail!("AI limits must be positive");
            }
            if c.memory.ai.provider.as_deref() == Some("command") && c.memory.ai.command.is_empty()
            {
                bail!("AI command argv is empty");
            }
            if c.memory.ai.provider.as_deref() == Some("http")
                && !c
                    .memory
                    .ai
                    .endpoint
                    .as_deref()
                    .map(|v| {
                        v.starts_with("https://")
                            || v.starts_with("http://127.0.0.1:")
                            || v.starts_with("http://localhost:")
                    })
                    .unwrap_or(false)
            {
                bail!("AI HTTP endpoint must use HTTPS (or loopback HTTP)");
            }
        }
        Ok(())
    }
    pub fn base(&self) -> PathBuf {
        self.home.join("lifecycle")
    }
    pub fn persist_registry(&self) -> Result<()> {
        registry::save_workspace_registry_at(&self.home, &self.registry)
    }
    pub fn persist_archives(&self) -> Result<()> {
        save(&self.base().join("archives.json"), &self.archives)
    }
    pub fn config_hash(&self) -> Result<String> {
        Ok(hash(&serde_json::to_vec(&(
            &self.config.lifecycle,
            &self.policies,
        ))?))
    }
    pub fn authorize_hash(&self) -> Result<String> {
        let mut c = self.config.lifecycle.clone();
        c.auto.enabled = false;
        c.auto.authorization = None;
        Ok(hash(&serde_json::to_vec(&(&c, &self.policies))?))
    }
    pub fn resolve_root(&self, root: &str) -> Result<PathBuf> {
        let home = self.home.parent().context("Missing home directory")?;
        absolute(&manager::resolve_workspace_root(root, home)?)
    }
    pub fn archive_root(&self, id: &str) -> Result<PathBuf> {
        let s = self
            .policies
            .get(id)
            .and_then(|p| p.archive_root.as_deref())
            .unwrap_or(&self.config.lifecycle.archive.root);
        let p = self.resolve_root(s)?;
        if p.starts_with(&self.store)
            || self
                .registry
                .workspaces
                .iter()
                .any(|w| p.starts_with(&w.path))
            || self.home.starts_with(&p)
            || self.store.starts_with(&p)
            || self
                .registry
                .workspaces
                .iter()
                .any(|w| w.path.starts_with(&p))
        {
            bail!("Archive location overlaps a workspace, VDW control directory, or Store");
        }
        Ok(p)
    }
    pub fn find(&self, selector: &str) -> Result<WorkspaceRegistration> {
        let mut matches = self
            .registry
            .workspaces
            .iter()
            .filter(|w| {
                w.workspace_id == selector || w.name == selector || w.path == Path::new(selector)
            })
            .cloned();
        let result = matches
            .next()
            .context("Workspace not registered; supply its exact path to import it")?;
        if matches.next().is_some() {
            bail!("Workspace selector is ambiguous; use workspace_id or exact path");
        }
        Ok(result)
    }
    pub fn select(&mut self, selectors: &[String]) -> Result<Vec<WorkspaceRegistration>> {
        let mut result = vec![];
        for s in selectors {
            let ws = match self.find(s) {
                Ok(w) => w,
                Err(e) => {
                    let p = Path::new(s);
                    if !p.is_absolute() || !p.is_dir() {
                        return Err(e);
                    }
                    let p = fs::canonicalize(p)?;
                    parser::load_config(&p)?;
                    let identity = p.join(".vdw/identity.json");
                    if identity.exists() {
                        let value: serde_json::Value = read(&identity)?;
                        if self
                            .registry
                            .workspaces
                            .iter()
                            .any(|w| value["workspace_id"] == w.workspace_id)
                        {
                            bail!("Workspace identity already registered at another path");
                        }
                    }
                    let ws = WorkspaceRegistration::new(p, Utc::now());
                    self.write_identity(&ws)?;
                    self.registry.workspaces.push(ws.clone());
                    self.persist_registry()?;
                    ws
                }
            };
            if !result
                .iter()
                .any(|w: &WorkspaceRegistration| w.workspace_id == ws.workspace_id)
            {
                result.push(ws);
            }
        }
        Ok(result)
    }
    pub fn touch(&mut self, selector: &str, source: &str) -> Result<WorkspaceRegistration> {
        let ws = self.find(selector)?;
        let w = self
            .registry
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == ws.workspace_id)
            .unwrap();
        if w.lifecycle_state != "active" {
            bail!(
                "Workspace is {}; use restore or operation resume",
                w.lifecycle_state
            );
        }
        w.last_active_at = Some(Utc::now());
        w.activity_source = source.into();
        w.activity_revision += 1;
        w.candidate_since = None;
        let r = w.clone();
        self.persist_registry()?;
        Ok(r)
    }
    pub fn protection(&self, id: &str) -> bool {
        self.protections
            .get(id)
            .map(|p| p.expires_at.map(|t| t > Utc::now()).unwrap_or(true))
            .unwrap_or(false)
    }
    pub fn active_lease(&self, id: &str) -> bool {
        self.leases
            .iter()
            .any(|l| l.workspace_id == id && l.expires_at > Utc::now())
    }
    pub fn lease(&mut self, selector: &str, client: &str, release: bool) -> Result<()> {
        if client.is_empty() || client.len() > 160 {
            bail!("Invalid client id");
        }
        let ws = self.find(selector)?;
        self.leases.retain(|l| {
            l.expires_at > Utc::now()
                && !(l.workspace_id == ws.workspace_id && l.client_id == client)
        });
        if !release {
            if ws.lifecycle_state != "active" {
                bail!("Cannot lease an archived or transitioning workspace");
            }
            self.leases.push(Lease {
                workspace_id: ws.workspace_id,
                client_id: client.into(),
                expires_at: Utc::now() + Duration::seconds(180),
            });
        }
        save(&self.base().join("leases.json"), &self.leases)
    }
    pub fn snapshot(
        &self,
        ws: &WorkspaceRegistration,
    ) -> Result<(Vec<RepoSnapshot>, Vec<Material>, String)> {
        let root = absolute(&ws.path)?;
        self.write_identity(ws)?;
        let mut config = parser::load_config(&root)?;
        env_resolver::resolve_env_vars(&mut config)?;
        if let Err(errors) = validator::validate_config(&config) {
            bail!("Invalid workspace configuration: {:?}", errors);
        }
        let locked = WorkspaceLock::load(&root)?;
        let state = WorkspaceState::load(&root)?;
        for l in &state.links {
            let rel = l
                .target
                .strip_prefix(&root)
                .context("Projection target escapes workspace")?;
            safe_relative(&rel.to_string_lossy())?;
            let parent = rel.parent().context("Missing link parent")?;
            if !parent.as_os_str().is_empty() {
                confined(&root, &parent.to_string_lossy())?;
            }
            if !l.source.starts_with(&self.store) {
                bail!("Unowned projection source");
            }
        }
        // Modified projections are dirty, not silently excluded or forcibly removed.
        let projection_audit = crate::link::engine::audit_links(&state.links);
        let mut repos = vec![];
        for dep in &config.dependencies {
            let path = confined(&root, &dep.path)?;
            let bare = object_pool::repo_store_dir(dep.dep_id(), &self.store).join("bare.git");
            let bare = fs::canonicalize(&bare)
                .with_context(|| format!("Missing Store repository: {}", dep.dep_id()))?;
            let commit = locked
                .as_ref()
                .and_then(|lock| {
                    lock.dependencies
                        .iter()
                        .find(|d| d.repo == dep.dep_id() && d.path == dep.path)
                })
                .context("Archive requires a resolved .vdw.lock for every dependency")?
                .resolved_commit
                .clone();
            let bare_repo = git2::Repository::open_bare(&bare)?;
            bare_repo.find_commit(git2::Oid::from_str(&commit)?)?;
            let (branch, head, dirty) = if dep.editable {
                if !path.join(".git").is_file() {
                    bail!(
                        "Editable dependency must be a native managed worktree: {}",
                        dep.path
                    );
                }
                let actual = git(
                    &path,
                    &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                )?;
                if fs::canonicalize(actual)? != bare {
                    bail!("Worktree Store ownership mismatch: {}", dep.path);
                }
                let branch = git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok();
                let head = git(&path, &["rev-parse", "HEAD"])?;
                if !git(&path, &["ls-files", "--stage"])?
                    .lines()
                    .all(|l| !l.starts_with("160000 "))
                {
                    bail!("Submodules need explicit lifecycle support: {}", dep.path);
                }
                for e in walkdir::WalkDir::new(&path)
                    .follow_links(false)
                    .into_iter()
                    .filter_entry(|e| {
                        e.depth() == 0
                            || !self
                                .config
                                .lifecycle
                                .archive
                                .materials
                                .disposable_ignored
                                .iter()
                                .any(|p| e.file_name().to_string_lossy() == p.as_str())
                    })
                {
                    let e = e?;
                    if e.depth() > 1 && e.file_name() == ".git" {
                        bail!("Nested Git repository in {}", dep.path);
                    }
                }
                (branch, head, self.dirty(&path, false)?)
            } else {
                let tree = bare_repo
                    .find_commit(git2::Oid::from_str(&commit)?)?
                    .tree()?;
                let mut dirty = DirtyCounts::default();
                for link in state
                    .links
                    .iter()
                    .filter(|l| l.dependency_repo == dep.dep_id())
                {
                    let rel = link.target.strip_prefix(&path)?;
                    let entry = tree.get_path(rel)?;
                    let blob = bare_repo.find_blob(entry.id())?;
                    if projection_audit
                        .conflicts
                        .iter()
                        .any(|c| c.target == link.target)
                        || hash_file(&link.target)
                            .map(|h| h != hash(blob.content()))
                            .unwrap_or(true)
                    {
                        dirty.unstaged += 1;
                    }
                }
                if path.is_dir() {
                    for e in walkdir::WalkDir::new(&path).follow_links(false) {
                        let e = e?;
                        if !e.file_type().is_dir()
                            && !state.links.iter().any(|l| l.target == e.path())
                        {
                            dirty.untracked += 1;
                        }
                    }
                }
                (None, commit.clone(), dirty)
            };
            repos.push(RepoSnapshot {
                repo: dep.dep_id().into(),
                repo_id: dep.repo_id(),
                path: dep.path.clone(),
                bare,
                editable: dep.editable,
                branch,
                head,
                locked_commit: commit,
                dirty,
                bundle: None,
                bundle_bytes: None,
            });
        }
        // A Git root at the workspace boundary is also protected. A parent repo is not.
        if root.join(".git").exists() {
            let d = self.dirty(&root, true)?;
            if d.dirty() || d.unknown_ignored > 0 {
                repos.push(RepoSnapshot {
                    repo: "workspace_root".into(),
                    repo_id: "workspace_root".into(),
                    path: ".".into(),
                    bare: PathBuf::new(),
                    editable: false,
                    branch: None,
                    head: String::new(),
                    locked_commit: String::new(),
                    dirty: d,
                    bundle: None,
                    bundle_bytes: None,
                });
            }
        }
        let deps = config
            .dependencies
            .iter()
            .map(|d| d.path.clone())
            .collect::<Vec<_>>();
        let materials = materials::collect(&root, &deps, &self.config.lifecycle.archive.materials)?;
        let raw_config = if root.join(".vdwrc").exists() {
            fs::read(root.join(".vdwrc"))?
        } else {
            fs::read(root.join(".vdwrc.yml"))?
        };
        let lock = fs::read(root.join(".vdw.lock")).unwrap_or_default();
        let fp = hash(&serde_json::to_vec(&(
            hash(&raw_config),
            hash(&lock),
            &repos,
            &materials,
        ))?);
        Ok((repos, materials, fp))
    }
    pub fn dirty(&self, path: &Path, workspace_root: bool) -> Result<DirtyCounts> {
        let repo = git2::Repository::open(path)?;
        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(true);
        let statuses = repo.statuses(Some(&mut opts))?;
        let mut counts = DirtyCounts::default();
        for e in statuses.iter() {
            let p = e
                .path()
                .context("Non-UTF8 Git path requires manual review")?;
            let s = e.status();
            if workspace_root
                && [".vdw/identity.json", ".vdw/state.json", ".vdw/archive.json"].contains(&p)
            {
                continue;
            }
            if s.contains(git2::Status::IGNORED) {
                let known = p.trim_end_matches('/').split('/').any(|v| {
                    self.config
                        .lifecycle
                        .archive
                        .materials
                        .disposable_ignored
                        .iter()
                        .any(|n| n == v)
                });
                if !known {
                    counts.unknown_ignored += 1;
                }
                continue;
            }
            if s.contains(git2::Status::CONFLICTED) {
                counts.conflicts += 1;
            }
            if s.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE,
            ) {
                counts.staged += 1;
            }
            if s.intersects(
                git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE,
            ) {
                counts.unstaged += 1;
            }
            if s.contains(git2::Status::WT_NEW) {
                counts.untracked += 1;
            }
        }
        Ok(counts)
    }
    pub fn preview_archive(&mut self, selectors: &[String], automatic: bool) -> Result<Plan> {
        let mut unresolved = vec![];
        let scope = if automatic {
            self.registry.workspaces.clone()
        } else {
            let mut scope = vec![];
            for selector in selectors {
                match self.select(std::slice::from_ref(selector)) {
                    Ok(rows) => {
                        for row in rows {
                            if !scope
                                .iter()
                                .any(|w: &WorkspaceRegistration| w.workspace_id == row.workspace_id)
                            {
                                scope.push(row);
                            }
                        }
                    }
                    Err(err) => unresolved.push((selector.clone(), format!("{err:#}"))),
                }
            }
            scope
        };
        let mut entries = vec![];
        for ws in scope {
            let root = self.archive_root(&ws.workspace_id);
            let root_error = root.as_ref().err().map(|e| format!("{e:#}"));
            let mut entry = PlanEntry {
                workspace_id: ws.workspace_id.clone(),
                name: ws.name.clone(),
                path: ws.path.clone(),
                reason_codes: if root_error.is_some() {
                    vec!["inspection_failed".into()]
                } else {
                    vec![]
                },
                message: root_error.unwrap_or_default(),
                fingerprint: None,
                repositories: vec![],
                materials: vec![],
                archive_root: root
                    .unwrap_or_else(|_| self.home.join("unavailable-archive-location")),
                git_backup: self.config.lifecycle.archive.git_backup.clone(),
                excluded: false,
                activity_revision: ws.activity_revision,
            };
            if ws.lifecycle_state == "archived" {
                entry.reason_codes.push("already_archived".into());
            } else if ws.lifecycle_state != "active" {
                entry.reason_codes.push("unfinished_operation".into());
            }
            if self.protection(&ws.workspace_id) {
                entry.reason_codes.push("protected".into());
            }
            if self.active_lease(&ws.workspace_id) {
                entry.reason_codes.push("active_lease".into());
            }
            if let Some(until) = self
                .policies
                .get(&ws.workspace_id)
                .and_then(|p| p.retain_until)
            {
                if until > Utc::now() {
                    entry.reason_codes.push("retain_until".into());
                }
            }
            if ws.lifecycle_state == "active" {
                match self.snapshot(&ws) {
                    Ok((repos, materials, fp)) => {
                        if repos.iter().any(|r| r.dirty.dirty()) {
                            entry.reason_codes.push("dirty".into());
                        }
                        if repos.iter().any(|r| r.dirty.unknown_ignored > 0) {
                            entry.reason_codes.push("ignored_files_need_review".into());
                        }
                        if automatic {
                            let w = self
                                .registry
                                .workspaces
                                .iter_mut()
                                .find(|w| w.workspace_id == ws.workspace_id)
                                .unwrap();
                            if w.observed_fingerprint
                                .as_ref()
                                .map(|old| old != &fp)
                                .unwrap_or(false)
                            {
                                w.last_active_at = Some(Utc::now());
                                w.activity_revision += 1;
                                w.activity_source = "observed_source_change".into();
                                w.candidate_since = None;
                            }
                            w.observed_fingerprint = Some(fp.clone());
                            let policy = self
                                .policies
                                .get(&ws.workspace_id)
                                .cloned()
                                .unwrap_or_default();
                            let baseline = w
                                .last_active_at
                                .unwrap_or(w.last_seen)
                                .max(w.observation_started_at.unwrap_or(w.last_seen));
                            if policy.auto_enabled == Some(false) {
                                entry.reason_codes.push("auto_disabled".into());
                            }
                            if Utc::now() - baseline
                                < Duration::days(i64::from(
                                    policy.idle_days.unwrap_or(self.config.lifecycle.idle_days),
                                ))
                            {
                                entry.reason_codes.push("not_due".into());
                                w.candidate_since = None;
                            } else if entry.reason_codes.is_empty() {
                                let since = *w.candidate_since.get_or_insert(Utc::now());
                                if Utc::now() - since
                                    < Duration::hours(i64::from(
                                        self.config.lifecycle.auto.grace_hours,
                                    ))
                                {
                                    entry.reason_codes.push("grace_period".into());
                                }
                            }
                            entry.activity_revision = w.activity_revision;
                        }
                        entry.repositories = repos;
                        entry.materials = materials;
                        entry.fingerprint = Some(fp);
                    }
                    Err(err) => {
                        entry.reason_codes.push("inspection_failed".into());
                        entry.message = format!("{err:#}");
                    }
                }
            }
            entries.push(entry);
        }
        for (selector, message) in unresolved {
            entries.push(PlanEntry {
                workspace_id: format!("unresolved-{}", &hash(selector.as_bytes())[..16]),
                name: selector.clone(),
                path: PathBuf::from(selector),
                reason_codes: vec!["inspection_failed".into()],
                message,
                fingerprint: None,
                repositories: vec![],
                materials: vec![],
                archive_root: self.home.join("unresolved-archive-location"),
                git_backup: self.config.lifecycle.archive.git_backup.clone(),
                excluded: false,
                activity_revision: 0,
            });
        }
        if automatic {
            self.persist_registry()?;
        }
        let plan = Plan {
            schema_version: 1,
            plan_id: id("plan"),
            kind: "archive".into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(30),
            config_hash: self.config_hash()?,
            entries,
            archive_id: None,
            target: None,
            restore_branch: None,
            value: Some(serde_json::json!({"automatic":automatic})),
            superseded: false,
            operation_id: None,
        };
        self.save_plan(&plan)?;
        Ok(plan)
    }
    pub fn save_plan(&self, plan: &Plan) -> Result<()> {
        save(
            &named_file(&self.base().join("plans"), &plan.plan_id)?,
            plan,
        )
    }
    pub fn plan(&self, id: &str) -> Result<Plan> {
        read(&named_file(&self.base().join("plans"), id)?)
    }
    pub fn save_operation(&self, op: &mut Operation) -> Result<()> {
        op.recount();
        save(
            &named_file(&self.base().join("operations"), &op.operation_id)?,
            op,
        )
    }
    pub fn operation(&self, id: &str) -> Result<Operation> {
        read(&named_file(&self.base().join("operations"), id)?)
    }
    pub fn new_operation(&self, plan: &Plan) -> Operation {
        let results = plan
            .entries
            .iter()
            .map(|e| WorkspaceResult {
                workspace_id: e.workspace_id.clone(),
                name: e.name.clone(),
                path: e.path.clone(),
                outcome: "unfinished".into(),
                reason_codes: vec![],
                message: String::new(),
                archive_id: None,
                archive_path: None,
                phase: "not_started".into(),
                completed_steps: vec![],
                repositories: e.repositories.clone(),
                memory_state: "not_started".into(),
                cloud_state: "disabled".into(),
                needs_attention: false,
            })
            .collect();
        Operation {
            schema_version: 1,
            operation_id: id("op"),
            plan_id: plan.plan_id.clone(),
            kind: plan.kind.clone(),
            state: "running".into(),
            started_at: Utc::now(),
            updated_at: Utc::now(),
            summary: Summary::default(),
            results,
            progress: Progress::default(),
            error: None,
            source_root: None,
            target_root: None,
            archive_ids: vec![],
            completed_archive_ids: vec![],
            attempts: 1,
            attempt_history: vec![],
        }
    }
    pub fn simple_plan(&self, kind: &str, value: serde_json::Value) -> Result<Plan> {
        let p = Plan {
            schema_version: 1,
            plan_id: id("plan"),
            kind: kind.into(),
            created_at: Utc::now(),
            expires_at: Utc::now() + Duration::minutes(30),
            config_hash: self.config_hash()?,
            entries: vec![],
            archive_id: None,
            target: None,
            restore_branch: None,
            value: Some(value),
            superseded: false,
            operation_id: None,
        };
        self.save_plan(&p)?;
        Ok(p)
    }
    pub fn finish(op: &mut Operation) {
        op.recount();
        op.state = if op.summary.failed_count > 0 {
            "partial_failure"
        } else if op.summary.unfinished_count > 0 {
            "needs_attention"
        } else if op.summary.skipped_count > 0 {
            "completed_with_skips"
        } else {
            "completed"
        }
        .into();
    }
}

/// Called while the standard CLI holds the shared operation guard.
pub fn ensure_active(path: &Path) -> Result<()> {
    if !path.join(".vdwrc").exists() && !path.join(".vdwrc.yml").exists() {
        return Ok(());
    }
    let registry = registry::load_workspace_registry()?;
    if let Some(ws) = registry.workspaces.iter().find(|w| w.path == path) {
        if ws.lifecycle_state != "active" {
            bail!(
                "Workspace is {}; use `vdw workspace restore {} --preview` or operation resume",
                ws.lifecycle_state,
                ws.latest_archive_id.as_deref().unwrap_or("<archive_id>")
            );
        }
    }
    if path.join(".vdw/archive.json").exists() {
        bail!("Workspace is archived; use `vdw workspace restore <archive_id> --preview`");
    }
    Ok(())
}
