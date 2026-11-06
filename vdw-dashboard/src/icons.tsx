const paths: Record<string, string> = {
  grid: 'M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z',
  archive: 'M3 4h18v4H3zM5 8v12h14V8M10 12h4',
  trash: 'M3 6h18M9 6V3h6v3M5 6l1 15h12l1-15M10 10v7M14 10v7',
  settings: 'm9 3-1 3-3 1-2 3 2 2v3l3 2 1 3h5l1-3 3-2v-3l2-2-2-3-3-1-1-3zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
  clock: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18M12 7v5l3 2',
  search: 'M11 3a8 8 0 1 0 0 16 8 8 0 0 0 0-16m6 14 4 4',
  refresh: 'M20 7a9 9 0 0 0-15-1L3 9m0-5v5h5M4 17a9 9 0 0 0 15 1l2-3m0 5v-5h-5',
  folder: 'M3 7V5h6l2 2h10v13H3z',
  arrow: 'M7 17 17 7M7 7h10v10',
  branch: 'M6 3v12m0-9h7a5 5 0 0 1 5 5v4M8 18a2 2 0 1 1-4 0 2 2 0 0 1 4 0m12 0a2 2 0 1 1-4 0 2 2 0 0 1 4 0',
  repo: 'M5 3h15v18H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2m0 14h15M7 7h8',
  chevron: 'm9 5 7 7-7 7',
  down: 'm6 9 6 6 6-6',
  check: 'm5 12 4 4L19 6',
  x: 'm6 6 12 12M18 6 6 18',
  alert: 'M12 3 2 21h20zM12 9v5m0 3v1',
  shield: 'm12 3 9 4v5c0 5-9 9-9 9s-9-4-9-9V7zM8 12l3 3 5-6',
  monitor: 'M3 4h18v13H3zM8 21h8m-4-4v4',
  copy: 'M8 8h13v13H8zM16 8V3H3v13h5',
  restore: 'M4 10a8 8 0 1 1 0 6M4 4v6h6M12 8v5l3 2',
};
export function Icon({ name, size = 18, className = '' }: { name: string; size?: number; className?: string }) {
  return <svg className={className} width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d={paths[name] || paths.folder} /></svg>;
}
