import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
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
    private gitOperationQueue: Promise<void> = Promise.resolve();

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
     * Open and refresh every editable dependency's managed worktree.
     * `origin` and the workspace projection are deliberately excluded: they are not
     * the repository that owns the files edited through VDW hardlinks.
     */
    async openAllEditableRepositories(): Promise<void> {
        if (!this.isAutoOpenEnabled()) return;

        return this.enqueueGitOperation(async () => {
            const config = this.configService.getConfig();
            if (!config) return;

            const managedWorktreePaths = config.dependencies
                .filter(dep => dep.editable && dep.local)
                .map(dep => dep.local);

            await this.syncManagedRepositories(managedWorktreePaths);
        });
    }

    /** Serialize Git API operations triggered by commands and .vdwrc watchers. */
    private enqueueGitOperation(operation: () => Promise<void>): Promise<void> {
        const queued = this.gitOperationQueue.then(operation, operation);
        this.gitOperationQueue = queued.catch(() => undefined);
        return queued;
    }

    private async syncManagedRepositories(managedWorktreePaths: string[]): Promise<void> {
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

            let refreshed = 0;
            const seen = new Set<string>();

            for (const worktreePath of managedWorktreePaths) {
                if (!worktreePath || worktreePath.trim() === '') continue;
                if (!this.isValidGitRepository(worktreePath)) continue;

                const normalizedPath = this.normalizeRepositoryPath(worktreePath);
                if (seen.has(normalizedPath)) continue;
                seen.add(normalizedPath);

                // getRepository(uri) may return a containing parent repository. Match
                // rootUri exactly so a workspace repository can never stand in for a
                // VDW-managed worktree.
                let repo = this.findExactRepository(gitAPI, normalizedPath);
                if (!repo) {
                    log(`AutoOpen: Opening managed worktree: ${worktreePath}`);
                    try {
                        if (typeof gitAPI.openRepository === 'function') {
                            await gitAPI.openRepository(vscode.Uri.file(worktreePath));
                        } else {
                            await vscode.commands.executeCommand('git.openRepository', worktreePath);
                        }
                        await new Promise(resolve => setTimeout(resolve, this.getOpenDelay()));
                    } catch (err) {
                        logError(`AutoOpen: Failed to open managed worktree: ${worktreePath}`, err);
                        continue;
                    }
                    repo = this.findExactRepository(gitAPI, normalizedPath);
                    if (!repo) {
                        log(`AutoOpen: Git extension did not register the exact worktree root: ${worktreePath}`);
                        continue;
                    }
                }

                const repoPath = repo.rootUri.fsPath;
                log(`AutoOpen: Refreshing exact managed worktree: ${repoPath}`);
                await repo.status();
                refreshed++;
            }

            log(`AutoOpen: Refresh done — ${refreshed}/${seen.size} managed worktrees refreshed`);
        } catch (err) {
            logError('AutoOpen: Failed to synchronize managed Git worktrees', err);
        }
    }

    private findExactRepository(gitAPI: any, normalizedPath: string): any | undefined {
        return gitAPI.repositories.find((repo: any) =>
            this.normalizeRepositoryPath(repo.rootUri.fsPath) === normalizedPath
        );
    }

    private normalizeRepositoryPath(repoPath: string): string {
        let normalized: string;
        try {
            normalized = fs.realpathSync.native(repoPath);
        } catch {
            normalized = path.resolve(repoPath);
        }
        normalized = normalized.replace(/[\\/]+$/, '');
        return process.platform === 'win32' ? normalized.toLowerCase() : normalized;
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
        return this.hasValidHeadRef(path.join(gitDir, 'HEAD'), gitDir);
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
            if (!path.isAbsolute(bareRepoPath)) {
                bareRepoPath = path.resolve(path.dirname(gitFile), bareRepoPath);
            }
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
        
        return this.hasValidHeadRef(worktreeHeadPath, bareGitDir);
    }

    /**
     * 读取 HEAD 并验证其指向的分支 ref 是否存在
     * @param headPath - HEAD 文件路径
     * @param gitDir - 仓库 git 目录，用于解析 loose ref 和 packed-refs
     */
    private hasValidHeadRef(headPath: string, gitDir: string): boolean {
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
            const refFile = path.join(gitDir, refPath);
            const packedRefsFile = path.join(gitDir, 'packed-refs');
            const packedRefExists = fs.existsSync(packedRefsFile)
                && fs.readFileSync(packedRefsFile, 'utf-8')
                    .split(/\r?\n/)
                    .some(line => line.endsWith(` ${refPath}`));
            if (!fs.existsSync(refFile) && !packedRefExists) {
                log(`AutoOpen: Branch ref not found in loose or packed refs: ${refPath}`);
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
