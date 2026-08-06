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
    local: string;    // VDW-managed worktree path (or original path for old-format configs)
    path: string;     // Workspace link path (relative)
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
