export interface VDWConfig {
    workspace: WorkspaceMeta;
    dependencies: Dependency[];
}

export interface WorkspaceMeta {
    name: string;
    version: string;
}

export interface Dependency {
    repo: string;
    path: string;
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
    worktrees: WorktreeInfo[];
}

export interface LinkEntry {
    source: string;
    target: string;
    link_type: 'HardLink' | 'Symlink' | 'Junction';
    dependency_repo: string;
}

export interface WorktreeInfo {
    repo_url: string;
    branch: string;
    path: string;
}

export interface ValidationResult {
    valid: boolean;
    errors: string[];
    workspace_name?: string;
    dependency_count?: number;
}
