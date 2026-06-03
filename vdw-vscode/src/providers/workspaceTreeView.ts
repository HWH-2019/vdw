import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { VDWConfig, Dependency } from '../types/vdwTypes';
import { ConfigService } from '../services/configService';
import { VDWCliService } from '../services/vdwCliService';

export class WorkspaceTreeProvider implements vscode.TreeDataProvider<WorkspaceTreeItem> {
    private _onDidChangeTreeData = new vscode.EventEmitter<WorkspaceTreeItem | undefined | null | void>();
    readonly onDidChangeTreeData = this._onDidChangeTreeData.event;

    private configService: ConfigService;
    private cliService: VDWCliService;
    private linkedRepos: Set<string> = new Set(); // Track which repos are linked

    constructor(configService: ConfigService, cliService: VDWCliService) {
        this.configService = configService;
        this.cliService = cliService;
        // Initialize linked repos on construction
        this.updateLinkedRepos();
    }

    refresh(): void {
        this.updateLinkedRepos();
        this._onDidChangeTreeData.fire();
    }

    /**
     * Update the set of linked repositories by checking if their paths exist
     */
    private updateLinkedRepos(): void {
        this.linkedRepos.clear();
        
        const config = this.configService.getConfig();
        if (!config) {
            return;
        }
        
        const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
        if (!workspaceFolder) {
            return;
        }
        
        const workspaceRoot = workspaceFolder.uri.fsPath;
        
        for (const dep of config.dependencies) {
            // Check if the path exists in the workspace
            const fullPath = path.join(workspaceRoot, dep.path);
            if (fs.existsSync(fullPath)) {
                // Additional check: see if it looks like a linked repo (has files)
                try {
                    const stats = fs.statSync(fullPath);
                    if (stats.isDirectory()) {
                        const files = fs.readdirSync(fullPath);
                        if (files.length > 0) {
                            this.linkedRepos.add(dep.path);
                        }
                    }
                } catch (error) {
                    // If we can't read it, assume it's not linked
                }
            }
        }
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

        // Show welcome message when no dependencies
        if (config.dependencies.length === 0) {
            return [new WorkspaceTreeItem(
                'No dependencies configured',
                vscode.TreeItemCollapsibleState.None,
                'welcome',
                undefined,
                'Click the + button above to add your first repository'
            )];
        }

        if (!element) {
            // Root level: show each dependency
            return config.dependencies.map(dep => {
                const repoName = dep.repo.split('/').pop()?.replace('.git', '') || dep.repo;
                const version = dep.branch || dep.tag || dep.commit?.substring(0, 7) || 'main';
                const label = `${repoName}@${version}`;
                
                // Determine link status
                const isLinked = this.linkedRepos.has(dep.path);
                const statusDesc = isLinked ? '✅ Linked' : '⚪ Not linked';
                const editableDesc = dep.editable ? '[可编辑]' : '[只读]';

                const item = new WorkspaceTreeItem(
                    label,
                    vscode.TreeItemCollapsibleState.Collapsed,
                    isLinked ? 'linkedRepo' : 'unlinkedRepo',
                    dep,
                    `${editableDesc} ${statusDesc}`
                );
                                
                // Set icon based on link status (both show link icon)
                if (isLinked) {
                    item.iconPath = new vscode.ThemeIcon('link', new vscode.ThemeColor('terminal.ansiGreen'));
                } else {
                    item.iconPath = new vscode.ThemeIcon('link');
                }
                                
                // Enhanced tooltip
                item.tooltip = new vscode.MarkdownString()
                    .appendMarkdown(`**${repoName}**\n\n`)
                    .appendMarkdown(`🔗 **URL**: ${dep.repo}\n\n`)
                    .appendMarkdown(` **Path**: ${dep.path}\n\n`)
                    .appendMarkdown(`🌿 **Branch**: ${version}\n\n`)
                    .appendMarkdown(`📝 **Scope**: ${dep.scope.include.join(', ')}\n\n`)
                    .appendMarkdown(`${dep.editable ? '️ **Editable** (worktree)' : ' **Read-only'} (linked)`) 
                    .appendMarkdown(`\n\n---\n\n${isLinked ? '✅ **Status**: Linked' : '⚪ **Status**: Not linked (run `vdw link`)'}`);
                                
                return item;
            });
        }

        if ((element.contextType === 'repo' || element.contextType === 'linkedRepo' || element.contextType === 'unlinkedRepo') && element.dependency) {
            // Show dependency details
            const dep = element.dependency;
            const children: WorkspaceTreeItem[] = [];

            // Path
            const pathItem = new WorkspaceTreeItem(
                `${dep.path}`,
                vscode.TreeItemCollapsibleState.None,
                'path',
                undefined,
                '路径'
            );
            pathItem.iconPath = new vscode.ThemeIcon('folder');
            children.push(pathItem);

            // Scope includes
            if (dep.scope.include.length > 0) {
                const scopeItem = new WorkspaceTreeItem(
                    `${dep.scope.include.join(', ')}`,
                    vscode.TreeItemCollapsibleState.None,
                    'scope',
                    undefined,
                    'Scope'
                );
                scopeItem.iconPath = new vscode.ThemeIcon('filter');
                children.push(scopeItem);
            }

            // Scope excludes
            if (dep.scope.exclude.length > 0) {
                const excludeItem = new WorkspaceTreeItem(
                    `${dep.scope.exclude.join(', ')}`,
                    vscode.TreeItemCollapsibleState.None,
                    'exclude',
                    undefined,
                    'Exclude'
                );
                excludeItem.iconPath = new vscode.ThemeIcon('circle-slash');
                children.push(excludeItem);
            }

            // Status
            const isLinked = this.linkedRepos.has(dep.path);
            const statusLabel = isLinked ? '已链接' : '未链接';
            const statusIcon = isLinked ? 'check' : 'warning';
            const statusDesc = dep.editable ? '可编辑 (worktree)' : '只读 (linked)';
            const statusItem = new WorkspaceTreeItem(
                statusLabel,
                vscode.TreeItemCollapsibleState.None,
                'status',
                undefined,
                statusDesc
            );
            statusItem.iconPath = new vscode.ThemeIcon(
                statusIcon, 
                isLinked ? new vscode.ThemeColor('terminal.ansiGreen') : new vscode.ThemeColor('terminal.ansiYellow')
            );
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
        public readonly description?: string,
    ) {
        super(label, collapsibleState);

        // Set contextValue for menu actions (this is what VS Code uses for when clauses)
        this.contextValue = contextType;

        // Set description if provided
        if (description) {
            this.description = description;
        }

        // Set icon based on type
        if (contextType === 'repo' || contextType === 'linkedRepo' || contextType === 'unlinkedRepo') {
            this.iconPath = new vscode.ThemeIcon('repo');
        } else if (contextType === 'info') {
            this.iconPath = new vscode.ThemeIcon('info');
        } else if (contextType === 'welcome') {
            this.iconPath = new vscode.ThemeIcon('info');
        }

        // Set tooltip
        if (dependency) {
            this.tooltip = `${dependency.repo}\nBranch: ${dependency.branch || dependency.tag || dependency.commit || 'main'}\nEditable: ${dependency.editable}`;
        }
    }
}
