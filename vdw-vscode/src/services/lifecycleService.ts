import * as path from 'path';
import * as vscode from 'vscode';
import { VDWCliService } from './vdwCliService';
import { LifecycleOperation, LifecyclePlan, LifecycleWorkspace, formatLifecycleReport } from '../utils/lifecycle';
import { logError } from '../utils/logger';

/** The CLI owns all state transitions. This service only previews, confirms and observes. */
export class LifecycleService implements vscode.Disposable {
    private readonly client = `vscode-${process.pid}`;
    private readonly subscriptions: vscode.Disposable[] = [];
    private timer: NodeJS.Timeout | undefined;
    private activityTimer: NodeJS.Timeout | undefined;
    private supported = false;
    private suspended = false;
    private disposed = false;
    private workspace: LifecycleWorkspace | undefined;

    constructor(private readonly cli: VDWCliService, private readonly root: string, private readonly refresh: () => void) {}

    register(context: vscode.ExtensionContext): void {
        const actions: Record<string, () => Promise<void>> = {
            'vdw.lifecycle.archive': () => this.archive(),
            'vdw.lifecycle.restore': () => this.restore(),
            'vdw.lifecycle.protect': () => this.protect(false),
            'vdw.lifecycle.unprotect': () => this.protect(true),
            'vdw.lifecycle.retention': () => this.retention(),
            'vdw.lifecycle.archiveRoot': () => this.archiveRoot(),
            'vdw.lifecycle.status': () => this.status(),
        };
        for (const [name, action] of Object.entries(actions)) {
            context.subscriptions.push(vscode.commands.registerCommand(name, async () => {
                try {
                    if (!this.supported) throw new Error('当前 CLI 不支持生命周期管理，请使用与插件匹配的新版 VDW CLI。');
                    await action();
                } catch (error) {
                    logError('Lifecycle operation failed', error);
                    void vscode.window.showErrorMessage(String(error));
                }
            }));
        }
        this.subscriptions.push(vscode.workspace.onDidSaveTextDocument(doc => {
            if (this.inside(doc.uri.fsPath)) this.activity();
        }));
        this.subscriptions.push(vscode.workspace.onDidChangeTextDocument(event => {
            if (event.contentChanges.length && this.inside(event.document.uri.fsPath)) this.activity();
        }));
        void this.initialize();
    }

    private inside(file: string): boolean {
        const rel = path.relative(this.root, file);
        return !rel.startsWith(`..${path.sep}`) && rel !== '..' && !path.isAbsolute(rel);
    }

    private async initialize(): Promise<void> {
        try {
            const capabilities = await this.cli.executeCommand('lifecycle', ['capabilities']);
            if (capabilities.exitCode !== 0) return;
            this.supported = JSON.parse(capabilities.stdout).lifecycle === true;
            if (!this.supported) return;
            await this.updateWorkspace();
            if (this.workspace?.lifecycle_state === 'active') {
                await this.cli.executeCommand('workspace', ['touch', this.workspace.workspace_id, '--source', 'ide_open']);
                await this.heartbeat();
            }
            this.timer = setInterval(() => { void this.heartbeat(); }, 60_000);
        } catch (error) { logError('Lifecycle capability check failed', error); }
    }

    private async updateWorkspace(): Promise<void> {
        const all = await this.json<LifecycleWorkspace[]>('workspace', ['list', '--json']);
        this.workspace = all.find(w => path.resolve(w.path) === path.resolve(this.root));
        await vscode.commands.executeCommand('setContext', 'vdw.workspaceArchived', this.workspace?.lifecycle_state === 'archived');
    }

    private activity(): void {
        if (!this.supported || this.suspended || this.disposed || this.activityTimer) return;
        this.activityTimer = setTimeout(() => {
            this.activityTimer = undefined;
            if (this.workspace?.lifecycle_state === 'active') {
                void this.cli.executeCommand('workspace', ['touch', this.workspace.workspace_id, '--source', 'ide_edit']);
            }
        }, 1_000);
    }

    private async heartbeat(): Promise<void> {
        if (this.suspended || this.disposed || !this.workspace || this.workspace.lifecycle_state !== 'active') return;
        await this.cli.executeCommand('workspace', ['lease', this.workspace.workspace_id, '--client', this.client]);
    }

    private async json<T>(command: string, args: string[]): Promise<T> {
        const result = await this.cli.executeCommand(command, args);
        if (result.exitCode !== 0) throw new Error(result.stderr || result.stdout);
        return JSON.parse(result.stdout) as T;
    }

    private async choose(state?: string, many = false): Promise<LifecycleWorkspace[] | undefined> {
        const args = ['list', '--json'];
        if (state) args.push('--state', state);
        const rows = await this.json<LifecycleWorkspace[]>('workspace', args);
        const picked = await vscode.window.showQuickPick(rows.map(w => ({ label: w.name, description: w.lifecycle_state, detail: w.path, workspace: w })), { canPickMany: many, placeHolder: many ? '选择本次要归档的工作空间' : '选择工作空间' });
        if (!picked) return undefined;
        return (Array.isArray(picked) ? picked : [picked]).map(p => p.workspace);
    }

    private async confirmPlan(plan: LifecyclePlan): Promise<boolean> {
        const doc = await vscode.workspace.openTextDocument({ content: JSON.stringify(plan, null, 2), language: 'json' });
        await vscode.window.showTextDocument(doc, { preview: true });
        const skips = plan.entries.filter(e => e.reason_codes.length > 0).length;
        return await vscode.window.showWarningMessage(`执行已预览的 ${plan.kind} 计划？涉及 ${plan.entries.length} 个工作空间，预检跳过 ${skips} 个。`, { modal: true }, '确认执行') === '确认执行';
    }

    private async executePlan(plan: LifecyclePlan, confirmed = false): Promise<void> {
        if (!confirmed && !await this.confirmPlan(plan)) return;
        const report = await vscode.window.withProgress({ location: vscode.ProgressLocation.Notification, title: 'VDW 正在执行计划', cancellable: false }, async progress => {
            let polling = false;
            const timer = setInterval(async () => {
                if (polling) return;
                polling = true;
                try {
                    const current = await this.json<LifecyclePlan>('lifecycle', ['plan', 'show', plan.plan_id, '--json']);
                    if (current.operation_id) {
                        const op = await this.json<LifecycleOperation>('lifecycle', ['operation', 'status', current.operation_id, '--json']);
                        const p = op.progress;
                        progress.report({ message: `${p.phase} ${p.archives_done}/${p.archives_total}；复制 ${p.bytes_copied}/${p.bytes_total} bytes；校验 ${p.bytes_verified} bytes` });
                    }
                } catch { /* The plan may not have acquired its operation id yet. */ }
                finally { polling = false; }
            }, 500);
            try {
                const result = await this.cli.executeCommand('lifecycle', ['plan', 'execute', plan.plan_id, '--confirm', '--json']);
                if (!result.stdout.trim()) throw new Error(result.stderr);
                const op = JSON.parse(result.stdout) as LifecycleOperation;
                return op;
            } finally { clearInterval(timer); }
        });
        const document = await vscode.workspace.openTextDocument({ content: formatLifecycleReport(report), language: 'plaintext' });
        await vscode.window.showTextDocument(document, { preview: false });
        await this.updateWorkspace();
        this.refresh();
    }

    private async archive(): Promise<void> {
        try {
            let selected = await this.choose('active', true);
            while (selected?.length) {
                if (selected.some(w => path.resolve(w.path) === path.resolve(this.root)) && vscode.workspace.textDocuments.some(d => d.isDirty && this.inside(d.uri.fsPath))) {
                    throw new Error('当前工作空间有尚未保存的编辑，请先保存或取消编辑后再预览归档。');
                }
                const plan = await this.json<LifecyclePlan>('workspace', ['archive', ...selected.map(w => w.workspace_id), '--client', this.client, '--preview', '--json']);
                const doc = await vscode.workspace.openTextDocument({ content: JSON.stringify(plan, null, 2), language: 'json' });
                await vscode.window.showTextDocument(doc, { preview: true });
                const action = await vscode.window.showQuickPick(['确认当前计划', '调整工作空间选择', '取消'], { placeHolder: '预览后确认或调整' });
                if (action === '调整工作空间选择') { selected = await this.choose('active', true); continue; }
                if (action === '确认当前计划') {
                    if (selected.some(w => path.resolve(w.path) === path.resolve(this.root)) && vscode.workspace.textDocuments.some(d => d.isDirty && this.inside(d.uri.fsPath))) {
                        throw new Error('预览后出现尚未保存的编辑，请重新整理后预览。');
                    }
                    this.suspended = true;
                    if (this.activityTimer) { clearTimeout(this.activityTimer); this.activityTimer = undefined; }
                    await this.executePlan(plan, true);
                }
                return;
            }
        } finally { this.suspended = false; await this.heartbeat(); }
    }

    private async restore(): Promise<void> {
        const selected = await this.choose('archived');
        if (!selected?.[0]?.latest_archive_id) return;
        const plan = await this.json<LifecyclePlan>('workspace', ['restore', selected[0].latest_archive_id, '--preview', '--json']);
        await this.executePlan(plan);
    }

    private async protect(remove: boolean): Promise<void> {
        const selected = await this.choose();
        if (!selected?.length) return;
        if (remove) {
            await this.executePlan(await this.json<LifecyclePlan>('workspace', ['protect', 'remove', selected[0].workspace_id, '--preview', '--json']));
        } else {
            const reason = await vscode.window.showInputBox({ prompt: '不清理原因', value: '长期保留' });
            if (reason === undefined) return;
            const result = await this.cli.executeCommand('workspace', ['protect', 'add', selected[0].workspace_id, '--reason', reason]);
            if (result.exitCode !== 0) throw new Error(result.stderr || result.stdout);
            void vscode.window.showInformationMessage(`已保护 ${selected[0].name}`);
        }
    }

    private async retention(): Promise<void> {
        const value = await vscode.window.showInputBox({ prompt: '默认闲置天数', value: '30', validateInput: v => /^[1-9]\d*$/.test(v) ? undefined : '请输入正整数' });
        if (!value) return;
        await this.executePlan(await this.json<LifecyclePlan>('config', ['set', 'idle-days', value, '--preview', '--json']));
    }

    private async archiveRoot(): Promise<void> {
        const value = await vscode.window.showInputBox({ prompt: '新的归档绝对路径或 ~/ 路径；确认后自动同步历史归档' });
        if (!value) return;
        await this.executePlan(await this.json<LifecyclePlan>('config', ['set', 'archive-root', value, '--preview', '--json']));
    }

    private async status(): Promise<void> {
        const rows = await this.json<LifecycleWorkspace[]>('workspace', ['list', '--json']);
        const document = await vscode.workspace.openTextDocument({ content: JSON.stringify(rows, null, 2), language: 'json' });
        await vscode.window.showTextDocument(document, { preview: true });
    }

    dispose(): void {
        this.disposed = true;
        if (this.timer) clearInterval(this.timer);
        if (this.activityTimer) clearTimeout(this.activityTimer);
        for (const sub of this.subscriptions) sub.dispose();
        if (this.supported && this.workspace) void this.cli.executeCommand('workspace', ['lease', this.workspace.workspace_id, '--client', this.client, '--release']);
    }
}
