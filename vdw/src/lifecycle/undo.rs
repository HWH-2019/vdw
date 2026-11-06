//! Undo is a new reviewed compensating operation; original history is never rewritten.
use super::{io::*, Engine, Operation, Plan, Policy};
use crate::services::settings::SettingsChange;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs, path::Path};

#[derive(Serialize, Deserialize)]
struct UndoRecord {
    operation_id: String,
    complete: bool,
}

pub fn status(home: &Path, operation: &Operation) -> Value {
    let result = (|| -> Result<()> {
        if operation.kind == "undo" {
            bail!("撤销结果已记录；如需继续调整，请在工作空间中操作");
        }
        let file = named_file(&home.join("lifecycle/undo"), &operation.operation_id)?;
        if file.exists() {
            let record: UndoRecord = read(&file)?;
            bail!(
                "{}：{}",
                if record.complete {
                    "已撤销"
                } else {
                    "已有撤销记录，请检查执行结果"
                },
                record.operation_id
            );
        }
        if matches!(
            operation.state.as_str(),
            "queued" | "running" | "needs_attention"
        ) || operation.results.iter().any(|r| r.needs_attention)
        {
            bail!("操作尚未完成或存在需要处理的中间状态");
        }
        let plan: Plan = read(&named_file(
            &home.join("lifecycle/plans"),
            &operation.plan_id,
        )?)?;
        if plan
            .value
            .as_ref()
            .is_some_and(|v| v["undo_parent"].is_string())
        {
            bail!("撤销步骤由所属撤销记录统一管理");
        }
        let has = |outcome: &str| operation.results.iter().any(|r| r.outcome == outcome);
        let value = plan.value.as_ref().unwrap_or(&Value::Null);
        let supported = match operation.kind.as_str() {
            "archive" => has("archived"),
            "restore" => has("restored"),
            "trash" => {
                for result in operation.results.iter().filter(|r| r.outcome == "trashed") {
                    super::trash::check_trash_restore(home, &plan, &result.workspace_id)?;
                }
                has("trashed")
            }
            "settings" => operation.state == "completed" && value["before"].is_object(),
            "config" | "migration" => {
                operation.state == "completed"
                    && (value["before_config"].is_object() || value["before_policy"].is_object())
            }
            _ => false,
        };
        if !supported {
            bail!("此操作没有可安全撤销的成功结果或完整的变更前快照");
        }
        Ok(())
    })();
    let file = named_file(&home.join("lifecycle/undo"), &operation.operation_id).ok();
    let undone = file.and_then(|p| read::<UndoRecord>(&p).ok());
    json!({"available":result.is_ok(),"reason":result.err().map(|e|format!("{e:#}")).unwrap_or_else(||"预览此操作成功部分的反向操作".into()),
        "operation_id":undone.map(|r|r.operation_id)})
}

impl Engine {
    fn validate_undo_source(&self, source: &Operation) -> Result<()> {
        let availability = status(&self.home, source);
        if availability["available"] != true {
            bail!("{}", availability["reason"].as_str().unwrap_or("无法撤销"));
        }
        let ids: Vec<_> = source
            .results
            .iter()
            .map(|r| r.workspace_id.as_str())
            .collect();
        let dir = self.base().join("operations");
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let path = entry?.path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") {
                    continue;
                }
                let other: Operation = read(&path)?;
                if other.operation_id != source.operation_id
                    && other.started_at > source.started_at
                    && other.results.iter().any(|r| {
                        ids.contains(&r.workspace_id.as_str())
                            && (matches!(r.outcome.as_str(), "archived" | "restored" | "trashed")
                                || r.needs_attention)
                    })
                {
                    bail!(
                        "工作空间存在后续操作 {}，请先处理后续记录",
                        other.operation_id
                    );
                }
            }
        }
        Ok(())
    }

    pub fn preview_undo(&mut self, operation_id: &str) -> Result<Plan> {
        let source = self.operation(operation_id)?;
        self.validate_undo_source(&source)?;
        let original = self.plan(&source.plan_id)?;
        let mut children = Vec::new();
        match source.kind.as_str() {
            "archive" => {
                for result in source.results.iter().filter(|r| r.outcome == "archived") {
                    let ws = self.find(&result.workspace_id)?;
                    if ws.lifecycle_state != "archived"
                        || ws.latest_archive_id != result.archive_id
                        || ws.path != result.path
                    {
                        bail!("工作空间已变化，不能撤销这次归档");
                    }
                    children.push(self.preview_restore(
                        result.archive_id.as_deref().context("Missing archive")?,
                        None,
                        None,
                    )?);
                }
            }
            "restore" => {
                for result in source.results.iter().filter(|r| r.outcome == "restored") {
                    let ws = self.find(&result.workspace_id)?;
                    if ws.lifecycle_state != "active"
                        || ws.path != result.path
                        || ws.latest_archive_id != result.archive_id
                    {
                        bail!("工作空间已变化，不能撤销这次恢复");
                    }
                    let (repos, materials, _) = self.snapshot(&ws)?;
                    let manifest = self.verified(&self.archive_record(
                        result.archive_id.as_deref().context("Missing archive")?,
                    )?)?;
                    if materials != manifest.materials
                        || repos.iter().any(|repo| {
                            !result.repositories.iter().any(|expected| {
                                expected.path == repo.path && expected.head == repo.head
                            })
                        })
                    {
                        bail!("恢复后文件或提交已变化，不能撤销；可手动预览归档当前内容");
                    }
                    let plan = self.preview_archive(&[ws.workspace_id], false)?;
                    if let Some(blocked) = plan.entries.iter().find(|e| !e.reason_codes.is_empty())
                    {
                        bail!(
                            "无法撤销恢复：{} ({})",
                            blocked.message,
                            blocked.reason_codes.join(", ")
                        );
                    }
                    children.push(plan);
                }
            }
            "trash" => {
                for result in source.results.iter().filter(|r| r.outcome == "trashed") {
                    children.push(self.preview_trash_restore(&original, &result.workspace_id)?);
                }
            }
            "settings" => {
                let value = original
                    .value
                    .as_ref()
                    .context("Missing settings snapshot")?;
                let now = json!({"workspace_root":self.config.workspace_root,"max_store_size_gb":self.config.max_store_size_gb,"gc_threshold_days":self.config.gc_threshold_days});
                if now != value["change"] {
                    bail!("基础配置已有后续修改，不能覆盖撤销");
                }
                let before = &value["before"];
                let change: SettingsChange = serde_json::from_value(
                    json!({"workspace_root":before["workspace_root"],"max_store_size_gb":before["max_store_size_gb"],"gc_threshold_days":before["gc_threshold_days"]}),
                )?;
                children.push(self.preview_settings(change)?);
            }
            "config" | "migration" => {
                let value = original
                    .value
                    .as_ref()
                    .context("Missing configuration snapshot")?;
                if let Some(id) = value["workspace_id"].as_str() {
                    let current =
                        serde_json::to_value(self.policies.get(id).cloned().unwrap_or_default())?;
                    if current != value["policy"] {
                        bail!("工作空间策略已有后续修改，不能覆盖撤销");
                    }
                    let before: Policy = serde_json::from_value(value["before_policy"].clone())?;
                    let key = value["key"].as_str().context("Missing policy key")?;
                    let inverse = match key {
                        "idle_days" => {
                            json!(before.idle_days.unwrap_or(self.config.lifecycle.idle_days))
                        }
                        "archive_root" => json!(before
                            .archive_root
                            .as_ref()
                            .unwrap_or(&self.config.lifecycle.archive.root)),
                        "auto_enabled" => json!(before.auto_enabled.unwrap_or(true)),
                        "retain_until" => json!(before.retain_until),
                        _ => bail!("不支持此策略的撤销"),
                    };
                    let mut plan = self.preview_config(key, inverse, Some(id))?;
                    plan.value.as_mut().unwrap()["before_policy"] = current;
                    plan.value.as_mut().unwrap()["policy"] = serde_json::to_value(before)?;
                    self.save_plan(&plan)?;
                    children.push(plan);
                } else {
                    let mut now = serde_json::to_value(&self.config.lifecycle)?;
                    let mut expected = value["next_config"].clone();
                    now["auto"]["authorization"] = Value::Null;
                    expected["auto"]["authorization"] = Value::Null;
                    if now != expected {
                        bail!("生命周期配置已有后续修改，不能覆盖撤销");
                    }
                    let mut plan =
                        self.preview_config("lifecycle", value["before_config"].clone(), None)?;
                    plan.value.as_mut().unwrap()["before_config"] =
                        serde_json::to_value(&self.config.lifecycle)?;
                    self.save_plan(&plan)?;
                    children.push(plan);
                }
            }
            _ => bail!("此操作不支持撤销"),
        }
        let mut entries = Vec::new();
        for child in &mut children {
            let mut value = child
                .value
                .take()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            value["undo_parent"] = json!(source.operation_id);
            child.value = Some(value);
            self.save_plan(child)?;
            for entry in &child.entries {
                let mut entry = entry.clone();
                entry.message = match child.kind.as_str() {
                    "restore" => "撤销归档：恢复到原工作空间。",
                    "archive" => "撤销恢复：将未变化的工作空间重新归档。",
                    "trash_restore" => "撤销删除：从系统回收站找回整个目录。",
                    _ => "恢复变更前的配置。",
                }
                .into();
                entries.push(entry);
            }
        }
        let mut plan = self.simple_plan(
            "undo",
            json!({"operation_id":source.operation_id,"original_kind":source.kind,
            "source_hash":hash_file(&named_file(&self.base().join("operations"),operation_id)?)?,
            "children":children.iter().map(|p|p.plan_id.clone()).collect::<Vec<_>>(),
            "changes":children.iter().filter_map(|p|p.value.clone()).collect::<Vec<_>>()}),
        )?;
        plan.entries = entries;
        self.save_plan(&plan)?;
        Ok(plan)
    }

    pub(crate) fn execute_undo(&mut self, plan: &Plan, op: &mut Operation) -> Result<()> {
        let value = plan.value.as_ref().context("Missing undo plan")?;
        let source_id = value["operation_id"]
            .as_str()
            .context("Missing source operation")?;
        let source = self.operation(source_id)?;
        self.validate_undo_source(&source)?;
        if value["source_hash"]
            != hash_file(&named_file(&self.base().join("operations"), source_id)?)?
        {
            bail!("PLAN_STALE: 原操作记录已变化，请重新预览");
        }
        let steps = value["children"]
            .as_array()
            .filter(|steps| !steps.is_empty())
            .context("Missing undo steps")?;
        let mut count = 0;
        for step in steps {
            let child = self.plan(step.as_str().context("Invalid undo step")?)?;
            if child.value.as_ref().and_then(|v| v["undo_parent"].as_str()) != Some(source_id)
                || !matches!(
                    child.kind.as_str(),
                    "restore" | "archive" | "trash_restore" | "config" | "settings"
                )
                || child.operation_id.is_some()
            {
                bail!("PLAN_STALE: 撤销步骤已变化，请重新预览");
            }
            count += child.entries.len();
        }
        if count != op.results.len() {
            bail!("Invalid undo scope");
        }
        let journal = named_file(&self.base().join("undo"), source_id)?;
        let mut record = UndoRecord {
            operation_id: op.operation_id.clone(),
            complete: false,
        };
        save(&journal, &record)?;
        let mut index = 0;
        for child in steps {
            let result = self.execute_plan(child.as_str().context("Invalid undo step")?)?;
            for entry in &result.results {
                op.results[index] = entry.clone();
                index += 1;
            }
            op.progress.phase = format!("undo:{}", result.kind);
            self.save_operation(op)?;
            if result.state != "completed" {
                op.state = "partial_failure".into();
                op.error = Some(
                    result
                        .error
                        .unwrap_or_else(|| "撤销步骤未全部完成，请检查逐项结果".into()),
                );
                for result in op.results.iter_mut().skip(index) {
                    result.outcome = "skipped".into();
                    result.message = "前序撤销未完成，已停止后续步骤".into();
                }
                return Ok(());
            }
        }
        record.complete = true;
        save(&journal, &record)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_fixture() -> (tempfile::TempDir, Engine, Operation) {
        let root = tempfile::tempdir().unwrap();
        let mut engine = Engine::test_at(root.path()).unwrap();
        let mut plan = engine
            .preview_settings(SettingsChange {
                workspace_root: None,
                max_store_size_gb: engine.config.max_store_size_gb + 1,
                gc_threshold_days: engine.config.gc_threshold_days,
            })
            .unwrap();
        let mut operation = engine.new_operation(&plan);
        operation.state = "completed".into();
        engine.config.max_store_size_gb += 1;
        engine.save_operation(&mut operation).unwrap();
        plan.operation_id = Some(operation.operation_id.clone());
        engine.save_plan(&plan).unwrap();
        (root, engine, operation)
    }

    #[test]
    fn settings_undo_refuses_later_configuration_and_duplicate_attempts() {
        let (_root, mut engine, source) = settings_fixture();
        assert_eq!(status(&engine.home, &source)["available"], true);
        engine.config.max_store_size_gb += 1;
        assert!(engine
            .preview_undo(&source.operation_id)
            .unwrap_err()
            .to_string()
            .contains("后续修改"));
        let journal = named_file(&engine.base().join("undo"), &source.operation_id).unwrap();
        save(
            &journal,
            &UndoRecord {
                operation_id: "op-prior-undo".into(),
                complete: false,
            },
        )
        .unwrap();
        assert_eq!(status(&engine.home, &source)["available"], false);
        assert!(status(&engine.home, &source)["reason"]
            .as_str()
            .unwrap()
            .contains("已有撤销记录"));
    }

    #[test]
    fn changed_source_after_preview_fails_without_mutation_or_undo_reservation() {
        let (_root, mut engine, mut source) = settings_fixture();
        let undo = engine.preview_undo(&source.operation_id).unwrap();
        source.progress.phase = "updated after review".into();
        engine.save_operation(&mut source).unwrap();
        let before = serde_json::to_value(&engine.config).unwrap();
        let result = engine.execute_plan(&undo.plan_id).unwrap();
        assert_eq!(result.state, "failed");
        assert!(result.error.unwrap().contains("PLAN_STALE"));
        assert_eq!(before, serde_json::to_value(&engine.config).unwrap());
        assert!(
            !named_file(&engine.base().join("undo"), &source.operation_id)
                .unwrap()
                .exists()
        );
    }

    #[test]
    fn undo_cannot_execute_a_step_from_another_operation() {
        let (_root, mut engine, source) = settings_fixture();
        let undo = engine.preview_undo(&source.operation_id).unwrap();
        let child_id = undo.value.as_ref().unwrap()["children"][0]
            .as_str()
            .unwrap();
        let mut child = engine.plan(child_id).unwrap();
        child.value.as_mut().unwrap()["undo_parent"] = json!("op-another");
        engine.save_plan(&child).unwrap();
        let before = serde_json::to_value(&engine.config).unwrap();
        let result = engine.execute_plan(&undo.plan_id).unwrap();
        assert_eq!(result.state, "failed");
        assert_eq!(before, serde_json::to_value(&engine.config).unwrap());
        assert!(
            !named_file(&engine.base().join("undo"), &source.operation_id)
                .unwrap()
                .exists()
        );
    }
}
