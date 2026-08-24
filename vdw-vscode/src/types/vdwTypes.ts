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

export type VDWTopologyState =
    | 'ready'
    | 'missing'
    | 'invalid_worktree'
    | 'unowned'
    | 'conflict'
    | 'detached';

export type VDWRecommendedAction =
    | 'none'
    | 'link'
    | 'relink'
    | 'resolve_conflicts';

export interface RepositoryTopologyStatus {
    repo: string;
    path: string;
    full_path: string;
    editable: boolean;
    declared_revision: string;
    topology_state: VDWTopologyState;
    recommended_action: VDWRecommendedAction;
    tracked_link_count: number;
    conflict_count: number;
    missing_count: number;
    detached_equivalent_count: number;
    message?: string;
}

export interface WorkspaceTopologyStatusResult {
    workspace_root: string;
    workspace_name: string;
    healthy: boolean;
    issue_count: number;
    conflict_count: number;
    repositories: RepositoryTopologyStatus[];
}

export type RepositoryRuntimeState =
    | 'checking'
    | 'missing'
    | 'invalidWorktree'
    | 'unowned'
    | 'topologyConflict'
    | 'detachedLink'
    | 'scmMissing'
    | 'branchDrift'
    | 'dirty'
    | 'ready';

/** Runtime-only repository state used by the IDE. Never persisted to state.json. */
export interface RepositoryRuntimeStatus {
    dependencyPath: string;
    fullPath: string;
    state: RepositoryRuntimeState;
    exists: boolean;
    scmRegistered: boolean;
    desiredRevision: string;
    currentBranch?: string;
    detachedCommit?: string;
    changeCount: number;
    ahead: number;
    behind: number;
    topologyState?: VDWTopologyState;
    recommendedAction?: VDWRecommendedAction;
    trackedLinkCount: number;
    conflictCount: number;
    missingLinkCount: number;
    detachedEquivalentCount: number;
    topologyMessage?: string;
}

export interface WorkspaceRuntimeStatus {
    repositories: RepositoryRuntimeStatus[];
    issueCount: number;
    dirtyCount: number;
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
