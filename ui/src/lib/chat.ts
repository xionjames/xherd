// Pure helpers of the chat view (#202): the log folded into the list the page
// draws, the Chat / Terminal choice, the slash menu and path links.

import { pathUnder } from './termLinks';
import type { ChatEvent, ChatQuestion, JobView, PermOption, PlanItem, Resolution, Waiting } from './types';

/** An event the list shows; `xherd` marks a message the flow typed, not the person. */
export type Item = ChatEvent & { xherd?: boolean };

/** The log as the page draws it: one item per tool / permission / question
 * id (the latest event, at the place the id first appeared), the latest plan
 * apart, live status and unknown lines left out. */
export function fold(events: ChatEvent[]): { items: Item[]; plan: PlanItem[] | null } {
  const items: Item[] = [];
  const at = new Map<string, number>();
  let plan: PlanItem[] | null = null;
  for (const e of events) {
    if (e.kind === 'plan') {
      plan = e.items;
      continue;
    }
    if (e.kind === 'status' || e.kind === 'unknown') continue;
    const item: Item = e.kind === 'user' && e.source === 'xherd' ? { ...e, xherd: true } : e;
    if (e.kind === 'tool' || e.kind === 'permission' || e.kind === 'question') {
      const key = `${e.kind}:${e.id}`;
      const i = at.get(key);
      if (i !== undefined) {
        items[i] = item;
        continue;
      }
      at.set(key, items.length);
    }
    items.push(item);
  }
  return { items, plan };
}

/** The tool the agent is running now, for the "working…" line. */
export function runningTool(items: Item[]): string | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const e = items[i];
    if (e.kind === 'tool' && e.status === 'running') return e.summary;
  }
  return null;
}

/** What a resolved card says; null while it waits. */
export function resolutionText(r: Resolution): string | null {
  switch (r.state) {
    case 'waiting': return null;
    case 'answered': return `${r.answer} · ${r.from === 'chat' ? 'in the chat' : r.from === 'inbox' ? 'in the Inbox' : 'in the terminal'}`;
    case 'in-terminal': return 'Answered in the terminal';
    case 'timed-out': return 'No answer in time: answer in the terminal';
    case 'lost': return 'Lost when xherd restarted: answer in the terminal';
  }
}

const key = (jobId: string) => `xherd.view.${jobId}`;

/** The view a job opens on: its own choice in this browser, else the config's
 * `ui.jobView`; always the terminal for a runtime without chat. */
export function jobView(jobId: string, hasChat: boolean, configView: unknown): JobView {
  if (!hasChat) return 'terminal';
  try {
    const v = localStorage.getItem(key(jobId));
    if (v === 'chat' || v === 'terminal') return v;
  } catch { /* storage blocked */ }
  return configView === 'chat' ? 'chat' : 'terminal';
}

export function saveJobView(jobId: string, view: JobView): void {
  try { localStorage.setItem(key(jobId), view); } catch { /* storage blocked */ }
}

/** Slash commands that start with what was typed (`/` + prefix, no space yet);
 * empty once the text has a space or does not start with `/`. */
export function slashMatches(input: string, commands: string[]): string[] {
  if (!input.startsWith('/') || /\s/.test(input)) return [];
  const p = input.toLowerCase();
  return commands.filter((c) => c.toLowerCase().startsWith(p));
}

const PATH = /(^|[\s"'(\[])(\/[\w@+.\-/]*[\w@+\-/])/g;

/** Links the absolute paths under `workdir` in rendered HTML (R9). Only text
 * outside tags and outside existing links is touched; `href` builds the
 * link's address from the clean path. */
export function linkPaths(html: string, workdir: string, href: (path: string) => string): string {
  let inLink = 0;
  return html
    .split(/(<[^>]*>)/)
    .map((part) => {
      if (part.startsWith('<')) {
        if (/^<a[\s>]/i.test(part)) inLink++;
        else if (/^<\/a\s*>/i.test(part)) inLink = Math.max(0, inLink - 1);
        return part;
      }
      if (inLink) return part;
      return part.replace(PATH, (whole, lead: string, p: string) => {
        const clean = pathUnder(workdir, p);
        if (!clean) return whole;
        return `${lead}<a href="${href(clean).replace(/"/g, '&quot;')}" download>${p}</a>`;
      });
    })
    .join('');
}

/** What `WaitingCard` draws: a permission or a question with how it stands. */
export interface Card {
  kind: 'permission' | 'question';
  id: string;
  /** A permission's tool, or a question's header. */
  title: string;
  /** A permission's one-line summary, or a question's text. */
  text: string;
  options: PermOption[];
  questions: ChatQuestion[];
  /** A question with buttons: one single-select question (R21). */
  answerable: boolean;
  res: Resolution;
}

export function cardFromEvent(e: ChatEvent): Card | null {
  if (e.kind === 'permission') {
    return { kind: 'permission', id: e.id, title: e.tool, text: e.summary, options: e.options, questions: [], answerable: true, res: e };
  }
  if (e.kind === 'question') {
    const q = e.questions[0];
    return { kind: 'question', id: e.id, title: q?.header || 'Question', text: q?.question ?? '', options: [], questions: e.questions, answerable: e.answerable, res: e };
  }
  return null;
}

/** A card for what `Job.waiting` lists: always waiting. */
export function cardFromWaiting(w: Waiting): Card {
  const q = w.questions?.[0];
  return w.kind === 'permission'
    ? { kind: 'permission', id: w.id, title: w.tool ?? 'Tool', text: w.summary, options: w.options ?? [], questions: [], answerable: true, res: { state: 'waiting' } }
    : { kind: 'question', id: w.id, title: q?.header || 'Question', text: q?.question ?? w.summary, options: [], questions: w.questions ?? [], answerable: w.answerable, res: { state: 'waiting' } };
}

export const PERM_LABEL: Record<PermOption, string> = { allow: 'Allow', 'allow-always': 'Always allow', deny: 'Deny' };

/** The buttons of a card: one per option while it waits, none after.
 * `value` is what the route takes (the answer, or the 0-based option). */
export function cardButtons(c: Card): { label: string; value: PermOption | number }[] {
  if (c.res.state !== 'waiting' || !c.answerable) return [];
  if (c.kind === 'permission') return c.options.map((o) => ({ label: PERM_LABEL[o], value: o }));
  return (c.questions[0]?.options ?? []).map((o, i) => ({ label: o.label, value: i }));
}

/** The line under a waiting card that cannot be answered here. */
export function cardNote(c: Card): string | null {
  if (c.res.state === 'waiting' && !c.answerable) return 'Answer in the terminal.';
  return resolutionText(c.res);
}
