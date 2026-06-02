import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { minimatch } from 'minimatch';
import { VDWConfig, Scope } from '../types/vdwTypes';
import { ConfigService } from '../services/configService';
import { log } from '../utils/logger';

export class VDWFileProvider implements vscode.TextDocumentContentProvider {
    private _onDidChangeEmitter = new vscode.EventEmitter<vscode.Uri>();
    readonly onDidChange = this._onDidChangeEmitter.event;

    private configService: ConfigService;
    private workspaceRoot: string;

    constructor(configService: ConfigService, workspaceRoot: string) {
        this.configService = configService;
        this.workspaceRoot = workspaceRoot;
    }

    /** Provide document content - only if file is within scope */
    provideTextDocumentContent(uri: vscode.Uri): string | undefined {
        try {
            const config = this.configService.getConfig();
            if (!config) {
                return undefined;
            }

            const filePath = uri.fsPath;
            const relativePath = path.relative(this.workspaceRoot, filePath);

            if (!this.isInAnyScope(relativePath, config)) {
                return undefined;
            }

            // Read and return actual file content
            if (fs.existsSync(filePath)) {
                return fs.readFileSync(filePath, 'utf-8');
            }
            return undefined;
        } catch (error) {
            log(`FileProvider error: ${error}`);
            return undefined;
        }
    }

    /** List all files within scope across all dependencies */
    async listFiles(): Promise<string[]> {
        const config = this.configService.getConfig();
        if (!config) {
            return [];
        }

        const scopedFiles: string[] = [];

        for (const dep of config.dependencies) {
            const depPath = path.join(this.workspaceRoot, dep.path);
            if (!fs.existsSync(depPath)) {
                continue;
            }
            const files = await this.walkDirectory(depPath, dep.scope);
            scopedFiles.push(...files);
        }

        return scopedFiles;
    }

    /** Check if a file is in any dependency's scope */
    private isInAnyScope(relativePath: string, config: VDWConfig): boolean {
        const normalizedPath = relativePath.replace(/\\/g, '/');

        return config.dependencies.some(dep => {
            const depPathNormalized = dep.path.replace(/^\.\//, '').replace(/\\/g, '/');
            if (!normalizedPath.startsWith(depPathNormalized)) {
                return false;
            }
            const fileInDep = normalizedPath.substring(depPathNormalized.length).replace(/^\//, '');
            return this.matchesScope(fileInDep, dep.scope);
        });
    }

    /** Check if file matches scope include/exclude rules */
    private matchesScope(filePath: string, scope: Scope): boolean {
        const normalized = filePath.replace(/\\/g, '/');

        // Check excludes first
        for (const pattern of scope.exclude) {
            if (minimatch(normalized, pattern, { dot: true })) {
                return false;
            }
        }

        // Check includes
        for (const pattern of scope.include) {
            if (minimatch(normalized, pattern, { dot: true })) {
                return true;
            }
        }

        return false;
    }

    /** Recursively walk directory and filter by scope */
    private async walkDirectory(dirPath: string, scope: Scope): Promise<string[]> {
        const files: string[] = [];

        const walk = (currentPath: string) => {
            try {
                const entries = fs.readdirSync(currentPath, { withFileTypes: true });
                for (const entry of entries) {
                    const fullPath = path.join(currentPath, entry.name);
                    if (entry.isDirectory()) {
                        // Skip common non-useful dirs
                        if (entry.name === 'node_modules' || entry.name === '.git') {
                            continue;
                        }
                        walk(fullPath);
                    } else if (entry.isFile()) {
                        const relativePath = path.relative(dirPath, fullPath).replace(/\\/g, '/');
                        if (this.matchesScope(relativePath, scope)) {
                            files.push(fullPath);
                        }
                    }
                }
            } catch {
                // Skip directories we can't read
            }
        };

        walk(dirPath);
        return files;
    }

    /** Notify subscribers that content has changed */
    refresh(): void {
        this._onDidChangeEmitter.fire(vscode.Uri.parse('vdw://refresh'));
    }

    /** Update workspace root */
    setWorkspaceRoot(newRoot: string): void {
        this.workspaceRoot = newRoot;
    }

    dispose(): void {
        this._onDidChangeEmitter.dispose();
    }
}
