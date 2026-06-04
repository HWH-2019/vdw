import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { VDWConfig, Dependency } from '../types/vdwTypes';
import { ConfigService } from './configService';
import { VDWFileProvider } from '../providers/fileProvider';
import { log } from '../utils/logger';

export interface ContextStats {
    totalFiles: number;
    totalSizeBytes: number;
    perDependency: DependencyContextStats[];
}

export interface DependencyContextStats {
    repo: string;
    path: string;
    fileCount: number;
    sizeBytes: number;
}

export class ContextService {
    private _onDidChange = new vscode.EventEmitter<void>();
    readonly onDidChange = this._onDidChange.event;

    private configService: ConfigService;
    private fileProvider: VDWFileProvider;
    private workspaceRoot: string;
    private cachedStats: ContextStats | null = null;

    constructor(
        configService: ConfigService,
        fileProvider: VDWFileProvider,
        workspaceRoot: string,
    ) {
        this.configService = configService;
        this.fileProvider = fileProvider;
        this.workspaceRoot = workspaceRoot;
    }

    /** Get context statistics */
    async getStats(): Promise<ContextStats> {
        if (this.cachedStats) {
            return this.cachedStats;
        }

        const config = this.configService.getConfig();
        if (!config) {
            return { totalFiles: 0, totalSizeBytes: 0, perDependency: [] };
        }

        let totalFiles = 0;
        let totalSizeBytes = 0;
        const perDependency: DependencyContextStats[] = [];

        for (const dep of config.dependencies) {
            const depPath = path.join(this.workspaceRoot, dep.path);
            const files = await this.fileProvider.listFiles();
            // Filter to just this dependency's files
            const depFiles = files.filter(f => f.startsWith(depPath));

            let depSize = 0;
            for (const file of depFiles) {
                try {
                    const stat = fs.statSync(file);
                    depSize += stat.size;
                } catch {
                    // skip
                }
            }

            totalFiles += depFiles.length;
            totalSizeBytes += depSize;
            perDependency.push({
                repo: dep.remote || dep.local,
                path: dep.path,
                fileCount: depFiles.length,
                sizeBytes: depSize,
            });
        }

        this.cachedStats = { totalFiles, totalSizeBytes, perDependency };
        return this.cachedStats;
    }

    /** Get total file count in context */
    async getFileCount(): Promise<number> {
        const files = await this.fileProvider.listFiles();
        return files.length;
    }

    /** Invalidate cache and notify listeners */
    refresh(): void {
        this.cachedStats = null;
        this._onDidChange.fire();
        log('Context refreshed');
    }

    dispose(): void {
        this._onDidChange.dispose();
    }
}
