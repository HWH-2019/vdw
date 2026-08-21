import * as vscode from 'vscode';
import * as fs from 'fs';
import * as path from 'path';
import { ConfigService } from './configService';
import { log, logError } from '../utils/logger';
import { getEditableWorktreePaths } from '../utils/gitRepositories';

/**
 * AutoOpenService - 在 add、link、relink 操作后以及扩展激活时，
 * 自动在 VS Code Source Control 面板中注册可编辑依赖的 Git 仓库。
 * 
 * Source Control 必须注册 dep.local 指向的完整 Store worktree。工作区内的
 * packages/<repo> 只是 scope 裁剪后的投影，注册它会把未投影的已提交文件
 * 误报为删除。Qoder 拒绝公开 API 打开工作区外仓库时使用内置强制命令兜底。
 */
export class AutoOpenService implements vscode.Disposable {
    private configService: ConfigService;
    private _disposables: vscode.Disposable[] = [];
    private gitOperationQueue: Promise<void> = Promise.resolve();
    private autoOpenSuspensionCount = 0;

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
            let settled = false;
            let sub: vscode.Disposable | undefined;
            const timer = setTimeout(() => {
                if (settled) return;
                settled = true;
                sub?.dispose();
                log('AutoOpen: Timed out waiting for Git extension to initialize');
                resolve(false);
            }, timeoutMs);
            const finish = () => {
                if (settled) return;
                settled = true;
                clearTimeout(timer);
                sub?.dispose();
                log('AutoOpen: Git extension initialized');
                resolve(true);
            };
            sub = gitAPI.onDidChangeState((state: string) => {
                if (state === 'initialized') {
                    finish();
                }
            });
            // Avoid losing the transition if initialization completed between
            // the first state check and listener registration.
            if (gitAPI.state === 'initialized') {
                finish();
            }
        });
    }

    /**
     * Open and refresh every linked editable dependency's complete worktree.
     */
    async openAllEditableRepositories(repoIdentifier?: string): Promise<void> {
        if (!this.isAutoOpenEnabled()) return;
        if (this.autoOpenSuspensionCount > 0) {
            log('AutoOpen: Link mutation in progress, skipping watcher-triggered repository registration');
            return;
        }

        return this.enqueueGitOperation(async () => {
            const config = this.configService.getConfig();
            if (!config) return;

            const worktreePaths = getEditableWorktreePaths(config, repoIdentifier);

            await this.syncManagedRepositories(worktreePaths);
        });
    }

    /** Close managed worktrees before unlink/relink changes their lifecycle. */
    async closeEditableRepositories(repoIdentifier?: string): Promise<void> {
        if (!this.isAutoOpenEnabled()) return;

        return this.enqueueGitOperation(async () => {
            const config = this.configService.getConfig();
            if (!config) return;

            const worktreePaths = getEditableWorktreePaths(config, repoIdentifier);
            await this.closeManagedRepositories(worktreePaths);
        });
    }

    /** Prevent config watchers from reopening repositories during link mutation. */
    suspendAutoOpen(): void {
        this.autoOpenSuspensionCount++;
    }

    resumeAutoOpen(): void {
        if (this.autoOpenSuspensionCount > 0) {
            this.autoOpenSuspensionCount--;
        }
    }

    /** Serialize Git API operations triggered by commands and .vdwrc watchers. */
    private enqueueGitOperation(operation: () => Promise<void>): Promise<void> {
        const queued = this.gitOperationQueue.then(operation, operation);
        this.gitOperationQueue = queued.catch(() => undefined);
        return queued;
    }

    private async syncManagedRepositories(worktreePaths: string[]): Promise<void> {
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

            for (const worktreePath of worktreePaths) {
                if (!worktreePath || worktreePath.trim() === '') continue;
                if (!this.isValidGitRepository(worktreePath)) continue;

                const normalizedPath = this.normalizeRepositoryPath(worktreePath);
                if (seen.has(normalizedPath)) continue;
                seen.add(normalizedPath);

                let repo = this.findExactRepository(gitAPI, normalizedPath);
                if (!repo) {
                    log(`AutoOpen: Opening managed worktree: ${worktreePath}`);
                    try {
                        repo = await this.openExactRepository(
                            gitAPI,
                            worktreePath,
                            normalizedPath,
                        );
                    } catch (err) {
                        logError(`AutoOpen: Failed to open managed worktree: ${worktreePath}`, err);
                        continue;
                    }
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

    private async openExactRepository(
        gitAPI: any,
        worktreePath: string,
        normalizedPath: string,
        timeoutMs: number = 10000,
    ): Promise<any | undefined> {
        let sub: vscode.Disposable | undefined;
        let timer: NodeJS.Timeout | undefined;
        let settled = false;
        let finish: (repo: any | undefined) => void = () => undefined;
        const registered = new Promise<any | undefined>(resolve => {
            finish = (repo: any | undefined) => {
                if (settled) return;
                settled = true;
                if (timer) clearTimeout(timer);
                sub?.dispose();
                resolve(repo);
            };
            sub = gitAPI.onDidOpenRepository((repo: any) => {
                if (this.isExactRepository(repo, normalizedPath)) {
                    finish(repo);
                }
            });
            timer = setTimeout(() => finish(undefined), timeoutMs);
        });

        try {
            const existing = this.findExactRepository(gitAPI, normalizedPath);
            if (existing) finish(existing);

            let current = this.findExactRepository(gitAPI, normalizedPath);
            if (!current && !settled) {
                const commands = await vscode.commands.getCommands(true);
                if (commands.includes('git.forceOpenRepository')) {
                    // Qoder's public API deliberately rejects repositories outside
                    // the current workspace. This built-in command bypasses only
                    // that location check and still performs normal Git validation.
                    log(`AutoOpen: Force-opening external managed worktree: ${worktreePath}`);
                    await vscode.commands.executeCommand('git.forceOpenRepository', worktreePath);
                } else if (typeof gitAPI.openRepository === 'function') {
                    const opened = await gitAPI.openRepository(vscode.Uri.file(worktreePath));
                    if (this.isExactRepository(opened, normalizedPath)) {
                        finish(opened);
                    }
                } else {
                    await vscode.commands.executeCommand('git.openRepository', worktreePath);
                }
                current = this.findExactRepository(gitAPI, normalizedPath);
            }
            if (current) finish(current);
            return await registered;
        } catch (error) {
            finish(undefined);
            throw error;
        }
    }

    private async closeManagedRepositories(worktreePaths: string[]): Promise<void> {
        try {
            const ready = await this.waitForGitReady();
            if (!ready) return;

            const gitExtension = vscode.extensions.getExtension('vscode.git')?.exports;
            const gitAPI = gitExtension?.getAPI(1);
            if (!gitAPI) return;

            for (const worktreePath of worktreePaths) {
                const normalizedPath = this.normalizeRepositoryPath(worktreePath);
                const repo = this.findExactRepository(gitAPI, normalizedPath);
                if (!repo) continue;

                log(`AutoOpen: Closing managed worktree before link mutation: ${worktreePath}`);
                const closed = await this.closeExactRepository(
                    gitAPI,
                    repo,
                    normalizedPath,
                );
                if (!closed) {
                    throw new Error(
                        `Git extension did not close the exact worktree root: ${worktreePath}`,
                    );
                }
            }
        } catch (err) {
            logError('AutoOpen: Failed to close managed Git worktrees', err);
            throw err;
        }
    }

    private async closeExactRepository(
        gitAPI: any,
        repo: any,
        normalizedPath: string,
        timeoutMs: number = 10000,
    ): Promise<boolean> {
        let sub: vscode.Disposable | undefined;
        let timer: NodeJS.Timeout | undefined;
        let settled = false;
        let finish: (closed: boolean) => void = () => undefined;
        const closed = new Promise<boolean>(resolve => {
            finish = (result: boolean) => {
                if (settled) return;
                settled = true;
                if (timer) clearTimeout(timer);
                sub?.dispose();
                resolve(result);
            };
            if (typeof gitAPI.onDidCloseRepository === 'function') {
                sub = gitAPI.onDidCloseRepository((closedRepo: any) => {
                    if (this.isExactRepository(closedRepo, normalizedPath)) {
                        finish(true);
                    }
                });
            }
            timer = setTimeout(() => finish(false), timeoutMs);
        });

        try {
            // The public Repository wrapper has no close() method. The built-in
            // command resolves this URI to the internal repository and closes it.
            await vscode.commands.executeCommand('git.close', repo.rootUri);
            if (!this.findExactRepository(gitAPI, normalizedPath)) {
                finish(true);
            }
            return await closed;
        } catch (error) {
            finish(false);
            throw error;
        }
    }

    private findExactRepository(gitAPI: any, normalizedPath: string): any | undefined {
        return gitAPI.repositories.find((repo: any) =>
            this.isExactRepository(repo, normalizedPath)
        );
    }

    private isExactRepository(repo: any, normalizedPath: string): boolean {
        return Boolean(repo?.rootUri?.fsPath)
            && this.normalizeRepositoryPath(repo.rootUri.fsPath) === normalizedPath;
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
