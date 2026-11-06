import { useEffect, useRef } from 'react';
import { Icon } from '../icons';
import type { Plan } from '../types';

export function TrashDialog({ plan, busy, close, confirm }: {
  plan: Plan; busy: boolean; close: () => void; confirm: () => void;
}) {
  const cancel = useRef<HTMLButtonElement>(null);
  const submit = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus(); }, []);
  return <div className="overlay"><section className="dialog trash-dialog" role="dialog" aria-modal="true" aria-labelledby="trash-title"
    onKeyDown={event => {
      if (event.key === 'Escape' && !busy) close();
      if (event.key === 'Tab') {
        event.preventDefault();
        (document.activeElement === cancel.current ? submit : cancel).current?.focus();
      }
    }}>
    <div className="dialog-header"><span className="dialog-icon danger-icon"><Icon name="trash" size={24} /></span><span className="small muted">删除工作空间</span></div>
    <h2 id="trash-title">确认移到回收站？</h2>
    <p className="muted">整个工作空间目录将从当前位置移走，包含所有仓库、未提交变更、未跟踪文件和本地资料。</p>
    {plan.entries.map(entry => <div className="trash-target" key={entry.workspace_id}><strong>{entry.name}</strong><code>{entry.path}</code></div>)}
    <div className="banner compact"><Icon name="restore" /><span>可从系统回收站找回目录。共享 Store、远端仓库和历史归档会保留。</span></div>
    <div className="dialog-actions"><button ref={cancel} className="button" disabled={busy} onClick={close}>取消</button><button ref={submit} className="button danger" disabled={busy} onClick={confirm}><Icon name="trash" size={15} />{busy ? '正在提交…' : '移到回收站'}</button></div>
  </section></div>;
}
