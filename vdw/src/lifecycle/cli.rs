use super::{engine::Engine, io::*, types::*};
use anyhow::{bail, Context, Result};
use clap::Subcommand;
use serde::Serialize;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Subcommand)]
pub enum WorkspaceCommands {
    /// Inspect exact-commit reconstruction without changing any workspace or Store state.
    Rebuild {
        /// Exact registered ID/name/path, or an existing unregistered workspace path. Defaults to all registrations.
        workspaces: Vec<String>,
        #[arg(long, required = true)]
        preview: bool,
        /// Search local branch reflogs and commit messages for candidates; never fetch.
        #[arg(long)]
        search_history: bool,
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Status {
        workspace: String,
        #[arg(long)]
        json: bool,
    },
    Archive {
        #[arg(required = true)]
        workspaces: Vec<String>,
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        git_backup: Option<String>,
        #[arg(long)]
        measure_bundle: bool,
        #[arg(long)]
        client: Option<String>,
    },
    Restore {
        archive: String,
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        branch: Option<String>,
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        json: bool,
    },
    Touch {
        workspace: String,
        #[arg(long, default_value = "manual")]
        source: String,
        #[arg(long)]
        json: bool,
    },
    Lease {
        workspace: String,
        #[arg(long)]
        client: String,
        #[arg(long)]
        release: bool,
    },
    Protect {
        #[command(subcommand)]
        command: ProtectCommands,
    },
    Memory {
        #[command(subcommand)]
        command: MemoryCommands,
    },
}
#[derive(Subcommand)]
pub enum ProtectCommands {
    Add {
        workspace: String,
        #[arg(long, default_value = "manual protection")]
        reason: String,
        #[arg(long)]
        expires: Option<String>,
    },
    Remove {
        workspace: String,
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        json: bool,
    },
    List {
        #[arg(long)]
        json: bool,
    },
}
#[derive(Subcommand)]
pub enum MemoryCommands {
    Summarize {
        archive: String,
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        json: bool,
    },
    List {
        archive: String,
        #[arg(long)]
        json: bool,
    },
}
#[derive(Subcommand)]
pub enum LifecycleCommands {
    Scan {
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        json: bool,
    },
    Run {
        #[arg(long)]
        json: bool,
    },
    Plan {
        #[command(subcommand)]
        command: PlanCommands,
    },
    Operation {
        #[command(subcommand)]
        command: OperationCommands,
    },
    Auto {
        #[command(subcommand)]
        command: AutoCommands,
    },
    Capabilities,
    #[command(hide = true)]
    MemoryWorker {
        archive: String,
    },
    #[command(hide = true)]
    MigrationWorker {
        operation: String,
    },
}
#[derive(Subcommand)]
pub enum PlanCommands {
    Show {
        plan: String,
        #[arg(long)]
        json: bool,
    },
    Adjust {
        plan: String,
        #[arg(long)]
        exclude_workspace: Vec<String>,
        #[arg(long)]
        git_backup: Option<String>,
        #[arg(long)]
        json: bool,
    },
    Execute {
        plan: String,
        #[arg(long, required = true)]
        confirm: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        background: bool,
    },
}
#[derive(Subcommand)]
pub enum OperationCommands {
    Status {
        operation: String,
        #[arg(long)]
        json: bool,
    },
    Watch {
        operation: String,
        #[arg(long)]
        json: bool,
    },
    Resume {
        operation: String,
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        json: bool,
    },
    Pause {
        operation: String,
    },
    Continue {
        operation: String,
        #[arg(long)]
        json: bool,
    },
    Cancel {
        operation: String,
    },
}
#[derive(Subcommand)]
pub enum AutoCommands {
    Enable {
        #[arg(long, required = true)]
        preview: bool,
        #[arg(long)]
        no_scheduler: bool,
        #[arg(long)]
        json: bool,
    },
    Disable,
}
#[derive(Debug)]
pub struct LifecycleExit {
    pub code: i32,
    pub message: String,
}
impl std::fmt::Display for LifecycleExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for LifecycleExit {}

pub fn workspace(command: WorkspaceCommands) -> Result<()> {
    // Atomic control files are readable while another process holds the mutation guard.
    let home = crate::store::manager::get_vdw_home()?;
    if let WorkspaceCommands::Rebuild {
        workspaces,
        search_history,
        json,
        ..
    } = &command
    {
        let report = super::rebuild::preview(workspaces, *search_history)?;
        return super::rebuild::print(&report, *json);
    }
    if let WorkspaceCommands::List { state, json } = &command {
        let registry = crate::store::registry::load_workspace_registry_at(&home)?;
        return emit(
            &registry
                .workspaces
                .iter()
                .filter(|w| {
                    state
                        .as_ref()
                        .map(|s| s == &w.lifecycle_state)
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>(),
            *json,
        );
    }
    if let WorkspaceCommands::Status { workspace, json } = &command {
        let registry = crate::store::registry::load_workspace_registry_at(&home)?;
        let candidates = registry
            .workspaces
            .iter()
            .filter(|w| {
                &w.workspace_id == workspace
                    || &w.name == workspace
                    || w.path == std::path::Path::new(workspace)
            })
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            bail!("Workspace selector is missing or ambiguous");
        }
        let ws = candidates[0];
        let archives: Vec<ArchiveRecord> = read_or(&home.join("lifecycle/archives.json"))?;
        let protections: std::collections::BTreeMap<String, Protection> =
            read_or(&home.join("lifecycle/protection.json"))?;
        let policies: std::collections::BTreeMap<String, Policy> =
            read_or(&home.join("lifecycle/policies.json"))?;
        let leases: Vec<Lease> = read_or(&home.join("lifecycle/leases.json"))?;
        return emit(
            &serde_json::json!({"workspace":ws,"archives":archives.iter().filter(|a|a.workspace_id==ws.workspace_id).collect::<Vec<_>>(),"protected":protections.get(&ws.workspace_id).map(|p|p.expires_at.map(|t|t>chrono::Utc::now()).unwrap_or(true)).unwrap_or(false),"active_lease":leases.iter().any(|l|l.workspace_id==ws.workspace_id&&l.expires_at>chrono::Utc::now()),"policy":policies.get(&ws.workspace_id)}),
            *json,
        );
    }
    if let WorkspaceCommands::Memory {
        command: MemoryCommands::List { archive, json },
    } = &command
    {
        let archives: Vec<ArchiveRecord> = read_or(&home.join("lifecycle/archives.json"))?;
        return emit(
            archives
                .iter()
                .find(|a| &a.archive_id == archive)
                .context("Archive not found")?,
            *json,
        );
    }
    if let WorkspaceCommands::Protect {
        command: ProtectCommands::List { json },
    } = &command
    {
        let records: std::collections::BTreeMap<String, Protection> =
            read_or(&home.join("lifecycle/protection.json"))?;
        return emit(&records, *json);
    }
    let mut e = Engine::open()?;
    match command {
        WorkspaceCommands::Rebuild { .. } => {
            unreachable!("handled without opening the mutation engine")
        }
        WorkspaceCommands::List { state, json } => {
            let rows = e
                .registry
                .workspaces
                .iter()
                .filter(|w| {
                    state
                        .as_ref()
                        .map(|s| &w.lifecycle_state == s)
                        .unwrap_or(true)
                })
                .collect::<Vec<_>>();
            emit(&rows, json)
        }
        WorkspaceCommands::Status { workspace, json } => {
            let ws = e.find(&workspace)?;
            let archives = e
                .archives
                .iter()
                .filter(|a| a.workspace_id == ws.workspace_id)
                .collect::<Vec<_>>();
            emit(
                &serde_json::json!({"workspace":ws,"archives":archives,"protected":e.protection(&ws.workspace_id),"active_lease":e.active_lease(&ws.workspace_id),"policy":e.policies.get(&ws.workspace_id)}),
                json,
            )
        }
        WorkspaceCommands::Archive {
            workspaces,
            preview: _,
            json,
            git_backup,
            measure_bundle,
            client,
        } => {
            if let Some(client) = &client {
                e.leases.retain(|l| &l.client_id != client);
            }
            let mut p = e.preview_archive(&workspaces, false)?;
            if let Some(client) = client {
                p.value = Some(serde_json::json!({"automatic":false,"client":client}));
            }
            if let Some(mode) = git_backup {
                if !["store_refs", "portable_bundle"].contains(&mode.as_str()) {
                    bail!("Invalid git_backup");
                }
                for w in &mut p.entries {
                    w.git_backup = mode.clone();
                }
            }
            if measure_bundle {
                measure(&e, &mut p)?;
            }
            e.save_plan(&p)?;
            print_plan(&p, json)
        }
        WorkspaceCommands::Restore {
            archive,
            target,
            branch,
            preview: _,
            json,
        } => print_plan(&e.preview_restore(&archive, target, branch)?, json),
        WorkspaceCommands::Touch {
            workspace,
            source,
            json,
        } => emit(&e.touch(&workspace, &source)?, json),
        WorkspaceCommands::Lease {
            workspace,
            client,
            release,
        } => {
            e.lease(&workspace, &client, release)?;
            Ok(())
        }
        WorkspaceCommands::Protect { command } => match command {
            ProtectCommands::Add {
                workspace,
                reason,
                expires,
            } => {
                let ws = e.find(&workspace)?;
                e.protections.insert(
                    ws.workspace_id.clone(),
                    Protection {
                        reason,
                        added_at: chrono::Utc::now(),
                        expires_at: expires.map(|v| v.parse()).transpose()?,
                    },
                );
                save(&e.base().join("protection.json"), &e.protections)?;
                if let Some(w) = e
                    .registry
                    .workspaces
                    .iter_mut()
                    .find(|w| w.workspace_id == ws.workspace_id)
                {
                    w.candidate_since = None;
                }
                e.persist_registry()?;
                println!("Protected {}", ws.name);
                Ok(())
            }
            ProtectCommands::Remove {
                workspace,
                preview: _,
                json,
            } => {
                let ws = e.find(&workspace)?;
                print_plan(&e.simple_plan("protect_remove",serde_json::json!({"workspace_id":ws.workspace_id,"name":ws.name,"protection":e.protections.get(&ws.workspace_id)}))?,json)
            }
            ProtectCommands::List { json } => emit(&e.protections, json),
        },
        WorkspaceCommands::Memory { command } => match command {
            MemoryCommands::Summarize {
                archive,
                preview: _,
                json,
            } => print_plan(&e.preview_summary(&archive)?, json),
            MemoryCommands::List { archive, json } => emit(&e.archive_record(&archive)?, json),
        },
    }
}
pub fn lifecycle(command: LifecycleCommands) -> Result<()> {
    if let LifecycleCommands::Plan {
        command: PlanCommands::Show { plan, json },
    } = &command
    {
        let path = named_file(
            &crate::store::manager::get_vdw_home()?.join("lifecycle/plans"),
            plan,
        )?;
        return print_plan(&read(&path)?, *json);
    }
    // Status/control reads never wait behind the worker they monitor.
    if let LifecycleCommands::Operation { command } = command {
        return operation_command(command);
    }
    if let LifecycleCommands::Capabilities = command {
        return emit(
            &serde_json::json!({"schema_version":1,"lifecycle":true,"dirty_policy":"skip_workspace","rebuild":{"preview":true,"execute":false,"local_history_search":true},"cloud":[ProviderCapabilities{provider:"kbase".into(),implemented:false,manifest:false,memory:false,blobs:false,download:false}],"external_triggers":{"implemented":false,"contract":"ArchiveTrigger/v1"},"ai_adapters":["command","http"],"native_scheduler":if cfg!(target_os="macos"){Some("launchd")}else{None}}),
            true,
        );
    }
    if let LifecycleCommands::MemoryWorker { archive } = command {
        let _worker = match Guard::job(&crate::store::manager::get_vdw_home()?, &archive) {
            Ok(g) => g,
            Err(_) => return Ok(()),
        };
        let start = Instant::now();
        let mut e = loop {
            match Engine::open() {
                Ok(e) => break e,
                Err(err) => {
                    if start.elapsed() > Duration::from_secs(30) {
                        return Err(err);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        let p = e.preview_summary(&archive)?;
        let op = e.execute_plan(&p.plan_id)?;
        if op.state == "failed" {
            if let Some(a) = e.archives.iter_mut().find(|a| a.archive_id == archive) {
                a.memory_state = "failed".into();
            }
            e.persist_archives()?;
        }
        return Ok(());
    }
    if let LifecycleCommands::MigrationWorker { operation } = command {
        let _worker = match Guard::job(&crate::store::manager::get_vdw_home()?, &operation) {
            Ok(g) => g,
            Err(_) => return Ok(()),
        };
        let start = Instant::now();
        let mut e = loop {
            match Engine::open() {
                Ok(e) => break e,
                Err(err) => {
                    if start.elapsed() > Duration::from_secs(30) {
                        return Err(err);
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        };
        let mut op = e.operation(&operation)?;
        if op.state != "running" {
            return Ok(());
        }
        if let Err(err) = e.migrate(&mut op) {
            op.state = "failed".into();
            op.error = Some(format!("{err:#}"));
            e.save_operation(&mut op)?;
        }
        return Ok(());
    }
    let mut e = Engine::open()?;
    match command {
        LifecycleCommands::Scan { preview: _, json } => {
            print_plan(&e.preview_archive(&[], true)?, json)
        }
        LifecycleCommands::Run { json } => {
            if !e.config.lifecycle.auto.enabled {
                emit(
                    &serde_json::json!({"state":"disabled","message":"Automatic archiving is disabled; no workspaces were changed"}),
                    json,
                )?;
                return Ok(());
            }
            if e.config.lifecycle.auto.authorization.as_deref()
                != Some(e.authorize_hash()?.as_str())
            {
                bail!("Automatic policy changed; preview and confirm lifecycle auto enable again");
            }
            let p = e.preview_archive(&[], true)?;
            let op = e.execute_plan(&p.plan_id)?;
            drop(e);
            print_operation(&op, json)?;
            start_summaries(&op);
            exit_for(&op)
        }
        LifecycleCommands::Plan { command } => match command {
            PlanCommands::Show { plan, json } => print_plan(&e.plan(&plan)?, json),
            PlanCommands::Adjust {
                plan,
                exclude_workspace,
                git_backup,
                json,
            } => {
                let mut old = e.plan(&plan)?;
                if old.operation_id.is_some() {
                    bail!("Executed plans cannot be adjusted");
                }
                let mut next = old.clone();
                next.plan_id = id("plan");
                next.created_at = chrono::Utc::now();
                next.expires_at = next.created_at + chrono::Duration::minutes(30);
                for s in exclude_workspace {
                    let ws = e.find(&s)?;
                    let row = next
                        .entries
                        .iter_mut()
                        .find(|w| w.workspace_id == ws.workspace_id)
                        .context("Workspace is outside this plan")?;
                    row.excluded = true;
                }
                if let Some(mode) = git_backup {
                    if !["store_refs", "portable_bundle"].contains(&mode.as_str()) {
                        bail!("Invalid git_backup");
                    }
                    for row in &mut next.entries {
                        row.git_backup = mode.clone();
                    }
                }
                e.save_plan(&next)?;
                old.superseded = true;
                e.save_plan(&old)?;
                print_plan(&next, json)
            }
            PlanCommands::Execute {
                plan,
                confirm: _,
                json,
                background,
            } => {
                e.background = background;
                let op = e.execute_plan(&plan)?;
                drop(e);
                print_operation(&op, json)?;
                start_summaries(&op);
                if background && op.kind == "migration" && op.state == "running" {
                    spawn_worker("migration-worker", &op.operation_id);
                }
                exit_for(&op)
            }
        },
        LifecycleCommands::Auto { command } => match command {
            AutoCommands::Enable {
                preview: _,
                no_scheduler,
                json,
            } => {
                let p=e.simple_plan("auto_enable",serde_json::json!({"policy":e.config.lifecycle,"install_scheduler":cfg!(target_os="macos")&&!no_scheduler,"scope":e.registry.workspaces,"dirty_policy":"skip_workspace"}))?;
                print_plan(&p, json)
            }
            AutoCommands::Disable => {
                e.disable_auto()?;
                println!("Automatic archiving disabled");
                Ok(())
            }
        },
        _ => unreachable!(),
    }
}
fn operation_path(id: &str) -> Result<PathBuf> {
    named_file(
        &crate::store::manager::get_vdw_home()?.join("lifecycle/operations"),
        id,
    )
}
fn operation_command(command: OperationCommands) -> Result<()> {
    let control_action = match &command {
        OperationCommands::Pause { .. } => "pause",
        OperationCommands::Cancel { .. } => "cancel",
        _ => "",
    };
    match command {
        OperationCommands::Status { operation, json } => {
            print_operation(&read(&operation_path(&operation)?)?, json)
        }
        OperationCommands::Watch { operation, json } => {
            let mut last = String::new();
            loop {
                let bytes = fs::read(operation_path(&operation)?)?;
                let current = hash(&bytes);
                let op: Operation = serde_json::from_slice(&bytes)?;
                if current != last {
                    print_operation(&op, json)?;
                    last = current;
                }
                if op.state != "running" {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
        OperationCommands::Pause { operation } | OperationCommands::Cancel { operation } => {
            let op: Operation = read(&operation_path(&operation)?)?;
            if op.kind != "migration" {
                bail!("Pause/cancel is supported for archive storage migration only");
            }
            let action = control_action;
            let file = named_file(
                &crate::store::manager::get_vdw_home()?.join("lifecycle/controls"),
                &operation,
            )?;
            save(&file, &serde_json::json!({"action":action}))?;
            println!("Requested {action}: {operation}");
            Ok(())
        }
        OperationCommands::Resume {
            operation,
            preview: _,
            json,
        } => {
            let e = Engine::open()?;
            print_plan(&e.preview_resume(&operation)?, json)
        }
        OperationCommands::Continue { operation, json } => {
            let mut e = Engine::open()?;
            let mut op = e.operation(&operation)?;
            if op.kind != "migration" {
                bail!("Use operation resume --preview for workspace recovery");
            }
            let control = named_file(&e.base().join("controls"), &operation)?;
            if control.exists() {
                fs::remove_file(control)?;
            }
            if let Err(err) = e.migrate(&mut op) {
                op.state = "failed".into();
                op.error = Some(format!("{err:#}"));
                e.save_operation(&mut op)?;
            }
            print_operation(&op, json)?;
            exit_for(&op)
        }
    }
}
pub fn config_preview(
    key: &str,
    value: serde_json::Value,
    workspace: Option<&str>,
    json: bool,
) -> Result<()> {
    let e = Engine::open()?;
    print_plan(&e.preview_config(key, value, workspace)?, json)
}
pub fn effective(workspace: Option<&str>, json: bool) -> Result<()> {
    let e = Engine::open()?;
    let ws = workspace.map(|w| e.find(w)).transpose()?;
    let policy = ws
        .as_ref()
        .and_then(|w| e.policies.get(&w.workspace_id).cloned());
    let p = policy.clone().unwrap_or_default();
    let effective = serde_json::json!({
        "idle_days":{"value":p.idle_days.unwrap_or(e.config.lifecycle.idle_days),"source":if p.idle_days.is_some(){"workspace_override"}else{"global"}},
        "archive_root":{"value":ws.as_ref().map(|w|e.archive_root(&w.workspace_id)).transpose()?.unwrap_or(e.resolve_root(&e.config.lifecycle.archive.root)?),"source":if p.archive_root.is_some(){"workspace_override"}else{"global"}},
        "auto_enabled":{"value":e.config.lifecycle.auto.enabled&&p.auto_enabled.unwrap_or(true),"source":"global_enabled_and_workspace_policy"},
        "retain_until":{"value":p.retain_until,"source":"workspace_override"},
        "protected":ws.as_ref().map(|w|e.protection(&w.workspace_id)).unwrap_or(false),"dirty_policy":"skip_workspace"
    });
    emit(
        &serde_json::json!({"global":e.config,"workspace_override":policy,"effective":effective,"precedence":["protection","reviewed_plan","workspace_override","global"]}),
        json,
    )
}
pub fn emit<T: Serialize>(value: &T, _json: bool) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
pub fn print_plan(plan: &Plan, json: bool) -> Result<()> {
    if json {
        return emit(plan, true);
    }
    println!(
        "Plan {} ({}) — expires {}",
        plan.plan_id, plan.kind, plan.expires_at
    );
    for e in &plan.entries {
        println!(
            "{} [{}] {}",
            e.name,
            e.workspace_id,
            if e.excluded {
                "excluded".into()
            } else if e.reason_codes.is_empty() {
                "eligible".into()
            } else {
                e.reason_codes.join(", ")
            }
        );
        if !e.message.is_empty() {
            println!("  {}", e.message);
        }
        for r in &e.repositories {
            println!("  {}: staged {}, unstaged {}, conflicts {}, untracked {}, unknown ignored {}; HEAD {}",r.path,r.dirty.staged,r.dirty.unstaged,r.dirty.conflicts,r.dirty.untracked,r.dirty.unknown_ignored,r.head);
        }
        println!(
            "  {} project entries; {} bytes; archive {}",
            e.materials.len(),
            e.materials.iter().map(|f| f.bytes).sum::<u64>(),
            e.archive_root.display()
        );
    }
    if let Some(value) = &plan.value {
        println!("{}", serde_json::to_string_pretty(value)?);
    }
    println!(
        "Confirm this exact plan: vdw lifecycle plan execute {} --confirm",
        plan.plan_id
    );
    Ok(())
}
pub fn print_operation(op: &Operation, json: bool) -> Result<()> {
    if json {
        return emit(op, true);
    }
    println!("Operation {}: {}", op.operation_id, op.state);
    if op.kind == "archive" {
        println!("本次涉及 {} 个工作空间；已归档 {} 个；未归档 {} 个，其中跳过 {} 个、失败 {} 个、未完成 {} 个",op.summary.scope_total,op.summary.archived_count,op.summary.not_archived_count,op.summary.skipped_count,op.summary.failed_count,op.summary.unfinished_count);
    }
    for r in &op.results {
        println!(
            "{} [{}]: {} — {} {}",
            r.name,
            r.workspace_id,
            r.outcome,
            r.reason_codes.join(", "),
            r.message
        );
        if let Some(p) = &r.archive_path {
            println!("  archive: {}", p.display());
        }
        for repo in &r.repositories {
            if repo.dirty.dirty() || repo.dirty.unknown_ignored > 0 {
                println!(
                    "  {}: staged {}, unstaged {}, conflicts {}, untracked {}, unknown ignored {}",
                    repo.path,
                    repo.dirty.staged,
                    repo.dirty.unstaged,
                    repo.dirty.conflicts,
                    repo.dirty.untracked,
                    repo.dirty.unknown_ignored
                );
            }
        }
    }
    if op.kind == "migration" {
        println!(
            "{}: archives {}/{}, copied {}/{}, verified {}, current {:?}",
            op.progress.phase,
            op.progress.archives_done,
            op.progress.archives_total,
            op.progress.bytes_copied,
            op.progress.bytes_total,
            op.progress.bytes_verified,
            op.progress.current_file
        );
    }
    if let Some(error) = &op.error {
        println!("{error}");
    }
    Ok(())
}
fn exit_for(op: &Operation) -> Result<()> {
    let stale = op
        .results
        .iter()
        .any(|r| r.reason_codes.iter().any(|c| c == "plan_stale"));
    let code = if stale {
        4
    } else if op.summary.failed_count > 0 || op.state == "failed" || op.state == "partial_failure" {
        2
    } else if op.summary.unfinished_count > 0
        || ["paused", "cancelled", "needs_attention"].contains(&op.state.as_str())
    {
        3
    } else {
        0
    };
    if code > 0 {
        return Err(LifecycleExit {
            code,
            message: format!(
                "Operation {}: {}; inspect its complete report",
                op.operation_id, op.state
            ),
        }
        .into());
    }
    Ok(())
}
fn start_summaries(op: &Operation) {
    for r in &op.results {
        if r.outcome == "archived" && r.memory_state == "pending" {
            if let (Some(aid), Ok(exe)) = (&r.archive_id, std::env::current_exe()) {
                let _ = Command::new(exe)
                    .args(["lifecycle", "memory-worker", aid])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
    }
}
fn spawn_worker(kind: &str, id: &str) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = Command::new(exe)
            .args(["lifecycle", kind, id])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}
pub fn recover_pending_jobs() {
    let Ok(home) = crate::store::manager::get_vdw_home() else {
        return;
    };
    if let Ok(entries) = fs::read_dir(home.join("lifecycle/operations")) {
        for entry in entries.flatten() {
            if let Ok(op) = read::<Operation>(&entry.path()) {
                if op.kind == "migration" && op.state == "running" {
                    if let Ok(guard) = Guard::job(&home, &op.operation_id) {
                        drop(guard);
                        spawn_worker("migration-worker", &op.operation_id);
                    }
                }
            }
        }
    }
    if crate::store::manager::load_global_config()
        .map(|c| c.lifecycle.memory.ai.enabled)
        .unwrap_or(false)
    {
        let archives = read_or::<Vec<ArchiveRecord>>(&home.join("lifecycle/archives.json"))
            .unwrap_or_default();
        let registry = crate::store::registry::load_workspace_registry_at(&home).ok();
        for a in archives {
            if a.memory_state == "pending"
                && registry
                    .as_ref()
                    .map(|r| {
                        r.workspaces
                            .iter()
                            .any(|w| w.latest_archive_id.as_deref() == Some(a.archive_id.as_str()))
                    })
                    .unwrap_or(false)
            {
                if let Ok(g) = Guard::job(&home, &a.archive_id) {
                    drop(g);
                    spawn_worker("memory-worker", &a.archive_id);
                }
            }
        }
    }
}
fn measure(e: &Engine, plan: &mut Plan) -> Result<()> {
    for entry in &mut plan.entries {
        if !entry.reason_codes.is_empty() {
            continue;
        }
        for repo in &mut entry.repositories {
            let root = e.base().join("measure").join(id("bundle"));
            private_dir(&root)?;
            let result = (|| -> Result<u64> {
                let bare = root.join("measure.git");
                git2::Repository::init_bare(&bare)?;
                atomic_bytes(
                    &bare.join("objects/info/alternates"),
                    format!("{}\n", repo.bare.join("objects").display()).as_bytes(),
                )?;
                git(&bare, &["update-ref", "refs/measure/head", &repo.head])?;
                git(
                    &bare,
                    &["update-ref", "refs/measure/lock", &repo.locked_commit],
                )?;
                let target = root.join("snapshot.bundle");
                git(
                    &bare,
                    &[
                        "bundle",
                        "create",
                        "--quiet",
                        target.to_str().context("Non-UTF8 temporary path")?,
                        "refs/measure/head",
                        "refs/measure/lock",
                    ],
                )?;
                Ok(fs::metadata(target)?.len())
            })();
            let _ = fs::remove_dir_all(&root);
            repo.bundle_bytes = Some(result?);
        }
    }
    Ok(())
}
