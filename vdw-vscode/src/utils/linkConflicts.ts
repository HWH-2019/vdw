import type {
    LinkConflictReason,
    LinkConflictResolutionSide,
} from '../types/vdwTypes';

export function formatConflictReason(reason: LinkConflictReason): string {
    switch (reason) {
        case 'content_differs': return '文件内容不同';
        case 'metadata_differs': return '文件权限或元数据不同';
        case 'source_missing': return 'Store 文件缺失';
        case 'target_unreadable': return '工作区文件无法读取';
        case 'target_type_changed': return '工作区文件类型已改变';
        case 'link_target_changed': return '链接目标已改变';
    }
}

export function buildConflictListArgs(repoUrl?: string): string[] {
    const args = ['list', '--json'];
    if (repoUrl) {
        args.push('--repo', repoUrl);
    }
    return args;
}

export function buildConflictResolveArgs(
    accepted: LinkConflictResolutionSide,
    repoUrl?: string,
    relativePaths: string[] = [],
): string[] {
    const args = ['resolve', '--accept', accepted, '--json'];
    if (repoUrl) {
        args.push('--repo', repoUrl);
    }
    for (const relativePath of relativePaths) {
        args.push('--path', relativePath);
    }
    return args;
}
