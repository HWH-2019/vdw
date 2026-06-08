import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import * as vscode from 'vscode';

interface EditableLock {
    repo_id: string;
    branch: string;
    workspace_path: string;
    worktree_path: string;
    created_at: string;
}

interface EditableLockRegistry {
    version: number;
    locks: EditableLock[];
}

export class EditableLockService {
    private getLocksPath(): string {
        return path.join(os.homedir(), '.vdw', 'editable_locks.json');
    }

    /**
     * Check if a repo+branch is already locked as editable by another workspace.
     * Returns the conflicting lock if found, null otherwise.
     */
    public checkEditableLock(repoId: string, branch: string): EditableLock | null {
        const locksPath = this.getLocksPath();
        if (!fs.existsSync(locksPath)) {
            return null;
        }

        try {
            const content = fs.readFileSync(locksPath, 'utf-8');
            const registry: EditableLockRegistry = JSON.parse(content);
            const currentWorkspace = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

            for (const lock of registry.locks) {
                if (lock.repo_id === repoId && lock.branch === branch) {
                    // Same workspace doesn't count as conflict (allows relink)
                    if (lock.workspace_path === currentWorkspace) {
                        continue;
                    }
                    // Validate lock by checking worktree_path existence (primary validity judge)
                    if (!fs.existsSync(lock.worktree_path)) {
                        // Stale lock, skip
                        continue;
                    }
                    return lock;
                }
            }
        } catch (err) {
            // Graceful degradation: if lock file is corrupted or unreadable, allow operation
            console.warn('Failed to read editable locks:', err);
        }

        return null;
    }
}
