// Saved agents in the UI (#141, R23): pure helpers, tested in agents.test.ts.
import type { AgentDef, AgentView, Job } from './types';
import type { PermissionMode } from './modes';

/** The New job form's fields an agent can fill (the names of its state). */
export interface JobForm {
  runtime: string;
  model: string;
  autonomy: string;
  mode: PermissionMode | '';
}

export const FORM_DEFAULTS: JobForm = { runtime: '', model: '', autonomy: 'ask', mode: '' };

export type FormField = keyof JobForm;

/** Picking an agent fills the fields it sets and marks them "from agent"; the
 *  fields a previous agent filled go back to their defaults first. A field
 *  the user then edits loses its mark (`unmark`). `form` is not changed. */
export function fillFromAgent(
  agent: AgentDef | undefined,
  form: JobForm,
  previous: readonly FormField[] = [],
): { form: JobForm; marks: FormField[] } {
  const out: JobForm = { ...form };
  for (const k of previous) (out as Record<FormField, string>)[k] = FORM_DEFAULTS[k];
  const marks: FormField[] = [];
  if (!agent) return { form: out, marks };
  const set = <K extends FormField>(k: K, v: JobForm[K] | undefined) => {
    if (v === undefined || v === '') return;
    out[k] = v;
    marks.push(k);
  };
  set('runtime', agent.runtime);
  set('model', agent.models?.[agent.runtime]);
  set('autonomy', agent.autonomy);
  set('mode', agent.permissionMode);
  return { form: out, marks };
}

export const unmark = (marks: readonly FormField[], field: FormField): FormField[] => marks.filter((m) => m !== field);

/** Pickers offer an agent unless it is archived or cannot start jobs now (R15, R18). */
export const pickable = (a: AgentView): boolean => a.status !== 'archived' && !a.broken;

/** The profile fields `profileMarks` reads (`POST /repos/resolve` → `profile`). */
export interface ProfileLimits {
  name: string;
  autonomy?: string;
  permissionMode?: string;
  sensitivePaths?: string[];
  maxDiffLines?: number;
  jobTokenBudget?: number;
}

export type MarkKind = 'caps' | 'adds' | 'tightens';
export interface ProfileMark { kind: MarkKind; profile: string; text: string; lowered: boolean }

const AUTONOMY_RANK = ['ask', 'important', 'never'];
const MODE_RANK = ['ask', 'edits', 'auto', 'all'];
const rank = (order: string[], v: string | undefined) => (v === undefined ? -1 : order.indexOf(v));

/** The agent's fields a matching profile overrides or tightens (R13, R23):
 *  autonomy and tool permissions are "at most" values (`lowered` when the
 *  agent's is looser), sensitive paths are added to, and the smaller diff
 *  limit and token budget win. Only fields the profile sets are returned. */
export function profileMarks(a: Pick<AgentDef, 'autonomy' | 'permissionMode' | 'sensitivePaths' | 'maxDiffLines' | 'jobTokenBudget'>, p: ProfileLimits | null | undefined): Partial<Record<'autonomy' | 'permissionMode' | 'sensitivePaths' | 'maxDiffLines' | 'jobTokenBudget', ProfileMark>> {
  const out: ReturnType<typeof profileMarks> = {};
  if (!p) return out;
  if (p.autonomy) {
    const lowered = rank(AUTONOMY_RANK, a.autonomy) > rank(AUTONOMY_RANK, p.autonomy);
    out.autonomy = { kind: 'caps', profile: p.name, lowered, text: `at most ${p.autonomy}${lowered ? `, so ${a.autonomy} becomes ${p.autonomy}` : ''}` };
  }
  if (p.permissionMode) {
    const lowered = rank(MODE_RANK, a.permissionMode) > rank(MODE_RANK, p.permissionMode);
    out.permissionMode = { kind: 'caps', profile: p.name, lowered, text: `at most ${p.permissionMode}${lowered ? `, so ${a.permissionMode} becomes ${p.permissionMode}` : ''}` };
  }
  if (p.sensitivePaths?.length) {
    out.sensitivePaths = { kind: 'adds', profile: p.name, lowered: false, text: `added to the profile's ${p.sensitivePaths.length} path${p.sensitivePaths.length === 1 ? '' : 's'}` };
  }
  if (p.maxDiffLines) {
    const lowered = a.maxDiffLines !== undefined && a.maxDiffLines > p.maxDiffLines;
    out.maxDiffLines = { kind: 'tightens', profile: p.name, lowered, text: `the smaller of this and ${p.maxDiffLines}` };
  }
  if (p.jobTokenBudget) {
    const lowered = !!a.jobTokenBudget && a.jobTokenBudget > p.jobTokenBudget;
    out.jobTokenBudget = { kind: 'tightens', profile: p.name, lowered, text: `the smaller of this and ${p.jobTokenBudget}` };
  }
  return out;
}

export interface FieldDiff { field: string; label: string; from: string; to: string }

const FIELDS: [keyof AgentDef, string][] = [
  ['description', 'Description'],
  ['runtime', 'Runtime'],
  ['models', 'Models'],
  ['fallbackRuntime', 'Fallback runtime'],
  ['autonomy', 'Autonomy'],
  ['permissionMode', 'Tool permissions'],
  ['instructions', 'Instructions'],
  ['jobTokenBudget', 'Token budget per job'],
  ['sensitivePaths', 'Sensitive paths'],
  ['maxDiffLines', 'Max diff lines'],
  ['skills', 'Skills'],
  ['maxLiveJobs', 'Live jobs at once'],
];

/** One value as a line of text; empty when the field is not set. */
export function show(field: keyof AgentDef, v: unknown): string {
  if (v === undefined || v === null || v === '') return '';
  if (field === 'models') return Object.entries(v as Record<string, string>).map(([k, m]) => `${k}: ${m}`).join(', ');
  if (Array.isArray(v)) return v.join(', ');
  return String(v);
}

/** What changed between two versions of an agent, field by field (R11, R23).
 *  `from` and `to` are empty text for a field that is not set. */
export function versionDiff(from: AgentDef, to: AgentDef): FieldDiff[] {
  const out: FieldDiff[] = [];
  for (const [field, label] of FIELDS) {
    const a = show(field, from[field]);
    const b = show(field, to[field]);
    if (a !== b) out.push({ field, label, from: a, to: b });
  }
  return out;
}

export const NO_AGENT_LANE = 'No agent';

/** The group of a job when jobs are grouped by agent (Herd `?group=agent`,
 *  Board `?lanes=agent-def`): the snapshot's name, else `No agent`. */
export function agentLane(job: Pick<Job, 'agent'>): string {
  return job.agent?.name ?? NO_AGENT_LANE;
}

/** Lane names in name order, `No agent` last. */
export function agentLaneOrder(names: string[]): string[] {
  const rank = (n: string) => (n === NO_AGENT_LANE ? 1 : 0);
  return [...new Set(names)].sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}

/** A queued job waits for its agent's `maxLiveJobs`, not for a free slot (R16):
 *  its agent has as many live jobs as it may. Nothing is stored. */
export function waitsForAgent(job: Pick<Job, 'status' | 'agent'>, agents: readonly Pick<AgentView, 'id' | 'liveJobs' | 'maxLiveJobs'>[]): boolean {
  if (job.status !== 'queued' || !job.agent) return false;
  const a = agents.find((x) => x.id === job.agent!.id);
  return !!a && a.liveJobs >= a.maxLiveJobs;
}

/** The saved agent a job started from, when it still exists. */
export const agentOf = (job: Pick<Job, 'agent'>, agents: readonly AgentView[] | undefined): AgentView | undefined =>
  job.agent ? agents?.find((a) => a.id === job.agent!.id) : undefined;

/** The agent has a newer version than the job started with (R23). */
export const changedSince = (job: Pick<Job, 'agent'>, agent: Pick<AgentView, 'version'> | undefined): boolean =>
  !!job.agent && !!agent && agent.version > job.agent.version;

export const STATUS_LABEL: Record<AgentView['status'], string> = {
  archived: 'archived',
  'needs-you': 'needs you',
  working: 'working',
  idle: 'idle',
};

/** The edit form's text boxes → the body of `PUT /agents/{id}`; empty text
 *  clears a field (like the CLI). `base` is what the form started from. */
export interface AgentForm {
  name: string;
  description: string;
  runtime: string;
  model: string;
  fallbackRuntime: string;
  autonomy: string;
  permissionMode: string;
  instructions: string;
  jobTokenBudget: string;
  sensitivePaths: string;
  maxDiffLines: string;
  maxLiveJobs: string;
}

export function toForm(a?: AgentDef): AgentForm {
  return {
    name: a?.name ?? '',
    description: a?.description ?? '',
    runtime: a?.runtime ?? '',
    model: a ? (a.models?.[a.runtime] ?? '') : '',
    fallbackRuntime: a?.fallbackRuntime ?? '',
    autonomy: a?.autonomy ?? '',
    permissionMode: a?.permissionMode ?? '',
    instructions: a?.instructions ?? '',
    jobTokenBudget: a?.jobTokenBudget ? String(a.jobTokenBudget) : '',
    sensitivePaths: (a?.sensitivePaths ?? []).join('\n'),
    maxDiffLines: a?.maxDiffLines ? String(a.maxDiffLines) : '',
    maxLiveJobs: String(a?.maxLiveJobs ?? 1),
  };
}

/** The request body for the form; `old` keeps the models of other runtimes,
 *  the skills and the other fields the form has no box for. */
export function fromForm(f: AgentForm, old?: AgentDef): Record<string, unknown> {
  const text = (s: string) => (s.trim() === '' ? undefined : s.trim());
  const num = (s: string) => (s.trim() === '' ? undefined : Number(s));
  const models = { ...(old?.models ?? {}) };
  if (f.model.trim()) models[f.runtime] = f.model.trim();
  else delete models[f.runtime];
  return {
    name: f.name.trim(),
    description: f.description.trim(),
    runtime: f.runtime,
    models,
    fallbackRuntime: text(f.fallbackRuntime),
    autonomy: text(f.autonomy),
    permissionMode: text(f.permissionMode),
    instructions: f.instructions.trim() === '' ? undefined : f.instructions,
    jobTokenBudget: num(f.jobTokenBudget),
    sensitivePaths: f.sensitivePaths.split('\n').map((s) => s.trim()).filter(Boolean),
    maxDiffLines: num(f.maxDiffLines),
    skills: old?.skills ?? [],
    maxLiveJobs: Number(f.maxLiveJobs) || 1,
  };
}

/** A job's name-like slug: lowercase letters, digits and `-`, up to 41 characters (R10). */
export function slugify(s: string): string {
  return s.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 41).replace(/-+$/, '');
}

/** The suggestion is on offer: the job has no agent, the daemon made one and it was not dismissed (R30, R32). */
export const suggestionOpen = (job: Pick<Job, 'agent' | 'suggestion'>): boolean =>
  !job.agent && !!job.suggestion && !job.suggestion.dismissed;

/** Mark done is for a job with no workflow, between turns (R30). */
export const canMarkDone = (job: Pick<Job, 'flow' | 'status'>): boolean =>
  (!job.flow || job.flow === 'none') && ['idle', 'waiting-input', 'paused', 'parked'].includes(job.status);

/** The "Save as agent" form, pre-filled from what the job ran with (R31): the
 *  effective settings, a name and description from the repository and the flow,
 *  and the job's own draft (name, description, instructions) over them. */
export function fillFromJob(job: Job): AgentForm {
  const flow = job.flow ?? 'none';
  const repoDir = (job.repo ?? '').split('/').filter(Boolean).pop() ?? '';
  const form = toForm();
  form.name = slugify(`${repoDir || 'agent'}-${flow}`);
  form.description = repoDir ? `${flow === 'none' ? 'Work' : `${flow} work`} on ${repoDir}` : '';
  form.runtime = job.runtime ?? 'claude-code';
  form.model = job.model ?? '';
  form.autonomy = job.autonomy && job.autonomy !== 'never' ? job.autonomy : '';
  form.permissionMode = job.permissionMode ?? '';
  const d = job.agentDraft;
  if (d) {
    if (slugify(d.name) === d.name && d.name) form.name = d.name;
    if (d.description) form.description = d.description;
    form.instructions = d.instructions;
  }
  return form;
}
