//! A reviewed, whole-directory move to the OS trash. Never fall back to unlinking.
use super::{io::*, Engine, Operation, Plan, PlanEntry};
use crate::store::registry::{self, WorkspaceRegistration, WorkspaceRegistry};
use anyhow::{bail, Context, Result};
use serde_json::json;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(serde::Serialize, serde::Deserialize)]
struct TrashRecord {
    workspace: WorkspaceRegistration,
    root_identity: String,
    references: Option<Vec<String>>,
    #[serde(default)]
    receipt: Option<TrashReceipt>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TrashReceipt {
    Path { path: PathBuf },
    Item { id: String },
}

fn store_references(root: &Path) -> Result<Vec<String>> {
    let mut config = crate::config::parser::load_config(root)?;
    crate::config::env_resolver::resolve_env_vars(&mut config)?;
    let mut references: std::collections::BTreeSet<_> = config
        .dependencies
        .iter()
        .map(|d| d.dep_id().to_owned())
        .collect();
    if let Some(lock) = crate::lockfile::WorkspaceLock::load(root)? {
        references.extend(lock.dependencies.into_iter().map(|d| d.repo));
    }
    Ok(references.into_iter().collect())
}

/// Store GC keeps references even though the original workspace directory is now absent.
pub(crate) fn retained_references(home: &Path, ws: &WorkspaceRegistration) -> Result<Vec<String>> {
    let record: TrashRecord = read(&named_file(
        &home.join("lifecycle/trash"),
        &ws.workspace_id,
    )?)?;
    if record.workspace.workspace_id != ws.workspace_id || record.workspace.path != ws.path {
        bail!("Trash retention record does not match workspace");
    }
    record
        .references
        .context("Trashed workspace reference inventory is unavailable")
}

/// Explicit CLI activity can re-register a directory put back from the OS trash.
pub(crate) fn restored_registration(
    home: &Path,
    ws: &WorkspaceRegistration,
) -> Result<Option<WorkspaceRegistration>> {
    let record: TrashRecord = read(&named_file(
        &home.join("lifecycle/trash"),
        &ws.workspace_id,
    )?)?;
    if record.workspace.workspace_id == ws.workspace_id
        && record.workspace.path == ws.path
        && ws.path.is_dir()
        && root_identity(&ws.path)? == record.root_identity
    {
        Ok(Some(record.workspace))
    } else {
        Ok(None)
    }
}

fn root_identity(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path).context("工作空间目录不存在或无法访问")?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!("工作空间路径必须是实际目录，不能是符号链接");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    Ok(format!("{:?}", metadata.created()?))
}

impl Engine {
    fn trash_target(&self, id: &str) -> Result<WorkspaceRegistration> {
        let ws = self
            .registry
            .workspaces
            .iter()
            .find(|w| w.workspace_id == id)
            .cloned()
            .context("工作空间未登记")?;
        if !matches!(ws.lifecycle_state.as_str(), "active" | "archived")
            || ws.current_operation_id.is_some()
        {
            bail!("工作空间正在执行操作，或已经移到回收站");
        }
        if self.protection(id) || self.active_lease(id) {
            bail!("工作空间受保护或编辑器租约仍有效，请先解除保护或结束使用");
        }
        root_identity(&ws.path)?;
        let root = fs::canonicalize(&ws.path)?;
        if root != ws.path
            || root.parent().is_none()
            || self.home.starts_with(&root)
            || self.store.starts_with(&root)
            || root.starts_with(&self.store)
            || std::env::var_os("HOME")
                .map(|h| root == PathBuf::from(h))
                .unwrap_or(false)
            || self.registry.workspaces.iter().any(|other| {
                other.workspace_id != id
                    && other.lifecycle_state != "trashed"
                    && (root.starts_with(&other.path) || other.path.starts_with(&root))
            })
            || self
                .archives
                .iter()
                .any(|a| a.location.starts_with(&root) || root.starts_with(&a.location))
        {
            bail!("拒绝移动与 Store、控制目录、历史归档或其他工作空间重叠的路径");
        }
        Ok(ws)
    }

    pub fn preview_trash(&self, id: &str) -> Result<Plan> {
        let ws = self.trash_target(id)?;
        let mut plan = Plan {
            schema_version: 1,
            plan_id: super::io::id("plan"),
            kind: "trash".into(),
            created_at: chrono::Utc::now(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(30),
            config_hash: self.config_hash()?,
            entries: vec![],
            archive_id: None,
            target: None,
            restore_branch: None,
            value: Some(json!({"workspace":ws,"root_identity":root_identity(&ws.path)?})),
            superseded: false,
            operation_id: None,
        };
        plan.entries.push(PlanEntry {
            workspace_id: ws.workspace_id, name: ws.name, path: ws.path,
            reason_codes: vec![], message: "整个工作空间目录将移到系统回收站，包含未提交、未跟踪及忽略的文件。共享 Store、远端仓库和历史归档保留。".into(),
            fingerprint: None, repositories: vec![], materials: vec![], archive_root: PathBuf::new(),
            git_backup: String::new(), excluded: false, activity_revision: ws.activity_revision,
        });
        self.save_plan(&plan)?;
        Ok(plan)
    }

    pub(crate) fn trash_workspace(&mut self, plan: &Plan, op: &mut Operation) -> Result<()> {
        let mut receipt = None;
        self.trash_with(plan, op, |root| {
            receipt = recycle(root).context("无法移到系统回收站；未执行永久删除")?;
            Ok(())
        })?;
        let file = named_file(&self.base().join("trash"), &op.results[0].workspace_id)?;
        let mut record: TrashRecord = read(&file)?;
        record.receipt = receipt;
        save(&file, &record)
    }

    fn trash_with(
        &mut self,
        plan: &Plan,
        op: &mut Operation,
        move_to_trash: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<()> {
        let value = plan.value.as_ref().context("Missing trash plan")?;
        let previous: WorkspaceRegistration = serde_json::from_value(value["workspace"].clone())?;
        let ws = self.trash_target(&previous.workspace_id)?;
        if plan.entries.len() != 1
            || op.results.len() != 1
            || plan.entries[0].workspace_id != ws.workspace_id
            || plan.entries[0].path != ws.path
        {
            bail!("Invalid trash plan target");
        }
        if ws != previous || value["root_identity"] != root_identity(&ws.path)? {
            bail!("PLAN_STALE: 工作空间发生变化，请重新确认");
        }
        save(
            &named_file(&self.base().join("trash"), &ws.workspace_id)?,
            &TrashRecord {
                workspace: ws.clone(),
                root_identity: root_identity(&ws.path)?,
                references: store_references(&ws.path).ok(),
                receipt: None,
            },
        )?;
        let reason = format!("VDW trash retention: {}", ws.workspace_id);
        let locks = retain_worktrees(&ws.path, &self.store, &reason)?;
        let index = self
            .registry
            .workspaces
            .iter()
            .position(|w| w.workspace_id == ws.workspace_id)
            .unwrap();
        self.registry.workspaces[index].lifecycle_state = "trashing".into();
        self.registry.workspaces[index].current_operation_id = Some(op.operation_id.clone());
        if let Err(error) = self.persist_registry() {
            self.registry.workspaces[index] = previous;
            release_locks(&locks, &reason);
            return Err(error);
        }
        op.results[0].phase = "moving_to_trash".into();
        op.results[0]
            .completed_steps
            .push("worktree_retention_locked".into());
        if let Err(error) = self.save_operation(op) {
            self.registry.workspaces[index] = previous;
            self.persist_registry()?;
            release_locks(&locks, &reason);
            return Err(error);
        }
        if let Err(error) = move_to_trash(&ws.path) {
            // If the source vanished despite an OS error, keep the transition for manual review.
            if ws.path.is_dir() {
                self.registry.workspaces[index] = previous;
                self.persist_registry()?;
                release_locks(&locks, &reason);
                op.results[0].completed_steps.clear();
            }
            return Err(error);
        }
        if ws.path.try_exists()? {
            bail!("回收站操作未移走源目录，请检查系统回收站状态");
        }
        op.results[0]
            .completed_steps
            .push("moved_to_system_trash".into());
        self.registry.workspaces[index].lifecycle_state = "trashed".into();
        self.registry.workspaces[index].current_operation_id = None;
        self.persist_registry()?;
        op.results[0].outcome = "trashed".into();
        op.results[0].phase = "completed".into();
        op.results[0].message = "整个工作空间已移到系统回收站，可从系统回收站找回目录。共享 Store、远端仓库和历史归档保留。".into();
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn recycle(root: &Path) -> Result<Option<TrashReceipt>> {
    use objc2_foundation::{NSFileManager, NSString, NSURL};
    let url = NSURL::fileURLWithPath(&NSString::from_str(
        root.to_str().context("Invalid workspace path")?,
    ));
    let mut resulting = None;
    NSFileManager::defaultManager()
        .trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting))
        .map_err(|error| anyhow::anyhow!("System Trash: {error}"))?;
    Ok(resulting
        .and_then(|url| url.path())
        .map(|path| TrashReceipt::Path {
            path: PathBuf::from(path.to_string()),
        }))
}

#[cfg(not(target_os = "macos"))]
fn recycle(root: &Path) -> Result<Option<TrashReceipt>> {
    let before = trash::os_limited::list().ok();
    system_trash(root)?;
    let mut found = trash::os_limited::list()
        .unwrap_or_default()
        .into_iter()
        .filter(|item| {
            item.original_path() == root
                && before
                    .as_ref()
                    .map(|items| !items.contains(item))
                    .unwrap_or(true)
        });
    let item = found.next();
    if found.next().is_some() {
        return Ok(None);
    }
    Ok(item.map(|item| TrashReceipt::Item {
        id: item.id.to_string_lossy().into_owned(),
    }))
}

impl Engine {
    pub(crate) fn preview_trash_restore(
        &self,
        original: &Plan,
        workspace_id: &str,
    ) -> Result<Plan> {
        let (ws, receipt, parent) =
            trash_restore_target(&self.home, &self.registry, original, workspace_id)?;
        let mut plan = self.simple_plan("trash_restore", json!({"workspace_id":workspace_id,"record_hash":hash_file(&named_file(&self.base().join("trash"),workspace_id)?)?,"receipt":receipt,"parent":parent}))?;
        plan.entries = original.entries.clone();
        for entry in &mut plan.entries {
            entry.message = "从系统回收站找回整个目录，恢复原登记，不覆盖现有路径。".into();
            entry.path = ws.path.clone();
        }
        self.save_plan(&plan)?;
        Ok(plan)
    }

    pub(crate) fn restore_trash(&mut self, plan: &Plan, op: &mut Operation) -> Result<()> {
        let value = plan.value.as_ref().context("Missing trash restore plan")?;
        let id = value["workspace_id"]
            .as_str()
            .context("Missing workspace")?;
        let ws = self.find(id)?;
        let record_file = named_file(&self.base().join("trash"), id)?;
        let record: TrashRecord = read(&record_file)?;
        if ws.lifecycle_state != "trashed"
            || ws.current_operation_id.is_some()
            || fs::symlink_metadata(&ws.path).is_ok()
            || self.registry.workspaces.iter().any(|other| {
                other.workspace_id != id
                    && other.lifecycle_state != "trashed"
                    && (other.path.starts_with(&ws.path) || ws.path.starts_with(&other.path))
            })
            || value["record_hash"] != hash_file(&record_file)?
            || PathBuf::from(value["parent"].as_str().context("Missing parent")?)
                != fs::canonicalize(ws.path.parent().context("Missing parent")?)?
        {
            bail!("PLAN_STALE: 回收记录或原路径已变化，请重新预览");
        }
        let receipt: TrashReceipt = serde_json::from_value(value["receipt"].clone())?;
        validate_receipt(&record, &receipt)?;
        op.results[0].phase = "restoring_from_trash".into();
        self.save_operation(op)?;
        restore_receipt(&receipt, &ws.path)?;
        op.results[0]
            .completed_steps
            .push("restored_from_system_trash".into());
        if root_identity(&ws.path)? != record.root_identity {
            bail!("目录已找回，但身份验证失败，请人工检查");
        }
        let restored = record.workspace;
        *self
            .registry
            .workspaces
            .iter_mut()
            .find(|w| w.workspace_id == id)
            .unwrap() = restored;
        self.persist_registry()?;
        release_retention(&self.store, id)?;
        op.results[0].outcome = "restored".into();
        op.results[0].phase = "completed".into();
        op.results[0].message = "已从系统回收站找回整个目录并恢复原登记。".into();
        Ok(())
    }
}

/// History reads use the same read-only checks as preview, without opening an Engine or writing plans.
pub(crate) fn check_trash_restore(home: &Path, original: &Plan, workspace_id: &str) -> Result<()> {
    let registry = registry::load_workspace_registry_at(home)?;
    trash_restore_target(home, &registry, original, workspace_id).map(|_| ())
}

fn trash_restore_target(
    home: &Path,
    registry: &WorkspaceRegistry,
    original: &Plan,
    workspace_id: &str,
) -> Result<(WorkspaceRegistration, TrashReceipt, PathBuf)> {
    let ws = registry
        .workspaces
        .iter()
        .find(|ws| ws.workspace_id == workspace_id)
        .context("工作空间未登记")?
        .clone();
    if ws.lifecycle_state != "trashed" || ws.current_operation_id.is_some() {
        bail!("工作空间已发生后续操作，不能撤销这次删除");
    }
    let record: TrashRecord = read(&named_file(&home.join("lifecycle/trash"), workspace_id)?)?;
    if record.workspace.workspace_id != ws.workspace_id
        || record.workspace.path != ws.path
        || original.value.as_ref().and_then(|v| v.get("workspace"))
            != Some(&serde_json::to_value(&record.workspace)?)
    {
        bail!("回收记录已被后续删除替换，不能撤销旧操作");
    }
    if fs::symlink_metadata(&ws.path).is_ok() {
        bail!("原路径已被占用，撤销不会覆盖现有文件");
    }
    let receipt = locate_trash(&record)?;
    let parent = fs::canonicalize(ws.path.parent().context("Missing parent")?)?;
    if registry.workspaces.iter().any(|other| {
        other.workspace_id != workspace_id
            && other.lifecycle_state != "trashed"
            && (other.path.starts_with(&ws.path) || ws.path.starts_with(&other.path))
    }) {
        bail!("原路径已登记了其他工作空间");
    }
    Ok((ws, receipt, parent))
}

fn release_retention(store: &Path, id: &str) -> Result<()> {
    let objects = store.join("objects");
    if !objects.is_dir() {
        return Ok(());
    }
    let reason = format!("VDW trash retention: {id}");
    for entry in walkdir::WalkDir::new(objects)
        .follow_links(false)
        .min_depth(5)
        .max_depth(5)
    {
        let entry = entry?;
        if entry.file_name() == "locked"
            && entry.file_type().is_file()
            && fs::read_to_string(entry.path())? == reason
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn locate_trash(record: &TrashRecord) -> Result<TrashReceipt> {
    if let Some(receipt) = &record.receipt {
        validate_receipt(record, receipt)?;
        return Ok(receipt.clone());
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").context("Missing home")?;
        let trash = PathBuf::from(home).join(".Trash");
        return locate_legacy_trash(record, &trash, || fs::read_dir(&trash));
    }
    #[cfg(not(target_os = "macos"))]
    bail!("回收站中的原目录不存在或旧记录缺少定位信息，请手动检查回收站")
}

#[cfg(any(test, target_os = "macos"))]
fn locate_legacy_trash(
    record: &TrashRecord,
    trash: &Path,
    list: impl FnOnce() -> std::io::Result<fs::ReadDir>,
) -> Result<TrashReceipt> {
    // Exact access can be allowed while enumeration is denied. Never trust a name without identity.
    let path = trash.join(
        record
            .workspace
            .path
            .file_name()
            .context("工作空间目录名无效")?,
    );
    if root_identity(&path).ok().as_ref() == Some(&record.root_identity) {
        return Ok(TrashReceipt::Path { path });
    }
    for entry in list().context(
        "旧记录未保存回收位置，无法读取回收站列表；请将原目录从系统回收站放回原路径后再恢复登记",
    )? {
        let path = entry?.path();
        if root_identity(&path).ok().as_ref() == Some(&record.root_identity) {
            return Ok(TrashReceipt::Path { path });
        }
    }
    bail!("回收站中未找到身份匹配的原目录，请检查目录是否已改名、移出或清空")
}

fn validate_receipt(record: &TrashRecord, receipt: &TrashReceipt) -> Result<()> {
    match receipt {
        TrashReceipt::Path { path } => {
            if root_identity(path).context("回收站中的原目录不存在或无法访问")?
                != record.root_identity
                || path == &record.workspace.path
            {
                bail!("回收站目录身份已变化");
            }
        }
        TrashReceipt::Item { id } => {
            #[cfg(not(target_os = "macos"))]
            if trash::os_limited::list()?.iter().any(|item| {
                item.id.to_string_lossy() == id.as_str()
                    && item.original_path() == record.workspace.path
            }) {
                return Ok(());
            }
            let _ = id;
            bail!("回收站条目不存在");
        }
    }
    Ok(())
}

fn restore_receipt(receipt: &TrashReceipt, target: &Path) -> Result<()> {
    match receipt {
        TrashReceipt::Path { path } => {
            #[cfg(target_os = "macos")]
            {
                use objc2_foundation::{NSFileManager, NSString, NSURL};
                let source = NSURL::fileURLWithPath(&NSString::from_str(
                    path.to_str().context("Invalid path")?,
                ));
                let target = NSURL::fileURLWithPath(&NSString::from_str(
                    target.to_str().context("Invalid target")?,
                ));
                NSFileManager::defaultManager()
                    .moveItemAtURL_toURL_error(&source, &target)
                    .map_err(|e| anyhow::anyhow!("Trash restore: {e}"))?;
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = path;
                let _ = target;
                bail!("不支持此回收站记录格式");
            }
        }
        TrashReceipt::Item { id } => {
            #[cfg(not(target_os = "macos"))]
            {
                let item = trash::os_limited::list()?
                    .into_iter()
                    .find(|item| {
                        item.id.to_string_lossy() == id.as_str() && item.original_path() == target
                    })
                    .context("Trash item not found")?;
                trash::os_limited::restore_all([item])?;
            }
            #[cfg(target_os = "macos")]
            {
                let _ = id;
                bail!("不支持此回收站记录格式");
            }
        }
    }
    Ok(())
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn system_trash(root: &Path) -> Result<()> {
    let context = trash::TrashContext::new();
    context.delete(root)?;
    Ok(())
}

#[cfg(windows)]
fn system_trash(root: &Path) -> Result<()> {
    use windows::{
        core::PCWSTR,
        Win32::{System::Com::*, UI::Shell::*},
    };
    // RECYCLEONDELETE forbids the Shell's permanent-deletion fallback on unsupported volumes.
    let path = root.to_str().context("回收站路径包含不受支持的字符")?;
    let ordinary = if let Some(unc) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_owned()
    };
    let wide: Vec<u16> = ordinary.encode_utf16().chain(Some(0)).collect();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _guard = ComGuard;
        let operation: IFileOperation =
            CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)?;
        operation.SetOperationFlags(
            FOF_NO_UI | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE | FOFX_ADDUNDORECORD,
        )?;
        let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)?;
        operation.DeleteItem(&item, None)?;
        operation.PerformOperations()?;
        if operation.GetAnyOperationsAborted()?.as_bool() {
            bail!("系统回收站操作已中止");
        }
    }
    Ok(())
}

fn retain_worktrees(root: &Path, store: &Path, reason: &str) -> Result<Vec<PathBuf>> {
    let mut created = Vec::new();
    let result = (|| -> Result<()> {
        let objects = store.join("objects");
        if !objects.is_dir() {
            return Ok(());
        }
        // Store/objects/<repo>/bare.git/worktrees/<name>/gitdir is the native backlink.
        // Inspect this small inventory instead of traversing all workspace build outputs.
        for entry in walkdir::WalkDir::new(objects)
            .follow_links(false)
            .min_depth(5)
            .max_depth(5)
        {
            let entry = entry?;
            if entry.file_name() != "gitdir" || !entry.file_type().is_file() {
                continue;
            }
            let admin = entry.path().parent().unwrap();
            if admin.parent().and_then(Path::file_name) != Some(std::ffi::OsStr::new("worktrees")) {
                continue;
            }
            let backlink = fs::read_to_string(entry.path())?;
            if !Path::new(backlink.trim()).starts_with(root) {
                continue;
            }
            let lock = admin.join("locked");
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
            {
                Ok(mut file) => {
                    created.push(lock);
                    file.write_all(reason.as_bytes())?;
                    file.sync_all()?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        release_locks(&created, reason);
        return Err(error);
    }
    Ok(created)
}

fn release_locks(paths: &[PathBuf], reason: &str) {
    for path in paths {
        if fs::read_to_string(path).ok().as_deref() == Some(reason) {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn fixture() -> (tempfile::TempDir, Engine, String) {
        let temp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(temp.path()).unwrap();
        let mut engine = Engine::test_at(&base).unwrap();
        let root = base.join("workspace");
        fs::create_dir_all(root.join(".vdw")).unwrap();
        fs::write(root.join("notes.txt"), "local notes").unwrap();
        fs::write(root.join(".env"), "ignored but preserved").unwrap();
        let ws = WorkspaceRegistration::new(root.clone(), chrono::Utc::now());
        let id = ws.workspace_id.clone();
        fs::write(
            root.join(".vdw/identity.json"),
            json!({"workspace_id":id}).to_string(),
        )
        .unwrap();
        engine.registry.workspaces.push(ws);
        engine.persist_registry().unwrap();
        (temp, engine, id)
    }

    fn add_worktree_backlink(engine: &Engine, id: &str) -> PathBuf {
        let admin = engine
            .store
            .join("objects/sample/bare.git/worktrees/sample");
        fs::create_dir_all(&admin).unwrap();
        fs::write(
            admin.join("gitdir"),
            engine
                .find(id)
                .unwrap()
                .path
                .join("packages/sample/.git")
                .to_string_lossy()
                .as_bytes(),
        )
        .unwrap();
        admin
    }

    #[test]
    fn legacy_trash_uses_exact_identity_without_needing_directory_enumeration() {
        let (_temp, mut engine, id) = fixture();
        let source = engine.find(&id).unwrap().path;
        let trash = source.parent().unwrap().join("trash");
        fs::create_dir(&trash).unwrap();
        let recycled = trash.join(source.file_name().unwrap());
        let plan = engine.preview_trash(&id).unwrap();
        let mut operation = engine.new_operation(&plan);
        engine
            .trash_with(&plan, &mut operation, |root| {
                Ok(fs::rename(root, &recycled)?)
            })
            .unwrap();
        let record: TrashRecord =
            read(&named_file(&engine.base().join("trash"), &id).unwrap()).unwrap();
        assert!(record.receipt.is_none());
        let before = fs::read(engine.home.join("workspaces.json")).unwrap();
        let receipt = locate_legacy_trash(&record, &trash, || {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap();
        assert!(matches!(receipt,TrashReceipt::Path{path} if path==recycled));
        assert_eq!(
            before,
            fs::read(engine.home.join("workspaces.json")).unwrap()
        );
        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(recycled.join("notes.txt")).unwrap(),
            "local notes"
        );
    }

    #[test]
    fn legacy_trash_never_selects_a_different_directory_with_the_same_name() {
        let (_temp, mut engine, id) = fixture();
        let source = engine.find(&id).unwrap().path;
        let trash = source.parent().unwrap().join("trash");
        fs::create_dir(&trash).unwrap();
        let recycled = trash.join("renamed-original");
        let plan = engine.preview_trash(&id).unwrap();
        let mut operation = engine.new_operation(&plan);
        engine
            .trash_with(&plan, &mut operation, |root| {
                Ok(fs::rename(root, &recycled)?)
            })
            .unwrap();
        let record: TrashRecord =
            read(&named_file(&engine.base().join("trash"), &id).unwrap()).unwrap();
        let replacement = trash.join(source.file_name().unwrap());
        fs::create_dir(&replacement).unwrap();
        fs::write(replacement.join("notes.txt"), "another directory").unwrap();
        let denied = locate_legacy_trash(&record, &trash, || {
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert!(denied.to_string().contains("无法读取回收站列表"));
        let receipt = locate_legacy_trash(&record, &trash, || fs::read_dir(&trash)).unwrap();
        assert!(matches!(receipt,TrashReceipt::Path{path} if path==recycled));
        assert_eq!(
            fs::read_to_string(replacement.join("notes.txt")).unwrap(),
            "another directory"
        );
    }

    #[test]
    fn history_checks_trash_availability_without_writing_plans_or_moving_files() {
        let (_temp, mut engine, id) = fixture();
        let source = engine.find(&id).unwrap().path;
        let recycled = source.parent().unwrap().join("recycled");
        let plan = engine.preview_trash(&id).unwrap();
        let mut operation = engine.new_operation(&plan);
        engine
            .trash_with(&plan, &mut operation, |root| {
                Ok(fs::rename(root, &recycled)?)
            })
            .unwrap();
        Engine::finish(&mut operation);
        engine.save_operation(&mut operation).unwrap();
        let record_path = named_file(&engine.base().join("trash"), &id).unwrap();
        let mut record: TrashRecord = read(&record_path).unwrap();
        record.receipt = Some(TrashReceipt::Path {
            path: recycled.clone(),
        });
        save(&record_path, &record).unwrap();
        let before = fs::read(engine.home.join("workspaces.json")).unwrap();
        let plans = fs::read_dir(engine.base().join("plans")).unwrap().count();
        assert_eq!(
            super::super::undo::status(&engine.home, &operation)["available"],
            true
        );
        fs::create_dir(&source).unwrap();
        let occupied = super::super::undo::status(&engine.home, &operation);
        assert_eq!(occupied["available"], false);
        assert!(occupied["reason"]
            .as_str()
            .unwrap()
            .contains("原路径已被占用"));
        fs::remove_dir(&source).unwrap();
        fs::rename(&recycled, source.parent().unwrap().join("relocated")).unwrap();
        let missing = super::super::undo::status(&engine.home, &operation);
        assert_eq!(missing["available"], false);
        assert!(missing["reason"]
            .as_str()
            .unwrap()
            .contains("回收站中的原目录"));
        assert_eq!(
            before,
            fs::read(engine.home.join("workspaces.json")).unwrap()
        );
        assert_eq!(
            plans,
            fs::read_dir(engine.base().join("plans")).unwrap().count()
        );
    }

    #[test]
    fn whole_directory_move_preserves_local_files_and_locks_shared_git_metadata() {
        let (_temp, mut engine, id) = fixture();
        let source = engine.find(&id).unwrap().path;
        let admin = add_worktree_backlink(&engine, &id);
        let target = source.parent().unwrap().join("recycle-bin");
        let before = fs::read(engine.home.join("workspaces.json")).unwrap();
        let plan = engine.preview_trash(&id).unwrap();
        assert!(source.is_dir());
        assert_eq!(
            before,
            fs::read(engine.home.join("workspaces.json")).unwrap()
        );
        let mut op = engine.new_operation(&plan);
        engine
            .trash_with(&plan, &mut op, |path| Ok(fs::rename(path, &target)?))
            .unwrap();
        Engine::finish(&mut op);
        assert!(!source.exists());
        assert_eq!(
            fs::read_to_string(target.join("notes.txt")).unwrap(),
            "local notes"
        );
        assert_eq!(
            fs::read_to_string(target.join(".env")).unwrap(),
            "ignored but preserved"
        );
        assert!(admin.join("locked").is_file());
        assert_eq!(engine.find(&id).unwrap().lifecycle_state, "trashed");
        assert_eq!(op.state, "completed");
        assert_eq!(op.summary.trashed_count, 1);
    }

    #[test]
    fn failed_os_move_keeps_directory_registration_and_existing_locks() {
        let (_temp, mut engine, id) = fixture();
        let admin = add_worktree_backlink(&engine, &id);
        let plan = engine.preview_trash(&id).unwrap();
        let mut op = engine.new_operation(&plan);
        let previous = engine.find(&id).unwrap();
        assert!(engine
            .trash_with(&plan, &mut op, |_| bail!("OS unavailable"))
            .is_err());
        assert_eq!(engine.find(&id).unwrap(), previous);
        assert!(previous.path.join("notes.txt").is_file());
        assert!(!admin.join("locked").exists());
        fs::write(admin.join("locked"), "user lock").unwrap();
        assert!(engine
            .trash_with(&plan, &mut op, |_| bail!("OS unavailable"))
            .is_err());
        assert_eq!(
            fs::read_to_string(admin.join("locked")).unwrap(),
            "user lock"
        );
    }

    #[test]
    fn stale_or_busy_workspace_never_calls_the_os_mover() {
        let (_temp, mut engine, id) = fixture();
        let plan = engine.preview_trash(&id).unwrap();
        let mut op = engine.new_operation(&plan);
        let invoked = AtomicBool::new(false);
        engine.registry.workspaces[0].activity_revision += 1;
        assert!(engine
            .trash_with(&plan, &mut op, |_| {
                invoked.store(true, Ordering::Relaxed);
                Ok(())
            })
            .is_err());
        assert!(!invoked.load(Ordering::Relaxed));
        engine.registry.workspaces[0].current_operation_id = Some("running-operation".into());
        assert!(engine.preview_trash(&id).is_err());
    }

    #[test]
    fn putting_back_the_same_directory_can_explicitly_reactivate_its_registration() {
        let (_temp, mut engine, id) = fixture();
        let source = engine.find(&id).unwrap().path;
        let target = source.parent().unwrap().join("recycled");
        let plan = engine.preview_trash(&id).unwrap();
        let mut op = engine.new_operation(&plan);
        engine
            .trash_with(&plan, &mut op, |root| Ok(fs::rename(root, &target)?))
            .unwrap();
        assert!(engine.touch(&id, "restore_test").is_err());
        fs::rename(&target, &source).unwrap();
        let restored = engine.touch(&id, "restore_test").unwrap();
        assert_eq!(restored.workspace_id, id);
        assert_eq!(restored.lifecycle_state, "active");
    }

    #[test]
    fn protected_leased_and_nested_workspaces_cannot_be_moved() {
        let (_temp, mut engine, id) = fixture();
        engine.protections.insert(
            id.clone(),
            super::super::Protection {
                reason: "retain".into(),
                added_at: chrono::Utc::now(),
                expires_at: None,
            },
        );
        assert!(engine.preview_trash(&id).is_err());
        engine.protections.clear();
        engine.leases.push(super::super::Lease {
            workspace_id: id.clone(),
            client_id: "IDE".into(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        });
        assert!(engine.preview_trash(&id).is_err());
        engine.leases.clear();
        let nested = engine.find(&id).unwrap().path.join("child");
        fs::create_dir(&nested).unwrap();
        engine
            .registry
            .workspaces
            .push(WorkspaceRegistration::new(nested, chrono::Utc::now()));
        assert!(engine.preview_trash(&id).is_err());
    }
    #[test]
    fn replacement_directory_is_not_the_reviewed_target() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("workspace");
        fs::create_dir(&root).unwrap();
        let first = root_identity(&root).unwrap();
        fs::rename(&root, temp.path().join("previous")).unwrap();
        fs::create_dir(&root).unwrap();
        assert_ne!(first, root_identity(&root).unwrap());
    }
    #[cfg(unix)]
    #[test]
    fn symlink_root_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let link = temp.path().join("alias");
        std::os::unix::fs::symlink(temp.path(), &link).unwrap();
        assert!(root_identity(&link).is_err());
    }
}
