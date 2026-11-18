// Waiting for the daemon to restart in place (R7a). The daemon serves its health
// route under /api/v1, like every other route.

export const HEALTHZ = '/api/v1/healthz';

const LIMIT_MS = 60_000;
const POLL_MS = 1000;

type Probe = { up: boolean; startId?: string };

async function probe(fetchFn: typeof fetch): Promise<Probe> {
  try {
    const r = await fetchFn(HEALTHZ, { cache: 'no-store' });
    if (!r.ok) return { up: false };
    const body = (await r.json()) as { startId?: unknown };
    return { up: true, startId: typeof body.startId === 'string' ? body.startId : undefined };
  } catch {
    return { up: false };
  }
}

/** The daemon's start id (`startId` of `HEALTHZ`), read before a restart. `undefined` when the
 * daemon does not answer or predates the field. */
export async function startId(fetchFn: typeof fetch = fetch): Promise<string | undefined> {
  return (await probe(fetchFn)).startId;
}

/** Polls `HEALTHZ` each second (at most 60 s) and returns once the daemon has restarted: its
 * `startId` differs from `before`, or it went down and answers again. Rejects saying whether it
 * never went down or never came back. */
export async function waitForRestart(
  before: string | undefined,
  fetchFn: typeof fetch = fetch,
  sleep: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
  now: () => number = Date.now,
): Promise<void> {
  const end = now() + LIMIT_MS;
  let wentDown = false;
  while (now() < end) {
    await sleep(POLL_MS);
    const p = await probe(fetchFn);
    if (!p.up) {
      wentDown = true;
      continue;
    }
    // A new start id is a restart even when no poll found the daemon down (a fast re-exec).
    if (before !== undefined && p.startId !== undefined && p.startId !== before) return;
    if (wentDown) return;
  }
  throw new Error(wentDown
    ? 'The daemon stopped but did not come back within 60 seconds.'
    : 'The daemon did not restart within 60 seconds.');
}
