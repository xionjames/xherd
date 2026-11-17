// The Jobs page (the Herd, calm version): which group a job is in, and the
// stage track, one segment per stage of its workflow with the initials of the
// agent that runs it.
import type { Job, FlowInfo } from './types';
import { needsYou, agentHue } from './format';
import { flowOf, stageLabel, steps, type StepState } from './flow';
import { agentFor } from './agents';

export type JobGroup = 'needs' | 'running' | 'waiting' | 'done';
export const GROUPS: JobGroup[] = ['needs', 'running', 'waiting', 'done'];
export const GROUP_LABEL: Record<JobGroup, string> = { needs: 'Needs you', running: 'Running', waiting: 'Waiting', done: 'Done' };

/** Needs you, running, waiting (queued, paused, idle, watching a PR, failed) or done. */
export function jobGroup(a: Job): JobGroup {
  if (needsYou(a)) return 'needs';
  if (a.status === 'running') return 'running';
  if (a.status === 'done' || a.status === 'cancelled') return 'done';
  return 'waiting';
}

/** The Jobs list order (R3): the ones that need you first, longest waiting first;
 *  then the rest, newest first. Ties by name. Returns a new list. */
export function sortJobs(list: Job[]): Job[] {
  return [...list].sort((a, b) => {
    const na = jobGroup(a) === 'needs';
    if (na !== (jobGroup(b) === 'needs')) return na ? -1 : 1;
    const d = na ? a.statusSince - b.statusSince : b.createdAt - a.createdAt;
    return d || a.name.localeCompare(b.name);
  });
}

/** Two letters for an avatar: `el-chino` → EC, `archie` → AR, none → –. */
export function initials(name?: string): string {
  if (!name) return '–';
  const parts = name.split(/[-_\s]+/).filter(Boolean);
  const s = parts.length > 1 ? parts[0][0] + parts[1][0] : name.replace(/[^a-z0-9]/gi, '').slice(0, 2);
  return s ? s.toUpperCase() : '–';
}

export interface Seg { stage: string; label: string; state: StepState; agent?: string; initials: string }

/** The track of a job: every stage of its workflow with its state and agent.
 *  A job with no workflow has none. */
export function track(a: Job, flows: FlowInfo[]): Seg[] {
  const f = flowOf(a, flows);
  if (!f) return [];
  return steps(a, f).map(({ stage, state }) => {
    const agent = agentFor(a, stage.id);
    return { stage: stage.id, label: stageLabel(stage), state, agent, initials: initials(agent) };
  });
}

/** The words for a segment's tooltip: "Spec · archie (current)". */
export const segTitle = (s: Seg): string =>
  `${s.label}${s.agent ? ` · ${s.agent}` : ''}${s.state === 'now' ? ' (waits for you)' : s.state === 'cur' ? ' (current)' : s.state === 'ok' ? ' (done)' : s.state === 'skip' ? ' (skipped)' : ''}`;

/** An avatar's background: the agent's hue, light enough for dark initials. */
export const avatarBg = (name?: string): string | undefined => (name ? agentHue(name).replace('62% 48%', '58% 72%') : undefined);

export type JobFilter = JobGroup | 'all';
export const isFilter = (f: string | null | undefined): f is JobFilter => f === 'all' || GROUPS.includes(f as JobGroup);

/** The jobs a counter shows: `all` hides the done ones, as every Jobs layout does. */
export const filterJobs = (jobs: Job[], f: JobFilter): Job[] =>
  f === 'all' ? jobs.filter((j) => jobGroup(j) !== 'done') : jobs.filter((j) => jobGroup(j) === f);

/** What the Waiting counter holds, in words: "2 queued · 1 watching PR". */
export function waitingDetail(jobs: Job[]): string {
  const n = (s: Job['status']) => jobs.filter((j) => j.status === s).length;
  const parts: [number, string][] = [[n('queued'), 'queued'], [n('watching'), 'watching PR'], [n('paused'), 'paused'], [n('parked'), 'parked'], [n('idle'), 'idle'], [n('error'), 'failed']];
  return parts.filter(([c]) => c).map(([c, w]) => `${c} ${w}`).join(' · ');
}
