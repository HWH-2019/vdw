import { exec } from 'child_process';
import { promisify } from 'util';
import * as vscode from 'vscode';
import { log, logError } from './logger';

const execAsync = promisify(exec);

const TIMEOUT_MS = 15000;
const MAX_ITEMS = 10;

export interface BranchItem extends vscode.QuickPickItem {
    label: string;          // branch name like "main"
    description: string;    // "$(star-full) Current" | "$(star-full) Default" | "remote" | ""
    detail?: string;        // e.g. "origin/main"
}

/**
 * Get all branches for a local repository (including remote tracking branches).
 * Runs git fetch first (best-effort), then lists and deduplicates branches.
 */
export async function getLocalBranches(repoPath: string): Promise<BranchItem[]> {
    // Step 1: git fetch --all (best-effort, non-blocking on failure)
    try {
        await execAsync('git fetch --all', { cwd: repoPath, timeout: TIMEOUT_MS });
        log('git fetch --all completed');
    } catch (err) {
        logError('git fetch --all failed (non-blocking)', err);
    }

    // Step 2: git branch -a --format='%(refname:short)'
    let branchOutput: string;
    try {
        const { stdout } = await execAsync("git branch -a --format='%(refname:short)'", {
            cwd: repoPath,
            timeout: TIMEOUT_MS,
        });
        branchOutput = stdout.trim();
    } catch (err) {
        logError('git branch -a failed', err);
        return [];
    }

    if (!branchOutput) {
        return [];
    }

    // Step 3: Deduplicate — strip origin/ prefix from remote tracking branches, merge with local
    const lines = branchOutput.split('\n').map(l => l.trim()).filter(Boolean);
    const localBranches = new Set<string>();
    const remoteBranches = new Set<string>();

    for (const line of lines) {
        // Skip symbolic refs like HEAD and origin/HEAD — not real branches
        if (line === 'HEAD' || line === 'origin/HEAD' || line.endsWith('/HEAD')) {
            continue;
        }
        if (line.startsWith('origin/')) {
            remoteBranches.add(line.slice('origin/'.length));
        } else {
            localBranches.add(line);
        }
    }

    // Merge: local takes priority
    const allBranches = new Set<string>(Array.from(localBranches));
    for (const remote of Array.from(remoteBranches)) {
        if (!allBranches.has(remote)) {
            allBranches.add(remote);
        }
    }

    // Step 4: Detect current branch
    const { branch: currentBranch, isDetached } = await getCurrentBranch(repoPath);

    // Step 5 & 6: Build BranchItem array with descriptions
    const items: BranchItem[] = [];
    for (const name of Array.from(allBranches)) {
        const isCurrent = !isDetached && name === currentBranch;
        const isRemoteOnly = !localBranches.has(name);
        items.push({
            label: name,
            description: isCurrent ? '$(star-full) Current' : isRemoteOnly ? 'remote' : '',
            detail: isRemoteOnly ? `origin/${name}` : undefined,
        });
    }

    // Step 7: Sort alphabetically, pin current branch to top
    items.sort((a, b) => {
        const aIsCurrent = a.description === '$(star-full) Current' ? 1 : 0;
        const bIsCurrent = b.description === '$(star-full) Current' ? 1 : 0;
        if (aIsCurrent !== bIsCurrent) {
            return bIsCurrent - aIsCurrent;
        }
        return a.label.localeCompare(b.label);
    });

    // Step 8: Return top 10 items only
    return items.slice(0, MAX_ITEMS);
}

/**
 * Get branch list for a remote repository via git ls-remote.
 * Returns empty array on failure (caller handles fallback).
 */
export async function getRemoteBranches(repoUrl: string): Promise<BranchItem[]> {
    // Step 1: git ls-remote --heads <url>
    let headsOutput: string;
    try {
        const { stdout } = await execAsync(`git ls-remote --heads ${repoUrl}`, {
            timeout: TIMEOUT_MS,
        });
        headsOutput = stdout.trim();
    } catch (err) {
        logError('git ls-remote --heads failed', err);
        return [];
    }

    if (!headsOutput) {
        return [];
    }

    // Step 2: Parse output — each line: <hash>\trefs/heads/<branch-name>
    const branches: string[] = [];
    for (const line of headsOutput.split('\n')) {
        const trimmed = line.trim();
        if (!trimmed) { continue; }
        const idx = trimmed.indexOf('\trefs/heads/');
        if (idx !== -1) {
            branches.push(trimmed.slice(idx + '\trefs/heads/'.length));
        }
    }

    // Step 3: Detect default branch via git ls-remote --symref <url> HEAD
    let defaultBranch = '';
    try {
        const { stdout } = await execAsync(`git ls-remote --symref ${repoUrl} HEAD`, {
            timeout: TIMEOUT_MS,
        });
        const symrefOutput = stdout.trim();
        for (const line of symrefOutput.split('\n')) {
            const match = line.match(/^ref:\s+refs\/heads\/(\S+)/);
            if (match) {
                defaultBranch = match[1];
                break;
            }
        }
        if (defaultBranch) {
            log(`Detected default branch: ${defaultBranch}`);
        }
    } catch (err) {
        logError('git ls-remote --symref failed', err);
    }

    // Step 4: Mark default branch with description
    const items: BranchItem[] = branches.map(name => ({
        label: name,
        description: name === defaultBranch ? '$(star-full) Default' : '',
    }));

    // Step 5: Sort alphabetically, pin default branch to top
    items.sort((a, b) => {
        const aIsDefault = a.description === '$(star-full) Default' ? 1 : 0;
        const bIsDefault = b.description === '$(star-full) Default' ? 1 : 0;
        if (aIsDefault !== bIsDefault) {
            return bIsDefault - aIsDefault;
        }
        return a.label.localeCompare(b.label);
    });

    // Step 6: Return top 10 items only
    return items.slice(0, MAX_ITEMS);
}

/**
 * Get the current branch name. Returns commit hash for detached HEAD state.
 */
export async function getCurrentBranch(repoPath: string): Promise<{
    branch: string;
    isDetached: boolean;
}> {
    try {
        const { stdout } = await execAsync('git branch --show-current', {
            cwd: repoPath,
            timeout: TIMEOUT_MS,
        });
        const name = stdout.trim();
        if (name) {
            return { branch: name, isDetached: false };
        }

        // Detached HEAD — fall back to short commit hash
        const { stdout: hashOut } = await execAsync('git rev-parse --short HEAD', {
            cwd: repoPath,
            timeout: TIMEOUT_MS,
        });
        const hash = hashOut.trim();
        return { branch: hash || 'HEAD', isDetached: true };
    } catch (err) {
        logError('getCurrentBranch failed', err);
        return { branch: '', isDetached: false };
    }
}
