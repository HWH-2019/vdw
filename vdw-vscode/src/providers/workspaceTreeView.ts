import * as vscode from 'vscode';
import { VDWConfig, Dependency } from '../types/vdwTypes';
import { ConfigService } from '../services/configService';

export class WorkspaceTreeProvider implements vscode.TreeDataProvider<WorkspaceTreeItem> {
    private _onDidChangeTreeData = new vscode.EventEmitter<WorkspaceTreeItem | undefined | null | void>();
    readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

    private configService: ConfigService;

    constructor(configService: ConfigService) {
        this.configService = configService;
    }

    refresh(): void {
        this._onDidChangeTreeData.fire();
    }

    getTreeItem(element: WorkspaceTreeItem): vscode.TreeItem {
        return element;
    }

    getChildren(element?: WorkspaceTreeItem): WorkspaceTreeItem[] {
        const config = this.configService.getConfig();
        if (!config) {
            return [new WorkspaceTreeItem(
                'No .vdwrc found',
                vscode.TreeItemCollapsibleState.None,
                'info'
            )];
        }

        if (!element) {
            // Root level: show each dependency
            return config.dependencies.map(dep => {
                const repoName = dep.repo.split('/').pop()?.replace('.git', '') || dep.repo;
                const version = dep.branch || dep.tag || dep.commit?.substring(0, 7) || 'main';
                const label = `${repoName}@${version}`;

                const item = new WorkspaceTreeItem(
                    label,
                    vscode.TreeItemCollapsibleState.Collapsed,
                    'repo',
                    dep
                );
                item.description = dep.editable ? 'editable' : 'read-only';
                return item;
            });
        }

        if (element.contextType === 'repo' && element.dependency) {
            // Show dependency details
            const dep = element.dependency;
            const children: WorkspaceTreeItem[] = [];

            // Path
            const pathItem = new WorkspaceTreeItem(
                `Path: ${dep.path}`,
                vscode.TreeItemCollapsibleState.None,
                'path'
            );
            pathItem.iconPath = new vscode.ThemeIcon('folder');
            children.push(pathItem);

            // Scope includes
            if (dep.scope.include.length > 0) {
                const scopeItem = new WorkspaceTreeItem(
                    `Include: ${dep.scope.include.join(', ')}`,
                    vscode.TreeItemCollapsibleState.None,
                    'scope'
                );
                scopeItem.iconPath = new vscode.ThemeIcon('filter');
                children.push(scopeItem);
            }

            // Scope excludes
            if (dep.scope.exclude.length > 0) {
                const excludeItem = new WorkspaceTreeItem(
                    `Exclude: ${dep.scope.exclude.join(', ')}`,
                    vscode.TreeItemCollapsibleState.None,
                    'exclude'
                );
                excludeItem.iconPath = new vscode.ThemeIcon('circle-slash');
                children.push(excludeItem);
            }

            // Status
            const statusLabel = dep.editable ? 'Editable (worktree)' : 'Read-only (linked)';
            const statusItem = new WorkspaceTreeItem(
                statusLabel,
                vscode.TreeItemCollapsibleState.None,
                'status'
            );
            statusItem.iconPath = new vscode.ThemeIcon(dep.editable ? 'unlock' : 'lock');
            children.push(statusItem);

            return children;
        }

        return [];
    }

    dispose(): void {
        this._onDidChangeTreeData.dispose();
    }
}

export class WorkspaceTreeItem extends vscode.TreeItem {
    constructor(
        public readonly label: string,
        public readonly collapsibleState: vscode.TreeItemCollapsibleState,
        public readonly contextType: string,
        public readonly dependency?: Dependency,
    ) {
        super(label, collapsibleState);

        // Set icon based on type
        if (contextType === 'repo') {
            this.iconPath = new vscode.ThemeIcon('repo');
        } else if (contextType === 'info') {
            this.iconPath = new vscode.ThemeIcon('info');
        }

        // Set tooltip
        if (dependency) {
            this.tooltip = `${dependency.repo}\nBranch: ${dependency.branch || dependency.tag || dependency.commit || 'main'}\nEditable: ${dependency.editable}`;
        }
    }
}
