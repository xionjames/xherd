// One global store: loaded from `GET /state`, kept live by `GET /ws`, fully
// reloaded on every (re)connect (design §13.1).
import { get } from './api';
import { setLinkRules } from './links';
import type { Job, AppState } from './types';
import { addNotice, settle } from './notices.svelte';

export const app = $state<{
  state: AppState | null;
  error: string | null;
  connected: boolean;
}>({ state: null, error: null, connected: false });

export async function reload() {
  try {
    app.state = await get<AppState>('/state');
    setLinkRules(app.state.trackerLinks ?? []);
    for (const a of app.state.jobs) settle(a);
    app.error = null;
  } catch (e) {
    app.error = (e as Error).message;
  }
}

export function upsertJob(a: Job) {
  const s = app.state;
  if (!s) return;
  const i = s.jobs.findIndex((x) => x.id === a.id);
  if (i >= 0) s.jobs[i] = a;
  else s.jobs.push(a);
  settle(a);
}

export function jobByName(idOrName: string): Job | undefined {
  return app.state?.jobs.find((a) => a.id === idOrName || a.name === idOrName);
}

type WsMsg = { type: 'job'; job: Job } | { type: 'notify'; jobId: string; kind: string; text: string };

let retry = 500;
let opened = false;

/** Load the state now, then keep it live over the websocket. */
export function start() {
  reload();
  connect();
}

function connect() {
  const proto = location.protocol === 'https:' ? 'wss' : 'ws';
  const ws = new WebSocket(`${proto}://${location.host}/api/v1/ws`);
  ws.onopen = async () => {
    retry = 500;
    app.connected = true;
    // The first load is done by start(); every reconnect reloads in full.
    if (opened) await reload();
    opened = true;
  };
  ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data) as WsMsg;
    if (m.type === 'job') upsertJob(m.job);
    else if (m.type === 'notify') addNotice(m.jobId, m.kind, m.text);
  };
  ws.onclose = () => {
    app.connected = false;
    setTimeout(connect, retry);
    retry = Math.min(retry * 2, 10_000);
  };
}
