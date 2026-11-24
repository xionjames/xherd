// HTTP calls to the daemon. Local browsers are trusted by address (design §7.2),
// so no token is sent.

import type { InstalledPlugin, PluginPreview, PluginsList, PluginUpdatePreview, UploadResult } from './types';

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
/** One multipart request for all the files (R1); `onProgress` gets 0..1 while the body is sent. Refusals are `ApiError`s with the daemon's code. */
export function uploadFiles(jobId: string, files: File[], onProgress?: (fraction: number) => void): Promise<string[]> {
  return new Promise((resolve, reject) => {
    const form = new FormData();
    for (const f of files) form.append('files', f, f.name);
    const xhr = new XMLHttpRequest();
    xhr.open('POST', `/api/v1/jobs/${encodeURIComponent(jobId)}/files`);
    xhr.responseType = 'text';
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress?.(e.loaded / e.total);
    };
    xhr.onerror = () => reject(new ApiError(0, 'network', 'The connection to xherd failed'));
    xhr.onabort = () => reject(new ApiError(0, 'aborted', 'The upload was cancelled'));
    xhr.onload = () => {
      let data: { paths?: string[]; code?: string; error?: string; message?: string } = {};
      try {
        data = xhr.responseText ? JSON.parse(xhr.responseText) : {};
      } catch {
        // not JSON (a proxy's page): fall back to the status text
      }
      if (xhr.status >= 200 && xhr.status < 300) resolve((data as UploadResult).paths ?? []);
      else reject(new ApiError(xhr.status, data.code ?? 'http', data.error ?? data.message ?? (xhr.statusText || 'Upload failed')));
    };
    xhr.send(form);
  });
}

/**
 * One image of a job (`/jobs/{id}/image?…`, R1). SVG comes back as a `data:` URL, so it can only be drawn as an
 * `<img>` and never runs; raster images as a `blob:` URL (the caller revokes it). `blob` is the git blob id
 * (`X-Xherd-Blob`), empty when the header is missing. Refusals are `ApiError`s with the daemon's code.
 */
export async function getImage(path: string): Promise<{ url: string; blob: string }> {
  const res = await fetch(`/api/v1${path}`);
  if (!res.ok) {
    let data: { code?: string; error?: string; message?: string } = {};
    try {
      data = JSON.parse(await res.text());
    } catch {
      // not JSON (a proxy's page): fall back to the status text
    }
    throw new ApiError(res.status, data.code ?? 'http', data.error ?? data.message ?? res.statusText);
  }
  const blob = res.headers.get('X-Xherd-Blob') ?? '';
  const body = await res.blob();
  if (body.type === 'image/svg+xml' || res.headers.get('content-type')?.startsWith('image/svg+xml')) {
    const url = await new Promise<string>((resolve, reject) => {
      const r = new FileReader();
      r.onload = () => resolve(String(r.result));
      r.onerror = () => reject(new ApiError(0, 'read', 'The image could not be read'));
      r.readAsDataURL(body.type === 'image/svg+xml' ? body : new Blob([body], { type: 'image/svg+xml' }));
    });
    return { url, blob };
  }
  return { url: URL.createObjectURL(body), blob };
}

/** The URL that downloads `path` (absolute, or relative to the job's workdir) as a file. */
export const downloadUrl = (jobId: string, path: string) =>
  `/api/v1/jobs/${encodeURIComponent(jobId)}/files?path=${encodeURIComponent(path)}`;

/** Re-executes the daemon in place; `waitForRestart` (restart.ts) polls `/api/v1/healthz` until it answers again. */
export const restartDaemon = () => post<{ restarting: boolean }>('/admin/restart');
