import { useEffect, useRef } from 'react';
import { Icon } from '../icons';
import { label } from '../utils';
import { ArchiveWarnings } from './ui';
import { PlanChanges } from './PlanChanges';
import type { Plan } from '../types';

export function UndoDialog({ plan, busy, close, confirm }: {
  plan: Plan; busy: boolean; close: () => void; confirm: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const submit = useRef<HTMLButtonElement>(null);
  const value = plan.value as { operation_id: string; original_kind: string; changes: unknown[] };
  useEffect(() => { cancel.current?.focus(); }, []);
  return <div className="overlay"><section className="dialog" role="dialog" aria-modal="true" aria-labelledby="undo-title"
    onKeyDown={event => {
      if (event.key === 'Escape' && !busy) close();
      if (event.key === 'Tab') {
        event.preventDefault();
        (document.activeElement === cancel.current ? submit : cancel).current?.focus();
      }
    }}>
    <div className="dialog-header"><span className="dialog-icon"><Icon name="restore" size={24} /></span><span className="small muted">撤销{label(value.original_kind)}</span></div>
    <h2 id="undo-title">确认撤销此操作？</h2>
    <p className="muted">将执行下面的反向操作，并保留原记录和撤销结果。</p>
    <code className="small">{value.operation_id}</code>
    {plan.entries.map(entry => <div className="plan-entry" key={entry.workspace_id}><strong>{entry.name}</strong><code>{entry.path}</code><p>{entry.message}</p>{entry.repositories.map(repo => <ArchiveWarnings key={repo.path} warnings={repo.archive_warnings} />)}</div>)}
    {value.changes.map((change, index) => <PlanChanges key={index} plan={{ ...plan, value: change }} />)}
    <div className="banner compact"><Icon name="shield" /><span>确认时会再次校验。文件或配置已有后续变化、原路径被占用时，会停止撤销并提示原因。</span></div>
    <div className="dialog-actions"><button ref={cancel} className="button" disabled={busy} onClick={close}>取消</button><button ref={submit} className="button primary" disabled={busy} onClick={confirm}><Icon name="restore" size={15} />{busy ? '正在提交…' : '确认撤销'}</button></div>
  </section></div>;
}
