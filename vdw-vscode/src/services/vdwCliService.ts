import * as vscode from 'vscode';
import { spawn } from 'child_process';
import * as path from 'path';
import { ValidationResult } from '../types/vdwTypes';
import { log, logError } from '../utils/logger';

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
        const config = vscode.workspace.getConfiguration('vdw');
        return config.get<string>('cliPath', 'vdw');
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

    /** Run vdw unlink */
    async unlink(): Promise<CommandResult> {
        return this.executeCommand('unlink');
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

    /** Update workspace path */
    setWorkspacePath(newPath: string): void {
        this.workspacePath = newPath;
    }
}
