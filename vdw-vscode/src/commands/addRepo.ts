import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import * as yaml from 'js-yaml';
import { VDWCliService } from '../services/vdwCliService';
import { ConfigService } from '../services/configService';
import { log, logError } from '../utils/logger';

/**
 * Handle adding a repository through interactive wizard
 * Supports both local and remote repositories
 */
export async function handleAddRepo(
    cliService: VDWCliService,
    configService: ConfigService,
): Promise<boolean> {
    log('Starting interactive repository addition wizard');

    try {
        // Step 1: Choose repository type
        const repoTypeOptions = [
            { label: ' Remote Repository', description: 'Add from Git URL (GitHub, GitLab, etc.)', value: 'remote' },
            { label: '📁 Local Repository', description: 'Add existing local Git repository', value: 'local' },
        ];
        
        const repoTypeChoice = await vscode.window.showQuickPick(repoTypeOptions, {
            placeHolder: 'Select repository type',
            title: 'Add Repository',
        });

        if (!repoTypeChoice) {
            log('User cancelled repository type selection');
            return false;
        }

        const repoType = repoTypeChoice.value;

        let repoUrl: string = '';
        let localPath: string = '';
        let branch: string = 'main';
        let editable: boolean = false;

        if (repoType === 'remote') {
            // Remote repository flow
            const urlInput = await vscode.window.showInputBox({
                prompt: 'Enter Git repository URL',
                placeHolder: 'git@github.com:org/repo.git or https://github.com/org/repo.git',
                validateInput: (value) => {
                    if (!value || value.trim() === '') {
                        return 'Repository URL is required';
                    }
                    if (!value.startsWith('git@') && !value.startsWith('https://') && !value.startsWith('http://')) {
                        return 'URL must start with git@, https://, or http://';
                    }
                    return null;
                },
            });

            if (!urlInput) {
                log('User cancelled repository URL input');
                return false;
            }
            repoUrl = urlInput;

            // Auto-suggest workspace link path from repo name
            const repoName = repoUrl.split('/').pop()?.replace('.git', '') || 'repo';
            const pathInput = await vscode.window.showInputBox({
                prompt: 'Enter workspace link path (relative to workspace root)',
                placeHolder: './apps/web or ./packages/shared',
                value: './packages/' + repoName,
                validateInput: (value) => {
                    if (!value || value.trim() === '') {
                        return 'Workspace path is required';
                    }
                    if (!value.startsWith('./')) {
                        return 'Path should be relative (start with ./)';
                    }
                    return null;
                },
            });

            if (!pathInput) {
                log('User cancelled local path input');
                return false;
            }
            localPath = pathInput;
        } else {
            // Local repository flow
            const localRepoPath = await vscode.window.showOpenDialog({
                canSelectFolders: true,
                canSelectFiles: false,
                canSelectMany: false,
                openLabel: 'Select Local Repository',
                title: 'Select Local Git Repository',
            });

            if (!localRepoPath || localRepoPath.length === 0) {
                log('User cancelled local repository selection');
                return false;
            }

            const selectedPath = localRepoPath[0].fsPath;
            repoUrl = selectedPath;

            // Get current branch
            try {
                const { execSync } = require('child_process');
                branch = execSync(`git -C "${selectedPath}" branch --show-current`).toString().trim() || 'main';
            } catch (error) {
                logError('Failed to get current branch', error);
            }

            // Ask for workspace link path
            const repoName = path.basename(selectedPath);
            const pathInput = await vscode.window.showInputBox({
                prompt: 'Enter workspace link path (relative to workspace root)',
                placeHolder: './apps/web or ./packages/shared',
                value: './packages/' + repoName,
                validateInput: (value) => {
                    if (!value || value.trim() === '') {
                        return 'Workspace path is required';
                    }
                    if (!value.startsWith('./')) {
                        return 'Path should be relative (start with ./)';
                    }
                    return null;
                },
            });

            if (!pathInput) {
                log('User cancelled workspace path input');
                return false;
            }
            localPath = pathInput;
        }

        // Step 3: Branch selection (shared, with different defaults)
        const branchInput = await vscode.window.showInputBox({
            prompt: 'Enter branch name for the repository' + (repoType === 'local' ? ' (leave empty to use detected branch)' : ''),
            placeHolder: 'main, master, develop, etc.',
            value: branch,
            validateInput: (value) => {
                if (!value || value.trim() === '') {
                    return 'Branch name is required';
                }
                return null;
            },
        });

        if (!branchInput) {
            log('User cancelled branch input');
            return false;
        }
        branch = branchInput.trim();

        // Step 4: Editable mode selection (shared for both remote and local)
        const editableChoice = await vscode.window.showQuickPick(
            ['Yes - Editable (Git worktree)', 'No - Read-only (linked)'],
            {
                placeHolder: 'Should this repository be editable?',
                title: 'Add Repository',
            },
        );

        if (!editableChoice) {
            log('User cancelled editable choice');
            return false;
        }
        editable = editableChoice === 'Yes - Editable (Git worktree)';

        // Step 5: Scope pattern selection (shared for both remote and local)
        const includePattern = await vscode.window.showInputBox({
            prompt: 'Enter scope include patterns (comma-separated globs)',
            placeHolder: '**/*, src/**, package.json',
            value: '**/*',
            validateInput: (value) => {
                if (!value || value.trim() === '') {
                    return 'At least one include pattern is required';
                }
                return null;
            },
        });

        if (!includePattern) {
            log('User cancelled scope pattern input');
            return false;
        }

        const scopeInclude = includePattern.split(',').map(p => p.trim()).filter(p => p.length > 0);

        // Step 6: Execute CLI command (for both remote and local)
        return await vscode.window.withProgress(
            {
                location: vscode.ProgressLocation.Notification,
                title: repoType === 'remote' ? 'Adding remote repository...' : 'Adding local repository...',
                cancellable: false,
            },
            async () => {
                try {
                    const args: string[] = [
                        repoUrl,
                        '--path',
                        localPath,
                        '--branch',
                        branch,
                    ];

                    if (editable) {
                        args.push('--editable');
                    }

                    const result = await cliService.executeCommand('repo', ['add', ...args]);

                    if (result.exitCode === 0) {
                        const repoLabel = repoType === 'remote' ? repoUrl : path.basename(repoUrl);
                        log(`Successfully added repository: ${repoLabel}`);

                        // Update .vdwrc with scope configuration (CLI doesn't support --scope yet)
                        try {
                            const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
                            if (workspaceFolder) {
                                const vdwrcPath = path.join(workspaceFolder.uri.fsPath, '.vdwrc');
                                if (fs.existsSync(vdwrcPath)) {
                                    const content = fs.readFileSync(vdwrcPath, 'utf-8');
                                    const config = yaml.load(content) as any;
                                    if (config && config.dependencies) {
                                        const dep = config.dependencies.find(
                                            (d: any) => (d.repo === repoUrl || d.path === localPath)
                                        );
                                        if (dep) {
                                            dep.scope = {
                                                include: scopeInclude,
                                            };
                                            const yamlStr = yaml.dump(config, {
                                                indent: 2,
                                                lineWidth: 120,
                                                noRefs: true,
                                            });
                                            fs.writeFileSync(vdwrcPath, yamlStr, 'utf-8');
                                            log('Updated .vdwrc with scope configuration');
                                        }
                                    }
                                }
                            }
                        } catch (scopeErr) {
                            logError('Failed to update .vdwrc with scope', scopeErr);
                            // Don't fail the operation, scope can be edited manually
                        }

                        vscode.window.showInformationMessage(
                            `Repository added successfully: ${repoLabel}\nPath: ${localPath}`,
                        );
                        return true;
                    } else {
                        logError('Failed to add repository', result.stderr);
                        vscode.window.showErrorMessage(
                            `Failed to add repository: ${result.stderr || 'Unknown error'}`,
                        );
                        return false;
                    }
                } catch (error) {
                    logError('Error adding repository', error);
                    vscode.window.showErrorMessage(
                        `Error adding repository: ${error instanceof Error ? error.message : 'Unknown error'}`,
                    );
                    return false;
                }
            },
        );
    } catch (error) {
        logError('Error in add repo wizard', error);
        vscode.window.showErrorMessage(
            `Failed to add repository: ${error instanceof Error ? error.message : 'Unknown error'}`,
        );
        return false;
    }
}
