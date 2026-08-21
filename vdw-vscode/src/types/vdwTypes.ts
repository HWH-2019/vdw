export interface VDWConfig {
    workspace: WorkspaceMeta;
    dependencies: Dependency[];
}

export interface WorkspaceMeta {
    name: string;
    version: string;
}

export interface Dependency {
    remote: string;   // Remote Git URL, empty string for local repos
    origin: string;   // Original source path for local-only repos (preserved when local is overwritten)
    local: string;    // Native workspace worktree after link (or original path for old configs)
    path: string;     // Native worktree root (editable) or projection root (read-only)
    branch?: string;
    tag?: string;
    commit?: string;
    scope: Scope;
    editable: boolean;
}

export interface Scope {
    include: string[];
    exclude: string[];
}

export interface WorkspaceState {
    workspace_root: string;
    links: LinkEntry[];
}

export interface LinkEntry {
    source: string;
    target: string;
    link_type: 'HardLink' | 'Symlink' | 'Junction';
    dependency_repo: string;
}

export interface ValidationResult {
    valid: boolean;
    errors: string[];
    workspace_name?: string;
    dependency_count?: number;
}

export interface RepoStatusResult {
    repo: string;
    branch: string;
    editable: boolean;
    dirty: boolean;
    changes: { modified: number; added: number; deleted: number; untracked: number } | null;
    worktree_path: string | null;
}

export type LinkConflictReason =
    | 'content_differs'
    | 'metadata_differs'
    | 'source_missing'
    | 'target_missing'
    | 'target_unreadable'
    | 'target_type_changed'
    | 'link_target_changed';

export interface LinkConflict {
    source: string;
    target: string;
    relative_path: string;
    link_type: 'HardLink' | 'Symlink' | 'Junction';
    dependency_repo: string;
    reason: LinkConflictReason;
    resolvable: boolean;
}

export interface LinkConflictReport {
    conflict_count: number;
    missing_count: number;
    detached_equivalent_count: number;
    conflicts: LinkConflict[];
}

export type LinkConflictResolutionSide = 'store' | 'workspace';

export interface LinkConflictResolutionResult {
    resolved_count: number;
    accepted: LinkConflictResolutionSide;
    backup_root: string;
}
