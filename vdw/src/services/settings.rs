use crate::{
    lifecycle::io::hash,
    lifecycle::{Engine, Operation, Plan},
    store::manager,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsChange {
    pub workspace_root: Option<String>,
    pub max_store_size_gb: u64,
    pub gc_threshold_days: u32,
}

pub fn version(config: &manager::GlobalConfig) -> Result<String> {
    Ok(hash(&serde_json::to_vec(config)?))
}

impl Engine {
    pub fn preview_settings(&self, change: SettingsChange) -> Result<Plan> {
        self.validate_settings(&change)?;
        self.simple_plan("settings", serde_json::json!({
            "base_version": version(&self.config)?, "before": self.config,
            "change": change, "note": "Workspace root affects new workspaces only; existing paths are preserved."
        }))
    }

    fn validate_settings(&self, change: &SettingsChange) -> Result<()> {
        if change.max_store_size_gb == 0 || change.gc_threshold_days == 0 {
            bail!("Store size and retention must be positive");
        }
        if let Some(root) = &change.workspace_root {
            let home = self.home.parent().context("Missing home directory")?;
            let resolved = manager::resolve_workspace_root(root, home)?;
            if !resolved.is_dir() {
                bail!("Workspace root must be an existing directory");
            }
            if resolved.starts_with(&self.store) || self.store.starts_with(&resolved) {
                bail!("Workspace root overlaps the Store");
            }
        }
        Ok(())
    }

    pub(crate) fn apply_settings(&mut self, plan: &Plan, _op: &mut Operation) -> Result<()> {
        let value = plan.value.as_ref().context("Missing settings plan")?;
        if value["base_version"].as_str() != Some(version(&self.config)?.as_str()) {
            bail!("PLAN_STALE: global configuration changed; preview again");
        }
        let change: SettingsChange = serde_json::from_value(value["change"].clone())?;
        self.validate_settings(&change)?;
        self.config.workspace_root = change.workspace_root;
        self.config.max_store_size_gb = change.max_store_size_gb;
        self.config.gc_threshold_days = change.gc_threshold_days;
        manager::save_global_config(&self.config)
    }
}
