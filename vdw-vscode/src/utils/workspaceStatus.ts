import * as fs from 'fs';
import * as path from 'path';
import {
    Dependency,
    RepositoryRuntimeStatus,
    RepositoryRuntimeState,
    WorkspaceRuntimeStatus,
} from '../types/vdwTypes';

const ISSUE_STATES = new Set<RepositoryRuntimeState>([
    'missing',
    'invalidWorktree',
    'scmMissing',
    'branchDrift',
]);

export function collectRepositoryRuntimeStatus(
    dependency: Dependency,
    workspaceRoot: string,
    gitApi?: any,
): RepositoryRuntimeStatus {
    const fullPath = path.resolve(workspaceRoot, dependency.path);
    const desiredRevision = getDesiredRevision(dependency);
    const exists = isDirectory(fullPath);
    const hasGitMetadata = dependency.editable && fs.existsSync(path.join(fullPath, '.git'));
    const repository = dependency.editable && exists
        ? findExactRepository(gitApi, fullPath)
        : undefined;
    const head = repository?.state?.HEAD;
    const currentBranch = typeof head?.name === 'string' ? head.name : undefined;
    const detachedCommit = !currentBranch && typeof head?.commit === 'string'
        ? head.commit
        : undefined;
    const changeCount = repository ? countChanges(repository.state) : 0;
    const ahead = numberOrZero(head?.ahead);
    const behind = numberOrZero(head?.behind);

    let state: RepositoryRuntimeStatus['state'];
    if (!exists) {
        state = 'missing';
    } else if (dependency.editable && !hasGitMetadata) {
        state = 'invalidWorktree';
    } else if (dependency.editable && !repository) {
        state = 'scmMissing';
    } else if (dependency.branch && currentBranch && dependency.branch !== currentBranch) {
        state = 'branchDrift';
    } else if (changeCount > 0) {
        state = 'dirty';
    } else {
        state = 'ready';
    }

    return {
        dependencyPath: dependency.path,
        fullPath,
        state,
        exists,
        scmRegistered: Boolean(repository),
        desiredRevision,
        currentBranch,
        detachedCommit,
        changeCount,
        ahead,
        behind,
    };
}

export function getDependencyName(dependency: Dependency): string {
    const source = dependency.remote || dependency.origin || dependency.local || dependency.path;
    return path.basename(source.replace(/[\\/]+$/, '').replace(/\.git$/, ''));
}

export function getDesiredRevision(dependency: Dependency): string {
    return dependency.branch
        || dependency.tag
        || dependency.commit?.substring(0, 7)
        || 'main';
}

export function isDefaultScope(dependency: Dependency): boolean {
    return dependency.scope.include.length === 1
        && dependency.scope.include[0] === '**/*'
        && dependency.scope.exclude.length === 0;
}

export function repositoryStatusPriority(status: RepositoryRuntimeStatus): number {
    if (ISSUE_STATES.has(status.state)) return 0;
    if (status.state === 'dirty') return 1;
    return 2;
}

export function summarizeWorkspaceStatus(
    repositories: RepositoryRuntimeStatus[],
): WorkspaceRuntimeStatus {
    return {
        repositories,
        issueCount: repositories.filter(status => ISSUE_STATES.has(status.state)).length,
        dirtyCount: repositories.filter(status => status.changeCount > 0).length,
    };
}

export function formatRepositoryDescription(
    dependency: Dependency,
    status: RepositoryRuntimeStatus,
): string {
    const revision = status.currentBranch
        || (status.detachedCommit ? `@${status.detachedCommit.substring(0, 7)}` : status.desiredRevision);

    switch (status.state) {
        case 'checking':
            return `${status.desiredRevision} · checking…`;
        case 'missing':
            return `${status.desiredRevision} · missing`;
        case 'invalidWorktree':
            return `${status.desiredRevision} · invalid worktree`;
        case 'scmMissing':
            return `${revision} · SCM not loaded`;
        case 'branchDrift':
            return `${revision} → ${status.desiredRevision}${formatChangeCount(status)}${formatSyncState(status)}`;
        case 'dirty': {
            const sync = formatSyncState(status);
            return `${revision} · ${status.changeCount} change${status.changeCount === 1 ? '' : 's'}${sync}`;
        }
        case 'ready': {
            const sync = formatSyncState(status);
            return `${revision}${dependency.editable ? sync : ' · read-only'}`;
        }
    }
}

function formatChangeCount(status: RepositoryRuntimeStatus): string {
    return status.changeCount > 0
        ? ` · ${status.changeCount} change${status.changeCount === 1 ? '' : 's'}`
        : '';
}

function formatSyncState(status: RepositoryRuntimeStatus): string {
    const parts: string[] = [];
    if (status.ahead > 0) parts.push(`↑${status.ahead}`);
    if (status.behind > 0) parts.push(`↓${status.behind}`);
    return parts.length > 0 ? ` · ${parts.join(' ')}` : '';
}

function isDirectory(candidate: string): boolean {
    try {
        return fs.statSync(candidate).isDirectory();
    } catch {
        return false;
    }
}

function normalizeRepositoryPath(candidate: string): string {
    const normalized = path.normalize(path.resolve(candidate)).replace(/[\\/]+$/, '');
    return process.platform === 'win32' ? normalized.toLowerCase() : normalized;
}

function findExactRepository(gitApi: any, repositoryPath: string): any | undefined {
    if (!gitApi) return undefined;
    const expected = normalizeRepositoryPath(repositoryPath);
    return (gitApi.repositories ?? []).find((repository: any) =>
        normalizeRepositoryPath(repository.rootUri.fsPath) === expected,
    );
}

function countChanges(state: any): number {
    return [
        state?.mergeChanges,
        state?.indexChanges,
        state?.workingTreeChanges,
        state?.untrackedChanges,
    ].reduce((total, changes) => total + (Array.isArray(changes) ? changes.length : 0), 0);
}

function numberOrZero(value: unknown): number {
    return typeof value === 'number' && Number.isFinite(value) ? value : 0;
}
