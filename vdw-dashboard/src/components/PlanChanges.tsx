import { Icon } from '../icons';
import type { Plan } from '../types';

export function PlanChanges({ plan }: { plan: Plan }) {
  const value = (plan.value || {}) as Record<string, unknown>;
  const changes: { path: string; before: unknown; after: unknown }[] = [];
  function compare(before: unknown, after: unknown, prefix = '') {
    if (after && typeof after === 'object' && !Array.isArray(after)) {
      for (const [key, entry] of Object.entries(after)) compare((before as Record<string, unknown> | null)?.[key], entry, prefix ? `${prefix}.${key}` : key);
    } else if (JSON.stringify(before ?? null) !== JSON.stringify(after ?? null)) changes.push({ path: prefix, before, after });
  }
  if (value.change) compare(value.before, value.change);
  else if (value.before_policy) compare(value.before_policy, value.policy);
  else if (value.before_config) compare(value.before_config, value.next_config);
  const names: Record<string, string> = { workspace_root: '工作空间根目录', max_store_size_gb: 'Store 容量阈值（GB）', gc_threshold_days: 'GC 保留天数',
    idle_days: '闲置天数', archive_root: '独立归档目录', auto_enabled: '参与自动归档', retain_until: '保留至',
    'archive.root': '归档根目录', 'auto.grace_hours': '归档宽限期（小时）', 'auto.scan_interval_hours': '扫描间隔（小时）', 'archive.git_backup': 'Git 备份方式' };
  const display = (v: unknown) => v == null ? '未设置 / 继承' : typeof v === 'boolean' ? v ? '开启' : '关闭' : typeof v === 'object' ? JSON.stringify(v) : String(v);
  return <>{changes.length > 0 && <div className="changes">{changes.map(change => <div className="change" key={change.path}><strong>{names[change.path] || change.path}</strong><div><span className="change-before">{display(change.before)}</span><Icon name="chevron" size={13} /><span className="change-after">{display(change.after)}</span></div></div>)}</div>}{Array.isArray(value.moves) && value.moves.length > 0 && <div className="banner compact"><Icon name="archive" /><span>将迁移 {value.moves.length} 份历史归档，复制与校验完成后切换索引。</span></div>}{plan.kind === 'auto_enable' && <div className="banner compact"><Icon name="clock" /><span>授权按当前策略自动归档现有及未来登记的工作空间。保护、活跃租约和脏状态检查持续生效。{value.install_scheduler ? '同时安装本机调度任务。' : '本平台需另行配置系统调度。'}</span></div>}</>;
}
