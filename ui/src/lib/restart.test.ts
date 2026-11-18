import { describe, expect, it } from 'vitest';
import { HEALTHZ, startId, waitForRestart } from './restart';

// A virtual clock: `sleep` advances it instead of waiting, so no test waits real seconds.
function clock() {
  let t = 0;
  return { now: () => t, sleep: async (ms: number) => { t += ms; } };
}

// A fake daemon: `down(i)` says whether poll number i (from 0) finds it down. `startIdAt(i)`
// gives the start id it answers with on poll i (default: one id for the whole run). When up, it
// answers 200 on /api/v1/healthz with that id and 404 on any other path, like the real router.
function daemon(down: (i: number) => boolean, startIdAt: (i: number) => string = () => 'A') {
  const urls: string[] = [];
  const fetchFn = async (input: RequestInfo | URL): Promise<Response> => {
    const url = String(input);
    const i = urls.length;
    urls.push(url);
    if (down(i)) throw new TypeError('Failed to fetch');
    const body = JSON.stringify({ ok: true, startId: startIdAt(i) });
    return new Response(body, { status: url === '/api/v1/healthz' ? 200 : 404 });
  };
  return { urls, fetchFn: fetchFn as typeof fetch };
}

describe('waitForRestart', () => {
  it('polls /api/v1/healthz and stops as soon as the daemon is back', async () => {
    const { urls, fetchFn } = daemon((i) => i < 2);
    const { now, sleep } = clock();
    await expect(waitForRestart('A', fetchFn, sleep, now)).resolves.toBeUndefined();
    expect(HEALTHZ).toBe('/api/v1/healthz');
    expect(urls.length).toBeGreaterThan(0);
    expect(urls.every((u) => u === '/api/v1/healthz')).toBe(true);
    expect(urls.length).toBeLessThanOrEqual(4);
    expect(now()).toBeLessThan(60_000);
  });

  it('resolves at the first poll when the start id changed without a visible outage', async () => {
    // The daemon re-executed between two polls: it never fails a poll, but answers with a new id.
    const { urls, fetchFn } = daemon(() => false, (i) => (i < 1 ? 'A' : 'B'));
    const { now, sleep } = clock();
    await expect(waitForRestart('A', fetchFn, sleep, now)).resolves.toBeUndefined();
    expect(urls.length).toBe(2);
    expect(now()).toBe(2_000);
  });

  it('rejects when the daemon never went down and kept its start id', async () => {
    const { fetchFn } = daemon(() => false);
    const { now, sleep } = clock();
    await expect(waitForRestart('A', fetchFn, sleep, now)).rejects.toThrow('did not restart');
  });

  it('rejects when the daemon went down and never came back', async () => {
    const { fetchFn } = daemon(() => true);
    const { now, sleep } = clock();
    await expect(waitForRestart('A', fetchFn, sleep, now)).rejects.toThrow('did not come back');
  });

  it('falls back to going down and up when the daemon has no start id', async () => {
    // An older daemon answers /healthz without startId: `before` is undefined, so only the outage counts.
    const { fetchFn } = daemon((i) => i < 2, () => undefined as unknown as string);
    const { now, sleep } = clock();
    await expect(waitForRestart(undefined, fetchFn, sleep, now)).resolves.toBeUndefined();
  });
});

describe('startId', () => {
  it('reads the start id from /api/v1/healthz', async () => {
    const { urls, fetchFn } = daemon(() => false, () => 'X1');
    await expect(startId(fetchFn)).resolves.toBe('X1');
    expect(urls).toEqual(['/api/v1/healthz']);
  });

  it('is undefined while the daemon does not answer', async () => {
    const { fetchFn } = daemon(() => true);
    await expect(startId(fetchFn)).resolves.toBeUndefined();
  });
});
