import { useEffect, useRef, useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import { api } from '../api';
import { terminal } from '../utils';
import type { Job } from '../types';

export function useOperationJobs(onSettled: (jobs: Job[]) => Promise<void>) {
  const [pending, setPending] = useState<string[]>(() => {
    try {
      const saved = JSON.parse(localStorage.getItem('vdw-pending-plans') || 'null');
      return Array.isArray(saved) ? saved.filter(id => typeof id === 'string') : [localStorage.getItem('vdw-last-plan')].filter((id): id is string => !!id);
    } catch { return []; }
  });
  const [latest, setLatest] = useState<Job>();
  const reported = useRef(new Set<string>());
  const queries = useQueries({ queries: pending.map(id => ({
    queryKey: ['job', id], queryFn: ({ signal }: { signal: AbortSignal }) => api<Job>(`/jobs/${encodeURIComponent(id)}`, { signal }),
    refetchInterval: (query: { state: { data?: Job } }) => query.state.data && terminal(query.state.data.state) ? false as const : 1000,
    refetchIntervalInBackground: true, retry: false,
  })) });
  useEffect(() => { localStorage.setItem('vdw-pending-plans', JSON.stringify(pending)); }, [pending]);
  useEffect(() => {
    const finished = queries.flatMap((query, index) => query.data && terminal(query.data.state) && !reported.current.has(pending[index])
      ? [{ ...query.data, plan_id: pending[index] }] : []);
    if (!finished.length) return;
    finished.forEach(job => reported.current.add(job.plan_id));
    setLatest(finished[finished.length - 1]);
    void onSettled(finished).finally(() => setPending(ids => ids.filter(id => !finished.some(job => job.plan_id === id))));
  }, [queries, pending, onSettled]);
  return {
    current: queries.find(query => query.data && !terminal(query.data.state))?.data || latest,
    track: (id: string) => {
      localStorage.setItem('vdw-last-plan', id);
      setPending(ids => ids.includes(id) ? ids : [...ids, id]);
    },
  };
}
