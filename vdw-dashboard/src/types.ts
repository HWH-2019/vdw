export interface ArchiveNotice { code: string; path: string; message: string }
export interface Repository {
  id: string; name: string; remote: string; web_url: string | null; path: string; editable: boolean;
  development_branch: string | null; branch_web_url: string | null;
  declared_revision: string | null; branch: string | null; head: string | null; locked_commit: string | null;
  topology: string | null; topology_message?: string; git_error?: string;
  remote_state: string;
  archive_blockers?: ArchiveNotice[];
  archive_warnings?: ArchiveNotice[];
  git: null | { branch: string | null; head: string; change_count: number; staged: number; unstaged: number;
    untracked: number; conflicts: number; ahead: number | null; behind: number | null; branch_drift: boolean };
  scope?: { include: string[]; exclude: string[] };
}
export interface Workspace {
  last_modified_at: string | null; modified_error: string | null;
  status: { state: 'active' | 'pending_archive' | 'archived'; archive_at: string | null; message: string };
  workspace: { workspace_id: string; name: string; path: string; lifecycle_state: string;
    last_active_at: string | null; latest_archive_id: string | null; current_operation_id: string | null; activity_source: string };
  repositories: Repository[]; errors: string[]; dirty_count: number; issue_count: number;
  protected: boolean; active_lease: boolean; archives: { archive_id: string; created_at: string; location: string; storage_policy: string }[];
  policy: { idle_days: number | null; auto_enabled: boolean | null; archive_root: string | null; retain_until: string | null };
  effective: { idle_days: number; idle_days_source: string; archive_root: string; auto_enabled: boolean };
}
export interface Snapshot { workspaces: Workspace[]; checked_at: string; version: string }
export interface Preferences {
  default_editor: string; refresh_seconds: number; remote_cache_seconds: number; theme: string;
  editor_program: string; editor_args: string[]; workspace_editors: Record<string, string>; remote_hosts: Record<string, string>;
}
export interface Lifecycle {
  version: number; idle_days: number; dirty_policy: string;
  archive: { root: string; git_backup: string; max_bundle_bytes: number | null;
    materials: { include: string[]; exclude: string[]; disposable_ignored: string[]; exclude_managed_dependencies: boolean };
    keep_workspace_skeleton: boolean; migration: Record<string, unknown> };
  auto: { enabled: boolean; grace_hours: number; scan_interval_hours: number; authorization: string | null };
  memory: { index_required: boolean; ai: { enabled: boolean; provider: string | null; trigger: string; command: string[];
    endpoint: string | null; model: string | null; credential_env: string | null; input_include: string[]; input_exclude: string[];
    timeout_seconds: number; max_input_chars: number; max_output_tokens: number } };
  cloud: { enabled: boolean; provider: string | null; content: string[] };
}
export interface GlobalConfig { workspace_root: string | null; store_path: string; max_store_size_gb: number; gc_threshold_days: number; lifecycle: Lifecycle }
export interface Configuration { global: GlobalConfig; version: string; preferences: Preferences; preferences_version: string;
  editors: { id: string; name: string; available: boolean }[]; defaults: GlobalConfig; read_only: string[];
  capabilities: { native_scheduler: boolean; remote_deletion_proof: boolean } }
export interface Plan { plan_id: string; kind: string; expires_at: string;
  entries: { workspace_id: string; name: string; message: string; reason_codes: string[]; excluded: boolean;
    path: string; archive_root: string; repositories: { repo: string; path: string; head: string; dirty: Record<string, number>; archive_warnings?: ArchiveNotice[] }[] }[];
  value: unknown; target: string | null; restore_branch: string | null }
export interface Operation { operation_id: string; plan_id: string; kind: string; state: string; started_at: string; updated_at: string; error: string | null;
  undo?: { available: boolean; reason: string; operation_id: string | null };
  summary: { scope_total: number; archived_count: number; restored_count: number; trashed_count: number; skipped_count: number; failed_count: number; unfinished_count: number };
  progress: { phase: string; files_done: number; files_total: number; bytes_copied: number; bytes_total: number };
  results: { name: string; outcome: string; message: string; reason_codes: string[]; repositories?: { path: string; archive_warnings?: ArchiveNotice[] }[] }[] }
export interface Job { plan_id: string; state: string; error?: string; operation?: Operation }
export interface Remote { state: string; repository_state: string; branch_state: string; branch?: string; checked_at?: string }
export interface DiagnosticDelivery { delivery: 'prefill_requested' | 'clipboard' | 'manual_copy'; clipboard_copied: boolean; prompt: string }
export interface OpenResult { state: 'launch_requested' | 'launch_failed'; launch_error: string | null; editor: string; remote: Record<string, Remote>; diagnostics: DiagnosticDelivery | null }
