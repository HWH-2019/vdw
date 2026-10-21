use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LifecycleConfig {
    pub version: u32,
    pub idle_days: u32,
    pub dirty_policy: String,
    pub archive: ArchiveConfig,
    pub auto: AutoConfig,
    pub memory: MemoryConfig,
    pub cloud: CloudConfig,
}
impl Default for LifecycleConfig {
    fn default() -> Self {
        Self {
            version: 1,
            idle_days: 30,
            dirty_policy: "skip_workspace".into(),
            archive: ArchiveConfig::default(),
            auto: AutoConfig::default(),
            memory: MemoryConfig::default(),
            cloud: CloudConfig::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchiveConfig {
    pub root: String,
    pub keep_workspace_skeleton: bool,
    pub git_backup: String,
    pub materials: MaterialConfig,
    pub migration: MigrationConfig,
    pub max_bundle_bytes: Option<u64>,
}
impl Default for ArchiveConfig {
    fn default() -> Self {
        Self {
            root: "~/.vdw/archives".into(),
            keep_workspace_skeleton: true,
            git_backup: "store_refs".into(),
            materials: MaterialConfig::default(),
            migration: MigrationConfig::default(),
            max_bundle_bytes: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MaterialConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub exclude_managed_dependencies: bool,
    pub disposable_ignored: Vec<String>,
}
impl Default for MaterialConfig {
    fn default() -> Self {
        Self {
            include: vec!["**/*".into()],
            exclude: vec![
                "packages/**".into(),
                "**/node_modules/**".into(),
                "**/target/**".into(),
            ],
            exclude_managed_dependencies: true,
            disposable_ignored: vec!["node_modules".into(), "target".into(), "__pycache__".into()],
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MigrationConfig {
    pub on_root_change: String,
    pub concurrency: u32,
    pub cleanup_source_after_verify: bool,
}
impl Default for MigrationConfig {
    fn default() -> Self {
        Self {
            on_root_change: "auto".into(),
            concurrency: 1,
            cleanup_source_after_verify: true,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutoConfig {
    pub enabled: bool,
    pub scan_interval_hours: u32,
    pub grace_hours: u32,
    pub authorization: Option<String>,
}
impl Default for AutoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            scan_interval_hours: 24,
            grace_hours: 72,
            authorization: None,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MemoryConfig {
    pub index_required: bool,
    pub ai: AiConfig,
}
impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            index_required: true,
            ai: AiConfig::default(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AiConfig {
    pub enabled: bool,
    pub provider: Option<String>,
    pub trigger: String,
    pub command: Vec<String>,
    pub endpoint: Option<String>,
    pub model: Option<String>,
    pub credential_env: Option<String>,
    pub input_include: Vec<String>,
    pub input_exclude: Vec<String>,
    pub timeout_seconds: u64,
    pub max_input_chars: usize,
    pub max_output_tokens: usize,
}
impl Default for AiConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: None,
            trigger: "after_archive".into(),
            command: vec![],
            endpoint: None,
            model: None,
            credential_env: None,
            input_include: vec!["**/*.md".into(), "**/*.txt".into()],
            input_exclude: vec!["**/.env*".into(), "**/*.pem".into(), "**/*.key".into()],
            timeout_seconds: 180,
            max_input_chars: 120000,
            max_output_tokens: 4000,
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CloudConfig {
    pub enabled: bool,
    pub provider: Option<String>,
    pub content: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub idle_days: Option<u32>,
    pub auto_enabled: Option<bool>,
    pub archive_root: Option<String>,
    pub retain_until: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Protection {
    pub reason: String,
    pub added_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub workspace_id: String,
    pub client_id: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DirtyCounts {
    pub staged: usize,
    pub unstaged: usize,
    pub conflicts: usize,
    pub untracked: usize,
    pub unknown_ignored: usize,
}
impl DirtyCounts {
    pub fn dirty(&self) -> bool {
        self.staged + self.unstaged + self.conflicts + self.untracked > 0
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSnapshot {
    pub repo: String,
    pub repo_id: String,
    pub path: String,
    pub bare: PathBuf,
    pub editable: bool,
    pub branch: Option<String>,
    pub head: String,
    pub locked_commit: String,
    pub dirty: DirtyCounts,
    pub bundle: Option<String>,
    pub bundle_bytes: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Material {
    pub path: String,
    pub kind: String,
    pub bytes: u64,
    pub hash: String,
    pub link_target: Option<String>,
    pub executable: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEntry {
    pub workspace_id: String,
    pub name: String,
    pub path: PathBuf,
    pub reason_codes: Vec<String>,
    pub message: String,
    pub fingerprint: Option<String>,
    pub repositories: Vec<RepoSnapshot>,
    pub materials: Vec<Material>,
    pub archive_root: PathBuf,
    pub git_backup: String,
    pub excluded: bool,
    pub activity_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub schema_version: u32,
    pub plan_id: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub config_hash: String,
    pub entries: Vec<PlanEntry>,
    pub archive_id: Option<String>,
    pub target: Option<PathBuf>,
    pub restore_branch: Option<String>,
    pub value: Option<serde_json::Value>,
    pub superseded: bool,
    pub operation_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceResult {
    pub workspace_id: String,
    pub name: String,
    pub path: PathBuf,
    pub outcome: String,
    pub reason_codes: Vec<String>,
    pub message: String,
    pub archive_id: Option<String>,
    pub archive_path: Option<PathBuf>,
    pub phase: String,
    pub completed_steps: Vec<String>,
    pub repositories: Vec<RepoSnapshot>,
    pub memory_state: String,
    pub cloud_state: String,
    pub needs_attention: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Summary {
    pub scope_total: usize,
    pub archived_count: usize,
    pub restored_count: usize,
    pub not_archived_count: usize,
    pub skipped_count: usize,
    pub failed_count: usize,
    pub unfinished_count: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Progress {
    pub phase: String,
    pub archives_done: usize,
    pub archives_total: usize,
    pub files_done: usize,
    pub files_total: usize,
    pub bytes_copied: u64,
    pub bytes_total: u64,
    pub bytes_verified: u64,
    pub bytes_released: u64,
    pub current_archive: Option<String>,
    pub current_file: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub schema_version: u32,
    pub operation_id: String,
    pub plan_id: String,
    pub kind: String,
    pub state: String,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub summary: Summary,
    pub results: Vec<WorkspaceResult>,
    pub progress: Progress,
    pub error: Option<String>,
    pub source_root: Option<PathBuf>,
    pub target_root: Option<PathBuf>,
    pub archive_ids: Vec<String>,
    pub completed_archive_ids: Vec<String>,
    pub attempts: u32,
    #[serde(default)]
    pub attempt_history: Vec<serde_json::Value>,
}
impl Operation {
    pub fn recount(&mut self) {
        let mut s = Summary {
            scope_total: self.results.len(),
            ..Default::default()
        };
        for r in &self.results {
            match r.outcome.as_str() {
                "archived" => s.archived_count += 1,
                "restored" => s.restored_count += 1,
                "skipped" => s.skipped_count += 1,
                "failed" => s.failed_count += 1,
                _ => s.unfinished_count += 1,
            }
        }
        s.not_archived_count = s.skipped_count + s.failed_count + s.unfinished_count;
        self.summary = s;
        self.updated_at = Utc::now();
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveRecord {
    pub archive_id: String,
    pub workspace_id: String,
    pub name: String,
    pub original_path: PathBuf,
    pub location: PathBuf,
    pub created_at: DateTime<Utc>,
    pub manifest_hash: String,
    pub storage_policy: String,
    pub memory_state: String,
    pub cloud_state: String,
    #[serde(default)]
    pub generations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub archive_id: String,
    pub workspace_id: String,
    pub name: String,
    pub original_path: PathBuf,
    pub created_at: DateTime<Utc>,
    pub reason: String,
    pub repositories: Vec<RepoSnapshot>,
    pub materials: Vec<Material>,
    pub links: crate::types::workspace::WorkspaceState,
    pub files: BTreeMap<String, String>,
}

/// Reserved contracts: connectors are intentionally not an execution or authorization API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveTrigger {
    pub event_id: String,
    pub source: String,
    pub workspace_id: String,
    pub reason: String,
    pub occurred_at: DateTime<Utc>,
    pub requested_action: String,
    pub policy_version: String,
    pub not_before: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub provider: String,
    pub implemented: bool,
    pub manifest: bool,
    pub memory: bool,
    pub blobs: bool,
    pub download: bool,
}
