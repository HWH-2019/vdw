import * as path from 'path';
import * as vscode from 'vscode';
import { ConfigService } from '../services/configService';
import { WorkspaceStatusService } from '../services/workspaceStatusService';
import { Dependency, RepositoryRuntimeStatus } from '../types/vdwTypes';
import {
    formatRepositoryDescription,
    getDependencyName,
    getDesiredRevision,
    isDefaultScope,
    repositoryStatusPriority,
} from '../utils/workspaceStatus';

export class WorkspaceTreeProvider implements vscode.TreeDataProvider<WorkspaceTreeItem>, vscode.Disposable {
    private readonly _onDidChangeTreeData = new vscode.EventEmitter<WorkspaceTreeItem | undefined | null | void>();
    readonly onDidChangeTreeData = this._onDidChangeTreeData.event;
    private readonly statusSubscription: vscode.Disposable;

    constructor(
        private readonly configService: ConfigService,
        private readonly statusService: WorkspaceStatusService,
        private readonly workspaceRoot: string,
    ) {
        this.statusSubscription = this.statusService.onDidChange(() => {
            this._onDidChangeTreeData.fire();
        });
    }

    refresh(): void {
        this._onDidChangeTreeData.fire();
        void this.statusService.refresh({ forceTopology: true });
    }

    getTreeItem(element: WorkspaceTreeItem): vscode.TreeItem {
        return element;
    }

    getChildren(element?: WorkspaceTreeItem): WorkspaceTreeItem[] {
        if (element) return [];

        const config = this.configService.getConfig();
        if (!config) {
            const initItem = new WorkspaceTreeItem(
                'Initialize VDW Workspace',
                'vdwInitGuide',
            );
            initItem.description = 'Create .vdwrc';
            initItem.iconPath = new vscode.ThemeIcon('play-circle');
            initItem.command = {
                command: 'vdw.init',
                title: 'Initialize VDW Workspace',
            };
            return [initItem];
        }

        if (config.dependencies.length === 0) {
            const welcome = new WorkspaceTreeItem('No repositories configured', 'vdwWelcome');
            welcome.description = 'Use + to add one';
            welcome.iconPath = new vscode.ThemeIcon('info');
            return [welcome];
        }

        return config.dependencies
            .map(dependency => {
                const status = this.statusService.getRepositoryStatus(dependency)
                    ?? createCheckingStatus(dependency, this.workspaceRoot);
                return createRepositoryItem(dependency, status);
            })
            .sort((left, right) => {
                const priority = repositoryStatusPriority(left.status!)
                    - repositoryStatusPriority(right.status!);
                return priority || String(left.label ?? '').localeCompare(String(right.label ?? ''));
            });
    }

    dispose(): void {
        this.statusSubscription.dispose();
        this._onDidChangeTreeData.dispose();
    }
}

export class WorkspaceTreeItem extends vscode.TreeItem {
    constructor(
        label: string,
        contextValue: string,
        public readonly dependency?: Dependency,
        public readonly status?: RepositoryRuntimeStatus,
    ) {
        super(label, vscode.TreeItemCollapsibleState.None);
        this.contextValue = contextValue;
    }
}

function createRepositoryItem(
    dependency: Dependency,
    status: RepositoryRuntimeStatus,
): WorkspaceTreeItem {
    const contextValue = getContextValue(dependency, status);
    const item = new WorkspaceTreeItem(
        getDependencyName(dependency),
        contextValue,
        dependency,
        status,
    );
    item.id = dependency.path;
    item.description = formatRepositoryDescription(dependency, status);
    item.iconPath = getStatusIcon(dependency, status);
    item.resourceUri = vscode.Uri.file(status.fullPath);
    item.tooltip = buildTooltip(dependency, status);
    item.accessibilityInformation = {
        label: `${getDependencyName(dependency)}, ${item.description}`,
    };

    item.command = getDefaultCommand(dependency, status, item);

    return item;
}

function getStatusIcon(
    dependency: Dependency,
    status: RepositoryRuntimeStatus,
): vscode.ThemeIcon {
    switch (status.state) {
        case 'checking':
            return new vscode.ThemeIcon('sync~spin');
        case 'missing':
        case 'invalidWorktree':
        case 'unowned':
            return new vscode.ThemeIcon('error', new vscode.ThemeColor('problemsErrorIcon.foreground'));
        case 'topologyConflict':
            return new vscode.ThemeIcon('diff', new vscode.ThemeColor('problemsErrorIcon.foreground'));
        case 'detachedLink':
        case 'scmMissing':
        case 'branchDrift':
            return new vscode.ThemeIcon('warning', new vscode.ThemeColor('problemsWarningIcon.foreground'));
        case 'dirty':
            return new vscode.ThemeIcon('git-commit', new vscode.ThemeColor('gitDecoration.modifiedResourceForeground'));
        case 'ready':
            return dependency.editable
                ? new vscode.ThemeIcon('repo')
                : new vscode.ThemeIcon('lock');
    }
}

function buildTooltip(
    dependency: Dependency,
    status: RepositoryRuntimeStatus,
): vscode.MarkdownString {
    const tooltip = new vscode.MarkdownString();
    tooltip.appendMarkdown(`**${getDependencyName(dependency)}**\n\n`);
    tooltip.appendMarkdown(`$(git-branch) Declared: \`${status.desiredRevision}\`\n\n`);
    if (status.currentBranch) {
        tooltip.appendMarkdown(`$(source-control) Current: \`${status.currentBranch}\`\n\n`);
    } else if (status.detachedCommit) {
        tooltip.appendMarkdown(`$(git-commit) Detached: \`${status.detachedCommit.substring(0, 12)}\`\n\n`);
    }
    tooltip.appendMarkdown(`$(folder) Path: \`${dependency.path}\`\n\n`);
    tooltip.appendMarkdown(`Mode: **${dependency.editable ? 'Editable worktree' : 'Read-only projection'}**\n\n`);
    tooltip.appendMarkdown(`Status: **${formatRepositoryDescription(dependency, status)}**`);
    if (status.topologyMessage) {
        tooltip.appendMarkdown(`\n\n$(warning) ${status.topologyMessage}`);
    }

    if (!isDefaultScope(dependency)) {
        tooltip.appendMarkdown(`\n\n$(filter) AI scope: \`${dependency.scope.include.join(', ')}\``);
        if (dependency.scope.exclude.length > 0) {
            tooltip.appendMarkdown(`\n\nExcluded: \`${dependency.scope.exclude.join(', ')}\``);
        }
    }
    const source = dependency.remote || dependency.origin || dependency.local;
    if (source) tooltip.appendMarkdown(`\n\nSource: \`${source}\``);
    return tooltip;
}

function createCheckingStatus(
    dependency: Dependency,
    workspaceRoot: string,
): RepositoryRuntimeStatus {
    return {
        dependencyPath: dependency.path,
        fullPath: path.resolve(workspaceRoot, dependency.path),
        state: 'checking',
        exists: false,
        scmRegistered: false,
        desiredRevision: getDesiredRevision(dependency),
        changeCount: 0,
        ahead: 0,
        behind: 0,
        trackedLinkCount: 0,
        conflictCount: 0,
        missingLinkCount: 0,
        detachedEquivalentCount: 0,
    };
}

function getContextValue(
    dependency: Dependency,
    status: RepositoryRuntimeStatus,
): string {
    switch (status.state) {
        case 'missing': return 'vdwRepoMissing';
        case 'invalidWorktree': return 'vdwRepoInvalid';
        case 'unowned': return 'vdwRepoUnowned';
        case 'topologyConflict': return 'vdwRepoConflict';
        case 'detachedLink': return 'vdwRepoDetached';
        default: return dependency.editable ? 'vdwRepoEditable' : 'vdwRepoReadOnly';
    }
}

function getDefaultCommand(
    dependency: Dependency,
    status: RepositoryRuntimeStatus,
    item: WorkspaceTreeItem,
): vscode.Command | undefined {
    if (status.state === 'missing') {
        return { command: 'vdw.linkRepo', title: 'Link Repository', arguments: [item] };
    }
    if (status.state === 'topologyConflict') {
        return { command: 'vdw.resolveRepoConflicts', title: 'Inspect Link Conflicts', arguments: [item] };
    }
    if (['invalidWorktree', 'unowned', 'detachedLink'].includes(status.state)) {
        return { command: 'vdw.relink', title: 'Reconcile Workspace' };
    }
    if (!status.exists) return undefined;
    return dependency.editable
        ? { command: 'vdw.openSourceControl', title: 'Open in Source Control', arguments: [item] }
        : { command: 'vdw.revealInExplorer', title: 'Reveal in Explorer', arguments: [item] };
}
