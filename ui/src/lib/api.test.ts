// @vitest-environment jsdom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, getImage } from './api';

const reply = (body: BodyInit, init: ResponseInit) => vi.fn().mockResolvedValue(new Response(body, init));

beforeEach(() => {
  URL.createObjectURL = vi.fn(() => 'blob:fake');
});
afterEach(() => vi.unstubAllGlobals());

describe('getImage', () => {
  it('turns a raster image into a blob URL and returns the blob id', async () => {
    const f = reply(new Uint8Array([1, 2, 3]), { headers: { 'content-type': 'image/png', 'X-Xherd-Blob': 'abc123' } });
    vi.stubGlobal('fetch', f);
    const r = await getImage('/jobs/j1/image?path=.%2Fa.png');
    expect(f).toHaveBeenCalledWith('/api/v1/jobs/j1/image?path=.%2Fa.png');
    expect(r).toEqual({ url: 'blob:fake', blob: 'abc123' });
  });

  it('turns an SVG into a data URL, never a blob URL', async () => {
    vi.stubGlobal('fetch', reply('<svg xmlns="http://www.w3.org/2000/svg"/>', { headers: { 'content-type': 'image/svg+xml', 'X-Xherd-Blob': 'def456' } }));
    const r = await getImage('/jobs/j1/image?path=.%2Fa.svg');
    expect(r.url.startsWith('data:image/svg+xml')).toBe(true);
    expect(r.blob).toBe('def456');
    expect(URL.createObjectURL).not.toHaveBeenCalled();
  });

  it('throws an ApiError with the daemon code on a refusal', async () => {
    vi.stubGlobal('fetch', reply(JSON.stringify({ code: 'not_an_image', error: 'not an image' }), { status: 415 }));
    const err = await getImage('/jobs/j1/image?path=.%2Fa.txt').catch((e) => e);
    expect(err).toBeInstanceOf(ApiError);
    expect(err).toMatchObject({ status: 415, code: 'not_an_image', message: 'not an image' });
  });

  it('falls back to the status when the error is not JSON', async () => {
    vi.stubGlobal('fetch', reply('<html>bad gateway</html>', { status: 502, statusText: 'Bad Gateway' }));
    const err = await getImage('/jobs/j1/image?path=.%2Fa.png').catch((e) => e);
    expect(err).toMatchObject({ status: 502, code: 'http', message: 'Bad Gateway' });
  });
});
