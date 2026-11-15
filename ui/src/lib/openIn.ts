// Settings → Open in (R-706, R-707): pure helpers for the terminal handler's
// state, its install text, and the user links' URL check.
import type { OpenInView } from './types';

export type HandlerState = 'missing' | 'installed' | 'update';

/** Not installed, installed, or installed but needing a new install (an
 * older handler, or one that accepts another worktrees folder). */
export function handlerState(view: Pick<OpenInView, 'handler'>): HandlerState {
  const h = view.handler;
  if (!h.installed) return 'missing';
  return h.updateAvailable || h.rootChanged ? 'update' : 'installed';
}

export type InstallText = { native: true } | { native: false; install: string; uninstall: string };

/** Natively the daemon runs the installer itself (a button); in Docker the
 * user runs it on their computer, fetched from the page's own origin. */
export function installText(origin: string, native: boolean): InstallText {
  if (native) return { native: true };
  const url = `${origin.replace(/\/+$/, '')}/api/v1/open-in/terminal-handler.sh`;
  return { native: false, install: `curl -fsSL ${url} | sh`, uninstall: `curl -fsSL ${url} | sh -s uninstall` };
}

// Keep in step with `REFUSED_LINK_SCHEMES` in crates/xherd-daemon/src/settings.rs.
const REFUSED = ['http', 'https', 'ws', 'wss', 'ftp', 'javascript', 'data', 'vbscript', 'blob', 'file', 'about', 'xherd-term'];

/** Why a user link URL would be refused by the daemon, else null. */
export function badLinkUrl(url: string): string | null {
  const u = url.trim();
  if (new TextEncoder().encode(u).length > 2048) return 'is over 2048 bytes';
  // eslint-disable-next-line no-control-regex
  if (/[\s\u0000-\u001f\u007f-\u009f]/.test(u)) return 'has spaces or control characters';
  const m = /^([a-z][a-z0-9+.-]*):/i.exec(u);
  if (!m) return 'needs a scheme, like idea://';
  if (REFUSED.includes(m[1].toLowerCase())) return 'uses a scheme that is not allowed for app links (web pages and files are refused)';
  return null;
}

/** A link id from its name: lower-case letters, digits and `-`, at most 40,
 * not one of `taken`. */
export function linkId(label: string, taken: string[]): string {
  const base = label.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 36) || 'link';
  let id = base;
  for (let n = 2; taken.includes(id); n++) id = `${base}-${n}`;
  return id;
}
