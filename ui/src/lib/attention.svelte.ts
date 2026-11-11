// What needs your attention, for the Needs attention page and the ✋ in the
// bar: agents waiting for you (from /state), TODOs that are due or whose check
// failed, and auto-approvals not yet marked seen (R-111). The last two are
// fetched every 30 s and after any change the page makes (`refresh`).
import { app } from './store.svelte';
import { get } from './api';
import { needsYou } from './format';

export type Todo = { id: string; title: string; due: number; status: 'open' | 'checking' | 'due' | 'done' | 'error'; condition?: string; summary?: string };
export type AutoApproval = { agentId: string; at: number; gate: string; files: string[]; reason?: string; seen: boolean };

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

export const waitingAgents = () => (app.state?.agents ?? []).filter(needsYou).sort((a, b) => a.statusSince - b.statusSince);
export const dueTodos = () => attention.todos.filter((t) => t.status === 'due' || t.status === 'error').sort((a, b) => a.due - b.due);
export const unseenApprovals = () => (attention.reviews ?? []).filter((i) => !i.seen);
export const attentionCount = () => waitingAgents().length + dueTodos().length + unseenApprovals().length;
