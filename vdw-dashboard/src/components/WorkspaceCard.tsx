import { useState } from 'react';
import { Icon } from '../icons';
import { date, label, modifiedDate } from '../utils';
import { ArchiveWarnings, Badge } from './ui';
import { remoteTarget, RemoteStatus } from './RemoteAddress';
import type { Configuration, Remote, Repository, Workspace } from '../types';

export function WorkspaceStatus({ row }: { row: Workspace }) {
  return <Badge tone={row.status.state === 'archived' ? 'gray' : row.status.state === 'pending_archive' ? 'orange' : 'green'}
    title={`${row.status.archive_at ? `预计归档：${date(row.status.archive_at)}\n` : ''}${row.status.message}`}>
    {label(row.status.state)}
  </Badge>;
}

export function RepoRow({ repo, remote, showRemoteStatus = true }: { repo: Repository; remote?: Remote; showRemoteStatus?: boolean }) {
  return <div className="repo-row"><span className="repo-icon"><Icon name="repo" size={16} /></span><div className="repo-content"><div className="repo-title"><strong title={repo.remote}>{repo.name}</strong><span className="mode">{repo.editable ? '可编辑' : '只读'}</span>{repo.web_url && <a className="remote-link" href={remoteTarget(repo, remote) || repo.web_url} target="_blank" rel="noreferrer" title="打开远端仓库或开发分支" aria-label={`打开 ${repo.name} 远端仓库或开发分支`}><Icon name="arrow" size={15} /></a>}</div><div className="repo-meta"><Icon name="branch" size={12} /><span className="mono" title={`开发分支：${repo.development_branch || '—'}\n实际：${repo.branch || repo.head || '—'}`}>{repo.branch || (repo.head ? repo.head.slice(0, 8) : repo.declared_revision || '未知版本')}</span>{repo.git?.branch_drift && <span className="warning-text">分支偏离</span>}{repo.git_error ? <span className="warning-text" title={repo.git_error}>检测失败</span> : repo.git?.change_count ? <span className="warning-text">{repo.git.change_count} 项变更</span> : repo.topology === 'ready' && <span>无未提交变更</span>}{repo.git?.ahead != null && <span title="基于本地远端跟踪引用">↑{repo.git.ahead} ↓{repo.git.behind}</span>}</div>{!['ready', 'archived'].includes(repo.topology || '') && <div className="repo-warning" title={repo.topology_message}>{label(repo.topology || '检测失败')}</div>}{repo.archive_blockers?.map(issue => <div className="repo-warning archive-warning" key={`${issue.code}:${issue.path}`}><Icon name="alert" size={12} /><span>{issue.message}</span></div>)}<ArchiveWarnings warnings={repo.archive_warnings} />{showRemoteStatus && <RemoteStatus remote={remote} />}</div></div>;
}

export function WorkspaceCard({ row, config, remote, busy, remoteChecking, onDetail, onArchive, onRestore, onOpen, onRemote, onTrash }: {
  row: Workspace; config?: Configuration; remote: Record<string, Remote>; busy: string; remoteChecking: boolean; onDetail: () => void; onArchive: () => void; onRestore: () => void; onOpen: (editor?: string) => void; onRemote: () => void; onTrash: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [editorMenu, setEditorMenu] = useState(false);
  const ws = row.workspace;
  const archived = ws.lifecycle_state === 'archived';
  const warningCount = row.repositories.reduce((count, repo) => count + (repo.archive_warnings?.length || 0), 0);
  return <article className={`card ${row.issue_count ? 'card-issue' : ''}`}><div className="card-heading"><span className={`workspace-icon ${archived ? 'archived-icon' : ''}`}><Icon name={archived ? 'archive' : 'folder'} size={22} /></span><div className="card-identity"><button className="workspace-name" onClick={onDetail} title={ws.name}>{ws.name}</button><span className="workspace-path mono" title={ws.path}>{ws.path.replace(/^\/Users\/[^/]+\//, '~/')}</span></div><WorkspaceStatus row={row} /></div>
    <div className="card-tags"><span>{row.repositories.length} 个仓库</span>{row.protected && <span className="protected"><Icon name="shield" size={12} /> 已保护</span>}{row.active_lease && <span><span className="green-dot" /> IDE 使用中</span>}{warningCount > 0 && <span className="warning-text"><Icon name="alert" size={12} /> {warningCount} 项归档提示</span>}{row.issue_count > 0 && <span className="warning-text"><Icon name="alert" size={12} /> {row.issue_count} 项需关注</span>}</div>
    <div className="repo-list">{row.repositories.slice(0, expanded ? undefined : 3).map(repo => <RepoRow key={repo.id} repo={repo} remote={remote[repo.id]} />)}{!row.repositories.length && <p className="muted small">{row.errors[0] || '尚未关联仓库'}</p>}</div>
    {row.repositories.length > 3 && <button className="expand-button" onClick={() => setExpanded(!expanded)}>{expanded ? '收起仓库' : `展开其余 ${row.repositories.length - 3} 个仓库`}<Icon name="down" size={13} /></button>}
    {row.repositories.some(repo => repo.archive_blockers?.length) && <div className="card-error"><Icon name="alert" size={13} /><span>归档受阻，请在详情中查看具体原因</span></div>}
    {row.errors.length > 0 && <div className="card-error" title={row.errors.join('\n')}><Icon name="alert" size={13} /><span>{row.errors[0]}</span></div>}
    <div className="card-activity"><Icon name="clock" size={13} /><span title={row.modified_error || (row.last_modified_at ? `${date(row.last_modified_at)}\n取源码及工作空间文件的修改时间，忽略运行缓存与构建产物` : '旧归档未记录源码修改时间，或尚无可读取的文件')}>最近修改日期：{modifiedDate(row.last_modified_at)}</span><button className="text-button" disabled={remoteChecking} onClick={onRemote}>{remoteChecking ? '检查中…' : '检查远端'}</button></div>
    <div className="card-actions">{archived ? <button className="button primary-soft" disabled={!!busy || !ws.latest_archive_id} onClick={onRestore}><Icon name="restore" size={15} /> 恢复工作空间</button> : <div className="split-button"><button className="button primary-soft" disabled={!!busy || ws.lifecycle_state !== 'active'} onClick={() => onOpen()}><Icon name="monitor" size={15} /> {busy === `open:${ws.workspace_id}` ? '正在检查异常…' : '打开工作空间'}</button><button className="button primary-soft arrow-button" aria-label="选择编辑器" onClick={() => setEditorMenu(!editorMenu)}><Icon name="down" size={13} /></button>{editorMenu && <div className="editor-menu">{config?.editors.filter(e => e.available).map(editor => <button key={editor.id} onClick={() => { setEditorMenu(false); onOpen(editor.id); }}>{editor.name}</button>)}</div>}</div>}<button className="button quiet" onClick={onDetail}>详情</button>{!archived && <button className="icon-button archive-button" title={row.protected ? '工作空间受保护，预览会展示阻止原因' : '预览归档'} aria-label={`归档 ${ws.name}`} disabled={!!busy || ws.lifecycle_state !== 'active'} onClick={onArchive}><Icon name="archive" size={17} /></button>}<button className={`button quiet delete-button ${archived ? "push-right" : ""}`} title={row.protected || row.active_lease ? "请先解除保护或结束编辑器租约" : "删除整个工作空间，移到系统回收站"} aria-label={`删除 ${ws.name}`} disabled={!!busy || !!ws.current_operation_id || row.protected || row.active_lease || !["active", "archived"].includes(ws.lifecycle_state)} onClick={onTrash}><Icon name="trash" size={16} />删除</button></div>
  </article>;
}
