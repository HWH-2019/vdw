import * as vscode from 'vscode';
import { VDWCliService } from './services/vdwCliService';
import { ConfigService } from './services/configService';
import { ContextService } from './services/contextService';
import { VDWFileProvider } from './providers/fileProvider';
import { WorkspaceTreeProvider } from './providers/workspaceTreeView';
import { StatusBarManager } from './providers/statusBar';
import { registerCommands } from './commands/index';
import { AutoOpenService } from './services/autoOpenService';
import { log, dispose as disposeLogger } from './utils/logger';

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
    configService.loadConfig();

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

    // 8. Watch .vdwrc for changes
    const watcher = vscode.workspace.createFileSystemWatcher('**/.vdwrc');
    watcher.onDidChange(() => {
        log('.vdwrc changed, reloading...');
        configService.reload();
        contextService?.refresh();
        fileProvider?.refresh();
        treeProvider?.refresh();
        statusBar?.update();
    });
    watcher.onDidCreate(() => {
        log('.vdwrc created, loading...');
        configService.reload();
        treeProvider?.refresh();
        statusBar?.update();
    });
    watcher.onDidDelete(() => {
        log('.vdwrc deleted');
        configService.reload();
        treeProvider?.refresh();
        statusBar?.update();
    });
    context.subscriptions.push(watcher);

    // 9. Initial context update
    contextService.getFileCount().then(count => {
        statusBar?.update(count);
    });

    // 10. Auto-open editable Git repositories on extension activation
    autoOpenService.openAllEditableRepositories().then(() => {
        log('Auto-open Git repositories completed');
    });

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
