// What needs your attention, for the Needs attention page and the ✋ in the
// bar: agents waiting for you (from /state), TODOs that are due or whose check
// failed, and auto-approvals not yet marked seen (R-111). The last two are
// fetched every 30 s and after any change the page makes (`refresh`).
// Approvals are listed on the page but never raise the hand (R1).
import { app } from './store.svelte';
import { get } from './api';
import { href } from './router.svelte';
import { needsYou } from './format';

export type Todo = { id: string; title: string; due: number; status: 'open' | 'checking' | 'due' | 'done' | 'error'; condition?: string; summary?: string };
export type AutoApproval = { jobId: string; at: number; gate: string; files: string[]; reason?: string; seen: boolean };

export const attention = $state({ todos: [] as Todo[], reviews: null as AutoApproval[] | null });

export async function refresh(): Promise<void> {
  const [t, r] = await Promise.allSettled([get<{ todos: Todo[] }>('/todos'), get<{ reviews: AutoApproval[] }>('/reviews')]);
  if (t.status === 'fulfilled') attention.todos = t.value.todos;
  if (r.status === 'fulfilled') attention.reviews = r.value.reviews;
}

let started = false;
/** Starts the 30 s refresh once (called by the bar). */
export function watchAttention(): void {
  if (started) return;
  started = true;
  refresh();
  setInterval(refresh, 30000);
}

export const waitingAgents = () => (app.state?.jobs ?? []).filter(needsYou).sort((a, b) => a.statusSince - b.statusSince);
export const dueTodos = () => attention.todos.filter((t) => t.status === 'due' || t.status === 'error').sort((a, b) => a.due - b.due);
export const unseenApprovals = () => (attention.reviews ?? []).filter((i) => !i.seen);
/** The bar's count: agents waiting for you and due TODOs, not approvals (R1). */
export const attentionCount = () => waitingAgents().length + dueTodos().length;

/** Where an approved item leads (R3): a docs gate opens the Spec tab on its
 *  first document, a commit gate (`commit C03`) the Changes tab on that commit. */
export function approvalHref(item: AutoApproval, agentName: string): string {
  const path = `/jobs/${agentName}`;
  const commit = /^commit (\S+)$/.exec(item.gate);
  if (commit) return href(path, { s: 'changes', view: 'commit', commit: commit[1] });
  return href(path, { s: 'spec', doc: item.files?.[0] });
}
