import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { Dependency } from '../types/vdwTypes';
import { ConfigService } from './configService';
import { log, logError } from '../utils/logger';

/**
 * AutoOpenService - 在 add、link、relink 操作后以及扩展激活时，
 * 自动在 VS Code Source Control 面板中注册可编辑依赖的 Git 仓库。
 * 
 * 直接从 .vdwrc 配置中读取 dep.local 获取 worktree 路径，然后调用
 * git.openRepository 命令触发原生 Source Control 面板显示该仓库。
 */
export class AutoOpenService implements vscode.Disposable {
    private configService: ConfigService;
    private _disposables: vscode.Disposable[] = [];

    constructor(configService: ConfigService) {
        this.configService = configService;
    }

    /**
     * Wait for the Git extension to finish its initial scan.
     * Resolves immediately if already initialized, or after a timeout (default 15s).
     */
    async waitForGitReady(timeoutMs: number = 15000): Promise<boolean> {
        const gitExtension = vscode.extensions.getExtension('vscode.git');
        if (!gitExtension) {
            log('AutoOpen: Git extension not found');
            return false;
        }
        if (!gitExtension.isActive) {
            await gitExtension.activate();
        }
        const gitAPI = gitExtension.exports?.getAPI(1);
        if (!gitAPI) {
            log('AutoOpen: Git API not available');
            return false;
        }
        if (gitAPI.state === 'initialized') {
            return true;
        }
        log(`AutoOpen: Git extension state is '${gitAPI.state}', waiting for initialized...`);
        return new Promise<boolean>((resolve) => {
            const timer = setTimeout(() => {
                sub.dispose();
                log('AutoOpen: Timed out waiting for Git extension to initialize');
                resolve(false);
            }, timeoutMs);
            const sub = gitAPI.onDidChangeState((state: string) => {
                if (state === 'initialized') {
                    clearTimeout(timer);
                    sub.dispose();
                    log('AutoOpen: Git extension initialized');
                    resolve(true);
                }
            });
        });
    }

    /**
     * Refresh specific Git repositories by path, so VS Code detects stale repos and drops them.
     * Uses the Git extension API to find and refresh the repo, avoiding path-matching issues.
     * Waits for the Git extension to be ready before attempting refresh.
     *
     * NOTE: repo.state is a NUMBER (RepositoryState enum: UNINITIALIZED=0, IDLE=1, ...)
     *       repo.HEAD is a SEPARATE property on the Repository object (not on repo.state)
     */
    async refreshGitRepositories(...candidatePaths: string[]): Promise<void> {
        try {
            const ready = await this.waitForGitReady();
            if (!ready) {
                log('AutoOpen: Git extension not ready, skipping refresh');
                return;
            }

            const gitExtension = vscode.extensions.getExtension('vscode.git')?.exports;
            const gitAPI = gitExtension?.getAPI(1);
            if (!gitAPI) {
                log('AutoOpen: Git extension not available');
                return;
            }

            // Diagnostic: dump ALL repos known to the Git extension
            log(`AutoOpen: Git extension has ${gitAPI.repositories.length} repos:`);
            for (const r of gitAPI.repositories) {
                log(`  - ${r.rootUri.fsPath} | state=${r.state} | HEAD.name=${r.HEAD?.name || '(none)'} | HEAD.commit=${r.HEAD?.commit?.slice(0, 7) || '(none)'}`);
            }
            log(`AutoOpen: Candidate paths: ${JSON.stringify(candidatePaths)}`);

            let refreshed = 0;
            const seen = new Set<string>();

            for (const p of candidatePaths) {
                if (!p || p.trim() === '') continue;

                let repo = gitAPI.getRepository(vscode.Uri.file(p));

                // Repo not registered with Git extension — try to open it.
                // Skip VDW store paths to avoid exposing internal worktree dirs.
                if (!repo) {
                    if (p.includes(path.sep + '.vdw' + path.sep)) {
                        log(`AutoOpen: Skipping VDW store path: ${p}`);
                        continue;
                    }
                    log(`AutoOpen: Repo not found at ${p}, attempting git.openRepository`);
                    try {
                        await vscode.commands.executeCommand('git.openRepository', p);
                        await new Promise(resolve => setTimeout(resolve, 500));
                    } catch (err) {
                        logError(`AutoOpen: Failed to open repository at ${p}`, err);
                        continue;
                    }
                    repo = gitAPI.getRepository(vscode.Uri.file(p));
                    if (!repo) {
                        log(`AutoOpen: Still not found after openRepository: ${p}`);
                        continue;
                    }
                }

                const repoPath = repo.rootUri.fsPath;
                if (seen.has(repoPath)) continue;
                seen.add(repoPath);

                const headName = repo.HEAD?.name;
                log(`AutoOpen: Matched repo: ${repoPath}, state=${repo.state}, HEAD.name=${headName || '(none)'}`);

                // Already has branch info — no refresh needed
                if (headName) {
                    log(`AutoOpen: Skipping (already has branch "${headName}"): ${repoPath}`);
                    continue;
                }

                // No branch info — call status() to trigger a refresh.
                // git status -b output includes "## branchname..." which updates HEAD.name
                log(`AutoOpen: Calling status() to refresh: ${repoPath}`);
                await repo.status();

                const afterHead = repo.HEAD?.name;
                log(`AutoOpen: After status(): HEAD.name=${afterHead || '(still none)'}`);
                if (afterHead) {
                    refreshed++;
                }
            }

            if (seen.size === 0) {
                log(`AutoOpen: No matching repository found for candidates`);
            }
            log(`AutoOpen: Refresh done — ${refreshed} refreshed, ${seen.size} unique repos found`);
        } catch (err) {
            logError('AutoOpen: Failed to refresh Git repositories', err);
        }
    }

    /**
     * 打开所有可编辑仓库的 Git 仓库注册
     * 遍历依赖列表，对每个 editable 的依赖优先使用 dep.origin（用户原始仓库路径），
     * 避免暴露 VDW store 内部路径（dep.local）到 Source Control。
     */
    async openAllEditableRepositories(): Promise<void> {
        const config = this.configService.getConfig();
        if (!config) return;
        if (!this.isAutoOpenEnabled()) return;

        const editableDeps = config.dependencies.filter(d => d.editable);
        const delay = this.getOpenDelay();

        for (const dep of editableDeps) {
            // Prefer dep.origin (user's original repo) over dep.local (VDW store worktree)
            const repoPath = dep.origin || dep.local;
            if (!repoPath) {
                log(`AutoOpen: ${dep.remote || dep.path} has no origin or local path set`);
                continue;
            }
            // Never expose VDW store internal paths to Source Control
            if (repoPath.includes(path.sep + '.vdw' + path.sep)) {
                log(`AutoOpen: Skipping VDW store path: ${repoPath}`);
                continue;
            }
            if (this.isValidGitRepository(repoPath)) {
                try {
                    await vscode.commands.executeCommand(
                        'git.openRepository',
                        repoPath
                    );
                } catch (err) {
                    logError(`AutoOpen: Failed to open Git repository: ${repoPath}`, err);
                }
                await new Promise(resolve => setTimeout(resolve, delay));
            }
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
