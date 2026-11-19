// Job actions in the job menu (R24, R25): pure helpers, no DOM.
import type { JobAction } from './types';

export type GroupKind = 'service' | 'task';

/** Entries shown before "Show all (n)"; a group of at most `OPEN_MAX` starts open. */
export const VISIBLE_MAX = 8;
export const OPEN_MAX = 4;

export const running = (a: JobAction): boolean => a.last?.state === 'running';

/** An action nobody allowed yet (or whose command changed): its first Run asks. */
export const needsApproval = (a: JobAction): boolean => !a.approved;

/** Services and tasks apart; running first, then by label. */
export function groups(actions: JobAction[] | undefined): Record<GroupKind, JobAction[]> {
  const out: Record<GroupKind, JobAction[]> = { service: [], task: [] };
  for (const a of actions ?? []) out[a.def.kind === 'service' ? 'service' : 'task'].push(a);
  const order = (x: JobAction, y: JobAction) =>
    Number(running(y)) - Number(running(x)) ||
    (x.def.label || x.def.id).localeCompare(y.def.label || y.def.id);
  out.service.sort(order);
  out.task.sort(order);
  return out;
}

/** A group starts open with at most 4 entries (R24). */
export const defaultOpen = (n: number): boolean => n <= OPEN_MAX;

/** What a closed header still shows: a running service, an action awaiting approval. */
export interface HeaderMarks { running: number; approval: number }
export function headerMarks(entries: JobAction[]): HeaderMarks {
  return {
    running: entries.filter(running).length,
    approval: entries.filter(needsApproval).length,
  };
}

/** "Services · 3 · 1 running", "Actions · 5". */
export function headerText(kind: GroupKind, entries: JobAction[]): string {
  const base = `${kind === 'service' ? 'Services' : 'Actions'} · ${entries.length}`;
  const n = headerMarks(entries).running;
  return n > 0 && kind === 'service' ? `${base} · ${n} running` : base;
}

/** The first `VISIBLE_MAX` entries unless all are asked for; `hidden` is what "Show all (n)" counts. */
export function visible(entries: JobAction[], showAll: boolean): { shown: JobAction[]; hidden: number } {
  if (showAll || entries.length <= VISIBLE_MAX) return { shown: entries, hidden: 0 };
  return { shown: entries.slice(0, VISIBLE_MAX), hidden: entries.length - VISIBLE_MAX };
}

/** The dot of an entry: a mark and a word, never colour alone. */
export interface ActionResult { icon: string; text: string; tone: 'ok' | 'fail' | 'run' | 'none' }
export function lastResult(a: JobAction): ActionResult {
  const l = a.last;
  if (!l) return needsApproval(a) ? { icon: '?', text: 'Needs approval', tone: 'none' } : { icon: '○', text: 'Not run', tone: 'none' };
  switch (l.state) {
    case 'running': return { icon: '▶', text: 'Running', tone: 'run' };
    case 'stopped': return { icon: '■', text: 'Stopped', tone: 'none' };
    case 'lost': return { icon: '!', text: 'Lost', tone: 'fail' };
    default:
      return l.exit === 0 || l.exit === undefined
        ? { icon: '✓', text: 'Exited 0', tone: 'ok' }
        : { icon: '✗', text: `Exited ${l.exit}`, tone: 'fail' };
  }
}

/** The tab id of an action page (`?s=act-<id>`). */
export const tabId = (id: string): string => `act-${id}`;
export const actionOfTab = (tab: string | null | undefined): string | null =>
  tab && tab.startsWith('act-') && tab.length > 4 ? tab.slice(4) : null;

/** The R23 card: the offered ids that still exist, in the job's order. */
export function saveCard(actions: JobAction[] | undefined, offer: string[], dismissed: boolean | undefined): JobAction[] {
  if (dismissed) return [];
  const set = new Set(offer);
  return (actions ?? []).filter(a => set.has(a.def.id));
}
