export interface LifecycleWorkspace {
    workspace_id: string;
    name: string;
    path: string;
    lifecycle_state: string;
    latest_archive_id?: string;
}

export interface LifecyclePlan {
    plan_id: string;
    operation_id?: string;
    kind: string;
    entries: Array<{ workspace_id: string; name: string; reason_codes: string[] }>;
}

export interface LifecycleOperation {
    operation_id: string;
    kind: string;
    state: string;
    summary: {
        scope_total: number;
        archived_count: number;
        restored_count: number;
        not_archived_count: number;
        skipped_count: number;
        failed_count: number;
        unfinished_count: number;
    };
    results: Array<{
        name: string;
        workspace_id: string;
        outcome: string;
        reason_codes: string[];
        message: string;
        archive_path?: string;
        repositories: Array<{ path: string; dirty: { staged: number; unstaged: number; conflicts: number; untracked: number; unknown_ignored: number } }>;
    }>;
    progress: { phase: string; archives_done: number; archives_total: number; bytes_copied: number; bytes_total: number; bytes_verified: number; current_file?: string };
    error?: string;
}

const reasons: Record<string, string> = {
    dirty: '存在未提交改动，整个工作空间跳过',
    ignored_files_need_review: '存在尚未确认用途的 Git 忽略文件',
    protected: '处于不清理白名单',
    active_lease: '正在被客户端使用',
    not_due: '未达到闲置时长',
    grace_period: '仍处于归档宽限期',
    excluded_by_user: '已在预览中排除',
    already_archived: '此前已归档，本次未重复执行',
    inspection_failed: '检查失败',
    plan_stale: '计划已失效，需要重新预览',
};

export function formatLifecycleReport(op: LifecycleOperation): string {
    const s = op.summary;
    const lines = [`操作 ${op.operation_id}：${op.state}`];
    if (op.kind === 'archive') {
        lines.push(`本次涉及 ${s.scope_total} 个工作空间；已归档 ${s.archived_count} 个；未归档 ${s.not_archived_count} 个，其中跳过 ${s.skipped_count} 个、失败 ${s.failed_count} 个、未完成 ${s.unfinished_count} 个。`);
    }
    for (const row of op.results) {
        const status = ({ archived: '已归档', restored: '已恢复', skipped: '未归档（跳过）', failed: '未完成（失败）', unfinished: '未完成' } as Record<string, string>)[row.outcome] ?? row.outcome;
        lines.push('', `${row.name} [${row.workspace_id}]：${status}`);
        for (const code of row.reason_codes) lines.push(`  ${reasons[code] ?? code}`);
        if (row.message) lines.push(`  ${row.message}`);
        if (row.archive_path) lines.push(`  归档位置：${row.archive_path}`);
        for (const repo of row.repositories) {
            const d = repo.dirty;
            if (d.staged + d.unstaged + d.conflicts + d.untracked + d.unknown_ignored > 0) {
                lines.push(`  ${repo.path}：暂存 ${d.staged}，未暂存 ${d.unstaged}，冲突 ${d.conflicts}，未跟踪 ${d.untracked}，待检查忽略项 ${d.unknown_ignored}`);
            }
        }
    }
    if (op.error) lines.push('', op.error);
    return lines.join('\n');
}
