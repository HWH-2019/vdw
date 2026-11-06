export const date = (value?: string | null) => value ? new Date(value).toLocaleString('zh-CN', { hour12: false }) : '暂无记录';
export const modifiedDate = (value: string | null) => value ? new Intl.DateTimeFormat('zh-CN', {
  timeZone: 'Asia/Shanghai', year: 'numeric', month: '2-digit', day: '2-digit',
}).format(new Date(value)).replaceAll('/', '-') : '暂无记录';
const labels: Record<string, string> = { inspection_failed: '归档预检未通过', dirty: '存在未提交变更', ignored_files_need_review: '忽略文件需要保留确认', protected: '工作空间受保护', active_lease: '仍有编辑器使用租约', retain_until: '仍在保留期内', already_archived: '已归档', unfinished_operation: '有未完成操作', excluded_by_user: '已手动排除',  active: '活跃', pending_archive: '将归档', archived: '已归档', archiving: '归档中', restoring: '恢复中', ready: '正常', missing: '链接缺失',
  conflict: '链接冲突', detached: '链接已脱离', invalid_worktree: '工作树异常', unowned: '未被 VDW 管理', pending: '待检测',
  available: '远端正常', branch_missing: '远端分支不存在', not_found_or_forbidden: '远端仓库不存在或无权限', auth_error: '认证失败', network_error: '网络异常',
  deleted: '远端已删除', local: '本地仓库', completed: '已完成', completed_with_skips: '已完成（含跳过）', partial_failure: '部分失败', restored: '已恢复', interrupted: '已中断', failed: '失败', skipped: '已跳过', unfinished: '未完成', running: '执行中', queued: '等待执行',
  paused: '已暂停', cancelled: '已取消', undo: '撤销操作', trash_restore: '从回收站恢复', trash: '移到回收站', trashed: '已移到回收站', trashing: '正在移到回收站', archive: '归档', restore: '恢复', config: '配置更新', settings: '基础配置', auto_enable: '启用自动归档', auto_disable: '关闭自动归档', migration: '归档迁移' };
export const label = (v: string) => labels[v] || v;
export const terminal = (state: string) => !['queued', 'running', 'pending'].includes(state);
