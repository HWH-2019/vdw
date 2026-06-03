import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { Dependency, WorkspaceState } from '../types/vdwTypes';
import { ConfigService } from './configService';
import { log, logError } from '../utils/logger';

/**
 * AutoOpenService - 在 add、link、relink 操作后以及扩展激活时，
 * 自动在 VS Code Source Control 面板中注册可编辑依赖的 Git 仓库。
 * 
 * 通过读取 .vdw/state.json 获取 worktree 路径，然后调用
 * git.openRepository 命令触发原生 Source Control 面板显示该仓库。
 */
export class AutoOpenService implements vscode.Disposable {
    private configService: ConfigService;
    private _disposables: vscode.Disposable[] = [];

    constructor(configService: ConfigService) {
        this.configService = configService;
    }

    /**
     * 打开所有可编辑仓库的 Git 仓库注册
     * 遍历依赖列表，对每个 editable 的依赖查找 worktree 路径并调用 git.openRepository
     */
    async openAllEditableRepositories(): Promise<void> {
        const config = this.configService.getConfig();
        if (!config) {
            log('AutoOpen: No config loaded, skipping');
            return;
        }

        if (!this.isAutoOpenEnabled()) {
            log('AutoOpen: Feature is disabled via settings');
            return;
        }

        const editableDeps = config.dependencies.filter(d => d.editable);
        if (editableDeps.length === 0) {
            log('AutoOpen: No editable dependencies found');
            return;
        }

        const delay = this.getOpenDelay();
        log(`AutoOpen: Opening ${editableDeps.length} editable Git repositories (delay: ${delay}ms)`);

        for (const dep of editableDeps) {
            const worktreePath = await this.getWorktreePath(dep);
            if (worktreePath && this.isValidGitRepository(worktreePath)) {
                try {
                    log(`AutoOpen: Opening Git repository at ${worktreePath}`);
                    await vscode.commands.executeCommand(
                        'git.openRepository',
                        vscode.Uri.file(worktreePath)
                    );
                } catch (err) {
                    logError(`AutoOpen: Failed to open Git repository: ${worktreePath}`, err);
                    // 不阻断流程，继续打开下一个仓库
                }
                // 在两个仓库打开之间添加延迟，避免并发冲突
                await new Promise(resolve => setTimeout(resolve, delay));
            } else {
                log(`AutoOpen: Skipping ${dep.repo} (${dep.branch || 'main'}) - worktree path invalid or not a Git repository: ${worktreePath}`);
            }
        }

        log('AutoOpen: Completed opening Git repositories');
    }

    /**
     * 从 .vdw/state.json 中查找依赖仓库对应的 worktree 路径
     */
    private async getWorktreePath(dep: Dependency): Promise<string | null> {
        const workspaceFolder = vscode.workspace.workspaceFolders?.[0];
        if (!workspaceFolder) return null;

        const statePath = path.join(workspaceFolder.uri.fsPath, '.vdw', 'state.json');
        if (!fs.existsSync(statePath)) {
            log(`AutoOpen: state.json not found at ${statePath}`);
            return null;
        }

        try {
            const content = fs.readFileSync(statePath, 'utf-8');
            const state: WorkspaceState = JSON.parse(content);
            const worktree = state.worktrees?.find(
                wt => wt.repo_url === dep.repo && wt.branch === (dep.branch || 'main')
            );
            if (worktree?.path) {
                log(`AutoOpen: Found worktree path for ${dep.repo}: ${worktree.path}`);
                return worktree.path;
            }
            return null;
        } catch (err) {
            logError('AutoOpen: Failed to parse state.json', err);
            return null;
        }
    }

    /**
     * 检查配置是否启用了自动打开功能
     */
    private isAutoOpenEnabled(): boolean {
        return vscode.workspace.getConfiguration('vdw')
            .get<boolean>('autoOpenGitRepositories', true);
    }

    /**
     * 获取仓库间打开延迟
     */
    private getOpenDelay(): number {
        return vscode.workspace.getConfiguration('vdw')
            .get<number>('autoOpenDelay', 500);
    }

    /**
     * 检查路径是否是一个有效的 Git 仓库（包含 .git 目录或 .git 文件）
     * 并且仓库有至少一个有效 commit（避免向 Git 扩展注册空仓库引发内部错误）
     * Git 标准仓库包含 .git 目录，Git worktree 包含 .git 文件
     */
    private isValidGitRepository(repoPath: string): boolean {
        if (!fs.existsSync(repoPath)) {
            log(`AutoOpen: Path does not exist: ${repoPath}`);
            return false;
        }
        
        const stat = fs.statSync(repoPath);
        if (!stat.isDirectory()) {
            log(`AutoOpen: Path is not a directory: ${repoPath}`);
            return false;
        }
        
        const gitPath = path.join(repoPath, '.git');
        if (!fs.existsSync(gitPath)) {
            log(`AutoOpen: No .git found at: ${repoPath}`);
            return false;
        }
        
        const gitStat = fs.statSync(gitPath);
        if (gitStat.isDirectory()) {
            // === Standard repository (.git is a directory) ===
            return this.isValidStandardRepo(repoPath, gitPath);
        } else if (gitStat.isFile()) {
            // === Git worktree (.git is a file pointing to the bare repo) ===
            return this.isValidWorktree(repoPath, gitPath);
        } else {
            log(`AutoOpen: .git is neither file nor directory at: ${repoPath}`);
            return false;
        }
    }

    /**
     * 验证标准 Git 仓库（.git 为目录）
     */
    private isValidStandardRepo(repoPath: string, gitDir: string): boolean {
        return this.hasValidHeadRef(path.join(gitDir, 'HEAD'), (refPath: string) => path.join(gitDir, refPath));
    }

    /**
     * 验证 Git Worktree（.git 为文件，指向裸仓库）
     */
    private isValidWorktree(repoPath: string, gitFile: string): boolean {
        let bareRepoPath: string;
        try {
            const content = fs.readFileSync(gitFile, 'utf-8').trim();
            if (!content.startsWith('gitdir:')) {
                log(`AutoOpen: .git file has invalid format: ${repoPath}`);
                return false;
            }
            bareRepoPath = content.replace(/^gitdir:\s*/, '').trim();
        } catch (err) {
            logError(`AutoOpen: Failed to read .git file at: ${repoPath}`, err);
            return false;
        }
        
        // bareRepoPath is like: /path/to/bare.git/worktrees/<name>
        // The HEAD for this worktree is at: bareRepoPath/HEAD
        const worktreeHeadPath = path.join(bareRepoPath, 'HEAD');
        if (!fs.existsSync(worktreeHeadPath)) {
            log(`AutoOpen: Worktree HEAD not found: ${worktreeHeadPath}`);
            return false;
        }
        
        // The bare repo root is two levels up: /path/to/bare.git/
        const bareGitDir = path.dirname(path.dirname(bareRepoPath));
        
        return this.hasValidHeadRef(worktreeHeadPath, (refPath: string) => path.join(bareGitDir, refPath));
    }

    /**
     * 读取 HEAD 并验证其指向的分支 ref 是否存在
     * @param headPath - HEAD 文件路径
     * @param resolveRef - 函数：将 ref 路径（如 refs/heads/main）解析为绝对路径
     */
    private hasValidHeadRef(headPath: string, resolveRef: (refPath: string) => string): boolean {
        let headContent: string;
        try {
            headContent = fs.readFileSync(headPath, 'utf-8').trim();
        } catch {
            log(`AutoOpen: Cannot read HEAD at: ${headPath}`);
            return false;
        }
        
        if (!headContent || headContent.length === 0) {
            log(`AutoOpen: HEAD is empty at: ${headPath}`);
            return false;
        }
        
        // If HEAD points to a branch (ref: refs/heads/<name>), verify the ref exists
        if (headContent.startsWith('ref: ')) {
            const refPath = headContent.substring(5).trim();
            const refFile = resolveRef(refPath);
            if (!fs.existsSync(refFile)) {
                log(`AutoOpen: Branch ref not found: ${refFile} (from HEAD: ${headContent})`);
                return false;
            }
        }
        // If HEAD is detached at a commit hash, no ref file to check - valid
        
        return true;
    }

    dispose(): void {
        this._disposables.forEach(d => d.dispose());
        this._disposables = [];
    }
}
