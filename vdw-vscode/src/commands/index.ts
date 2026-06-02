import * as vscode from 'vscode';
import * as path from 'path';
import * as fs from 'fs';
import { VDWCliService } from '../services/vdwCliService';
import { ConfigService } from '../services/configService';
import { ContextService } from '../services/contextService';
import { VDWFileProvider } from '../providers/fileProvider';
import { WorkspaceTreeProvider } from '../providers/workspaceTreeView';
import { StatusBarManager } from '../providers/statusBar';
import { log, logError, show as showLog } from '../utils/logger';

export function registerCommands(
    context: vscode.ExtensionContext,
    cliService: VDWCliService,
    configService: ConfigService,
    contextService: ContextService,
    fileProvider: VDWFileProvider,
    treeProvider: WorkspaceTreeProvider,
    statusBar: StatusBarManager,
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

    // vdw.addRepo
    context.subscriptions.push(
        vscode.commands.registerCommand('vdw.addRepo', async () => {
            try {
                // Get repo URL
                const url = await vscode.window.showInputBox({
                    prompt: 'Repository URL',
                    placeHolder: 'https://github.com/org/repo.git',
                });
                if (!url) { return; }

                // Get local path
                const localPath = await vscode.window.showInputBox({
                    prompt: 'Local path (relative to workspace)',
                    placeHolder: './libs/my-lib',
                });
                if (!localPath) { return; }

                // Get branch
                const branch = await vscode.window.showInputBox({
                    prompt: 'Branch name (leave empty for main)',
                    placeHolder: 'main',
                });

                // Append to .vdwrc
                const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
                if (!workspaceFolder) {
                    vscode.window.showErrorMessage('No workspace folder open');
                    return;
                }

                const vdwrcPath = path.join(workspaceFolder.uri.fsPath, '.vdwrc');
                if (!fs.existsSync(vdwrcPath)) {
                    vscode.window.showErrorMessage('.vdwrc not found. Run "VDW: Initialize Workspace" first.');
                    return;
                }

                // Read current config and append new dependency
                let content = fs.readFileSync(vdwrcPath, 'utf-8');
                const newDep = `\n  - repo: "${url}"\n    path: ${localPath}\n    branch: ${branch || 'main'}\n    scope:\n      include:\n        - "src/**"\n    editable: false\n`;
                content += newDep;
                fs.writeFileSync(vdwrcPath, content);

                vscode.window.showInformationMessage(`Added dependency: ${url}`);
                configService.reload();
                treeProvider.refresh();
                statusBar.update();
            } catch (err) {
                logError('Add repo command failed', err);
                vscode.window.showErrorMessage(`Failed to add repository: ${err}`);
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
