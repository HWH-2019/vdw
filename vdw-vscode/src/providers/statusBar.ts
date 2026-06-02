import * as vscode from 'vscode';
import { ConfigService } from '../services/configService';

export class StatusBarManager {
    private statusBarItem: vscode.StatusBarItem;
    private configService: ConfigService;

    constructor(configService: ConfigService) {
        this.configService = configService;
        this.statusBarItem = vscode.window.createStatusBarItem(
            vscode.StatusBarAlignment.Left,
            100
        );
        this.statusBarItem.command = 'workbench.action.quickOpen';
        this.statusBarItem.tooltip = 'VDW Workspace - Click to open command palette';
    }

    /** Update the status bar with current state */
    update(fileCount?: number): void {
        const config = this.configService.getConfig();

        if (!config) {
            this.statusBarItem.text = '$(link) VDW: No config';
            this.statusBarItem.backgroundColor = undefined;
            this.statusBarItem.show();
            return;
        }

        const depCount = config.dependencies.length;
        const filesText = fileCount !== undefined ? `, ${fileCount} files` : '';
        this.statusBarItem.text = `$(link) VDW: ${depCount} deps${filesText}`;
        this.statusBarItem.backgroundColor = undefined;
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
        this.statusBarItem.dispose();
    }
}
