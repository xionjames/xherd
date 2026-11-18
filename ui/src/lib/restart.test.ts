import { describe, expect, it } from 'vitest';
import { HEALTHZ, waitForRestart } from './restart';

// A virtual clock: `sleep` advances it instead of waiting, so no test waits real seconds.
function clock() {
  let t = 0;
  return { now: () => t, sleep: async (ms: number) => { t += ms; } };
}

// A fake daemon: `down(i)` says whether poll number i (from 0) finds it down. When up, it
// answers 200 on /api/v1/healthz and 404 on any other path, like the real router.
function daemon(down: (i: number) => boolean) {
  const urls: string[] = [];
  const fetchFn = async (input: RequestInfo | URL): Promise<Response> => {
    const url = String(input);
    const i = urls.length;
    urls.push(url);
    if (down(i)) throw new TypeError('Failed to fetch');
    return new Response('{"ok":true}', { status: url === '/api/v1/healthz' ? 200 : 404 });
  };
  return { urls, fetchFn: fetchFn as typeof fetch };
}

describe('waitForRestart', () => {
  it('polls /api/v1/healthz and stops as soon as the daemon is back', async () => {
    const { urls, fetchFn } = daemon((i) => i < 2);
    const { now, sleep } = clock();
    await expect(waitForRestart(fetchFn, sleep, now)).resolves.toBeUndefined();
    expect(HEALTHZ).toBe('/api/v1/healthz');
    expect(urls.length).toBeGreaterThan(0);
    expect(urls.every((u) => u === '/api/v1/healthz')).toBe(true);
    expect(urls.length).toBeLessThanOrEqual(4);
    expect(now()).toBeLessThan(60_000);
  });

  it('rejects when the daemon never went down', async () => {
    const { fetchFn } = daemon(() => false);
    const { now, sleep } = clock();
    await expect(waitForRestart(fetchFn, sleep, now)).rejects.toThrow('did not restart');
  });

  it('rejects when the daemon went down and never came back', async () => {
    const { fetchFn } = daemon(() => true);
    const { now, sleep } = clock();
    await expect(waitForRestart(fetchFn, sleep, now)).rejects.toThrow('did not come back');
  });
});
