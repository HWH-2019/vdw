use super::preferences;
use crate::{
    commands::status,
    config::parser,
    lifecycle::{
        activity, io::*, ArchiveRecord, Engine, Lease, LifecycleConfig, Manifest, Operation, Plan,
        Policy, Protection,
    },
    lockfile::WorkspaceLock,
    services::settings,
    store::{manager, registry},
    types::config::Dependency,
};
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use git2::{BranchType, Repository, StatusOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path};

pub fn snapshot() -> Result<Value> {
    snapshot_for(None)
}
pub fn workspace_snapshot(id: &str) -> Result<Value> {
    snapshot_for(Some(id))
}
fn snapshot_for(selected: Option<&str>) -> Result<Value> {
    let home = manager::get_vdw_home()?;
    let config = manager::load_global_config()?;
    let store = manager::get_store_path()?;
    let registry = registry::load_workspace_registry_at(&home)?;
    let protections: BTreeMap<String, Protection> =
        read_or(&home.join("lifecycle/protection.json"))?;
    let policies: BTreeMap<String, Policy> = read_or(&home.join("lifecycle/policies.json"))?;
    let leases: Vec<Lease> = read_or(&home.join("lifecycle/leases.json"))?;
    let archives: Vec<ArchiveRecord> = read_or(&home.join("lifecycle/archives.json"))?;
    let prefs = preferences::load()?;
    let now = Utc::now();
    let rows = registry.workspaces.iter().filter(|ws| ws.lifecycle_state != "trashed" && selected.map(|id|ws.workspace_id==id).unwrap_or(true)).map(|ws| {
        let archived = ws.lifecycle_state == "archived";
        let records: Vec<_> = archives.iter().filter(|a| a.workspace_id == ws.workspace_id).collect();
        let mut repos = Vec::new();
        let mut errors = Vec::new();
        let mut last_modified_at = None;
        let mut modified_error = None;
        if archived {
            if let Some(record) = records.iter().find(|a| Some(&a.archive_id) == ws.latest_archive_id.as_ref()) {
                let manifest = (|| -> Result<Manifest> {
                    let path = confined(&record.location, "manifest.json")?;
                    if hash_file(&path)? != record.manifest_hash { bail!("Archive manifest checksum mismatch"); }
                    let m: Manifest = read(&path)?;
                    if m.archive_id != record.archive_id || m.workspace_id != ws.workspace_id { bail!("Archive identity mismatch"); }
                    Ok(m)
                })();
                match manifest {
                    Ok(m) => {
                        last_modified_at = m.last_modified_at;
                        let declared = archived_branches(&record.location, &m.files).unwrap_or_else(|error| {
                            errors.push(format!("{error:#}")); BTreeMap::new()
                        });
                        for repo in m.repositories {
                        let development_branch = declared.get(&repo.path).cloned().unwrap_or_else(|| repo.branch.clone());
                        repos.push(json!({
                        "id": repo.path, "name": repo_name(&repo.repo), "remote": repo.repo,
                        "web_url": web_url(&repo.repo, &prefs.remote_hosts), "branch_web_url":branch_url(&repo.repo, development_branch.as_deref(), &prefs.remote_hosts),
                        "development_branch":development_branch,"revision_kind":"branch","path": repo.path, "editable": repo.editable,
                        "declared_revision": repo.branch, "branch": repo.branch, "head": repo.head,
                        "locked_commit": repo.locked_commit, "topology":"archived", "git":null, "archive_blockers":[], "archive_warnings":repo.archive_warnings,
                        "remote_state": if is_remote(&repo.repo) {"pending"} else {"local"}
                    })); } },
                    Err(e) => errors.push(format!("{e:#}")),
                }
            } else { errors.push("归档索引缺失，请检查生命周期记录".into()); }
        } else {
            match activity::last_modified_at(&ws.path) {
                Ok(value) => last_modified_at = value,
                Err(error) => modified_error = Some(format!("{error:#}")),
            }
            match parser::load_config(&ws.path) {
                Ok(declaration) => {
                    let topology = status::inspect_workspace(&ws.path, &store);
                    if let Err(e) = &topology { errors.push(format!("{e:#}")); }
                    let lock = WorkspaceLock::load(&ws.path);
                    if let Err(e) = &lock { errors.push(format!("{e:#}")); }
                    for dep in &declaration.dependencies {
                        let t = topology.as_ref().ok().and_then(|v| v.repositories.iter().find(|r| r.path == dep.path));
                        let actual = if dep.editable { inspect_git(&ws.path, dep) } else { Ok(Value::Null) };
                        let (git, git_error) = match actual { Ok(v) => (v, None), Err(e) => (Value::Null, Some(format!("{e:#}"))) };
                        let mut archive_blockers = Vec::new();
                        let mut archive_warnings = Vec::new();
                        if dep.editable && git_error.is_none() {
                            match confined(&ws.path, &dep.path).and_then(|path| crate::lifecycle::inspection::inspect_submodules(&path)) {
                                Ok(inspection) => {
                                    archive_blockers.extend(inspection.blockers);
                                    archive_warnings.extend(inspection.warnings);
                                },
                                Err(error) => archive_blockers.push(crate::lifecycle::inspection::ArchiveNotice {
                                    code:"archive_inspection_failed".into(),path:dep.path.clone(),message:format!("归档结构检查失败：{error:#}"),
                                }),
                            }
                        }
                        let source = dep.dep_id();
                        let locked = lock.as_ref().ok().and_then(|l| l.as_ref()).and_then(|l| l.dependencies.iter().find(|r| r.path == dep.path));
                        repos.push(json!({"id":dep.path,"name":repo_name(source),"remote":source,
                            "web_url":web_url(source, &prefs.remote_hosts),"branch_web_url":branch_url(source,dep.branch.as_deref(),&prefs.remote_hosts),
                            "development_branch":dep.branch,"path":dep.path,"editable":dep.editable,
                            "declared_revision":dep.branch.as_ref().or(dep.tag.as_ref()).or(dep.commit.as_ref()),
                            "revision_kind":if dep.branch.is_some(){"branch"}else if dep.tag.is_some(){"tag"}else{"commit"},
                            "branch":git["branch"],"head":git["head"],"locked_commit":locked.map(|l|&l.resolved_commit),
                            "topology":t.map(|r|serde_json::to_value(r.topology_state).unwrap_or(Value::Null)),
                            "topology_message":t.and_then(|r|r.message.as_ref()), "git":git,"git_error":git_error,"archive_blockers":archive_blockers,"archive_warnings":archive_warnings,
                            "scope":dep.scope,"remote_state":if is_remote(source){"pending"}else{"local"}}));
                    }
                }
                Err(e) => errors.push(format!("{e:#}")),
            }
        }
        let dirty = repos.iter().filter(|r| r["git"]["change_count"].as_u64().unwrap_or(0) > 0).count();
        let issues = repos.iter().filter(|r| r["git_error"].is_string()
            || !matches!(r["topology"].as_str(), Some("ready"|"archived"))
            || r["git"]["branch_drift"] == true
            || r["archive_blockers"].as_array().is_some_and(|items|!items.is_empty())).count() + errors.len();
        let policy = policies.get(&ws.workspace_id).cloned().unwrap_or_default();
        let protected = protections.get(&ws.workspace_id).map(|p|p.expires_at.map(|t|t>now).unwrap_or(true)).unwrap_or(false);
        let active_lease = leases.iter().any(|l|l.workspace_id==ws.workspace_id&&l.expires_at>now);
        let blocked = if protected { Some("工作空间受保护") }
            else if active_lease { Some("工作空间仍在使用") }
            else if dirty > 0 { Some("存在未提交变更，自动归档会跳过整个工作空间") }
            else if issues > 0 || modified_error.is_some() { Some("工作空间状态需检查") }
            else { None };
        let status = display_status(ws, &policy, &config.lifecycle, last_modified_at, blocked, now);
        json!({"workspace":ws,"repositories":repos,"errors":errors,"dirty_count":dirty,"issue_count":issues,
            "last_modified_at":last_modified_at,"modified_error":modified_error,"status":status,
            "protected":protected,"active_lease":active_lease,
            "policy":policy,"effective":{"idle_days":policy.idle_days.unwrap_or(config.lifecycle.idle_days),
                "idle_days_source":if policy.idle_days.is_some(){"workspace_override"}else{"global"},
                "archive_root":policy.archive_root.as_ref().unwrap_or(&config.lifecycle.archive.root),
                "auto_enabled":config.lifecycle.auto.enabled&&policy.auto_enabled.unwrap_or(true)},"archives":records})
    }).collect::<Vec<_>>();
    Ok(json!({"workspaces":rows,"checked_at":Utc::now(),"version":env!("CARGO_PKG_VERSION")}))
}

fn archived_branches(
    root: &Path,
    files: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Option<String>>> {
    for relative in ["config/.vdwrc", "config/.vdwrc.yml"] {
        if let Some(expected) = files.get(relative) {
            let path = confined(root, relative)?;
            if hash_file(&path)? != *expected {
                bail!("Archived workspace configuration checksum mismatch");
            }
            let config = parser::load_config(
                path.parent()
                    .context("Missing archive configuration directory")?,
            )?;
            return Ok(config
                .dependencies
                .into_iter()
                .map(|dep| (dep.path, dep.branch))
                .collect());
        }
    }
    Ok(BTreeMap::new())
}

fn display_status(
    ws: &registry::WorkspaceRegistration,
    policy: &Policy,
    config: &LifecycleConfig,
    modified: Option<DateTime<Utc>>,
    blocked: Option<&str>,
    now: DateTime<Utc>,
) -> Value {
    let state =
        |name: &str, message: &str| json!({"state":name,"archive_at":null,"message":message});
    if ws.lifecycle_state == "archived" {
        return state("archived", "工作空间已归档");
    }
    if ws.lifecycle_state == "archiving" {
        return state("pending_archive", "正在执行已确认的归档操作");
    }
    if ws.lifecycle_state != "active" {
        return state("active", "工作空间有进行中的操作，请查看操作记录");
    }
    if !config.auto.enabled || policy.auto_enabled == Some(false) {
        return state("active", "自动归档未启用");
    }
    if let Some(reason) = blocked {
        return state("active", reason);
    }
    if policy
        .retain_until
        .map(|until| until > now)
        .unwrap_or(false)
    {
        return state("active", "工作空间仍在保留期内");
    }
    // A read-only estimate: never advance activity or create lifecycle candidates here.
    let baseline = ws
        .last_active_at
        .unwrap_or(ws.last_seen)
        .max(ws.observation_started_at.unwrap_or(ws.last_seen));
    let baseline = modified.map(|time| baseline.max(time)).unwrap_or(baseline);
    let eligible_at =
        baseline + Duration::days(i64::from(policy.idle_days.unwrap_or(config.idle_days)));
    let candidate_at = ws
        .candidate_since
        .filter(|since| *since >= eligible_at)
        .unwrap_or(eligible_at);
    let archive_at = candidate_at + Duration::hours(i64::from(config.auto.grace_hours));
    json!({"state":if now >= archive_at - Duration::days(7) {"pending_archive"} else {"active"},
        "archive_at":archive_at,"message":"按最近活动、源码修改时间、闲置阈值及宽限期预计；归档前 7 天预警，执行时仍会重新校验"})
}

pub fn exact_workspace(id: &str) -> Result<registry::WorkspaceRegistration> {
    registry::load_workspace_registry()?
        .workspaces
        .into_iter()
        .find(|w| w.workspace_id == id)
        .context("Workspace ID not found")
}
pub fn inspect_git(root: &Path, dep: &Dependency) -> Result<Value> {
    let path = confined(root, &dep.path)?;
    let repo = Repository::open(&path).context("Cannot inspect actual Git worktree")?;
    if repo.workdir().and_then(|p| p.canonicalize().ok()) != Some(path.canonicalize()?) {
        bail!("Dependency is not an independent Git worktree");
    }
    let head = repo.head()?;
    let branch = if head.is_branch() {
        head.shorthand().map(String::from)
    } else {
        None
    };
    let oid = head.peel_to_commit()?.id();
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .update_index(false);
    let entries = repo.statuses(Some(&mut options))?;
    let (mut staged, mut unstaged, mut untracked, mut conflicts) = (0, 0, 0, 0);
    for entry in entries.iter() {
        let s = entry.status();
        if s.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            staged += 1;
        }
        if s.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            unstaged += 1;
        }
        if s.contains(git2::Status::WT_NEW) {
            untracked += 1;
        }
        if s.contains(git2::Status::CONFLICTED) {
            conflicts += 1;
        }
    }
    let upstream = branch
        .as_deref()
        .and_then(|name| repo.find_branch(name, BranchType::Local).ok())
        .and_then(|branch| branch.upstream().ok())
        .and_then(|b| b.get().target());
    let distance = upstream.and_then(|u| repo.graph_ahead_behind(oid, u).ok());
    Ok(
        json!({"branch":branch,"head":oid.to_string(),"staged":staged,"unstaged":unstaged,"untracked":untracked,
        "conflicts":conflicts,"change_count":entries.len(),"ahead":distance.map(|d|d.0),"behind":distance.map(|d|d.1),
        "branch_drift":dep.branch.as_ref().map(|b|Some(b)!=branch.as_ref()).unwrap_or(false),
        "origin":repo.find_remote("origin").ok().and_then(|r|r.url().map(String::from))}),
    )
}
pub fn is_remote(source: &str) -> bool {
    source.starts_with("https://")
        || source.starts_with("http://")
        || source.starts_with("git@")
        || source.starts_with("ssh://")
}
pub fn repo_name(source: &str) -> String {
    source
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(source)
        .trim_end_matches(".git")
        .to_string()
}
pub fn web_url(source: &str, hosts: &BTreeMap<String, String>) -> Option<String> {
    if !is_remote(source) {
        return None;
    }
    let stripped = source
        .strip_prefix("https://")
        .or_else(|| source.strip_prefix("http://"))
        .or_else(|| source.strip_prefix("ssh://"));
    let (host, path) = if let Some(rest) = source.strip_prefix("git@") {
        rest.split_once(':')?
    } else {
        let (authority, path) = stripped?.split_once('/')?;
        (authority.rsplit('@').next()?, path)
    };
    if path.contains(['?', '#', '\n', '\r']) || host.is_empty() {
        return None;
    }
    let base = hosts.get(host).cloned().unwrap_or_else(|| {
        format!(
            "{}://{host}",
            if source.starts_with("http://") {
                "http"
            } else {
                "https"
            }
        )
    });
    Some(format!(
        "{}/{}",
        base.trim_end_matches('/'),
        path.trim_end_matches(".git")
    ))
}

pub fn url_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

pub fn branch_url(
    source: &str,
    branch: Option<&str>,
    hosts: &BTreeMap<String, String>,
) -> Option<String> {
    let branch = branch.filter(|name| !name.is_empty())?;
    let base = web_url(source, hosts)?;
    let host = base
        .split("://")
        .nth(1)?
        .split('/')
        .next()?
        .to_ascii_lowercase();
    let route = if host.contains("github") || host == "gitee.com" {
        "tree"
    } else if host.contains("bitbucket") {
        "src"
    } else {
        "-/tree"
    };
    Some(format!("{base}/{route}/{}", url_component(branch)))
}

pub fn configuration() -> Result<Value> {
    let config = manager::load_global_config()?;
    let prefs = preferences::load()?;
    Ok(
        json!({"global":config,"version":settings::version(&config)?,"preferences":prefs,
        "preferences_version":preferences::version(&prefs)?,"editors":preferences::editors(),
        "defaults":manager::GlobalConfig::default(),"read_only":["store_path","lifecycle.version","lifecycle.dirty_policy",
            "lifecycle.archive.keep_workspace_skeleton","lifecycle.archive.materials.exclude_managed_dependencies",
            "lifecycle.archive.migration","lifecycle.memory.index_required","lifecycle.cloud"],
        "capabilities":{"native_scheduler":cfg!(target_os="macos"),"remote_deletion_proof":false}}),
    )
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigRequest {
    pub kind: String,
    pub version: String,
    pub value: Value,
    pub workspace_id: Option<String>,
    pub key: Option<String>,
}
pub fn preview_config(request: ConfigRequest) -> Result<Plan> {
    let e = Engine::open()?;
    if settings::version(&e.config)? != request.version {
        bail!("PLAN_STALE: configuration changed; reload first");
    }
    match request.kind.as_str() {
        "base" => e.preview_settings(serde_json::from_value(request.value)?),
        "lifecycle" => {
            let next: crate::lifecycle::LifecycleConfig =
                serde_json::from_value(request.value.clone())?;
            if next.auto.enabled != e.config.lifecycle.auto.enabled
                || next.auto.authorization != e.config.lifecycle.auto.authorization
            {
                bail!("Use the dedicated automatic archiving enable/disable action");
            }
            let mut plan = e.preview_config("lifecycle", request.value, None)?;
            if let Some(value) = plan.value.as_mut().and_then(Value::as_object_mut) {
                value.insert(
                    "before_config".into(),
                    serde_json::to_value(&e.config.lifecycle)?,
                );
            }
            e.save_plan(&plan)?;
            Ok(plan)
        }
        "policy" => {
            let id = request.workspace_id.context("Missing workspace ID")?;
            exact_workspace(&id)?;
            let key = request.key.context("Missing policy key")?;
            if !["idle_days", "archive_root", "auto_enabled", "retain_until"]
                .contains(&key.as_str())
            {
                bail!("Unsupported policy key");
            }
            let mut plan = e.preview_config(&key, request.value, Some(&id))?;
            if let Some(value) = plan.value.as_mut().and_then(Value::as_object_mut) {
                value.insert(
                    "before_policy".into(),
                    serde_json::to_value(e.policies.get(&id).cloned().unwrap_or_default())?,
                );
            }
            e.save_plan(&plan)?;
            Ok(plan)
        }
        "auto_enable" => e.simple_plan(
            "auto_enable",
            json!({"policy":e.config.lifecycle,"install_scheduler":cfg!(target_os="macos"),
            "scope":e.registry.workspaces,"dirty_policy":"skip_workspace"}),
        ),
        "auto_disable" => e.simple_plan("auto_disable", Value::Null),
        _ => bail!("Unknown configuration action"),
    }
}
pub fn operations() -> Result<Value> {
    let home = manager::get_vdw_home()?;
    let dir = home.join("lifecycle/operations");
    let mut entries = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                match read::<Operation>(&path) {
                    Ok(op) => entries.push(op),
                    Err(e) => return Err(e),
                }
            }
        }
    }
    entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    let mut visible = Vec::new();
    for operation in entries {
        let plan = read::<Plan>(&named_file(
            &home.join("lifecycle/plans"),
            &operation.plan_id,
        )?)
        .ok();
        if plan
            .as_ref()
            .and_then(|p| p.value.as_ref())
            .is_some_and(|v| v["undo_parent"].is_string())
        {
            continue;
        }
        let mut value = serde_json::to_value(&operation)?;
        value["undo"] = crate::lifecycle::undo::status(&home, &operation);
        visible.push(value);
        if visible.len() == 100 {
            break;
        }
    }
    Ok(json!(visible))
}
pub fn operation(id: &str) -> Result<Operation> {
    read(&named_file(
        &manager::get_vdw_home()?.join("lifecycle/operations"),
        id,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archived_development_branch_comes_from_verified_configuration() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("config")).unwrap();
        let path = root.path().join("config/.vdwrc");
        fs::write(
            &path,
            serde_json::to_vec(&json!({"workspace":{"name":"fixture","version":"1.0"},
            "dependencies":[{"remote":"git@example.com:team/repo.git","path":"packages/sample",
                "branch":"develop","editable":false,"scope":{"include":["**/*"],"exclude":[]}}]}))
            .unwrap(),
        )
        .unwrap();
        let files = BTreeMap::from([("config/.vdwrc".into(), hash_file(&path).unwrap())]);
        assert_eq!(
            archived_branches(root.path(), &files).unwrap()["packages/sample"].as_deref(),
            Some("develop")
        );
        fs::write(&path, "modified").unwrap();
        assert!(archived_branches(root.path(), &files).is_err());
    }

    #[test]
    fn archive_warning_starts_exactly_seven_days_before_and_respects_blockers() {
        let now = DateTime::parse_from_rfc3339("2026-10-23T12:00:00+08:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut ws = registry::WorkspaceRegistration::new(
            std::path::PathBuf::from("/fixture"),
            now - Duration::days(26),
        );
        let mut config = LifecycleConfig::default();
        config.auto.enabled = true;
        let policy = Policy::default();
        let before = display_status(
            &ws,
            &policy,
            &config,
            None,
            None,
            now - Duration::seconds(1),
        );
        assert_eq!(before["state"], "active");
        let warning = display_status(&ws, &policy, &config, None, None, now);
        assert_eq!(warning["state"], "pending_archive");
        assert_eq!(warning["archive_at"], json!(now + Duration::days(7)));
        assert_eq!(
            display_status(&ws, &policy, &config, None, None, now + Duration::days(8))["state"],
            "pending_archive"
        );
        assert_eq!(
            display_status(&ws, &policy, &config, Some(now), None, now)["state"],
            "active"
        );
        assert_eq!(
            display_status(&ws, &policy, &config, None, Some("dirty"), now)["state"],
            "active"
        );
        config.auto.enabled = false;
        assert_eq!(
            display_status(&ws, &policy, &config, None, None, now)["state"],
            "active"
        );
        config.auto.enabled = true;
        let excluded = Policy {
            auto_enabled: Some(false),
            ..Policy::default()
        };
        assert_eq!(
            display_status(&ws, &excluded, &config, None, None, now)["state"],
            "active"
        );
        let retained = Policy {
            retain_until: Some(now + Duration::days(1)),
            ..Policy::default()
        };
        assert_eq!(
            display_status(&ws, &retained, &config, None, None, now)["state"],
            "active"
        );
        ws.candidate_since = Some(now);
        // A candidate older than the current eligibility date is stale and ignored.
        assert_eq!(
            display_status(&ws, &policy, &config, None, None, now)["archive_at"],
            json!(now + Duration::days(7))
        );
        ws.candidate_since = Some(now + Duration::days(5));
        assert_eq!(
            display_status(&ws, &policy, &config, None, None, now)["archive_at"],
            json!(now + Duration::days(8))
        );
        ws.lifecycle_state = "archived".into();
        assert_eq!(
            display_status(&ws, &policy, &config, None, None, now)["state"],
            "archived"
        );
    }

    #[test]
    fn remote_links_strip_credentials_and_support_host_aliases() {
        let mut hosts = BTreeMap::new();
        hosts.insert("company-git".into(), "https://git.example.com".into());
        assert_eq!(
            web_url("git@company-git:team/tool.git", &hosts).as_deref(),
            Some("https://git.example.com/team/tool")
        );
        assert_eq!(
            web_url("https://user:secret@git.example.com/team/tool.git", &hosts).as_deref(),
            Some("https://git.example.com/team/tool")
        );
        assert_eq!(
            web_url("http://git.example.com/team/tool.git", &hosts).as_deref(),
            Some("http://git.example.com/team/tool")
        );
        assert_eq!(web_url("/tmp/local-repo", &hosts), None);
        assert_eq!(
            web_url("https://git.example.com/tool.git?token=x", &hosts),
            None
        );
        assert_eq!(
            branch_url(
                "git@gitlab.example.com:team/tool.git",
                Some("feature/a & b"),
                &hosts
            )
            .as_deref(),
            Some("https://gitlab.example.com/team/tool/-/tree/feature%2Fa%20%26%20b")
        );
        assert_eq!(
            branch_url("git@github.com:team/tool.git", Some("main"), &hosts).as_deref(),
            Some("https://github.com/team/tool/tree/main")
        );
        assert_eq!(branch_url("/tmp/local", Some("main"), &hosts), None);
    }

    #[test]
    fn actual_git_branch_dirty_state_and_missing_worktree_are_distinct() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("packages/repo");
        fs::create_dir_all(&path).unwrap();
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("actual-branch");
        let repository = Repository::init_opts(&path, &options).unwrap();
        fs::write(path.join("tracked.txt"), "initial").unwrap();
        let mut index = repository.index().unwrap();
        index.add_path(Path::new("tracked.txt")).unwrap();
        index.write().unwrap();
        let oid = index.write_tree().unwrap();
        let tree = repository.find_tree(oid).unwrap();
        let signature = git2::Signature::now("VDW test", "test@example.invalid").unwrap();
        repository
            .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
            .unwrap();
        let dependency: Dependency = serde_json::from_value(json!({"local":path,"path":"packages/repo", "branch":"configured-branch", "editable":true,"scope":{"include":["**/*"],"exclude":[]}})).unwrap();
        let clean = inspect_git(root.path(), &dependency).unwrap();
        assert_eq!(clean["branch"], "actual-branch");
        assert_eq!(clean["change_count"], 0);
        assert_eq!(clean["branch_drift"], true);
        assert_eq!(clean["ahead"], Value::Null);
        fs::write(path.join("untracked.txt"), "preserve").unwrap();
        let dirty = inspect_git(root.path(), &dependency).unwrap();
        assert_eq!(dirty["untracked"], 1);
        let mut missing = dependency.clone();
        missing.path = "packages/missing".into();
        assert!(inspect_git(root.path(), &missing).is_err());
        let mut escaping = dependency;
        escaping.path = "../outside".into();
        assert!(inspect_git(root.path(), &escaping).is_err());
    }
}
pub fn plan(id: &str) -> Result<Plan> {
    read(&named_file(
        &manager::get_vdw_home()?.join("lifecycle/plans"),
        id,
    )?)
}
