import type React from 'react';
import type { ArchiveNotice } from '../types';
import { Icon } from '../icons';

export function ArchiveWarnings({ warnings }: { warnings?: ArchiveNotice[] }) {
  return <>{warnings?.map(warning => <div className="repo-warning archive-warning" role="note" aria-label="归档提示" key={`${warning.code}:${warning.path}`}><Icon name="alert" size={12} /><span>{warning.message}</span></div>)}</>;
}

export function Badge({ children, tone = 'gray', title }: { children: React.ReactNode; tone?: string; title?: string }) { return <span className={`badge ${tone}`} title={title}>{children}</span>; }

export function Field({ label: name, hint, children }: { label: string; hint?: string; children: React.ReactNode }) { return <label className="field"><span>{name}</span>{children}{hint && <small>{hint}</small>}</label>; }
