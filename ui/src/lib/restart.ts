// Waiting for the daemon to restart in place (R7a). The daemon serves its health
// route under /api/v1, like every other route.

export const HEALTHZ = '/api/v1/healthz';

const LIMIT_MS = 60_000;
const POLL_MS = 1000;

/** Polls `HEALTHZ` each second until the daemon has gone down and answers again
 * (at most 60 s). Rejects saying whether it never went down or never came back. */
export async function waitForRestart(
  fetchFn: typeof fetch = fetch,
  sleep: (ms: number) => Promise<void> = (ms) => new Promise((r) => setTimeout(r, ms)),
  now: () => number = Date.now,
): Promise<void> {
  const alive = () => fetchFn(HEALTHZ, { cache: 'no-store' }).then((r) => r.ok, () => false);
  const end = now() + LIMIT_MS;
  let wentDown = false;
  while (now() < end) {
    await sleep(POLL_MS);
    if (!(await alive())) wentDown = true;
    else if (wentDown) return;
  }
  throw new Error(wentDown
    ? 'The daemon stopped but did not come back within 60 seconds.'
    : 'The daemon did not restart within 60 seconds.');
}
