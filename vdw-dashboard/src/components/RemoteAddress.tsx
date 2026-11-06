import { date, label } from '../utils';
import type { Remote, Repository } from '../types';

export const remoteTarget = (repo: Repository, remote?: Remote) =>
  remote?.branch_state === 'missing' ? repo.web_url : repo.branch_web_url || repo.web_url;

export function RemoteStatus({ remote }: { remote?: Remote }) {
  if (!remote || remote.state === 'local') return null;
  return <div className="remote-state" title={date(remote.checked_at)}>
    {remote.repository_state === 'available' ? <>
      <span className="success-text">远端仓库存在</span>
      {remote.branch_state === 'present' && <span className="success-text">远端分支存在</span>}
      {remote.branch_state === 'missing' && <span className="danger-text">远端分支已删除</span>}
    </> : <span className="warning-text">{label(remote.state)} · 分支状态未确认</span>}
  </div>;
}

export function RemoteAddress({ repo, remote }: { repo: Repository; remote?: Remote }) {
  const target = remoteTarget(repo, remote);
  const branch = remote?.branch_state !== 'missing' ? repo.development_branch : null;
  return <>
    {target ? <a className="repository-address" href={target} target="_blank" rel="noreferrer">
      {repo.web_url}{branch && <span> / {branch}</span>}
    </a> : <span>{repo.remote}</span>}
    <RemoteStatus remote={remote} />
  </>;
}
