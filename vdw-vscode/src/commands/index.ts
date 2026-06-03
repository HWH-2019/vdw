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
import { log, logError, show as showLog } from '../utils/logger';
import { handleAddRepo } from './addRepo';

export function registerCommands(
    context: vscode.ExtensionContext,
    cliService: VDWCliService,
    configService: ConfigService,
    contextService: ContextService,
    fileProvider: VDWFileProvider,
    treeProvider: WorkspaceTreeProvider,
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
            try {
                statusBar.showBusy('Unlinking...');
                const result = await cliService.unlink();

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage('Dependencies unlinked successfully!');
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    statusBar.update(0);
                } else {
                    vscode.window.showErrorMessage(`VDW unlink failed: ${result.stderr || result.stdout}`);
                    statusBar.showError('Unlink failed');
                }
            } catch (err) {
                logError('Unlink command failed', err);
                vscode.window.showErrorMessage(`VDW unlink error: ${err}`);
                statusBar.showError('Error');
            }
        })
    );

    // vdw.relink - Unlink then link with updated config
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.relink', async () => {
            try {
                const proceed = await vscode.window.showWarningMessage(
                    'This will unlink all dependencies and re-link them based on the current configuration. Continue?',
                    { modal: true },
                    'Yes, Relink',
                    'Cancel'
                );
                
                if (proceed !== 'Yes, Relink') {
                    return;
                }
                
                statusBar.showBusy('Relinking...');
                log('Running vdw relink (unlink + link)...');
                
                // Step 1: Unlink
                const unlinkResult = await cliService.unlink();
                if (unlinkResult.exitCode !== 0) {
                    vscode.window.showErrorMessage(`Unlink failed: ${unlinkResult.stderr || unlinkResult.stdout}`);
                    statusBar.showError('Relink failed');
                    return;
                }
                
                // Step 2: Link
                const linkResult = await cliService.link();
                if (linkResult.exitCode === 0) {
                    vscode.window.showInformationMessage('Dependencies relinked successfully with updated config!');
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                    
                    // Auto-open Git repositories for editable dependencies
                    await autoOpenService?.openAllEditableRepositories();
                } else {
                    vscode.window.showErrorMessage(`Link failed: ${linkResult.stderr || linkResult.stdout}`);
                    statusBar.showError('Relink failed');
                }
            } catch (err) {
                logError('Relink command failed', err);
                vscode.window.showErrorMessage(`VDW relink error: ${err}`);
                statusBar.showError('Error');
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
            const repoUrl = dep.repo;
            const repoName = repoUrl.split('/').pop()?.replace('.git', '') || repoUrl;

            const proceed = await vscode.window.showWarningMessage(
                `Unlink repository "${repoName}"?\n\nPath: ${dep.path}\n\nThis will remove the linked files from the workspace.`,
                { modal: true },
                'Yes, Unlink',
                'Cancel'
            );

            if (proceed !== 'Yes, Unlink') {
                return;
            }

            try {
                statusBar.showBusy(`Unlinking ${repoName}...`);
                log(`Unlinking repository: ${repoName} (${repoUrl})`);

                // Execute per-repo unlink
                const result = await cliService.unlinkRepo(repoUrl);

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage(`Repository "${repoName}" unlinked successfully!`);
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                } else {
                    vscode.window.showErrorMessage(`Failed to unlink ${repoName}: ${result.stderr || result.stdout}`);
                    statusBar.showError('Unlink failed');
                }
            } catch (err) {
                logError(`Unlink repository ${repoName} failed`, err);
                vscode.window.showErrorMessage(`Error unlinking ${repoName}: ${err}`);
                statusBar.showError('Error');
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
            const repoUrl = dep.repo;
            const repoName = repoUrl.split('/').pop()?.replace('.git', '') || repoUrl;

            try {
                statusBar.showBusy(`Linking ${repoName}...`);
                log(`Linking repository: ${repoName} (${repoUrl})`);

                const result = await cliService.linkRepo(repoUrl);

                if (result.exitCode === 0) {
                    vscode.window.showInformationMessage(`Repository "${repoName}" linked successfully!`);
                    configService.reload();
                    contextService.refresh();
                    fileProvider.refresh();
                    treeProvider.refresh();
                    const fileCount = await contextService.getFileCount();
                    statusBar.update(fileCount);
                    
                    // Auto-open Git repository for this editable dependency
                    await autoOpenService?.openAllEditableRepositories();
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

    // vdw.refreshContext
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.refreshContext', async () => {
            try {
                configService.reload();
                contextService.refresh();
                fileProvider.refresh();
                treeProvider.refresh();
                const fileCount = await contextService.getFileCount();
                statusBar.update(fileCount);
                vscode.window.showInformationMessage(`Context refreshed: ${fileCount} files in scope`);
            } catch (err) {
                logError('Refresh context failed', err);
                vscode.window.showErrorMessage(`Failed to refresh context: ${err}`);
            }
        })
    );
}
