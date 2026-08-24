import * as path from 'path';
import * as vscode from 'vscode';
import { ConfigService } from './configService';
import { VDWCliService } from './vdwCliService';
import {
    Dependency,
    WorkspaceTopologyStatusResult,
    RepositoryRuntimeStatus,
    WorkspaceRuntimeStatus,
} from '../types/vdwTypes';
import {
    collectRepositoryRuntimeStatus,
    summarizeWorkspaceStatus,
} from '../utils/workspaceStatus';
import { log, logError } from '../utils/logger';

const EMPTY_STATUS: WorkspaceRuntimeStatus = {
    repositories: [],
    issueCount: 0,
    dirtyCount: 0,
};

const TOPOLOGY_CACHE_MS = 5_000;

interface RefreshOptions {
    forceTopology?: boolean;
}

/**
 * Builds a runtime-only snapshot from the workspace and VS Code Git model.
 * Dynamic UI state deliberately stays out of VDW's persisted state.json.
 */
export class WorkspaceStatusService implements vscode.Disposable {
    private readonly _onDidChange = new vscode.EventEmitter<WorkspaceRuntimeStatus>();
    readonly onDidChange = this._onDidChange.event;

    private snapshot: WorkspaceRuntimeStatus = EMPTY_STATUS;
    private refreshPromise: Promise<WorkspaceRuntimeStatus> | undefined;
    private refreshRequested = false;
    private forceTopologyRequested = false;
    private topologySnapshot: WorkspaceTopologyStatusResult | undefined;
    private topologyRefreshedAt = 0;
    private gitApi: any | undefined;
    private gitApiSubscriptions: vscode.Disposable[] = [];
    private repositorySubscriptions: vscode.Disposable[] = [];
    private refreshTimer: NodeJS.Timeout | undefined;

    constructor(
        private readonly configService: ConfigService,
        private readonly cliService: VDWCliService,
        private readonly workspaceRoot: string,
    ) {}

    getSnapshot(): WorkspaceRuntimeStatus {
        return this.snapshot;
    }

    getRepositoryStatus(dependency: Dependency): RepositoryRuntimeStatus | undefined {
        return this.snapshot.repositories.find(status => status.dependencyPath === dependency.path);
    }

    refresh(options: RefreshOptions = {}): Promise<WorkspaceRuntimeStatus> {
        if (this.refreshPromise) {
            this.refreshRequested = true;
            this.forceTopologyRequested ||= options.forceTopology === true;
            return this.refreshPromise.then(() => {
                if (!this.refreshRequested) return this.snapshot;
                this.refreshRequested = false;
                const forceTopology = this.forceTopologyRequested;
                this.forceTopologyRequested = false;
                return this.refresh({ forceTopology });
            });
        }

        this.refreshPromise = this.collectSnapshot(options.forceTopology === true)
            .then(snapshot => {
                this.snapshot = snapshot;
                this.bindRepositoryListeners();
                this._onDidChange.fire(snapshot);
                return snapshot;
            })
            .catch(error => {
                logError('Workspace status refresh failed', error);
                return this.snapshot;
            })
            .finally(() => {
                this.refreshPromise = undefined;
            });

        return this.refreshPromise;
    }

    private async collectSnapshot(forceTopology: boolean): Promise<WorkspaceRuntimeStatus> {
        const config = this.configService.getConfig();
        if (!config) return EMPTY_STATUS;

        const [gitApi, topology] = await Promise.all([
            this.getGitApi(),
            this.getTopologyStatus(forceTopology),
        ]);
        const topologyByPath = new Map(
            topology?.repositories.map(repository => [repository.path, repository]) ?? [],
        );
        const repositories = config.dependencies.map(dependency =>
            collectRepositoryRuntimeStatus(
                dependency,
                this.workspaceRoot,
                gitApi,
                topologyByPath.get(dependency.path),
            ),
        );
        return summarizeWorkspaceStatus(repositories);
    }

    private async getTopologyStatus(force: boolean): Promise<WorkspaceTopologyStatusResult | undefined> {
        const cacheFresh = Date.now() - this.topologyRefreshedAt < TOPOLOGY_CACHE_MS;
        if (!force && cacheFresh) return this.topologySnapshot;

        const topology = await this.cliService.getWorkspaceStatus();
        this.topologyRefreshedAt = Date.now();
        if (topology) {
            this.topologySnapshot = topology;
        }
        return topology ?? this.topologySnapshot;
    }

    private async getGitApi(): Promise<any | undefined> {
        if (this.gitApi) return this.gitApi;

        const gitExtension = vscode.extensions.getExtension('vscode.git');
        if (!gitExtension) return undefined;
        try {
            if (!gitExtension.isActive) await gitExtension.activate();
            this.gitApi = gitExtension.exports?.getAPI(1);
            if (this.gitApi) this.bindGitApiListeners();
            return this.gitApi;
        } catch (error) {
            logError('Workspace status could not activate the Git extension', error);
            return undefined;
        }
    }

    private bindGitApiListeners(): void {
        if (this.gitApiSubscriptions.length > 0 || !this.gitApi) return;

        if (typeof this.gitApi.onDidOpenRepository === 'function') {
            this.gitApiSubscriptions.push(
                this.gitApi.onDidOpenRepository(() => this.scheduleRefresh()),
            );
        }
        if (typeof this.gitApi.onDidCloseRepository === 'function') {
            this.gitApiSubscriptions.push(
                this.gitApi.onDidCloseRepository(() => this.scheduleRefresh()),
            );
        }
        log('Workspace status service subscribed to Git repository lifecycle events');
    }

    private bindRepositoryListeners(): void {
        this.repositorySubscriptions.forEach(subscription => subscription.dispose());
        this.repositorySubscriptions = [];
        if (!this.gitApi) return;

        const managedPaths = new Set(
            this.snapshot.repositories
                .filter(status => status.scmRegistered)
                .map(status => normalizeRepositoryPath(status.fullPath)),
        );

        for (const repository of this.gitApi.repositories ?? []) {
            if (!managedPaths.has(normalizeRepositoryPath(repository.rootUri.fsPath))) continue;
            if (typeof repository?.state?.onDidChange !== 'function') continue;
            this.repositorySubscriptions.push(
                repository.state.onDidChange(() => this.scheduleRefresh()),
            );
        }
    }

    private scheduleRefresh(): void {
        if (this.refreshTimer) clearTimeout(this.refreshTimer);
        this.refreshTimer = setTimeout(() => {
            this.refreshTimer = undefined;
            void this.refresh();
        }, 150);
    }

    dispose(): void {
        if (this.refreshTimer) clearTimeout(this.refreshTimer);
        this.repositorySubscriptions.forEach(subscription => subscription.dispose());
        this.gitApiSubscriptions.forEach(subscription => subscription.dispose());
        this._onDidChange.dispose();
    }
}

function normalizeRepositoryPath(candidate: string): string {
    const normalized = path.normalize(path.resolve(candidate)).replace(/[\\/]+$/, '');
    return process.platform === 'win32' ? normalized.toLowerCase() : normalized;
}
