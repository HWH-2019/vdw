const params = new URLSearchParams(window.location.hash.slice(1));
const incoming = params.get('session');
if (incoming) {
  sessionStorage.setItem('vdw-session', incoming);
  history.replaceState(null, '', window.location.pathname);
}

export async function api<T>(path: string, options?: RequestInit): Promise<T> {
  const response = await fetch(`/api${path}`, {
    ...options,
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${sessionStorage.getItem('vdw-session') || ''}`, ...options?.headers },
  });
  const data = await response.json().catch(() => ({ error: `服务返回 ${response.status}` }));
  if (!response.ok) throw new Error(data.error || `请求失败 (${response.status})`);
  return data as T;
}
export const post = <T,>(path: string, body: unknown) => api<T>(path, { method: 'POST', body: JSON.stringify(body) });
