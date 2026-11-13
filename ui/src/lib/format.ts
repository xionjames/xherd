import type { Agent, Status } from './types';

/** "now", "42m", "3h", "2d" since `ms`. */
export function ago(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return 'now';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

/** "just now" or "42m ago". */
export function agoText(ms: number, now = Date.now()): string {
  const a = ago(ms, now);
  return a === 'now' ? 'just now' : `${a} ago`;
}

export const STATUS_LABEL: Record<Status, string> = {
  queued: 'queued',
  running: 'running',
  'waiting-input': 'your answer',
  blocked: 'blocked',
  idle: 'idle',
  parked: 'parked',
  paused: 'paused',
  watching: 'watching PR',
  done: 'done',
  cancelled: 'cancelled',
  error: 'error',
};

/** Hollow dot for agents that are not working and not waiting on you. */
export function hollow(s: Status): boolean {
  return ['queued', 'paused', 'watching', 'parked', 'cancelled'].includes(s);
}

/** Needs you = waiting for input, blocked, or an open gate (R-007). */
export function needsYou(a: Agent): boolean {
  return a.status === 'waiting-input' || a.status === 'blocked' || !!a.gate;
}

/** One-line description of what waits on the human. */
export function waitText(a: Agent): string {
  const g = a.gate;
  if (!g) return a.status === 'blocked' ? (a.blockReason ? `Blocked: ${a.blockReason}.` : 'The agent is blocked.') : 'The agent is waiting for your answer.';
  switch (g.kind) {
    case 'docs': return `${g.files.map((f) => f.path).join(', ') || g.stage} ready for review`;
    case 'commit': return `${g.id} ${g.subject}`;
    case 'pr-triage': return `${g.count} new PR review comments waiting for your decision`;
    case 'pr-replies': return `${g.count} PR replies waiting for your approval`;
    case 'question': return g.text;
  }
}

/** Since when the agent waits on you (oldest first in lists). */
export const waitingSince = (a: Agent) => a.statusSince;

/** Stable per-agent hue (never in the coral range reserved for "needs you"). */
export function agentHue(name: string): string {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  // 70..320 avoids the red/orange band reserved for "needs you".
  const hue = 70 + (h % 250);
  return `hsl(${hue} 62% 48%)`;
}

export function repoName(a: Agent): string {
  return a.repo?.split('/').filter(Boolean).pop() ?? 'folder';
}

export function shortBase(a: Agent): string {
  return a.base?.replace(/^origin\//, '') ?? '';
}

/** `1h 05m`, `12m`, `45s`: a span of work for the work log. */
export function span(ms: number): string {
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h ${String(m % 60).padStart(2, '0')}m` : `${Math.floor(h / 24)}d ${h % 24}h`;
}
