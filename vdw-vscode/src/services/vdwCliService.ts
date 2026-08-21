import * as vscode from 'vscode';
import { spawn } from 'child_process';
import * as path from 'path';
import {
    LinkConflictReport,
    LinkConflictResolutionResult,
    LinkConflictResolutionSide,
    RepoStatusResult,
    ValidationResult,
} from '../types/vdwTypes';
import { log, logError } from '../utils/logger';
import { buildConflictListArgs, buildConflictResolveArgs } from '../utils/linkConflicts';

export interface CommandResult {
    stdout: string;
    stderr: string;
    exitCode: number;
}

export class VDWCliService {
    private workspacePath: string;

    constructor(workspacePath: string) {
        this.workspacePath = workspacePath;
    }

    /** Get the configured CLI binary path */
    private getCliPath(): string {
        // Debug override: explicit CLI path injected by the debugger (see .vscode/launch.json).
        // Points to the debug binary deployed into the vdw-test directory.
        const debugCliPath = process.env.VDW_CLI_PATH;
        if (debugCliPath) {
            log(`Debug mode: using injected CLI path ${debugCliPath}`);
            return debugCliPath;
        }

        const config = vscode.workspace.getConfiguration('vdw');
        const configuredPath = config.get<string>('cliPath', '');

        log(`Using CLI path: ${configuredPath}`);
        
        // If configured path is set, use it
        if (configuredPath) {
            return configuredPath;
        }
        
        // Auto-detect debug binary in development mode
        if (process.env.VDW_DEBUG === 'true') {
            const debugPath = path.join(__dirname, '..', '..', '..', 'vdw', 'target', 'debug', 'vdw');
            log(`Development mode: using debug CLI at ${debugPath}`);
            return debugPath;
        }
        
        // Try to detect local vdw binary in workspace
        const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
        if (workspaceFolder) {
            const localVdw = path.join(workspaceFolder.uri.fsPath, 'vdw', 'vdw');
            if (require('fs').existsSync(localVdw)) {
                log(`Using local vdw binary: ${localVdw}`);
                return localVdw;
            }
        }
        
        // Fallback to system PATH
        return 'vdw';
    }

    /** Execute a VDW CLI command */
    async executeCommand(subcommand: string, args: string[] = []): Promise<CommandResult> {
        const cliPath = this.getCliPath();
        const fullArgs = [subcommand, ...args];

        log(`Executing: ${cliPath} ${fullArgs.join(' ')}`);

        return new Promise((resolve, reject) => {
            const proc = spawn(cliPath, fullArgs, {
                cwd: this.workspacePath,
                env: { ...process.env },
            });

            let stdout = '';
            let stderr = '';

            proc.stdout.on('data', (data) => { stdout += data.toString(); });
            proc.stderr.on('data', (data) => { stderr += data.toString(); });

            proc.on('close', (code) => {
                const result: CommandResult = {
                    stdout: stdout.trim(),
                    stderr: stderr.trim(),
                    exitCode: code ?? 1,
                };

                if (code !== 0) {
                    logError(`Command failed (exit ${code}): ${cliPath} ${fullArgs.join(' ')}`, stderr);
                }

                resolve(result);
            });

            proc.on('error', (err) => {
                logError(`Failed to spawn: ${cliPath}`, err);
                reject(new Error(`Failed to execute VDW CLI: ${err.message}. Is 'vdw' installed and in PATH?`));
            });
        });
    }

    /** Run vdw link */
    async link(): Promise<CommandResult> {
        return this.executeCommand('link');
    }

    /** Run vdw link --repo for a specific repository */
    async linkRepo(repoUrl: string): Promise<CommandResult> {
        return this.executeCommand('link', ['--repo', repoUrl]);
    }

    /** Run vdw unlink */
    async unlink(): Promise<CommandResult> {
        return this.executeCommand('unlink');
    }

    /** Run transactional vdw relink */
    async relink(): Promise<CommandResult> {
        return this.executeCommand('relink');
    }

    /** Inspect workspace files that no longer match their Store hardlinks. */
    async getLinkConflicts(repoUrl?: string): Promise<LinkConflictReport> {
        const args = buildConflictListArgs(repoUrl);
        const result = await this.executeCommand('conflicts', args);
        if (result.exitCode !== 0) {
            throw new Error(result.stderr || result.stdout || 'Failed to inspect link conflicts');
        }
        return JSON.parse(result.stdout) as LinkConflictReport;
    }

    /** Resolve all or selected conflicts, keeping a CLI-managed backup of both sides. */
    async resolveLinkConflicts(
        accepted: LinkConflictResolutionSide,
        repoUrl?: string,
        relativePaths: string[] = [],
    ): Promise<LinkConflictResolutionResult> {
        const args = buildConflictResolveArgs(accepted, repoUrl, relativePaths);
        const result = await this.executeCommand('conflicts', args);
        if (result.exitCode !== 0) {
            throw new Error(result.stderr || result.stdout || 'Failed to resolve link conflicts');
        }
        return JSON.parse(result.stdout) as LinkConflictResolutionResult;
    }

    /** Run vdw unlink --repo for a specific repository */
    async unlinkRepo(repoUrl: string): Promise<CommandResult> {
        return this.executeCommand('unlink', ['--repo', repoUrl]);
    }

    /** Run vdw validate and parse result */
    async validate(): Promise<ValidationResult> {
        const result = await this.executeCommand('validate');

        if (result.exitCode === 0) {
            // Parse success output
            const lines = result.stdout.split('\n');
            const nameLine = lines.find(l => l.includes('Workspace:'));
            const depsLine = lines.find(l => l.includes('Dependencies:'));

            return {
                valid: true,
                errors: [],
                workspace_name: nameLine?.split(':')[1]?.trim(),
                dependency_count: depsLine ? parseInt(depsLine.split(':')[1]?.trim() || '0') : 0,
            };
        } else {
            // Parse error output
            const errors = result.stdout.split('\n')
                .filter(l => l.trim().startsWith('-'))
                .map(l => l.trim().substring(2));

            return {
                valid: false,
                errors: errors.length > 0 ? errors : [result.stderr || 'Unknown validation error'],
            };
        }
    }

    /** Run vdw init */
    async init(template?: string): Promise<CommandResult> {
        const args = template ? ['--template', template] : [];
        return this.executeCommand('init', args);
    }

    /** Run vdw context preview */
    async contextPreview(): Promise<string> {
        const result = await this.executeCommand('context', ['preview']);
        return result.stdout;
    }

    /** Run vdw context stats */
    async contextStats(): Promise<string> {
        const result = await this.executeCommand('context', ['stats']);
        return result.stdout;
    }

    /** Run vdw doctor */
    async doctor(): Promise<string> {
        const result = await this.executeCommand('doctor');
        return result.stdout + (result.stderr ? '\n' + result.stderr : '');
    }

    /** Run vdw repo remove for a specific repository */
    async removeRepo(repoUrl: string): Promise<CommandResult> {
        return this.executeCommand('repo', ['remove', repoUrl]);
    }

    /** Run vdw repo status for a specific repository */
    async checkRepoStatus(repoName: string): Promise<RepoStatusResult | null> {
        try {
            const result = await this.executeCommand('repo', ['status', repoName]);
            if (result.exitCode === 0) {
                return JSON.parse(result.stdout);
            }
        } catch {
            // Graceful degradation: CLI may not support repo status (old version)
        }
        return null;
    }

    /** Update workspace path */
    setWorkspacePath(newPath: string): void {
        this.workspacePath = newPath;
    }
}
