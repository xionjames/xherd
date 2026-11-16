// Herd view plugins, page side of protocol v1 (pure; R9, R13).
//
// A view plugin runs in a sandboxed iframe with an opaque origin. The Herd
// page sends it `data` messages holding a projection of the agents (never
// their prompts, messages, question texts, documents, comments or paths)
// and accepts only `ready` and `open` back. Keep the shapes in step with
// `ui/view-kit/xherd-view.js`.
import { stageNow } from './flow';
import { agentHue, needsYou, repoName } from './format';
import { jobAgent, usesAgent } from './agents';
import type { AgentView, Job, FlowInfo, Gate, Status, Ticket } from './types';

export const VIEW_PROTOCOL = 1;

/** What a view plugin sees of an agent. */
export interface ViewAgent {
  id: string;
  name: string;
  status: Status;
  statusSince: number;
  createdAt: number;
  lastActivity: number;
  /** The repository's folder name. */
  repo: string;
  branch: string;
  ticket?: Ticket;
  runtime?: string;
  flow?: string;
  stage?: string;
  /** The stage's label (its role's name). */
  stageName?: string;
  /** What the open gate waits on: document names, a commit id, a count. */
  waitsOn?: string;
  /** The open gate's kind only. */
  gate?: Gate['kind'];
  needsYou: boolean;
  /** The saved agent running the job's current stage, by name; absent for a job with none. */
  agent?: string;
  /** The agent's colour, a CSS `hsl(…)`. */
  hue: string;
  prUrl?: string;
  prState?: Job['prState'];
  prCi?: Job['prCi'];
}

/** What a view plugin sees of a saved agent: never its instructions, memory,
 *  skills, budgets or paths. `jobs` are ids of jobs in `agents`. */
export interface ViewSavedAgent {
  id: string;
  name: string;
  description: string;
  version: number;
  runtime: string;
  status: AgentView['status'];
  archived: boolean;
  liveJobs: number;
  maxLiveJobs: number;
  tokensToday: number;
  jobs: string[];
}

export interface DataMessage {
  xherdView: typeof VIEW_PROTOCOL;
  kind: 'data';
  /** The jobs (the field keeps its protocol v1 name). */
  agents: ViewAgent[];
  /** The saved agents, archived ones included (flagged). */
  savedAgents: ViewSavedAgent[];
  flows: FlowInfo[];
  theme: 'light' | 'dark';
}

export type ViewRequest = { kind: 'ready' } | { kind: 'open'; id: string } | { kind: 'open-agent'; name: string };

const str = (v: unknown) => (typeof v === 'string' ? v : undefined);

/** The projection of one agent; fields not listed here never reach a view. */
export function project(a: Job, flows: FlowInfo[]): ViewAgent {
  const now = stageNow(a, flows);
  const ticket = a.ticket ? { key: str(a.ticket.key), url: str(a.ticket.url) } : undefined;
  return strip({
    id: a.id,
    name: a.name,
    status: a.status,
    statusSince: a.statusSince,
    createdAt: a.createdAt,
    lastActivity: a.lastActivity,
    repo: repoName(a),
    branch: a.branch,
    ticket: ticket && strip(ticket),
    runtime: a.runtime,
    flow: a.flow,
    stage: a.stage,
    stageName: now?.name,
    waitsOn: now?.waits,
    gate: a.gate?.kind,
    needsYou: needsYou(a),
    agent: jobAgent(a)?.name,
    hue: agentHue(a.name),
    prUrl: a.prUrl,
    prState: a.prState,
    prCi: a.prCi,
  });
}

/** Drops undefined fields, so the message holds only what is known. */
function strip<T extends object>(o: T): T {
  return Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as T;
}

/** The projection of one saved agent, with the ids of the jobs that use it. */
export function projectSaved(d: AgentView, jobs: readonly Job[]): ViewSavedAgent {
  return {
    id: d.id,
    name: d.name,
    description: d.description,
    version: d.version,
    runtime: d.runtime,
    status: d.status,
    archived: d.archivedAt !== undefined,
    liveJobs: d.liveJobs,
    maxLiveJobs: d.maxLiveJobs,
    tokensToday: d.tokensToday,
    jobs: jobs.filter((j) => usesAgent(j, d.id)).map((j) => j.id),
  };
}

export function dataMessage(agents: Job[], flows: FlowInfo[], theme: 'light' | 'dark', saved: readonly AgentView[] = []): DataMessage {
  return {
    xherdView: VIEW_PROTOCOL,
    kind: 'data',
    agents: agents.map((a) => project(a, flows)),
    savedAgents: saved.map((d) => projectSaved(d, agents)),
    flows: flows.map((f) => ({ pack: f.pack, flow: f.flow, stages: f.stages.map((s) => ({ id: s.id, role: s.role, class: s.class, gate: s.gate })) })),
    theme,
  };
}

/**
 * A message from the view, or null to ignore it: it must come from the
 * frame's window, with the opaque origin `'null'`, protocol v1, and be
 * `ready`, `open` of a job the page sent, or `open-agent` of a saved agent it sent.
 */
export function acceptMessage(
  e: { source: unknown; origin: string; data: unknown },
  frame: unknown,
  ids: ReadonlySet<string>,
  agentNames: ReadonlySet<string> = new Set(),
): ViewRequest | null {
  if (!frame || e.source !== frame || e.origin !== 'null') return null;
  const m = e.data;
  if (!m || typeof m !== 'object') return null;
  const d = m as Record<string, unknown>;
  if (d.xherdView !== VIEW_PROTOCOL) return null;
  if (d.kind === 'ready') return { kind: 'ready' };
  if (d.kind === 'open' && typeof d.agentId === 'string' && ids.has(d.agentId)) return { kind: 'open', id: d.agentId };
  if (d.kind === 'open-agent' && typeof d.name === 'string' && agentNames.has(d.name)) return { kind: 'open-agent', name: d.name };
  return null;
}
