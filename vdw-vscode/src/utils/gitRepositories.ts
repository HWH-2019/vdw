import * as path from 'path';
import { Dependency, VDWConfig } from '../types/vdwTypes';

export function dependencyMatchesIdentifier(
    dependency: Dependency,
    identifier?: string,
): boolean {
    if (!identifier) return true;
    return [
        dependency.remote,
        dependency.origin,
        dependency.local,
        dependency.path,
    ].some(candidate => candidate === identifier);
}

/**
 * Return native editable worktrees inside the VDW workspace. Since VDW 0.7,
 * dependency.path is the complete Git worktree and scope only filters AI context.
 */
export function getEditableWorktreePaths(
    config: VDWConfig,
    workspaceRoot: string,
    identifier?: string,
): string[] {
    const seen = new Set<string>();
    const worktrees: string[] = [];

    for (const dependency of config.dependencies) {
        if (!dependency.editable) continue;
        if (!dependencyMatchesIdentifier(dependency, identifier)) continue;

        const worktreePath = path.resolve(workspaceRoot, dependency.path);
        if (seen.has(worktreePath)) continue;
        seen.add(worktreePath);
        worktrees.push(worktreePath);
    }

    return worktrees;
}
