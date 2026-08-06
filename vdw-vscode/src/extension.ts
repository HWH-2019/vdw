import * as vscode from 'vscode';
import { VDWCliService } from './services/vdwCliService';
import { ConfigService } from './services/configService';
import { ContextService } from './services/contextService';
import { VDWFileProvider } from './providers/fileProvider';
import { WorkspaceTreeProvider } from './providers/workspaceTreeView';
import { StatusBarManager } from './providers/statusBar';
import { registerCommands } from './commands/index';
import { AutoOpenService } from './services/autoOpenService';
import { log, logError, dispose as disposeLogger } from './utils/logger';

let fileProvider: VDWFileProvider | undefined;
let treeProvider: WorkspaceTreeProvider | undefined;
let statusBar: StatusBarManager | undefined;
let contextService: ContextService | undefined;
let autoOpenService: AutoOpenService | undefined;

export function activate(context: vscode.ExtensionContext) {
    log('VDW extension activating...');

    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
    if (!workspaceFolder) {
        log('No workspace folder found, VDW features disabled');
        return;
    }

    const workspacePath = workspaceFolder.uri.fsPath;

    // 1. Initialize services
    const cliService = new VDWCliService(workspacePath);
    const configService = new ConfigService(workspacePath);
    const hasConfig = configService.loadConfig() !== null;
    void vscode.commands.executeCommand('setContext', 'vdw.hasConfig', hasConfig);

    if (!hasConfig) {
        log('No valid VDW configuration found, showing initialization guide');

        // Minimal init: TreeView with init guide + commands
        treeProvider = new WorkspaceTreeProvider(configService, cliService);
        const treeView = vscode.window.createTreeView('vdwWorkspace', {
            treeDataProvider: treeProvider,
            showCollapseAll: true,
        });
        context.subscriptions.push(treeView);

        // Create a minimal StatusBar so vdw.init can use it
        statusBar = new StatusBarManager(configService);
        statusBar.update();

        // Register only vdw.init so the init guide button works
        context.subscriptions.push(
            vscode.commands.registerCommand('vdw.init', async () => {
                try {
                    statusBar!.showBusy('Initializing...');
                    const result = await cliService.init();
                    if (result.exitCode === 0) {
                        vscode.window.showInformationMessage('VDW workspace initialized successfully!');
                        const config = configService.reload();
                        void vscode.commands.executeCommand('setContext', 'vdw.hasConfig', config !== null);
                        treeProvider!.refresh();
                        statusBar!.update();
                        const wsFolder = vscode.workspace.workspaceFolders?.[0];
                        if (wsFolder) {
                            const vdwrcUri = vscode.Uri.joinPath(wsFolder.uri, '.vdwrc');
                            const doc = await vscode.workspace.openTextDocument(vdwrcUri);
                            await vscode.window.showTextDocument(doc);
                        }
                    } else {
                        vscode.window.showErrorMessage(`VDW init failed: ${result.stderr || result.stdout}`);
                        statusBar!.showError('Init failed');
                    }
                } catch (err) {
                    logError('Init command failed', err);
                    vscode.window.showErrorMessage(`VDW init error: ${err}`);
                    statusBar!.showError('Error');
                }
            })
        );

        // Watch for either supported config filename to trigger full initialization.
        const initWatcher = vscode.workspace.createFileSystemWatcher('**/.vdwrc{,.yml}');
        initWatcher.onDidCreate(() => {
            log('VDW configuration created, reloading window...');
            const config = configService.reload();
            void vscode.commands.executeCommand('setContext', 'vdw.hasConfig', config !== null);
            vscode.commands.executeCommand('workbench.action.reloadWindow');
        });
        context.subscriptions.push(initWatcher);
        return;
    }

    // 2. Initialize FileProvider
    fileProvider = new VDWFileProvider(configService, workspacePath);
    context.subscriptions.push(
        vscode.workspace.registerTextDocumentContentProvider('vdw', fileProvider)
    );

    // 3. Initialize TreeView
    treeProvider = new WorkspaceTreeProvider(configService, cliService);
    const treeView = vscode.window.createTreeView('vdwWorkspace', {
        treeDataProvider: treeProvider,
        showCollapseAll: true,
    });
    context.subscriptions.push(treeView);

    // 4. Initialize StatusBar
    statusBar = new StatusBarManager(configService);
    statusBar.update();

    // 5. Initialize AutoOpenService
    autoOpenService = new AutoOpenService(configService);

    // 6. Initialize Context Service
    contextService = new ContextService(configService, fileProvider, workspacePath);

    // 7. Register commands
    registerCommands(
        context,
        cliService,
        configService,
        contextService,
        fileProvider,
        treeProvider,
        statusBar,
        autoOpenService,
    );

    // 8. Watch supported VDW configuration filenames for changes
    const watcher = vscode.workspace.createFileSystemWatcher('**/.vdwrc{,.yml}');
    watcher.onDidChange(() => {
        log('VDW configuration changed, reloading...');
        const config = configService.reload();
        void vscode.commands.executeCommand('setContext', 'vdw.hasConfig', config !== null);
        contextService?.refresh();
        fileProvider?.refresh();
        treeProvider?.refresh();
        statusBar?.update();
        void autoOpenService?.openAllEditableRepositories();
    });
    watcher.onDidCreate(() => {
        log('VDW configuration created, loading...');
        const config = configService.reload();
        void vscode.commands.executeCommand('setContext', 'vdw.hasConfig', config !== null);
        treeProvider?.refresh();
        statusBar?.update();
        void autoOpenService?.openAllEditableRepositories();
    });
    watcher.onDidDelete(() => {
        log('VDW configuration deleted');
        const config = configService.reload();
        void vscode.commands.executeCommand('setContext', 'vdw.hasConfig', config !== null);
        treeProvider?.refresh();
        statusBar?.update();
    });
    context.subscriptions.push(watcher);

    // 9. Initial context update
    contextService.getFileCount().then(count => {
        statusBar?.update(count);
    });

    // 10. Auto-open and refresh exact VDW-managed worktrees on activation
    (async () => {
        await autoOpenService.openAllEditableRepositories();
        log('Auto-open Git repositories completed');
    })();

    log('VDW extension activated successfully');
}

export function deactivate() {
    log('VDW extension deactivating...');
    statusBar?.dispose();
    fileProvider?.dispose();
    treeProvider?.dispose();
    contextService?.dispose();
    disposeLogger();
}
