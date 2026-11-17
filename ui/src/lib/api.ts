// HTTP calls to the daemon. Local browsers are trusted by address (design §7.2),
// so no token is sent.

import type { InstalledPlugin, PluginPreview, PluginsList, PluginUpdatePreview } from './types';

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string) {
    super(message);
  }
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const res = await fetch(`/api/v1${path}`, {
    method,
    headers: body === undefined ? {} : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  const data = text ? JSON.parse(text) : {};
  if (!res.ok) {
    throw new ApiError(res.status, data.code ?? 'http', data.error ?? data.message ?? res.statusText);
  }
  return data as T;
}

export const get = <T>(path: string) => call<T>('GET', path);
export const post = <T>(path: string, body?: unknown) => call<T>('POST', path, body ?? {});
export const put = <T>(path: string, body: unknown) => call<T>('PUT', path, body);
export const del = <T>(path: string) => call<T>('DELETE', path);

// Installing plugins from a git URL (the CLI's `xherd plugin`). Install and
// update send back the digest the user saw in the preview.
export const pluginsList = () => get<PluginsList>('/plugins');
export const pluginPreview = (url: string, ref?: string, path?: string) =>
  post<PluginPreview>('/plugins/preview', { url, ref: ref || undefined, path: path || undefined });
export const pluginInstall = (url: string, digest: string, ref?: string, path?: string) =>
  post<InstalledPlugin>('/plugins/install', { url, ref: ref || undefined, path: path || undefined, digest });
export const pluginUpdatePreview = (name: string, ref?: string) =>
  post<PluginUpdatePreview>(`/plugins/${encodeURIComponent(name)}/update`, { ref: ref || undefined });
export const pluginUpdate = (name: string, digest: string, ref?: string) =>
  post<InstalledPlugin>(`/plugins/${encodeURIComponent(name)}/update`, { ref: ref || undefined, digest });
export const pluginRemove = (name: string, forget = false) =>
  del<{ name: string; forgot: boolean }>(`/plugins/${encodeURIComponent(name)}${forget ? '?forget=true' : ''}`);
/** Re-executes the daemon in place; poll `/healthz` until it answers again. */
export const restartDaemon = () => post<{ restarting: boolean }>('/admin/restart');
