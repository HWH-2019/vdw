import * as vscode from 'vscode';
import * as path from 'path';
import * as fs from 'fs';
import { VDWCliService } from '../services/vdwCliService';
import { ConfigService } from '../services/configService';
import { ContextService } from '../services/contextService';
import { AutoOpenService } from '../services/autoOpenService';
import { VDWFileProvider } from '../providers/fileProvider';
import { WorkspaceTreeProvider } from '../providers/workspaceTreeView';
import { StatusBarManager } from '../providers/statusBar';
import { WorkspaceStatusService } from '../services/workspaceStatusService';
import { Dependency, RepositoryRuntimeStatus } from '../types/vdwTypes';
import {
    formatRepositoryDescription,
    getDependencyName,
    repositoryStatusPriority,
} from '../utils/workspaceStatus';
import * as yaml from 'js-yaml';
import { log, logError, show as showLog } from '../utils/logger';
import { handleAddRepo } from './addRepo';
import { ensureLinkConflictsResolved } from './linkConflicts';

export function registerCommands(
    context: vscode.ExtensionContext,
    cliService: VDWCliService,
    configService: ConfigService,
    contextService: ContextService,
    fileProvider: VDWFileProvider,
    treeProvider: WorkspaceTreeProvider,
    workspaceStatusService: WorkspaceStatusService,
    statusBar: StatusBarManager,
    autoOpenService?: AutoOpenService,
): void {
    // vdw.init
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.init', async () => {
            try {
                statusBar.showBusy('Initializing...');
                const result = await cliService.init();

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage('VDW workspace initialized successfully!');
                    // Reload config
                    configService.reload();
                    treeProvider.refresh();
                    statusBar.update();

                    // Open the created .vdwrc
                    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
                    if (workspaceFolder) {
                        const vdwrcUri = vscode.Uri.joinPath(workspaceFolder.uri, '.vdwrc');
                        const doc = await vscode.workspace.openTextDocument(vdwrcUri);
                        await vscode.window.showTextDocument(doc);
                    }
                } else {
                    vscode.window.showErrorMessage(`VDW init failed: ${result.stderr || result.stdout}`);
                    statusBar.showError('Init failed');
                }
            } catch (err) {
                logError('Init command failed', err);
                vscode.window.showErrorMessage(`VDW init error: ${err}`);
                statusBar.showError('Error');
            }
        })
    );

    // vdw.link
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.link', async () => {
            try {
                statusBar.showBusy('Linking...');
                log('Running vdw link...');
                const result = await cliService.link();

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage('Dependencies linked successfully!');
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                    
                    // Auto-open Git repositories for editable dependencies
                    await autoOpenService?.openAllEditableRepositories();
                } else {
                    vscode.window.showErrorMessage(`VDW link failed: ${result.stderr || result.stdout}`);
                    statusBar.showError('Link failed');
                }
            } catch (err) {
                logError('Link command failed', err);
                vscode.window.showErrorMessage(`VDW link error: ${err}`);
                statusBar.showError('Error');
            }
        })
    );

    // vdw.unlink
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.unlink', async () => {
            let autoOpenSuspended = false;
            let reopenRepositories = false;
            try {
                if (!await ensureLinkConflictsResolved(cliService)) {
                    statusBar.update();
                    return;
                }
                if (autoOpenService) {
                    autoOpenService.suspendAutoOpen();
                    autoOpenSuspended = true;
                }
                await autoOpenService?.closeEditableRepositories();
                statusBar.showBusy('Unlinking...');
                const result = await cliService.unlink();

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage('Dependencies unlinked successfully!');
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    statusBar.update(0);
                } else {
                    reopenRepositories = true;
                    vscode.window.showErrorMessage(`VDW unlink failed: ${result.stderr || result.stdout}`);
                    statusBar.showError('Unlink failed');
                }
            } catch (err) {
                reopenRepositories = true;
                logError('Unlink command failed', err);
                vscode.window.showErrorMessage(`VDW unlink error: ${err}`);
                statusBar.showError('Error');
            } finally {
                if (autoOpenSuspended) {
                    autoOpenService?.resumeAutoOpen();
                }
                if (autoOpenSuspended && reopenRepositories) {
                    configService.reload();
                    await autoOpenService?.openAllEditableRepositories();
                }
            }
        })
    );

    // vdw.relink - refresh without replacing native editable worktrees
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.relink', async () => {
            try {
                const proceed = await vscode.window.showWarningMessage(
                    'This will refresh read-only dependencies and migrate legacy editable projections. Native editable worktrees and local changes stay in place. Continue?',
                    { modal: true },
                    'Yes, Relink',
                    'Cancel'
                );
                
                if (proceed !== 'Yes, Relink') {
                    return;
                }

                if (!await ensureLinkConflictsResolved(cliService)) {
                    statusBar.update();
                    return;
                }

                statusBar.showBusy('Relinking...');
                log('Refreshing VDW workspace...');

                const relinkResult = await cliService.relink();
                if (relinkResult.exitCode === 0) {
                    vscode.window.showInformationMessage('Dependencies relinked successfully with updated config!');
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                } else {
                    vscode.window.showErrorMessage(`Relink failed: ${relinkResult.stderr || relinkResult.stdout}`);
                    statusBar.showError('Relink failed');
                }
            } catch (err) {
                logError('Relink command failed', err);
                vscode.window.showErrorMessage(`VDW relink error: ${err}`);
                statusBar.showError('Error');
            } finally {
                configService.reload();
                await autoOpenService?.openAllEditableRepositories();
            }
        })
    );

    // vdw.validate
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.validate', async () => {
            try {
                const result = await cliService.validate();

                if (result.valid) {
                    vscode.window.showInformationMessage(
                        `Configuration valid! Workspace: ${result.workspace_name}, ${result.dependency_count} dependencies`
                    );
                } else {
                    const message = result.errors.join('\n');
                    vscode.window.showErrorMessage(`Configuration errors:\n${message}`);
                }
            } catch (err) {
                logError('Validate command failed', err);
                vscode.window.showErrorMessage(`VDW validate error: ${err}`);
            }
        })
    );

    // vdw.addRepo - Interactive wizard, auto-link on success
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.addRepo', async () => {
            const success = await handleAddRepo(cliService, configService);
            if (success) {
                log('Add repository succeeded, auto-linking...');
                statusBar.showBusy('Linking...');
                const result = await cliService.link();
                if (result.exitCode === 0) {
                    log('Auto-link after add succeeded');
                } else {
                    logError('Auto-link after add failed', result.stderr);
                }
            }
            // Refresh UI after adding
            treeProvider?.refresh();
            statusBar?.update();
            // Auto-open Git repositories if new dependency is editable
            await autoOpenService?.openAllEditableRepositories();
        }),
    );

    // vdw.unlinkRepo - Unlink a specific repository
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.unlinkRepo', async (treeItem?: any) => {
            if (!treeItem || !treeItem.dependency) {
                vscode.window.showErrorMessage('No repository selected');
                return;
            }

            const dep = treeItem.dependency;
            const repoIdentifier = dep.remote || dep.origin || dep.local;
            const repoName = dep.remote
                ? dep.remote.split('/').pop()?.replace('.git', '') || dep.remote
                : path.basename(dep.origin || dep.local);

            const proceed = await vscode.window.showWarningMessage(
                `Unlink repository "${repoName}"?\n\nPath: ${dep.path}\n\nThis will remove the linked files from the workspace.`,
                { modal: true },
                'Yes, Unlink',
                'Cancel'
            );

            if (proceed !== 'Yes, Unlink') {
                return;
            }

            let autoOpenSuspended = false;
            let reopenRepository = false;
            try {
                if (!await ensureLinkConflictsResolved(cliService, repoIdentifier)) {
                    statusBar.update();
                    return;
                }
                if (autoOpenService) {
                    autoOpenService.suspendAutoOpen();
                    autoOpenSuspended = true;
                }
                await autoOpenService?.closeEditableRepositories(repoIdentifier);
                statusBar.showBusy(`Unlinking ${repoName}...`);
                log(`Unlinking repository: ${repoName} (${repoIdentifier})`);

                // Execute per-repo unlink
                const result = await cliService.unlinkRepo(repoIdentifier);

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage(`Repository "${repoName}" unlinked successfully!`);
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                } else {
                    reopenRepository = true;
                    vscode.window.showErrorMessage(`Failed to unlink ${repoName}: ${result.stderr || result.stdout}`);
                    statusBar.showError('Unlink failed');
                }
            } catch (err) {
                reopenRepository = true;
                logError(`Unlink repository ${repoName} failed`, err);
                vscode.window.showErrorMessage(`Error unlinking ${repoName}: ${err}`);
                statusBar.showError('Error');
            } finally {
                if (autoOpenSuspended) {
                    autoOpenService?.resumeAutoOpen();
                }
                if (autoOpenSuspended && reopenRepository) {
                    configService.reload();
                    await autoOpenService?.openAllEditableRepositories(repoIdentifier);
                }
            }
        })
    );

    // vdw.linkRepo - Link a specific repository
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.linkRepo', async (treeItem?: any) => {
            if (!treeItem || !treeItem.dependency) {
                vscode.window.showErrorMessage('No repository selected');
                return;
            }

            const dep = treeItem.dependency;
            const repoIdentifier = dep.remote || dep.origin || dep.local;
            const repoName = dep.remote
                ? dep.remote.split('/').pop()?.replace('.git', '') || dep.remote
                : path.basename(dep.origin || dep.local);

            try {
                statusBar.showBusy(`Linking ${repoName}...`);
                log(`Linking repository: ${repoName} (${repoIdentifier})`);

                const result = await cliService.linkRepo(repoIdentifier);

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage(`Repository "${repoName}" linked successfully!`);
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                    
                    // Auto-open Git repository for this editable dependency
                    await autoOpenService?.openAllEditableRepositories(repoIdentifier);
                } else {
                    vscode.window.showErrorMessage(`Failed to link ${repoName}: ${result.stderr || result.stdout}`);
                    statusBar.showError('Link failed');
                }
            } catch (err) {
                logError(`Link repository ${repoName} failed`, err);
                vscode.window.showErrorMessage(`Error linking ${repoName}: ${err}`);
                statusBar.showError('Error');
            }
        })
    );

    // vdw.removeRepo - Remove a repository (from context menu)
    // Flow: check dirty state → confirm → unlink → remove from .vdwrc (or keep files)
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.removeRepo', async (treeItem?: any) => {
            if (!treeItem || !treeItem.dependency) {
                vscode.window.showErrorMessage('No repository selected');
                return;
            }

            const dep = treeItem.dependency;
            const repoIdentifier = dep.remote || dep.origin || dep.local;
            const repoName = dep.remote
                ? dep.remote.split('/').pop()?.replace('.git', '') || dep.remote
                : path.basename(dep.origin || dep.local);

            // Helper: remove dependency from .vdwrc only (no unlink)
            const removeFromVdwrcOnly = async (): Promise<boolean> => {
                const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
                if (!workspaceFolder) {
                    throw new Error('No workspace folder found');
                }
                const vdwrcPath = configService.getConfigPath()
                    || path.join(workspaceFolder.uri.fsPath, '.vdwrc');
                if (!fs.existsSync(vdwrcPath)) {
                    throw new Error('VDW configuration file not found');
                }
                const content = fs.readFileSync(vdwrcPath, 'utf-8');
                const config = yaml.load(content) as any;
                if (!config || !config.dependencies) {
                    throw new Error('Invalid .vdwrc format');
                }
                const beforeCount = config.dependencies.length;
                config.dependencies = config.dependencies.filter((d: any) => {
                    const sameDependency = (dep.remote && d.remote === dep.remote)
                        || (dep.origin && d.origin === dep.origin)
                        || (dep.local && d.local === dep.local)
                        || d.repo === repoIdentifier
                        || d.path === dep.path;
                    return !sameDependency;
                });
                if (config.dependencies.length === beforeCount) {
                    logError('Dependency not found in .vdwrc, nothing to remove', { remote: dep.remote, local: dep.local, path: dep.path });
                } else {
                    const yamlStr = yaml.dump(config, { indent: 2, lineWidth: 120, noRefs: true });
                    fs.writeFileSync(vdwrcPath, yamlStr, 'utf-8');
                    log(`Removed dependency from .vdwrc: ${repoName}`);
                }
                return true;
            };

            // Helper: let the CLI own unlink, state, lock, store, and config updates.
            const fullRemoval = async (force = false): Promise<boolean> => {
                if (!await ensureLinkConflictsResolved(cliService, repoIdentifier)) {
                    return false;
                }
                let autoOpenSuspended = false;
                let reopenRepository = false;
                try {
                    if (autoOpenService) {
                        autoOpenService.suspendAutoOpen();
                        autoOpenSuspended = true;
                    }
                    await autoOpenService?.closeEditableRepositories(repoIdentifier);
                    log(`Removing repository through CLI: ${repoIdentifier}`);
                    const result = await cliService.removeRepo(repoIdentifier, force);
                    if (result.exitCode !== 0) {
                        throw new Error(result.stderr || result.stdout || 'VDW CLI remove failed');
                    }
                    return true;
                } catch (error) {
                    reopenRepository = true;
                    throw error;
                } finally {
                    if (autoOpenSuspended) {
                        autoOpenService?.resumeAutoOpen();
                    }
                    if (autoOpenSuspended && reopenRepository) {
                        configService.reload();
                        await autoOpenService?.openAllEditableRepositories(repoIdentifier);
                    }
                }
            };

            try {
                // Check dirty state for editable repos
                let isDirty = false;
                let changesInfo = '';
                if (dep.editable) {
                    log(`Checking dirty state for: ${repoName}`);
                    // dep.path is supported by the first CLI version that introduced
                    // `repo status`, while full remote URLs were not matched reliably.
                    const statusIdentifier = dep.path || repoIdentifier;
                    const statusPromise = cliService.checkRepoStatus(statusIdentifier);
                    const timeoutPromise = new Promise<null>(resolve => setTimeout(() => resolve(null), 5000));
                    const statusResult = await Promise.race([statusPromise, timeoutPromise]);

                    if (statusResult && statusResult.dirty) {
                        isDirty = true;
                        const c = statusResult.changes;
                        if (c) {
                            const parts: string[] = [];
                            if (c.modified) parts.push(`${c.modified} modified`);
                            if (c.added) parts.push(`${c.added} added`);
                            if (c.deleted) parts.push(`${c.deleted} deleted`);
                            if (c.untracked) parts.push(`${c.untracked} untracked`);
                            changesInfo = parts.join(', ');
                        }
                    }
                }

                if (isDirty) {
                    // Dirty: show enhanced warning with 3 buttons
                    const warningMsg = changesInfo
                        ? `Repository "${repoName}" has uncommitted changes (${changesInfo}).\n\nPath: ${dep.path}\n\nRemoving will affect these files. What would you like to do?`
                        : `Repository "${repoName}" has uncommitted changes.\n\nPath: ${dep.path}\n\nRemoving will affect these files. What would you like to do?`;

                    const choice = await vscode.window.showWarningMessage(
                        warningMsg,
                        { modal: true },
                        'Discard & Remove',
                        'Keep Files',
                        'Cancel'
                    );

                    if (choice === 'Discard & Remove') {
                        statusBar.showBusy(`Removing ${repoName}...`);
                        log(`Discard & Remove: full removal for ${repoName}`);
                        if (!await fullRemoval(true)) return;
                    } else if (choice === 'Keep Files') {
                        // Keep worktree files: remove from .vdwrc only, skip unlink
                        log(`Keep Files: removing from config only for ${repoName}`);
                        statusBar.showBusy(`Updating config...`);
                        await removeFromVdwrcOnly();
                        vscode.window.showInformationMessage(`Repository "${repoName}" removed from .vdwrc (worktree files preserved).`);
                        configService.reload();
                        contextService.refresh();
                        fileProvider.refresh();
                        treeProvider.refresh();
                        const fileCount2 = await contextService.getFileCount();
                        statusBar.update(fileCount2);
                        return;
                    } else {
                        // Cancel or dismissed
                        return;
                    }
                } else {
                    // Clean or not editable or status check failed: standard confirmation
                    const confirm = await vscode.window.showWarningMessage(
                        `Remove repository "${repoName}"?\n\nPath: ${dep.path}\n\nThis will unlink the repository and remove it from .vdwrc configuration.`,
                        { modal: true },
                        'Yes, Remove',
                        'Cancel'
                    );

                    if (confirm !== 'Yes, Remove') {
                        return;
                    }

                    statusBar.showBusy(`Removing ${repoName}...`);
                    log(`Removing repository: ${repoName} (${repoIdentifier})`);
                    if (!await fullRemoval()) return;
                }

                // Refresh UI
                vscode.window.showInformationMessage(`Repository "${repoName}" removed successfully!`);
                configService.reload();
                contextService.refresh();
                fileProvider.refresh();
                treeProvider.refresh();
                const fileCount = await contextService.getFileCount();
                statusBar.update(fileCount);
            } catch (err) {
                logError(`Remove repository ${repoName} failed`, err);
                vscode.window.showErrorMessage(`Error removing ${repoName}: ${err}`);
                statusBar.showError('Error');
            }
        })
    );

    // vdw.openSourceControl - Ensure the editable worktree is registered and show SCM.
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.openSourceControl', async (treeItem: any) => {
            const dependency = treeItem?.dependency as Dependency | undefined;
            if (!dependency?.path) return;

            const fullPath = resolveDependencyPath(dependency);
            if (!fs.existsSync(fullPath)) {
                const link = await vscode.window.showWarningMessage(
                    `Repository path is missing: ${dependency.path}`,
                    'Link Repository',
                );
                if (link === 'Link Repository') {
                    await vscode.commands.executeCommand('vdw.linkRepo', treeItem);
                }
                return;
            }

            try {
                const identifier = dependency.remote || dependency.origin || dependency.local || dependency.path;
                await autoOpenService?.openAllEditableRepositories(identifier);
                await vscode.commands.executeCommand('git.openRepository', fullPath);
                await vscode.commands.executeCommand('workbench.view.scm');
                await workspaceStatusService.refresh();
            } catch (error) {
                logError(`Failed to open Source Control for ${dependency.path}`, error);
                vscode.window.showErrorMessage(`Failed to open Source Control: ${error}`);
            }
        })
    );

    // vdw.revealInExplorer - Reveal the repository root in the native Explorer.
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.revealInExplorer', async (treeItem: any) => {
            const dependency = treeItem?.dependency as Dependency | undefined;
            if (!dependency?.path) return;

            const fullPath = resolveDependencyPath(dependency);
            if (!fs.existsSync(fullPath)) {
                vscode.window.showWarningMessage(`Repository path is missing: ${dependency.path}`);
                return;
            }
            await vscode.commands.executeCommand('revealInExplorer', vscode.Uri.file(fullPath));
        })
    );

    // vdw.openInTerminal - Open dependency path in terminal
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.openInTerminal', (treeItem: any) => {
            const dependency = treeItem?.dependency as Dependency | undefined;
            if (!dependency?.path) return;

            const fullPath = resolveDependencyPath(dependency);
            if (!fs.existsSync(fullPath)) {
                vscode.window.showWarningMessage(`Repository path is missing: ${dependency.path}`);
                return;
            }
            const repoName = getDependencyName(dependency);
            const terminal = vscode.window.createTerminal({ cwd: fullPath, name: repoName });
            terminal.show();
        })
    );

    // vdw.openWorkspaceSwitcher - Keyboard-first repository navigation.
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.openWorkspaceSwitcher', async () => {
            const config = configService.getConfig();
            if (!config || config.dependencies.length === 0) {
                const action = await vscode.window.showInformationMessage(
                    'No VDW repositories are configured.',
                    'Add Repository',
                );
                if (action === 'Add Repository') {
                    await vscode.commands.executeCommand('vdw.addRepo');
                }
                return;
            }

            const snapshot = await workspaceStatusService.refresh();
            const items: RepositoryQuickPickItem[] = config.dependencies
                .map(dependency => {
                    const status = snapshot.repositories.find(candidate =>
                        candidate.dependencyPath === dependency.path,
                    );
                    return {
                        label: `${getQuickPickIcon(status)} ${getDependencyName(dependency)}`,
                        description: status
                            ? formatRepositoryDescription(dependency, status)
                            : 'checking…',
                        detail: resolveDependencyPath(dependency),
                        dependency,
                        status,
                    };
                })
                .sort((left, right) => {
                    if (!left.status || !right.status) return 0;
                    const priority = repositoryStatusPriority(left.status) - repositoryStatusPriority(right.status);
                    return priority || getDependencyName(left.dependency).localeCompare(getDependencyName(right.dependency));
                });

            const selected = await vscode.window.showQuickPick(items, {
                placeHolder: 'Select a VDW repository',
                matchOnDescription: true,
                matchOnDetail: true,
            });
            if (!selected) return;

            const treeItem = { dependency: selected.dependency, status: selected.status };
            if (selected.status?.state === 'missing') {
                await vscode.commands.executeCommand('vdw.linkRepo', treeItem);
            } else if (selected.status?.state === 'invalidWorktree') {
                await vscode.commands.executeCommand('vdw.relink');
            } else if (selected.dependency.editable) {
                await vscode.commands.executeCommand('vdw.openSourceControl', treeItem);
            } else {
                await vscode.commands.executeCommand('vdw.revealInExplorer', treeItem);
            }
        })
    );

    // vdw.refreshContext
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.refreshContext', async () => {
            try {
                configService.reload();
                contextService.refresh();
                fileProvider.refresh();
                treeProvider.refresh();
                await autoOpenService?.openAllEditableRepositories();
                const snapshot = await workspaceStatusService.refresh();
                statusBar.update();

                vscode.window.showInformationMessage(
                    `Workspace status refreshed: ${snapshot.repositories.length} repositories`,
                );
            } catch (err) {
                logError('Refresh context failed', err);
                vscode.window.showErrorMessage(`Failed to refresh context: ${err}`);
            }
        })
    );
}

interface RepositoryQuickPickItem extends vscode.QuickPickItem {
    dependency: Dependency;
    status?: RepositoryRuntimeStatus;
}

function resolveDependencyPath(dependency: Dependency): string {
    const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
    if (!workspaceRoot) throw new Error('No workspace folder found');
    return path.resolve(workspaceRoot, dependency.path);
}

function getQuickPickIcon(status?: RepositoryRuntimeStatus): string {
    switch (status?.state) {
        case 'missing':
        case 'invalidWorktree':
            return '$(error)';
        case 'scmMissing':
        case 'branchDrift':
            return '$(warning)';
        case 'dirty':
            return '$(git-commit)';
        case 'ready':
            return '$(repo)';
        default:
            return '$(sync~spin)';
    }
}
