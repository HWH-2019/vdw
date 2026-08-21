import * as vscode from 'vscode';
import { ConfigService } from '../services/configService';
import { WorkspaceStatusService } from '../services/workspaceStatusService';

export class StatusBarManager {
    private statusBarItem: vscode.StatusBarItem;
    private statusSubscription: vscode.Disposable;

    constructor(
        private readonly configService: ConfigService,
        private readonly statusService: WorkspaceStatusService,
    ) {
        this.statusBarItem = vscode.window.createStatusBarItem(
            vscode.StatusBarAlignment.Left,
            100
        );
        this.statusSubscription = this.statusService.onDidChange(() => this.update());
    }

    /** Update the status bar with current state */
    update(_fileCount?: number): void {
        const config = this.configService.getConfig();

        if (!config) {
            this.statusBarItem.text = '$(link) VDW: No config';
            this.statusBarItem.backgroundColor = undefined;
            this.statusBarItem.command = 'vdw.init';
            this.statusBarItem.tooltip = 'Initialize a VDW workspace';
            this.statusBarItem.show();
            return;
        }

        const depCount = config.dependencies.length;
        const snapshot = this.statusService.getSnapshot();
        this.statusBarItem.command = 'vdw.openWorkspaceSwitcher';

        if (snapshot.issueCount > 0) {
            this.statusBarItem.text = `$(warning) VDW: ${snapshot.issueCount} issue${snapshot.issueCount === 1 ? '' : 's'}`;
            this.statusBarItem.backgroundColor = new vscode.ThemeColor('statusBarItem.warningBackground');
            this.statusBarItem.tooltip = `Open VDW repositories — ${snapshot.issueCount} workspace issue${snapshot.issueCount === 1 ? '' : 's'}`;
        } else if (snapshot.dirtyCount > 0) {
            this.statusBarItem.text = `$(git-commit) VDW: ${snapshot.dirtyCount} dirty`;
            this.statusBarItem.backgroundColor = undefined;
            this.statusBarItem.tooltip = `Open VDW repositories — ${snapshot.dirtyCount} repositor${snapshot.dirtyCount === 1 ? 'y' : 'ies'} with changes`;
        } else {
            this.statusBarItem.text = `$(repo) VDW: ${depCount} repo${depCount === 1 ? '' : 's'}`;
            this.statusBarItem.backgroundColor = undefined;
            this.statusBarItem.tooltip = 'Open VDW repositories';
        }
        this.statusBarItem.show();
    }

    /** Show error state */
    showError(message: string): void {
        this.statusBarItem.text = `$(warning) VDW: ${message}`;
        this.statusBarItem.backgroundColor = new vscode.ThemeColor('statusBarItem.errorBackground');
        this.statusBarItem.show();
    }

    /** Show busy/loading state */
    showBusy(message: string): void {
        this.statusBarItem.text = `$(sync~spin) VDW: ${message}`;
        this.statusBarItem.show();
    }

    /** Hide the status bar item */
    hide(): void {
        this.statusBarItem.hide();
    }

    dispose(): void {
        this.statusSubscription.dispose();
        this.statusBarItem.dispose();
    }
}
