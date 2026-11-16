// Notifications (spec §16.3): `notify` messages from /ws. One per agent and
// kind: a new one replaces the previous. Browser notifications are opt-in
// (permission is asked on the click, never on load).
import { go } from './router.svelte';
import { jobByName } from './store.svelte';
import type { Job } from './types';
import { hasAgents } from './agents';

export interface Notice { key: string; jobId: string; kind: string; text: string; at: number; read: boolean }

const KEY = 'xherd.notify';
const saved = (): boolean => { try { return localStorage.getItem(KEY) === '1'; } catch { return false; } };

export const notices = $state<{ list: Notice[]; browser: boolean; why: Block | null; err: string }>({
  list: [],
  browser: saved() && typeof Notification !== 'undefined' && Notification.permission === 'granted',
  why: null,
  // The last error the browser gave when showing one, shown in the bell
  // (a notification that fails is otherwise silent).
  err: '',
});

export const unread = () => notices.list.filter((n) => !n.read).length;
const supported = () => typeof Notification !== 'undefined';

/** Why browser notifications cannot be turned on (R5). */
export type Block = 'insecure' | 'unsupported' | 'denied';
export interface BrowserEnv { notification: boolean; secure: boolean; permission?: NotificationPermission }

/** Pure. The insecure check comes first: some browsers hide `Notification` on http. */
export function browserBlock(env: BrowserEnv): Block | null {
  if (!env.secure) return 'insecure';
  if (!env.notification) return 'unsupported';
  if (env.permission === 'denied') return 'denied';
  return null;
}

export const BLOCK_TEXT: Record<Block, string> = {
  insecure: 'Browsers only allow notifications on a secure page: open xherd at http://localhost or http://127.0.0.1, or over https.',
  denied: "Notifications are blocked for this site in the browser's settings.",
  unsupported: 'This browser does not support notifications.',
};

/** This page's environment, for `browserBlock`. */
export const browserEnv = (): BrowserEnv => ({
  notification: supported(),
  secure: typeof window === 'undefined' || window.isSecureContext !== false,
  permission: supported() ? Notification.permission : undefined,
});

export function addNotice(jobId: string, kind: string, text: string) {
  const key = `${jobId}:${kind}`;
  const n: Notice = { key, jobId, kind, text, at: Date.now(), read: false };
  notices.list = [n, ...notices.list.filter((x) => x.key !== key)].slice(0, 50);
  if (notices.browser && supported() && Notification.permission === 'granted') {
    show(text, key, () => open(n));
  }
}

/** Shows one browser notification; a failure is kept in `notices.err`. */
function show(body: string, tag: string, onclick: () => void) {
  try {
    // `tag` makes the browser replace an earlier one of the same agent and
    // kind; `renotify` alerts again when it does (R4; missing from some DOM typings).
    const note = new Notification('xherd', { body, tag, renotify: true } as NotificationOptions);
    note.onclick = () => { window.focus(); onclick(); };
    note.onshow = () => { notices.err = ''; };
    note.onerror = () => { notices.err = 'The browser refused to show the notification.'; };
  } catch (e) {
    // Some browsers only allow it from a service worker (Chrome on Android).
    notices.err = `The browser refused to show the notification: ${e instanceof Error ? e.message : String(e)}`;
  }
}

/** Shows a test notification now (from the bell), so the whole path can be checked. */
export function testBrowser() {
  notices.why = browserBlock(browserEnv());
  if (notices.why || Notification.permission !== 'granted') return;
  show('Test: browser notifications work.', 'xherd:test', () => {});
}

export function open(n: Notice) {
  n.read = true;
  if (n.jobId.startsWith('todo:')) return go('/todos');
  const a = jobByName(n.jobId);
  if (a) go(`/jobs/${a.name}`);
}

// A notice that the agent waits for you is read once it no longer waits:
// the gate was approved or changes were requested (it runs again), the
// dialog was answered, the idle agent got work.
const over: Record<string, (a: Job) => boolean> = {
  gate: (a) => !a.gate || a.status === 'running',
  blocked: (a) => a.status !== 'blocked',
  idle: (a) => a.status !== 'idle',
  suggestion: (a) => !a.suggestion || a.suggestion.dismissed || hasAgents(a),
};

/** Marks read the agent's notices that its new state has settled. */
export function settle(a: Job) {
  for (const n of notices.list) if (!n.read && n.jobId === a.id && over[n.kind]?.(a)) n.read = true;
}

export const markAllRead = () => { for (const n of notices.list) n.read = true; };
export const clearAll = () => { notices.list = []; };

/** Asks for permission (call from a click) and turns browser notifications on. */
export async function enableBrowser() {
  notices.why = browserBlock(browserEnv());
  if (notices.why) return;
  const p = Notification.permission === 'granted' ? 'granted' : await Notification.requestPermission();
  notices.browser = p === 'granted';
  notices.why = browserBlock(browserEnv());
  try { localStorage.setItem(KEY, notices.browser ? '1' : '0'); } catch { /* private mode */ }
}

export function disableBrowser() {
  notices.browser = false;
  try { localStorage.setItem(KEY, '0'); } catch { /* private mode */ }
}
