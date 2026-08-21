import * as path from 'path';
import * as fs from 'fs';
import * as vscode from 'vscode';

import { VDWCliService } from '../services/vdwCliService';
import {
    LinkConflict,
    LinkConflictReport,
    LinkConflictResolutionSide,
} from '../types/vdwTypes';
import { log, logError } from '../utils/logger';
import { formatConflictReason } from '../utils/linkConflicts';

const OPEN_DIFF = '打开文件对比';
const ACCEPT_ALL_STORE = '全部接受 Store';
const ACCEPT_ALL_WORKSPACE = '全部接受工作区';
const ACCEPT_STORE = '接受 Store';
const ACCEPT_WORKSPACE = '接受工作区';
const BACK_TO_LIST = '返回冲突列表';
const CANCEL = '取消';

/**
 * Resolve legacy projection conflicts before migration/unlink/remove. Returns false when the
 * user cancels or a conflict cannot be resolved safely.
 */
export async function ensureLinkConflictsResolved(
    cliService: VDWCliService,
    repoUrl?: string,
): Promise<boolean> {
    let report: LinkConflictReport;
    try {
        report = await cliService.getLinkConflicts(repoUrl);
    } catch (error) {
        logError('Failed to inspect VDW link conflicts', error);
        vscode.window.showErrorMessage(`检查 VDW 文件冲突失败: ${error}`);
        return false;
    }

    while (report.conflict_count > 0) {
        const resolvableCount = report.conflicts.filter(conflict => conflict.resolvable).length;
        const message = [
            `检测到 ${report.conflict_count} 个 Store 与工作区内容或文件拓扑冲突。`,
            '请先打开对应文件进行对比，或批量选择保留一侧。',
            '未采用的一侧会备份到工作区 .vdw/conflict-backups。',
            resolvableCount < report.conflict_count
                ? `其中 ${report.conflict_count - resolvableCount} 个冲突需要手工处理。`
                : '',
        ].filter(Boolean).join('\n\n');

        const actions = resolvableCount === report.conflict_count
            ? [OPEN_DIFF, ACCEPT_ALL_STORE, ACCEPT_ALL_WORKSPACE, CANCEL]
            : [OPEN_DIFF, CANCEL];
        const choice = await vscode.window.showWarningMessage(
            message,
            { modal: true },
            ...actions,
        );

        if (!choice || choice === CANCEL) {
            return false;
        }
        if (choice === ACCEPT_ALL_STORE || choice === ACCEPT_ALL_WORKSPACE) {
            const accepted: LinkConflictResolutionSide = choice === ACCEPT_ALL_STORE
                ? 'store'
                : 'workspace';
            if (!await resolveConflicts(cliService, report, accepted, repoUrl)) {
                return false;
            }
            report = await cliService.getLinkConflicts(repoUrl);
            continue;
        }

        const selected = await pickConflict(report.conflicts);
        if (!selected) {
            continue;
        }
        await openConflictDiff(selected);
        if (!selected.resolvable) {
            vscode.window.showErrorMessage(
                `该冲突无法自动解决：${formatConflictReason(selected.reason)}。请手工处理后重试。`,
            );
            continue;
        }

        const decision = await vscode.window.showWarningMessage(
            `正在对比 ${selected.relative_path}。请选择要保留的版本；另一侧会自动备份。`,
            ACCEPT_STORE,
            ACCEPT_WORKSPACE,
            BACK_TO_LIST,
        );
        if (decision === ACCEPT_STORE || decision === ACCEPT_WORKSPACE) {
            const accepted: LinkConflictResolutionSide = decision === ACCEPT_STORE
                ? 'store'
                : 'workspace';
            if (!await resolveConflicts(
                cliService,
                report,
                accepted,
                repoUrl,
                [selected.relative_path],
            )) {
                return false;
            }
            report = await cliService.getLinkConflicts(repoUrl);
        }
    }

    return true;
}

async function pickConflict(conflicts: LinkConflict[]): Promise<LinkConflict | undefined> {
    const items = conflicts.map(conflict => ({
        label: `$(diff) ${conflict.relative_path}`,
        description: formatConflictReason(conflict.reason),
        detail: `Store: ${conflict.source}  |  工作区: ${conflict.target}`,
        conflict,
    }));
    const selected = await vscode.window.showQuickPick(items, {
        title: '选择要对比的 VDW 冲突文件',
        placeHolder: 'Store 显示在左侧，工作区显示在右侧',
        matchOnDescription: true,
        matchOnDetail: true,
    });
    return selected?.conflict;
}

async function openConflictDiff(conflict: LinkConflict): Promise<void> {
    const sourceExists = fs.existsSync(conflict.source);
    const targetExists = fs.existsSync(conflict.target);
    if (!sourceExists || !targetExists) {
        const existingPath = sourceExists ? conflict.source : conflict.target;
        if (existingPath) {
            const document = await vscode.workspace.openTextDocument(vscode.Uri.file(existingPath));
            await vscode.window.showTextDocument(document, { preview: true });
        }
        vscode.window.showInformationMessage(
            `${formatConflictReason(conflict.reason)}，缺失的一侧无法打开对比；请选择接受 Store 或工作区来决定保留文件还是删除文件。`,
        );
        return;
    }
    const title = `VDW 冲突：Store ↔ 工作区 — ${path.basename(conflict.relative_path)}`;
    await vscode.commands.executeCommand(
        'vscode.diff',
        vscode.Uri.file(conflict.source),
        vscode.Uri.file(conflict.target),
        title,
    );
}

async function resolveConflicts(
    cliService: VDWCliService,
    report: LinkConflictReport,
    accepted: LinkConflictResolutionSide,
    repoUrl?: string,
    relativePaths: string[] = [],
): Promise<boolean> {
    const count = relativePaths.length || report.conflict_count;
    const sideLabel = accepted === 'store' ? 'Store' : '工作区';
    const confirmed = await vscode.window.showWarningMessage(
        `确认对 ${count} 个冲突接受${sideLabel}版本？\n\n现有文件会先备份，随后按选择同步内容、创建或删除文件。`,
        { modal: true },
        `确认接受${sideLabel}`,
        CANCEL,
    );
    if (confirmed !== `确认接受${sideLabel}`) {
        return false;
    }

    try {
        const result = await cliService.resolveLinkConflicts(accepted, repoUrl, relativePaths);
        log(`Resolved ${result.resolved_count} link conflict(s); backup: ${result.backup_root}`);
        vscode.window.showInformationMessage(
            `已解决 ${result.resolved_count} 个冲突。备份目录：${result.backup_root}`,
        );
        return true;
    } catch (error) {
        logError('Failed to resolve VDW link conflicts', error);
        vscode.window.showErrorMessage(`解决 VDW 文件冲突失败: ${error}`);
        return false;
    }
}
