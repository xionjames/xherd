// What a job waiting for the user shows in the Inbox and on the Needs-you
// cards: the tab that holds its decision, the button's word, a line of facts.
import type { Job } from './types';
import { prGate } from './flow';
import { repoName } from './format';

/** The tab that holds the decision for a waiting job. */
export function decisionTab(a: Job): string {
  const g = a.gate;
  if (!g) return 'terminal';
  if (g.kind === 'commit') return 'changes';
  if (g.kind === 'question') return 'terminal';
  if (prGate(g)) return 'pr';
  if (g.kind === 'docs') return 'spec';
  return 'pr';
}

export function actionLabel(a: Job): string {
  const g = a.gate;
  if (!g || g.kind === 'question') return 'Answer';
  if (g.kind === 'commit') return 'Review changes';
  if (prGate(g)) return 'Decide';
  if (g.kind === 'docs') return 'Review';
  return 'Decide';
}

/** The headline: "Approve spec", "Review C03"… */
export function waitTitle(a: Job): string {
  const g = a.gate;
  if (!g) return a.status === 'blocked' ? 'Blocked' : 'Answer needed';
  if (g.kind === 'commit') return `Review commit ${g.id}`;
  if (g.kind === 'question') return 'Answer a question';
  if (prGate(g)) return 'Decide on PR comments';
  if (g.kind === 'docs') return `Approve ${g.stage}`;
  return 'Decision needed';
}

export function waitMeta(a: Job): string {
  const g = a.gate;
  const parts = [repoName(a)];
  if (g?.kind === 'commit') parts.push(`${g.files.length} files`);
  const nq = g?.kind === 'docs' ? (g.questions?.length ?? 0) + (g.asked?.length ?? 0) : 0;
  if (nq) parts.push(`${nq} open question${nq === 1 ? '' : 's'}`);
  if (g && 'needsHuman' in g && g.needsHuman) parts.push(`held: ${g.needsHuman}`);
  if (!g) parts.push('asked in the terminal');
  return parts.join(' · ');
}
