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
 * Return complete editable worktrees. Workspace projections only contain the
 * configured scope and therefore cannot be used as Git roots: Git would report
 * every excluded tracked file as deleted.
 */
export function getEditableWorktreePaths(
    config: VDWConfig,
    identifier?: string,
): string[] {
    const seen = new Set<string>();
    const worktrees: string[] = [];

    for (const dependency of config.dependencies) {
        if (!dependency.editable || !dependency.local) continue;
        if (!dependencyMatchesIdentifier(dependency, identifier)) continue;

        if (seen.has(dependency.local)) continue;
        seen.add(dependency.local);
        worktrees.push(dependency.local);
    }

    return worktrees;
}
