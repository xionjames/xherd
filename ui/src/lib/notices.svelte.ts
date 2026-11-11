// Notifications (spec §16.3): `notify` messages from /ws. One per agent and
// kind: a new one replaces the previous. Browser notifications are opt-in
// (permission is asked on the click, never on load).
import { go } from './router.svelte';
import { agentByName } from './store.svelte';

export interface Notice { key: string; agentId: string; kind: string; text: string; at: number; read: boolean }

const KEY = 'xherd.notify';
const saved = (): boolean => { try { return localStorage.getItem(KEY) === '1'; } catch { return false; } };

export const notices = $state<{ list: Notice[]; browser: boolean }>({
  list: [],
  browser: saved() && typeof Notification !== 'undefined' && Notification.permission === 'granted',
});

export const unread = () => notices.list.filter((n) => !n.read).length;
export const supported = () => typeof Notification !== 'undefined';
export const denied = () => supported() && Notification.permission === 'denied';

export function addNotice(agentId: string, kind: string, text: string) {
  const key = `${agentId}:${kind}`;
  const n: Notice = { key, agentId, kind, text, at: Date.now(), read: false };
  notices.list = [n, ...notices.list.filter((x) => x.key !== key)].slice(0, 50);
  if (notices.browser && supported() && Notification.permission === 'granted') {
    try {
      // `tag` makes the browser replace an earlier one of the same agent and kind.
      const note = new Notification('xherd', { body: text, tag: key });
      note.onclick = () => { window.focus(); open(n); };
    } catch { /* some browsers only allow it from a service worker */ }
  }
}

export function open(n: Notice) {
  n.read = true;
  if (n.agentId.startsWith('todo:')) return go('/todos');
  const a = agentByName(n.agentId);
  if (a) go(`/agent/${a.name}`);
}

export const markAllRead = () => { for (const n of notices.list) n.read = true; };
export const clearAll = () => { notices.list = []; };

/** Asks for permission (call from a click) and turns browser notifications on. */
export async function enableBrowser() {
  if (!supported()) return;
  const p = Notification.permission === 'granted' ? 'granted' : await Notification.requestPermission();
  notices.browser = p === 'granted';
  try { localStorage.setItem(KEY, notices.browser ? '1' : '0'); } catch { /* private mode */ }
}

export function disableBrowser() {
  notices.browser = false;
  try { localStorage.setItem(KEY, '0'); } catch { /* private mode */ }
}
