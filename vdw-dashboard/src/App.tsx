import { useCallback, useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { api, post } from './api';
import { Icon } from './icons';
import { date, label, terminal } from './utils';
import { ArchiveWarnings, Badge } from './components/ui';
import { PlanChanges } from './components/PlanChanges';
import { WorkspaceCard } from './components/WorkspaceCard';
import { Masonry } from './components/Masonry';
import { UndoDialog } from './components/UndoDialog';
import { TrashDialog } from './components/TrashDialog';
import { useOperationJobs } from './hooks/useOperationJobs';
import { Detail, RestoreDialog } from './components/WorkspaceDetail';
import { ConfigPage } from './pages/Configuration';
import { Operations } from './pages/Operations';
import type { Configuration, DiagnosticDelivery, Job, OpenResult, Operation, Plan, Remote, Snapshot, Workspace } from './types';

type DiagnosticNotice = DiagnosticDelivery & { editor: string; name: string };

export function App() {
  const client = useQueryClient();
  const [page, setPage] = useState('workspaces');
  const [search, setSearch] = useState('');
  const [filter, setFilter] = useState('all');
  const [sort, setSort] = useState('modified');
  const [auto, setAuto] = useState(true);
  const [detail, setDetail] = useState<Workspace | null>(null);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [restore, setRestore] = useState<Workspace | null>(null);
  const [notice, setNotice] = useState<{ text: string; error?: boolean; diagnostics?: DiagnosticNotice } | null>(null);
  const [diagnostic, setDiagnostic] = useState<DiagnosticNotice | null>(null);
  const [busy, setBusy] = useState('');
  const [undoErrors, setUndoErrors] = useState<Record<string, string>>({});
  const busyRef = useRef(false);
  const mountedAt = useRef(Date.now());
  const remoteGeneration = useRef(0);
  const remoteInFlight = useRef(new Set<string>());
  const [remoteChecking, setRemoteChecking] = useState<Set<string>>(new Set());
  const reloadQueue = useRef<Promise<unknown>>(Promise.resolve());
  const [remote, setRemote] = useState<Record<string, Record<string, Remote>>>({});
  const config = useQuery({ queryKey: ['config'], queryFn: () => api<Configuration>('/config'), staleTime: 30000 });
  const snapshot = useQuery({ queryKey: ['workspaces'], queryFn: ({ signal }) => api<Snapshot>('/workspaces', { signal }),
    refetchInterval: auto ? (config.data?.preferences.refresh_seconds || 15) * 1000 : false, refetchIntervalInBackground: false });
  const operations = useQuery({ queryKey: ['operations'], queryFn: () => api<Operation[]>('/operations'), enabled: page === 'operations', refetchInterval: 5000 });
  const notify = useCallback((text: string, error = false, diagnostics?: DiagnosticNotice) => setNotice({ text, error, diagnostics }), []);
  const reloadData = useCallback(() => {
    const run = reloadQueue.current.catch(() => {}).then(async () => {
      await client.cancelQueries({ queryKey: ['workspaces'] });
      const fresh = await post<Snapshot>('/refresh', {});
      client.setQueryData(['workspaces'], fresh);
      await Promise.all(['config', 'operations'].map(key => client.invalidateQueries({ queryKey: [key], refetchType: 'all' })));
    });
    reloadQueue.current = run;
    return run;
  }, [client]);
  const monitor = useOperationJobs(useCallback(async results => {
    const result = results[results.length - 1];
    // Reload recovered jobs too, without replaying old success messages on page load.
    if (!result.operation?.updated_at || Date.parse(result.operation.updated_at) >= mountedAt.current) {
      notify(result.error || result.operation?.error || (result.operation?.kind === 'trash' && result.state === 'completed' ? '工作空间已移到系统回收站' : `操作${label(result.state)}，请查看逐项结果`), ['failed', 'partial_failure', 'unfinished'].includes(result.state));
    }
    setDetail(null);
    remoteGeneration.current += 1;
    setRemote({});
    try { await reloadData(); } catch (error) { notify(`操作已结束，重新加载失败：${String(error)}`, true); }
  }, [notify, reloadData]));
  const jobs = { data: monitor.current };
  const jobId = monitor.current?.plan_id || '';
  useEffect(() => { if (notice) { const id = setTimeout(() => setNotice(null), notice.error ? 15000 : 6000); return () => clearTimeout(id); } }, [notice]);
  useEffect(() => {
    const preference = config.data?.preferences.theme || 'light';
    document.documentElement.dataset.theme = preference === 'system' ? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') : preference;
  }, [config.data?.preferences.theme]);
  async function action<T>(key: string, run: () => Promise<T>, done?: (result: T) => void) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(key);
    try { const result = await run(); done?.(result); } catch (error) { notify(String(error instanceof Error ? error.message : error), true); }
    finally { busyRef.current = false; setBusy(''); }
  }
  const preview = (path: string, body: unknown) => action('preview', () => post<Plan>(path, body), setPlan);
  const previewUndo = (operation: Operation) => action(`undo:${operation.operation_id}`, async () => {
    setUndoErrors(errors => ({ ...errors, [operation.operation_id]: '' }));
    try { return await post<Plan>(`/operations/${operation.operation_id}/undo/preview`, {}); }
    catch (error) {
      setUndoErrors(errors => ({ ...errors, [operation.operation_id]: error instanceof Error ? error.message : String(error) }));
      throw error;
    }
  }, setPlan);
  const checkRemote = async (row: Workspace) => {
    const id = row.workspace.workspace_id;
    if (remoteInFlight.current.has(id)) return;
    const generation = remoteGeneration.current;
    remoteInFlight.current.add(id);
    setRemoteChecking(new Set(remoteInFlight.current));
    try {
      const result = await post<Record<string, Remote>>(`/workspaces/${id}/remote`, {});
      if (generation === remoteGeneration.current) setRemote(current => ({ ...current, [id]: result }));
    } catch (error) { notify(`${row.workspace.name}：${String(error)}`, true); }
    finally { remoteInFlight.current.delete(id); setRemoteChecking(new Set(remoteInFlight.current)); }
  };
  const openWorkspace = (row: Workspace, editor?: string) => action(`open:${row.workspace.workspace_id}`,
    () => post<OpenResult>(`/workspaces/${row.workspace.workspace_id}/open`, { editor }), result => {
      setRemote(current => ({ ...current, [row.workspace.workspace_id]: result.remote }));
      void client.invalidateQueries({ queryKey: ['workspaces'] });
      const launchFailed = result.state === 'launch_failed';
      if (!result.diagnostics) { notify(launchFailed ? result.launch_error || '编辑器启动失败，请检查配置' : '已向编辑器发送打开请求', launchFailed); return; }
      const details = { ...result.diagnostics, editor: result.editor, name: row.workspace.name };
      const editorName = result.editor === 'qoder' ? 'Qoder' : result.editor === 'codex' ? 'Codex' : '编辑器';
      if (details.delivery === 'prefill_requested') {
        notify(`${launchFailed ? '编辑器启动失败；' : ''}已请求在 ${editorName} 对话框预填异常说明${result.editor === 'qoder' ? '，请在 Qoder 确认' : ''}${details.clipboard_copied ? '；已备份到剪贴板' : ''}`, launchFailed, details);
      } else if (details.clipboard_copied) {
        notify(`${launchFailed ? '编辑器启动失败' : '已发送打开请求'}；异常说明已复制到剪贴板，请粘贴到编辑器对话框`, launchFailed, details);
      } else {
        setDiagnostic(details);
        notify(`${launchFailed ? '编辑器启动失败' : '已发送打开请求'}，请复制异常说明到编辑器对话框`, true, details);
      }
    });
  const rows = snapshot.data?.workspaces || [];
  const archived = rows.filter(r => r.workspace.lifecycle_state === 'archived');
  const selectStatus = (next: string) => { setFilter(next); setPage('workspaces'); };
  const filtered = rows
    .filter(row => !search || `${row.workspace.name} ${row.workspace.path} ${row.repositories.map(r => `${r.name} ${r.branch} ${r.declared_revision}`).join(' ')}`.toLowerCase().includes(search.toLowerCase()))
    .filter(row => filter === 'all' || row.status.state === filter)
    .sort((a, b) => sort === 'name' ? a.workspace.name.localeCompare(b.workspace.name) : sort === 'issues' ? b.issue_count - a.issue_count : (b.last_modified_at || '').localeCompare(a.last_modified_at || ''));
  const archiveBlocked = plan?.kind === 'archive' && !plan.entries.some(entry => !entry.excluded && entry.reason_codes.length === 0);
  const titles: Record<string, [string, string]> = {
    workspaces: ['工作空间', '把多仓库开发，放回一个清晰的工作空间。'],
    config: ['配置', '管理 VDW 的全局策略与本机偏好。'], operations: ['操作记录', '追踪每次归档、恢复与配置变更的实际结果。'],
  };
  return <div className="app">
    <aside className="sidebar">
      <a href="#" className="brand" onClick={e => { e.preventDefault(); setPage('workspaces'); }}><span className="brand-mark">v<span>dw</span></span><span>Workspace<span className="brand-sub">VIRTUAL DEV WORKSPACE</span></span></a>
      <div className="nav-label">工作台</div>
      <nav>{[['workspaces', 'grid', '工作空间'], ['operations', 'clock', '操作记录'], ['config', 'settings', '配置']].map(([key, icon, name]) =>
        <button key={key} className={`nav-item ${page === key ? 'selected' : ''}`} onClick={() => { setPage(key); setFilter('all'); }}><Icon name={icon} />{name}{key === 'workspaces' && <span className="nav-count">{rows.length}</span>}</button>)}</nav>
      <div className="sidebar-note"><span className="green-dot" /> 本地运行<div>数据留在你的设备上</div></div>
      <div className="sidebar-bottom"><span className="version">VDW {snapshot.data?.version || 'CLI'}</span><span className="small muted">内置操作台</span></div>
    </aside>
    <div className="main">
      <header className="topbar"><span className="breadcrumb">工作台 <Icon name="chevron" size={14} /> <strong>{titles[page][0]}</strong></span><span className="connection"><span className={snapshot.isError ? 'orange-dot' : 'green-dot'} />{snapshot.isError ? '连接异常' : '本地连接'}<span className="mono">{window.location.host}</span></span></header>
      <main className="content">
        <div className="page-heading"><div><div className="eyebrow">YOUR DEVELOPMENT, ORGANIZED</div><h1>{titles[page][0]}</h1><p>{titles[page][1]}</p></div><div className="heading-actions">{page === 'workspaces' && <><label className="auto"><input type="checkbox" checked={auto} onChange={e => setAuto(e.target.checked)} /> 自动刷新</label><button className="button" disabled={!!busy || snapshot.isFetching} onClick={() => action('refresh', reloadData)}><Icon name="refresh" className={snapshot.isFetching ? 'spinning' : ''} /> 刷新</button></>}</div></div>
        {snapshot.error && <div className="banner error"><Icon name="alert" /><span>{snapshot.error.message}{snapshot.data && ' · 当前展示上次检测结果，数据可能已过期'}</span><button onClick={() => void snapshot.refetch()}>重试</button></div>}
        {jobs.data && !terminal(jobs.data.state) && <div className="banner"><Icon name="refresh" className="spinning" /><span>操作{label(jobs.data.state)} · {jobId}</span><button onClick={() => setPage('operations')}>查看进度</button></div>}
        {page === 'workspaces' && <>
          <div className="stats">{[
            ['全部空间', rows.length, 'folder', 'all'], ['活跃空间', rows.filter(r => r.status.state === 'active').length, 'grid', 'active'],
            ['将归档', rows.filter(r => r.status.state === 'pending_archive').length, 'clock', 'pending_archive'], ['已归档', archived.length, 'archive', 'archived'],
          ].map(([name, count, icon, key]) => <button className={`stat ${filter === key ? 'stat-selected' : ''}`} key={key} onClick={() => selectStatus(String(key))}><span className="stat-title">{name}<Icon name={String(icon)} /></span><span className="stat-number">{count}<span>个工作空间</span></span></button>)}</div>
          <div className="toolbar"><div className="segmented">{[['all', '全部'], ['active', '活跃'], ['pending_archive', '将归档'], ['archived', '已归档']].map(([key, name]) => <button className={filter === key ? 'selected' : ''} key={key} onClick={() => selectStatus(key)}>{name}</button>)}</div><div className="toolbar-right"><label className="search"><Icon name="search" size={17} /><input aria-label="搜索工作空间" placeholder="搜索工作空间、仓库或分支…" value={search} onChange={e => setSearch(e.target.value)} /></label><select aria-label="排序方式" value={sort} onChange={e => setSort(e.target.value)}><option value="modified">最近修改</option><option value="name">名称排序</option><option value="issues">异常优先</option></select></div></div>
          <div className="list-meta"><span>{filtered.length} 个{filter === 'archived' ? '已归档工作' : '工作'}空间</span><span>{snapshot.isFetching ? '正在检测状态…' : `更新于 ${snapshot.data ? date(snapshot.data.checked_at) : '—'}`}</span></div>
          {snapshot.isPending ? <Masonry>{[0, 1, 2].map(i => <div className="card skeleton" key={i}><div /><div /><div /><div /></div>)}</Masonry> : filtered.length ? <Masonry>{filtered.map(row => <WorkspaceCard key={row.workspace.workspace_id} row={row} config={config.data} remote={remote[row.workspace.workspace_id] || {}} busy={busy} remoteChecking={remoteChecking.has(row.workspace.workspace_id)}
            onDetail={() => setDetail(row)} onArchive={() => preview('/plans/archive', { workspace_ids: [row.workspace.workspace_id] })} onRestore={() => setRestore(row)}
            onOpen={editor => openWorkspace(row, editor)} onRemote={() => checkRemote(row)} onTrash={() => preview(`/workspaces/${row.workspace.workspace_id}/trash/preview`, {})} />)}</Masonry> : <div className="empty"><span className="empty-icon"><Icon name={filter === 'archived' ? 'archive' : 'folder'} size={32} /></span><h3>{search || (filter !== 'all' && filter !== 'archived') ? '没有匹配的工作空间' : filter === 'archived' ? '还没有已归档的工作空间' : '你的工作空间，从这里开始'}</h3><p>{search || (filter !== 'all' && filter !== 'archived') ? '试试其他搜索词，或调整筛选条件。' : filter === 'archived' ? '归档后，工作空间的版本与上下文会保存在这里。' : '通过 VDW CLI 创建或关联工作空间，操作台会自动发现。'}</p>{filter === 'all' && !search && <code>vdw init my-workspace</code>}<button className="button" onClick={() => setPage('config')}>查看配置 <Icon name="chevron" size={14} /></button></div>}
          <div className="footnote"><Icon name="shield" size={14} /> 状态检测不会修改仓库。归档预警窗口为 7 天，远端状态按需检测。</div>
        </>}
        {page === 'config' && (config.data ? <ConfigPage config={config.data} busy={busy} preview={body => preview('/config/preview', body)} save={prefs => action('preferences', () => api('/preferences', { method: 'PUT', body: JSON.stringify({ version: config.data.preferences_version, preferences: prefs }) }), () => { void client.invalidateQueries({ queryKey: ['config'] }); notify('操作台偏好已保存'); })} /> : <div className="empty">{config.error?.message || '正在读取配置…'}</div>)}
        {page === 'operations' && <Operations rows={operations.data || []} loading={operations.isPending} error={operations.error?.message} job={jobs.data} busy={!!busy} undoErrors={undoErrors} onUndo={previewUndo} />}
      </main>
    </div>
    {detail && <Detail row={rows.find(r => r.workspace.workspace_id === detail.workspace.workspace_id) || detail} remote={remote[detail.workspace.workspace_id] || {}} close={() => setDetail(null)} checkRemote={() => checkRemote(detail)} remoteChecking={remoteChecking.has(detail.workspace.workspace_id)} preview={body => preview('/config/preview', { ...body, version: config.data?.version })} />}
    {restore && <RestoreDialog row={restore} busy={!!busy} close={() => setRestore(null)} preview={body => { setRestore(null); preview('/plans/restore', body); }} />}
    {plan?.kind === 'trash' && <TrashDialog plan={plan} busy={!!busy} close={() => setPlan(null)} confirm={() => action('execute', () => post<Job>(`/plans/${plan.plan_id}/execute`, { confirm: true }), () => { monitor.track(plan.plan_id); setPlan(null); })} />}
    {plan?.kind === 'undo' && <UndoDialog plan={plan} busy={!!busy} close={() => setPlan(null)} confirm={() => action('execute', () => post<Job>(`/plans/${plan.plan_id}/execute`, { confirm: true }), () => { monitor.track(plan.plan_id); setPlan(null); })} />}
    {plan && !['trash', 'undo'].includes(plan.kind) && <div className="overlay" role="presentation"><section className="dialog" role="dialog" aria-modal="true" aria-labelledby="plan-title"><div className="dialog-header"><span className="dialog-icon"><Icon name={plan.kind === 'archive' ? 'archive' : 'check'} size={23} /></span><button className="icon-button" aria-label="关闭预览" onClick={() => setPlan(null)}><Icon name="x" /></button></div><h2 id="plan-title">确认{label(plan.kind)}计划</h2><p className="muted">检查本次操作的影响。确认后会重新校验当前状态。</p><div className="plan-info"><span>计划有效期</span><strong>{date(plan.expires_at)}</strong></div>
      <PlanChanges plan={plan} />
      {archiveBlocked && <div className="banner error compact" role="alert"><Icon name="alert" /><span>本次计划没有可归档的工作空间，请先处理下方原因后重新预览。</span></div>}
      {plan.entries.map(entry => <div className="plan-entry" key={entry.workspace_id}><div><strong>{entry.name}</strong><Badge tone={entry.reason_codes.length ? 'orange' : 'green'}>{entry.excluded ? '已排除' : entry.reason_codes.includes('inspection_failed') ? '预检失败，无法归档' : entry.reason_codes.length ? '将跳过' : '可执行'}</Badge></div><code>{entry.path}</code><p>{entry.message || '已完成预检'}</p>{entry.reason_codes.length > 0 && <p className="warning-text">{entry.reason_codes.map(label).join(' · ')}</p>}{entry.repositories.map(repo => <ArchiveWarnings key={repo.path} warnings={repo.archive_warnings} />)}<span className="small muted">归档 / 恢复位置：{plan.target || entry.archive_root}</span>{plan.kind === 'restore' && entry.repositories.map(repo => <div className="restore-revision" key={repo.repo}><span>{repo.repo.split('/').at(-1)}</span><code>{repo.head}</code></div>)}</div>)}
      {plan.kind === 'archive' && <div className="banner compact"><Icon name="shield" /><span>任何仓库存在暂存、未暂存、冲突或未跟踪变更时，整个工作空间会跳过归档。</span></div>}
      <details className="raw-details"><summary>查看完整计划与配置差异</summary><pre>{JSON.stringify(plan, null, 2)}</pre></details><div className="dialog-actions"><button className="button" onClick={() => setPlan(null)}>取消</button><button className="button primary" disabled={!!busy || archiveBlocked} onClick={() => action('execute', () => post<Job>(`/plans/${plan.plan_id}/execute`, { confirm: true }), () => { monitor.track(plan.plan_id); setPlan(null); setPage('operations'); })}>{busy === 'execute' ? '正在提交…' : archiveBlocked ? '当前无法归档' : '确认执行'}<Icon name="chevron" size={15} /></button></div></section></div>}
    {diagnostic && <div className="overlay"><section className="dialog" role="dialog" aria-modal="true" aria-label="仓库异常说明"><div className="dialog-header"><h2>仓库异常说明</h2><button className="icon-button" aria-label="关闭异常说明" onClick={() => setDiagnostic(null)}><Icon name="x" /></button></div><p className="muted">{diagnostic.name} · 可粘贴到编辑器对话框，请先诊断并保留未提交变更。</p><textarea className="diagnostic-prompt" aria-label="异常诊断内容" readOnly value={diagnostic.prompt} onFocus={event => event.currentTarget.select()} /><div className="dialog-actions"><button className="button" onClick={() => setDiagnostic(null)}>关闭</button><button className="button primary" onClick={async () => { try { await navigator.clipboard.writeText(diagnostic.prompt); notify('异常说明已复制到剪贴板'); } catch { notify('复制失败，请选中内容后手动复制', true); } }}>复制异常说明</button></div></section></div>}
    {notice && <div className={`toast ${notice.error ? 'toast-error' : ''}`} role="status"><Icon name={notice.error ? 'alert' : 'check'} /><span>{notice.text}</span>{notice.diagnostics && <button onClick={() => setDiagnostic(notice.diagnostics!)}>查看异常说明</button>}<button aria-label="关闭通知" className="icon-button" onClick={() => setNotice(null)}><Icon name="x" size={16} /></button></div>}
  </div>;
}
