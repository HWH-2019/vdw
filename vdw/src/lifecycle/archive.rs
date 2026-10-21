use super::{engine::Engine, io::*, materials, types::*};
use crate::{
    store::editable_locks,
    types::workspace::{LinkEntry, WorkspaceState},
    worktree::manager as worktrees,
};
use anyhow::{bail, Context, Result};
use chrono::Utc;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

impl Engine {
    pub fn verified(&self, record: &ArchiveRecord) -> Result<Manifest> {
        verify_archive(record)
    }
}
pub fn verify_archive(record: &ArchiveRecord) -> Result<Manifest> {
    let root = absolute(&record.location)?;
    if hash_file(&confined(&root, "manifest.json")?)? != record.manifest_hash {
        bail!("Archive manifest checksum mismatch");
    }
    let m: Manifest = read(&root.join("manifest.json"))?;
    if m.schema_version != 1
        || m.archive_id != record.archive_id
        || m.workspace_id != record.workspace_id
    {
        bail!("Archive identity mismatch");
    }
    materials::validate_entries(&m.materials)?;
    for (rel, expected) in &m.files {
        if hash_file(&confined(&root, rel)?)? != *expected {
            bail!("Archive checksum mismatch: {rel}");
        }
    }
    for file in &m.materials {
        let rel = safe_relative(&file.path)?;
        let p = root.join("workspace-files").join(&rel);
        match file.kind.as_str() {
            "file" => {
                if hash_file(&confined(&root, &format!("workspace-files/{}", file.path))?)?
                    != file.hash
                {
                    bail!("Material checksum mismatch");
                }
            }
            "directory" => {
                if !confined(&root, &format!("workspace-files/{}", file.path))?.is_dir() {
                    bail!("Archive directory missing");
                }
            }
            "symlink" => {
                let parent = rel.parent().unwrap_or(Path::new(""));
                if !parent.as_os_str().is_empty() {
                    confined(
                        &root,
                        &format!("workspace-files/{}", parent.to_string_lossy()),
                    )?;
                }
                if fs::read_link(&p)?.to_string_lossy() != file.link_target.as_deref().unwrap_or("")
                {
                    bail!("Archive symlink mismatch");
                }
            }
            _ => bail!("Invalid archive material kind"),
        }
    }
    for generation in &record.generations {
        let base = confined(&root, &format!("derived/ai/{generation}"))?;
        if fs::read(confined(&base, "COMPLETE")?)? != b"1\n" {
            bail!("Incomplete AI generation");
        }
        let checksums: BTreeMap<String, String> = read(&confined(&base, "checksums.json")?)?;
        for (path, expected) in checksums {
            if hash_file(&confined(&base, &path)?)? != expected {
                bail!("AI generation checksum mismatch");
            }
        }
    }
    Ok(m)
}
impl Engine {
    pub fn archive_record(&self, id: &str) -> Result<ArchiveRecord> {
        self.archives
            .iter()
            .find(|a| a.archive_id == id)
            .cloned()
            .context("Archive not found")
    }
    pub fn execute_plan(&mut self, id: &str) -> Result<Operation> {
        let mut plan = self.plan(id)?;
        if let Some(op) = &plan.operation_id {
            return self.operation(op);
        }
        if plan.superseded {
            bail!("PLAN_STALE: this plan was replaced; review the new plan");
        }
        if plan.kind == "resume" {
            return self.resume_operation_plan(&mut plan);
        }
        let mut op = self.new_operation(&plan);
        self.save_operation(&mut op)?;
        plan.operation_id = Some(op.operation_id.clone());
        self.save_plan(&plan)?;
        if plan.kind == "archive" {
            let stale = plan.expires_at < Utc::now() || plan.config_hash != self.config_hash()?;
            if !stale {
                if let Some(client) = plan.value.as_ref().and_then(|v| v["client"].as_str()) {
                    self.leases.retain(|l| {
                        !(l.client_id == client
                            && plan
                                .entries
                                .iter()
                                .any(|e| e.workspace_id == l.workspace_id && !e.excluded))
                    });
                    save(&self.base().join("leases.json"), &self.leases)?;
                }
            }
            for (i, entry) in plan.entries.iter().enumerate() {
                if stale {
                    op.results[i].outcome = "failed".into();
                    op.results[i].reason_codes = vec!["plan_stale".into()];
                    op.results[i].message = "Plan expired or policy changed; preview again".into();
                } else if let Err(err) = self.archive_entry(entry, &mut op, i, false) {
                    op.results[i].outcome = "failed".into();
                    op.results[i].reason_codes = vec![if err.to_string().contains("PLAN_STALE") {
                        "plan_stale"
                    } else {
                        "execution_failed"
                    }
                    .into()];
                    op.results[i].message = format!("{err:#}");
                    op.results[i].needs_attention = op.results[i]
                        .completed_steps
                        .iter()
                        .any(|s| s.starts_with("removed:") || s.starts_with("projection:"));
                }
                self.save_operation(&mut op)?;
            }
            Self::finish(&mut op);
        } else if plan.expires_at < Utc::now() || plan.config_hash != self.config_hash()? {
            op.state = "failed".into();
            op.error = Some("PLAN_STALE: preview again".into());
        } else {
            match self.execute_other(&plan, &mut op) {
                Ok(()) => {
                    if op.state == "running" && !(self.background && op.kind == "migration") {
                        Self::finish(&mut op);
                    }
                }
                Err(err) => {
                    op.state = "failed".into();
                    op.error = Some(format!("{err:#}"));
                    for r in &mut op.results {
                        if r.outcome == "unfinished" {
                            r.outcome = "failed".into();
                            r.message = format!("{err:#}");
                            r.needs_attention = !r.completed_steps.is_empty();
                        }
                    }
                }
            }
        }
        self.save_operation(&mut op)?;
        Ok(op)
    }
    fn skip(
        &self,
        op: &mut Operation,
        i: usize,
        codes: Vec<String>,
        message: String,
    ) -> Result<()> {
        let r = &mut op.results[i];
        r.outcome = if codes.iter().any(|c| c == "inspection_failed") {
            "failed"
        } else {
            "skipped"
        }
        .into();
        r.reason_codes = codes;
        r.message = message;
        r.phase = "preflight".into();
        self.save_operation(op)
    }
    pub(crate) fn archive_entry(
        &mut self,
        entry: &PlanEntry,
        op: &mut Operation,
        i: usize,
        resume: bool,
    ) -> Result<()> {
        if entry.excluded {
            return self.skip(
                op,
                i,
                vec!["excluded_by_user".into()],
                "Excluded in reviewed plan".into(),
            );
        }
        if !resume && !entry.reason_codes.is_empty() {
            return self.skip(op, i, entry.reason_codes.clone(), entry.message.clone());
        }
        let ws = self.find(&entry.workspace_id)?;
        if self.protection(&ws.workspace_id) {
            return self.skip(
                op,
                i,
                vec!["protected".into()],
                "Workspace is protected".into(),
            );
        }
        if self.active_lease(&ws.workspace_id) {
            return self.skip(
                op,
                i,
                vec!["active_lease".into()],
                "Workspace has an active client lease".into(),
            );
        }
        let mut record = if resume {
            op.results[i]
                .archive_id
                .as_deref()
                .and_then(|id| self.archives.iter().find(|a| a.archive_id == id))
                .cloned()
        } else {
            None
        };
        if resume && record.is_none() {
            if let (Some(aid), Some(location)) =
                (&op.results[i].archive_id, &op.results[i].archive_path)
            {
                let expected = entry.archive_root.join(&ws.workspace_id).join(aid);
                if location != &expected {
                    bail!("Operation archive path does not match reviewed scope");
                }
                if location.join("manifest.json").is_file() {
                    let recovered = ArchiveRecord {
                        archive_id: aid.clone(),
                        workspace_id: ws.workspace_id.clone(),
                        name: ws.name.clone(),
                        original_path: ws.path.clone(),
                        location: location.clone(),
                        created_at: op.started_at,
                        manifest_hash: hash_file(&location.join("manifest.json"))?,
                        storage_policy: if self
                            .policies
                            .get(&ws.workspace_id)
                            .and_then(|p| p.archive_root.as_ref())
                            .is_some()
                        {
                            ws.workspace_id.clone()
                        } else {
                            "global".into()
                        },
                        memory_state: if self.config.lifecycle.memory.ai.enabled
                            && self.config.lifecycle.memory.ai.trigger == "after_archive"
                        {
                            "pending"
                        } else {
                            "disabled"
                        }
                        .into(),
                        cloud_state: "disabled".into(),
                        generations: vec![],
                    };
                    let manifest = self.verified(&recovered)?;
                    if manifest.materials != entry.materials
                        || manifest.repositories.len() != entry.repositories.len()
                    {
                        bail!("Published snapshot differs from reviewed plan");
                    }
                    for (actual, planned) in manifest.repositories.iter().zip(&entry.repositories) {
                        if actual.repo != planned.repo
                            || actual.head != planned.head
                            || actual.path != planned.path
                            || actual.bare != planned.bare
                            || actual.locked_commit != planned.locked_commit
                        {
                            bail!("Published repository snapshot differs from reviewed plan");
                        }
                    }
                    self.archives.push(recovered.clone());
                    self.persist_archives()?;
                    record = Some(recovered);
                    if !op.results[i]
                        .completed_steps
                        .iter()
                        .any(|s| s == "archive_verified")
                    {
                        op.results[i]
                            .completed_steps
                            .push("archive_verified".into());
                        self.save_operation(op)?;
                    }
                }
            }
        }
        let record = if let Some(record) = record {
            let m = self.verified(&record)?;
            self.check_remaining(&m, op, i)?;
            record
        } else {
            if ws.lifecycle_state != "active" {
                bail!("Workspace has an unfinished operation; use its resume plan");
            }
            let (repos, files, fp) = self.snapshot(&ws)?;
            op.results[i].repositories = repos.clone();
            if repos.iter().any(|r| r.dirty.dirty()) {
                return self.skip(
                    op,
                    i,
                    vec!["dirty".into()],
                    "One repository is dirty; the entire workspace was skipped".into(),
                );
            }
            if repos.iter().any(|r| r.dirty.unknown_ignored > 0) {
                return self.skip(
                    op,
                    i,
                    vec!["ignored_files_need_review".into()],
                    "Unknown ignored files need review".into(),
                );
            }
            if Some(&fp) != entry.fingerprint.as_ref()
                || ws.activity_revision != entry.activity_revision
            {
                bail!("PLAN_STALE: workspace activity or source changed");
            }
            let aid = op.results[i]
                .archive_id
                .clone()
                .unwrap_or_else(|| id("archive"));
            let parent = entry.archive_root.join(&ws.workspace_id);
            private_dir(&parent)?;
            let dest = parent.join(&aid);
            let staging = parent.join(format!(".{aid}.staging"));
            op.results[i].archive_id = Some(aid.clone());
            op.results[i].archive_path = Some(dest.clone());
            op.results[i].phase = "snapshot".into();
            self.save_operation(op)?;
            if dest.exists() {
                bail!("Unindexed archive exists; inspect operation before retrying");
            }
            if staging.exists() {
                if resume {
                    fs::remove_dir_all(&staging)?;
                } else {
                    bail!("Staging data exists; preserve it and inspect the previous operation");
                }
            }
            private_dir(&staging)?;
            materials::copy_materials(&ws.path, &staging.join("workspace-files"), &files, false)?;
            let cname = if ws.path.join(".vdwrc").exists() {
                ".vdwrc"
            } else {
                ".vdwrc.yml"
            };
            atomic_bytes(
                &staging.join("config").join(cname),
                &fs::read(ws.path.join(cname))?,
            )?;
            if ws.path.join(".vdw.lock").exists() {
                atomic_bytes(
                    &staging.join("config/.vdw.lock"),
                    &fs::read(ws.path.join(".vdw.lock"))?,
                )?;
            }
            save(
                &staging.join("config/identity.json"),
                &serde_json::json!({"workspace_id":ws.workspace_id}),
            )?;
            let mut manifest = Manifest {
                schema_version: 1,
                archive_id: aid.clone(),
                workspace_id: ws.workspace_id.clone(),
                name: ws.name.clone(),
                original_path: ws.path.clone(),
                created_at: Utc::now(),
                reason: if entry.reason_codes.is_empty() {
                    "reviewed_archive"
                } else {
                    "automatic_archive"
                }
                .into(),
                repositories: repos,
                materials: files,
                links: WorkspaceState::load(&ws.path)?,
                files: BTreeMap::new(),
            };
            for repo in &mut manifest.repositories {
                let href = format!("refs/vdw/archives/{aid}/{}/head", repo.repo_id);
                let lref = format!("refs/vdw/archives/{aid}/{}/lock", repo.repo_id);
                git(&repo.bare, &["update-ref", &href, &repo.head])?;
                git(&repo.bare, &["update-ref", &lref, &repo.locked_commit])?;
                if entry.git_backup == "portable_bundle" {
                    let rel = format!("git/{}.bundle", repo.repo_id);
                    let target = staging.join(&rel);
                    private_dir(target.parent().unwrap())?;
                    git(
                        &repo.bare,
                        &[
                            "bundle",
                            "create",
                            "--quiet",
                            target.to_str().context("Non-UTF8 archive path")?,
                            &href,
                            &lref,
                        ],
                    )?;
                    let bytes = fs::metadata(&target)?.len();
                    if self
                        .config
                        .lifecycle
                        .archive
                        .max_bundle_bytes
                        .map(|n| bytes > n)
                        .unwrap_or(false)
                    {
                        bail!("Bundle exceeds configured capacity budget");
                    }
                    git(&repo.bare, &["bundle", "verify", target.to_str().unwrap()])?;
                    repo.bundle = Some(rel);
                    repo.bundle_bytes = Some(bytes);
                }
            }
            materials::index(&staging, &manifest)?;
            save(
                &staging.join("evidence/repositories.json"),
                &manifest.repositories,
            )?;
            manifest.files = materials::archive_files(&staging)?;
            save(&staging.join("manifest.json"), &manifest)?;
            let manifest_hash = hash_file(&staging.join("manifest.json"))?;
            if self.snapshot(&ws)?.2 != fp {
                bail!("PLAN_STALE: source changed while snapshotting; no worktrees removed");
            }
            fs::rename(&staging, &dest)?;
            let record = ArchiveRecord {
                archive_id: aid,
                workspace_id: ws.workspace_id.clone(),
                name: ws.name.clone(),
                original_path: ws.path.clone(),
                location: dest,
                created_at: Utc::now(),
                manifest_hash,
                storage_policy: if self
                    .policies
                    .get(&ws.workspace_id)
                    .and_then(|p| p.archive_root.as_ref())
                    .is_some()
                {
                    ws.workspace_id.clone()
                } else {
                    "global".into()
                },
                memory_state: if self.config.lifecycle.memory.ai.enabled
                    && self.config.lifecycle.memory.ai.trigger == "after_archive"
                {
                    "pending"
                } else {
                    "disabled"
                }
                .into(),
                cloud_state: "disabled".into(),
                generations: vec![],
            };
            self.verified(&record)?;
            self.archives.push(record.clone());
            self.persist_archives()?;
            op.results[i]
                .completed_steps
                .push("archive_verified".into());
            self.save_operation(op)?;
            record
        };
        let m = self.verified(&record)?;
        self.check_remaining(&m, op, i)?;
        let w = self
            .registry
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == ws.workspace_id)
            .unwrap();
        w.lifecycle_state = "archiving".into();
        w.current_operation_id = Some(op.operation_id.clone());
        self.persist_registry()?;
        op.results[i].phase = "remove_worktrees".into();
        self.save_operation(op)?;
        for repo in m.repositories.iter().filter(|r| r.editable) {
            let marker = format!("removed:{}", repo.path);
            if op.results[i].completed_steps.contains(&marker) {
                continue;
            }
            self.check_remaining(&m, op, i)?;
            let target = confined(&ws.path, &repo.path)?;
            if target.exists() {
                worktrees::remove_worktree_at(&repo.bare, &target, false)?;
            }
            if let Some(branch) = &repo.branch {
                editable_locks::release_editable_lock(&repo.repo, &branch, &ws.path)?;
            }
            op.results[i].completed_steps.push(marker);
            self.save_operation(op)?;
        }
        op.results[i].phase = "remove_projections".into();
        self.save_operation(op)?;
        for link in &m.links.links {
            let rel = link
                .target
                .strip_prefix(&ws.path)?
                .to_string_lossy()
                .into_owned();
            let marker = format!("projection:{rel}");
            if op.results[i].completed_steps.contains(&marker) {
                continue;
            }
            self.check_remaining(&m, op, i)?;
            if !link.target.starts_with(&ws.path) {
                bail!("Projection escaped workspace");
            }
            crate::link::engine::verify_links_for_removal(std::slice::from_ref(link))?;
            crate::link::engine::remove_links(std::slice::from_ref(link))?;
            op.results[i].completed_steps.push(marker);
            self.save_operation(op)?;
        }
        WorkspaceState::new(ws.path.clone()).save()?;
        save(
            &confined(&ws.path, ".vdw/archive.json")?,
            &serde_json::json!({"workspace_id":ws.workspace_id,"archive_id":record.archive_id}),
        )?;
        let w = self
            .registry
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == ws.workspace_id)
            .unwrap();
        w.lifecycle_state = "archived".into();
        w.latest_archive_id = Some(record.archive_id.clone());
        w.current_operation_id = None;
        w.candidate_since = None;
        self.persist_registry()?;
        for repo in &m.repositories {
            if let Some(branch) = &repo.branch {
                git(
                    &repo.bare,
                    &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
                )?;
            }
        }
        let r = &mut op.results[i];
        r.outcome = "archived".into();
        r.archive_id = Some(record.archive_id);
        r.archive_path = Some(record.location);
        r.memory_state = record.memory_state;
        r.phase = "completed".into();
        r.reason_codes.clear();
        r.message = "Local archive verified; worktrees removed and branches retained".into();
        r.needs_attention = false;
        self.save_operation(op)?;
        Ok(())
    }
    fn check_remaining(&self, m: &Manifest, op: &Operation, i: usize) -> Result<()> {
        if self.active_lease(&m.workspace_id) || self.protection(&m.workspace_id) {
            bail!("Protection or lease prevents continuing archive");
        }
        let cfg = crate::config::parser::load_config(&m.original_path)?;
        let remaining = m
            .links
            .links
            .iter()
            .filter(|l| {
                let rel = l
                    .target
                    .strip_prefix(&m.original_path)
                    .unwrap_or(&l.target)
                    .to_string_lossy();
                !op.results[i]
                    .completed_steps
                    .contains(&format!("projection:{rel}"))
                    && fs::symlink_metadata(&l.target).is_ok()
            })
            .cloned()
            .collect::<Vec<_>>();
        crate::link::engine::verify_links_for_removal(&remaining)?;
        for repo in m.repositories.iter().filter(|r| !r.editable) {
            let bare = git2::Repository::open_bare(&repo.bare)?;
            let tree = bare
                .find_commit(git2::Oid::from_str(&repo.locked_commit)?)?
                .tree()?;
            let root = m.original_path.join(&repo.path);
            for link in m
                .links
                .links
                .iter()
                .filter(|l| l.dependency_repo == repo.repo)
            {
                let rel = link
                    .target
                    .strip_prefix(&m.original_path)?
                    .to_string_lossy();
                if op.results[i]
                    .completed_steps
                    .contains(&format!("projection:{rel}"))
                {
                    continue;
                }
                if op.results[i].phase == "remove_projections"
                    && fs::symlink_metadata(&link.target).is_err()
                {
                    continue;
                }
                let file = tree.get_path(link.target.strip_prefix(&root)?)?;
                let blob = bare.find_blob(file.id())?;
                if hash_file(&link.target)? != hash(blob.content()) {
                    bail!("Read-only projection changed; no further deletion");
                }
            }
            if root.is_dir() {
                for e in walkdir::WalkDir::new(&root).follow_links(false) {
                    let e = e?;
                    if !e.file_type().is_dir()
                        && !m.links.links.iter().any(|l| l.target == e.path())
                    {
                        bail!("Untracked content appeared in read-only projection");
                    }
                }
            }
        }
        if m.original_path.join(".git").exists() {
            let d = self.dirty(&m.original_path, true)?;
            if d.dirty() || d.unknown_ignored > 0 {
                bail!("Workspace Git root has new uncommitted changes");
            }
        }
        let current = materials::collect(
            &m.original_path,
            &cfg.dependencies
                .iter()
                .map(|d| d.path.clone())
                .collect::<Vec<_>>(),
            &self.config.lifecycle.archive.materials,
        )?;
        if current != m.materials {
            bail!("Project materials changed; preserve current workspace and review recovery");
        }
        for r in m.repositories.iter().filter(|r| r.editable) {
            if op.results[i]
                .completed_steps
                .contains(&format!("removed:{}", r.path))
            {
                continue;
            }
            let target = confined(&m.original_path, &r.path)?;
            if !target.exists() {
                // A crash can occur after native removal but before its checkpoint.
                let list = git(&r.bare, &["worktree", "list", "--porcelain"])?;
                if list
                    .lines()
                    .any(|l| l == format!("worktree {}", target.display()))
                {
                    bail!("Registered worktree directory is unexpectedly missing");
                }
                continue;
            }
            if git(
                &target,
                &["rev-parse", "--path-format=absolute", "--git-common-dir"],
            )? != r.bare.to_string_lossy()
            {
                bail!("Worktree owner changed");
            }
            let dirty = self.dirty(&target, false)?;
            if dirty.dirty() || dirty.unknown_ignored > 0 {
                bail!(
                    "New dirty or unknown ignored files: {}; no further deletion",
                    r.path
                );
            }
            if git(&target, &["rev-parse", "HEAD"])? != r.head {
                bail!("Worktree HEAD changed: {}", r.path);
            }
        }
        Ok(())
    }
    pub fn preview_restore(
        &self,
        aid: &str,
        target: Option<PathBuf>,
        branch: Option<String>,
    ) -> Result<Plan> {
        let record = self.archive_record(aid)?;
        let m = self.verified(&record)?;
        let target = absolute(&target.unwrap_or_else(|| record.original_path.clone()))?;
        let ws = self.find(&record.workspace_id)?;
        self.restore_preflight(&record, &m, &target, branch.as_deref(), false)?;
        let mut p = self.simple_plan("restore", serde_json::Value::Null)?;
        p.archive_id = Some(aid.into());
        p.target = Some(target);
        p.restore_branch = branch;
        p.entries = vec![PlanEntry {
            workspace_id: ws.workspace_id,
            name: ws.name,
            path: ws.path,
            reason_codes: vec![],
            message: String::new(),
            fingerprint: Some(record.manifest_hash),
            repositories: m.repositories,
            materials: m.materials,
            archive_root: record.location,
            git_backup: "store_refs".into(),
            excluded: false,
            activity_revision: ws.activity_revision,
        }];
        self.save_plan(&p)?;
        Ok(p)
    }
    fn restore_preflight(
        &self,
        record: &ArchiveRecord,
        m: &Manifest,
        target: &Path,
        branch: Option<&str>,
        resume: bool,
    ) -> Result<()> {
        let ws = self.find(&record.workspace_id)?;
        if ws.lifecycle_state == "active" {
            bail!("Workspace is already active; a workspace_id cannot be active twice");
        }
        if self.active_lease(&ws.workspace_id) {
            bail!("Workspace has an active lease");
        }
        if target.starts_with(&self.store)
            || target.starts_with(&record.location)
            || self.home.starts_with(target)
        {
            bail!("Unsafe restore target");
        }
        if target.is_dir() && fs::read_dir(target)?.next().is_some() {
            let identity: serde_json::Value = read(&confined(target, ".vdw/identity.json")?)
                .context("Non-empty restore target is not a matching archived skeleton")?;
            if identity["workspace_id"] != record.workspace_id {
                bail!("Restore target has another workspace identity");
            }
        }
        materials::preflight_restore(target, &m.materials)?;
        for r in &m.repositories {
            let path = confined(target, &r.path)?;
            let mut already_restored = false;
            if path.exists() && path.read_dir()?.next().is_some() {
                if !resume {
                    bail!("Restore repository target is not empty: {}", r.path);
                }
                if r.editable {
                    if git(
                        &path,
                        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                    )? != r.bare.to_string_lossy()
                        || git(&path, &["rev-parse", "HEAD"])? != r.head
                    {
                        bail!("Existing restore worktree has changed; preserve it and review");
                    }
                    let d = self.dirty(&path, false)?;
                    if d.dirty() || d.unknown_ignored > 0 {
                        bail!("Existing restore worktree is dirty");
                    }
                    if git(&path, &["symbolic-ref", "--quiet", "--short", "HEAD"])
                        .ok()
                        .as_deref()
                        != branch.or(r.branch.as_deref())
                    {
                        bail!("Existing restored branch changed");
                    }
                    already_restored = true;
                } else {
                    let expected = m
                        .links
                        .links
                        .iter()
                        .filter(|l| l.dependency_repo == r.repo)
                        .map(|l| Ok(target.join(l.target.strip_prefix(&m.original_path)?)))
                        .collect::<Result<Vec<_>>>()?;
                    for e in walkdir::WalkDir::new(&path).follow_links(false) {
                        let e = e?;
                        if !e.file_type().is_dir() && !expected.iter().any(|p| p == e.path()) {
                            bail!("Unmanaged file appeared in restored projection");
                        }
                    }
                }
            }
            if r.bare.exists() {
                git(
                    &r.bare,
                    &["cat-file", "-e", &format!("{}^{{commit}}", r.head)],
                )?;
                if r.editable {
                    let desired = branch.or(r.branch.as_deref());
                    if let Some(b) = desired {
                        git(&r.bare, &["check-ref-format", "--branch", b])?;
                        if let Ok(head) = git(
                            &r.bare,
                            &["rev-parse", "--verify", &format!("refs/heads/{b}")],
                        ) {
                            if head != r.head {
                                bail!("Branch advanced; choose --branch <new-restore-branch> without resetting it: {b}");
                            }
                        }
                        let list = git(&r.bare, &["worktree", "list", "--porcelain"])?;
                        if !already_restored
                            && list.lines().any(|l| l == format!("branch refs/heads/{b}"))
                        {
                            bail!("Branch is checked out by another worktree: {b}");
                        }
                    }
                }
            } else if r.bundle.is_none() {
                bail!(
                    "Store unavailable and archive has no portable bundle: {}",
                    r.repo
                );
            }
        }
        Ok(())
    }
    pub(crate) fn restore(&mut self, plan: &Plan, op: &mut Operation) -> Result<()> {
        let record = self.archive_record(plan.archive_id.as_deref().context("Missing archive")?)?;
        let m = self.verified(&record)?;
        let target = plan.target.as_deref().context("Missing restore target")?;
        let resume = op.attempts > 1;
        self.restore_preflight(&record, &m, target, plan.restore_branch.as_deref(), resume)?;
        private_dir(target)?;
        materials::copy_materials(
            &record.location.join("workspace-files"),
            target,
            &m.materials,
            true,
        )?;
        let config_dir = record.location.join("config");
        for name in [".vdwrc", ".vdwrc.yml", ".vdw.lock"] {
            let src = config_dir.join(name);
            if src.exists() {
                let dst = confined(target, name)?;
                if !dst.exists() {
                    atomic_bytes(&dst, &fs::read(src)?)?;
                }
            }
        }
        save(
            &confined(target, ".vdw/identity.json")?,
            &serde_json::json!({"workspace_id":record.workspace_id}),
        )?;
        let w = self
            .registry
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == record.workspace_id)
            .unwrap();
        w.lifecycle_state = "restoring".into();
        w.current_operation_id = Some(op.operation_id.clone());
        self.persist_registry()?;
        op.results[0].phase = "restore_worktrees".into();
        self.save_operation(op)?;
        for r in &m.repositories {
            if !r.bare.exists() {
                let expected = crate::store::object_pool::repo_store_dir(&r.repo, &self.store)
                    .join("bare.git");
                if r.bare != expected {
                    bail!("Portable restore needs the original Store location");
                }
                private_dir(r.bare.parent().unwrap())?;
                git2::Repository::init_bare(&r.bare)?;
                let bundle = confined(
                    &record.location,
                    r.bundle.as_deref().context("Missing portable bundle")?,
                )?;
                git(&r.bare, &["bundle", "verify", bundle.to_str().unwrap()])?;
                git(
                    &r.bare,
                    &[
                        "fetch",
                        bundle.to_str().unwrap(),
                        "refs/vdw/archives/*:refs/vdw/archives/*",
                    ],
                )?;
                let bare_repo = git2::Repository::open_bare(&r.bare)?;
                if bare_repo.find_remote("origin").is_err() {
                    bare_repo.remote("origin", &r.repo)?;
                }
                crate::types::store::StoreMetadata {
                    repo_url: r.repo.clone(),
                    commit_hash: r.head.clone(),
                    branch: r.branch.clone(),
                    tag: None,
                    reference_count: 1,
                    last_accessed: Utc::now(),
                    created_at: Utc::now(),
                    size_bytes: 0,
                    alternates_configured: false,
                }
                .save(r.bare.parent().unwrap())?;
            }
            let path = confined(target, &r.path)?;
            if r.editable {
                if let Some(b) = plan.restore_branch.as_deref().or(r.branch.as_deref()) {
                    if git(
                        &r.bare,
                        &["rev-parse", "--verify", &format!("refs/heads/{b}")],
                    )
                    .is_err()
                    {
                        git(&r.bare, &["branch", b, &r.head])?;
                    }
                    worktrees::create_worktree_at(&r.bare, b, &path)?;
                    editable_locks::acquire_editable_lock(&r.repo, b, target, &path)?;
                } else {
                    git(
                        &r.bare,
                        &[
                            "worktree",
                            "add",
                            "--detach",
                            path.to_str().unwrap(),
                            &r.head,
                        ],
                    )?;
                }
                if git(&path, &["rev-parse", "HEAD"])? != r.head {
                    bail!("Restored HEAD mismatch");
                }
            }
            op.results[0]
                .completed_steps
                .push(format!("restored:{}", r.path));
            self.save_operation(op)?;
        }
        let mut state = WorkspaceState::new(target.to_path_buf());
        for link in &m.links.links {
            let rel = link.target.strip_prefix(&m.original_path)?;
            let parent = rel.parent().context("Missing projection parent")?;
            let target_path = if parent.as_os_str().is_empty() {
                target.to_path_buf()
            } else {
                confined(target, &parent.to_string_lossy())?
            }
            .join(rel.file_name().context("Missing projection file")?);
            if fs::symlink_metadata(&target_path).is_ok() && !resume {
                bail!("Read-only restore would overwrite existing content");
            }
            if !link.source.starts_with(&self.store) {
                bail!("Read-only source is outside Store");
            }
            let mut next = LinkEntry {
                target: target_path,
                ..link.clone()
            };
            if fs::symlink_metadata(&next.target).is_ok() {
                crate::link::engine::verify_links_for_removal(std::slice::from_ref(&next))?;
            }
            if !next.source.exists() {
                self.rebuild_readonly_source(&m, &next)?;
            }
            if !crate::link::platform::is_same_partition(
                &next.source,
                next.target.parent().unwrap(),
            ) {
                next.link_type = crate::types::workspace::LinkType::Symlink;
            }
            let current = crate::link::engine::get_file_inode(&next.source);
            next.source_inode = current.map(|v| v.0);
            next.source_volume = current.map(|v| v.1);
            crate::link::engine::restore_links(std::slice::from_ref(&next))?;
            state.links.push(next);
        }
        state.save()?;
        if plan.restore_branch.is_some() {
            let mut c = crate::config::parser::load_config(target)?;
            for dep in &mut c.dependencies {
                if dep.editable {
                    dep.branch = plan.restore_branch.clone();
                    dep.tag = None;
                    dep.commit = None;
                }
            }
            crate::config::parser::save_config(target, &c)?;
            if let Some(mut lock) = crate::lockfile::WorkspaceLock::load(target)? {
                for dep in &mut lock.dependencies {
                    if dep.editable {
                        dep.revision.kind = crate::lockfile::RevisionKind::Branch;
                        dep.revision.value = plan.restore_branch.clone().unwrap();
                    }
                }
                lock.save(target)?;
            }
        }
        let restored_config = crate::config::parser::load_config(target)?;
        if let Err(errors) = crate::config::validator::validate_config(&restored_config) {
            bail!("Invalid restored configuration: {:?}", errors);
        }
        let topology = crate::commands::status::inspect_workspace(target, &self.store)?;
        if !topology.healthy {
            bail!("Restored topology is incomplete");
        }
        let marker = confined(target, ".vdw/archive.json")?;
        if marker.exists() {
            fs::remove_file(marker)?;
        }
        let w = self
            .registry
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == record.workspace_id)
            .unwrap();
        w.path = target.to_path_buf();
        w.lifecycle_state = "active".into();
        w.current_operation_id = None;
        w.last_active_at = Some(Utc::now());
        w.activity_source = "restore".into();
        w.activity_revision += 1;
        w.observation_started_at = Some(Utc::now());
        w.observed_fingerprint = None;
        self.persist_registry()?;
        let r = &mut op.results[0];
        r.path = target.to_path_buf();
        r.outcome = "restored".into();
        r.phase = "completed".into();
        r.archive_id = Some(record.archive_id);
        r.archive_path = Some(record.location);
        r.message = "Configuration, project materials and exact repository commits restored".into();
        Ok(())
    }
    fn rebuild_readonly_source(&self, m: &Manifest, link: &LinkEntry) -> Result<()> {
        let r = m
            .repositories
            .iter()
            .find(|r| r.repo == link.dependency_repo)
            .context("Unknown projection repository")?;
        let base = r
            .bare
            .parent()
            .context("Missing Store object")?
            .join("checkouts")
            .join(&r.locked_commit);
        let rel = link
            .source
            .strip_prefix(&base)
            .context("Unknown read-only checkout layout")?;
        let target = confined(&base, &rel.to_string_lossy())?;
        let repo = git2::Repository::open_bare(&r.bare)?;
        let tree = repo
            .find_commit(git2::Oid::from_str(&r.locked_commit)?)?
            .tree()?;
        let entry = tree.get_path(rel)?;
        let blob = repo.find_blob(entry.id())?;
        atomic_bytes(&target, blob.content())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &target,
                fs::Permissions::from_mode(if entry.filemode() & 0o111 != 0 {
                    0o555
                } else {
                    0o444
                }),
            )?;
        }
        Ok(())
    }
    pub fn preview_resume(&self, operation: &str) -> Result<Plan> {
        let op = self.operation(operation)?;
        if !["archive", "migration", "restore", "summary"].contains(&op.kind.as_str()) {
            bail!("This operation cannot be resumed");
        }
        self.simple_plan(
            "resume",
            serde_json::json!({"operation_id":operation,"previous_result":op}),
        )
    }
    fn resume_operation_plan(&mut self, plan: &mut Plan) -> Result<Operation> {
        if plan.expires_at < Utc::now() || plan.config_hash != self.config_hash()? {
            bail!("PLAN_STALE: preview resume again");
        }
        let oid = plan
            .value
            .as_ref()
            .and_then(|v| v["operation_id"].as_str())
            .context("Missing resume operation")?;
        let mut op = self.operation(oid)?;
        let original = self.plan(&op.plan_id)?;
        op.attempt_history.push(serde_json::json!({"attempt":op.attempts,"state":op.state,"results":op.results,"error":op.error,"at":Utc::now()}));
        op.attempts += 1;
        op.state = "running".into();
        op.error = None;
        if op.kind == "archive" {
            for i in 0..op.results.len() {
                if !["failed", "unfinished"].contains(&op.results[i].outcome.as_str()) {
                    continue;
                }
                if let Err(err) = self.archive_entry(&original.entries[i], &mut op, i, true) {
                    op.results[i].outcome = "failed".into();
                    op.results[i].message = format!("{err:#}");
                    op.results[i].needs_attention = !op.results[i].completed_steps.is_empty();
                }
                self.save_operation(&mut op)?;
            }
            Self::finish(&mut op);
        } else if op.kind == "migration" {
            if let Err(err) = self.migrate(&mut op) {
                op.state = "failed".into();
                op.error = Some(format!("{err:#}"));
            }
        } else if op.kind == "restore" || op.kind == "summary" {
            for r in &mut op.results {
                if r.outcome != "restored" {
                    r.outcome = "unfinished".into();
                    r.reason_codes.clear();
                    r.message.clear();
                }
            }
            match self.execute_other(&original, &mut op) {
                Ok(()) => Self::finish(&mut op),
                Err(err) => {
                    op.state = "failed".into();
                    op.error = Some(format!("{err:#}"));
                    for r in &mut op.results {
                        r.outcome = "failed".into();
                        r.message = format!("{err:#}");
                        r.needs_attention = !r.completed_steps.is_empty();
                    }
                }
            }
        } else {
            bail!("Review current restore/summary state and create a fresh plan; no existing files will be overwritten");
        }
        self.save_operation(&mut op)?;
        plan.operation_id = Some(op.operation_id.clone());
        self.save_plan(plan)?;
        Ok(op)
    }
}
